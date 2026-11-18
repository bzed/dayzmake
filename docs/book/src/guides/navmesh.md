# Navigation meshes for a terrain mod

A custom terrain needs a **navmesh** so that infected, animals and AI can walk on it. The game cannot compute one
from the terrain files alone: it asks a running `DayZDiag_x64.exe` for the geometry of the world, and
`dayzmake navmesh generate` does that and builds the `.nm` file. This guide goes from a built terrain mod to a navmesh
in the mod. The [reference](../reference/navmesh.md) lists every command and option.

> The steps that start the game could not be run for this manual, because they need DayZ and a terrain mod, which
> are not part of the repository. The `dayzmake` outputs shown here are real: they were produced against a test
> server that serves a small synthetic world. The description of the game side follows the way the game is started
> for such work.

## What you need

- A built **terrain mod**: the PBOs of your world (see [A first mod](first-mod.md) for building PBOs). It already
  loads in the game, only without a navmesh, or with an old one.
- **DayZ** (the client, Steam app 221100), which contains `DayZDiag_x64.exe`, the diagnostic build that has the
  navmesh data server.
- On Linux, **Proton** (see [Binarizing models on Linux](binarize-linux.md) for installing it through Steam).
- A few gigabytes of free memory for the game and, for a large map, time: the tiles of a big world take minutes to
  hours (see *Speed* below).

## 1. Start the data server

`DayZDiag_x64.exe` is started like a dedicated server with the extra option `-startNavmeshDataServer`. It loads
your world and then listens on TCP port **36533** for requests for geometry. The port is fixed by the game; the
normal game port is not, and must not be the default, so that a real server on the same machine is not disturbed.

### On Windows

Make a server configuration file, `serverDZ_navmesh.cfg`, next to the game, that does not check signatures and
does not start BattlEye, and an empty mission: a folder `empty.<yourworld>` (the name after the dot is the world)
whose `init.c` has an empty `main()`:

```text
void main()
{
}
```

The configuration needs these lines (the rest is a normal `serverDZ.cfg`; its `Missions` class names the mission
folder):

```text
verifySignatures = 0;
BattlEye = 0;
```

Start the server from the DayZ folder, with your mod and a game port that is not 2302:

```text
DayZDiag_x64.exe -server -startNavmeshDataServer -config=serverDZ_navmesh.cfg -mission=<path of the mission folder> -port=2402 "-mod=@MyTerrain"
```

`@MyTerrain` is the folder of your mod in the DayZ folder. Wait until the log shows the world loaded. Keep the
window open while you generate.

### On Linux, under Proton

The Windows program runs through Proton, from the **client** installation (`steamapps/common/DayZ`), which has
`DayZDiag_x64.exe`. Run it the way Steam would, with the environment of the game:

```text
export STEAM_ROOT="$HOME/.local/share/Steam"            # your Steam folder
export STEAM_COMPAT_CLIENT_INSTALL_PATH="$STEAM_ROOT"
export STEAM_COMPAT_DATA_PATH="$STEAM_ROOT/steamapps/compatdata/221100"
export STEAM_COMPAT_INSTALL_PATH="$STEAM_ROOT/steamapps/common/DayZ"
export STEAM_COMPAT_APP_ID=221100 SteamAppId=221100 SteamGameId=221100
cd "$STEAM_COMPAT_INSTALL_PATH"
"$STEAM_ROOT/steamapps/common/Proton - Experimental/proton" run ./DayZDiag_x64.exe \
    -server -startNavmeshDataServer -config=serverDZ_navmesh.cfg \
    -mission=<path of the mission folder> -port=2402 "-mod=@MyTerrain"
```

Some points that are easy to get wrong:

- **The mod must be inside the DayZ folder.** A `-mod=` path outside it fails with `Cannot open file`. Make a
  symbolic link `@MyTerrain` in `steamapps/common/DayZ` that points to your mod folder, and **remove it when you
  are done**. Do not change or delete any other file of the installation.
