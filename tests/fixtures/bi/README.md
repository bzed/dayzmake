# BI reference fixtures

Generated 2026-10-31 with DayZ Tools (Steam app 830640) under Proton Hotfix.

- `cfgconvert/<name>.cpp`: input. `<name>.expected.cpp`: what `CfgConvert.exe -bin` produced, decoded with
  `dayzmake derapify`. Compare decoded output, not raw bytes: CfgConvert writes an extra 4-byte offset after every
  class body, which dayzmake does not, and the game accepts both.
  Note: `g.expected.cpp`'s `delete Bar;` (line 18) was first corrected by hand after #3 (the earlier text had been
  decoded with the buggy reader, which turned deletions into empty classes). Re-verified 2026-11-15 with DayZ Tools'
  `CfgConvert.exe -bin` under Proton Hotfix: all 12 inputs decode (fixed reader) byte-identically to their
  `.expected.cpp`, and CfgConvert writes entry type 4 (delete) for `Bar`.
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
