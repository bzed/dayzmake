# Navigation meshes: navmesh

A navmesh (`.nm` file) tells the AI of DayZ where infected, animals and players' pathfinding can walk. A terrain mod
ships one next to its world. `dayzmake navmesh` builds it: `generate` asks the **navmesh data server** of
`DayZDiag_x64.exe` for the geometry of the world, tile by tile, and builds the mesh from it; `probe` shows what the
server says about the world; `info` summarises a `.nm` file. The [guide](../guides/navmesh.md) goes from a built
terrain mod to a mesh in the game; this page lists every command.

The generator is a Rust port of the Recast and Detour libraries with the changes DayZ made to them. Its output is
independent of the number of threads and connections: the same input gives the same bytes. On x86 and x86_64
it uses the same approximate reciprocal square root instruction as DayZ; on other CPUs (ARM, for example) a plain
division is used, and a slope very close to the maximum slope can be classified differently there.

The examples use a small synthetic world of 3 by 3 tiles of 50 by 50 m, served by a test server and saved as the
capture `world.dzgeo`. Against the game, use the address of the machine that runs `DayZDiag_x64.exe`; the port is
always 36533. The outputs below were produced by the program.

## Data servers and captures

`probe` and `generate` talk to a data server: `DayZDiag_x64.exe` started with `-server -startNavmeshDataServer`. The
server listens on TCP port 36533 (fixed by the game). A host is written `host` or `host:port`; the port defaults to
36533, and the host to `127.0.0.1`. Names are resolved to IPv4 addresses (IPv6 is not supported).

`probe`, and the request for the world parameters at the start of `generate`, make one attempt: when the connection
cannot be made within 10 seconds, or the answer fails, the command fails at once. Tile requests connect when they are
first needed, and a tile request that fails on the network (a connect that fails, a closed connection, a timeout, a
wrong answer) is repeated on a fresh connection, five attempts in all, waiting 2, 4, 6 and 8 seconds between them.
A server that sends nothing for 10 minutes counts as failed. A malformed answer is
not repeated. All data from a server is checked and limited before it is used.

A **capture** (`.dzgeo`) is a recording of the geometry that a server sent. `generate --record` writes one while it
fetches, and `generate --capture` builds from one without any server. A capture lets you rebuild the mesh after a
change of the generator, or on another machine, without starting the game.

## dayzmake navmesh probe

Ask the data server for the world parameters, and optionally for one tile.

<div class="synopsis">

```text
dayzmake navmesh probe [-v] [--json] [--server <host>] [--tile <x,z>]
```

</div>

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Lifts the limit of ten warnings per warning name. |
| `--json` | off | Print one JSON object `{"params": {...}, "tile": {...}}` instead of text. `tile` is only there with `--tile`. |
| `--server <host>` | `127.0.0.1` | The data server, `host` or `host:port`. |
| `--tile <x,z>` | none | Also fetch tile `x,z` (tile coordinates, from 0) and summarise it. |

The text output is, in order: the world size and the tile grid; the sea, contour and block parameters; the island
filter switch and seed; the agent (the sizes the mesh is built for); one line per cell class (the resolution of
a tile); one line per jump link class; and with `--tile` a line for the tile and one per group of triangles
(`area` is the surface type, `parts` the number of door and object parts, `verts` the number of vertices).

```text
$ dayzmake navmesh probe --server 127.0.0.1:36533 --tile 1,1
world 150.0 x 150.0 m, tiles 3 x 3 of 50.0 x 50.0 m
deep sea height 0.75, deep water depth 1.50, contour error 0.300, obstacles 1, block 4
filter islands 1, seed (60.0, 10.0, 60.0)
agent: stand 1.50 crouch 1.00 prone 0.50 diameter 0.60 step 0.45 slope 45.0
cell class 0: 200 cells over 50.00 m (cs 0.250) [unk 200 0]
cell class 1: 500 cells over 50.00 m (cs 0.100) [unk 500 0]
link 0: typeId -1 len 1.80 height 1.20 centre 0.40 dropdown 0.00..0.00 agent h 1.50 step 0.45 r 0.30 flags 0x100 area 48
tile (1,1) cell class 1 bounds (50.00 0.00 50.00)-(100.00 30.00 100.00), 7 groups
  area  1: 0 parts, 7350 verts
  area  9: 0 parts, 72 verts
  area  8: 0 parts, 186 verts
  area 12: 1 parts, 36 verts
  area 13: 1 parts, 36 verts
  area 10: 1 parts, 72 verts
  area 15: 0 parts, 2 verts
```

With `--json` the object has `params` (`worldSizeX`, `worldSizeZ`, `tilesX`, `tilesZ`, `tileWidth`, `tileHeight`,
`seaLevel`, `deepWaterDepth`, `contourMaxError`, `obstacles`, `blockSize`, `filterIsolatedIslands`, `seed`, `agent`,
`cellClasses`, `linkClasses`) and, with `--tile`, `tile` (`x`, `z`, `cellClass`, `bmin`, `bmax` and `groups`, a list
of `area`, `parts` and `verts`):

