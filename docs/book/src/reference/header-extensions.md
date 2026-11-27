# Header extensions and prefixes

A PBO starts with a header that holds *header extensions*: `key=value` pairs. The most important one is `prefix`.
It is the virtual path of the contents of the PBO. A PBO with the prefix `mymod\scripts` gives its file
`config.bin` the path `mymod\scripts\config.bin` in the game. Textures, models and includes use these paths to
refer to files. Other extensions carry information, for example the product that the PBO was made for.

## Where extensions come from

`build` and `pack` collect them in this order. A later source wins:

1. The file `$PBOPREFIX$` (or `$PREFIX$`) in the source folder. Each line is one setting. dayzmake stops reading at
   the first empty line. A line without `=` is the prefix. A line `key=value` sets that extension, and
   `prefix=value` sets the prefix. A repeated key keeps its first position and takes the last value. dayzmake does
   not pack the file itself as an entry.
2. `-e key=value` on the command line, repeatable. An entry without `=` is an error:
   `Invalid header extension "x" (expected key=value).`
3. If no prefix was given, the name of the source folder.

dayzmake does not add any extension on its own (no default `product` or `version`). The header always lists the
prefix first. Then it lists the other extensions in this order: those of `$PBOPREFIX$` in file order, then the new
ones of `-e`. An `-e` for a key that already exists replaces its value in place.

## The prefix

- dayzmake changes slashes to `\`. It removes a leading or trailing separator and gives the `prefix-slash` warning.
- If there is no `$PBOPREFIX$`, the prefix is the name of the source folder: `dayzmake build mymod` gives `mymod`.
  dayzmake checks models against this prefix. It expects a texture `mymod\data\a_co.paa` in the PBO. It does not
  expect a texture of another prefix.
- A model that `binarize.exe` processes sees the addon below its prefix (`P:\<prefix>`), so textures resolve
  as in the game.
- `unpack` writes the prefix and all other extensions to `$PBOPREFIX$` in the extracted folder.
  `unpack --use-prefix` puts the files below `<target>/<prefix>/`.

## Example

A folder `m2` with this `$PBOPREFIX$`:

```text
m2
product=dayz
```

gives these extensions, which `-e` can add to or override:

```text
$ dayzmake build m2 m2.pbo
$ dayzmake inspect m2.pbo | head -4
Header extensions:
- prefix=m2
- product=dayz
$ dayzmake build -e author=bzed -e product=dz m2 -f m2.pbo
$ dayzmake inspect m2.pbo | head -5
Header extensions:
- prefix=m2
- product=dz
- author=bzed
```

## Exclude patterns

The patterns of `-x` are not header extensions. They decide which files go into the PBO. A pattern is a string with
`*` as the only wildcard. dayzmake compares it with the whole path inside the PBO (with `\` as the separator) and
does match case:

| Pattern | Leaves out |
|---|---|
| `*.psd` | every file ending in `.psd`, anywhere |
| `data\*.png` | every `.png` below `data` |
| `source\*` | everything below `source` |
| `*\Thumbs.db` | `Thumbs.db` in any subfolder (a pattern without `*` in front only matches the root) |

There are no default patterns. dayzmake packs every file unless you exclude it. This includes hidden files and,
for `build`, the marker files `$NOBIN$` and `$NOBIN-NOTEST$`. dayzmake skips excluded files without a warning.

## Prefix and header values in untrusted PBOs

Values in a PBO header are untrusted. dayzmake never makes a path from them without a check. An unsafe prefix is
an error for `unpack --use-prefix` and a warning (`api-unsafe-prefix`) for the script API. When dayzmake writes
`$PBOPREFIX$`, it replaces control characters in a value with spaces (`unsafe-header`).
