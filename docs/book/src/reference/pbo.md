# PBOs: build, pack, inspect, unpack, extract, cat

A PBO is the archive format of DayZ. It has a header with the *prefix* and other [header
extensions](header-extensions.md), a list of entries and a checksum. `build` makes the PBO of a mod folder the way
the game needs it. `pack` only archives. `inspect`, `unpack` and `cat` read PBOs. dayzmake treats the contents of a
PBO as untrusted. It never writes an entry name with `..`, a drive letter or an absolute path outside the target
folder.

The examples use the folder `mymod/` from [A first mod](../guides/first-mod.md).

## dayzmake build

Build a PBO from a folder. The command rapifies configs, binarizes models, checks model textures and packs the files.

<div class="synopsis">

```text
dayzmake build [-v] [-f] [--proton-binarize] [-w <wname>]... [-i <includefolder>]... [-x <excludepattern>]... [-e <headerext>]... [-k <privatekey>] [-s <signature>] <sourcefolder> [<target>]
```

</div>

| Argument | Meaning |
|---|---|
| `<sourcefolder>` | The folder to pack. Its `$PBOPREFIX$` (or its name) is the prefix. |
| `<target>` | The PBO to write. Standard output when omitted (not allowed together with `-k`). |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Show every warning. |
| `-f`, `--force` | off | Overwrite `<target>` and an existing signature file. |
| `--proton-binarize` | off | Linux: binarize `.p3d` and `.rtm` files with DayZ Tools' `binarize.exe` under Proton. Without it, dayzmake copies models as they are. See [Binarizing models on Linux](../guides/binarize-linux.md). On Windows `binarize.exe` is always used. |
| `-w <wname>`, `--warning <wname>` | none | Mute a [warning](warnings.md) by name. |
| `-i <includefolder>`, `--include <includefolder>` | none | Folder to search for absolute includes in configs and for textures of other mods when it checks models. dayzmake checks the current directory directly only. |
| `-x <excludepattern>`, `--exclude <excludepattern>` | none | Glob pattern of files to leave out. You can repeat it. See below. |
| `-e <headerext>`, `--headerext <headerext>` | none | Header extension `key=value`. You can repeat it. See [Header extensions and prefixes](header-extensions.md). |
| `-k <privatekey>`, `--key <privatekey>` | none | Sign the PBO with this `.biprivatekey` after writing it. |
| `-s <signature>`, `--signature <signature>` | `<target>.<keyname>.bisign` | Where `-k` writes the signature. |

Behaviour:

- dayzmake visits the files in sorted order, so the same folder always gives the same bytes. It stores no timestamps.
- dayzmake rapifies `config.cpp` and stores it as `config.bin`. It rapifies every other `.cpp` and every `.rvmat`, and
  they keep their names. It copies a config that is already rapified, with the `prerapified-file` warning.
- dayzmake binarizes `.p3d` and `.rtm` files (with `--proton-binarize` on Linux). It copies a model that is already
  binarized (ODOL, BMTR) with `prebinarized-file`. It packs a `.p3do` file as `.p3d`. On Linux without
  `--proton-binarize`, it copies the models, and each one raises `non-windows-binarization`.
- dayzmake checks every editable (MLOD) model for textures and materials that are not in the addon
  (`missing-texture`, `missing-material`). See [Textures and model checks](../guides/textures.md).
- A file named `$NOBIN$` or `$NOBIN-NOTEST$` in the source folder switches rapification and binarization off for
  the whole folder. dayzmake packs the files as they are.
- dayzmake reads `$PBOPREFIX$` (or `$PREFIX$`) and does not store it as an entry. Its first bare line is the prefix.
  Its `key=value` lines become header extensions.
- dayzmake skips files that are not regular files (FIFOs, devices, sockets) with `unsupported-file`. It skips names
  that are not valid as PBO entry names with `unsafe-path`. Names that collide when case is ignored give
  `duplicate-entry`. The first name in sorted order wins, but a generated entry (the rapified `config.bin`)
  wins over a plain file of the same name.
