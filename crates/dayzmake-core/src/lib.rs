//! File format handling for DayZ: PBO archives, rapified configs, P3D models, PAA textures and BI
//! keys and signatures, plus the preprocessor and `__EVAL` evaluator the config pipeline needs.

/// The `image` crate the PAA conversion API uses, so consumers build against the same version.
#[cfg(feature = "image")]
#[cfg_attr(docsrs, doc(cfg(feature = "image")))]
pub use image;

/// Binarization through BI's `binarize.exe` (Windows or Proton).
pub mod binarize;
/// Config parsing and rapification/derapification.
pub mod config;
/// Error types and the `error!` macro.
pub mod error;
/// Evaluator for config `__EVAL` / `__EXEC` expressions.
pub(crate) mod eval;
/// I/O helpers for the binary formats.
pub(crate) mod io;
/// P3D model reading and inspection.
pub mod p3d;
/// PAA texture conversion.
pub mod paa;
/// PBO archive packing, unpacking and inspection.
pub mod pbo;
/// Config preprocessing (macros, includes, conditionals).
pub mod preprocess;
/// BI signature (`.bisign`) creation and verification.
#[cfg(feature = "sign")]
#[cfg_attr(docsrs, doc(cfg(feature = "sign")))]
pub mod sign;
/// Warnings raised by library calls and the sinks that receive them.
pub mod warning;
