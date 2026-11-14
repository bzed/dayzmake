# Security review of untrusted-input code (2026-11)

Untrusted data: any PBO, rapified config, PAA, P3D, `.bikey`, `.biprivatekey`, `.bisign`, and every
name, prefix and header extension inside them. Out of scope: `binarize.exe` itself and building a PBO
from an untrusted source folder (the source folder is the author's own).

Method: `tests/untrusted.rs` mutates every seed (300 mutants per seed in `make test`, 20 000 with
`--ignored`; xorshift64 seed `0x9E3779B97F4A7C15`) and feeds it to the matching reader, then every
reader of untrusted data in `src/` was read for unchecked arithmetic, file-sized allocations,
recursion, file-supplied offsets, slicing, paths built from file content, output outside the target,
predictable temp files and external processes.

Format, one line per finding: `file:line — issue — fixed in <commit/this task> | accepted because <reason>`.
Line numbers are those of the commit that added this document.

## Findings

src/pbo.rs:819 — `--use-prefix` with an unsafe prefix (`..`, absolute, drive, dot-only component) could unpack outside the target folder — fixed in Task 1.1 (`safe_relative_path`)
src/pbo.rs:850 — entry names could leave the target folder, or be written through a symlink in the target tree — fixed in Task 1.1 (`safe_relative_path`, `crosses_symlink`)
src/config.rs:335 — rapified config recursion on self-referencing or deeply nested class bodies — fixed in Task 1.2 (depth and cycle checks); exponential fan-out was only partly fixed there: the entry budget (4 000 000) bounds the entries but not the bytes, and a 1.4 KB file with 20 levels of two class entries sharing one body, ending in 1 KB strings, derapified to 1.5 GB — fixed in the final review (`ReadBudget::seen_bodies`: a body offset that was read once is rejected with "Rapified class body shared at offset N."; real configs never share bodies; tests `rapified_shared_class_body_fan_out_is_an_error`, corpus seed `shared-body-fan-out.bin`)
src/pbo.rs:151 — LZSS allocation sized by the entry's `original_size` — fixed in Task 1.2 (at most 9x the compressed size)
src/paa.rs:32 — PAA tagg size, mipmap dimension and LZO ratio unbounded — fixed in Task 1.2
src/pbo.rs:205 — pathological glob patterns — fixed in Task 1.2 (`matches_glob` rewrite)
src/pbo.rs:750 — `crosses_symlink` failed open on any `symlink_metadata` error other than NotFound (e.g. PermissionDenied) — fixed in this task (only NotFound ends the walk, any other error counts as unsafe; test `crosses_symlink_fails_closed_when_the_path_cannot_be_inspected`)
src/paa.rs:35 — no limit on the number of mipmaps — fixed in this task (at most 16; real PAAs have at most 16, 16384 down to 1 pixel is 15; test `paa_too_many_mipmaps_is_an_error`)
src/paa.rs:39 — no limit on total decompressed mipmap size, a 1.5 MiB file could make the reader hold about 400 MiB (256x its size) — fixed in this task (256 MiB total, checked before each allocation; test `paa_lzo_expansion_is_bounded_in_total`)
src/paa.rs:99 — the 24-bit mipmap size field preallocated up to 16 MiB per mipmap before reading — fixed in this task (bounded `take` read, no preallocation from the field)
src/paa.rs:264 — `to_dynamic_image` panicked in texpresso on a DXT mipmap shorter than its dimensions need (found by the mutation test on `glass_ca.paa` and `solid_co.paa`), and zero dimensions were not rejected — fixed in this task (size and zero checks; tests `paa_short_dxt_mipmap_is_an_error_not_a_panic`, `paa_zero_width_decode_is_an_error_not_a_panic`)
src/paa.rs:264 — `to_dynamic_image` allocated 4 bytes per pixel with no bound: a 16384 x 16384 DXT1 LZO mipmap (128 MiB, from about 600 KB) passes `read`, then decoding allocated 1 GiB — fixed in this task (decoded RGBA must not exceed 256 MiB, i.e. at most 8192 x 8192 pixels; test `paa_decoded_size_is_bounded`)
src/sign.rs:109 — the key name from the key file went into the default signature path via `Path::set_extension`, which panics on `/` or `\` and on Windows could steer the file elsewhere — fixed in this task (`default_signature_path` rejects separators, `:`, control characters and an empty name; test `sign_rejects_a_key_name_with_a_path_separator`)
src/error.rs:179 — names, header extensions, texture, material, property and selection names from a file were printed raw (pbo.rs:651, pbo.rs:850, p3d.rs:547, p3d.rs:632, binarize.rs:403), so a file could inject terminal escape sequences — fixed in this task (`printable` escapes control characters; test `inspect_and_unpack_do_not_print_terminal_escapes`); `BIOUTPUT=1` still passes binarize.exe output through unchanged because the user asked for the raw output
src/pbo.rs:707 — Windows device names (`CON`, `NUL.txt`, `COM1`, ...) would open a device instead of creating a file on Windows — fixed in this task (rejected on every platform; test `safe_path_rejects_windows_device_names`)
src/pbo.rs:819 — `--use-prefix` with an empty prefix was an error; decision: an empty prefix, or one of only separators, means "no prefix" and extracts into the output folder (a PBO without a prefix already behaves that way, nothing can escape from it, a non-empty unsafe prefix is still an error) — fixed in this task (test `unpack_use_prefix_with_empty_prefix_extracts_into_target`)
src/pbo.rs:271 — `PBO::read` holds the whole PBO in memory and the header list is bounded only by the file size — accepted because every allocation is bounded by bytes actually read (`take` + `read_to_end`, no preallocation from sizes), so memory is at most about the file size
src/pbo.rs:598 — two entries with the same name: the map keeps the last data while `file_data` uses the first header — accepted because it only gives wrong bytes for that name, and `lzss_decompress` is bounded by the data it gets; `cmd_unpack` skips the second path anyway
src/pbo.rs:850 — a file or directory swapped for a symlink between the `crosses_symlink` check and `File::create` (time of check/time of use) — accepted because it needs a second local process writing into the target folder during unpacking, and that folder is the user's own
src/pbo.rs:850 — the target folder itself, or a parent of it, may be a symlink — accepted because it is the folder the user named
src/pbo.rs:850 — an entry named `$PBOPREFIX$` collides with the generated prefix file and is refused without `--force` — accepted because the result is an error, or an overwrite of a file the tool wrote itself inside the target
src/pbo.rs:850 — Windows trims trailing dots and spaces (`a.`) — accepted because the result stays in the same directory and only collides with its own trimmed name
src/io.rs:83 — `read_cstring_bytes` reads until NUL without a length limit — accepted because the string cannot be longer than the remaining input and a missing NUL is an EOF error; `read_compressed_int` (io.rs:97) stops after 5 bytes and its shift cannot overflow
src/p3d.rs:447 — `P3D::read` memory is bounded by the file size times the record size — accepted because counts are not used for preallocation (`capacity` at p3d.rs:22 caps at 65 536), face vertex counts must be 3 or 4 and tagg data is read with `take`; the mutation test found no panic
src/sign.rs:301 — key readers allocate by the key length — accepted because the length is checked (288 to 2^20 bits, sign.rs:93) before any allocation, so a buffer is at most 128 KiB, and the length fields are cross-checked
src/sign.rs:499 — `verify` runs `mod_exp` on a key of up to 2^20 bits, which takes seconds of CPU — accepted because the user chose the key and signature file and only waits
src/binarize.rs:51 — Windows temp dir `dayzmake_<name>_<i>` is predictable with an exists-then-create race — accepted because `%TEMP%` is per user; an attacker who can write there already is the user
src/binarize.rs:525 — Proton sandbox `dayzmake-binarize-<pid>-<nanos>` is predictable — accepted because it is created with `create_dir`, which fails if it exists and does not follow a symlink, and it holds copies of the user's own sources
src/binarize.rs:403 — external processes (`proton run`, `binarize.exe`) — accepted because their arguments come from the CLI and the source folder, never from a PBO, config or model, and are passed as an argument vector, not through a shell
src/pbo.rs:628 — `list_files` (used by `from_directory`) follows directory symlinks, so a symlink cycle in the source folder recurses without end — accepted because it is the author's own source folder (out of scope); noted for a future change
src/preprocess.rs:648 — `#include` can reach files outside the include folders (relative `../..` or an absolute `\` path; the content ends up in the output config) and has no depth limit, so a file that includes itself overflows the stack — accepted because the preprocessor only runs on the author's own source folder (out of scope); it must not be run on a source folder from an untrusted party

## What the mutation test reaches

Seeds (12): the 2 PBOs, 2 PAAs, 2 P3Ds, the key pair and 2 signatures under `tests/fixtures/bi`,
`tests/fixtures/mlod/small.p3d`, and one synthetic PBO built in the test (`lzss_pbo`) whose single
entry is `Cprs`-compressed (literals plus one back reference); `lzss_seed_decompresses` checks that
the unmutated seed unpacks to the expected bytes.

Reached:
- `PBO::read` and `file_data` for every entry: the header parser, entry sizes, and (only through the synthetic seed; the fixture PBOs hold uncompressed entries, method 0) `lzss_decompress`.
- `Config::read_rapified`, only on entries named `*.bin` inside PBO mutants. The fixtures hold no `.bin` file of their own and no rapified config, so the reader gets mutants of a PBO's raw entries, not of a valid rapified seed; its hardening is covered by the Task 1.2 tests in `tests/config.rs`.
- `Paa::read` and `to_dynamic_image` on the two PAA seeds (LZO and DXT paths).
- `P3D::read` and `model_magic` on the MLOD seeds. The ODOL seed (`model.expected.p3d`) only exercises `model_magic` and the "this is ODOL" error; there is no ODOL parser.
- `BIPublicKey::read`, `BIPrivateKey::read`, `BISign::read`, each from a single small seed.

Not reached: `cmd_unpack` / `cmd_inspect` / `cmd_sign` / `cmd_verify` (covered by the targeted regression tests instead), `PBO::from_directory`, the preprocessor, and `binarize`.
