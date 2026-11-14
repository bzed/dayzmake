# Security review of untrusted-input code (2026-11)

Untrusted data: any PBO, rapified config, PAA, P3D, `.bikey`, `.biprivatekey`, `.bisign`, and every
name, prefix and header extension inside them. Out of scope: `binarize.exe` itself and building a PBO
from an untrusted source folder (the source folder is the author's own).

Method: `tests/untrusted.rs` mutates every fixture (300 mutants per file in `make test`, 20 000 with
`--ignored`; xorshift64 seed `0x9E3779B97F4A7C15`) and feeds it to the matching reader, then every
reader of untrusted data in `src/` was read for unchecked arithmetic, file-sized allocations,
recursion, file-supplied offsets, slicing, paths built from file content, output outside the target,
predictable temp files and external processes.

Format: `file:line — issue — fixed in <where> | accepted because <reason>`.

## Fixed in earlier tasks (listed for completeness)

- `src/pbo.rs` `cmd_unpack` — entry names and prefix could leave the target folder (`..`, absolute, drive, dot-only components) — fixed in Task 1.1 (`safe_relative_path`).
- `src/pbo.rs` `cmd_unpack` — extraction through a symlink in the target tree — fixed in Task 1.1 (`crosses_symlink`).
- `src/config.rs` `read_rapified` — recursion on self-referencing or deeply nested class bodies, exponential fan-out — fixed in Task 1.2 (depth, cycle and entry budgets).
- `src/pbo.rs` `lzss_decompress` — allocation sized by the entry's `original_size` — fixed in Task 1.2 (at most 9x the compressed size).
- `src/paa.rs` — tagg size, mipmap dimension and LZO ratio — fixed in Task 1.2.
- `src/pbo.rs` `matches_glob` — pathological patterns — fixed in Task 1.2.

## Fixed in this task

- `src/pbo.rs` `crosses_symlink` — failed open: any `symlink_metadata` error other than NotFound (for example PermissionDenied) was treated as "nothing here, so no symlink" — fixed in this task (only NotFound ends the walk; any other error counts as unsafe). Test `crosses_symlink_fails_closed_when_the_path_cannot_be_inspected`.
- `src/paa.rs` `Paa::read` — no limit on the number of mipmaps or on the total decompressed size; a 1.5 MiB file could make the reader hold about 400 MiB (256x its size) — fixed in this task (at most 16 mipmaps, at most 256 MiB of mipmap data in total, checked before each allocation). Tests `paa_too_many_mipmaps_is_an_error`, `paa_lzo_expansion_is_bounded_in_total`. Real PAAs have at most 16 mipmaps (16384 down to 1 pixel is 15).
- `src/paa.rs` `Paa::read` — the 24-bit mipmap size field preallocated up to 16 MiB per mipmap before reading — fixed in this task (bounded read, nothing preallocated from the field).
- `src/paa.rs` `Paa::to_dynamic_image` — a DXT mipmap shorter than its dimensions need made texpresso panic (`range end index 4112 out of range for slice of length 4096`; found by the mutation test on `glass_ca.paa` and `solid_co.paa`); zero dimensions are rejected too — fixed in this task. Test `paa_short_dxt_mipmap_is_an_error_not_a_panic`.
- `src/sign.rs` `cmd_sign` / `cmd_verify` — the key name from the key file was put into the default signature path with `Path::set_extension`, which panics on a `/` or `\` in the name (and on Windows could steer the file elsewhere) — fixed in this task (`default_signature_path` rejects separators, `:`, control characters and an empty name). Test `sign_rejects_a_key_name_with_a_path_separator`.
- `src/pbo.rs` `cmd_inspect`, `cmd_unpack` warnings, `src/p3d.rs` `cmd_inspect`, `check_dependencies`, `src/binarize.rs` log warnings — names, header extensions, texture, material, property and selection names from the file were printed raw, so a file could inject terminal escape sequences — fixed in this task (`error::printable` escapes control characters as `\u{1b}`, `\n`, ...). Test `inspect_and_unpack_do_not_print_terminal_escapes`. `BIOUTPUT=1` still passes binarize.exe output through unchanged because the user asked for the raw output.
- `src/pbo.rs` `safe_relative_path` — Windows device names (`CON`, `NUL.txt`, `COM1`, ...) would open a device instead of creating a file on Windows — fixed in this task (such components are rejected on every platform). Test `safe_path_rejects_windows_device_names`.

## Open items from earlier reviews

- `src/pbo.rs` `cmd_unpack --use-prefix` with an empty prefix — it was an error ("Unsafe PBO prefix"). Decision: an empty prefix, or one of only separators, means "no prefix" and extracts into the output folder. A PBO without a prefix already behaves that way, and an empty prefix is how tools write "none"; nothing can escape from it. A prefix that is non-empty but unsafe (`..`, drive, device name) is still an error. Fixed in this task. Test `unpack_use_prefix_with_empty_prefix_extracts_into_target`.
- `src/pbo.rs` `crosses_symlink` — see above, fixed.
- `src/paa.rs` mipmap count and total size — see above, fixed.

## Reviewed and accepted

- `src/pbo.rs` `PBO::read` — the whole PBO is held in memory and the header list is bounded only by the file size — accepted because every allocation is bounded by bytes actually read from the input (`take` + `read_to_end`, no preallocation from sizes), so memory is at most about the file size.
- `src/pbo.rs` `PBO::read` / `file_data` — two entries with the same name: the map keeps the last data, `file_data` uses the first header — accepted because it only gives wrong bytes for that name; `lzss_decompress` is bounded by the data it is given. `cmd_unpack` skips the second path anyway.
- `src/pbo.rs` `cmd_unpack` — a file or directory swapped for a symlink between the `crosses_symlink` check and `File::create` (time of check/time of use) — accepted because it needs a second local process writing into the target folder while unpacking; the target folder is the user's own.
- `src/pbo.rs` `cmd_unpack` — the target folder itself or a parent may be a symlink — accepted because it is the folder the user named.
- `src/pbo.rs` `cmd_unpack` — entries named `$PBOPREFIX$` collide with the generated prefix file and are refused without `--force` — accepted because the result is an error or the entry overwrites a file the tool wrote itself, inside the target.
- `src/pbo.rs` `cmd_unpack` — Windows names that end in a dot or space (`a.`) are trimmed by Windows — accepted because they stay inside the component's directory and only collide with their own trimmed name.
- `src/io.rs` `read_cstring_bytes` — reads until NUL without a length limit, one byte at a time — accepted because the string cannot be longer than the remaining input and a missing NUL is an EOF error. `read_compressed_int` stops after 5 bytes (the shift of the last byte drops the high bits, it cannot overflow the shift amount).
- `src/p3d.rs` `P3D::read` — counts from the file are not used for preallocation (`capacity` caps at 65 536), face vertex counts must be 3 or 4, tagg data is read with `take`; memory is bounded by the file size times the struct size per record — accepted. Mutation test found no panic.
- `src/sign.rs` `BIPrivateKey/BIPublicKey/BISign::read` — key length is checked (288 to 2^20 bits) before any allocation, so a buffer is at most 128 KiB; the length fields are cross-checked — accepted. `verify` runs `mod_exp` on a key of up to 2^20 bits, which takes seconds — accepted because the user chose the key and signature file and only waits.
- `src/sign.rs` `cmd_sign` / `cmd_verify` with a key name that is a long string — accepted, it is only a file name part and a too long name fails in `File::create`.
- `src/config.rs` `read_rapified` — see Task 1.2; the mutation test found no panic or hang.
- `src/binarize.rs` `create_temp_directory` (Windows) — predictable name `dayzmake_<name>_<i>` with an exists-then-create race in the user's own temp folder — accepted because `%TEMP%` is per user; an attacker who can write there already is the user.
- `src/binarize.rs` Proton sandbox — `dayzmake-binarize-<pid>-<nanos>` is predictable but created with `create_dir`, which fails if it exists and does not follow a symlink; the sandbox holds copies of the user's own sources — accepted.
- `src/binarize.rs` external processes — `proton run` and `binarize.exe` get paths from the CLI and the source folder, never strings from a PBO, config or model — accepted. Arguments are passed as an argument vector, not through a shell.
- `src/pbo.rs` `list_files` (used by `from_directory`) follows directory symlinks, so a symlink cycle in the source folder recurses without end — accepted because it is the author's own source folder (out of scope); noted for a future change.
- `src/preprocess.rs` `#include` — an include path is resolved relative to the including file or searched in the `-i` include folders; a relative path such as `../../x` or an absolute `\` path can reach files outside the include folders (the content then ends up in the output config), and there is no include depth limit, so a file that includes itself overflows the stack — accepted because the preprocessor only runs on the author's own source folder, which is trusted input (out of scope). It must not be run on a source folder from an untrusted party.
- `src/pbo.rs` `from_directory` and `src/preprocess.rs` read the author's own `$PBOPREFIX$`; nothing from a PBO is used there.

## Not covered by a reader test

- `Config::read_rapified` has no `.bin` seed in `tests/fixtures`; the mutation test reaches it through the `.bin` entries inside the PBO fixtures and through the Task 1.2 tests in `tests/config.rs`.
- There are no `.bin` fixtures of their own, and no `.biprivatekey`/`.bikey`/`.bisign` beyond one key pair and two signatures, so those readers are mutated from a small seed set.
