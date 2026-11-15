# dayzmake-core

[![crates.io](https://img.shields.io/crates/v/dayzmake-core.svg)](https://crates.io/crates/dayzmake-core)
[![docs.rs](https://docs.rs/dayzmake-core/badge.svg)](https://docs.rs/dayzmake-core)

Readers and writers for the DayZ file formats: PBO archives, rapified configs (with the preprocessor and
`__EVAL`), P3D models, PAA textures, and BI keys and signatures. It is the library behind the
[`dayzmake`](https://crates.io/crates/dayzmake) command line tool, a fork of armake2.

Every reader treats its input as hostile: errors instead of panics, checked arithmetic, bounded recursion.

## Features

| Feature | Default | Enables |
|---|---|---|
| `sign` | yes | the `sign` module (BI keys, `.bisign` creation and verification); needs OpenSSL |
| `image` | yes | `Paa::from_dynamic_image`, `Paa::to_dynamic_image` and the re-exported `image` crate |

Without them (`default-features = false`) the crate builds without OpenSSL and without `image`. PBO reading
and writing, configs, P3D and PAA reading/writing stay available.

## Usage

See the [API documentation](https://docs.rs/dayzmake-core) for the entry points and examples.

## Licence

GPL-2.0-or-later, inherited from armake2 (KoffeinFlummi). Projects under AGPL or GPLv3 may link it; others
need to be GPL-2.0-or-later compatible.
