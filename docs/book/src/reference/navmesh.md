# Navigation meshes: navmesh

A navmesh (`.nm` file) tells the AI of DayZ where infected, animals and players' pathfinding can walk. A terrain mod
ships one next to its world. `dayzmake navmesh` builds it. `generate` asks the **navmesh data server** of
`DayZDiag_x64.exe` for the geometry of the world, tile by tile, and builds the mesh from it. `probe` shows what the
server says about the world. `info` summarises a `.nm` file. The [guide](../guides/navmesh.md) goes from a built
terrain mod to a mesh in the game. This page lists every command.

The generator is a Rust port of the Recast and Detour libraries, with the changes that DayZ made to them. Its output
does not depend on the number of threads and connections: the same input gives the same bytes. On x86 and x86_64,
the generator uses the same approximate reciprocal square root instruction as DayZ. On other CPUs (ARM, for example),
it uses a plain division. A slope very close to the maximum slope can be classified differently there. The
instruction is approximate, and its result can differ between x86 CPU makers (Intel and AMD). The same slope case
can therefore also be classified differently on two x86 machines.

The examples use a small synthetic world of 3 by 3 tiles of 50 by 50 m. A test server serves the world, and the
examples save it as the capture `world.dzgeo`. For the game, use the address of the machine that runs
`DayZDiag_x64.exe`. The port is always 36533. The program produced the outputs below.

## Data servers and captures

`probe` and `generate` talk to a data server. This is `DayZDiag_x64.exe`, started with `-server
-startNavmeshDataServer`. The server listens on TCP port 36533 (fixed by the game). Write a host as `host` or
`host:port`. The port defaults to 36533, and the host defaults to `127.0.0.1`. dayzmake resolves names to IPv4
addresses (it does not support IPv6).

The data server opens its port about 25 seconds after `DayZDiag_x64.exe` starts. A connection that you make while the
world is still loading can be reset (`Connection reset by peer`). Repeat `probe` until the server answers. The
[guide](../guides/navmesh.md) shows the command line, the mission and the config that the server needs. We compared
the builder with the navmeshes that ship with DayZ 1.30. The ignored test `navmesh_live` of `dayzmake-core` repeats
the comparison. It uses the variables `DAYZ_NAVMESH`, `DAYZ_NAVMESH_CAPTURE` and `DAYZ_NAVMESH_INNER`, and the
optional `DAYZ_NAVMESH_RANGE`. Run it with `cargo test -p dayzmake-core --test navmesh_live -- --ignored`. The guide
lists what we compared and the result. The inner tiles of the central half of Badlands are identical to the shipped
mesh, except 60 tiles with known causes. We did not compare the polygon flags, and we did not compare the link array
and its size.

`probe`, and the request for the world parameters at the start of `generate`, make one attempt. The command fails at
once when the connection cannot be made within 10 seconds, or when the answer fails.

Tile requests connect when they are first needed. A tile request can fail on the network (a connect that fails, a
closed connection, a timeout, a wrong answer). dayzmake then repeats it on a fresh connection, five attempts in all.
It waits 1, 2, 3 and 4 seconds between the attempts (10 seconds of waiting). An attempt that cannot connect can also
wait up to the 10 second connect timeout, so a host that does not answer at all costs about a minute.

A connection can still fail after its five attempts. A server that serves one client at a time refuses or resets the
other connections in this way. That connection then stops, and its tile goes back to the queue for the connections
that are left. A tile can fail on two connections that had delivered a tile, or on one that had and another one. Then
dayzmake reports the tile as not fetched, and the connection that reported it carries on. One bad tile therefore
costs one tile, not the run. Failures on connections that never delivered a tile (a refused or reset start) count
only together with a failure on a connection that had. A refused start therefore loses no tile.

A complete answer that is wrong (not a tile, another tile, an unknown cell class) is a failure of that tile alone.
The connection is not dead, because dayzmake read the whole frame, and the next request uses a fresh connection.
dayzmake reports a tile as not fetched because of dead connections only when no connection is left. The text of its
error is the last error of the first connection that died. A server that does not deliver a whole message within 10
minutes counts as failed. The deadline covers the message, so a server that sends bytes very slowly also fails.
dayzmake does not repeat a malformed answer. It checks and limits all data from a server before it uses the data.

