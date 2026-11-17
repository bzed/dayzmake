# BI reference fixtures

Generated 2026-10-31 with DayZ Tools (Steam app 830640) under Proton Hotfix.

- `cfgconvert/<name>.cpp`: input. `<name>.expected.cpp`: what `CfgConvert.exe -bin` produced, decoded with
  `dayzmake derapify`. Compare decoded output, not raw bytes: CfgConvert writes an extra 4-byte offset after every
  class body, which dayzmake does not, and the game accepts both.
  Note: `g.expected.cpp`'s `delete Bar;` (line 18) was first corrected by hand after #3 (the earlier text had been
  decoded with the buggy reader, which turned deletions into empty classes). Re-verified 2026-11-15 with DayZ Tools'
  `CfgConvert.exe -bin` under Proton Hotfix: all 12 inputs decode (fixed reader) byte-identically to their
  `.expected.cpp`, and CfgConvert writes entry type 4 (delete) for `Bar`.
- `cfgconvert/<name>.bin`: CfgConvert's own binary for each input, made 2026-11-15 with
  `CfgConvert.exe -bin -dst <out> <in>` (DayZ Tools as installed 2026-11-15, Steam app 830640) under Proton
  Hotfix. The tests decode them with `Config::read_rapified` and compare the text with `<name>.expected.cpp`.
  The bytes themselves are not compared with dayzmake's output: CfgConvert writes an extra 4-byte offset after each
  class body.
