//! Test support for programs that call [`generate`](crate::navmesh::generate::generate) with
//! [`Source::Servers`](crate::navmesh::generate::Source::Servers).
//!
//! Feature `navmesh-testing`. The module has a fake navmesh data server ([`FakeServer`]) with scripted
//! faults ([`Faults`], [`Fault`]) and a small synthetic world ([`synthetic_world`]): 3 x 3 tiles with
//! ground, a pond, a crate, a wall, a room, trees and a ladder.
//!
//! The server binds `127.0.0.1:0`, so tests do not need a fixed port. The frame format stays internal
//! to this crate. Use the fake server in your tests. Do not copy its wire code.
//!
//! ```
//! # #[cfg(feature = "navmesh-testing")]
//! # fn main() -> std::io::Result<()> {
//! use dayzmake_core::navmesh::testing::{FakeServer, Faults, synthetic_world};
//!
//! let (params, tiles) = synthetic_world();
//! let server = FakeServer::start(params, tiles, Faults::none())?;
//! assert!(server.addr().port() != 0);
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "navmesh-testing"))]
//! # fn main() {}
//! ```

pub mod server;
pub mod world;

pub use server::{FakeServer, Fault, Faults};
pub use world::synthetic_world;