- Use the files from the Windows section: `verifySignatures = 0;`, `BattlEye = 0;` and the empty mission. Put them in
  a folder of their own (not in the game folder), and give `-config=` and `-mission=` full paths.
- Do **not** pass `-profiles=` with a Linux path: the Windows program cannot use it. Its logs end up in the Proton
  prefix, `compatdata/221100/pfx/drive_c/users/steamuser/AppData/Local/DayZ/`.
- Use a game port other than the default 2302 (`-port=2402` above). The data server port 36533 cannot be changed.
- Start only one game at a time, and stop `DayZDiag_x64.exe` when you have your navmesh.

## 2. Check the connection with `probe`

`dayzmake navmesh probe` connects, asks for the world parameters and prints them. If it works, the data server is up
and the world is the one you expect (the size and the tile grid):

```text
$ dayzmake navmesh probe
world 150.0 x 150.0 m, tiles 3 x 3 of 50.0 x 50.0 m
deep sea height 0.75, deep water depth 1.50, contour error 0.300, obstacles 1, block 4
filter islands 1, seed (60.0, 10.0, 60.0)
agent: stand 1.50 crouch 1.00 prone 0.50 diameter 0.60 step 0.45 slope 45.0
cell class 0: 200 cells over 50.00 m (cs 0.250) [unk 200 0]
cell class 1: 500 cells over 50.00 m (cs 0.100) [unk 500 0]
link 0: typeId -1 len 1.80 height 1.20 centre 0.40 dropdown 0.00..0.00 agent h 1.50 step 0.45 r 0.30 flags 0x100 area 48
```

(This is the synthetic test world; a real map is much larger, for example 15360 by 15360 m.) The game may be on
another machine: `dayzmake navmesh probe --server 192.168.1.20`. `--tile 1,1` also fetches one tile, which shows
that the geometry arrives.

When the server is not running the answer tells you what to do:

```text
$ dayzmake navmesh probe --server 127.0.0.1:9
error: cannot connect to 127.0.0.1:9: Connection refused (os error 111); start DayZDiag_x64.exe with -server -startNavmeshDataServer
```

## 3. Generate

First try a few tiles to see that everything works, then the whole world:

```text
$ dayzmake navmesh generate --range 0,0,2,0 test.nm
$ dayzmake navmesh generate mymap.nm
```

`--range x0,z0,x1,z1` limits the run to a block of tiles (columns and rows from 0). `--record <file>` also saves
the geometry that arrives (see *Regenerating*). Progress goes to standard error; the summary of a run on the
synthetic world looks like this:

```text
$ dayzmake navmesh generate --capture world.dzgeo --threads 2 mymap.nm
built 9 tiles, 0 empty, 0 failed in 0.4 s
island filter: seed (60.0 10.0 60.0) -> 87 reachable, 1 isolated polys
wrote mymap.nm
```

What the lines say: tiles *built* have walkable ground; *empty* ones have none (open sea, for example); *failed*
ones could not be built or fetched and are listed (the first twenty) and exit status 1 is the result. The *island
filter* flags polygons that cannot be reached from the world's seed position (a spot that must be walkable, like
the middle of a town) as isolated, so that the AI does not wander onto separate pieces. `--seed x,y,z` chooses
another spot, and `--no-island-filter` switches it off.

`info` shows what you got:

```text
$ dayzmake navmesh info mymap.nm
navmesh version 9, grid 3 x 3 tiles of 50.0 x 50.0 m, max tiles 9
tiles 9, polys 88, verts 161, off-mesh connections 12
```

An existing target is never overwritten without `-f`; this is checked before the first tile is fetched, so you do
not lose a long run to it:

```text
$ dayzmake navmesh generate --capture world.dzgeo mymap.nm
error: Target file "mymap.nm" already exists. Use --force to overwrite.
```

