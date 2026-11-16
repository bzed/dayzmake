# PBOs: build, pack, inspect, unpack, extract, cat

A PBO is the archive format of DayZ: a header with the *prefix* and other [header
extensions](header-extensions.md), a list of entries and a checksum. `build` makes the PBO of a mod folder the way
the game needs it, `pack` only archives, and `inspect`, `unpack` and `cat` read PBOs. PBO contents are treated as
untrusted: entry names with `..`, drive letters or absolute paths are never written outside the target folder.

The examples use the folder `mymod/` from [A first mod](../guides/first-mod.md).

## dayzmake build

Build a PBO from a folder: rapify configs, binarize models, check model textures, pack.

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
| `--proton-binarize` | off | Linux: binarize `.p3d` and `.rtm` files with DayZ Tools' `binarize.exe` under Proton. Without it models are copied as they are. See [Binarizing models on Linux](../guides/binarize-linux.md). On Windows `binarize.exe` is always used. |
| `-w <wname>`, `--warning <wname>` | none | Mute a [warning](warnings.md) by name. |
| `-i <includefolder>`, `--include <includefolder>` | none | Folder searched for absolute includes in configs and for textures of other mods when checking models. The current directory is only checked directly. |
| `-x <excludepattern>`, `--exclude <excludepattern>` | none | Glob pattern of files to leave out. Repeatable. See below. |
| `-e <headerext>`, `--headerext <headerext>` | none | Header extension `key=value`. Repeatable. See [Header extensions and prefixes](header-extensions.md). |
| `-k <privatekey>`, `--key <privatekey>` | none | Sign the PBO with this `.biprivatekey` after writing it. |
| `-s <signature>`, `--signature <signature>` | `<target>.<keyname>.bisign` | Where `-k` writes the signature. |

Behaviour:

- Files are visited in sorted order, so the same folder always gives the same bytes. No timestamps are stored.
- `config.cpp` is rapified and stored as `config.bin`. Every other `.cpp` and every `.rvmat` is rapified and keeps
  its name. A config that is already rapified is copied with the `prerapified-file` warning.
- `.p3d` and `.rtm` files are binarized (with `--proton-binarize` on Linux). A model that is already binarized
  (ODOL, BMTR) is copied with `prebinarized-file`. A `.p3do` file is packed as `.p3d`. Without
  `--proton-binarize` on Linux, models are copied and each one raises `non-windows-binarization`.
- Every editable (MLOD) model is checked for textures and materials that are not in the addon
  (`missing-texture`, `missing-material`). See [Textures and model checks](../guides/textures.md).
- A file named `$NOBIN$` or `$NOBIN-NOTEST$` in the source folder switches rapification and binarization off for
  the whole folder; the files are packed as they are.
- `$PBOPREFIX$` (or `$PREFIX$`) is read and not stored as an entry; its first bare line is the prefix and its
  `key=value` lines become header extensions.
- Files that are not regular files (FIFOs, devices, sockets) are skipped with `unsupported-file`. Names that
  are not valid as PBO entry names are skipped with `unsafe-path`. Names that collide ignoring case give
  `duplicate-entry` and the first in sorted order wins, except that a generated entry (the rapified `config.bin`)
  wins over a plain file of the same name.