- `cfgconvert/last_*.cpp`, `enum_dup*.cpp` (with `.bin` and `.expected.cpp`): made 2026-11-17 with
  `CfgConvert.exe -bin -dst <out> <in>` under Proton 10.0 (Proton Hotfix hung on every run, even for `cmd.exe /c exit`),
  a fresh prefix, drive `P:` linked to the work folder, a copy of `Bin/CfgConvert` as `P:\tool`, `xvfb-run` (the tool
  opens a window on failure and Wine has no display here), stdin from `/dev/null`, `prlimit --as=4G timeout 300`.
  Observed: `x = 1.5`, `x = 1e5` and `x = abc` with a final newline give `x = 1.5;`, `x = 100000.0;`, `x = "abc";`
  (`last_float`, `last_exp`, `last_str`); `x = 1e999` with or without `;` gives `x = inf;` (since #102 dayzmake does the same); the same four inputs without a final newline fail with
  `File ..., line 0: '.x': '<EOF byte>' encountered instead of ';'`, `x = 1.5\r\n` works. Enums (`enum_dup.cpp`):
  `enum { A, B = 5, A };` is accepted silently and the table holds `a` once, at its first place, with the last value
  (`a = 6`, `b = 5`); `enum_dup2.cpp`: `enum { A = 1, A = 2 };` gives `a = 2`; a later `y = A;` stays the string `"A"`. The enum
  expected files document CfgConvert; dayzmake keeps every name, so only the values are compared.
  Fix round 1 (same setup): `last_mid` (`x = 1.5`, `y = "s"`, `z = 2` without `;`, then `w = 3;`), `last_eval`
  (`x = __EVAL(1+1)` without `;` gives `x = 2;`), `last_endif` and `last_define` (`x = 1` before a final
  `#endif` / `#define Y 1` without line end) are all accepted and typed.
- `cfgconvert/last_comment*.cpp` (with `.bin`, `.expected.cpp`): made 2026-11-17, same setup (Proton 10.0, xvfb-run).
  None has a final newline. `last_comment` is `x = 1\n// c`, `last_comment_semi` is `x = 1;\n// c`, `last_comment_block`
  is `x = 1;\n/* c */`, `last_comment_open` is `x = 1;\n/* c` (unterminated): all four are accepted and give `x = 1;`.
  Unterminated `/*` with text after it (same setup, no output on stderr, nothing in the log): `x = 1;\n/* c\ny = 2;`
  and `x = 1;\n/* c\ny = 2;\nz = 3;\n` give `x = 1;`; `x = 1;\n/* c\n#include "nope.hpp"\nz = 3;\n` gives `x = 1;`
  (the missing include is never looked up); `a = 1;\n/* open\nb = 2;` in `inc.hpp`, included from a file that goes on
  with `c = 3;`, gives `a = 1; c = 3;` (the comment ends with the include). So CfgConvert is silent; dayzmake does
  the same and warns `unterminated-comment`.
  Lone CR, not committed: `x = 1\r// c` and `x = 1\r#define Y 1` and `x = 1;\r#define Y 1` are rejected (no output);
  `x = 1;\r// c`, `x = 1\r\n// c` and `x = 1\r\n#define Y 1` give `x = 1;`; `x = 1\ry = 2;` gives `x = "1y = 2";`
  (CfgConvert does not treat a lone CR as a line end; dayzmake's preprocessor does, and is lenient there).
- `cfgconvert/float_ovf*.cpp` (with `.bin`, `.expected.cpp`): made 2026-11-17, same setup (Proton 10.0, xvfb-run).
  `a = 1e999; b = -1e39; c = 1e-999; d = 3.4e38;` gives f32 bits `7f800000`, `ff800000`, `00000000`, `7f7fc99e`
  (inf, -inf, 0, 3.4e38); `float_ovf2` adds `-1e999` (`ff800000`), `1e39` and `3.5e38` and `3.4028236e38` (all `7f800000`),
  `-1e-999` (`80000000`, `-0.0`), `3.4028235e38` (`7f7fffff`) and an array `{1e999, -1e999, 1e-999}`. No error, empty log.
  `x = 1e999x;` stays the string `"1e999x"`. CfgConvert's own decode shows `inf`; the expected files hold dayzmake's
  form `1e999` / `-1e999`.
- `cfgconvert/int_ovf*.cpp` and `define_open_comment.cpp` (with `.bin`, `.expected.cpp`): made 2026-11-17, same setup
  (Proton 10.0, xvfb-run, work folder linked as `P:`, `Bin/CfgConvert` copied to `P:\tool`). `x = 99999999999999999999\ny = 1;\n`
  (`int_ovf`) gives `x = 1e20;` (a float, not a string and not an error); `int_ovf2` adds `-99999999999999999999`
  (`-1e20`), `w = 99999999999999999999.5;`, and an array `{99999999999999999999, 3000000000, -3000000000, 2147483647}` that gives
  `{1e20, 3000000000.0, -3000000000.0, 2147483647}`. `#define X a /* b\ny = 2;\n` (`define_open_comment`) gives an empty
  config, no error. Not committed: `x = 1 // c` and `y = 1;\nx = 2 \\\n// c` (no final newline) are rejected (no output file);
  `m.cpp` = `#include "inc.hpp"\nz = 3;\n` with `inc.hpp` = `a = 1;\nb = 2` (no final newline) is accepted and gives
  `a = 1; b = "2#line 1 ""P:\in\m.cpp"""; z = 3;` (CfgConvert glues its `#line` marker to the last value; dayzmake gives `b = 2;`). Re-checked 2026-11-17 with the include on line 3 (`y = 1;\nw = 2;\n#include "inc.hpp"\nz = 3;\n`): the marker is `#line 3`, so `#line <line of the include>` is right and `#line 1` above is correct, not an off-by-one.
- `cfgconvert/int64*.cpp`, `hex_*.cpp` (with `.bin`, `.expected.cpp`): made 2026-11-17, same setup (Proton 10.0, xvfb-run).
  Entries (`int64`: `a = 2147483647; b = 2147483648; c = -2147483649; d = 9223372036854775807; e = 9223372036854775808; f = 0xFFFFFFFF;
  g = 0x100000000; h = 0x1FFFFFFFFF; j = -2147483648; k = -9223372036854775808; l = -9223372036854775809; m = 0x7FFFFFFF; n = 0x80000000`):
  `a`, `j`, `m` subtype 2 (int32); `b`, `c`, `d`, `k` subtype 6 (8-byte little-endian i64); `e`, `l` subtype 1 (f32, 2^63 and -2^63);
  `f` -1, `g` 0, `h` -1, `n` -2147483648, all subtype 2 (hex keeps its low 32 bits). `int64_arr` (`arr[]` of the same values): there is
  no int64 element type; a decimal outside i32 is an element of type 1 (float: 2^31, -2^31, 2^63), a hex literal is type 2 with the low 32 bits.
  `hex_long` (`p = 0x1FFFFFFFFFFFFFFFF;`) is -1, `hex_zeros` (`0x0000000000000000000001`) is 1, `hex_long_arr` the same in an array.
  `hex_wide`: `-0x10` and `-0x1FFFFFFFFF` are subtype 1 floats (-16.0, -2^37), `0X10` is 16, `+2147483648` is subtype 6, `+5` is 5, `0x8000000000000000` is 0,
  `0xFFFFFFFFFFFFFFFF` is -1. `hex_sign`: `+0x10` is the float 16.0, `-0x0` the float -0.0, `-0X10` -16.0, `00012` the int 12 (decimal), `-0` the int 0.
  `hex_sign_arr`: the same rules in an array (`-0x10`, `+0x10` floats, `0X10` int, `+2147483648` float, `-0` int 0). `x = 0x;` is the string `"0x"`.
- `sign/`: throwaway test key `armk` (made by `dayzmake keygen`). `testsig.pbo` was built by dayzmake.
  `nonutf8.pbo` is the same PBO with the entry `data\aXb.txt` renamed to `data\a<0x97>b.txt` (cp1252 em dash) and its
  SHA-1 trailer recomputed. Both `.bisign` files were made by `DSSignFile.exe`, and `DSCheckSignatures.exe -deep`
  reports them OK. dayzmake's signature for `testsig.pbo` is byte-identical to BI's.
- `binarize/t21/`: addon with prefix `t21`. `model.p3d` is an MLOD with two triangles (made by `binarize/mkmlod.py`):
  one textured `t21\data\glass_ca.paa` (DXT5, alpha), one `t21\data\solid_co.paa` (DXT1).
  `model.expected.p3d` is binarize.exe's ODOL output when the textures are reachable: run with the current
  directory at the project root (`P:\`), as BI's tools do. Its model and section flags contain `0x802020`
  (IsAlpha|IsTransparent|IsAlphaFog). Binarized without the textures, the output is identical except
  that those flags are `0` (upstream #21).
- `paa/`: 16-bit PAA files made by `ImageToPAA.exe` (DayZ Tools, Steam app 830640, as installed 2026-11-16) under
  Proton Hotfix on 2026-11-16. `rgbq.png` is the input: 8 × 8 RGBA, red top left, green top right, blue bottom
  left, white with alpha 136 bottom right (4 × 4 pixel quadrants; made by a small Python script, committed as the
  PNG). `ImageToPAA.exe` picks the output format from the file name through `TexConvert.cfg`, so the same image was
  copied to `rgbq_4444.png`, `rgbq_1555.png` and `rgbq_88.png` (suffixes `*_4444.*` = ARGB4444, `*_1555.*` =
  ARGB1555, `*_88.*` = AI88) and converted with
  `proton run ImageToPAA.exe 'Z:\...\rgbq_4444.png' 'Z:\...\rgbq_4444.paa'` (likewise for `1555` and `88`; the
  environment is the one `crates/dayzmake-core/src/binarize.rs` sets for `binarize.exe`: `STEAM_COMPAT_CLIENT_INSTALL_PATH`,
  `STEAM_COMPAT_DATA_PATH` and the app id 830640, with `stdin` from `/dev/null`; each run takes about a minute). The
  tool writes the magics `0x4444`, `0x1555` and `0x8080`, so `0x4444` is ARGB4444: bits 12 to 15 alpha, then 4 bits
  each of R, G and B (the red quadrant is the word `0xFF00`). `crates/dayzmake-core/tests/bi_paa_golden.rs` decodes
  the three files and checks the colours. Only the `.paa` files and `rgbq.png` are committed.
