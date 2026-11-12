//! dayzmake: DayZ modding tools.
//!
//! File format handling for PBO archives, rapified configs, P3D models, PAA
//! textures and BI signatures, plus the preprocessing/rapification pipeline
//! used by the `dayzmake` binary.

/// Binarization through BI's `binarize.exe` (Windows or Proton).
pub mod binarize;
/// Config parsing and rapification/derapification.
pub mod config;
/// Error types, the `error!` macro and warning bookkeeping.
pub mod error;
/// Evaluator for config `__EVAL` / `__EXEC` expressions.
pub mod eval;
/// I/O helpers for the binary formats.
pub mod io;
/// P3D model reading and inspection.
pub mod p3d;
/// PAA texture conversion.
pub mod paa;
/// PBO archive packing, unpacking and inspection.
pub mod pbo;
/// Config preprocessing (macros, includes, conditionals).
pub mod preprocess;
/// CLI definition and command dispatch.
pub mod run;
/// BI signature (`.bisign`) creation and verification.
pub mod sign;
