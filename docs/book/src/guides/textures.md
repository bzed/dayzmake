# Textures and model checks

DayZ textures are PAA files. dayzmake converts them from and to ordinary images. It also checks that the models in
your addon do not reference missing textures.

## From an image to a PAA and back

Start from a PNG, `tex.png`, 64 by 64 pixels:

```text
$ dayzmake img2paa tex.png t21/data/a_co.paa
$ dayzmake img2paa -t dxt1 tex.png t21/data/b_co.paa
$ dayzmake img2paa -t rgba tex.png t21/data/c.paa
$ ls -l t21/data
-rw-r--r-- 1 me me  5577 a_co.paa
-rw-r--r-- 1 me me  2849 b_co.paa
-rw-r--r-- 1 me me 21945 c.paa
```

The `-t` option selects the pixel format:

| Type | Use it for | Size of 64 by 64 |
|---|---|---|
| `dxt1` | no alpha or 1-bit alpha; the smallest | 2849 bytes |
| `dxt3` | explicit 4-bit alpha | not measured here |
| `dxt5` (default) | interpolated alpha | 5577 bytes |
| `rgba` | uncompressed 8-bit RGBA, no loss | 21945 bytes |

dayzmake makes the mipmap chain itself (down to 4 pixels on a side). It does not change the size of your image.
Make the image size a power of two before you convert it. For the DXT types, dayzmake compresses mipmaps that are
wider than 128 pixels with LZO automatically, so `-c` does not change the file.

To look at a PAA from a mod or the game, convert it to an image. The extension of the output file sets the format:

```text
$ dayzmake paa2img t21/data/a_co.paa back.png
$ file back.png
back.png: PNG image data, 64 x 64, 8-bit/color RGBA, non-interlaced
```

`paa2img` also reads the 16-bit formats ARGB4444, ARGB1555 and AI88 (grey with alpha). The game uses these formats
for noise maps and detail maps. If a `.paa` file is really a PNG file, `paa2img` gives an error that says so. Then
convert the image with `img2paa`.

## Checking the textures of your models

`dayzmake inspect` lists the textures and materials of a model:

```text
$ dayzmake inspect t21/small.p3d
Format:  MLOD (editable model)
Version: 257
LODs:    2

  #  LOD                        Points    Faces  Normals  UV sets
  0  Resolution 1                    4        3        2        2
  1  Memory                          4        0        2        0

Textures:
- #(argb,8,8,3)color(0.5,0.5,0.5,1,CO) (1 faces)
- t21\data\a_co.paa (1 faces)
- t21\data\gone_co.paa (1 faces)
Materials:
- t21\data\m.rvmat (2 faces)

Properties:
- LOD 0: class = house
- LOD 0: autocenter = 0
- LOD 1: x = y

Selections:
- LOD 0 (Resolution 1): body, old
- LOD 1 (Memory): start
```

`inspect` also lists a binarized ODOL v56 model (LODs, textures, materials, proxies, selections, animations). For
other ODOL versions and for BMTR, it shows only the format and the version.

`build` goes further. It checks each editable model that it packs against the files in the addon. It does this with
or without `--proton-binarize`:

```text
$ dayzmake build t21 t21.pbo
In file small.p3d: warning: Model references texture "t21\data\gone_co.paa", which is not in the addon. [missing-texture]
In file small.p3d: warning: Model references material "t21\data\m.rvmat", which is not in the addon. [missing-material]
In file small.p3d: warning: On non-Windows systems binarize.exe is only used with --proton-binarize; file will be copied as-is. [non-windows-binarization]
```

How dayzmake checks references:

- dayzmake looks up a reference that starts with the prefix of the addon (`t21\...`) in the source folder and
  ignores the case. If the file is not there, dayzmake reports it. Here `a_co.paa` exists (the model uses it) and
  `gone_co.paa` does not.
- For other references (vanilla `dz\...`, other mods), dayzmake checks only if the file exists below an `-i` folder
  or the current directory. If it does not, dayzmake does not report it, because it cannot know if the reference is
  wrong.
- dayzmake skips procedural textures such as `#(argb,8,8,3)color(...)`.
- To mute the checks, use `-w missing-texture` and `-w missing-material`.

A file with the `.p3d` extension that is not a model is not fatal. dayzmake reports it once and copies it:

```text
In file data/broken.p3d: warning: Cannot check textures and materials: not an MLOD model [model-unreadable]
```

For every message, see [Warnings](../reference/warnings.md). For the options, see [Textures](../reference/paa.md).
