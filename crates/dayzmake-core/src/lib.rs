//! File format handling for DayZ: PBO archives, rapified configs, P3D models, PAA textures and BI
//! keys and signatures, plus the preprocessor and `__EVAL` evaluator the config pipeline needs.
//!
//! # Modules
//!
//! - [`pbo`]: read, build, pack, unpack and selectively read PBO archives ([`pbo::PBO`]).
//! - [`config`], [`preprocess`]: config parsing, rapification and derapification.
//! - [`p3d`]: MLOD model reading and inspection (ODOL is only detected there).
//! - [`odol`]: binarized ODOL v56 models, read for what a listing needs (LODs, textures, materials, proxies).
//! - [`paa`]: PAA textures, with conversion from and to images (feature `image`).
//! - `sign`: BI keys and `.bisign` signatures (feature `sign`).
//! - [`binarize`]: BI's `binarize.exe`, natively on Windows or under Proton.
//! - [`warning`]: the warnings library calls raise.
//!
//! The library never prints and never exits the process; every reader treats its input as hostile and
//! returns errors instead of panicking.
//!
//! # Warnings
//!
//! Calls that can warn take a `&mut dyn` [`warning::WarningSink`]. [`warning::Warnings`]
//! is the ready-made sink: it collects [`warning::Warning`]s (and plain notes), can mute
//! warnings by name and caps repeats. Warnings are never global: give every thread or job its own sink and
//! its warnings stay with it.
//!
//! # Binarizing
//!
//! [`binarize::Binarizer`] owns the Proton sandbox, if any. Pass it to
//! [`pbo::PBO::from_directory`] and call [`binarize::Binarizer::close`] when done so the
//! sandbox is removed and its warnings reach your sink; dropping it also cleans up, but cannot report.
//!
//! # Features
//!
//! - `sign` (default): the `sign` module.
//! - `image` (default): `Paa::from_dynamic_image`, `Paa::to_dynamic_image` and the
//!   re-exported `image` crate.
//!
//! # Licence
//!
//! GPL-2.0-or-later, inherited from armake2. Projects under GPLv3 or AGPL-3.0 may link it.
//!
//! # Example
//!
//! Generate a key, convert an image to PAA, pack and sign a mod folder, and verify the result:
//!
//! ```
//! # #[cfg(all(feature = "sign", feature = "image"))]
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use std::fs::{self, File};
//! use dayzmake_core::image::{DynamicImage, RgbaImage};
//! use dayzmake_core::paa::{Paa, PaaFormat};
//! use dayzmake_core::pbo::{PBO, PackOptions};
//! use dayzmake_core::sign::{BIPrivateKey, BISignVersion, signature_path};
//! use dayzmake_core::warning::Warnings;
//!
//! let tmp = tempfile::tempdir()?;
//! let stage = tmp.path().join("stage");
//! fs::create_dir_all(stage.join("data"))?;
//!
//! // A satellite tile as DXT1.
//! let image = DynamicImage::ImageRgba8(RgbaImage::new(64, 64));
//! let paa = Paa::from_dynamic_image(&image, PaaFormat::Dxt1)?;
//! paa.write(&mut File::create(stage.join("data/s_0.paa"))?)?;
//!
//! // Pack, sign, verify. Each job has its own warnings.
//! let mut warnings = Warnings::new();
//! let pbo = PBO::from_directory(&stage, &PackOptions::pack(), None, &mut warnings)?;
//! let key = BIPrivateKey::generate(1024, "example".to_string())?;
//! let signature = key.sign(&pbo, BISignVersion::V3)?;
//! let pbo_path = tmp.path().join("example.pbo");
//! pbo.write(&mut File::create(&pbo_path)?)?;
//! signature.write(&mut File::create(signature_path(&pbo_path, key.name())?)?)?;
//! key.to_public_key()?.verify(&pbo, &signature)?;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "sign", feature = "image")))]
//! # fn main() {}
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]

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
/// File identity (the same file under different spellings).
pub mod fileid;
/// I/O helpers for the binary formats.
pub(crate) mod io;
/// Bohemia's LZSS compression.
pub(crate) mod lzss;
/// Binarized ODOL v56 model reading.
pub mod odol;
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
