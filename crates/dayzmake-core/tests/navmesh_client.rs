//! Tests of the navmesh data-server client and the `.dzgeo` capture files (spec 3.1, 3.6, 3.8).

#![cfg(feature = "navmesh")]
#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use std::io::{self, Cursor};
use std::net::TcpListener;
use std::time::Duration;

use dayzmake_core::navmesh::capture::{Capture, CaptureWriter};
use dayzmake_core::navmesh::client::{Client, FetchError, Host, RetryPolicy};
use dayzmake_core::navmesh::wire::{
    CellClass, DEFAULT_PORT, TileGroup, TileInput, TilePart, WorldBlock, WorldParams,
};

#[path = "support/navmesh_server.rs"]
mod navmesh_server;

use navmesh_server::{FakeServer, Fault, Faults};

fn world() -> WorldParams {
    let mut block = WorldBlock::default();
    block.world_size_x = 100.0;
    block.world_size_z = 100.0;
    block.tile_width = 50.0;
    block.tile_height = 50.0;
    block.tiles_x = 2;
    block.tiles_z = 2;
    block.block_size = 4;
    let mut class = CellClass::default();
    class.cells = 200;
    class.cells2 = 200;
    class.width = 50.0;
    let mut params = WorldParams::default();
    params.world = block;
    params.cell_classes = vec![class];
    params
}

fn small(v: i32) -> f32 {
    f32::from(i16::try_from(v).expect("small"))
}

fn tile(x: i32, z: i32) -> TileInput {
    let mut group = TileGroup::default();
    group.area = 1;
    group.parts = vec![TilePart::default()];
    group.verts = vec![[0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 1.0]];
    let mut t = TileInput::default();
    t.x = x;
    t.z = z;
    let (fx, fz) = (small(x), small(z));
    t.bmin = [50.0 * fx, 1.0, 50.0 * fz];
    t.bmax = [50.0 * (fx + 1.0), 1.0, 50.0 * (fz + 1.0)];
    t.groups = vec![group];
    t
}

fn policy() -> RetryPolicy {
    let mut p = RetryPolicy::default();
    p.delay_unit = Duration::ZERO;
    p.connect_timeout = Duration::from_secs(5);
    p.recv_timeout = Duration::from_secs(5);
    p
}

fn client_for(server: &FakeServer) -> Client {
    let host = Host::parse(&server.addr().to_string()).expect("host");
    Client::connect(&host, &policy()).expect("connect")
}

fn tiles() -> Vec<TileInput> {
    vec![tile(0, 0), tile(1, 0), tile(0, 1)]
}

fn msg<T>(r: Result<T, FetchError>) -> String {
    match r {
        Ok(_) => "<ok>".to_owned(),
        Err(e) => e.to_string(),
    }
}

#[test]
fn defaults() {
    let p = RetryPolicy::default();
    assert_eq!(p.attempts, 5);
    assert_eq!(p.delay_unit, Duration::from_secs(2));
    assert_eq!(p.connect_timeout, Duration::from_secs(10));
    assert_eq!(p.recv_timeout, Duration::from_secs(600));
}

