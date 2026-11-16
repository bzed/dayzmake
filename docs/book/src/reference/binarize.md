# Models: binarize

Models (`.p3d`) and animations (`.rtm`) are authored in an editable format (MLOD) and converted to the game's
binary format (ODOL, BMTR) by Bohemia's `binarize.exe` from DayZ Tools. dayzmake does not reimplement that
conversion: it runs `binarize.exe`, directly on Windows and under Proton on Linux. The guide
[Binarizing models on Linux](../guides/binarize-linux.md) walks through the setup; the environment variables are
listed in [Environment and files](environment.md).

## dayzmake binarize

Binarize one model.

<div class="synopsis">

```text
dayzmake binarize [-v] [-f] [-w <wname>]... <source> <target>
```

</div>

| Argument | Meaning |
|---|---|
| `<source>` | The model to binarize. |
| `<target>` | The file to write. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Show every warning. |
| `-f`, `--force` | off | Overwrite an existing `<target>`. |
| `-w <wname>`, `--warning <wname>` | none | Mute a [warning](warnings.md) by name. |

Behaviour:

- A file that is already binarized (an `ODOL` model or a `BMTR` animation) is copied unchanged with the
  `prebinarized-file` warning, which names the format and version (for example `ODOL v58`). Any version is
  accepted; dayzmake does not know which ones the game loads. `binarize.exe` is not started for it.
- Otherwise `binarize.exe` is started. On Windows it is found through the registry key
  `HKEY_CURRENT_USER\Software\Bohemia Interactive\binarize`, value `path`, file `binarize_x64.exe`. On Linux it
  runs under Proton from DayZ Tools (Steam app 830640); a missing DayZ Tools, Steam or Proton is an error that
  names what is missing and the variable to set (`STEAM_ROOT`, `DAYZ_TOOLS`, `PROTON`).
- The model's folder is treated as the addon and its name as the prefix. This only works for a model in the
  root of a single-component addon. For a model in a subfolder (`data\`), or an addon whose prefix has several
  components (`my\mod`), use [build](pbo.md#dayzmake-build) with `--proton-binarize`, which takes the prefix from
  `$PBOPREFIX$`.
- The first model of a run is slow on Linux because a Wine prefix is created in a temporary sandbox; the
  sandbox is removed when the command ends. Nothing in the Steam installation is modified.
- `BIOUTPUT=1` shows the output of `binarize.exe` and Proton; otherwise it is shown only when the run fails.

Examples:

```text
$ dayzmake binarize t21/small.p3d x.p3d
error: binarize.exe not found in "…/steam/steamapps/common/DayZ Tools". Install DayZ Tools (Steam app 830640) or set DAYZ_TOOLS.
```

```text
$ DAYZ_TOOLS=/nonexistent dayzmake binarize t21/small.p3d -f x.p3d
error: binarize.exe not found in "/nonexistent". Install DayZ Tools (Steam app 830640) or set DAYZ_TOOLS.
```

```text
$ STEAM_ROOT=/nonexistent HOME=/nonexistent dayzmake binarize t21/small.p3d -f x.p3d
error: No Steam installation found (set STEAM_ROOT, or DAYZ_TOOLS to the DayZ Tools folder).
```

With DayZ Tools present but no Proton in the Steam library:

```text
$ dayzmake binarize t21/small.p3d -f x.p3d
error: No Proton found under "…/steam/steamapps/common". Install one in Steam or set PROTON.
```

These examples fail on purpose, because the test machine for this documentation has no DayZ Tools; a successful
run writes the binarized model to `<target>` and prints the progress notes of the Proton sandbox.

See also: [Binarizing models on Linux](../guides/binarize-linux.md), [build](pbo.md#dayzmake-build),
[Environment and files](environment.md).
