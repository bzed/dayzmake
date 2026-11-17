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
  Lone CR, not committed: `x = 1\r// c` and `x = 1\r#define Y 1` and `x = 1;\r#define Y 1` are rejected (no output);
  `x = 1;\r// c`, `x = 1\r\n// c` and `x = 1\r\n#define Y 1` give `x = 1;`; `x = 1\ry = 2;` gives `x = "1y = 2";`
  (CfgConvert does not treat a lone CR as a line end; dayzmake's preprocessor does, and is lenient there).
- `cfgconvert/float_ovf*.cpp` (with `.bin`, `.expected.cpp`): made 2026-11-17, same setup (Proton 10.0, xvfb-run).
  `a = 1e999; b = -1e39; c = 1e-999; d = 3.4e38;` gives f32 bits `7f800000`, `ff800000`, `00000000`, `7f7fc99e`
  (inf, -inf, 0, 3.4e38); `float_ovf2` adds `-1e999` (`ff800000`), `1e39` and `3.5e38` and `3.4028236e38` (all `7f800000`),
  `-1e-999` (`80000000`, `-0.0`), `3.4028235e38` (`7f7fffff`) and an array `{1e999, -1e999, 1e-999}`. No error, empty log.
  `x = 1e999x;` stays the string `"1e999x"`. CfgConvert's own decode shows `inf`; the expected files hold dayzmake's
  form `1e999` / `-1e999`.
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