- Exclude patterns use `*` as the only wildcard. It also matches `\`. dayzmake compares a pattern with the whole
  path inside the PBO, written with `\`. It does this before the rename of `config.cpp` to `config.bin`, and the
  comparison is case-sensitive: `-x '*.psd'` leaves out every `.psd`, and `-x 'data\*.png'` leaves out every PNG
  below `data`. dayzmake skips excluded files without a message.
- dayzmake never packs the target, or the temporary files next to it, into the PBO.
- With `-k`, dayzmake signs the PBO with a version 3 signature after it writes the PBO (see
  [sign](signing.md#dayzmake-sign)).

Examples:

```text
$ dayzmake build -x '*.psd' -x 'data\*.png' m2 m2.pbo
$ dayzmake inspect m2.pbo
Header extensions:
- prefix=m2
- product=dayz

# Files: 4

Path                                                  Method  Original    Packed
                                                                  Size      Size
================================================================================
config.bin                                                 0       291       291
data\note.cpp                                              0        31        31
data\x.rvmat                                               0        88        88
scripts\s.c                                                0         2         2
```

`-e` overrides what `$PBOPREFIX$` says:

```text
$ dayzmake build -e author=bzed -e product=dz m2 -f m2.pbo
$ dayzmake inspect m2.pbo | head -4
Header extensions:
- prefix=m2
- product=dz
- author=bzed
```

Signing needs a target:

```text
$ dayzmake build -k mykey.biprivatekey t21
error: Cannot sign a pbo that is piped to stdout.
```

These are the warnings for a model that references files outside the addon (here `t21` contains the test model `small.p3d`):

```text
$ dayzmake build t21 t21.pbo
In file small.p3d: warning: Model references texture "t21\data\gone_co.paa", which is not in the addon. [missing-texture]
In file small.p3d: warning: Model references material "t21\data\m.rvmat", which is not in the addon. [missing-material]
In file small.p3d: warning: On non-Windows systems binarize.exe is only used with --proton-binarize; file will be copied as-is. [non-windows-binarization]
```

See also: [pack](#dayzmake-pack), [Header extensions and prefixes](header-extensions.md), [Warnings](warnings.md).

## dayzmake pack

Pack a folder into a PBO. The command does not rapify or binarize anything.

<div class="synopsis">

```text
dayzmake pack [-v] [-f] [-x <excludepattern>]... [-e <headerext>]... [-k <privatekey>] [-s <signature>] <sourcefolder> [<target>]
```

</div>

| Argument | Meaning |
|---|---|
| `<sourcefolder>` | The folder to pack. |
| `<target>` | The PBO to write. Standard output when omitted (not allowed together with `-k`). |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Show every warning. |
| `-f`, `--force` | off | Overwrite `<target>` and an existing signature. |
| `-x <excludepattern>`, `--exclude <excludepattern>` | none | Glob pattern of files to leave out, as for `build`. |
| `-e <headerext>`, `--headerext <headerext>` | none | Header extension `key=value`. |
| `-k <privatekey>`, `--key <privatekey>` | none | Sign the PBO afterwards. |
| `-s <signature>`, `--signature <signature>` | `<target>.<keyname>.bisign` | Signature path for `-k`. |

Behaviour: the rules for prefixes, exclusion, ordering and unsafe names are the same as for `build`. dayzmake copies
the file contents unchanged. `config.cpp` stays `config.cpp`, and models stay as they are. There are no model checks
and no include handling. Use `pack` to repack a folder that you already built, or for PBOs that hold plain data.

Example. The folder `w` contains a name that is not allowed, a name that differs only in case, a FIFO and a
prefix with slashes:

```text
$ dayzmake pack . ../w.pbo -f
warning: Removed leading/trailing slashes from prefix "/leading/". [prefix-slash]
warning: Skipping "data/a.txt": an entry named "data\A.txt" was already packed. [duplicate-entry]
warning: Skipping "data/bad:name.txt": unsafe entry name. [unsafe-path]
In file data/pipe: warning: Skipping a file that is not a regular file (FIFO, device or socket). [unsupported-file]
```

The error names a folder that does not exist:

```text
$ dayzmake pack nonexist x.pbo
error: Failed to read source folder "nonexist":
No such file or directory (os error 2)
```

See also: [build](#dayzmake-build), [unpack](#dayzmake-unpack).

## dayzmake inspect

List the contents of a PBO, or describe a model.

<div class="synopsis">

```text
dayzmake inspect [-v] [--json] [<source>]
```

</div>

| Argument | Meaning |
|---|---|
| `<source>` | A PBO or a `.p3d` model. Standard input when omitted. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `--json` | off | PBOs only: print the header as one JSON object (see below). |

Behaviour:

- If a file starts with a model header, `inspect` describes it as a model. It reads anything else as a PBO.
- For a PBO, `inspect` shows the header extensions, the number of files, and a table. The table has the name, the
  packing method (0 is stored; compressed entries show their packed size), the original size and the stored size.
- For an editable MLOD model, `inspect` shows the format and version. It also shows the LODs with the counts of points,
  faces, normals and UV sets, the textures and materials with the number of faces that use them, the properties and
  the named selections.
- For a binarized ODOL v56 model, `inspect` shows the format and version, the skeleton and the bone count, the
  property class and damage, the mass, the bounding box and the LODs with vertex and face counts. It also shows the
  textures and materials with the number of LODs that use them (a built-in material without a name is `(none)`), the
  proxies, the properties, the named selections and the animation classes. A v56 model that dayzmake cannot read is
  an error. For another ODOL version, or a BMTR file, `inspect` shows only the format and version (ODOL adds a note
  that dayzmake decodes only v56).
- The command only reads. It writes nothing.
- `--json` prints one JSON object on one line, with the fields in this order:
  - `header_extensions`: an ordered array of `{key, value, key_hex, value_hex}`;
  - `prefix` and `prefix_hex`: the prefix, or both `null` when the PBO has none;
  - `entries`: an array of `{name, name_hex, packing, original_size, data_size, timestamp}` in header order.

  `key`, `value`, `prefix` and `name` are the bytes, decoded lossily as UTF-8 and only JSON-escaped (no
  control-character escaping of the text output). The `*_hex` fields are the lowercase hex of the raw bytes. A
  parser must read the hex, because the text of a name with a byte that is not UTF-8 has lost that byte. For a
  model, `--json` is an error: `--json is only supported for PBOs`.

Examples:

```text
$ dayzmake inspect --json mymod.pbo
{"header_extensions":[{"key":"prefix","value":"mymod","key_hex":"707265666978","value_hex":"6d796d6f64"}],"prefix":"mymod","prefix_hex":"6d796d6f64","entries":[{"name":"config.bin","name_hex":"636f6e6669672e62696e","packing":0,"original_size":291,"data_size":291,"timestamp":0}]}
```

```text
$ dayzmake inspect mymod.pbo
Header extensions:
- prefix=mymod