- Exclude patterns use `*` as the only wildcard; it also matches `\`. A pattern is matched against the whole
  path inside the PBO written with `\`, before the `config.cpp` to `config.bin` rename and case-sensitively:
  `-x '*.psd'` leaves out every `.psd`, `-x 'data\*.png'` every PNG below `data`. Excluded files are skipped
  silently.
- The target and the temporary files next to it are never packed into themselves.
- With `-k` the PBO is signed with a version 3 signature after it was written (see [sign](signing.md#dayzmake-sign)).

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

Warnings of a model that references files outside the addon (here `t21` contains the test model `small.p3d`):

```text
$ dayzmake build t21 t21.pbo
In file small.p3d: warning: Model references texture "t21\data\gone_co.paa", which is not in the addon. [missing-texture]
In file small.p3d: warning: Model references material "t21\data\m.rvmat", which is not in the addon. [missing-material]
In file small.p3d: warning: On non-Windows systems binarize.exe is only used with --proton-binarize; file will be copied as-is. [non-windows-binarization]
```

See also: [pack](#dayzmake-pack), [Header extensions and prefixes](header-extensions.md), [Warnings](warnings.md).

## dayzmake pack

Pack a folder into a PBO without rapifying or binarizing anything.

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

Behaviour: the same rules as `build` for prefixes, exclusion, ordering and unsafe names, but file contents are
copied unchanged: `config.cpp` stays `config.cpp`, models stay as they are, and there are no model checks and no
include handling. Use it to repack a folder that was already built, or for PBOs that hold plain data.

Example (the folder `w` contains a name that is not allowed, a name that differs only in case, a FIFO and a
prefix with slashes):

```text
$ dayzmake pack . ../w.pbo -f
warning: Removed leading/trailing slashes from prefix "/leading/". [prefix-slash]
warning: Skipping "data/a.txt": an entry named "data\A.txt" was already packed. [duplicate-entry]
warning: Skipping "data/bad:name.txt": unsafe entry name. [unsafe-path]
In file data/pipe: warning: Skipping a file that is not a regular file (FIFO, device or socket). [unsupported-file]
```

A missing folder is named in the error:

```text
$ dayzmake pack nonexist x.pbo
error: Failed to read source folder "nonexist":
No such file or directory (os error 2)
```

See also: [build](#dayzmake-build), [unpack](#dayzmake-unpack).

## dayzmake inspect

List what a PBO contains, or describe a model.

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

- A file that starts with a model header is described as a model; anything else is read as a PBO.
- For a PBO: the header extensions, the number of files, and a table with name, packing method (0 stored,
  compressed entries are listed with their packed size), original size and stored size.
- For an editable MLOD model: format and version, the LODs with point, face, normal and UV-set counts, textures
  and materials with the number of faces that use them, properties and named selections. For a binarized ODOL
  or BMTR file only the format and version are known.
- Nothing is written; the command only reads.
- `--json` prints one JSON object on one line, with the fields in this order:
  - `header_extensions`: an ordered array of `{key, value, key_hex, value_hex}`;
  - `prefix` and `prefix_hex`: the prefix, or both `null` when the PBO has none;
  - `entries`: an array of `{name, name_hex, packing, original_size, data_size, timestamp}` in header order.

  `key`, `value`, `prefix` and `name` are the bytes decoded lossily as UTF-8 and only JSON-escaped (no
  control-character escaping of the text output); the `*_hex` fields are the lowercase hex of the raw bytes. A
  parser reads the hex, because the text of a name with a byte that is not UTF-8 has lost it. For a model,
  `--json` is an error: `--json is only supported for PBOs`.

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

Something that is not a PBO:

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
| `<targetfolder>` | The folder to extract into; it is created when missing. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Show every warning. |
| `-f`, `--force` | off | Overwrite files that already exist in the target folder. |
| `--use-prefix` | off | Extract into `<targetfolder>/<prefix>/` instead of directly into `<targetfolder>`. |
| `-x`, `--exclude <excludepattern>` | none | Glob pattern of entries to leave out. Repeatable. |
| `--only <pattern>` | none | Glob pattern: write only the entries that match one. Repeatable. |

Behaviour:

- The header extensions are written to `$PBOPREFIX$` in the extracted folder, so that `build` or `pack` of the
  folder gives the same prefix again. Control characters in extensions are replaced with spaces (`unsafe-header`).
- Entry names are validated before use. Absolute paths, drive letters, `..`, empty names and names that are not
  valid file names are skipped with a warning (`unsafe-path`); names with a leading `\` are extracted without it
  (`leading-separator`). A name that is not UTF-8 is extracted as Windows-1252 (`non-utf8-filename`), and a second
  entry with the same path ignoring case is skipped (`duplicate-entry`). Nothing is ever written outside the target folder, and a symbolic link in the way is
  never followed.
- Without `-f`, extraction stops with an error when the target folder already contains a file of the PBO, and
  names the first one. Nothing is overwritten.
- `--only` and `-x` select the entries to write: an entry is written when it matches at least one `--only`
  pattern (or none is given) and no `-x` pattern, so `-x` wins. `*` stands for any run of characters, as in the
  `-x` of `pack`. Patterns are matched against the entry path with `/` separators, without the prefix, ignoring
  case. `$PBOPREFIX$` is always written, and the conflict check without `-f` covers only the selected entries.
  `dayzmake unpack --only '*.c' --only '*config.bin' scripts.pbo out` extracts scripts and configs, not models
  and textures.
- Compressed entries are decompressed. A rapified `config.bin` stays rapified; use
  [derapify](config.md#dayzmake-derapify) to read it.
- With `--use-prefix` an unsafe prefix is an error. An empty prefix means no prefix directory.

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

A PBO with an entry name that starts with a separator and a header extension that contains a line break:

```text
$ dayzmake unpack odd.pbo odd-out
warning: Header extension "note" contains control characters; replaced them with spaces. [unsafe-header]
warning: Entry "\scripts\hello.c" starts with a path separator; extracting it as "scripts/hello.c". [leading-separator]
```

See also: [cat](#dayzmake-cat), [Header extensions and prefixes](header-extensions.md).

## dayzmake extract

Unpack a whole DayZ install, a mod or a single PBO into one tree organised by prefix, optionally derapified, with a
map from prefix to PBO.

<div class="synopsis">

```text
dayzmake extract [-v] [-f] [--derapify] [--only <pattern>]... [-x <excludepattern>]... <source> <targetfolder>
```

</div>

| Argument | Meaning |
|---|---|
| `<source>` | A DayZ client or server folder, a mod folder (`@Foo` or a workshop folder) or a single `.pbo`. |
| `<targetfolder>` | The folder for the tree; the parent is created when missing. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Print a summary (files, PBOs). |
| `-f`, `--force` | off | Replace an existing target folder. |
| `--derapify` | off | Write every rapified `*.bin` as text, with the extension `.cpp`. |
| `-x`, `--exclude <excludepattern>` | none | Glob pattern of entries to leave out. Repeatable. |
| `--only <pattern>` | none | Glob pattern: write only the entries that match one. Repeatable. |

Behaviour:

- For a folder, every `*.pbo` directly in its `dta` and `addons` subfolders is read; the folder names match in any
  case (DayZ uses `Addons`), other folders and deeper levels are not searched, and a folder without PBOs is an
  error. PBOs are processed in sorted order of their path relative to `<source>`, so the result is deterministic.
- The entries of a PBO go to `<targetfolder>/<prefix>/<entry>`. An empty prefix means the file name of the PBO
  without `.pbo`. Entry names are validated like those of [unpack](#dayzmake-unpack): unsafe names are skipped
  with a warning (`unsafe-path`), and so is a PBO whose prefix is not a safe folder name or is `pbo-map.tsv`;
  `$PBOPREFIX$` is not written, the map holds the prefixes.
- When two PBOs write the same path, ignoring case, the later one in sorted order wins and the warning
  `duplicate-path` names both PBOs. Two entries of one PBO with the same path give `duplicate-entry`.
- `--derapify`: a `*.bin` that starts with the rapified signature (`config.bin` or any other) is written as
  `.cpp` of the same name, in the text that [derapify](config.md#dayzmake-derapify) prints. One that cannot be
  read stays a `.bin`, with the warning `derapify-failed`. `--only` and `-x` match the entry name in the PBO
  (`config.bin`), as for `unpack`.
- The tree is written to a staging folder next to `<targetfolder>` (same parent, named
  `.<name>.extract-<pid>-<n>`) and replaces `<targetfolder>` with a rename when everything succeeded, then the old
  tree is removed. A re-run therefore drops files that no longer exist, and a failure (a PBO that cannot be read)
  leaves the old tree untouched and removes the staging folder.
- Without `-f`, an existing `<targetfolder>` is an error. With `-f` it is replaced and deleted after the swap, but
  only when it is an empty folder or holds a `pbo-map.tsv` at its top level, that is, looks like the result of an
  earlier `extract`; any other folder is refused with an error that names it, so a wrong target cannot wipe a home
  folder. It must also be a real folder (not a file or a symbolic link), must not contain the source and must not lie
  inside it: `extract -f m m/addons` is refused, so the PBOs are never deleted.
- Everything that goes wrong with a PBO is an error that leaves the old tree alone: a PBO or entry that cannot be
  read (truncated, a compressed entry that does not decompress) and a file that cannot be written. Only unsafe names
  and a `.bin` that cannot be derapified are skipped, with a warning.
- `<targetfolder>/pbo-map.tsv` has one line per PBO that was read, `prefix<TAB>path of the PBO relative to
  <source>` with `/` separators, sorted by prefix (then by path). A PBO without prefix is listed with its file
  name without `.pbo`. The prefix is written with `/` instead of `\` (like the folders of the tree), and then `\`, TAB, CR and
  LF in either column are written as `\\`, `\t`, `\r` and `\n`, so every `\` in the file starts an escape and a
  line reads back unambiguously (a prefix `dz\gear\navigation` is `dz/gear/navigation`). The prefix as stored is in
  `inspect --json`. The PBOs whose files were skipped because of their prefix are listed too.
- Memory: an entry that is stored uncompressed is copied from the PBO to its file in pieces. Only compressed entries
  and, with `--derapify`, `*.bin` files that start with the rapified signature are read into memory, up to 512 MiB
  each (a compressed entry over that is an error), so the largest PBOs extract in a small amount of memory.

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

Behaviour: the entry is decompressed when it is stored compressed. The name must match exactly (including the
case the PBO uses); a name that is not in the PBO is an error. An existing `<target>` is not overwritten unless
you pass `-f`.

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
