//! Tests of the public `navmesh::testing` module: the fake data server and the synthetic world, used
//! only through the library API (no `#[path]` include).

#![cfg(feature = "navmesh-testing")]
#![expect(
    clippy::expect_used,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use std::io::{Read, Write};
use std::net::TcpStream;

use dayzmake_core::navmesh::client::Host;
use dayzmake_core::navmesh::generate::{GenerateOptions, Progress, Source, generate};
use dayzmake_core::navmesh::testing::{FakeServer, Fault, Faults, synthetic_world};
use dayzmake_core::warning::Warnings;

/// Generates the synthetic world from `server` with two builders and checks the report.
fn generate_from(server: &FakeServer) {
    let host = Host::parse(&server.addr().to_string()).expect("host");
    let mut options = GenerateOptions::default();
    options.threads = Some(2);
    let report = generate(
        Source::Servers(vec![host]),
        &options,
        &mut |_: &Progress| {},
        &mut Warnings::new(),
    )
    .expect("generate");
    assert_eq!(report.built, 9, "{:?}", report.failed);
    assert_eq!(report.failed.len(), 0);
    assert_eq!(report.fetch_errors.len(), 0);
}

#[test]
fn generate_against_the_public_fake_server_builds_the_synthetic_world() {
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::none()).expect("start");
    generate_from(&server);
    assert!(server.connections() >= 1);
}

#[test]
fn fake_server_survives_a_garbage_client() {
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::none()).expect("start");
    {
        let mut stream = TcpStream::connect(server.addr()).expect("connect");
        let mut frame = u32::MAX.to_le_bytes().to_vec();
        frame.extend_from_slice(&3u32.to_le_bytes());
        frame.extend_from_slice(&[0xde, 0xad, 0xbe]);
        stream.write_all(&frame).expect("write");
    }
    generate_from(&server);
}

/// Sends one frame: `u32 length` (payload + 4), `u32 id`, payload.
fn send(stream: &mut TcpStream, id: u32, payload: &[u8]) {
    let len = u32::try_from(payload.len() + 4).expect("small");
    let mut frame = len.to_le_bytes().to_vec();
    frame.extend_from_slice(&id.to_le_bytes());
    frame.extend_from_slice(payload);
    stream.write_all(&frame).expect("write");
}

#[test]
fn fake_server_survives_a_tile_request_at_the_edge_of_the_coordinate_range() {
    let (params, tiles) = synthetic_world();
    let server = FakeServer::start(params, tiles, Faults::always(Fault::WrongTile)).expect("start");
    {
        let mut stream = TcpStream::connect(server.addr()).expect("connect");
        let mut payload = i32::MAX.to_le_bytes().to_vec();
        payload.extend_from_slice(&0i32.to_le_bytes());
        send(&mut stream, 3, &payload);
        // The server closes this connection: it has no such tile.
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).expect("read");
        assert_eq!(rest, Vec::<u8>::new());
    }
    // A new connection still gets the world parameters (message 4).
    let mut stream = TcpStream::connect(server.addr()).expect("connect");
    send(&mut stream, 2, &[]);
    let mut head = [0u8; 8];
    stream.read_exact(&mut head).expect("reply");
    assert_eq!(u32::from_le_bytes([head[4], head[5], head[6], head[7]]), 4);
}
