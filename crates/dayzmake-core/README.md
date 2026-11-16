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

### Unreleased

- Config values in exponent notation (`1e-5`, `1.5E+3`, `3.162277e-5`) are rapified as floats (#85), not as strings with an
  `unquoted-string` warning, so derapify then rapify keeps a float a float. Values such as `1e5x` or `1e` stay strings.
  A float that does not fit an f32 (`1e999`, `-1e39`) is an error, never infinity or NaN; followed by text
  (`1e999x`) it stays an unquoted string, and a float entry may end at a closing brace without a semicolon.
- `odol::Odol::read` reads binarized ODOL v56 models (#84): model properties, skeleton, animation classes and, per
  LOD, vertex and face counts, textures, materials, proxies, selections and properties. Only v56 is decoded,
  other versions are an error, and LOD ranges that overlap are refused. New dependency: `lzokay-native`
  (decompression only).
- `Paa::read` accepts the formats `0x4444` (`"ARGB4444"`), `0x1555` (`"ARGB1555"`) and `0x8080` (`"AI88"`) and
  `Paa::to_dynamic_image` decodes them to RGBA (#83); `Paa::write` writes their stored bytes back unchanged. A
  wrong checksum or a cut-short LZSS stream is an error, and a PNG file gives an error that names the PNG.
- `Config` reads, keeps and writes the enum table of a rapified config (#82): the parser accepts `enum { ... };`
  blocks, `Config::enum_values` returns the ordered `(lowercase name, value)` pairs, and `Config::to_string` prints
  them as an `enum` block first. A bad table offset or a cut-short table is not an error.

### 0.2.2

- PAA fixes in dayzmake-core: `Paa` remembers per mipmap whether it is LZO-compressed, so `Paa::write` no longer
  flags a short raw mipmap as compressed (#76) and `Paa::to_dynamic_image` decodes a `Paa` built by
  `Paa::from_dynamic_image` or `Paa::lzo_compress` without a write/read round trip (#78). `Paa::read` drops the
  `GGATSFFO` offset tagg, so a re-written PAA has it once (#75); `Paa::tagg("GGATSFFO")` now returns `None` after
  `read`.
- `Paa::write` and `img2paa` reject a mipmap of 16 MiB or more (its length field has 3 bytes), for example RGBA
  2048×2048 and larger or a DXT5 4096×4096 texture that LZO cannot shrink, instead of writing a wrong length, and `Paa::read`
  keeps an LZO-compressed mipmap compressed, so a read and re-written PAA stays readable (#79).
- `Paa::lzo_compress` LZO-compresses the DXT mipmaps wider than 128 pixels that are still raw (#70).
  `Paa::from_dynamic_image` is unchanged: it already compresses them.
- `config::WriteOptions` and `Config::write_with` write a config with a chosen indentation (#69). `Config::write`
  is unchanged (four spaces).

### 0.2.1

- `PBO::from_directory` with `PackOptions::pack()` no longer raises the `non-windows-binarization` warning for
  models; only a build that binarizes warns (#72).

### 0.2.0

The first published release.

Stricter checks, so code that relied on the old leniency can now get errors:

- `BIPrivateKey::generate` refuses key lengths that are not a multiple of 16, below 384 bits or above 16384 bits, and
  `sign` refuses keys below 384 bits.
- Reading a key or a `.bisign` rejects a wrong key blob header (`RSA1`/`RSA2`) or a modulus that is not exactly as wide as the header
  length says (a private key's length must also be a multiple of 16).
- `sign` and `verify` recompute the SHA-1 of the PBO and fail with a checksum-mismatch error when its stored trailer
  differs; `verify` also rejects a signature value that is not smaller than the modulus.
- OpenSSL is no longer used: key generation, signing and verification are pure Rust.

New public items: `preprocess::IncludeFolders` (folders searched recursively plus folders that are only checked
directly), `preprocess::preprocess_in`, `Config::read_in`, `Config::from_string_in` and `PackOptions::direct_include_folders`. The recursive
search of an include folder no longer follows directory symlinks that leave the folder, and enters no directory twice, and it
fails with an error naming the include after visiting 1,000,000 entries. `preprocess` and `Config::read` keep
their signatures and search every folder they get recursively.

New public items: the `fileid` module (`FileId::of`, `FileId::stdout`, `is_same_file`) compares files by identity
(device and inode, or volume and file index) instead of by name; `PackOptions::exclude_stdout` leaves out a
file that is the redirected stdout, and `PackOptions::exclude_paths` now matches by file identity too, so the
own output is never packed on case-insensitive file systems.

New errors on hostile input, all naming the offending item: nesting deeper than 128 levels (macro arguments,
config classes and arrays, `__EVAL`); an include cycle or include nesting deeper than 32; more than 10,000
`#include`s per `preprocess` call; macro expansion above 500,000 tokens, 64 MiB or 100,000 macro calls per line, or
16,000,000 tokens, 512 MiB or 100,000 macro calls per file (per `preprocess` call, includes counted). Lines in a disabled `#ifdef` branch are no longer expanded.
`pbo::is_device_name` recognises the full list of Windows device names.