# Files: 2

Path                                                  Method  Original    Packed
                                                                  Size      Size
================================================================================
config.bin                                                 0       291       291
scripts\4_world\player.c                                   0        27        27
```

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

A file that is not a PBO:

```text
$ dayzmake inspect notpbo
error: Failed to read PBO:
failed to fill whole buffer
```

See also: [Textures and model checks](../guides/textures.md).

## dayzmake unpack

Extract a PBO into a folder.

<div class="synopsis">

```text
dayzmake unpack [-v] [-f] [--use-prefix] [-x <excludepattern>]... [--only <pattern>]... <source> <targetfolder>
```

</div>

| Argument | Meaning |
|---|---|
| `<source>` | The PBO. |
| `<targetfolder>` | The folder to extract into. dayzmake creates it when it does not exist. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Show every warning. |
| `-f`, `--force` | off | Overwrite files that already exist in the target folder. |
| `--use-prefix` | off | Extract into `<targetfolder>/<prefix>/` instead of directly into `<targetfolder>`. |
| `-x`, `--exclude <excludepattern>` | none | Glob pattern of entries to leave out. You can repeat it. |
| `--only <pattern>` | none | Glob pattern: write only the entries that match one. You can repeat it. |

Behaviour:

- dayzmake writes the header extensions to `$PBOPREFIX$` in the extracted folder. A later `build` or `pack` of the
  folder then gives the same prefix again. dayzmake replaces control characters in extensions with spaces
  (`unsafe-header`).
- dayzmake validates entry names before it uses them. It skips absolute paths, drive letters, `..`, empty names and
  names that are not valid file names, with a warning (`unsafe-path`). It extracts a name with a leading `\`
  without it (`leading-separator`). It extracts a name that is not UTF-8 as Windows-1252 (`non-utf8-filename`). It
  skips a second entry with the same path when case is ignored (`duplicate-entry`). dayzmake never writes outside
  the target folder, and it never follows a symbolic link in the way.
- Without `-f`, extraction stops with an error when the target folder already contains a file of the PBO. The error
  names the first file. dayzmake overwrites nothing.
- `--only` and `-x` select the entries to write. dayzmake writes an entry when it matches at least one `--only`
  pattern (or no `--only` pattern exists) and no `-x` pattern, so `-x` wins. `*` stands for any run of characters,
  as in the `-x` of `pack`. dayzmake compares the patterns with the entry path, with `/` separators and without the
  prefix. The comparison ignores case. dayzmake always writes `$PBOPREFIX$`. The conflict check without `-f` covers
  only the selected entries. The command `dayzmake unpack --only '*.c' --only '*config.bin' scripts.pbo out`
  extracts scripts and configs, but not models and textures.
- dayzmake decompresses compressed entries. A rapified `config.bin` stays rapified. Use
  [derapify](config.md#dayzmake-derapify) to read it.
- With `--use-prefix`, an unsafe prefix is an error. An empty prefix means no prefix directory.

Examples:

```text
$ dayzmake unpack mymod.pbo out
$ find out
out
out/scripts
out/config.bin
out/$PBOPREFIX$
out/scripts/4_world
out/scripts/4_world/player.c
```

```text
$ dayzmake unpack mymod.pbo out2 --use-prefix
$ find out2 -type f
out2/mymod/config.bin
out2/mymod/$PBOPREFIX$
out2/mymod/scripts/4_world/player.c
```

```text
$ dayzmake unpack mymod.pbo out
error: Target folder "out" already contains files of this PBO (first: "$PBOPREFIX$"). Use --force to overwrite.
```

This PBO has an entry name that starts with a separator and a header extension that contains a line break:

```text
$ dayzmake unpack odd.pbo odd-out
warning: Header extension "note" contains control characters; replaced them with spaces. [unsafe-header]
warning: Entry "\scripts\hello.c" starts with a path separator; extracting it as "scripts/hello.c". [leading-separator]
```

See also: [cat](#dayzmake-cat), [Header extensions and prefixes](header-extensions.md).

## dayzmake extract

Unpack a whole DayZ install, a mod or a single PBO into one tree, organised by prefix. Optionally, the command
derapifies the configs. It also writes a map from prefix to PBO.

<div class="synopsis">

```text
dayzmake extract [-v] [-f] [--derapify] [--only <pattern>]... [-x <excludepattern>]... <source> <targetfolder>
```

</div>

| Argument | Meaning |
|---|---|
| `<source>` | A DayZ client or server folder, a mod folder (`@Foo` or a workshop folder) or a single `.pbo`. |
| `<targetfolder>` | The folder for the tree. dayzmake creates the parent folder when it does not exist. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Print a summary (files, PBOs). |
| `-f`, `--force` | off | Replace an existing target folder. |
| `--derapify` | off | Write every rapified `*.bin` as text, with the extension `.cpp`. |
| `-x`, `--exclude <excludepattern>` | none | Glob pattern of entries to leave out. You can repeat it. |
| `--only <pattern>` | none | Glob pattern: write only the entries that match one. You can repeat it. |

Behaviour:

- For a folder, dayzmake reads every `*.pbo` directly in its `dta` and `addons` subfolders. The folder names match
  in any case (DayZ uses `Addons`). dayzmake does not search other folders or deeper levels. A folder without PBOs
  is an error. dayzmake processes the PBOs in sorted order of their path relative to `<source>`, so the result is
  deterministic.
- The entries of a PBO go to `<targetfolder>/<prefix>/<entry>`. An empty prefix means the file name of the PBO
  without `.pbo`. dayzmake validates entry names like those of [unpack](#dayzmake-unpack). It skips unsafe names
  with a warning (`unsafe-path`). It also skips a PBO whose prefix is not a safe folder name or is `pbo-map.tsv`.
  dayzmake does not write `$PBOPREFIX$`, because the map holds the prefixes.
- When two PBOs write the same path (case ignored), the later one in sorted order wins. The warning
  `duplicate-path` names both PBOs. Two entries of one PBO with the same path give `duplicate-entry`. A PBO can
  have a file where another PBO (or the same one) has a folder. Then dayzmake writes the first entry in that order
  and skips the later entry with the warning `extract-path-clash`. The extract does not fail.
- `--derapify`: dayzmake writes a `*.bin` that starts with the rapified signature (`config.bin` or any other) as
  `.cpp` of the same name, in the text that [derapify](config.md#dayzmake-derapify) prints. A file that dayzmake
  cannot read stays a `.bin`, with the warning `derapify-failed`. `--only` and `-x` match the entry name in the PBO
  (`config.bin`), as for `unpack`.
- dayzmake writes the tree to a staging folder next to `<targetfolder>`. The staging folder has the same parent and
  the name `.<name>.extract-<pid>-<n>`. When everything succeeds, dayzmake replaces `<targetfolder>` with a rename.
  Then it removes the old tree. A new run therefore drops files that no longer exist. If a PBO cannot be read,
  dayzmake leaves the old tree untouched and removes the staging folder.
- Without `-f`, an existing `<targetfolder>` is an error. With `-f`, dayzmake replaces it and deletes it after the
  swap. This works only when the folder is empty or holds a `pbo-map.tsv` at its top level, that is, when it looks
  like the result of an earlier `extract`. dayzmake refuses any other folder with an error that names it, so a wrong
  target cannot wipe a home folder. The target must also be a real folder (not a file or a symbolic link). It must
  not contain the source and must not lie inside it. `extract -f m m/addons` is refused, so the PBOs are never
  deleted. dayzmake also refuses a new `<targetfolder>` inside the source (`extract m m/out`), without `-f`,
  before it creates anything. It compares the paths with symbolic links resolved. dayzmake cannot resolve a new
  `<targetfolder>` with a `..` after a folder that does not exist (`elsewhere/missing/../x`). It refuses it with
  the error `Invalid target folder "<path>": cannot resolve ".." after a folder that does not exist.`
- Each problem with a PBO is an error that leaves the old tree alone. These problems are a PBO or entry that
  dayzmake cannot read (truncated, or a compressed entry that does not decompress) and a file that dayzmake cannot
  write. dayzmake skips only unsafe names, a path clash across entries and a `.bin` that it cannot derapify, with a
  warning.
- `<targetfolder>/pbo-map.tsv` has one line for each PBO that dayzmake read: `prefix<TAB>path of the PBO relative to
  <source>`, with `/` separators. The lines are sorted by prefix (then by path). A PBO without prefix is listed with
  its file name without `.pbo`. dayzmake writes the prefix with `/` instead of `\` (like the folders of the tree).
  Then it writes `\`, TAB, CR and LF in either column as `\\`, `\t`, `\r` and `\n`. Every `\` in the file
  therefore starts an escape, and a line reads back without doubt (a prefix `dz\gear\navigation` is
  `dz/gear/navigation`). `inspect --json` shows the prefix as stored. The map also lists the PBOs whose files
  dayzmake skipped because of their prefix.
- Memory: dayzmake copies an entry that is stored uncompressed from the PBO to its file in pieces. It reads only
  compressed entries into memory. With `--derapify`, it also reads `*.bin` files that start with the rapified
  signature into memory. The limit is 512 MiB for each file (a compressed entry over that limit is an error), so
  the largest PBOs extract in a small amount of memory.

Example:

```text
$ dayzmake extract --derapify --only '*.c' --only '*.bin' --only '*.cpp' ~/dayz-server game
$ cat game/pbo-map.tsv
dz/gear/camping	addons/gear_camping.pbo
dz/scripts	dta/scripts.pbo
$ ls game/dz/scripts
3_game  4_world  5_mission  config.cpp
```

```text
$ dayzmake extract install out
error: Target folder "out" already exists. Use --force to replace it.
```

See also: [unpack](#dayzmake-unpack), [derapify](config.md#dayzmake-derapify),
[Looking up the script API](../guides/script-api.md#reading-the-game-sources).

## dayzmake cat

Write one entry of a PBO to standard output or to a file.

<div class="synopsis">

```text
dayzmake cat [-v] [-f] <source> <filename> [<target>]
```

</div>

| Argument | Meaning |
|---|---|
| `<source>` | The PBO. |
| `<filename>` | The entry name exactly as `inspect` prints it, with `\` as separator (quote it in a shell: `'scripts\4_world\player.c'`). |
| `<target>` | The file to write. Standard output when omitted. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `-f`, `--force` | off | Overwrite an existing `<target>`. Without a `<target>` (standard output) it has no effect. |

Behaviour: dayzmake decompresses the entry when it is stored compressed. The name must match exactly (with the
case that the PBO uses). A name that is not in the PBO is an error. dayzmake does not overwrite an existing
`<target>` unless you use `-f`.

Examples:

```text
$ dayzmake cat mymod.pbo 'scripts\4_world\player.c'
modded class PlayerBase {}
```

```text
$ dayzmake cat mymod.pbo scripts/4_world/player.c
error: File "scripts/4_world/player.c" not found in PBO.
```

See also: [inspect](#dayzmake-inspect), [unpack](#dayzmake-unpack).