Use one connection for each `DayZDiag_x64.exe` (`--connections` defaults to one for each server). It serves one client
at a time, and it refuses or resets the other connections. With three or more connections to one such server, a tile
can also fail on two connections that had each delivered a tile. dayzmake then reports the tile as not fetched,
although the server can serve it. The printed `to continue` command tries it again.

A **capture** (`.dzgeo`) is a recording of the geometry that a server sent. `generate --record` writes one while it
fetches. `generate --capture` builds from one without any server. A capture lets you rebuild the mesh without
starting the game, after a change of the generator or on another machine.

`generate --capture` does not load the file. It reads the world parameters and the position of every tile record (a
few bytes for each record, found by skipping over the tile data). It then reads only the tiles of the range, when it
builds them. It puts only the tiles of `--range` into the index. A range of a capture of many gigabytes therefore
needs memory in proportion to the tiles in that range. A 3 x 3 range of a 50 GB capture of 53,000 tiles takes a few
seconds and about 300 MB. The format is unchanged, so existing captures work.

dayzmake ignores records that lie outside the world grid (one message says how many). It refuses a capture with more
records than the world has tiles. A library caller that opens a capture with `CaptureFile` and a `range` must pass the
range of the run to `generate`. A run whose range the indexed range does not cover is an error, not a mesh without
those tiles. dayzmake ignores a tile that is cut off at the end of a capture. It reports a malformed tile record when
it builds the tile, as a failed tile, not when it opens the file.

## dayzmake navmesh probe

Ask the data server for the world parameters. Optionally, ask for one tile.

<div class="synopsis">

```text
dayzmake navmesh probe [-v] [--json] [--server <host>] [--tile <x,z>]
```

</div>

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Remove the limit of ten warnings for each warning name. |
| `--json` | off | Print one JSON object `{"params": {...}, "tile": {...}}` instead of text. `tile` is only there with `--tile`. |
| `--server <host>` | `127.0.0.1` | The data server, `host` or `host:port`. |
| `--tile <x,z>` | none | Also fetch tile `x,z` (tile coordinates, from 0) and summarise it. |

The text output has these parts, in this order:

- the world size and the tile grid;
- the sea, contour and block parameters;
- the island filter switch and seed;
- the agent (the sizes that the mesh is built for);
- one line for each cell class (the resolution of a tile);
- one line for each jump link class;
- with `--tile`, a line for the tile and one line for each group of triangles. `area` is the surface type, `parts` is
  the number of door and object parts, and `verts` is the number of vertices.

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

When nothing listens on the port, the command fails with status 1. It says how to start the server:

```text
$ dayzmake navmesh probe --server 127.0.0.1:9
error: cannot connect to 127.0.0.1:9: Connection refused (os error 111); start DayZDiag_x64.exe with -server -startNavmeshDataServer
```

## dayzmake navmesh generate

Build a navmesh from a data server or a capture. Write it to `<target>`.

<div class="synopsis">

```text
dayzmake navmesh generate [-v] [-f] [--server <host>]... [--capture <dzgeo>] [--record <dzgeo>] [--range <x0,z0,x1,z1>] [--base <nm>] [--skip-existing] [--checkpoint <n>] [--threads <n>] [--connections <n>] [--sea-level <m>] [--doors <state>] [--no-links] [--no-island-filter] [--seed <x,y,z>] <target>
```

</div>