Options that change the mesh: `--doors closed|open|enabled` (the door state that is baked in; the default is
`closed`), `--no-links` (no jump, climb or ladder links) and `--sea-level <m>`. Leave them alone unless you know
you need them.

### Speed

Tiles are fetched and built in parallel: `--threads` builder threads (the default is the number of cores minus one)
and one connection per `--server` (`--connections` changes that). If fetching is the slow part, start
`DayZDiag_x64.exe` on several machines and give `--server a --server b --server c`; they must load the same world.
The output does not depend on the number of threads, connections or servers.

### When a run is interrupted

The mesh is written even when the run was partial, and `--checkpoint <n>` writes it after every `n` tiles, so a
long run can be killed without losing much. To continue, give the file as the base and skip what it has:

```text
$ dayzmake navmesh generate --server 127.0.0.1:36533 --range 0,0,2,0 --threads 2 --record rec.dzgeo part.nm
built 3 tiles, 0 empty, 0 failed in 0.1 s
wrote part.nm
$ dayzmake navmesh generate --server 127.0.0.1:36533 --base part.nm --skip-existing --threads 2 -f part.nm
built 6 tiles, 0 empty, 0 failed in 0.3 s
island filter: seed (60.0 10.0 60.0) -> 87 reachable, 1 isolated polys
wrote part.nm
```

The second run built only the six tiles the first did not, and filtered islands over the whole mesh. The result is the
same as an uninterrupted run. When a run ends with failed tiles or a failed checkpoint, the last line prints the
command to continue (`to continue: dayzmake navmesh generate --base ... --skip-existing ...`). A short run
before the full one (`--range`) shows a warning that the island filter was skipped when the seed is not in the
built tiles: that is expected, and the full run does it.

## 4. Put the file into the mod and rebuild

The world's config names its navmesh with `navmeshName` in its `CfgWorlds` class. Copy `mymap.nm` to the path that
property gives, **inside the mod's folder tree** (for example, with the prefix `mymap` and `navmeshName = "mymap\navmesh\mymap.nm";` that is
`navmesh/mymap.nm` in the addon folder), and build the PBO again:

```text
$ dayzmake build mymap mymap.pbo
```

`.nm` files are copied into the PBO as they are. Sign the PBO as usual (see [Keys and signatures](../reference/signing.md)).
Load the mod in a server and check that the log has no `Unable to load navmesh`.

## Regenerating

**After a change of the terrain** (new roads, objects, buildings, heightmap) the navmesh is out of date: generate it
again from a fresh `DayZDiag_x64.exe` that loads the new mod. To change only an area, build only its tiles into the
old file:

```text
$ dayzmake navmesh generate --base mymap.nm --range 10,20,14,24 -f mymap.nm
```

The new tiles replace those of their cells; the other tiles stay, and a tile that became empty is removed.

**Regenerating offline.** With `--record` the geometry that came from the game is saved in a `.dzgeo` file. Later you
can build again from it without starting the game, for example with another door state or after an update of
dayzmake:

```text
$ dayzmake navmesh generate --server 127.0.0.1:36533 --record mymap.dzgeo mymap.nm
$ dayzmake navmesh generate --capture mymap.dzgeo --doors open -f mymap-open.nm
```

A capture holds the geometry of a whole world, so it is large; keep it with your mod's sources, not in the PBO.

## Good to know

- On x86 and x86_64 computers the generator uses the same approximate reciprocal square root as DayZ, so the mesh
  is what DayZ itself would make. On other processors (ARM) it uses an exact division instead: a slope that is
  almost exactly the maximum slope can then be classified differently, and the file differs in a few polygons.
  Both are valid meshes.
- The generator never modifies the game, the server or your mod: it only reads from the data server and writes
  the files you name.
- Warnings are explained in [Warnings](../reference/warnings.md): `navmesh-tile-failed`, `navmesh-stray-part` and
  `navmesh-seed`.
