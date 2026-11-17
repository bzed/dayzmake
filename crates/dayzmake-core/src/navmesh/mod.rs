//! Navmesh generation for DayZ: the data-server wire format, and (in later modules) the tile builder
//! and the `.nm` writer.
//!
//! Feature `navmesh`. The wire format ([`wire`]) is what DayZDiag's navmesh data server sends: world
//! parameters and one geometry record per tile. Every reader treats its input as hostile and returns
//! errors instead of panicking.

pub(crate) mod bytes;
pub mod capture;
pub mod client;
pub(crate) mod dayz;
pub mod detour;
#[doc(hidden)]
pub mod internals;
pub mod islands;
pub(crate) mod num;
pub(crate) mod recast;
pub mod wire;