| Argument | Meaning |
|---|---|
| `<target>` | The `.nm` file to write. dayzmake writes it atomically (a temporary file next to it, renamed at the end). |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Remove the limit of ten warnings for each warning name. |
| `-f`, `--force` | off | Overwrite an existing `<target>` and an existing `--record` file. Without it, an existing target is an error. dayzmake reports it before it fetches anything. |
| `--server <host>` | `127.0.0.1` | A data server, `host` or `host:port`. You can repeat it. dayzmake spreads the connections over the servers. The servers must serve the same world (several `DayZDiag_x64.exe` on several machines build faster). The world parameters come from the first server. Do not use it together with `--capture`. |
| `--capture <dzgeo>` | none | Build from this capture instead of a server. dayzmake leaves out the tiles that the capture does not have. Without `--range`, the range is the bounding box of the captured tiles. |
| `--record <dzgeo>` | none | Also write the fetched tiles to a capture file. It must not be the file that you read with `--capture`. dayzmake refuses the run, because writing the file would destroy the capture. |
| `--range <x0,z0,x1,z1>` | the whole world | The tiles to build: columns `x0` to `x1` and rows `z0` to `z1`, inclusive. dayzmake cuts values below 0 or beyond the grid to the world. An upper bound below 0 means the last tile. A range with nothing left is the error `empty tile range`. |
| `--base <nm>` | none | Start from this navmesh. dayzmake keeps its grid, and the new tiles replace the tiles of their cells. The navmesh must fit the world (grid, tile size and origin), or the run fails with `base navmesh does not match the world`. |
| `--skip-existing` | off | Leave out the tiles that `--base` already has. It needs `--base` (`--skip-existing needs --base`). |
| `--checkpoint <n>` | 0 (off) | Write the target after every `n` finished tiles (built, empty or failed). Each checkpoint rewrites the whole file, so use a large `n` (hundreds or thousands of tiles) for a big world. A negative value is an error. 0 means off. A checkpoint that dayzmake cannot write stops the run, and the run is then a failure (see below). |
| `--threads <n>` | cores - 1, at least 1 | Builder threads, at most 256. 0 means the default. A negative number is an error. |
| `--connections <n>` | one per server (1 for a capture) | Connections to the servers, at most 256. 0 means the default. A negative number is an error. |
| `--sea-level <m>` | the server's deep sea height | The sea level in metres. |
| `--doors <state>` | `closed` | The state of the doors that dayzmake bakes into the mesh. `closed`: dayzmake disables the polygons of open doors, but not those of closed doors. `open`: the other way round. `enabled`: dayzmake disables no door polygon. |
| `--no-links` | links on | Leave out jump, climb and ladder links. |
| `--no-island-filter` | filter on | Do not flag polygons that cannot be reached from the seed. |
| `--seed <x,y,z>` | the world's seed position | Where the island filter starts (world coordinates in metres). |

The command prints a progress line to standard error. On a terminal, it rewrites the line in place. When you
redirect the output, the line appears at most once for each 5 %:

```text
1/9 tiles (0 failed), 21.8 tiles/s, 0:00:00 left
```

Then the command prints the summary to standard output. The summary has these parts:

- the number of tiles built, empty and failed, and the time;
- the first twenty failures, indented;
- a line when tiles had door part numbers outside their range (dayzmake raises the warning `navmesh-stray-part` for
  them);
- the result of the island filter;
- `wrote <target>`.

```text
$ dayzmake navmesh generate --capture world.dzgeo --threads 2 mymap.nm
built 9 tiles, 0 empty, 0 failed in 0.4 s
island filter: seed (60.0 10.0 60.0) -> 84 reachable, 1 isolated polys
wrote mymap.nm
```

A tile without a walkable surface is *empty*. dayzmake removes the tile of that cell from a `--base` mesh. A tile that
dayzmake cannot build is *failed*. It raises the warning `navmesh-tile-failed` and does not stop the run. The island
filter runs over the whole mesh at the end, so a resumed run (below) filters the merged mesh. When the seed finds no
polygon, dayzmake skips the filter with the warning `navmesh-seed`.

Memory: the tiles that dayzmake fetched but did not yet build or apply wait in queues. The length of the queues is
limited, but their size is not. A tile record can be large (a frame can be up to 1 GiB), and nothing limits the total
bytes. A world with huge tiles therefore needs memory in proportion. `--record` keeps each fetched tile until it
writes the tile.

