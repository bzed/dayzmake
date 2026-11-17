//! Navmesh generation for DayZ: the data-server wire format, and (in later modules) the tile builder
//! and the `.nm` writer.
//!
//! Feature `navmesh`. The wire format ([`wire`]) is what DayZDiag's navmesh data server sends: world
//! parameters and one geometry record per tile. Every reader treats its input as hostile and returns
//! errors instead of panicking.

pub(crate) mod num;
pub mod wire;
