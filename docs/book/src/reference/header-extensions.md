# Header extensions and prefixes

A PBO starts with a header that holds *header extensions*: `key=value` pairs. The most important one is `prefix`,
the virtual path of the PBO's contents. A PBO with the prefix `mymod\scripts` makes its file `config.bin` known
to the game as `mymod\scripts\config.bin`; textures, models and includes refer to files by these paths. Other
extensions carry information such as the product the PBO was made for.

## Where extensions come from

`build` and `pack` collect them in this order; a later source wins:

1. The file `$PBOPREFIX$` (or `$PREFIX$`) in the source folder. Each line is one setting; reading stops at the
   first empty line. A line without `=` is the prefix. A line `key=value` sets that extension, and `prefix=value`
   sets the prefix. A repeated key keeps its first position and takes the last value. The file itself is not
   packed as an entry.
2. `-e key=value` on the command line, repeatable. An entry without `=` is an error:
   `Invalid header extension "x" (expected key=value).`
3. If no prefix was given, the name of the source folder.

dayzmake does not add any extension on its own (no default `product` or `version`). The header always lists the prefix first, then the other extensions in this order: those of `$PBOPREFIX$` in file order, then new ones of `-e`; an `-e` for a key that already
exists replaces its value in place.

## The prefix

- Slashes are normalised to `\` and a leading or trailing separator is removed with the `prefix-slash` warning.
- With no `$PBOPREFIX$` the prefix is the name of the source folder: `dayzmake build mymod` gives `mymod`.
  Models are checked against this prefix: a texture `mymod\data\a_co.paa` is expected in the PBO, a texture of
  another prefix is not.
- A model that `binarize.exe` processes sees the addon below its prefix (`P:\<prefix>`), so textures resolve
  as in the game.
- `unpack` writes the prefix and all other extensions to `$PBOPREFIX$` in the extracted folder, and
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

Patterns of `-x` are not header extensions but decide which files enter the PBO. A pattern is a string with
`*` as the only wildcard, compared with the whole path inside the PBO (with `\` as the separator), case-sensitively:

| Pattern | Leaves out |
|---|---|
| `*.psd` | every file ending in `.psd`, anywhere |
| `data\*.png` | every `.png` below `data` |
| `source\*` | everything below `source` |
| `*\Thumbs.db` | `Thumbs.db` in any subfolder (a pattern without `*` in front only matches the root) |

There are no default patterns: every file is packed unless excluded, including hidden files and, for `build`,
the marker files `$NOBIN$` and `$NOBIN-NOTEST$`. Excluded files are skipped without a warning.

## Prefix and header values in untrusted PBOs

Values in a PBO header are untrusted. dayzmake never builds a path from them without checking: an unsafe
prefix is an error for `unpack --use-prefix` and a warning (`api-unsafe-prefix`) for the script API; control
characters in a value are replaced by spaces when `$PBOPREFIX$` is written (`unsafe-header`).
