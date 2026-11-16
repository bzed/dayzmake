# A first mod, from folder to signed PBO

This guide builds a small script mod from an empty folder, signs it and lays it out the way DayZ servers and the
Steam Workshop expect. It takes five minutes and needs only dayzmake. Every command below was run on a clean
folder; the output is what dayzmake printed.

## 1. The source folder

A mod addon is a folder. Its name, or the line in `$PBOPREFIX$`, becomes the *prefix*: the path under which the
game sees the files. Create this layout:

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

The prefix came from `$PBOPREFIX$`; `config.cpp` is stored as `config.bin`. If `build` printed warnings,
read them: they point at real mistakes (see [Warnings](../reference/warnings.md)). Building the same folder
again gives the same bytes.

## 3. Make a key pair

A key pair belongs to you, not to a mod: use the same one for all your mods, so that servers need only one
`.bikey` of yours.

```text
$ dayzmake keygen mykey
$ ls mykey.*
mykey.bikey  mykey.biprivatekey
```

`mykey.biprivatekey` is the secret half and is created readable by you only. Never commit it, never upload it.
`mykey.bikey` is public; it goes to servers and into your mod.

## 4. Build and sign in one step

```text
$ dayzmake build -k mykey.biprivatekey -f mymod mymod.pbo
$ ls mymod*
mymod.pbo  mymod.pbo.mykey.bisign
```

The signature file is named after the PBO and the key. It must stay next to the PBO under that name. (`-f`
overwrites the PBO of step 2.) You can also sign an existing PBO: `dayzmake sign mykey.biprivatekey mymod.pbo`.

## 5. Check it

```text
$ dayzmake verify mykey.bikey mymod.pbo
$ echo $?
0
```

`verify` prints nothing when the signature is valid. After any change to the PBO the signature is wrong, so sign
again after every rebuild; `build -k` does that for you.

## 6. The Workshop and server layout

A mod folder, as it is uploaded to the Steam Workshop and as servers load it with `-mod=@MyMod`, looks like
this:

```text
@MyMod/
  addons/
    mymod.pbo
    mymod.pbo.mykey.bisign
  keys/
    mykey.bikey
  meta.cpp
```

```text
$ mkdir -p @MyMod/addons @MyMod/keys
$ cp mymod.pbo mymod.pbo.mykey.bisign @MyMod/addons/
$ cp mykey.bikey @MyMod/keys/
```

A server owner copies `mykey.bikey` into the `keys` folder of the server; the server then accepts clients that
load PBOs signed with `mykey`. A client without a matching signature is rejected with a signature error.

With a mod laid out like this you can ask dayzmake about its scripts: `dayzmake api -m @MyMod class PlayerBase`
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

`config.bin` in the unpacked folder is rapified; read it with `dayzmake derapify out/config.bin`.

## Where to go next

- Configs with includes and macros: [Config files and the preprocessor](configs.md).
- Models and textures: [Textures and model checks](textures.md) and [Binarizing models on Linux](binarize-linux.md).
- All options of the commands used here: [PBOs](../reference/pbo.md) and [Keys and signatures](../reference/signing.md).
