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
| `sign` | yes | the `sign` module (BI keys, `.bisign` creation and verification) |
| `image` | yes | `Paa::from_dynamic_image`, `Paa::to_dynamic_image` and the re-exported `image` crate |

Without them (`default-features = false`) the crate builds without `sign` and `image`. PBO reading
and writing, configs, P3D and PAA reading/writing stay available.

## Usage

See the [API documentation](https://docs.rs/dayzmake-core) for the entry points and examples.

## Licence

GPL-2.0-or-later, inherited from armake2 (KoffeinFlummi). Projects under AGPL or GPLv3 may link it; others
need to be GPL-2.0-or-later compatible.

## Changes

### 0.2.0

Stricter checks, so code that relied on the old leniency can now get errors:

- `BIPrivateKey::generate` refuses key lengths that are not a multiple of 16, below 384 bits or above 16384 bits, and
  `sign` refuses keys below 384 bits.
- Reading a key or a `.bisign` rejects a wrong key blob header (`RSA1`/`RSA2`) or a modulus that is not exactly as wide as the header
  length says (a private key's length must also be a multiple of 16).
- `sign` and `verify` recompute the SHA-1 of the PBO and fail with a checksum-mismatch error when its stored trailer
  differs; `verify` also rejects a signature value that is not smaller than the modulus.
- OpenSSL is no longer used: key generation, signing and verification are pure Rust.