#[test]
fn host_parse_and_display() {
    let h = Host::parse("example.org").expect("host");
    assert_eq!(h.to_string(), format!("example.org:{DEFAULT_PORT}"));
    let h = Host::parse("127.0.0.1:4000").expect("host");
    assert_eq!(h.to_string(), "127.0.0.1:4000");
    for bad in ["", ":5", "a:", "a:x", "a:70000", "a:1:2"] {
        assert!(Host::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn probe_world_params() {
    let server = FakeServer::start(world(), tiles(), Faults::none());
    let mut client = client_for(&server);
    assert_eq!(client.world_params().expect("params"), world());
    assert_eq!(server.connections(), 1);
}

#[test]
fn tile_fetch() {
    let server = FakeServer::start(world(), tiles(), Faults::none());
    let mut client = client_for(&server);
    let w = client.world_params().expect("params");
    let t = client.tile(1, 0, &w).expect("tile");
    assert_eq!(t, tile(1, 0));
    // A second tile on the same connection.
    assert_eq!(client.tile(0, 1, &w).expect("tile"), tile(0, 1));
    assert_eq!(server.connections(), 1);
    assert_eq!(server.requests(), vec![(1, 0), (0, 1)]);
}

#[test]
fn tile_without_world_params_request() {
    // Secondary fetch connections send a tile request first (spec 3.1).
    let server = FakeServer::start(world(), tiles(), Faults::none());
    let mut client = client_for(&server);
    assert_eq!(client.tile(0, 0, &world()).expect("tile"), tile(0, 0));
}

#[test]
fn wrong_tile_is_network_error_and_retried() {
    let server = FakeServer::start(world(), tiles(), Faults::script(&[Fault::WrongTile]));
    let mut client = client_for(&server);
    let t = client.tile(0, 0, &world()).expect("tile");
    assert_eq!(t, tile(0, 0));
    assert_eq!(server.connections(), 2);
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn wrong_message_and_unknown_cell_class_are_network_errors() {
    let server = FakeServer::start(world(), tiles(), Faults::always(Fault::WrongMessage));
    let mut client = client_for(&server);
    let e = client.tile(0, 0, &world()).expect_err("fails");
    assert!(matches!(e, FetchError::Network(_)), "{e}");
    assert!(
        e.to_string().contains("expected tile, got message 4"),
        "{e}"
    );

    let mut odd = tile(0, 0);
    odd.cell_class = 7;
    let server = FakeServer::start(world(), vec![odd], Faults::none());
    let mut client = client_for(&server);
    let e = client.tile(0, 0, &world()).expect_err("fails");
    assert!(matches!(e, FetchError::Network(_)), "{e}");
    assert!(e.to_string().contains("cell class"), "{e}");
    assert_eq!(server.connections(), 5);
}

#[test]
fn malformed_tile_is_wire_error_not_retried() {
    let server = FakeServer::start(world(), tiles(), Faults::always(Fault::Malformed));
    let mut client = client_for(&server);
    let e = client.tile(0, 0, &world()).expect_err("fails");
    assert!(matches!(e, FetchError::Wire(_)), "{e}");
    assert_eq!(server.requests().len(), 1);
    assert_eq!(server.connections(), 1);
}

#[test]
fn retries_give_up_after_five() {
    let server = FakeServer::start(world(), tiles(), Faults::always(Fault::Close));
    let mut client = client_for(&server);
    let e = client.tile(0, 0, &world()).expect_err("fails");
    assert!(matches!(e, FetchError::Network(_)), "{e}");
    assert_eq!(server.connections(), 5);
    assert_eq!(server.requests().len(), 5);
}

#[test]
fn frame_length_rules() {
    for (fault, text) in [
        (Fault::ShortFrame, "invalid frame"),
        (Fault::HugeFrame, "invalid frame"),
    ] {
        let server = FakeServer::start(world(), tiles(), Faults::always(fault));
        let mut client = client_for(&server);
        let e = client.tile(0, 0, &world()).expect_err("fails");
        assert!(matches!(e, FetchError::Network(_)), "{e}");
        assert!(msg::<()>(Err(e)).contains(text));
    }
}

#[test]
fn recv_timeout_is_a_network_error() {
    // A listener that never answers.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let host = Host::parse(&listener.local_addr().expect("addr").to_string()).expect("host");
    let mut p = policy();
    p.recv_timeout = Duration::from_millis(50);
    p.attempts = 1;
    let mut client = Client::connect(&host, &p).expect("connect");
    let e = client.tile(0, 0, &world()).expect_err("times out");
    assert!(matches!(e, FetchError::Network(_)), "{e}");
    assert!(e.to_string().contains("timed out"), "{e}");
}

#[test]
fn recv_timeout_covers_the_whole_message() {
    // Every single read returns within the timeout; the message as a whole does not.
    let server = FakeServer::start(world(), tiles(), Faults::always(Fault::Trickle));
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut p = policy();
    p.recv_timeout = Duration::from_millis(50);
    p.attempts = 1;
    let mut client = Client::connect(&host, &p).expect("connect");
    let started = std::time::Instant::now();
    let e = client.tile(0, 0, &world()).expect_err("times out");
    assert!(matches!(e, FetchError::Network(_)), "{e}");
    assert!(e.to_string().contains("timed out"), "{e}");
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn connect_refused_names_host() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        l.local_addr().expect("addr").port()
    };
    let host = Host::parse(&format!("127.0.0.1:{port}")).expect("host");
    let e = Client::connect(&host, &policy()).expect_err("refused");
    let text = e.to_string();
    assert!(text.contains(&format!("127.0.0.1:{port}")), "{text}");
    assert!(
        text.contains("start DayZDiag_x64.exe with -server -startNavmeshDataServer"),
        "{text}"
    );
}

#[test]
fn world_params_error_is_one_attempt() {
    let server = FakeServer::start(world(), tiles(), Faults::none());
    let mut client = client_for(&server);
    drop(server);
    assert!(client.world_params().is_err());
}

fn capture_bytes(params: &WorldParams, tiles: &[TileInput]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut w = CaptureWriter::new(&mut out, params).expect("writer");
    for t in tiles {
        w.add(t).expect("add");
    }
    out
}

fn err_text<T>(r: io::Result<T>) -> String {
    match r {
        Ok(_) => "<ok>".to_owned(),
        Err(e) => e.to_string(),
    }
}

#[test]
fn capture_roundtrip() {
    let t = tiles();
    let bytes = capture_bytes(&world(), &t);
    assert_eq!(&bytes[..4], b"DZGC");
    assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
    let c = Capture::read(&mut Cursor::new(&bytes)).expect("read");
    assert_eq!(c.params(), &world());
    // Sorted by (z, x).
    let order: Vec<_> = c.tiles().iter().map(|t| (t.z, t.x)).collect();
    assert_eq!(order, vec![(0, 0), (0, 1), (1, 0)]);
    assert_eq!(c.tile(0, 1), Some(&tile(0, 1)));
    assert_eq!(c.tile(5, 5), None);
}

#[test]
fn capture_truncated_keeps_complete_tiles() {
    let bytes = capture_bytes(&world(), &[tile(0, 0), tile(1, 0)]);
    // Cut inside the second tile's block, and inside its size field.
    for cut in [bytes.len() - 7, bytes.len() - tile(1, 0).encode().len() - 2] {
        let c = Capture::read(&mut Cursor::new(&bytes[..cut])).expect("read");
        assert_eq!(c.tiles().len(), 1, "cut {cut}");
        assert_eq!(c.tile(0, 0), Some(&tile(0, 0)));
    }
    // Truncated before the world block is complete.
    assert_eq!(
        err_text(Capture::read(&mut Cursor::new(&bytes[..20]))),
        "capture truncated"
    );
    assert_eq!(
        err_text(Capture::read(&mut Cursor::new(&bytes[..9]))),
        "capture truncated"
    );
}

#[test]
fn capture_bad_magic() {
    assert_eq!(
        err_text(Capture::read(&mut Cursor::new(
            b"NOPE\x01\0\0\0".as_slice()
        ))),
        "not a capture file"
    );
    assert_eq!(
        err_text(Capture::read(&mut Cursor::new(b"DZ".as_slice()))),
        "not a capture file"
    );
}

#[test]
fn capture_bad_version() {
    let mut bytes = capture_bytes(&world(), &[]);
    bytes[4] = 2;
    assert!(
        err_text(Capture::read(&mut Cursor::new(&bytes)))
            .starts_with("unsupported capture version")
    );
}

#[test]
fn capture_duplicate_tile_keeps_later() {
    let mut later = tile(0, 0);
    later.bmax = [9.0, 9.0, 9.0];
    let bytes = capture_bytes(&world(), &[tile(0, 0), tile(1, 0), later.clone()]);
    let c = Capture::read(&mut Cursor::new(&bytes)).expect("read");
    assert_eq!(c.tiles().len(), 2);
    assert_eq!(c.tile(0, 0), Some(&later));
}

#[test]
fn capture_hostile_sizes_do_not_allocate() {
    // World block claims 1 GiB but the file ends.
    let mut bytes = b"DZGC".to_vec();
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&(1u32 << 30).to_le_bytes());
    bytes.extend_from_slice(&[0; 10]);
    assert_eq!(
        err_text(Capture::read(&mut Cursor::new(&bytes))),
        "capture truncated"
    );
    // A block bigger than the frame cap is refused outright.
    let mut bytes = capture_bytes(&world(), &[]);
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(err_text(Capture::read(&mut Cursor::new(&bytes))).contains("block size"));
    // A complete but malformed tile block is an error.
    let mut bytes = capture_bytes(&world(), &[]);
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&[1, 2, 3]);
    assert!(Capture::read(&mut Cursor::new(&bytes)).is_err());
}

#[test]
fn capture_writer_flushes_every_tile() {
    struct Counting(Vec<u8>, usize);
    impl io::Write for Counting {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.1 += 1;
            Ok(())
        }
    }
    let mut sink = Counting(Vec::new(), 0);
    let mut w = CaptureWriter::new(&mut sink, &world()).expect("writer");
    w.add(&tile(0, 0)).expect("add");
    w.add(&tile(1, 0)).expect("add");
    assert!(sink.1 >= 3);
}
