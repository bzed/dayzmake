# A first mod, from folder to signed PBO

This guide builds a small script mod from an empty folder. It signs the mod and puts the files in the layout that
DayZ servers and the Steam Workshop expect. It takes five minutes and needs only dayzmake. We ran every command
below on a clean folder. The output is what dayzmake printed.

## 1. The source folder

A mod addon is a folder. Its name, or the line in `$PBOPREFIX$`, is the *prefix*. The prefix is the path under
which the game sees the files. Create this layout:

```text
mymod/
  $PBOPREFIX$
  config.cpp
  scripts/4_world/player.c
```

`$PBOPREFIX$` has one line, the prefix:

```text
mymod
```

`config.cpp` registers the addon and its script module:

```cpp
class CfgPatches
{
    class MyMod_Scripts
    {
        units[] = {};
        weapons[] = {};
        requiredVersion = 0.1;
        requiredAddons[] = {"DZ_Data"};
    };
};
class CfgMods
{
    class MyMod
    {
        dir = "MyMod";
        name = "My Mod";
        type = "mod";
        dependencies[] = {"World"};
        class defs
        {
            class worldScriptModule
            {
                value = "";
                files[] = {"MyMod/scripts/4_world"};
            };
        };
    };
};
```

`scripts/4_world/player.c` is the script:

```c
modded class PlayerBase {}
```

## 2. Build

`build` rapifies `config.cpp` to `config.bin`, packs the scripts and writes the PBO:

```text
$ dayzmake build mymod mymod.pbo
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

The prefix came from `$PBOPREFIX$`. The PBO stores `config.cpp` as `config.bin`. If `build` prints warnings, read
them. They point at real mistakes (see [Warnings](../reference/warnings.md)). If you build the same folder again,
you get the same bytes.

## 3. Make a key pair

A key pair belongs to you, not to a mod. Use the same key pair for all your mods. Then a server needs only one
`.bikey` of yours.

```text
$ dayzmake keygen mykey
$ ls mykey.*
mykey.bikey  mykey.biprivatekey
```

`mykey.biprivatekey` is the secret half. Only you can read it. Do not commit it and do not upload it. Other people
can use it to sign files in your name. `mykey.bikey` is public. Give it to servers and put it in your mod.

## 4. Build and sign in one step

```text
$ dayzmake build -k mykey.biprivatekey -f mymod mymod.pbo
$ ls mymod*
mymod.pbo  mymod.pbo.mykey.bisign
```

The signature file has the name of the PBO and the name of the key. Keep it next to the PBO under that name. (`-f`
overwrites the PBO of step 2.) You can also sign an existing PBO: `dayzmake sign mykey.biprivatekey mymod.pbo`.

## 5. Check it

```text
$ dayzmake verify mykey.bikey mymod.pbo
$ echo $?
0
```

`verify` prints nothing when the signature is valid. After a change to the PBO, the signature is not valid. Sign
the PBO again after each rebuild. `build -k` does this for you.

## 6. The Workshop and server layout

Servers load a mod folder with `-mod=@MyMod`, and you upload the same folder to the Steam Workshop. It looks like
this:

```text
@MyMod/
  addons/
    mymod.pbo
    mymod.pbo.mykey.bisign
  keys/
    mykey.bikey
```

```text
$ mkdir -p @MyMod/addons @MyMod/keys
$ cp mymod.pbo mymod.pbo.mykey.bisign @MyMod/addons/
$ cp mykey.bikey @MyMod/keys/
```

Upload tools can add a `meta.cpp` with the name and the Workshop id of the mod to this folder. You do not need to
write one. If the file has a `name` line, dayzmake reads it to show a name in `dayzmake api mods`.

The server owner copies `mykey.bikey` into the `keys` folder of the server. The server then accepts clients that
load PBOs signed with `mykey`. The server rejects a client without a matching signature with a signature error.

With this layout, you can ask dayzmake about the scripts of the mod: `dayzmake api -m @MyMod class PlayerBase`
(see [Looking up the script API](script-api.md)).

## 7. Looking inside

```text
$ dayzmake cat mymod.pbo 'scripts\4_world\player.c'
modded class PlayerBase {}
$ dayzmake unpack mymod.pbo out
$ find out -type f
out/config.bin
out/$PBOPREFIX$
out/scripts/4_world/player.c
```

The `config.bin` in the unpacked folder is rapified. To read it, use `dayzmake derapify out/config.bin`.

## Where to go next

- Configs with includes and macros: [Config files and the preprocessor](configs.md).
- Models and textures: [Textures and model checks](textures.md) and [Binarizing models on Linux](binarize-linux.md).
- All options of the commands used here: [PBOs](../reference/pbo.md) and [Keys and signatures](../reference/signing.md).
