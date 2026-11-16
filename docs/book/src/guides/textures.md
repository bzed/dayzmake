# Textures and model checks

DayZ textures are PAA files. dayzmake converts them from and to ordinary images and checks that the models in
your addon do not reference textures that are missing.

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

The `-t` option chooses the pixel format:

| Type | Use it for | Size of 64 by 64 |
|---|---|---|
| `dxt1` | no alpha or 1-bit alpha; the smallest | 2849 bytes |
| `dxt3` | explicit 4-bit alpha | not measured here |
| `dxt5` (default) | interpolated alpha | 5577 bytes |
| `rgba` | uncompressed 8-bit RGBA, no loss | 21945 bytes |

dayzmake builds the mipmap chain itself (down to 4 pixels on a side). It does not resize your image, so make it
a power of two before. Mipmaps wider than 128 pixels of the DXT types are LZO-compressed automatically, so `-c` does not change the file.

To look at a PAA from a mod or the game, convert it to an image; the format follows the extension:

```text
$ dayzmake paa2img t21/data/a_co.paa back.png
$ file back.png
back.png: PNG image data, 64 x 64, 8-bit/color RGBA, non-interlaced
```

`paa2img` also reads the 16-bit formats ARGB4444, ARGB1555 and AI88 (grey with alpha) that the game uses for
noise and detail maps. A `.paa` that is really a PNG file gets an error that says so; convert the image with
`img2paa`.

## Checking the textures of your models

`dayzmake inspect` on a model lists its textures and materials:

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

A binarized ODOL v56 model is listed as well (LODs, textures, materials, proxies, selections, animations); for other
ODOL versions and for BMTR only format and version are shown.

`build` goes further: it checks every editable model it is about to pack, with or without
`--proton-binarize`, against the files in the addon:

```text
$ dayzmake build t21 t21.pbo
In file small.p3d: warning: Model references texture "t21\data\gone_co.paa", which is not in the addon. [missing-texture]
In file small.p3d: warning: Model references material "t21\data\m.rvmat", which is not in the addon. [missing-material]
In file small.p3d: warning: On non-Windows systems binarize.exe is only used with --proton-binarize; file will be copied as-is. [non-windows-binarization]
```

How references are judged:

- A reference that starts with the addon's prefix (`t21\...`) is looked up in the source folder, ignoring case.
  If it is not there, it is reported. Here `a_co.paa` exists (the model uses it), `gone_co.paa` does not.
- Other references (vanilla `dz\...`, other mods) are only checked if they exist below an `-i` folder or the
  current directory. If they do not, they are not reported, because there is no way to tell whether they are
  wrong.
- Procedural textures such as `#(argb,8,8,3)color(...)` are skipped.
- Mute the checks with `-w missing-texture` and `-w missing-material` if you must.

A file with the `.p3d` extension that is not a model is not fatal; it is reported once and copied:

```text
In file data/broken.p3d: warning: Cannot check textures and materials: not an MLOD model [model-unreadable]
```

See [Warnings](../reference/warnings.md) for every message and [Textures](../reference/paa.md) for the options.
