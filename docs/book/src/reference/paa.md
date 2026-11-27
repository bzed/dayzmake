# Textures: paa2img, img2paa

PAA is the texture format of DayZ. These commands convert between PAA and ordinary image files. They are
available when dayzmake has the `image` feature. The released binaries have it. The examples start from a 64 by 64
PNG named `tex.png`.

## dayzmake paa2img

Convert a PAA texture to an image.

<div class="synopsis">

```text
dayzmake paa2img [-v] [-f] <source> <target>
```

</div>

| Argument | Meaning |
|---|---|
| `<source>` | The PAA file. |
| `<target>` | The image to write. The extension gives the format: `png`, `jpg`, `bmp`, `tga` and `gif` work. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `-f`, `--force` | off | Overwrite an existing `<target>`. |

Behaviour:

- dayzmake decodes the largest mipmap to 8-bit RGBA. It supports DXT1, DXT3, DXT5 and uncompressed RGBA PAAs, with
  or without LZO-compressed mipmaps. It also reads the 16-bit formats ARGB4444, ARGB1555 and AI88 (grey with
  alpha). Their mipmaps are LZSS-compressed. A wrong checksum or a cut-short stream is an error. dayzmake cannot
  write these formats (`img2paa -t` offers DXT1, DXT3, DXT5 and RGBA).
- If a file starts with the PNG signature, dayzmake reports a PNG file, not an unknown PAA.
- The reader treats the file as untrusted. It limits the tag sizes, the mipmap sizes and the number of mipmaps, so a
  small hostile file cannot make dayzmake allocate gigabytes.
- An extension that is not an image format is an error, and dayzmake writes nothing.

Examples:

```text
$ dayzmake paa2img t21/data/a_co.paa back.png
$ file back.png
back.png: PNG image data, 64 x 64, 8-bit/color RGBA, non-interlaced
$ dayzmake paa2img t21/data/a_co.paa back.png
error: Target file "back.png" already exists. Use --force to overwrite.
$ dayzmake paa2img t21/data/a_co.paa back.txt
error: Failed to save output image:
The file extension `."txt"` was not recognized as an image format
```

See also: [img2paa](#dayzmake-img2paa), [Textures and model checks](../guides/textures.md).

## dayzmake img2paa

Convert an image to a PAA texture.

<div class="synopsis">

```text
dayzmake img2paa [-v] [-f] [-c] [-t <paatype>] <source> <target>
```

</div>

| Argument | Meaning |
|---|---|
| `<source>` | The image (PNG, JPEG, BMP, TGA, GIF and the other formats of the `image` library). |
| `<target>` | The PAA to write. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `-f`, `--force` | off | Overwrite an existing `<target>`. |
| `-c`, `--compress` | off | LZO-compress the mipmaps of a DXT texture that are wider than 128 pixels, where this makes them smaller. `img2paa` already does this for every conversion, so the option does not change the output. It stays for compatibility with armake2. `rgba` is never compressed, and this is not an error. |
| `-t <paatype>`, `--type <paatype>` | `dxt5` | The pixel format: `dxt1`, `dxt3`, `dxt5` or `rgba`, in any case. |

Behaviour:

- `dxt1` has no alpha or 1-bit alpha and is the smallest. `dxt3` has explicit 4-bit alpha. `dxt5` has interpolated
  alpha. `rgba` is uncompressed 8-bit RGBA (about eight times the size of `dxt1`).
- The output has a chain of mipmaps. Each mipmap has half the size of the one before, until a side reaches 4 pixels
  (five levels for a 64 by 64 image). The sizes of the image must be powers of two, as the game expects. dayzmake
  does not change the size (a 100 by 50 image stays 100 by 50).
- An unknown type is an error: `Unsupported PAA compression type for creation: foo`.

Examples (a 64 by 64 image gives these sizes):

```text
$ dayzmake img2paa tex.png t21/data/a_co.paa
$ dayzmake img2paa -t dxt1 tex.png t21/data/b_co.paa
$ dayzmake img2paa -t rgba tex.png t21/data/c.paa
$ ls -l t21/data
-rw-r--r-- 1 me me  5577 Nov 16 15:34 a_co.paa
-rw-r--r-- 1 me me  2849 Nov 16 15:34 b_co.paa
-rw-r--r-- 1 me me 21945 Nov 16 15:34 c.paa
```

```text
$ dayzmake img2paa -t foo tex.png x.paa
error: Failed to convert image to PAA:
Unsupported PAA compression type for creation: foo
$ dayzmake img2paa nonexist.png y.paa
error: Failed to open input image file "nonexist.png":
No such file or directory (os error 2)
```

See also: [paa2img](#dayzmake-paa2img).