```text
$ dayzmake navmesh probe --json --server 127.0.0.1:36533
{"params":{"agent":{"crouchHeight":1.0,"diameter":0.6,"maxSlope":45.0,"maxStepHeight":0.45,"proneHeight":0.5,"standHeight":1.5},"blockSize":4,"cellClasses":[{"cellSize":0.25,"cells":200,"width":50.0},{"cellSize":0.1,"cells":500,"width":50.0}],"contourMaxError":0.3,"deepWaterDepth":1.5,"filterIsolatedIslands":true,"linkClasses":[{"agentHeight":1.5,"agentRadius":0.3,"area":48,"dropdownMax":0.0,"dropdownMin":0.0,"flags":256,"index":0,"jumpHeight":1.2,"jumpLength":1.8,"maxStepHeight":0.45,"minCenterHeight":0.4,"typeId":-1}],"obstacles":true,"seaLevel":0.75,"seed":[60.0,10.0,60.0],"tileHeight":50.0,"tileWidth":50.0,"tilesX":3,"tilesZ":3,"worldSizeX":150.0,"worldSizeZ":150.0}}
```

When nothing listens on the port the command fails with status 1 and says how to start the server:

```text
$ dayzmake navmesh probe --server 127.0.0.1:9
error: cannot connect to 127.0.0.1:9: Connection refused (os error 111); start DayZDiag_x64.exe with -server -startNavmeshDataServer
```

## dayzmake navmesh generate

Build a navmesh from a data server or a capture and write it to `<target>`.

<div class="synopsis">

```text
dayzmake navmesh generate [-v] [-f] [--server <host>]... [--capture <dzgeo>] [--record <dzgeo>] [--range <x0,z0,x1,z1>] [--base <nm>] [--skip-existing] [--checkpoint <n>] [--threads <n>] [--connections <n>] [--sea-level <m>] [--doors <state>] [--no-links] [--no-island-filter] [--seed <x,y,z>] <target>
```

</div>

| Argument | Meaning |
|---|---|
| `<target>` | The `.nm` file to write. It is written atomically (a temporary file next to it, renamed at the end). |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Lifts the limit of ten warnings per warning name. |
| `-f`, `--force` | off | Overwrite an existing `<target>` and an existing `--record` file. Without it, an existing target is an error that is reported before anything is fetched. |
| `--server <host>` | `127.0.0.1` | A data server, `host` or `host:port`. Repeatable: the connections are spread over the servers, which must serve the same world (several `DayZDiag_x64.exe` on several machines build faster). The world parameters come from the first one. Not together with `--capture`. |
| `--capture <dzgeo>` | none | Build from this capture instead of a server. Tiles the capture does not have are left out. Without `--range` the range is the bounding box of the captured tiles. |
| `--record <dzgeo>` | none | Also write the fetched tiles to a capture file. |
| `--range <x0,z0,x1,z1>` | the whole world | The tiles to build: columns `x0` to `x1` and rows `z0` to `z1`, inclusive. Values below 0 or beyond the grid are cut to the world; an upper bound below 0 means the last tile. A range with nothing left is the error `empty tile range`. |
| `--base <nm>` | none | Start from this navmesh: its grid is kept, and the new tiles replace the tiles of their cells. |
| `--skip-existing` | off | Leave out the tiles that `--base` already has. Needs `--base` (`--skip-existing needs --base`). |
| `--checkpoint <n>` | 0 (off) | Write the target after every `n` finished tiles (built, empty or failed). A checkpoint that cannot be written stops the run, which is then a failure (see below). |
| `--threads <n>` | cores - 1, at least 1 | Builder threads. 0 and negative numbers mean the default. |
| `--connections <n>` | one per server (1 for a capture) | Connections to the servers. |
| `--sea-level <m>` | the server's deep sea height | The sea level in metres. |
| `--doors <state>` | `closed` | The state of the doors baked into the mesh. `closed`: the polygons of open doors are disabled, those of closed doors are not. `open`: the other way round. `enabled`: no door polygon is disabled. |
| `--no-links` | links on | Leave out jump, climb and ladder links. |
| `--no-island-filter` | filter on | Do not flag polygons that cannot be reached from the seed. |
| `--seed <x,y,z>` | the world's seed position | Where the island filter starts (world coordinates in metres). |

The command prints, to standard error, a progress line, which is rewritten in place on a terminal and appears at most
once per 5 % when the output is redirected:

```text
1/9 tiles (0 failed), 21.8 tiles/s, 0:00:00 left
```

Then, to standard output, the summary: the number of tiles built, empty and failed and the time; the first twenty
failures, indented; a line when tiles had door part numbers outside their range (the warning `navmesh-stray-part`
is raised for them); the result of the island filter; and `wrote <target>`.

```text
$ dayzmake navmesh generate --capture world.dzgeo --threads 2 mymap.nm
built 9 tiles, 0 empty, 0 failed in 0.4 s
island filter: seed (60.0 10.0 60.0) -> 87 reachable, 1 isolated polys
wrote mymap.nm
```

