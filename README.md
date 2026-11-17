dayzmake
========

[![](https://github.com/bzed/dayzmake/actions/workflows/ci.yml/badge.svg)](https://github.com/bzed/dayzmake/actions/workflows/ci.yml)
[![](https://github.com/bzed/dayzmake/actions/workflows/codeql.yml/badge.svg)](https://github.com/bzed/dayzmake/actions/workflows/codeql.yml)
[![crates.io](https://img.shields.io/crates/v/dayzmake.svg)](https://crates.io/crates/dayzmake)
[![crates.io](https://img.shields.io/crates/v/dayzmake-core.svg)](https://crates.io/crates/dayzmake-core)
[![docs.rs](https://docs.rs/dayzmake-core/badge.svg)](https://docs.rs/dayzmake-core)
[![](https://img.shields.io/badge/license-GPL--2.0--or--later-blue.svg)](LICENSE)

Documentation: <https://bzed.github.io/dayzmake/> (guides and a reference of every command, option, MCP tool and warning).

<!-- ANCHOR: intro -->
DayZ modding tools written in Rust: PBO packing and unpacking, config
preprocessing and rapification, model inspection and binarization, PAA
conversion and signature handling.

## History

dayzmake is a fork of [armake2](https://github.com/KoffeinFlummi/armake2)
itself the successor to armake, diverged from it for DayZ modding: Arma 3
targets are no longer the focus, while DayZ-specific behaviour (binarization
through DayZ Tools under Proton, `product=dayz` header extensions and similar)
is maintained here.

**Status:** `paa2img` and `img2paa` work.
<!-- ANCHOR_END: intro -->

## Changes
<!-- ANCHOR: changes -->

### Unreleased

- `inspect` of an ODOL v56 model leaves out the `Class:` and `Damage:` lines when they are empty and a section (Textures, Materials, Proxies, Properties, Selections, Animations) that has no items; an empty material name was already printed as `(none)` (#96).
- Any config entry may end at the line end instead of `;`, as in CfgConvert; it keeps its value type, as in CfgConvert: `x = 1.5` is a float, no longer the string `"1.5"`, and `x = 1e999` is an infinity (see below). At the very end of a file without a line end such an entry is a syntax error, as in CfgConvert (#95).
- The preprocessor strips a `//` comment at the very end of the input, with no line end after it, as CfgConvert does; before, `x = 1;` followed by `// c` without a final newline was a syntax error. An unterminated `/*` runs to the end of the file, also as in CfgConvert (silently there), and raises the new warning `unterminated-comment` with file and line; an included file's comment ends with that file. When the last line of a config is picked out to decide whether its entry may end at the end of the file, a lone CR now counts as a line break; CfgConvert rejects `x = 1\r#define Y 1` and `x = 1\r// c`, dayzmake accepts them (#103).
- A config float literal that overflows f32 (`1e999`, `-1e39`) is rapified as an infinity, no longer an error, as CfgConvert does; a literal that underflows is 0. `derapify` writes an infinity as `1e999` / `-1e999`, so derapify then rapify keeps the bits (#102).
- Config errors raised while parsing a value (`Invalid integer`, a failing `__EVAL`/`__EXEC`, nesting too deep) carry the `In line <file>:<line>:` context of a parse error (#94).
- `api config <path>` looks up a config class such as `CfgVehicles/Hatchback_02`: its inheritance chain, its own and
  inherited properties each tagged with the class they come from, the classes inside it, its subclasses and the PBOs
  that define it. It indexes every `config.bin` and `config.cpp` of the `dta` and `addons` PBOs of the game (also the
  ones in folders of a PBO) and of the `-m` mods, loads vanilla first and then the mods in order (inside a source by
  `CfgPatches` `requiredAddons`, then sorted path), and merges like the engine: a class defined again extends the
  earlier one, properties overwrite, the base class becomes what the last definition names (none included; the lookup prints `Base changed: A -> (none) by [<source>] <pbo>` and a mod's change raises `api-config-base-changed`), `delete` removes, `+=` appends, `class X;` creates nothing. `--max` cuts the
  output, `--json` prints an object, `--refresh` rebuilds the cached index of each source. An inheritance cycle is an
  error. An `#include` in a mod `config.cpp` is resolved from the entries of the same PBO only, never from the file system (#97). The MCP tool is `api_config`; new warnings `api-config-order`, `api-config-base-changed` and `api-config-limit` (#89).
- `extract [-f] [--derapify] [--only <pattern>]... [-x <pattern>]... <source> <targetfolder>` unpacks every PBO in the
  `dta` and `addons` folders of a DayZ install or mod, or a single PBO, into `<targetfolder>/<prefix>/`, optionally
  derapifying the rapified `.bin` files to `.cpp`, and writes `pbo-map.tsv` (prefix with `/` separators, TAB, PBO path; `\`, TAB, CR and LF are escaped as `\\`, `\t`, `\r`, `\n`). The tree is
  written to a staging folder and swapped in, so a re-run drops stale files and a failure keeps the old tree; `-f`
  only replaces an empty folder or an earlier result (one with a `pbo-map.tsv`) and never one that contains or lies
  inside the source. Entries are streamed, not read whole, and an I/O or decompression error fails the extract. The MCP
  tool is `extract` (#87).
- `api grep <regex>` searches the lines of every `.c` file of vanilla and of the selected mods, straight from the
  PBOs, and prints `[<source>] <prefix>\<path>:<line>: <text>` sorted by source, path and line. `--ignore-case`
  matches case-insensitively, `--max` cuts the output and bounds the memory used (all further matches are still counted for the
  `... N more lines` note) and `--json` prints an array of `{source, path, line, text}`. The MCP tool is
  `api_grep` (#88).
- `api src --context <n>` prints n lines around each declaration and `--lines <a-b>` lines a to b of each file, as
  `<line>| <text>` (with `--json`: `"lines"`); the options exclude each other and `--cat`. The MCP tool `api_source`
  takes `context` and `lines` (#90).
- `unpack` takes `-x <pattern>` (leave entries out) and `--only <pattern>` (write only matching entries); `-x`
  wins. The MCP tool `pbo_unpack` has the arguments `exclude` and `only`. `UnpackOptions` gains `exclude` and
  `only`, and `dayzmake_core::pbo::EntryFilter` is the shared filter (#86).
- `inspect --json <file.pbo>` prints the PBO header as one JSON object: the header extensions, the prefix and the
  entries, each name next to the lowercase hex of its raw bytes, so a prefix with a control character or a byte that
  is not UTF-8 is unambiguous. The MCP tool `pbo_inspect` takes the argument `json`. A model gives an error (#91).
- Writing and signing a PBO keep the stored bytes of header extensions and entry names that a UTF-8 and a windows-1252
  spelling would otherwise swap (#92).
- Config values in exponent notation (`1e-5`, `1.5E+3`, `3.162277e-5`) are rapified as floats (#85), not as strings with an
  `unquoted-string` warning, so derapify then rapify keeps a float a float. Values such as `1e5x` or `1e` stay strings.
  A float that does not fit an f32 (`1e999`, `-1e39`) becomes an f32 infinity, as in CfgConvert (#102); followed by text
  (`1e999x`) it stays an unquoted string, like an integer that does not fit (`99999999999x`, #93), and a float
  entry may end at a closing brace without a semicolon.
- `inspect` and `pbo_inspect` list ODOL v56 models (#84): skeleton, bounding box, LODs with vertex and face counts,
  textures, materials, proxies, properties, selections and animations. Other ODOL versions still show the format
  and version, with a note that only v56 is decoded. A model whose LOD ranges overlap is refused.
- PAA reads ARGB4444, ARGB1555 and AI88 (#83): `paa2img` and `paa_to_image` decode the 19 such textures the game
  ships (LZSS mipmaps with a checksum); a wrong checksum or a cut-short stream is an error. A PNG file named
  `.paa` is reported as a PNG file.
- Config enum blocks (#82): `rapify` accepts `enum { A, B = 5, C };` at any class level (values count up from
  the last explicit one, as in C) and writes the names, lowercase, into the enum table of the `config.bin`;
  `derapify` prints the table as an `enum { name = value, ... };` block at the top. A table that is missing or
  damaged gives no block and no error.

### 3.0.2

With dayzmake-core 0.2.2.


- PAA fixes in dayzmake-core: `Paa` remembers per mipmap whether it is LZO-compressed, so `Paa::write` no longer
  flags a short raw mipmap as compressed (#76) and `Paa::to_dynamic_image` decodes a `Paa` built by
  `Paa::from_dynamic_image` or `Paa::lzo_compress` without a write/read round trip (#78). `Paa::read` drops the
  `GGATSFFO` offset tagg, so a re-written PAA has it once (#75); `Paa::tagg("GGATSFFO")` now returns `None` after
  `read`.
- `Paa::write` and `img2paa` reject a mipmap of 16 MiB or more (its length field has 3 bytes), for example RGBA
  2048×2048 and larger or a DXT5 4096×4096 texture that LZO cannot shrink, instead of writing a wrong length, and `Paa::read`
  keeps an LZO-compressed mipmap compressed, so a read and re-written PAA stays readable (#79).
- `dayzmake cat` accepts `-f` to overwrite its target; the "Use --force" error was already printed for it (#71).
- `img2paa -c` and the MCP tool `image_to_paa` (`compress`) LZO-compress DXT mipmaps wider than 128 pixels (#70).
  dayzmake already did that for every conversion, so the output does not change. New in dayzmake-core:
  `Paa::lzo_compress`.
- `derapify -d` and the MCP tool `config_derapify` (`indent`) set the indentation: spaces and tabs only, empty
  allowed (#69). New in dayzmake-core: `config::WriteOptions` and `Config::write_with`.

### 3.0.1

- Releases carry `SHA256SUMS` and `SHA512SUMS` of all archives with GPG signatures, and the public key
  (`dayzmake-release-key.asc`); see *Verifying downloads*.
- The documentation is published as a web site with guides and a reference of every command, option, MCP tool and
  warning: <https://bzed.github.io/dayzmake/>.
- `pack` no longer warns that models were not binarized; only `build` does (#72, fixed in dayzmake-core 0.2.1).

### 3.0.0

The changes since 2.0.0. Breaking: a missing `#include` no longer searches the working directory (it is only
checked directly; pass `-i` for folders to search), stricter key and PBO checks (see below), and new errors on
hostile config input. `dayzmake` 3.0.0 needs `dayzmake-core` 0.2.0.

- Backslash line continuations count as source lines: in a `#define` and in disabled lines they used to be
  lost, and a macro use no longer shifts the line numbers after it, so warnings and errors point at the right line
  (#65).
- Behaviour change: a missing absolute include no longer walks the whole working directory (#66): the implicit working directory is
  only checked directly (`./DZ/data/x.hpp`), so a file buried below it is no longer found without `-i`. Folders
  given with `-i` are still searched recursively, but the search does not follow directory symlinks that leave the
  folder, and enters no directory twice, and it fails with an error naming the include after 1,000,000 visited entries (the
  author's `P:` drive copy has 109,000).
- Line numbers in warnings and errors stay right after a disabled `#ifdef`/`#ifndef` branch (plain lines, `#define`,
  `#undef`, `#include`, nested branches); they used to drift up by one per disabled line (#63).
- Macro expansion also charges its work: every call of a defined macro costs, so an exponential chain whose
  expansion is empty (`#define E(x)` with `E(A)E(A)` levels) fails like the others; at most 100,000 macro calls
  per line, real configs need at most 34 (#59).
- Macro work is capped per file too: at most 100,000 macro calls over a file and its includes (a 3000-line file of
  `A13` lines, each about 16,000 calls with an empty expansion, took 37 s); real configs under the author's trees
  need at most 174 (#64).
- At most 10,000 `#include`s are processed per `preprocess` call (nested ones and repeated inclusions count), so an
  exponential include fan-out fails at once; real configs under the author's trees use at most 1 (#60).
- Lines in a disabled `#ifdef`/`#ifndef` branch are skipped before any macro expansion, so they cannot raise macro
  errors or cost time (#61).
- All Windows device names are recognised in PBO entry names and output targets: `COM0`-`COM9` and `LPT0`-`LPT9`
  also with the superscript digits (`COM¹`), `CONIN$` and `CONOUT$`, in any case, with or without an extension
  (#62).
- `pack` and `build` recognise their own output by file identity (device and inode, or volume and file index), so
  `M\OUT.PBO` over an existing `m\out.pbo` on a case-insensitive file system is never packed into itself (#37).
- An existing regular file with a device-like name (`NUL`, `COM1`, reached through a verbatim path) is not
  overwritten in place and needs `--force` like any file (#40); a new target with such a name can still be created
  (#45).
- `unpack` warns about a leading separator only for entries it extracts, and `build`'s duplicate-entry warning names
  the stored entry (`config.bin`, `m.p3d`) it kept (#41).
- A closed stderr no longer panics: usage and other errors still end with their normal exit status (1, or 0 with
  only warnings) instead of 101 (#43).
- Macro expansion is capped: a source line that makes expansion copy more than 500,000 tokens, or a file (with its
  includes) more than 16,000,000, is an error naming the macro, so an exponential definition chain (billion
  laughs) fails at once instead of exhausting memory (#57). The counters measure copying work, re-counted at each
  expansion level, not the size of the output. The most that the author's DayZ trees (1596 configs) need is 888
  tokens per line and 749,577 per file.
- The same cap also counts bytes (64 MiB per line, 512 MiB per file), so that `#x` stringification, which
  collapses a large expansion into one token, cannot bypass it (#58). Real configs need at most 12 KB per line
  and 7.3 MB per file.
- Nesting deeper than 128 levels (macro argument parentheses, config classes, config arrays, `__EVAL`
  parentheses and operator chains) is a parse error, not a stack overflow (#54). Parentheses inside macro
  arguments count even in strings, as the preprocessor reads them.
- An include cycle (`#include` of a file already being included) or include nesting deeper than 32 is an error
  naming the chain, not a stack overflow (#53).
- A closed stdout pipe (`dayzmake inspect x.pbo | head -3`) ends the command quietly with exit status 0 and no
  error line (it used to print `Broken pipe` and exit 1, or panic in `api`). A closed pipe on stderr, a target file
  or an external tool is still an error (#39).
- OpenSSL is gone from the dependency tree, tests included (they use `num-bigint` as the independent check): building
  and testing need no system libraries, and the CI and the Dockerfile no longer install `libssl-dev` or `pkg-config`.
  `dayzmake keygen` generates the RSA key pair with the pure-Rust `rsa` crate and the operating system's random number
  generator, and signing and verification run on `rsa` too (blinded, constant-time private-key operation; the result is
  checked against the public key before it is written). Key files and `.bisign` output are byte-identical to before. A
  signature value that is not smaller than the key's modulus is a verification error. Our copies of key components
  read from a key file are wiped from memory when dropped (best effort: the `rsa` crate makes internal copies it does
  not wipe).
- Two misleading errors are fixed: a `.bisign` with a bad header says "Invalid signature file" (not "public key"), and
  verifying with a key other than the one that made the signature says "The signature was made with a different key"
  (names and lengths only) instead of a complaint about the modulus (#56).
- `sign` and key generation refuse keys below 384 bits, the `CryptoAPI` minimum BI's tools and the game need (smaller
  keys could only sign with broken padding). Reading such keys and verifying their signatures still works (#55).
- A `.bisign` file is read with the same checks as a public key: its embedded key must carry the `RSA1` blob header and a
  modulus exactly as wide as the header length says; anything else is a read error (#52).
- Macro expansion has a depth limit on every path (64 levels); a config that nests deeper, or expands a macro into
  itself through its arguments, is an error naming the macro instead of a stack overflow (#51).
- Key files must carry the CryptoAPI blob header (type, key algorithm, `RSA1`/`RSA2` magic) that `dayzmake` and BI's
  tools write; any other header is a read error (#50).
- Keys and `.bisign` files are limited to 16384 bits (288 at the low end); a larger header length is a read error
  (#49).
- A key file whose modulus is not exactly as wide as its header length says, and a private key whose length is not a
  multiple of 16, is rejected when read (#48).
- `verify` and `sign` recompute the SHA-1 of the PBO instead of trusting its stored trailer: a PBO whose bytes
  changed (`config.bin`, files a signature version leaves out, the header) no longer verifies, and a PBO with a
  wrong trailer is not signed. Both fail with a "checksum mismatch" error (#46); repack the PBO
  (`dayzmake pack`/`build`) before signing. BI's `DSCheckSignatures` trusts the stored trailer, `dayzmake verify` does
  not. `inspect` and `unpack` still read such files.
- `dayzmake cat` on an entry that is not in the PBO now exits 1 with an error (it used to exit 0 and print
  `not found`).
- `x = ;` (or a value macro that expands to nothing) now raises the `empty-value` warning; the value is still
  stored as `""` (#19).
- `build .` (and a symlinked addon folder) without `$PBOPREFIX$` uses the folder's name as the prefix instead of
  failing with "Invalid addon directory" (#11).
- Preprocessor parse errors report 1-based line numbers (#12).
- Unpacking a PBO skips entry names with any control character (C1 included), `..` or drive-letter components or
  an empty component (`unsafe-path` warning); a leading separator is stripped instead (`\scripts\x.c` is written as
  `scripts/x.c`, `leading-separator` warning). Modifying a PBO rejects such names. The API index (`api`) is more
  lenient: an entry with a leading or trailing separator is indexed under its normalised name instead of being
  skipped (#13, #21).
- Rewriting a read PBO keeps the packing method of entries it does not know (#14).
- Names, paths and values from untrusted files are shown with control characters escaped in errors, warnings and
  `inspect` listings. `derapify` refuses a class or entry name that cannot be written as config text (#15, #22, #26,
  #28).
- Writing a P3D rejects an inconsistent model (bad selection size, vertex index out of range, a face without 3 or 4
  vertices, a NUL in a name) before the first byte is written; the error names the LOD index (#16).
- A string with an embedded NUL fails rapifying with an error naming the config entry, instead of writing a corrupt
  file (#23).
- Proton binarizing rejects a `$PBOPREFIX$` that would leave its sandbox (#17).
- Output files (`build`, `pack`, `rapify`, `binarize`, `keygen`, ...) are replaced atomically: a failed command
  keeps the old target, an existing target keeps its permissions (with `--force` a read-only target is replaced on unix and stays
  read-only; Windows refuses it and leaves it intact), a symlink is written through, and a device, FIFO
  or pipe (`/dev/null`, `/dev/stdout`, and on Windows `NUL`, `CON`, ...) is written in place. A failure to create the temporary file names the
  directory (#18, #25).
- `build` never packs its own output: the target and its temporary file are left out when they lie inside the
  source folder (#24). The same holds for `pack` and `build` with stdout redirected into the source folder
  (`dayzmake pack m > m/out.pbo`) (#38).
- The `api` warning for a mod with an unsafe prefix shows the prefix escaped once, not twice (#31).
- `keygen` creates the private key readable by its owner only (`0600`) (#20).
- `build` and `pack` skip FIFOs, devices and sockets in the source folder (`unsupported-file` warning, no more hanging
  on a FIFO) and files whose names are unsafe as PBO entry names (`unsafe-path` warning); `-x` excludes apply first.
  Names that collide (case-insensitively, for example `config.cpp` and `config.bin`) give a `duplicate-entry`
  warning; the first in sorted order wins, except that an entry generated from source (the rapified `config.cpp`, a
  `.p3do` packed as `.p3d`) wins over a plain `config.bin` or `.p3d` of the same name (#35). An unreadable source file, such as a dangling symlink, is named in the
  error (#27, #30, #32, #33, #35).
- `build` matches the model extensions `.p3d`, `.rtm` and `.p3do` case-insensitively (`N.P3D` is binarized like
  `n.p3d`; `xp3d` is not a model) (#34).

<!-- ANCHOR_END: changes -->
## Building
<!-- ANCHOR: building -->

The build requires `cargo`, Rust's package manager. No system libraries are needed: everything, including the RSA
code for `sign` and `keygen`, is pure Rust.
To compile and run, use:

```
cargo run
```

To build a release, use:

```
cargo build --release
```

<!-- ANCHOR_END: building -->
## Verifying downloads
<!-- ANCHOR: verifying -->
Every release lists `SHA256SUMS` and `SHA512SUMS` of its archives, each with a detached GPG signature
(`SHA256SUMS.asc`, `SHA512SUMS.asc`). The signing key is

```
dayzmake release signing key (https://github.com/bzed/dayzmake)
ed25519, expires 2029-11-15
6624 41B3 6A57 85AD 18FB  8667 DF75 3D99 D064 EB7E
```

The public key is a release asset (`dayzmake-release-key.asc`) and is published at
<https://bzed.github.io/dayzmake/release-key.asc>. Get it from the site or from a release, and compare the
fingerprint with the one above (and with the one in the repository's `AGENTS.md`) before you trust it.

Download the archive you want together with the checksum files and the key, from the release page or with the
GitHub CLI (without a tag it downloads the latest release):

```
gh release download -R bzed/dayzmake -p '*debian-trixie*' -p 'SHA*' -p 'dayzmake-release-key.asc'
```

On Linux or macOS, in the folder with the downloaded files (the Linux archive is named like `dayzmake-v3.0.2-debian-trixie-x86_64.tar.gz`):

```
gpg --import dayzmake-release-key.asc
gpg --fingerprint 662441B36A5785AD18FB8667DF753D99D064EB7E
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS
```

`gpg --verify` must say `Good signature from "dayzmake release signing key (https://github.com/bzed/dayzmake)"`;
the warning that the key is not certified with a trusted signature is expected as long as you have not
signed it yourself, the fingerprint printed next to it is what counts. `sha256sum -c --ignore-missing` checks
the archives you downloaded and ignores the ones you did not (on macOS use `shasum -a 256 -c --ignore-missing`).
`SHA512SUMS` works the same way with `sha512sum`.

On Windows, in PowerShell with [Gpg4win](https://www.gpg4win.org/) installed, check the signature and compare the
hash of your download with its line in `SHA256SUMS`:

```
gpg --import dayzmake-release-key.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
$file = 'dayzmake-v3.0.2-windows-x86_64.zip'
$expected = ((Select-String -SimpleMatch $file SHA256SUMS).Line -split '\s+')[0]
(Get-FileHash -Algorithm SHA256 $file).Hash -eq $expected.ToUpper()
```

The last line must print `True`.

<!-- ANCHOR_END: verifying -->

## Testing
<!-- ANCHOR: testing -->

Unit tests:

```
cargo test
```

The end-to-end DayZ test harness builds a small test mod, binarizes it, packs
it into a PBO, signs it, and boots a local DayZ server (Steam app 223350, plus
app 1042420 with `--both`) to prove the mod loads and runs:

```
testharness/run.sh --both
```

See [testharness/README.md](https://github.com/bzed/dayzmake/blob/main/testharness/README.md) for details.

<!-- ANCHOR_END: testing -->
## Usage

```
dayzmake

Usage:
    dayzmake rapify [-v] [-f] [-w <wname>]... [-i <includefolder>]... [<source> [<target>]]
    dayzmake preprocess [-v] [-f] [-w <wname>]... [-i <includefolder>]... [<source> [<target>]]
    dayzmake derapify [-v] [-f] [-d <indentation>] [<source> [<target>]]
    dayzmake binarize [-v] [-f] [-w <wname>]... <source> <target>
    dayzmake build [-v] [-f] [--proton-binarize] [-w <wname>]... [-i <includefolder>]... [-x <excludepattern>]... [-e <headerext>]... [-k <privatekey>] [-s <signature>] <sourcefolder> [<target>]
    dayzmake pack [-v] [-f] [-x <excludepattern>]... [-e <headerext>]... [-k <privatekey>] [-s <signature>] <sourcefolder> [<target>]
    dayzmake inspect [-v] [--json] [<source>]
    dayzmake unpack [-v] [-f] [--use-prefix] [-x <excludepattern>]... [--only <pattern>]... <source> <targetfolder>
    dayzmake extract [-v] [-f] [--derapify] [--only <pattern>]... [-x <excludepattern>]... <source> <targetfolder>
    dayzmake cat [-v] [-f] <source> <filename> [<target>]
    dayzmake keygen [-v] [-f] <keyname>
    dayzmake sign [-v] [-f] [--v2] <privatekey> <pbo> [<signature>]
    dayzmake paa2img [-v] [-f] <source> <target>
    dayzmake img2paa [-v] [-f] [-c] [-t <paatype>] <source> <target>
    dayzmake verify [-v] <publickey> <pbo> [<signature>]
    dayzmake api class [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--inherited] [--mods-only] [--json] <name>
    dayzmake api find [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--class <class>] [--json] <regex>
    dayzmake api sub [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--all] [--json] <name>
    dayzmake api enum [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--json] <name>
    dayzmake api chain [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--json] <name>
    dayzmake api src [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--cat] [--context <n>] [--lines <a-b>] [--json] <name>
    dayzmake api grep [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--max <n>] [--ignore-case] [--json] <regex>
    dayzmake api defines [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--json] [<regex>]
    dayzmake api config [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--json] <path>
    dayzmake api update [-v] [--experimental] [--dayz <dir>] [-m <mods>]
    dayzmake api mods [-v] [--experimental] [--dayz <dir>] [--json]
    dayzmake mcp [-v]
    dayzmake (-h | --help)
    dayzmake --version

Commands:
    rapify      Preprocess and rapify a config file.
    preprocess  Preprocess a file.
    derapify    Derapify a config.
    binarize    Binarize a file using BI's binarize.exe (Windows; on Linux through Proton).
                The model's folder is the addon and its name the prefix; use build
                for models whose textures are referenced by a longer prefix.
    build       Build a PBO from a folder.
    pack        Pack a folder into a PBO without any binarization or rapification.
    inspect     Inspect a PBO and list contained files, or a model (MLOD/ODOL v56) and
                show its format, version, LODs, textures, materials and selections.
    unpack      Unpack a PBO into a folder.
    extract     Unpack every PBO of a DayZ install, a mod folder or a single PBO into one tree by prefix,
                optionally derapified, with a prefix -> PBO map (pbo-map.tsv).
    cat         Read the named file from the target PBO to stdout.
    keygen      Generate a keypair with the specified path (extensions are added).
    sign        Sign a PBO with the given private key.
    verify      Verify a PBO's signature with the given public key.
    paa2img     Convert a PAA file to a standard image format (e.g. PNG).
    img2paa     Convert a standard image file to PAA.
    api         Query the DayZ script API from the installed scripts.pbo (vanilla) and,
                with -m, from mods: class, find, sub, enum, chain, src, grep, defines, update;
                config looks up a config class (inheritance chain, resolved properties);
                mods lists the installed mods.
    mcp         Run an MCP server on stdin/stdout (tools for the API index and the file commands).

Options:
    -v --verbose                Enable verbose output.
    -f --force                  Overwrite the target file/folder if it already exists.
    -w --warning <wname>        Warning to disable
    -i --include <includefolder>    Folder to search for includes; the CWD is only checked directly.
    -x --exclude <excludepattern>   Glob pattern to exclude from PBO.
                                      For unpack and extract: pattern of entries to leave out (matched against the
                                      entry path).
    -d --indent <indentation>   String to use for indentation. 4 spaces by default.
    -e --headerext <headerext>  Extension to add to PBO header as "key=value".
    -k --key <privatekey>       Sign the PBO with the given private key.
    -c --compress               LZO-compress DXT mipmaps wider than 128 px (always done; kept for compatibility).
    -t --type <paatype>         PAA compression type (e.g. dxt1, dxt5). [default: dxt5]
    -s --signature <signature>  Signature path to use when signing the PBO.
    --v2                     Generate an older v2 signature.
    --use-prefix                unpack: extract into <targetfolder>/<prefix>/.
    --only <pattern>            unpack, extract: write only the entries that match a pattern (-x wins).
    --derapify                  extract: write every rapified .bin as a .cpp of the same name.
    --proton-binarize           Linux: binarize .p3d/.rtm with DayZ Tools' binarize.exe under Proton.
    --experimental              api: use the experimental branch instead of stable.
    --dayz <dir>                api: DayZ client/server directory (uses <dir>/dta/scripts.pbo).
    -m --mod <mods>             api: also query mods, comma separated: cf, cot, dabs, expansion, aliases
                                (those four, if installed), all (every installed mod with scripts; the
                                first run can take a while: each mod is indexed once, then cached), a
                                workshop id, a mod name or @dir name (see `api mods`) or a directory.
                                Without -m only vanilla is queried.
    --refresh                   api: rebuild the cached index.
    --max <n>                   api: print at most n lines (or JSON items). [default: 120]
    --inherited                 api class: also show the base classes.
    --mods-only                 api class: show only what the mods declare.
    --class <class>             api find: search only the members of this class or enum.
    --all                       api sub: include indirect subclasses.
    --ignore-case               api grep: match case-insensitively.
    --cat                       api src: print the full source files.
    --context <n>               api src: print n lines around each declaration, with line numbers.
    --lines <a-b>               api src: print lines a to b of each file, with line numbers.
    --json                      api: print JSON instead of text.
    -h --help                   Show usage information and exit.
       --version                Print the version number and exit.
```

See `dayzmake --help` for more.

`build` and `pack` pack only regular files (and links to them). Files excluded with `-x` are skipped silently. A file whose name is not valid as a PBO entry name (not UTF-8, control characters, `:`, a Windows device name) is skipped with an `unsafe-path` warning, and FIFOs, devices and sockets with an `unsupported-file` warning. Entry names compare case-insensitively: if two files map to the same name (`A.txt` and `a.txt`, or `config.cpp` and `config.bin` in `build`, where the `.cpp` becomes `config.bin`), the first in sorted order is packed and the other is skipped with a `duplicate-entry` warning naming both; an entry generated from source (the rapified `config.cpp`, a `.p3do`) wins over a plain `config.bin` or `.p3d` of the same name. Model extensions (`.p3d`, `.rtm`) are matched case-insensitively, and `.p3do` files are packed as `.p3d` (a colliding `m.p3d` and `m.p3do` is reported, and the `.p3do` wins). A file that cannot be opened or read fails the build with an error naming it.

## Using dayzmake-core as a library
<!-- ANCHOR: library -->

The file-format code lives in the [`dayzmake-core`](https://crates.io/crates/dayzmake-core) crate
(`crates/dayzmake-core`): PBO, configs, P3D, PAA, BI keys and signatures, with no CLI dependencies. It never
prints; warnings go to a `WarningSink` you provide, so it is safe to use from many threads.

```
cargo add dayzmake-core
```

Features: `sign` (BI keys and signatures) and `image` (PAA from and to images), both on by
default; `default-features = false` builds without them. The
[API documentation](https://docs.rs/dayzmake-core) has a complete example (key, PAA, pack, sign, verify).
The licence is GPL-2.0-or-later, so GPLv3 and AGPLv3 projects may link it. `dayzmake-core` is versioned
independently of the CLI.

<!-- ANCHOR_END: library -->
## Script API index

`dayzmake api` answers questions about the DayZ script API (classes, members, enums, base class chains, where a
declaration lives, compile-time defines) from the installed game, without extracting anything.

```
dayzmake api class PlayerBase --max 40        # members of a class
dayzmake api class PlayerBase --inherited     # with its base classes
dayzmake api find GetGroup -m cf              # regex over members, classes and enum items
dayzmake api sub ManBase --all                # subclasses
dayzmake api enum EStatLevels
dayzmake api chain PlayerBase
dayzmake api src AIAgent                      # file and line of every declaration
dayzmake api src AIAgent --cat                # the full source files
dayzmake api src AIAgent --context 5          # five lines around each declaration
dayzmake api grep 'GetGame\(\)\.GetMission\(\)'   # call sites: a text search over the script bodies
dayzmake api defines [regex]
dayzmake api config CfgVehicles/Hatchback_02   # a config class: chain, resolved properties, subclasses
dayzmake api update                           # rebuild the cache now
```

- **Branch:** stable by default, `--experimental` for the experimental branch. The two branches have separate
  indexes and caches. The install is found through Steam (`STEAM_ROOT` first; on Windows then the registry's `SteamPath`, `%ProgramFiles(x86)%\Steam` and `%ProgramFiles%\Steam`); `--dayz <dir>` names a DayZ client or
  server directory (`<dir>/dta/scripts.pbo`) and skips the Steam search.
- **Mods:** without `-m` only vanilla is queried. `-m cf,cot` (aliases `cf`, `cot`, `dabs`, `expansion`), a Steam
  workshop id, a mod name (`meta.cpp` name or `@dir`, see `dayzmake api mods`) or a mod directory adds mods;
  `-m aliases` means the four aliases that are installed, `-m all` every installed mod that has scripts (the first
  run indexes each mod once, which can take a while; later runs use the cache). A mod contributes the
  scripts of every `addons/*.pbo` (the `.c` files, with the `defines[]` of its `config.cpp`/`config.bin`).
  Results from several indexes are tagged with the index name.
- **Output:** text, at most `--max` lines (default 120). `--json` prints JSON instead (errors stay on stderr as
  text); `-v` first prints one `# <index>: <build>` line per index.
- **Cache:** one JSON file per index in `$DAYZMAKE_CACHE`, else `$XDG_CACHE_HOME/dayzmake/api`, else
  `~/.cache/dayzmake/api` (Windows: `%LOCALAPPDATA%\dayzmake\api`). It is rebuilt automatically when the size or
  modification time of a PBO changes or the index format changes; `--refresh` forces it. A corrupt file is replaced. A cache that cannot be found or written (read-only directory, no `HOME`) is a warning
  (`api-cache`); the query still runs on the freshly built index.
- PBO contents are untrusted: entries and prefixes with unsafe paths are skipped with a warning and nothing is
  extracted to disk.

## MCP server

`dayzmake mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io) server on stdin/stdout
(newline-delimited JSON-RPC, tools only). Register it with Claude Code:

```
claude mcp add dayzmake -- dayzmake mcp
```

Tools:

- `api_class`, `api_find`, `api_subclasses`, `api_enum`, `api_chain`, `api_source`, `api_grep`, `api_defines`,
  `api_config`, `api_update`, `api_mods`: the script API index (see above). All but `api_grep` and `api_mods` take
  `mods`, `experimental`, `dayz` and `refresh`; `api_grep` takes the same without `refresh` (it reads the PBOs and
  builds no index); `api_mods` takes `experimental`, `dayz` and `json`. The queries also take `json`, and all but
  `api_chain` take `max` (default 120); `api_update` takes neither. Without `mods` only vanilla is queried; no
  tool defaults to `all`. The first `mods=all` run builds the caches and can take a while; `api_update` with
  `mods=all` warms them.
- `pbo_inspect`, `pbo_cat`, `pbo_unpack`, `extract`, `pbo_pack`, `pbo_build`, `pbo_sign`, `pbo_verify`, `key_generate`,
  `config_rapify`, `config_derapify`, `config_preprocess`, `paa_to_image`, `image_to_paa`, `binarize`: the file
  commands with the arguments of their CLI counterparts. No tool overwrites a file unless `force` is set.

The tools take file paths as given (relative to the server's working directory) and act with the permissions of
the user running the server: they can read, create and, with `force`, overwrite any file that user can. Only
register the server with a client you trust to do that. Output is capped at 200 000 bytes; `api_source` (with `cat`) and `pbo_cat` take `offset` and `length`
(bytes) to fetch the rest, and `api_source` takes `target` to write the files instead. Warnings come first in
the result, so a cut never drops them, and failures are results with `isError: true`.

## Inspecting models and checking their textures

`dayzmake inspect <file.p3d>` shows the format and version of a model. For an editable MLOD it also lists the LODs, textures, materials, properties and selections. For a binarized ODOL version 56 (the version of the game's models) it lists the skeleton, the bounding box, the LODs with vertex and face counts, textures, materials, proxies, properties, selections and animations; a damaged v56 model is an error. For other ODOL versions and for BMTR only the format and version are shown, with a note that only v56 is decoded. Without a model header, `inspect` treats the input as a PBO.

`build` checks every MLOD model it is about to binarize (also without `--proton-binarize`) and warns about textures and materials that are not in the addon (`missing-texture`, `missing-material`; mute with `-w`). References starting with the addon's prefix are looked up in the source folder, ignoring case. Other references (vanilla `dz\...`, other mods) are only checked if they exist below an `-i` folder or the current directory; if they don't, they are not reported because there is no way to tell whether they are wrong. Procedural textures (`#(argb,...)`) are skipped.

## Binarizing models on Linux

BI's `binarize.exe` only runs on Windows, so on Linux `build` copies `.p3d`/`.rtm` files as-is (with the `non-windows-binarization` warning). With `--proton-binarize` (and for `dayzmake binarize`) dayzmake runs the DayZ Tools `binarize.exe` under Proton instead. Config and rvmat rapification is unchanged. Files that are already binarized (`ODOL` models, `BMTR` animations) never reach `binarize.exe`; they are copied as-is with the `prebinarized-file` warning, which names the format and version (e.g. `ODOL v56`). Any version is accepted; dayzmake does not know which ones the game loads.

Requirements: DayZ Tools (Steam app 830640) and any Proton installed through Steam.

- `STEAM_ROOT`: Steam directory. Default: first of `~/.steam/debian-installation`, `~/.steam/steam`, `~/.local/share/Steam` that has `steamapps/common`.
- `DAYZ_TOOLS`: DayZ Tools directory. Default: `<steam>/steamapps/common/DayZ Tools` (uses `Bin/Binarize/binarize.exe`).
- `PROTON`: Proton directory (containing the `proton` script). Default: `Proton Hotfix`, `Proton - Experimental`, `Proton Experimental`, then any `Proton*` in `steamapps/common`.
- `BIOUTPUT=1`: show the output of Proton/binarize.exe.

dayzmake builds a throw-away sandbox in the temp directory (a copy of `Binarize`, a fresh Wine prefix and an output folder) once per run and removes it at the end. Wine's default `Z:` drive maps to `/`, and `binarize.exe` walks directory trees from the drive root, so it would run for minutes on a real home directory; the sandbox therefore points `Z:` at itself. The first model of a run is slow because the Wine prefix has to be created (up to a few minutes on a cold start). Nothing in the Steam installation is modified.

`binarize.exe` resolves texture and material paths (`my\mod\data\x_ca.paa`) relative to the project root, and only sets a model's alpha flags if it can read the texture. dayzmake therefore gives the sandbox a `P:` drive with the addon linked at its PBO prefix (`P:\my\mod` → the source folder) and runs `binarize.exe` from `P:\`. Paths outside the addon (vanilla `dz\...`) stay unresolved and are reported as `binarize-missing-texture`. The source folder is linked, not copied, so `binarize.exe` can read and in principle write it; its output goes to the sandbox, and the source is not modified.

`dayzmake binarize <source> <target>` has no PBO to take the prefix from: it treats the model's folder as the addon and uses the folder name as the prefix. That only works for a model in the addon root of a single-component prefix; for anything else (a model in `data\`, or a prefix like `my\mod`) use `build --proton-binarize`, which takes the prefix from `$PBOPREFIX$`.
