# Navigation meshes for a terrain mod

A custom terrain needs a **navmesh** so that infected, animals and AI can walk on it. The game cannot compute one
from the terrain files alone: it asks a running `DayZDiag_x64.exe` for the geometry of the world, and
`dayzmake navmesh generate` does that and builds the `.nm` file. This guide goes from a built terrain mod to a navmesh
in the mod. The [reference](../reference/navmesh.md) lists every command and option.

The generator has been compared with the navmeshes shipped with DayZ 1.30 (see [Validation](#validation) at the end).
Check the first mesh you build in the game before you ship it all the same.

> The `dayzmake` outputs shown here are real: they were produced against a test server that serves a small synthetic
> world. The Linux command line that starts the game was run with DayZ 1.30 under GE-Proton11-1 (the example below
> names `Proton - Experimental` as a generic path; any Proton build should do, GE-Proton11-1 is the one tested). The
> Windows command line has the same options but was not run on Windows. Neither is part of the automated tests,
> because they need DayZ.

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

### The server files (both systems)

Keep the files of the data server in a folder of your own, **outside the game installation**: here `C:\dayz-navmesh`
on Windows and `~/dayz-navmesh` on Linux. It holds

- `serverDZ_navmesh.cfg`: a copy of the stock `serverDZ.cfg` that does not check signatures and does not start
  BattlEye, so it has the lines `verifySignatures = 0;` and `BattlEye = 0;`, and whose mission `template` names your
  mission, for example `template="dayzOffline.mymap";`;
- the mission, in the folder `mpmissions` of that folder: a **complete copy** of a mission folder, in which only
  `init.c` is replaced by an empty `main()`. For a stock map this is a copy of its own stock mission under its own
  name (`DayZServer/mpmissions/dayzOffline.enoch`, `dayzOffline.nasdara`); for your terrain it is a copy of your
  mod's own full mission (`dayzOffline.<yourworld>`):

```text
void main()
{
}
```

Do not shorten the mission to the `init.c` alone: a folder that contains only `init.c` makes the server log
`[Server] :: termination in: 10` and exit about 7 seconds after the start, before the data server opens.

### On Windows

Start the server from the DayZ folder, with full paths, your mod and a game port that is not 2302:

```text
DayZDiag_x64.exe -server -startNavmeshDataServer -config=C:\dayz-navmesh\serverDZ_navmesh.cfg -mission=C:\dayz-navmesh\mpmissions\dayzOffline.mymap -profiles=C:\dayz-navmesh\profiles -port=2402 -dologs -adminlog -nosplash -nopause "-mod=@MyTerrain"
```

`@MyTerrain` is the folder of your mod in the DayZ folder. `-profiles=` puts the RPT and the script logs into that
folder. A stock DLC map such as Badlands (`dayzOffline.nasdara`) needs no `-mod`. Keep the window open while you
generate, and wait for the data server before you use it (see *Check the connection* below).

### On Linux, under Proton

<!-- verified: GE-Proton11-1, DayZ 1.30 -->

The Windows program runs through Proton, from the **client** installation (`steamapps/common/DayZ`), which has
`DayZDiag_x64.exe`. Run it the way Steam would, with the environment of the game. The program is a Windows program:
it sees the Linux file system as drive `Z:`, so `/home/you/dayz-navmesh/serverDZ_navmesh.cfg` is written
`Z:\home\you\dayz-navmesh\serverDZ_navmesh.cfg`.

```text
export STEAM_ROOT="$HOME/.local/share/Steam"            # your Steam folder
export STEAM_COMPAT_CLIENT_INSTALL_PATH="$STEAM_ROOT"
export STEAM_COMPAT_DATA_PATH="$STEAM_ROOT/steamapps/compatdata/221100"
export STEAM_COMPAT_INSTALL_PATH="$STEAM_ROOT/steamapps/common/DayZ"
export STEAM_COMPAT_APP_ID=221100 SteamAppId=221100 SteamGameId=221100
cd "$STEAM_COMPAT_INSTALL_PATH"
"$STEAM_ROOT/steamapps/common/Proton - Experimental/proton" run ./DayZDiag_x64.exe \
    -server -startNavmeshDataServer \
    "-config=Z:\home\you\dayz-navmesh\serverDZ_navmesh.cfg" \
    "-mission=Z:\home\you\dayz-navmesh\mpmissions\dayzOffline.mymap" \
    "-profiles=Z:\home\you\dayz-navmesh\profiles" \
    -port=2402 -dologs -adminlog -nosplash -nopause "-mod=@MyTerrain"
```

For a stock map such as Badlands leave out `-mod`.

Some points that are easy to get wrong:

- **The mod must be inside the DayZ folder.** A `-mod=` path outside it fails with `Cannot open file`. Make a
  symbolic link `@MyTerrain` in `steamapps/common/DayZ` that points to your mod folder, and **remove it when you
  are done**. Do not change or delete any other file of the installation.
- The server files stay in your own folder (see above); give `-config=`, `-mission=` and `-profiles=` as `Z:\`
  paths, as shown. A plain Linux path does not work for the Windows program. With `-profiles=` the RPT and the
  script logs end up in that folder.
- Use a game port other than the default 2302 (`-port=2402` above). The data server port 36533 cannot be changed.
- Start only one game at a time, and stop `DayZDiag_x64.exe` when you have your navmesh.

## 2. Check the connection with `probe`

The data server opens port 36533 about 25 seconds after the start. A connection made while the world is still
loading can be reset (`Connection reset by peer`), so run `probe` until it answers instead of connecting as soon as
the port shows up.

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
island filter: seed (60.0 10.0 60.0) -> 84 reachable, 1 isolated polys
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
tiles 9, polys 85, verts 159, off-mesh connections 12
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
and one connection per `--server` (`--connections` changes that; keep it at one per `DayZDiag_x64.exe`, which serves
one client at a time). If fetching is the slow part, start `DayZDiag_x64.exe` on several machines and give `--server a --server b --server c`; they must load the same world.
The output does not depend on the number of threads, connections or servers.

### When a run is interrupted

The mesh is written even when the run was partial, and `--checkpoint <n>` writes it after every `n` tiles, so a
long run can be killed without losing much. Each checkpoint rewrites the whole file, so give a large `n` (hundreds
or thousands of tiles), not a small one. To continue, give the file as the base and skip what it has:

```text
$ dayzmake navmesh generate --server 127.0.0.1:36533 --range 0,0,2,0 --threads 2 --record rec.dzgeo part.nm
warning: island filter skipped: no polygon near the seed (60.0 10.0 60.0) [navmesh-seed]
built 3 tiles, 0 empty, 0 failed in 0.1 s
wrote part.nm
$ dayzmake navmesh generate --server 127.0.0.1:36533 --base part.nm --skip-existing --threads 2 -f part.nm
built 6 tiles, 0 empty, 0 failed in 0.3 s
island filter: seed (60.0 10.0 60.0) -> 84 reachable, 1 isolated polys
wrote part.nm
```

The second run built only the six tiles the first did not, and filtered islands over the whole mesh. The result is the
same as an uninterrupted run. When a run is **stopped** (a checkpoint or `--record` file that cannot be written) or
some tiles **could not be fetched** (and none failed to build), the summary ends with the command to continue, for
example `to continue: dayzmake navmesh generate --base mymap.nm --skip-existing -f --server a --range 0,0,9,9 mymap.nm`.
It repeats every option of the first run (`--doors`, `--sea-level`, `--no-links`, `--seed`, ...) except `--record`
and `--base`: the options must stay the same, or the rest of the mesh would be built differently from the first part,
and a repeated `--record` with `-f` would overwrite the first recording. A tile that failed to *build* is not offered
for a retry, because it would fail again; look at the error that was listed for it. A short run before the full
one (`--range`) shows a warning that the island filter was skipped when the seed is not in the built tiles: that is
expected, and the full run does it.

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

A capture holds the geometry of a whole world, so it is large; keep it with your mod's sources, not in the PBO. A capture of a whole map can be tens of gigabytes; building a
`--range` from it reads only the tiles of that range, so a small range stays fast and small.

## Good to know

- On x86 and x86_64 computers the generator uses the same approximate reciprocal square root as DayZ, so the mesh
  is what DayZ itself would make. On other processors (ARM) it uses an exact division instead: a slope that is
  almost exactly the maximum slope can then be classified differently, and the file differs in a few polygons.
  The approximate instruction itself can also give different results on Intel and AMD CPUs, so two x86 machines
  can differ in such a polygon too. All of them are valid meshes.
- Objects with moving parts (for example the top of a tall tower that turns slowly) are sent in the pose they have
  when the tile is requested, so the same object has a different pose in each tile, and in a mesh that
  shipped with the game it had yet another one. The tiles around such an object can differ in a few polygons and in
  the height bound in their header (`bmax` y); this is a property of the data, and the mesh is valid. The same goes for
  the order of several door objects in one tile, which the game does not always send in the same order, and which
  decides which of two overlapping door footprints a cell belongs to.
- The generator never modifies the game, the server or your mod: it only reads from the data server and writes
  the files you name.
- Warnings are explained in [Warnings](../reference/warnings.md): `navmesh-tile-failed`, `navmesh-stray-part` and
  `navmesh-seed`.

## Validation

The generator was compared with the navmeshes that ship with DayZ 1.30, using captures made from a live
`DayZDiag_x64.exe` (GE-Proton) on Livonia and on Badlands. Only the **inner tiles** can be compared, one tile in from
the edge of what was built: an off-mesh connection (jump or climb link) into a neighbouring tile has its far end
snapped to that tile's mesh only when the neighbour exists, and the tiles at the edge of a block have neighbours
outside the capture. Two parts of a tile are left out of every comparison and are not errors: the polygon flags (they
come from the island filter, which was off) and the Detour link array between polygons and its size, which stock
Detour builds when a tile is added to the mesh, so what is stored is not meaningful for a comparison.

- **The central half of Badlands.** Tiles 48-278 on both axes were rebuilt from one live capture (DayZ 1.30, recorded
  2026-11-18) with the island filter off: 53,358 tiles built, none failed, in 1124 s on 24 cores. Of the 52,441 inner
  tiles (49-277) **52,381 are byte-identical** to the shipped mesh. The other 60 all have a known cause:
  - 44 lie in the area of the animated towers (171-178 x 210-230). Objects with moving parts are sent in the pose they
    have at the moment of the request, so the same object has a different pose in each tile and in the shipped mesh:
    this is a property of the data, not of the generator;
  - 8 differ in the order of door objects within a tile, which the game does not always send in the same order and
    which decides which of two overlapping door footprints a cell belongs to (data as well);
  - 8 have one off-mesh connection whose hide-part membership differs from the shipped mesh: six are listed in a part
    where the shipped mesh lists none, one is missing and one is in another part of the same object (open issue
    [#127](https://github.com/bzed/dayzmake/issues/127); doors never hold off-mesh polygons, so this is not a door
    matter). These are the counts on the 949 tiles that were analysed in detail, which include these tiles;
  - 1 differs in the order of two tree obstacles (open issue
    [#128](https://github.com/bzed/dayzmake/issues/128)).

  (One tile, 171,213, is both in the tower area and one of the #127 cases.) The three tiles of the first row that were
  fetched again without `--record` are not in the capture and are outside the comparison.
- **Earlier blocks, all inner tiles identical:** on Livonia, tiles 128-131 x 128-131 (open terrain) and 128-131 x
  220-223 (a dense town); on Badlands, a dense block (136-139 x 100-103) and two blocks with the irrigation tunnels (180-183 x
  186-189 and 183-186 x 198-201), whose floors are walkable below the surface.

So the claim is exact and limited: the inner tiles of the central half of Badlands and of five earlier blocks, with the
polygon flags and the link array excluded, and with the 60 tiles above as the known differences. It does not cover the
edge tiles, the outer parts of Badlands, whole maps or other maps, so the first mesh of your own terrain is still worth
a look in the game.

You can repeat the comparison with your own copy of the game data. The test is ignored by default, because it needs
files that are not part of the repository: `DAYZ_NAVMESH` is the shipped `.nm`, `DAYZ_NAVMESH_CAPTURE` a capture
(`--record`) of the same world and `DAYZ_NAVMESH_INNER` the inner tile range `x0,z0,x1,z1` to compare:

```text
DAYZ_NAVMESH=navmesh.nm DAYZ_NAVMESH_CAPTURE=world.dzgeo DAYZ_NAVMESH_INNER=128,128,131,131 \
    cargo test -p dayzmake-core --test navmesh_live -- --ignored
```
