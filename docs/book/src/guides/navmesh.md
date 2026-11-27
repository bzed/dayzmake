# Navigation meshes for a terrain mod

A custom terrain needs a **navmesh**. Infected, animals and AI use it to walk on the terrain. The game cannot
calculate a navmesh from the terrain files alone. It asks a running `DayZDiag_x64.exe` for the geometry of the world.
`dayzmake navmesh generate` does this and builds the `.nm` file. This guide goes from a built terrain mod to a
navmesh in the mod. The [reference](../reference/navmesh.md) lists each command and option.

We compared the generator with the navmeshes that ship with DayZ 1.30 (see [Validation](#validation) at the end).
Even so, check the first mesh that you build in the game before you ship it.

> The `dayzmake` outputs in this guide are real. We made them against a test server that serves a small synthetic
> world. We ran the Linux command line that starts the game with DayZ 1.30 under GE-Proton11-1. The example below
> names `Proton - Experimental` as a generic path. Each Proton build should work, but GE-Proton11-1 is the build that
> we tested. The Windows command line has the same options, but we did not run it on Windows. The automated tests do
> not include either command line, because they need DayZ.

## What you need

- A built **terrain mod**: the PBOs of your world (see [A first mod](first-mod.md) to build PBOs). The mod already
  loads in the game, but without a navmesh or with an old navmesh.
- **DayZ** (the client, Steam app 221100). It contains `DayZDiag_x64.exe`, the diagnostic build that has the navmesh
  data server.
- On Linux, **Proton** (see [Binarizing models on Linux](binarize-linux.md) to install it through Steam).
- A few gigabytes of free memory for the game. For a large map, you also need time: the tiles of a big world take
  minutes to hours (see *Speed* below).

## 1. Start the data server

Start `DayZDiag_x64.exe` like a dedicated server, with the extra option `-startNavmeshDataServer`. It loads your
world and then listens on TCP port **36533** for requests for geometry. The game sets this port and you cannot change
it. The normal game port is different. Do not use the default game port. Then a real server on the same machine is
not disturbed.

### The server files (both systems)

Keep the files of the data server in a folder of your own, **outside the game installation**. Here the folder is
`C:\dayz-navmesh` on Windows and `~/dayz-navmesh` on Linux. It holds

- `serverDZ_navmesh.cfg`: a copy of the stock `serverDZ.cfg`. It does not check signatures and does not start
  BattlEye, so it has the lines `verifySignatures = 0;` and `BattlEye = 0;`. The mission `template` names your
  mission, for example `template="dayzOffline.mymap";`;
- the mission, in the folder `mpmissions` of that folder: a **complete copy** of a mission folder. In the copy,
  replace only `init.c` with an empty `main()`. For a stock map, copy its own stock mission under its own name
  (`DayZServer/mpmissions/dayzOffline.enoch`, `dayzOffline.nasdara`). For your terrain, copy the full mission of your
  mod (`dayzOffline.<yourworld>`):

```text
void main()
{
}
```

Do not shorten the mission to the `init.c` alone. If the folder contains only `init.c`, the server logs
`[Server] :: termination in: 10` and exits about 7 seconds after the start, before the data server opens.

### On Windows

Start the server from the DayZ folder. Use full paths, your mod and a game port that is not 2302:

```text
DayZDiag_x64.exe -server -startNavmeshDataServer -config=C:\dayz-navmesh\serverDZ_navmesh.cfg -mission=C:\dayz-navmesh\mpmissions\dayzOffline.mymap -profiles=C:\dayz-navmesh\profiles -port=2402 -dologs -adminlog -nosplash -nopause "-mod=@MyTerrain"
```

`@MyTerrain` is the folder of your mod in the DayZ folder. `-profiles=` puts the RPT and the script logs into that
folder. A stock DLC map such as Badlands (`dayzOffline.nasdara`) needs no `-mod`. Keep the window open while you
generate. Wait for the data server before you use it (see *Check the connection* below).

### On Linux, under Proton

<!-- verified: GE-Proton11-1, DayZ 1.30 -->

Run the Windows program through Proton, from the **client** installation (`steamapps/common/DayZ`). That
installation has `DayZDiag_x64.exe`. Run it as Steam does, with the environment of the game. The program is a Windows
program. It sees the Linux file system as drive `Z:`. So write `/home/you/dayz-navmesh/serverDZ_navmesh.cfg` as
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

For a stock map such as Badlands, leave out `-mod`.

These points are easy to get wrong:

- **The mod must be inside the DayZ folder.** A `-mod=` path outside it fails with `Cannot open file`. Make a
  symbolic link `@MyTerrain` in `steamapps/common/DayZ` that points to your mod folder. **Remove the link when you
  are done.** Do not change or delete other files of the installation.
- Keep the server files in your own folder (see above). Give `-config=`, `-mission=` and `-profiles=` as `Z:\`
  paths, as shown. The Windows program does not accept a plain Linux path. With `-profiles=`, the RPT and the
  script logs go to that folder.
- Use a game port other than the default 2302 (`-port=2402` above). You cannot change the data server port 36533.
- Start only one game at a time. Stop `DayZDiag_x64.exe` when you have your navmesh.

## 2. Check the connection with `probe`

The data server opens port 36533 about 25 seconds after the start. A connection that you make while the world is
loading can be reset (`Connection reset by peer`). Run `probe` until it answers. Do not connect as soon as the port
appears.

`dayzmake navmesh probe` connects, asks for the world parameters and prints them. If it works, the data server is
up. Check that the world is the world that you expect (the size and the tile grid):

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

(This is the synthetic test world. A real map is much larger, for example 15360 by 15360 m.) The game can be on
another machine: `dayzmake navmesh probe --server 192.168.1.20`. `--tile 1,1` also fetches one tile. This shows that
the geometry arrives.

When the server does not run, the error tells you what to do:

```text
$ dayzmake navmesh probe --server 127.0.0.1:9
error: cannot connect to 127.0.0.1:9: Connection refused (os error 111); start DayZDiag_x64.exe with -server -startNavmeshDataServer
```

## 3. Generate

First build a few tiles to see that everything works. Then build the whole world:

```text
$ dayzmake navmesh generate --range 0,0,2,0 test.nm
$ dayzmake navmesh generate mymap.nm
```

`--range x0,z0,x1,z1` limits the run to a block of tiles (columns and rows from 0). `--record <file>` also saves
the geometry that arrives (see *Regenerating*). Progress goes to standard error. The summary of a run on the
synthetic world looks like this:

```text
$ dayzmake navmesh generate --capture world.dzgeo --threads 2 mymap.nm
built 9 tiles, 0 empty, 0 failed in 0.4 s
island filter: seed (60.0 10.0 60.0) -> 84 reachable, 1 isolated polys
wrote mymap.nm
```

This is what the lines mean. Tiles *built* have walkable ground. *Empty* tiles have none (open sea, for example).
*Failed* tiles could not be built or fetched. dayzmake lists them (the first twenty), and the exit status is 1. The
*island filter* marks polygons as isolated when they cannot be reached from the seed position of the world. The seed
position is a spot that must be walkable, like the middle of a town. The filter stops the AI from walking onto
separate pieces. `--seed x,y,z` selects another spot, and `--no-island-filter` switches the filter off.

`info` shows what you made:

```text
$ dayzmake navmesh info mymap.nm
navmesh version 9, grid 3 x 3 tiles of 50.0 x 50.0 m, max tiles 9
tiles 9, polys 85, verts 159, off-mesh connections 12
```

dayzmake does not overwrite an existing target without `-f`. It checks this before it fetches the first tile, so you
do not lose a long run:

```text
$ dayzmake navmesh generate --capture world.dzgeo mymap.nm
error: Target file "mymap.nm" already exists. Use --force to overwrite.
```

These options change the mesh: `--doors closed|open|enabled` (the door state in the mesh; the default is `closed`),
`--no-links` (no jump, climb or ladder links) and `--sea-level <m>`. Do not use them unless you need them.

### Speed

dayzmake fetches and builds tiles in parallel. It uses `--threads` builder threads (the default is the number of
cores minus one) and one connection for each `--server`. `--connections` changes the number of connections. Keep it
at one for each `DayZDiag_x64.exe`, because the program serves one client at a time. If the fetching is slow, start
`DayZDiag_x64.exe` on several machines and give `--server a --server b --server c`. They must load the same world.
The output does not depend on the number of threads, connections or servers.

### When a run is interrupted

dayzmake writes the mesh also when the run is partial. `--checkpoint <n>` writes the mesh after each `n` tiles. Then
you can stop a long run and not lose much. Each checkpoint rewrites the whole file, so give a large `n` (hundreds or
thousands of tiles) and not a small `n`. To continue, give the file as the base and skip the tiles that it has:

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

The second run built only the six tiles that the first run did not build. It filtered islands over the whole mesh.
The result is the same as the result of an uninterrupted run. Sometimes a run **stops** (dayzmake cannot write a
checkpoint or a `--record` file). Sometimes dayzmake **cannot fetch** some tiles (and none failed to build). Then the
summary ends with the command to continue, for example
`to continue: dayzmake navmesh generate --base mymap.nm --skip-existing -f --server a --range 0,0,9,9 mymap.nm`.
The command repeats each option of the first run (`--doors`, `--sea-level`, `--no-links`, `--seed`, ...) except
`--record` and `--base`. Keep the options the same. If you change them, dayzmake builds the rest of the mesh in a
different way from the first part. A repeated `--record` with `-f` overwrites the first recording. If a tile failed
to *build*, dayzmake does not offer a retry, because the tile fails again. Look at the error that dayzmake listed for
it. A short run before the full run (`--range`) shows a warning that dayzmake skipped the island filter, when the
seed is not in the built tiles. This is normal, and the full run does the filtering.

## 4. Put the file into the mod and rebuild

The config of the world gives the name of its navmesh with `navmeshName` in its `CfgWorlds` class. Copy `mymap.nm`
to the path that this property gives, **inside the folder tree of the mod**. For example, with the prefix `mymap` and
`navmeshName = "mymap\navmesh\mymap.nm";`, the path is `navmesh/mymap.nm` in the addon folder. Then build the PBO
again:

```text
$ dayzmake build mymap mymap.pbo
```

`build` copies `.nm` files into the PBO as they are. Sign the PBO as usual (see
[Keys and signatures](../reference/signing.md)). Load the mod in a server. Make sure that the log does not have
`Unable to load navmesh`.

## Regenerating

**After a change of the terrain** (new roads, objects, buildings, heightmap), the navmesh is out of date. Generate it
again from a new `DayZDiag_x64.exe` that loads the new mod. To change only an area, build only its tiles into the
old file:

```text
$ dayzmake navmesh generate --base mymap.nm --range 10,20,14,24 -f mymap.nm
```

The new tiles replace the tiles of their cells. The other tiles stay. dayzmake removes a tile that became empty.

**Regenerating offline.** With `--record`, dayzmake saves the geometry from the game in a `.dzgeo` file. Later you
can build again from this file and not start the game. You can do this, for example, with another door state or after
an update of dayzmake:

```text
$ dayzmake navmesh generate --server 127.0.0.1:36533 --record mymap.dzgeo mymap.nm
$ dayzmake navmesh generate --capture mymap.dzgeo --doors open -f mymap-open.nm
```

A capture holds the geometry of a whole world, so it is large. Keep it with the sources of your mod, not in the PBO.
A capture of a whole map can be tens of gigabytes. If you build a `--range` from it, dayzmake reads only the tiles of
that range. So a small range stays fast and small.

## Good to know

- On x86 and x86_64 computers, the generator uses the same approximate reciprocal square root as DayZ. So the mesh
  is the mesh that DayZ itself makes. On other processors (ARM), it uses an exact division. Then a slope that is
  almost exactly the maximum slope can get a different class, and the file differs in a few polygons. The approximate
  instruction can also give different results on Intel and AMD CPUs, so two x86 machines can differ in such a
  polygon too. All of these meshes are valid.
- The game sends an object with moving parts (for example the top of a tall tower that turns slowly) in the pose that
  it has when the tile is requested. So the same object has a different pose in each tile, and it had another pose in
  a mesh that shipped with the game. The tiles around such an object can differ in a few polygons and in the height
  bound in their header (`bmax` y). This is a property of the data, and the mesh is valid. The same is true for the
  order of several door objects in one tile. The game does not always send them in the same order, and the order
  decides which of two overlapping door footprints a cell belongs to.
- The generator never changes the game, the server or your mod. It only reads from the data server and writes the
  files that you name.
- [Warnings](../reference/warnings.md) explains these warnings: `navmesh-tile-failed`, `navmesh-stray-part` and
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
  - 1 (146,83) differs in the order of two tree obstacles, which is data as well: tree obstacles come in the
    input order of the tree parts, which holds on the other 19,875 tiles of the half with two or more of them, and no
    sort key (radius, size, position, ...) reproduces both
    ([#128](https://github.com/bzed/dayzmake/issues/128)).

  (One tile, 171,213, is both in the tower area and one of the #127 cases.) The three tiles of the first row that were
  fetched again without `--record` are not in the capture and are outside the comparison.
- **Earlier blocks, all inner tiles identical:** on Livonia, tiles 128-131 x 128-131 (open terrain) and 128-131 x
  220-223 (a dense town); on Badlands, a dense block (136-139 x 100-103) and two blocks with the irrigation
  tunnels (180-183 x 186-189 and 183-186 x 198-201), whose floors are walkable below the surface.

So the claim is exact and limited: the inner tiles of the central half of Badlands and of five earlier blocks, with the
polygon flags and the link array excluded, and with the 60 tiles above as the known differences. It does not cover the
edge tiles, the outer parts of Badlands, whole maps or other maps, so the first mesh of your own terrain is still worth
a look in the game.

You can repeat the comparison with your own copy of the game data. The test is ignored by default, because it needs
files that are not part of the repository: `DAYZ_NAVMESH` is the shipped `.nm`, `DAYZ_NAVMESH_CAPTURE` a capture
(`--record`) of the same world and `DAYZ_NAVMESH_INNER` the inner tile range `x0,z0,x1,z1` to compare. A capture that is
too big to build whole can be limited with `DAYZ_NAVMESH_RANGE` (`x0,z0,x1,z1`): only that range is built, so it must
reach one tile beyond `DAYZ_NAVMESH_INNER` on each side:

```text
DAYZ_NAVMESH=navmesh.nm DAYZ_NAVMESH_CAPTURE=world.dzgeo DAYZ_NAVMESH_INNER=128,128,131,131 \
    cargo test -p dayzmake-core --test navmesh_live -- --ignored
```
