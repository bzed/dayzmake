//! Tests of the public `navmesh::testing` module: the fake data server and the synthetic world, used
//! only through the library API (no `#[path]` include).

#![cfg(feature = "navmesh-testing")]
#![expect(
    clippy::expect_used,
    reason = "test code; the workspace clippy.toml exceptions only reach the root crate"
)]

use std::io::Write;
use std::net::TcpStream;

use dayzmake_core::navmesh::client::Host;
use dayzmake_core::navmesh::generate::{GenerateOptions, Progress, Source, generate};
use dayzmake_core::navmesh::testing::{FakeServer, Faults, synthetic_world};
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
