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

- Parsing a config nested to the depth limit (128 levels) fits a 1 MiB stack again on Windows debug builds: the recursive grammar rules for classes and arrays have smaller frames (about 410 KiB for the deepest class nesting instead of 920 KiB). The behaviour and the limits are unchanged (#108).
- New `PBO::read_entries_tolerant(input, wanted, &TolerantRead)` (a truncated or undecodable selected entry is an error of its own, and the selection is bounded by the byte limit of the new `#[non_exhaustive]` options struct `TolerantRead::new(max_bytes)`), `SelectedEntry::into_data`, `SelectedEntry::is_over_limit` and `PboSelection::into_entries`; `read_entries` is unchanged (#99).
- New `MemoryIncludes::with_unreadable` and `MemoryIncludes::get`: an include of an entry that could not be read fails with the reason instead of "not found". `MemoryIncludes` resolves `..` inside the PBO, against the folder of the including entry; a path that leaves the PBO's root is still an error (#99).
- `PBO::insert_entry` over an entry whose stored name is not UTF-8 (windows-1252) stores the raw bytes of the new name; it kept the old entry's bytes (part of #98).
- `extract`: a new target inside the source is refused without `force` too, before anything is created; a file/folder clash across entries skips the later entry with the warning `extract-path-clash` instead of failing (part of #98).
- `Config::read`/`from_string`: enum values are computed like CfgConvert, in `f32` (#107): a literal is rounded to `f32` and a value outside `i32` is `-2147483648` (`A = 2147483647`, `2147483648`, `99999999999999999999` and `-2147483649` all give it), hex keeps its low 32 bits at any length (`0x1FFFFFFFFF` is `-1`) and `0X` works, and an implicit value is the last explicit value plus its distance, added in `f32` (`A = 16777217, B` gives `16777216` twice; `A = 16777216, B, C` gives `16777216`, `16777216`, `16777218`). Such values were an error. `__EVAL` of large numbers keeps its exact floats; CfgConvert's imprecise low bits (`__EVAL(2^31)` is `0x4effffe4` there) are not copied.
- `Config::read`/`from_string`: a config value entry (not an array or a class) may end at the line end instead of `;` and keeps its value type (CfgConvert does the same) instead of becoming a string; the last entry of a file without a line end after it is a parse error (#95).
- `Config::read`/`from_string`: a float literal that overflows f32 (`1e999`, `-1e39`) is an infinity, not an error (CfgConvert does the same); `Config::to_string` writes an infinity as `1e999` / `-1e999`, which reads back to the same f32 (#102).
- `preprocess`: a `//` comment at the very end of the input (no line end after it) is stripped, and an unterminated `/*` runs to the end of the file (an include's ends with the include), as in CfgConvert, with a new warning `unterminated-comment`; before, the comment text reached the grammar (#103).
- `Config::read`/`from_string`: a decimal integer literal that does not fit 64 bits is a float, as in CfgConvert, at a line end and before `;`; in an array one that does not fit 32 bits is a float too. It was an error. A parse error no longer brings the `unquoted-string` warning of the failed attempt on the error's own line; warnings of earlier lines are kept (part of #104).
- New `ConfigEntry::Int64Entry(i64)` (the enum is `#[non_exhaustive]`): the rapified reader reads entry subtype 6 (it was an error), `Config::to_string` writes it as an integer literal and the writer rapifies a decimal literal that fits `i64` but not `i32` as subtype 6, as CfgConvert does. Arrays have no 64-bit element type, so such a literal is a float there. A hex literal without a sign keeps its low 32 bits (`0x1FFFFFFFFF` is `-1`) and one with a sign is a float (`-0x10` is `-16.0`), as in CfgConvert (#106).
- Config errors raised from a grammar action (a failing `__EVAL`/`__EXEC`, nesting too deep) start with `In line <file>:<line>:` like a parse error (#94).
- New `preprocess::MemoryIncludes` and `IncludeFolders::with_memory`: includes come from in-memory files only (a PBO's entries, relative to the including entry or as `\prefix\path`); absolute and drive paths are errors (and `..` in the `\prefix\path` form; in the relative form `..` is resolved inside the PBO since #99) and the file system is never read (#97). `IncludeFolders` has a new public field `memory`.
- `extract`: the `extract-path-clash` warning names the skipped entry, says whether it is a file or a folder and what the other PBO has at that path, with `/` separators on every platform; a source that cannot be resolved is an error of its own (part of #104).
- `extract`: the `pbo-map.tsv` prefix is written with `/` instead of `\`, and `\` is escaped as `\\` in both columns (#87).
- New module `extract`: `extract(source, target, &ExtractOptions, warnings)` unpacks an install, a mod or a PBO into one tree by prefix, with `ExtractOptions` (`force`, `derapify`, `only`, `exclude`), a `pbo-map.tsv` and an atomic swap; it streams entries, fails on I/O and decompression errors, and returns an `ExtractSummary` with `pbos()` and `files()` (#87). `force` only replaces an empty folder or an earlier extract result, never a folder that contains or lies inside the source.
- `UnpackOptions` gains `exclude` and `only` (glob patterns); `pbo::EntryFilter::new(only, exclude)` and `matches(name)` apply them; `exclude` wins (#86).
- `EntryHeader` gains `timestamp` and `name_bytes`, and `PBO::header_extensions_bytes` returns the stored bytes of the header extensions (#91).
- `PBO::write` and signing use the stored bytes of each header extension by position, so a UTF-8 and a windows-1252 spelling of one string no longer swap (#92).
- Config values in exponent notation (`1e-5`, `1.5E+3`, `3.162277e-5`) are rapified as floats (#85), not as strings with an
  `unquoted-string` warning, so derapify then rapify keeps a float a float. Values such as `1e5x` or `1e` stay strings.
  A float that does not fit an f32 (`1e999`, `-1e39`) becomes an f32 infinity, as in CfgConvert (#102); followed by text
  (`1e999x`) it stays an unquoted string, like an integer that does not fit (`99999999999x`, #93), and a float
  entry may end at a closing brace without a semicolon.
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
