//! Navmesh generation for DayZ: the data-server client and wire format, captures, the Recast/Detour tile
//! builder, the `.nm` reader and writer, and the generator that ties them together ([`generate`]).
//!
//! Feature `navmesh`. The wire format ([`wire`]) is what DayZDiag's navmesh data server sends: world
//! parameters and one geometry record per tile. Every reader treats its input as hostile and returns
//! errors instead of panicking.

pub mod build;
pub(crate) mod bytes;
pub mod capture;
pub mod client;
pub(crate) mod dayz;
pub mod detour;
pub mod generate;
#[doc(hidden)]
pub mod internals;
pub mod islands;
pub(crate) mod links;
pub(crate) mod num;
pub(crate) mod recast;
pub mod wire;
