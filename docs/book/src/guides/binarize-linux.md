# Binarizing models on Linux

DayZ models are authored as editable `.p3d` files (MLOD) and must be converted to the binary format (ODOL)
before they are packed into a mod for release. The converter is Bohemia's `binarize.exe`, which only runs on
Windows. On Linux dayzmake runs it for you under Proton, the compatibility layer of Steam.

Without any setup, `dayzmake build` still works on Linux: it copies `.p3d` and `.rtm` files as they are and warns
with `non-windows-binarization`. This guide sets up real binarization.

> The commands that start `binarize.exe` could not be run for this manual, because it needs DayZ Tools and
> Proton, which are not part of the repository. The failure messages shown below are real; the description of a
> successful run follows the program's source and README.

## What you need

1. **Steam** with the **DayZ Tools** app installed (Steam app 830640, in the Tools category of your library).
2. **Proton**, installed through Steam: any of Proton Hotfix, Proton - Experimental, Proton Experimental, or a
   numbered Proton. Start any Windows game with it once, or install it from the Steam Tools list.
3. dayzmake finds both through the Steam folder. If your Steam is in a usual place (`~/.steam/steam`,
   `~/.steam/debian-installation`, `~/.local/share/Steam`) nothing needs to be set.

## First try

```text
$ dayzmake build --proton-binarize mymod mymod.pbo
```

The first model needs a Wine prefix, which is created in a temporary sandbox and takes up to a few minutes
on a cold start (dayzmake says so: `Creating a Wine prefix for binarize.exe (the first run takes a while)...`).
Later models in the same run are quick. The sandbox is removed at the end of the run and nothing in the Steam
folder is changed.

For a single model:

```text
$ dayzmake binarize mymod/model.p3d model_binarized.p3d
```

`binarize` treats the folder of the model as the addon and its name as the prefix. That is right only for a model
in the root of a single-component addon. For a model in a subfolder (`data\`) or an addon with a prefix like
`my\mod`, use `build --proton-binarize`, which reads the prefix from `$PBOPREFIX$`.

## Telling dayzmake where things are

The variables are all optional; set them in your shell or in front of the command.

| Variable | Use |
|---|---|
| `STEAM_ROOT` | The Steam folder, if it is not in a usual place. It must contain `steamapps/common`. |
| `DAYZ_TOOLS` | The DayZ Tools folder, if it is not `<steam>/steamapps/common/DayZ Tools`. It must contain `Bin/Binarize/binarize.exe`. |
| `PROTON` | The Proton folder (the one with the `proton` script). Default: Proton Hotfix, then Proton - Experimental, then Proton Experimental, then the alphabetically last `Proton*` folder. |
| `BIOUTPUT=1` | Show what `binarize.exe` and Proton print. Use it when a model fails. |

The error messages name what is missing and the variable that fixes it:

```text
$ STEAM_ROOT=/nonexistent HOME=/nonexistent dayzmake binarize t21/small.p3d -f x.p3d
error: No Steam installation found (set STEAM_ROOT, or DAYZ_TOOLS to the DayZ Tools folder).
$ DAYZ_TOOLS=/nonexistent dayzmake binarize t21/small.p3d -f x.p3d
error: binarize.exe not found in "/nonexistent". Install DayZ Tools (Steam app 830640) or set DAYZ_TOOLS.
$ dayzmake binarize t21/small.p3d -f x.p3d      # DayZ Tools present, no Proton installed
error: No Proton found under "…/steam/steamapps/common". Install one in Steam or set PROTON.
```

## Textures: why paths matter

`binarize.exe` resolves texture and material paths such as `my\mod\data\x_ca.paa` relative to a project root and
sets a model's alpha flags only if it can read the texture. dayzmake therefore gives the sandbox a `P:` drive with
your addon linked at its prefix (`P:\my\mod` points at your source folder) and runs `binarize.exe` from `P:\`.
Paths outside your addon (vanilla `dz\...`) stay unresolved, and `binarize.exe`'s complaint about them is shown
as the warning `binarize-missing-texture`. The model is still produced.

The source folder is linked, not copied, so `binarize.exe` could in principle write into it. Its output goes to
the sandbox, and dayzmake does not change your source folder.

## Models that are already binarized

A model that is already binarized (`ODOL`, or `BMTR` for animations) never reaches `binarize.exe`. It is copied
unchanged with the warning `prebinarized-file`, which names the format and version (for example `ODOL v58`).
Any version is accepted; dayzmake cannot know which ones the game loads.

## Checking a build

`build` checks every editable model for textures and materials that are missing from the addon, with or without
`--proton-binarize` (see [Textures and model checks](textures.md)). A good routine on Linux:

1. `dayzmake build mymod mymod.pbo` while you work: fast, copies models, reports missing files.
2. `dayzmake build --proton-binarize -k mykey.biprivatekey -f mymod mymod.pbo` for the release.
3. `dayzmake inspect mymod.pbo` to see what was packed and how big it is.

See [binarize](../reference/binarize.md) and [Environment and files](../reference/environment.md) for details.
