//! The commands behind the CLI and the MCP tools: they read and write files and streams and call
//! into `dayzmake-core`.

/// `dayzmake binarize`.
pub mod binarize;
/// Rapify, derapify and preprocess.
pub mod config;
/// `dayzmake extract`.
pub mod extract;
/// `dayzmake inspect` for models.
pub mod p3d;
/// PAA conversion.
pub mod paa;
/// PBO inspect, cat, pack, build and unpack.
pub mod pbo;
/// Key generation, signing and verification.
pub mod sign;
