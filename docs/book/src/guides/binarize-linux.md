# Binarizing models on Linux

You make DayZ models as editable `.p3d` files (MLOD). Before you pack a model into a mod for release, convert it to
the binary format (ODOL). The converter is `binarize.exe` from Bohemia. It runs only on Windows. On Linux, dayzmake
runs it for you under Proton, the compatibility layer of Steam.

Without setup, `dayzmake build` also works on Linux. It copies `.p3d` and `.rtm` files as they are and gives the
warning `non-windows-binarization`. This guide shows how to get real binarization.

> We could not run the commands that start `binarize.exe` for this manual. They need DayZ Tools and Proton, and the
> repository does not contain them. The failure messages below are real. The description of a successful run comes
> from the source code and the README of the program.

## What you need

1. **Steam** with the **DayZ Tools** app installed (Steam app 830640, in the Tools category of your library).
2. **Proton**, installed through Steam: Proton Hotfix, Proton - Experimental, Proton Experimental, or a numbered
   Proton. Start a Windows game with it once, or install it from the Steam Tools list.
3. dayzmake finds both through the Steam folder. If Steam is in a usual place (`~/.steam/steam`,
   `~/.steam/debian-installation`, `~/.local/share/Steam`), you do not set anything.

## First try

```text
$ dayzmake build --proton-binarize mymod mymod.pbo
```

The first model needs a Wine prefix. dayzmake makes it in a temporary sandbox. On a cold start this can take a few
minutes, and dayzmake says so: `Creating a Wine prefix for binarize.exe (the first run takes a while)...`. Later
models in the same run are quick. dayzmake removes the sandbox at the end of the run. It does not change anything in
the Steam folder.

For a single model:

```text
$ dayzmake binarize mymod/model.p3d model_binarized.p3d
```

`binarize` uses the folder of the model as the addon and the name of the folder as the prefix. This is correct only
for a model in the root of an addon with a one-part prefix. For a model in a subfolder (`data\`) or an addon with a
prefix such as `my\mod`, use `build --proton-binarize`. It reads the prefix from `$PBOPREFIX$`.

## Telling dayzmake where things are

All variables are optional. Set them in your shell or in front of the command.

| Variable | Use |
|---|---|
| `STEAM_ROOT` | The Steam folder, if it is not in a usual place. It must contain `steamapps/common`. |
| `DAYZ_TOOLS` | The DayZ Tools folder, if it is not `<steam>/steamapps/common/DayZ Tools`. It must contain `Bin/Binarize/binarize.exe`. |
| `PROTON` | The Proton folder (the one with the `proton` script). Default: Proton Hotfix, then Proton - Experimental, then Proton Experimental, then the alphabetically last `Proton*` folder. |
| `BIOUTPUT=1` | Show what `binarize.exe` and Proton print. Use it when a model fails. |

Each error message names the missing item and the variable that fixes it:

```text
$ STEAM_ROOT=/nonexistent HOME=/nonexistent dayzmake binarize t21/small.p3d -f x.p3d
error: No Steam installation found (set STEAM_ROOT, or DAYZ_TOOLS to the DayZ Tools folder).
$ DAYZ_TOOLS=/nonexistent dayzmake binarize t21/small.p3d -f x.p3d
error: binarize.exe not found in "/nonexistent". Install DayZ Tools (Steam app 830640) or set DAYZ_TOOLS.
$ dayzmake binarize t21/small.p3d -f x.p3d      # DayZ Tools present, no Proton installed
error: No Proton found under "…/steam/steamapps/common". Install one in Steam or set PROTON.
```

## Textures: why paths matter

`binarize.exe` finds texture and material paths such as `my\mod\data\x_ca.paa` relative to a project root. It sets
the alpha flags of a model only if it can read the texture. Therefore dayzmake gives the sandbox a `P:` drive with
your addon linked at its prefix (`P:\my\mod` points at your source folder). It runs `binarize.exe` from `P:\`.
Paths outside your addon (vanilla `dz\...`) stay unresolved. dayzmake shows the complaint of `binarize.exe` about
them as the warning `binarize-missing-texture`. The model is still made.

dayzmake links the source folder and does not copy it, so `binarize.exe` could write into it. The output goes to the
sandbox, and dayzmake does not change your source folder.

## Models that are already binarized

A model that is already binarized (`ODOL`, or `BMTR` for animations) never goes to `binarize.exe`. dayzmake copies
it without change and gives the warning `prebinarized-file`. The warning names the format and the version (for
example `ODOL v58`). dayzmake accepts each version, because it cannot know which versions the game loads.

## Checking a build

`build` checks each editable model for textures and materials that are not in the addon. It does this with or
without `--proton-binarize` (see [Textures and model checks](textures.md)). A good routine on Linux:

1. While you work, run `dayzmake build mymod mymod.pbo`. It is fast, copies the models and reports missing files.
2. For the release, run `dayzmake build --proton-binarize -k mykey.biprivatekey -f mymod mymod.pbo`.
3. Run `dayzmake inspect mymod.pbo` to see what dayzmake packed and how big the files are.

For more information, see [binarize](../reference/binarize.md) and [Environment and files](../reference/environment.md).