dayzmake writes the target **even when the run was partial**. The exit status is 0 when dayzmake handled every tile.
The exit status is 1 when a tile failed, when dayzmake could not fetch a tile, or when a checkpoint or `--record`
write failed.

A run can be **stopped** (a checkpoint or `--record` write failed). A run can also have tiles that dayzmake could not
fetch, and no tile that failed to build. In both cases the run ends with the command that continues it. The command
repeats every option of the run, except `--record` (with `-f` it would overwrite the first recording) and `--base`.
`--base` becomes the target, so dayzmake builds the rest with the same settings. dayzmake does not offer a retry for
a tile that failed to *build*, because it would fail again. The printed command is quoted for a POSIX shell (single
quotes around anything but letters, digits and `_@%+=:,./-`). It works in bash and zsh. In PowerShell and `cmd.exe`,
change the quotes (`'\''` does not work in PowerShell). A server that closes every connection gives this result for
one tile (five attempts, 1+2+3+4 s between them, 10 seconds in all):

```text
$ dayzmake navmesh generate --server 127.0.0.1:36533 --range 0,0,0,0 --threads 2 lost.nm
warning: island filter skipped: no polygon near the seed (60.0 10.0 60.0) [navmesh-seed]
built 0 tiles, 0 empty, 0 failed in 10.0 s
  tile (0,0): connection to 127.0.0.1:36533 closed by the server
wrote lost.nm
to continue: dayzmake navmesh generate --base lost.nm --skip-existing -f --server 127.0.0.1:36533 --range 0,0,0,0 --threads 2 lost.nm
error: 0 tiles could not be built and 1 could not be fetched; the mesh was saved without them
```

`--base <target> --skip-existing` over the same range builds only the tiles that are still missing. The result is
the same as the result of an uninterrupted run:

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

(The first run raised the warning `navmesh-seed` shown above, because the seed is in a row that dayzmake did not build.)

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
| `-v`, `--verbose` | off | Remove the limit of ten warnings for each warning name. |
| `--json` | off | Print one JSON object instead of text. |
| `--tiles` | off | List every tile: `x z ref polys verts offmesh parts obstacles`. |

The summary shows the file version as dayzmake reads it (7 or 9; `generate` always writes 9), the grid and the size of
a tile, and the number of tile slots (`max tiles`). It also shows the number of tiles and the totals of polygons,
vertices and off-mesh connections (the jump, climb and ladder links). `ref` is the tile reference in hex, `parts` is
the door and object parts of the tile, and `obstacles` is its tree obstacles.

```text
$ dayzmake navmesh info --tiles mymap.nm
navmesh version 9, grid 3 x 3 tiles of 50.0 x 50.0 m, max tiles 9
tiles 9, polys 85, verts 159, off-mesh connections 12
x z ref polys verts offmesh parts obstacles
0 0 0x1000000000000 2 6 0 0 1
1 0 0x1000000100000 1 4 0 2 0
2 0 0x1000000200000 1 4 0 2 1
0 1 0x1000000300000 4 8 0 0 1
1 1 0x1000000400000 41 85 12 2 0
2 1 0x1000000500000 2 6 0 2 0
0 2 0x1000000600000 30 34 0 0 0
1 2 0x1000000700000 2 6 0 0 1
2 2 0x1000000800000 2 6 0 0 1
```

With `--json` the object has `version`, `tilesX`, `tilesZ`, `tileWidth`, `tileHeight`, `maxTiles`, `maxPolys`,
`tileCount`, `polys`, `verts` and `offMesh`, and with `--tiles` a list `tiles` of objects with `x`, `z`, `ref`,
`polys`, `verts`, `offMesh`, `parts` and `obstacles`:

```text
$ dayzmake navmesh info --json mymap.nm
{"maxPolys":1048576,"maxTiles":9,"offMesh":12,"polys":85,"tileCount":9,"tileHeight":50.0,"tileWidth":50.0,"tilesX":3,"tilesZ":3,"version":9,"verts":159}
```

A file that is not a navmesh set is an error.
