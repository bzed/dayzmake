# Models: binarize

You write models (`.p3d`) and animations (`.rtm`) in an editable format (MLOD). `binarize.exe` from DayZ Tools, a
program of Bohemia, converts them to the binary format of the game (ODOL, BMTR). dayzmake does not implement this
conversion again. It runs `binarize.exe`: directly on Windows and under Proton on Linux. The guide
[Binarizing models on Linux](../guides/binarize-linux.md) explains the setup. [Environment and files](environment.md)
lists the environment variables.

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

- If the file is already binarized (an `ODOL` model or a `BMTR` animation), dayzmake copies it unchanged and gives
  the `prebinarized-file` warning. The warning names the format and the version (for example `ODOL v58`). dayzmake
  accepts any version. It does not know which versions the game loads. It does not start `binarize.exe` for this
  file.
- Otherwise, dayzmake starts `binarize.exe`. On Windows, dayzmake finds it through the registry key
  `HKEY_CURRENT_USER\Software\Bohemia Interactive\binarize`, value `path`, file `binarize_x64.exe`. On Linux, it
  runs under Proton from DayZ Tools (Steam app 830640). If DayZ Tools, Steam or Proton is missing, dayzmake gives
  an error. The error names the missing item and the variable to set (`STEAM_ROOT`, `DAYZ_TOOLS`, `PROTON`).
- dayzmake treats the folder of the model as the addon and the name of the folder as the prefix. This works only
  for a model in the root of an addon that has one component. For a model in a subfolder (`data\`), or an addon
  whose prefix has several components (`my\mod`), use [build](pbo.md#dayzmake-build) with `--proton-binarize`. It
  takes the prefix from `$PBOPREFIX$`.
- On Linux, the first model of a run is slow. dayzmake creates a Wine prefix in a temporary sandbox, and it removes
  the sandbox when the command ends. It does not change anything in the Steam installation.
- `BIOUTPUT=1` shows the output of `binarize.exe` and Proton. Otherwise, dayzmake shows it only when the run fails.

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

These examples fail on purpose, because the test machine for this documentation has no DayZ Tools. A successful
run writes the binarized model to `<target>` and prints the progress notes of the Proton sandbox.

See also: [Binarizing models on Linux](../guides/binarize-linux.md), [build](pbo.md#dayzmake-build),
[Environment and files](environment.md).