A tile without walkable surface is *empty*: the tile of that cell is removed from a `--base` mesh. A tile that cannot
be built is *failed* and raises the warning `navmesh-tile-failed`; it does not stop the run. The island filter
runs over the whole mesh at the end, so a resumed run (below) filters the merged mesh. When the seed finds no
polygon the filter is skipped with the warning `navmesh-seed`.

The target is written **even when the run was partial**. The exit status is 0 when every tile was handled, and 1 when
a tile failed, a tile could not be fetched, or a checkpoint or `--record` write failed.

A run that was **stopped** (a checkpoint or `--record` write failed), or that had tiles it could not fetch and no tile
that failed to build, ends with the command that continues it. The command repeats every option of the run except
`--record` (with `-f` it would overwrite the first recording) and `--base`, which becomes the target, so the rest is
built with the same settings. A tile that failed to *build* is not offered for a retry, because it would fail
again. With a server that closes every connection, one tile (retried five times, 20 seconds in all) gives:

```text
$ dayzmake navmesh generate --server 127.0.0.1:36533 --range 0,0,0,0 --threads 2 lost.nm
built 0 tiles, 0 empty, 0 failed in 20.0 s
  tile (0,0): connection to 127.0.0.1:36533 closed by the server
wrote lost.nm
to continue: dayzmake navmesh generate --base lost.nm --skip-existing -f --server 127.0.0.1:36533 --range 0,0,0,0 --threads 2 lost.nm
error: 0 tiles could not be built and 1 could not be fetched; the mesh was saved without them
```

`--base <target> --skip-existing` over the same range builds only the tiles that are still missing, and the result
is the same as an uninterrupted run:

```text
$ dayzmake navmesh generate --server 127.0.0.1:36533 --range 0,0,2,0 --threads 2 --record rec.dzgeo part.nm
built 3 tiles, 0 empty, 0 failed in 0.1 s
wrote part.nm
$ dayzmake navmesh generate --server 127.0.0.1:36533 --base part.nm --skip-existing --threads 2 -f part.nm
built 6 tiles, 0 empty, 0 failed in 0.3 s
island filter: seed (60.0 10.0 60.0) -> 87 reachable, 1 isolated polys
wrote part.nm
```

(The first run raised `warning: island filter skipped: no polygon near the seed (60.0 10.0 60.0) [navmesh-seed]`,
because the seed lies in a row that was not built.)

Errors:

```text
$ dayzmake navmesh generate --capture world.dzgeo --threads 2 mymap.nm
error: Target file "mymap.nm" already exists. Use --force to overwrite.
$ dayzmake navmesh generate --capture world.dzgeo --range 1,2,3 mymap2.nm
error: --range: expected x0,z0,x1,z1, got "1,2,3"
```

## dayzmake navmesh info

Summarise a `.nm` file.

<div class="synopsis">

```text
dayzmake navmesh info [-v] [--json] [--tiles] <source>
```

</div>

| Argument | Meaning |
|---|---|
| `<source>` | The `.nm` file (set version 7 or 9). |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Lifts the limit of ten warnings per warning name. |
| `--json` | off | Print one JSON object instead of text. |
| `--tiles` | off | List every tile: `x z ref polys verts offmesh parts obstacles`. |

The summary gives the file version, the grid and the size of a tile, the number of tile slots (`max tiles`), the
number of tiles and the totals of polygons, vertices and off-mesh connections (the jump, climb and ladder links).
`ref` is the tile reference in hex, `parts` the door and object parts of the tile and `obstacles` its tree obstacles.

```text
$ dayzmake navmesh info --tiles mymap.nm
navmesh version 9, grid 3 x 3 tiles of 50.0 x 50.0 m, max tiles 9
tiles 9, polys 88, verts 161, off-mesh connections 12
x z ref polys verts offmesh parts obstacles
0 0 0x1000000000000 2 6 0 0 1
1 0 0x1000000100000 1 4 0 2 0
2 0 0x1000000200000 1 4 0 2 1
0 1 0x1000000300000 7 13 0 0 1
1 1 0x1000000400000 41 85 12 2 0
2 1 0x1000000500000 2 6 0 2 0
0 2 0x1000000600000 30 31 0 0 0
1 2 0x1000000700000 2 6 0 0 1
2 2 0x1000000800000 2 6 0 0 1
```

With `--json` the object has `version`, `tilesX`, `tilesZ`, `tileWidth`, `tileHeight`, `maxTiles`, `maxPolys`,
`tileCount`, `polys`, `verts` and `offMesh`, and with `--tiles` a list `tiles` of objects with `x`, `z`, `ref`,
`polys`, `verts`, `offMesh`, `parts` and `obstacles`:

```text
$ dayzmake navmesh info --json mymap.nm
{"maxPolys":1048576,"maxTiles":9,"offMesh":12,"polys":88,"tileCount":9,"tileHeight":50.0,"tileWidth":50.0,"tilesX":3,"tilesZ":3,"version":9,"verts":161}
```

A file that is not a navmesh set is an error.
