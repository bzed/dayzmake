//! The DayZ `.nm` navmesh set file and its Detour tile layout (spec 8.1, 9 and 7.2).
//!
//! A `.nm` file ([`NavMeshSet`]) is a header, an offset table and tile records. Each record holds one
//! Detour tile blob ([`TileData`]): standard Detour tile data (version 12, 64-bit polygon refs) with a
//! 4-byte header extension, 44-byte off-mesh connections and a trailing extra section
//! ([`TileExtra`]). All integers and floats are little-endian.
//!
//! Every reader treats its input as hostile: counts are checked with checked arithmetic against the
//! bytes available before anything is allocated, and errors are returned instead of panics.

mod common;
mod create;
mod mesh;
mod query;
mod set;
mod tile;

pub use create::{CreateParams, OffMeshInput, create_tile_data};
pub use mesh::NavMesh;
pub use set::{
    NavMeshParams, NavMeshSet, SET_MAGIC, SET_VERSION, SET_VERSION_OLD, SetTile, decode_poly_ref,
    encode_poly_ref,
};
pub use tile::{
    BvNode, Link, MeshHeader, NAVMESH_MAGIC, NAVMESH_VERSION, Obstacle, OffMeshConnection, Poly,
    TileData, TileExtra, TilePart, TilePartPoly,
};
