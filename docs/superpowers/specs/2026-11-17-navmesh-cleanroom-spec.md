# Flare Navgen navmesh generator: functional specification

Upstream commit studied: `7d2b0b8` (2026-10-21, "Format the engine's reader imports", app version 0.3.0).

Clean-room functional specification; contains no code from Flare Navgen.

Purpose: let an engineer who never sees the source reimplement the navmesh generator (data-server client, per-tile
Recast build, jump-link generation, Detour assembly, `.nm` writer, CLI) in Rust under GPL-2.0-or-later. Everything
below is behaviour, constants and formulas observed in the source. Recast and Detour themselves are zlib-licensed, so
their function and struct names are used freely. Where this document says "stock", it means upstream Recast/Detour
behaviour that the reimplementer can read in upstream's (zlib) source.

Contents

0. Which implementation is current
1. Overview and pipeline
2. CLI
3. Data-server protocol
4. Generation parameters (world params to Recast config)
5. Per-tile build, step by step
6. Jump, climb and ladder links
7. Tile finalisation, Detour blob and `.nm` extra section
8. Assembling the navmesh (Detour `addTile`), island filter, tile order, parallelism
9. `.nm` file format
10. Recast/Detour version and the list of DayZ modifications
11. Probe output
12. Validation, limits and error cases
13. Test vectors
14. Open questions

---

## 0. Which implementation is current

The repository holds two generations of the engine:

* **Current (version 0.3.0):** a Rust workspace `engine/` with three crates. `dznav` (library: protocol client,
  capture files, generator scheduling, `.nm` reader/writer, comparison, preview), `dznav-cli` (the `dznav` binary)
  and `dznav-sys` (builds the C++ core with the `cc` crate behind a small C interface). The C++ core is still C++: tile
  building (`TileBuilder.cpp`, `JumpLinks.cpp`, `WorldInput.cpp`) and the DayZ-patched, vendored
  `third_party/recastnavigation` (Recast and Detour, built with `DT_POLYREF64`). Rust owns everything else.
* **Old, kept as a reference ("oracle"):** the all-C++ CLI in `src/cli/main.cpp` plus `src/dznav/*.cpp`. The Rust
  engine's tests run both and require byte-identical `.nm` files and equal JSON events. So the two are behaviourally
  the same; this spec describes that shared behaviour.
* `app/` is a Tauri GUI that runs `dznav --json` as a sidecar; it is out of scope.

The algorithmically interesting code (tile builder, jump links, the Recast/Detour patches) lives in the C++ core. A
Rust reimplementation must reproduce those algorithms. The Recast passes are stock Recast with a short list of DayZ
changes (section 10); porting stock Recast/Detour to Rust (or binding upstream through FFI and patching) plus those
changes is the intended route.

The goal of the original is **byte-identical tiles** to what DayZ's own tool produces from the same geometry
(validated on tens of thousands of tiles; see section 13). Many details below exist only to reproduce DayZ's float
rounding and quirks, so apparent oddities are deliberate and must be kept.

---

## 1. Overview and pipeline

End to end:

1. The user runs `DayZDiag_x64 -server -mod=<mods> -startNavmeshDataServer -config=serverDZ_navmesh.cfg` with an
   empty mission for the terrain. It listens on TCP port **36533** and serves one client at a time.
2. `dznav generate out.nm` connects (default host 127.0.0.1) and sends message 2 (request world params). The server
   answers with message 4 (protocol version 10, world block, cell classes, link classes).
3. The generator plans the tile list (rectangular range, blocks of `blockSize` tiles) and runs three kinds of workers:
   * **fetchers** (one per server by default): for each planned tile send message 3 (request tile) and receive
     message 5 (tile geometry: triangle soup in groups by area id, with object parts);
   * **builders** (CPU cores minus one): turn a tile's geometry into a Detour tile blob (Recast pipeline + jump links
     + Detour `dtCreateNavMeshData`), with no links to neighbours yet;
   * **one assembler** (the main thread): adds each finished blob to a single `dtNavMesh` (`addTile`), which links
     the tile to its neighbours and to itself (Detour is not thread-safe).
4. After the last tile (and unless stopped), an **island filter** floods the mesh from a seed position and sets poly
   flag 0x8000 on every unreachable polygon.
5. The mesh is exported tile by tile (in Detour tile-slot order) into the `.nm` file, written to `<out>.tmp` and
   renamed over `<out>`.

The server is the bottleneck, so the tool can fetch from several DayZDiag instances in parallel, can record the
fetched geometry into a `.dzgeo` capture file and can regenerate offline from one.

---

## 2. CLI

Binary name `dznav`. `--json` may appear anywhere on the command line; it is removed before arguments are
interpreted, and switches output to JSON lines (see section 2.4). Numbers are parsed like C `atoi` / `atof` (leading
number, else 0; trailing junk ignored).

### 2.1 Commands (all)

| Command | Meaning |
|---|---|
| `generate <out.nm> [options]` | the generator (2.2) |
| `probe [host] [x z]` | print world params; with `x z` also fetch and summarise tile (x,z). `x z` are only recognised when a host is given first (three arguments). Section 11. |
| `capture <out.dzgeo> <x0> <z0> <x1> <z1> [host]` | save tiles of an inclusive range (row by row, z outer, x inner) to a capture file |
| `capture-info <file.dzgeo>` | print world params, tile count, summaries of the first 5 tiles |
| `replay-server <file.dzgeo>... [--bind addr] [--port N] [--crop] [--sea-level M] [--no-island-filter] [--once]` | act as a data server serving captured tiles (section 3.9) |
| `info <file.nm> [--tiles]` | summary of a navmesh |
| `tile <file.nm> <x> <z>` | dump one tile |
| `roundtrip <file.nm>` | parse and re-serialise, compare bytes |
| `export-obj <file.nm> <out.obj> [x0 z0 x1 z1]` | OBJ export |
| `render <file.nm> <out.png> <minX> <minZ> <maxX> <maxZ> [px/m]` | top-down PNG preview |
| `diff <a.nm> <b.nm> [top N]` | tile by tile comparison |
| `verify <reference.nm> <ours.nm> [--ignore-order]` | canonical byte comparison of common tiles |
| `merge <base.nm> <patch.nm> <out.nm> [x0 z0 x1 z1]` | replace base tiles with patch tiles, relink |
| `resave <in.nm> <out.nm>` | load into Detour and save |
| `islands <in.nm> <out.nm> <x> <y> <z> [--all-heights]` | re-run the island filter |

Only `generate`, `probe` and the file format matter for a generator reimplementation; the inspection commands are
convenience tooling. `--json` is rejected (error kind `usage`) for commands other than: generate, probe, capture,
info, tile, render, verify, diff, merge, islands.

With no arguments or an unknown command: usage text on stderr, exit code **2**.

### 2.2 `generate` options

| Option | Default | Meaning |
|---|---|---|
| `--server h1[,h2,...]` | `127.0.0.1` | data server hosts (IPv4 name or address); port always 36533 |
| `--capture <file.dzgeo>` | none | use captured geometry instead of a server |
| `--range x0 z0 x1 z1` | whole world (or the capture's bounding range of tile coords) | inclusive tile range; clamped to the world |
| `--base <file.nm>` | none | load an existing navmesh and patch generated tiles into it |
| `--skip-existing` | off | only build tiles (layer 0) the base does not already have; error "--skip-existing needs --base" without `--base` |
| `--checkpoint N` | 0 (off) | save the output after every N finished tiles |
| `--record <file.dzgeo>` | none | write every fetched tile input to a capture file as it arrives |
| `--threads N` | cores - 1 (min 1) | builder threads (negative values clamp to 0, meaning default) |
| `--connections N` | number of hosts (1 with `--capture`) | fetcher count |
| `--sea-level M` | the server's `deepSeaHeight` | overrides the sea level (float) |
| `--doors closed\|open\|enabled` | `closed` | baked door state; any other value acts as `closed` |
| `--no-links` | links on | skip jump, climb and ladder links entirely |
| `--no-island-filter` | filter on | skip the island filter |
| `--seed x y z` | world params' `seedPosition` | island filter seed |

An unknown option or a missing option value is an error ("unknown option X" / "missing value for X", error kind
`file`). `--help` does not exist.

### 2.3 `generate` run behaviour

1. Parse arguments. If `--capture`: read the capture (section 3.8); the source is the capture. Else the source is the
   data server(s).
2. Get world params: from the capture, or request message 2 from the first host (single attempt, no retry).
3. Create the target mesh: with `--base`, load that `.nm` (its header's grid, tile size and origin are kept and its
   tiles are added in file order with their tile refs); otherwise a fresh Detour navmesh with
   `orig = (0,0,0)`, `tileWidth/Height = world tile size`, `maxTiles = tilesX * tilesZ`, `maxPolys = 1 << 20`.
4. Plan tiles (section 8.4). With `--capture` and no `--range`, the range is the bounding box of captured tile
   coordinates. With `--skip-existing`, tiles present in the base (layer 0) are removed from the plan.
5. Run fetchers/builders/assembler. A tile that arrives for build is built (section 5). Outcomes per tile:
   * **built**: blob added to the mesh, replacing an existing tile at that grid cell;
   * **empty**: no walkable surface (no contours or no polygons): the tile at that grid cell is **removed** from the
     mesh if present; counted `empty`;
   * **failed**: build or add error; recorded as `tile (x,z): <message>`; counted `failed`.
   A tile with no input available (capture lacks it) is silently skipped.
6. Ctrl+C (first press) or, in `--json` mode, a line `stop` on stdin sets a cancel flag: no new tile is fetched,
   unbuilt queued tiles are discarded, running builds finish. A second Ctrl+C exits immediately with code 130. Closing
   stdin does not stop.
7. Checkpoints: every N finished tiles (built, empty and failed), write the whole mesh to the output (atomic, see
   9.6). A failing checkpoint cancels the run and is reported after it.
8. After the run, if not cancelled and `--no-island-filter` not given and the world's `filterIsolatedIslands != 0`:
   run the island filter (section 8.5) with the seed `--seed` or `seedPosition`, half-extent search vertical range 30.
   If the seed finds no polygon, report "island filter skipped" (not an error).
9. Save unless (cancelled and zero tiles built/empty). A stopped run skips the island filter.
10. Exit code: **0** success; **1** when any tile failed or any error string was recorded (including fetch errors);
    **130** when cancelled.
11. "Resumable" is `cancelled || (errors non-empty && failed == 0)`. The suggested resume is `--base <out> --skip-existing`.
    The resumed run re-runs the island filter over the whole merged mesh.

Console (non-JSON) output: a progress line rewritten with `\r`:
`{done}/{total} tiles ({failed} failed), {rate:.1} tiles/s, {h}:{mm:02}:{ss:02} left` followed by three spaces, where
`done = built + empty + failed`, `rate = done / max(elapsed, 0.001 s)`, `eta = (total - done) / rate`. After the run:
`built {n} tiles, {n} empty, {n} failed in {s:.1} s` (prefixed `stopped: ` when cancelled), the first 20 errors
(indented two spaces), a line about out-of-range door part ids when any tile had them, the island-filter line
`island filter: seed ({:.1} {:.1} {:.1}) -> {reachable} reachable, {isolated} isolated polys`, then `wrote <out>`
and the resume hint.

### 2.4 Errors and `--json`

Failures: human mode prints `\n<label>: <message>` to stderr with label `network error` / `protocol error` / `error`;
JSON mode prints `{"event":"error","kind":"usage|file|network|protocol","message":...}`. Exit code 1. Error kinds:
network failures (connect, timeout, closed, bad frame) are `network`; malformed message payloads are `protocol`;
bad files, bad arguments and core errors are `file`.

JSON event streams for `generate` (`start`, `fetched`, `tile`, `progress` at most 4 per second plus a final one,
`saved` for checkpoints, `islands`, `done`) are fully specified in `docs/cli-json.md` of the original and summarised
here: `done` has `output, saved, cancelled, built, empty, failed, seconds, strayTiles, resumable, errorCount, errors`
(first 100 messages). A reimplementation is free to match this contract or not; it is not part of the data format.

---

## 3. Data-server protocol

All integers and floats are **little-endian**, IEEE-754 binary32 for floats. There is no handshake, no checksum and no
compression.

### 3.1 Transport and framing

* TCP over IPv4 only (name resolution takes the first IPv4 address). Port **36533** default. `TCP_NODELAY` enabled.
* Connect timeout 10 s. Each receive waits up to **600 s** (10 minutes) for a complete message; a timeout is a network
  error.
* Frame: `u32 length` (the number of bytes after this field, i.e. 4 + payload size), `u32 messageId`, then
  `length - 4` bytes of payload. A receiver must reject `length < 4` or `length > 2^30` (1 GiB) as an invalid frame.
* One request is outstanding per connection. The server handles one client connection at a time (DayZDiag serves one
  client per frame and drops broken connections), so parallel fetching needs several servers (several machines/instances).
* There is no handshake: a client may send a tile request as the first message of a connection without having
  requested world params on it (secondary fetch connections do exactly that).

### 3.2 Message ids

| Id | Direction | Payload |
|---|---|---|
| 1 | client to server | empty: "finished", the server shuts down. Defined but never sent by this tool. |
| 2 | client to server | empty: request world params |
| 3 | client to server | tile request (24 bytes, 3.3) |
| 4 | server to client | world params (3.4) |
| 5 | server to client | one tile's input geometry (3.5) |

No error message id is known. The client treats any other id in answer position as an error
("expected world params, got message N" / "expected tile, got message N").

### 3.3 Tile request (id 3), 24 bytes

| Offset | Type | Field |
|---|---|---|
| 0 | i32 | tile x |
| 4 | i32 | tile z |
| 8 | f32 | minX = `float(x) * tileWidth` |
| 12 | f32 | minZ = `float(z) * tileHeight` |
| 16 | f32 | maxX = `float(x + 1) * tileWidth` |
| 20 | f32 | maxZ = `float(z + 1) * tileHeight` |

The whole frame is thus `0x1C 00 00 00  03 00 00 00` + 24 bytes. `tileWidth`/`tileHeight` come from the world block.
(The replay server only reads x and z.)

### 3.4 World params (id 4)

Payload = `u32 protocolVersion` (must be **10**; otherwise error "server protocol version V, expected 10"), then:

**World block, 0x54 (84) bytes** (struct is packed, no padding other than the explicit pad bytes):

| Offset | Type | Field (descriptive name) |
|---|---|---|
| 0x00 | f32 | worldSizeX (m) |
| 0x04 | f32 | worldSizeZ (m) |
| 0x08 | f32 | tileWidth (m) |
| 0x0C | f32 | tileHeight (m) |
| 0x10 | i32 | tilesX |
| 0x14 | i32 | tilesZ |
| 0x18 | f32 | deepSeaHeight: top of the sea box (area 5). This is the **live tide value** and varies between runs |
| 0x1C | f32 | deepWaterDepth (m): water deeper than this becomes "deep water" (area 3) |
| 0x20 | f32 | contourMaxError (m) |
| 0x24 | u8 + 3 pad | obstacles: non-zero = extract tree parts (area 10) as cylinder obstacles |
| 0x28 | i32 | blockSize: scheduler block edge length in tiles |
| 0x2C | u8 + 3 pad | filterIsolatedIslands |
| 0x30 | f32[3] | seedPosition (x, y, z) for the island filter |
| 0x3C | f32 | agent standHeight |
| 0x40 | f32 | agent crouchHeight |
| 0x44 | f32 | agent proneHeight |
| 0x48 | f32 | agent diameter |
| 0x4C | f32 | agent maxStepHeight |
| 0x50 | f32 | agent maxSlope (degrees) |

Then `u32 n` cell classes, each **16 bytes**:

| Offset | Type | Field |
|---|---|---|
| 0 | i32 | cells: number of cells across the tile width (see below) |
| 4 | u32 | second cell count (observed equal to `cells`; unused by the generator) |
| 8 | f32 | width in metres covered by `cells` (observed = tile width) |
| 12 | u32 | unused (docs name it a height; the generator ignores it; test fixtures use 0) |

Cell size of a class `cs = width / float(cells)` (float division). Shipped maps use 0.25 m and 0.1 m (cells 200 and
500 over 50 m). The tile's heightfield is square: both axes use `cells` and `cs`.

Then `u32 m` link classes (off-mesh jump/climb link types), each **52 (0x34) bytes**, in config order:

| Offset | Type | Field |
|---|---|---|
| 0x00 | u32 | index: becomes the off-mesh connection `userId` |
| 0x04 | u32 | name hash (unused) |
| 0x08 | i32 | typeId (-1 if unset; unused by the generator) |
| 0x0C | f32 | jumpHeight |
| 0x10 | f32 | jumpLength |
| 0x14 | f32 | jumpDropdownMin |
| 0x18 | f32 | jumpDropdownMax |
| 0x1C | f32 | minCenterHeight (sign selects the obstacle-height test, see 6.4) |
| 0x20 | f32 | agentHeight |
| 0x24 | f32 | maxStepHeight |
| 0x28 | f32 | agentRadius |
| 0x2C | u16 | poly flags assigned to the generated off-mesh polygons |
| 0x2E | u8 | area id assigned to them (48 + N for class N in shipped maps; must fit 6 bits) |
| 0x2F | u8 | padding (0) |
| 0x30 | i32 | debug colour (unused) |

The payload must be consumed exactly (no trailing bytes).

### 3.5 Tile input (id 5)

```
i32  x
i32  z
u32  cellClass          index into the cell-class list
u32  groupCount
f32  bmin[3]            tile bounds; xz = tile footprint (x*tileWidth .. (x+1)*tileWidth), y = min of geometry
f32  bmax[3]            y = max of geometry
repeat groupCount times:
  u32 area
  u32 partCount
  partCount times: u32 partIndex, u64 objectId, u32 firstVertex     (16 bytes)
  i32 vertexCount
  f32 verts[vertexCount][3]                                         (x, y up, z)
```

Part `i` covers vertices `[parts[i].firstVertex, parts[i+1].firstVertex)`; the last part runs to `vertexCount`.
Vertices are a **triangle soup** (3 consecutive vertices per triangle; a trailing 1-2 vertices are ignored), except
ladders (area 15) which are **vertex pairs** (a segment from bottom to top).

Observed input area ids and meaning:

| Area | Meaning |
|---|---|
| 1 | terrain; always the first group; terrain grid, extends about 24-25 m beyond the tile on each side |
| 2 | water surface |
| 3, 4, 5 | (navmesh output areas; ignored as input) |
| 6, 7, 8, 9, 11 | building / object geometry (8 = building proper; the meaning of 6, 7, 9, 11 is not documented) |
| 10 | tree obstacle parts: one collision mesh per part, object id 0 |
| 12 / 13 | door footprint, opened / closed state. One part per door state; parts of the two states of one door carry the same objectId and partIndex 2k / 2k+1 |
| 14 | "hide parts" (not described by the docs; rasterised like doors, see 5.4) |
| 15 | ladders (vertex pairs) |
| 18 | "WallWithHole" ceilings (used for crawl/crouch clearance) |

The part index is **not** used for anything except being copied into the output extra section (as `partIndex`).
Objects' door **state is not sent** by the server (hence `--doors`).

### 3.6 Client behaviour and retry

* The world params request is tried once.
* A tile request failing with a **network-class** error (connect failure, send/recv failure, timeout, peer closed,
  invalid frame length, wrong message id, answered tile coordinates differ from those asked, unknown cell class) closes
  the connection, sleeps `2 * (attempt + 1)` seconds (2, 4, 6, 8) and retries on a fresh connection; the 5th failure
  (attempts 0..4) is final. The error for that tile is recorded as `tile (x,z): <message>` and **that fetcher stops**
  (its remaining tiles are not fetched by it; other fetchers continue).
* A **wire-class** error (payload malformed: see 12.1) is not retried; the tile is recorded as failed and the fetcher
  continues with its next tile.
* Invariants checked on each tile: answered `(x,z)` equals the requested one; `cellClass < cellClassCount`.

### 3.7 Multiple servers

Hosts are numbered in the order given. Fetcher 0 uses host 0; the k-th additional connection uses host
`k mod hostCount` (round robin), counting from 1. Fetchers pull the next planned tile from a shared atomic counter.

### 3.8 Capture files (`.dzgeo`)

Little-endian: ASCII `DZGC`, `u32 version = 1`, then a block `u32 size, bytes` holding the world params (the message-4
payload **without** the leading protocol version), then repeated blocks `u32 size, bytes` each holding one tile body
(message-5 payload). A capture cut short keeps every complete tile; a partial trailing block is ignored. Bad magic:
"not a capture file"; other version: "unsupported capture version"; truncated before the world block: "capture
truncated". A tile captured twice is served as the later copy. Tiles are written as they arrive (so a crashed run
keeps its prefix).

### 3.9 Replay server (for testing a client)

Listens (default 127.0.0.1:36533, IPv4 address required), serves each client in its own thread. Message 2: reply 4 with
`u32 10` plus the (possibly modified) params. Message 3: read x, z (first 8 bytes); reply 5 with the captured tile, or
for an uncaptured tile a synthetic one: cell class 1 if more than one cell class else 0, bounds
`(x0, -41, z0)`-`(x1, -39, z1)` (with `x0 = x*tileWidth`, `x1 = x0+tileWidth`, etc.), one terrain group (area 1) of two
triangles forming a flat quad at y = -40 padded 24 m on all sides (vertices
`(x0-24,-40,z0-24) (x1+24,-40,z0-24) (x0-24,-40,z1+24)` and `(x1+24,-40,z0-24) (x1+24,-40,z1+24) (x0-24,-40,z1+24)`),
no parts. Message 1: close that client. Other ids are ignored. Options: `--crop` shrinks the served world to the
captured tiles' grid `(max x + 1, max z + 1)` and recomputes world size; `--sea-level` overrides deepSeaHeight;
`--no-island-filter` zeroes the filter flag.

---

## 4. Generation parameters: world params to Recast config

All arithmetic is **single precision (f32)** and the results depend on it. `ceil` is `ceilf`, `cos` is the float
cosine. Casting `(int)` truncates toward zero. Let `CC = cellClasses[tile.cellClass]` and `B` the world block.

| Quantity | Formula |
|---|---|
| tile cells `N` (also `tileSize`) | `CC.cells` |
| cell size `cs` | `CC.width / float(CC.cells)` |
| cell height `ch` | `0.1f` (fixed) |
| slope threshold | `cos((B.maxSlope / 180.0f) * 3.1415927f)` (the constant is the f32 literal 3.1415927) |
| walkableHeight (voxels) | `(int) ceil(B.standHeight / 0.1f)` (e.g. 15 for 1.5 m) |
| crouchHeight (voxels) | `(int) ceil(B.crouchHeight / 0.1f) - 2` (e.g. 8) |
| proneHeight (voxels) | `(int) ceil(B.proneHeight / 0.1f) - 2` (e.g. 3) |
| walkableClimb (voxels) | `(int) ceil(B.maxStepHeight / 0.1f)` (e.g. 5) |
| walkableRadius (voxels) | `(int) ceil((B.diameter * 0.5f) / cs)` (2 for cs 0.25 and diameter 0.6; 3 for cs 0.1) |
| deepWaterDepth (voxels) | `(int) ceil(B.deepWaterDepth / 0.1f)` |
| seaLevel (voxels) | `(int) ceil(S / 0.1f)` where `S = --sea-level` if given else `B.deepSeaHeight` |
| borderSize | `max(10, walkableRadius - (int)(-3.0f / cs))` (14 for cs 0.25, 33 for cs 0.1) |
| heightfield width = height | `N + 2 * borderSize` |
| pad (m) | `float(borderSize) * cs` |
| padded bounds | `bmin.x = in.bmin.x - pad`, `bmin.y = in.bmin.y`, `bmin.z = in.bmin.z - pad`; `bmax.x = in.bmax.x + pad`, `bmax.y = in.bmax.y`, `bmax.z = in.bmax.z + pad` |
| maxEdgeLen | 120 (voxels) |
| maxSimplificationError | `B.contourMaxError / cs` (cells) |
| maxVerticalError | `8.0f` (voxels), a DayZ addition to Recast's contour simplifier |
| minRegionArea (default) | `(int)((1.0f / cs) * (1.0f / cs))` (16 for 0.25, 100 for 0.1) |
| minRegionArea per area id | the default for all ids 0..63, except ids 10, 12, 13 and 14 which are `INT_MAX` (a connected group whose first region has one of these areas and does not touch the tile border is always removed as too small, see §6.2 and §10.1 item 6) |
| mergeRegionArea | 400 |
| vertsPerPoly (nvp) | 6 |
| detail mesh | none (no `rcBuildPolyMeshDetail`) |
| Detour header agent values | height `B.standHeight`, radius `B.diameter * 0.5f`, climb `B.maxStepHeight` (metres) |
| BV tree | built; quantisation `1 / cs` |

Fixed constants not derived from the world: sea box floor y = -500; door dilation radius `(int)(float(walkableRadius) * 0.8f)`;
tree clearance 2.0 m, tree maximum footprint 3.0 m; mode filter window 21 x 21 (radius 10); jump-link constants in
section 6.

The world's `worldSize*`, `blockSize` (besides tile ordering), `nameHash`, `typeId` and `color` fields do not enter
the geometry.

---

## 5. Per-tile build, step by step

Input: world params, one tile input, build options (links on/off, door policy, sea level override). Output: either
"empty" (no walkable surface), an error, or a Detour tile blob. Allocate a Recast context that records error logs so a
failure can report them. Builds for different tiles are independent and run in parallel.

Recast `rcSpan` here carries, besides `smin/smax/area`, a 16-bit **part id** `subArea` (used for door parts and tree
indices). Spans store `smin`, `smax` in 13 bits each (`RC_SPAN_MAX_HEIGHT = 8191`); the compact heightfield carries
the part id too.

### 5.1 Setup

1. Compute config (section 4).
2. Build the **tile extras list** (needed at finalisation, 7.2): walk all groups in order; skip groups of area 10
   (for these only remember the number of parts of the last such group, `treeItems`; it is assigned, not summed);
   for every part of every other group append an extra part record with `objectId`, `partIndex`, `area` = the group's
   area (as a byte) and state: door state policy `closed`: state 0 if area 12 else 1; `open`: state 0 if area 13 else
   1; `enabled`: 1. (State 0 means the polygons of this part get the disabled flag 0x2.)
3. `rcCreateHeightfield(width, width, paddedBmin, paddedBmax, cs, ch)`.

### 5.2 Triangle handling and orientation

The server's triangles are wound opposite to Recast's expectation: every triangle `(v0, v1, v2)` is passed to the
rasteriser as `(v0, v2, v1)`. Walkability uses the original triangle with a specific normal computation:

* `e1 = v1 - v0`, `e2 = v2 - v0`; `n = cross(e2, e1)` (components `(e2.y*e1.z - e2.z*e1.y, e2.z*e1.x - e2.x*e1.z,
  e2.x*e1.y - e2.y*e1.x)`); `d = (n.x*n.x + n.y*n.y) + n.z*n.z`.
* If `d > FLT_MIN`: `r = rsqrt_approx(d)` using the x86 SSE instruction `RSQRTSS` (hardware approximation; Enfusion
  does this), `inv = (0.5*r) * (3.0 - (d*r)*r)` (one Newton step), `n.y = n.y * inv`. Otherwise `n.y` stays unnormalised.
* The triangle is walkable iff `slopeThreshold < n.y`.

Note: `RSQRTSS` precision differs between CPU vendors (Intel vs AMD). Byte equality with DayZ's own output relies on
using the same instruction; use `core::arch::x86_64::_mm_rsqrt_ss` and accept that results can differ on other
hardware. Triangles failing the slope test are **still rasterised** with area `RC_NULL_AREA` (0): they occupy space as
unwalkable spans.

Rasterisation of a triangle into the heightfield clips it against grid cells like stock Recast's `rasterizeTri`
(see 5.3 for the five per-cell actions and the `flagMergeThreshold`, which is `1` for all triangles).

### 5.3 The DayZ rasteriser

Common cell clipping (stock Recast `rasterizeTri` algorithm): bounding-box overlap test against the heightfield; rows
`z0 = clamp((int)((triMin.z - hf.bmin.z) * invCs), -1, h-1)`, `z1 = clamp(..., 0, h-1)`; polygon is divided against
each row (and then each column) with Recast's `dividePoly` (max 12 vertices); row/column index `-1` pieces are clipped
away and discarded; a cell piece becomes a span range
`smin = clamp((int)floorf((minY - hf.bmin.y) * invCh), 0, 8191)`,
`smax = clamp((int)ceilf((maxY - hf.bmin.y) * invCh), smin + 1, 8191)`, after clamping `minY/maxY` to the heightfield
height `[0, hf.bmax.y - hf.bmin.y]` (a piece wholly above or below is skipped). `invCs = 1.0f/cs`, `invCh = 1.0f/ch`.

The per-cell action depends on a **mode**; `column` means the cell's span list in ascending height order; `(smin,
smax)` is the new piece's voxel range; all comparisons are on integers.

* **ADD (mode 0):** stock `addSpan(hf, x, z, smin, smax, area, flagMergeThreshold)`: merges overlapping spans; when
  merging, the merged span's area is `max(newArea, oldArea)` if the merged `smax` differs from the old span's `smax` by
  at most `flagMergeThreshold`, else the new area is kept for the merged span. The span's `subArea` is set to 0 for a
  new span and is not inherited by merging. (A per-column "hint" is a pure speed optimisation.)
* **PAINT_BELOW (mode 1), used for doors:** find `s` = the last span in the column whose `smin <= smin`, or the first
  span if none. Then for `s` and the spans above it, in order: if `smax <= s.smax` stop (success); if `s.area != 0`:
  if `s.area == area && s.subArea == subArea` return **repeat** (stop); else set `s.area = area`, `s.subArea = subArea`.
  Returns success at the end of the column.
  * Used per triangle (when the group has no parts), a *repeat* aborts the remaining cells of that triangle (the
    abort is not an error).
  * Used per part, a *repeat* only ends that cell.
* **TREE (mode 2), used for area 10 parts:** for each span `s` in the column: stop if `smax < s.smin`; stop if
  `smax <= s.smax + walkableClimb`; `hit = (s.smax < smin) ? (smin < s.smax + walkableHeight) : (s.smax < smax)`; if
  `hit`: `s.area = area (10)`, `s.subArea = subArea`. Nothing is added: trees paint the ground around the trunk.
* **WATER (mode 3):** for each span in the column: stop if `smax <= s.smin`; if `s.smin + waterDepth <= smin`:
  `s.area = max(s.area, area)`. (Also upgrades unwalkable spans to water.)
* **PAINT_OR_ADD (mode 4), used for hide parts:** "paint touching": for each span: stop if `smax < s.smin`; stop if
  `smax <= s.smax + 1`; if `s.smax >= smin`: paint `(area, subArea)` and note a hit. If nothing was hit, `addSpan` the
  piece with the given `flagMergeThreshold`.

**Part rasterisation** (door and hide parts, "as one footprint"): first rasterise all triangles of the part (still
`(v0, v2, v1)` order) into a per-cell record of the union of vertical extents `lo[cell] = min smin`, `hi[cell] = max
smax` (no span is touched yet); then for each cell with a record (row by row, `z` outer, `x` inner) apply the mode's
cell action with `(lo, hi)`. Part rasterisation clips "the older way": the first row and column index are clamped to
**0 instead of -1** (so the piece at the min edge keeps its geometry beyond the heightfield edge), and then any cell
piece having a vertex with `z < hf.bmin.z` or `x < hf.bmin.x` is **skipped** (so a part crossing the min x/z edge never
paints row or column 0 there). This quirk matters: it controls which cells doors paint and thereby which regions
survive the link-stage filter (6.1).

### 5.4 Steps

1. **Terrain.** Rasterise `groups[0]` (assumed terrain) with the slope test, mode ADD, area = the group's area,
   subArea 0.
2. **Mode filter** (only if `borderSize > 10`): build `src[cell]` = area of the **lowest span** of the column (0 if
   the column is empty). For every column with `borderSize <= x,z < width - borderSize`, set `dst` = the most frequent
   non-zero value of `src` over the 21 x 21 window centred on the column (ties go to the lowest area id; 0 if every
   window value is 0). Columns in the border keep `dst = src`. Finally write `dst` into the **lowest span** of every
   non-empty column (this can turn an unwalkable lowest span walkable). Window reads stay inside the field because
   `borderSize >= 10`.
3. **Objects**, for each group in order, skipping every group whose area is 1 (only group 0 was terrain) and
   collecting groups of area 15 (ladders) for later; with `partCounter = 0` initially, by area:
   * **6, 7, 8, 9, 11:** ADD with the slope test, area = group area.
   * **2 (water):** two passes over all triangles of the group with mode WATER, no slope test: first area 2 with
     `waterDepth = walkableClimb` (voxels, ruling R14), then area 3 with `waterDepth = deepWaterDepth` (voxels). (The
     pond cell test of mode WATER is in 5.3.)
   * **10 (trees):** if the group has no parts: every triangle with mode TREE, area 10, subArea 0. Else for each part
     `k` (0-based within the group): if `B.obstacles != 0`: mode TREE, area 10, subArea `k`; otherwise the part's
     triangles are added as ADD with area 0 (a plain blocker), subArea 0.
   * **12, 13 (doors):** no parts: per triangle PAINT_BELOW (area, subArea 0). With parts: for each part call part
     rasterisation with PAINT_BELOW, area = group area, subArea = `partCounter++`, flagMergeThreshold = walkableHeight.
   * **14 (hide parts):** no parts: per triangle PAINT_OR_ADD, subArea 0, threshold 1. With parts: part
     rasterisation with PAINT_OR_ADD, area 14, subArea = `partCounter++`, threshold 1.
   * **18:** ADD, no slope test, area 18.
   * any other area: ignored.
   Part ids for doors/hides are a running counter in group-then-part order across all area 12, 13 and 14 groups; the
   extras list built in 5.1 indexes parts of **all** non-tree groups in group/part order, so the two numberings agree
   when the only groups with parts are 12/13/14/10 (the shipped situation). Tree ids and door ids share the same
   16-bit span field; this is the root of the "stray part id" case (12.3).
4. **Span filters**, in this order:
   1. *Multi-height low-height filter* with heights `(proneHeight, crouchHeight, walkableHeight)` = (3, 8, 15 in the
      example) and areas `(0, 16, 17)`, flagged ceiling area 18: for each column and each span `s` that has a span
      `next` above it, `gap = next.smin - s.smax`. If `next.area != 18`: if `gap <= walkableHeight`, `s.area = 0`.
      Else (ceiling is area 18): the first `i` in 0,1,2 with `gap <= heights[i]` sets `s.area = areas[i]` (crawl 16,
      crouch 17, or 0). This overwrites the area even when it was already 0.
   2. stock `rcFilterLowHangingWalkableObstacles(walkableClimb)`, extended so that when it promotes a non-walkable span
      to the area of the walkable span below it, it also copies that span's `subArea`.
   3. stock `rcFilterLedgeSpans(walkableHeight, walkableClimb, skipAreas)`, with a DayZ `skipAreas[64]` parameter: a
      span whose area has `skipAreas[area] != 0` is skipped (here areas 0 and 1 are skipped, i.e. terrain is exempt;
      area 0 is exempt as usual). Everything else is the stock ledge/steep-slope test (stock uses the walkable-height
      test on neighbour gaps, climb test, and the accessible-neighbour min/max test).
5. **Compact heightfield:** `rcBuildCompactHeightfield(walkableHeight = proneHeight, walkableClimb, hf)`. (Stock
   code; it adds `proneHeight * ch` to `bmax.y` of the compact field and copies span `subArea`.)
6. **Post-compact area passes** (`postCompact`):
   1. *Sea box:* `rcMarkBoxArea` over `x,z` = padded bounds, `y` from -500 to `float(seaLevelVoxels) * ch`, area 5.
      (Stock semantics: truncating conversion of the box to voxel indices; every walkable span whose `y` lies in
      range gets area 5, overwriting any prior area including 10; part ids stay.)
   2. *Dilations* (`dzDilateArea`, definition below), mask = every area except 12, 13, 14: dilate area **13** by
      `doorRadius`; dilate **12** by `doorRadius`; dilate **14** by `walkableRadius`; dilate **16** by `walkableRadius`.
   3. *Tree obstacles* (only if `B.obstacles != 0` and `treeItems > 0`): `dzExtractTreeObstacles` (definition below),
      with tree area 10, walkable area 1, `partCount = treeItems`, seed areas {0, 10, 12, 13, 14}, min clearance 2.0 m,
      max footprint 3.0 m. Result: a list of `(x, y, z, radius)`.
   4. Stock `rcErodeWalkableArea(walkableRadius)` (marks spans with chamfer distance `< 2*radius` as area 0).
   5. Remap areas: 7 and 9 become 1; 11 becomes 8.
   6. Roof rule: for each column, if its **topmost** span has area 8 and `h == 255` (which is the case for the topmost
      span of every column) its area becomes 1.
7. **Links** (if links enabled): section 6. It uses the final compact heightfield (modifying only its region ids) and
   the still-existing solid heightfield, which is then freed.
8. Stock `rcBuildDistanceField`, then **`rcBuildRegions`** (watershed) with `borderSize` and the per-area min-area table
   and merge area 400 (DayZ: regions never cross a change of `(area, subArea)`; see section 10).
9. **`rcBuildContours`** with `maxError = maxSimplificationError`, `maxVertError = 8.0`, `maxEdgeLen = 120`, flag
   `RC_CONTOUR_TESS_WALL_EDGES`. No contours: return "empty tile".
10. **`rcBuildPolyMesh`** with nvp = 6. Zero polygons: "empty tile". `nverts >= 0xffff`: error "Too many vertices per tile
    N (max: 65535)". Border vertices are never removed (DayZ).
11. Finalisation (section 7).

Messages for stock failures used by the original (not required): e.g. "buildNavigation: Could not create solid
heightfield.", "...Could not build compact data.", "...Could not erode.", "...Could not build distance field.",
"...Could not build regions.", "...Could not create contours.", "...Could not triangulate contours.", "Could not build
Detour navmesh.".

#### dzDilateArea(radius, target area, mask)

Chamfer distance transform over the compact heightfield's 4-connected spans: `dist` is a byte per span, initialised to
255 except `0` for spans with `area == target`. A parallel `sub` array copies each span's `subArea`. Run the two passes
below; every time a span's `dist` is strictly improved from neighbour `m`, also copy `sub[m]` to it. Pass 1 (rows `y`
ascending, `x` ascending), for each span: via direction 0 (x-1) neighbour `a`: candidate `dist[a]+2`; then via `a`'s
direction 3 (y-1): candidate `dist+3`; then via direction 3 (y-1) neighbour: `+2`, then via that neighbour's
direction 2 (x+1): `+3`. Pass 2 (rows descending, `x` descending): direction 2 (+x) `+2` then via its direction 1
(+y) `+3`; direction 1 (+y) `+2` then via its direction 0 (-x) `+3`. (Directions: 0 = -x, 1 = +z, 2 = +x, 3 = -z
in the usual Recast convention with offsets `x: {-1,0,1,0}`, `z: {0,1,0,-1}`; "z" is the second grid coordinate.)
Improvement means `dist[neighbour] + k < dist[span]` (compared as ints, store as byte). Afterwards, every span with
`mask[area] != 0` and `dist <= radius * 2` gets `area = target`, `subArea = sub[span]`.

#### dzExtractTreeObstacles

Inputs as in 5.4 step 6.3. Distances use the same chamfer machinery as above (the same pass order, byte distances,
copying both **area and part id** from the nearest seed). Algorithm:

1. Seeds: every span whose area has `seedAreas[area] != 0` gets distance 0; others 255. After the chamfer, each span
   carries `(label area, label part id, distance)` of its nearest seed (spans no seed can reach keep their own).
2. For each part `p < partCount` compute `minDist[p]` (u16, initially 65535). For every span whose label area is 10:
   for each of its 4 directions: if there is no connection: candidate `d = own distance`; else if the neighbour's label
   `(area, part)` equals its own: nothing; else `d = neighbour distance + own distance`. Then
   `minDist[labelPart] = min(minDist[labelPart], d)` when `labelPart < partCount`.
3. `thr = (u16)(int)((2.0f + 2.0f) / cs)`. For every span (rows `y` outer, columns `x`, spans of the cell in order)
   with its **own** area 10 and own part id `p < partCount` and `thr < minDist[p]`: accumulate per part `count`,
   `sum x`, `sum span.y`, `sum z` (grid cell indices of the padded grid), and min/max `x` and `z`.
4. `maxCells = (int)(3.0f / cs)`. For each part with `count > 0`: `size = maxZ - minZ`; `wx = maxX - minX`; if
   `size + 1 <= wx` then `size = wx`. If `maxCells < size + 1`: reject the part (`minDist[p] = 0`, `count = 0`); else
   remember footprint cells `size + 1`.
5. Rewrite every span whose own area is 10: with `p` = its label part id (for a tree span this equals its own), the
   area becomes 1 (walkable) if `p < partCount && thr < minDist[p]`, else 0 (unwalkable); its part id becomes 0.
6. For each part with `count > 0` in index order, output one obstacle:
   `x = ((float)sumX / (float)count + (float)chf.borderSize + 0.5f) * cs + chf.bmin.x`,
   `y = ((float)sumY / (float)count + (float)chf.borderSize + 0.5f) * ch + chf.bmin.y`,
   `z = ((float)sumZ / (float)count + (float)chf.borderSize + 0.5f) * cs + chf.bmin.z`,
   `radius = (float)footprintCells * cs * 0.5f`. At this point `chf.borderSize` is **0** (the field has not been
   through region building yet), and `bmin` is the padded origin.

Net effect: a tree whose nearest other seed is farther than 2 m and whose footprint is at most 3 m becomes walkable
terrain with an obstacle cylinder record; other trees become holes.

---

## 6. Jump, climb and ladder links

Runs when links are enabled (`--no-links` disables **everything** in this section including ladders). Produces a list
of off-mesh connections (start point, end point, centre height, radius, flags, area, bidirectional flag, userId) in
`dtNavMeshCreateParams` layout. They are passed to `dtCreateNavMeshData`, which stores only those whose **start point
lies inside the tile** (7.3).

### 6.1 Ladders

For each ladder group (area 15) in group order, for each vertex pair `(verts[i], verts[i+1])`, `i = 0, 2, 4, ...`
(`i + 1 < vertexCount`): one connection: start = first vertex, end = second vertex, centre height 0.0, radius 1.0,
flags `0x40`, area 15, direction value 1 (bidirectional flag), userId 0. These come first in the list. (A source
comment calls them one-way and says they are unused in shipped maps; the code passes direction value 1. See open
questions.)

### 6.2 Candidate edges

If the world has no link classes, stop after ladders.

1. **Monotone regions and contours on the final heightfield.** Call `rcBuildRegionsMonotone(chf, borderSize = 1,
   minRegionAreaByArea, mergeRegionArea)` (DayZ: monotone sweeps also stop at changes of subArea), then
   `rcBuildContours(chf, maxSimplificationError, maxVerticalError, maxEdgeLen, RC_CONTOUR_TESS_WALL_EDGES)`.
   Because the heightfield has been eroded (eroded spans stay in the compact field with area 0) the walkable regions
   are separated from the 1-cell border, so every group of connected regions is subject to the minimum-size filter
   with the threshold taken from the **lowest-numbered region of the group**; if that region is a door, tree or hide
   part (threshold `INT_MAX`) the whole connected group is dropped and yields no links. DayZ's own tool behaves the
   same; whether a group is dropped depends on exactly which cells door parts painted (5.3's min-edge quirk).
   Contour vertices in the contour set are in grid units relative to `cset.bmin`, which is the padded origin shifted
   by `borderSize * cs` on x and z. Convert a contour vertex `v = (vx, vy, vz)` to world: 
   `P(v) = (vx*cs + cset.bmin.x, (vy + 2)*ch + cset.bmin.y, vz*cs + cset.bmin.z)` (note the **+2 voxels** on y).
2. **Edge segments.** For each contour, for each consecutive vertex pair `(vj, vi)` with `j` the index before `i`
   (cyclically): consider the edge only if vertex `vj`'s flag word has `RC_AREA_BORDER` (0x20000) set; make a segment
   from `P(vi)` to `P(vj)`; keep it only if its squared 3-D length is `>= 1.5` (m², not m). (Area-border flags are set
   in contours where the walked edge borders a connected span of different area or part, including eroded-to-0 spans,
   which is why edges next to obstacles qualify.) Keep segments in contour order as `segs`.
3. **Depth.** `depth = 3.0`; for each link class `c`: `depth = min(depth, max(B.diameter * 3.0, c.jumpLength))`.
4. For each segment `s` in `segs` run the facing-edge search `refineEdge(s, depth, width = 1.0, segs, chf.cs)` (6.3);
   among its output segments choose the one with the largest squared 3-D length (strictly greater than 0, ties keep the
   first), and append it to the `edges` list if one exists.

### 6.3 refineEdge(s, depth, width, segs, cs)

Find all other segments that face `s` across an obstacle within `depth`, group them, and return the midlines.

1. `p0 = s.p0`, `d = s.p1 - s.p0`, `len = |d|` (3-D). `u = d * (1.0f / len)` if `len >= FLT_EPSILON` (every normalisation in §6 multiplies by the reciprocal of the length, ruling R16) else `(d.x, 1, d.z)`.
   `n = (u.z, 0, -u.x)`, normalised by its 3-D length if that is `>= FLT_EPSILON`, else `(u.z, 1, -u.x)`.
2. Local coordinates of a world point `q` (let `dq = q - p0`): `local = (u.x*dq.x + n.x*dq.z,  u.y*dq.x + dq.y + n.y*dq.z,
   u.z*dq.x + n.z*dq.z)`. Box in local space: `min = (0, -width/2, 0)`, `max = (len, +width/2, depth)`.
3. For every segment `k` in `segs` (including `s` itself): transform its endpoints `a`, `b` to local, `e = b - a`;
   clip the parametric segment `a + t*e` against the box: `tmin = 0`, `tmax = FLT_MAX`; for each axis: if `|e| < 1e-6`
   reject when `a` lies outside `[min,max]` on that axis; else `ood = 1.0f/e`, `t1 = (min - a)*ood`,
   `t2 = (max - a)*ood` (a multiplication by the reciprocal, ruling R15), swap so `t1 <= t2`, `tmin = max(tmin, t1)`,
   `tmax = min(tmax, t2)`, reject if `tmax < tmin`. Accept if not rejected and
   `tmin <= 1` and `tmax >= 0`.
4. For accepted segments compute the world direction of segment `k` flattened to y = 0: `v = (p1.x - p0.x, 0, p1.z -
   p0.z)`, normalised if the 2-D length is `>= FLT_EPSILON` (else `v.y = 1`). Require `dot(v, u) <= cos(2.3561945f)`
   (cos 135 degrees, 3-component dot with the 3-D unit vector `u`) and fewer than 64 records so far. Record
   `{group = 0, seg = k, t0 = clamp(tmin, 0, 1), t1 = clamp(tmax, 0, 1), zmin = min(a.z, b.z), zmax = max(a.z, b.z)}`
   with `a.z`, `b.z` being the **local** z.
5. **Grouping:** `g = 1`; for each record `i` in order that has `group == 0`: set `group = g`; for each later record
   `j > i` (even if it already has a group): if `rec[i].zmin - cs <= cs + rec[j].zmax` and
   `rec[j].zmin - cs <= cs + rec[i].zmax` then set `rec[j].group = g`. Then `g += 1`.
6. For each group id `1..g-1`: `smin = 10`, `smax = -10`; for each record of the group, take its segment `q` and the
   two points `c = q.p0 + (q.p1 - q.p0) * t0` and `c = q.p0 + (q.p1 - q.p0) * t1`; project onto `d` with
   `t = (c.y - p0.y)*d.y + (c.x - p0.x)*d.x + (c.z - p0.z)*d.z` (that summation order), clamp: `t <= 0 -> 0`,
   `t < dd -> t/dd`, else `1` (`dd = d.x*d.x + d.y*d.y + d.z*d.z`); if `t < smin` record `smin = t`, `minPt = c`; if
   `smax < t` record `smax = t`, `maxPt = c`. If `smin <= smax` and fewer than 32 outputs so far emit the segment
   `(e0 + (minPt - e0)*0.5, e1 + (maxPt - e1)*0.5)` with `e0 = p0 + d*smin`, `e1 = p0 + d*smax`.

### 6.4 Per link class

Link classes are processed in config order; for each one the `spans` list starts empty. Let `r = agentRadius`,
`cs = chf.cs`, `ch = chf.ch`.

**Trajectory.** `total = (r + r) + jumpLength`; `nT = (int)ceil(total / cs)`; sample `i` has `x_i = float(i) * cs` and
`y_i = maxStepHeight` if `x_i < r`; `dropdownMin` if `x_i >= total - r`; else `jumpHeight`. Then for each `i`:
window `lo = max(x_i - r, x_0)`, `hi = min(x_i + r, x_{nT-1})`; `ymin_i` = min and `ymax_i` = max of `y_j` over all `j`
with `lo <= x_j < hi` (starting from `y_i`); finally `ymax_i += agentHeight` (the class's agentHeight). Trajectory
heights are relative (taken above the start floor).

**Sampling.** Constants: `h = total * 0.5`, `L2 = jumpLength * 0.5`, `mch = minCenterHeight`, `invNT = 1.0f/float(nT)`,
`minRun = (int)ceil((r + r) / cs)`, water area range 2..6 inclusive. For **each edge `E` in `edges`, two passes**
(pass 0 jumps along `+n`, pass 1 along `-n`; spans of both are appended in that order; this two-sided sweep is DayZ 1.30
behaviour, earlier versions swept one side):

1. `A = E.p0`, `B = E.p1`, `side = +1` (pass 0) or `-1` (pass 1). `dn = normalize3(B - A) * side` (`normalize3(v) = v * (1.0f / |v|)`, ruling R16) (if the 3-D length
   is `< FLT_EPSILON` use `(B.x - A.x, 1, B.z - A.z)` unnormalised); `n = normalize3((dn.z, 0, -dn.x))` (fallback
   `(dn.z, 1, -dn.x)`).
2. `S0 = n*(-h) + A`; `dir = (n*(-h) + B) - S0` (both sums with rounded products, in f32; with the reciprocal-normalised `n` this is the shipped value and `B - A` is not, see 14.15 and 14.18); `P0 = n*(-L2) + A`; `P1 = n*L2 + A`; `Ebase = n*h + A` (computed
   **once**; it must not be recomputed as `(dir*t + n*h) + A`, which rounds differently and changes cell lookups on
   about 1 % of tiles); `dirLen = |dir|`; `nS = (int)ceil(dirLen / cs)`; skip the pass if `nS <= 0`;
   `step = (nS == 1) ? 0 : 1.0f / float(nS - 1)`. `state[0..nS) = 1` (invalid), `value[] = 0`.
3. For `k = 0..nS-1`, `t = float(k) * step`, `S = dir*t + S0`, `E_k = dir*t + Ebase`, `cy = dir.y*t + A.y`:
   1. `yS` = floor of `S` in the **compact** heightfield: cell `(floor((S.x - bmin.x)/cs), floor((S.z - bmin.z)/cs))`,
      among that cell's spans with area != 0 the one whose height `span.y * ch + bmin.y` is closest to `S.y`
      (strictly closer wins; first wins ties); no cell or no span: leave invalid.
   2. Require `yS <= dropdownMin + maxStep + S.y` and `S.y - (maxStep - dropdownMax) <= yS`.
   3. `ys = yS + ch`; `yTop = ys + traj[nT-1].ymin`.
   4. `yE` = same lookup at `E_k`; require `yE <= maxStep + yTop` and `(yTop - dropdownMin + dropdownMax) - maxStep <= yE`.
   5. `ye = yE + ch`.
   6. Initialise `hiU = 0`, `loU = 10000` (u16), `last = 0` (declared per sample and **not** reset between trajectory
      steps unless the column has spans). For `j = 0..nT-1` (stop at the first failure):
      * `u = float(j) * invNT`; `y = ys + (ye - ys)*u`; `x = S.x + (E_k.x - S.x)*u`; `z = S.z + (E_k.z - S.z)*u`;
        solid-heightfield cell `(cx, cz) = ((int)((x - bmin.x)/cs), (int)((z - bmin.z)/cs))` (truncating); `inRange`
        = within `[0,w) x [0,h)`.
      * If `(j == 0 || j == nT-1)` and `inRange` and the **compact** cell has spans and the area of its first compact
        span is in 2..6: result **water** (state 2); stop.
      * If `inRange` and the solid column has spans: `last = 0`; `top = (u32)(i64)((y + traj[j].ymax - bmin.y)/ch)`,
        `bot = (u32)(i64)((y + traj[j].ymin - bmin.y)/ch)` (negative values wrap as unsigned); walk the spans in order
        keeping `prev` (the previously visited span): break if `top < s.smin`; `last = s.smax`; if `bot <= s.smax &&
        s.smin <= top` then result **blocked** (state 1) and stop. After the walk (without a block): if `prev` exists and
        its area is in 2..6, result **water** and stop.
      * `hiU = max(hiU, (u16)last)`; `loU = min(loU, (u16)last)`.
   7. If a failure state was set, `state[k] = that`, continue to the next `k`. Otherwise `hsel = (mch >= 0) ? hiU : loU`;
      `centerH = float(hsel + 1) * ch + bmin.y - cy`; `value[k] = centerH`; `state[k] = 1` (invalid) if
      `(mch >= 0 && centerH < mch)` or `(mch < 0 && centerH > mch)`, else `0` (valid).
4. **Run extraction.** Scan `k = 0..nS` (with `k = nS` acting as an invalid sentinel). A run of valid samples
   `[start, k)` becomes a span if its length `k - start > minRun`: `t0 = float(start)*step`, `t1 = float(k-1)*step`;
   `s0 = P0 + dir*t0`, `s1 = P0 + dir*t1`, `e0 = P1 + dir*t0`, `e1 = P1 + dir*t1`; `centerH = value[k-1]`; then for
   each of the four points set its y to the **top** (`smax*ch + bmin.y`) of the **solid** span in that column
   (`(int)` truncating cell index, any area) whose top is closest to the point's y; if out of range or the column is
   empty leave y. Span validity starts at 1 (one-way). Append to `spans`.

**Reverse merge.** `thr = (cs*5)^2`. For `i` over spans in order (skip if validity 0), for `j > i` (skip if validity 0):
if `distSq(b.e0, a.s0) < thr && distSq(b.e1, a.s1) < thr && distSq(b.s0, a.e0) < thr && distSq(b.s1, a.e1) < thr`
(`distSq` is the 3-D squared distance), then compare `|a.s1 - a.s0|^2` with `|b.s1 - b.s0|^2`: if `a <= b`:
`a.validity = 0`, `b.validity = 2`; else `a.validity = 2`, `b.validity = 0`, `a.centerH = b.centerH`; then stop
searching for this `i`. Validity 2 means bidirectional. (The original docs claim each span merges at most once; the
logic only skips validity-0 spans, so a validity-2 span can be matched again as `a`. Implement the literal logic.)

**Emission.** For each span of this class with validity != 0 (in `spans` order): `d = s1 - s0`, `len = |d|`,
`d2 = 2 * r`. If `len >= 2*d2`: `rem = len - 2*d2`, `f = floor(rem * 0.5)`, `off = (rem - (f + f)) * 0.5 + d2`; else
`rem = 0`, `off = len * 0.5`. `count = 1 - (int)(rem * -0.5f)`. For `i = 0..count-1`: `w = (len > 0) ? off / len : 0`;
`start = s0 + d*w`, `end = e0 + d*w`; add a connection with those endpoints, centre height `span.centerH`, radius
`off` if `count == 1` else `1.0`, flags = class flags, area = class area, direction value `1` if validity != 1 (i.e. 2)
else 0, userId = class index; then `off += 2.0`. (So one link per ~2 m along the span, centred.)

Connections are therefore ordered: ladders, then per class in config order, spans in order, links along the span.

---

## 7. Tile finalisation, Detour blob and `.nm` extra section

### 7.1 Poly flags

For every polygon `i` of the poly mesh set `flags[i]` from `areas[i]` (starting from 0):

| Area | Flags |
|---|---|
| 1 (terrain), 6, 7, 9, 14 | 0x01 |
| 2 (water), 4 (sea) | 0x11 |
| 3 (deep water) | 0x10 |
| 5 (deep sea) | 0x20 |
| 8 (building) | 0x09 |
| 12 (door open), 13 (door closed) | 0x05 |
| 16 (crawl) | 0x0400 |
| 17 (crouch) | 0x0800 |
| anything else (e.g. 10, 15, 18) | unchanged (0) |

Bit meanings: 0x1 walk, 0x2 disabled, 0x4 door, 0x8 inside, 0x10 swim, 0x20 swimsea, 0x40 ladder, 0x80 jumpover, 0x100
jumpdown (0x180 = jump), 0x200 climb, 0x400 crawl, 0x800 crouch, 0x8000 isolated (set later by the island filter).
Ground polygon area ids that can appear in output are 1, 2, 3, 5, 8, 12, 13, 14, 16, 17 (and 10 if a tile has area-10
geometry without parts, so no obstacle extraction ran); off-mesh polygons carry 48+N (link classes) or 15 (ladders).
Area 4 is never produced by this pipeline, and 6, 7, 9, 11 are remapped away before meshing.

### 7.2 Extra section content

If the extras list from 5.1 is non-empty: for each polygon `i` with part id `p = polyMesh.subAreas[i]`:
if `p >= extras.size()` count a "stray part id" for the tile and skip the polygon; if `extras[p].area != areas[i]` skip;
otherwise set (`state == 0`) or clear (`state != 0`) flag 0x2 on the polygon and append `i` to `extras[p].polys`.
Polygon indices are in ascending order.

Extra section bytes (little-endian):

```
u32 partCount (= extras.size(), counting all parts of all non-tree groups, including those with no polygons)
u32 partPolyCount (= total polygons attached to parts)
partCount x TilePart  { u64 objectId, u32 partIndex, i32 firstPoly }        // 16 bytes
partPolyCount x TilePartPoly { u32 poly, i32 next }                         // 8 bytes
u32 obstacleCount
obstacleCount x { f32 x, f32 y, f32 z, f32 radius }                          // 16 bytes
```

`firstPoly` is the index into the `TilePartPoly` array of the part's first entry (running sum over parts in order of the
poly counts of previous parts) or `-1` if the part has no polygons. Within a part, entry `k` of its `m` entries has
`next` = the array index of its successor (the global index of the next entry) or `-1` for the last. The `partIndex`
is the server's value (2k + state in practice). Obstacles come from `dzExtractTreeObstacles` (empty if disabled).
Size = `12 + 16*parts + 8*partPolys + 16*obstacles`.

### 7.3 dtCreateNavMeshData (DayZ-modified)

Call with: `verts/polys/areas/flags` from the poly mesh, `nvp = 6`, off-mesh arrays (verts, radius, centre height, flags,
areas, direction, userId), `extraData/extraDataSize`, tile `x`, tile `y = z grid index`, layer 0, userId 0,
`bmin/bmax` from the poly mesh (below), `walkableHeight = standHeight`, `walkableRadius = diameter*0.5`,
`walkableClimb = maxStepHeight`, `cs`, `ch = 0.1`, `buildBvTree = true`. No detail mesh is supplied.

Poly mesh bounds (computed in f32 literally): `bmin.x = (in.bmin.x - pad) + pad`, `bmin.z` likewise, `bmax.x = (in.bmax.x
+ pad) - pad`, `bmax.z` likewise (not necessarily bit-equal to the input bounds); `bmin.y = in.bmin.y`;
`bmax.y = in.bmax.y + float(proneHeight) * 0.1f` (from `rcBuildCompactHeightfield`). Vertex coordinates:
`v = (bmin.x + iv.x*cs, bmin.y + iv.y*ch, bmin.z + iv.z*cs)` where `iv` are the poly mesh's u16 cell coordinates
(relative to the unpadded origin).

Behaviour (stock Detour with these differences):

* Header = `dtMeshHeader` (104 bytes, see 9.3) with magic `'DNAV'`, **version 12**, `extraDataSize` = size of the extra
  section (4-byte aligned), `detailMeshCount = 0`, `detailVertCount = 0`, `detailTriCount = 0`, `bvQuantFactor = 1/cs`,
  `bvNodeCount = 2 * groundPolyCount`, `offMeshBase = groundPolyCount`, `offMeshConCount` = stored connections.
* Off-mesh classification as stock: each endpoint is classified by side (stock `classifyOffMeshPoint` over tile
  `bmin/bmax` xz, "inside" = 0xff); **only connections whose start point is inside the tile are stored** (each stored
  connection adds one polygon and two vertices); a start point whose y is outside `[hmin - climb, hmax + climb]`
  (hmin/hmax over the tile's vertex heights) counts as outside. `maxLinkCount = edgeCount + portalCount*2 +
  offMeshLinkCount*2` where `offMeshLinkCount` counts **every endpoint inside the tile of every passed connection**,
  including connections that start in a neighbour tile (the padded heightfield sees those starts) and land here.
  (Those reserve link slots for incoming connections.)
* Off-mesh polygon: `vertCount = 2`, `flags` = class flags, area = class area, type = `DT_POLYTYPE_OFFMESH_CONNECTION`.
* `dtOffMeshConnection` entry (44 bytes, 9.4): `pos[6]`, `centerHeight = connection centre height`, `dzReserved = 0.0`,
  `rad`, `poly`, `flags = 1 if direction value != 0 else 0`, `side` = classification of the end point (0xff = in this
  tile), `userId`.
* No dummy detail mesh; the area of the detail section is empty.
* The BV tree is built over ground polygons with stock `createBVTree` with the MSVC `qsort` tie order (section 14 item 3, answered).
* The extra section is appended after the off-mesh connections.
* The links array is left zeroed (linking happens in `addTile`).
* Section sizes use `dtAlign4`; all section element sizes (104, 12, 32, 16, 12, 12, 4, 16, 44) make the layout
  contiguous without padding when the extra size is itself a multiple of 4 (it always is).

The stats for the tile: dataSize, header polyCount, vertCount, offMeshConCount (stored), number of obstacles, number of
stray part ids.

---

## 8. Assembling the navmesh

### 8.1 `dtNavMeshParams`

`orig = (0, 0, 0)`, `tileWidth = world tileWidth (e.g. 50.0)`, `tileHeight = world tileHeight (50.0)`,
`maxTiles = tilesX * tilesZ` (must be > 0, overflow checked), `maxPolys = 1 << 20`. 64-bit poly refs
(`DT_POLYREF64`): `salt` 16 bits, `tile` 28 bits, `poly` 20 bits, ref = `salt << 48 | tileIndex << 20 | polyIndex`.
`dtNavMesh::init`: all tile slots get `salt = 1`; free list so that slot 0 is allocated first. Header polygon-per-tile
limit is `1 << 20` (checked by ref width only).

### 8.2 Adding a tile (`setTile`)

* If a tile exists at the same `(x, y, layer 0)`: `removeTile` it (this increments the slot's salt), then add the new tile
  with `lastRef = ref(slot's current salt, slot index, 0)` so it takes the same slot and tile order is stable.
* `addTile(copyOfBlob, size, DT_TILE_FREE_DATA, lastRef)` (stock, with the DayZ changes in section 10). Tile refs for
  a fresh slot are `salt 1`, so `1 << 48 | index << 20`.
* `addTile` fails if the blob has wrong magic/version, the cell is occupied, or no slot is free.
* A tile marked "empty" by the build removes the existing tile if any (`removeTile`).

### 8.3 What `addTile` does (stock order, DayZ differences in section 10)

Patch data pointers (the extra section follows the off-mesh connections); build the link free list; `connectIntLinks`;
`baseOffMeshLinks` (also connects the tile's own off-mesh end points: `connectExtOffMeshLinks(tile, tile, -1)`);
connect with other layers at the same cell (none here); for each of the 8 neighbour cells' tiles: `connectExtLinks`
both ways and `connectExtOffMeshLinks` both ways; finally (DayZ) **snap each obstacle's Y**: for every obstacle in the
extra section, `findNearestPolyInTile(tile, obstaclePos, halfExtents = (0, 30, 0))`; if a polygon is found, set the
obstacle's y to the nearest point's y. (The `.nm` therefore stores snapped heights, and re-snapping is not idempotent.)

### 8.4 Tile order, ranges and parallelism

* Range `[x0, z0, x1, z1]` (inclusive). Effective: `x0 = max(0, x0)`, `z0 = max(0, z0)`, `x1 = (x1 < 0) ? tilesX - 1 :
  min(x1, tilesX - 1)`, same for z.
* Order: split the range into square blocks of `blockSize = max(1, B.blockSize)` tiles starting **at the range's
  `(x0, z0)`** (not aligned to a global grid): for `bz = z0, z0 + block, ...` (outer), `bx = x0, x0 + block, ...`
  (inner), then tiles inside the block row by row (`z` outer, `x` inner), clipped to `x1, z1`. This keeps the data
  server's streaming cache warm. An `include` predicate (used for `--skip-existing`) removes tiles from this list.
* Threads: fetchers (default one per host) pull the next index from an atomic counter. A bounded queue of at most **64**
  fetched-but-unbuilt tiles separates fetchers from builders; builders (default `cores - 1`, minimum 1) push results into
  a bounded queue of `builders * 4`; a single assembler consumes results **in completion order** and calls `setTile`.
  Consequently the tile-slot order (and the order in which links were allocated) depends on completion order unless a
  single builder is used (`--threads 1`; the original's parity tests use one thread for determinism).
* Failed fetch for tile: that fetcher stops (network class) or only that tile is lost (wire class).
* On cancel: queued tiles are dropped without being built.

### 8.5 Island filter

1. Clear flag 0x8000 on every polygon of every tile.
2. Create a `dtNavMeshQuery` (max nodes 2048). Query filter: include flags `0xffff`, exclude flags `0x8002` (disabled +
   isolated). `findNearestPoly(seed, halfExtents = (30, 30, 3), filter)` — extents are `(x, y, z)`; with
   `--all-heights` (only the `islands` command) the y extent is 10000.
3. If no polygon is found: seed not found (generate: skip, `islands`: error "no polygon near the seed (try
   --all-heights)").
4. Depth-first flood from that polygon over **every** link of each polygon (ignoring flags, so closed doors are
   traversed, and following one-way off-mesh links in their link direction only), visiting each polygon once.
5. Every visited polygon counts `reachable`; every other polygon (in every tile, including off-mesh polygons) gets flag
   0x8000 and counts `isolated`.

`findNearestPoly` is stock Detour using the DayZ `closestPointOnPoly` (section 10), polygon candidates from the tile BV
trees via stock `queryPolygons`, and Detour's usual "directly over a polygon within climb height" preference.

---

## 9. `.nm` output file format

All little-endian. Layout is RecastDemo's "tile mesh set" extended by BI. Verified by round-tripping shipped Chernarus,
Livonia and Sakhal navmeshes byte for byte (game versions 1.27 to 1.30) and by re-linking shipped tiles with zeroed links
to identical bytes.

### 9.1 File

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic: bytes `54 45 53 4D` = ASCII `TESM` (the u32 constant `'M'<<24 \| 'S'<<16 \| 'E'<<8 \| 'T'`) |
| 4 | 4 | set version = **9** (DayZ 1.30; the game accepts only its own version). Version 7 (DayZ <= 1.29) has an identical layout and is accepted when reading; always write 9 |
| 8 | 4 | numTiles (i32) |
| 12 | 4 | tilesX (grid width in tiles, i32) |
| 16 | 4 | tilesZ (i32) |
| 20 | 12 | `dtNavMeshParams.orig[3]` (f32, 0,0,0) |
| 32 | 4 | tileWidth f32 (50.0) |
| 36 | 4 | tileHeight f32 (50.0) |
| 40 | 4 | maxTiles i32 (= tilesX * tilesZ for generated files; = numTiles in shipped maps) |
| 44 | 4 | maxPolys i32 (`1 << 20` = 1048576) |
| 48 | 4 * maxTiles | tile offset table, indexed by **tile slot** (`(tileRef >> 20) & 0x0FFFFFFF`); entry = byte offset of the tile record **relative to the end of the table**, i.e. relative to the start of the first record; `0xFFFFFFFF` for empty slots |
| 48 + 4*maxTiles | ... | `numTiles` tile records, back to back, in tile-slot order (ascending) |

The game skips the table (uses `maxTiles` to find the end) and reads `numTiles` records sequentially; it aborts the load
if any record has `tileRef == 0` or `dataSize == 0`. Shipped maps contain a tile in every grid cell including open sea.
A partial file (a generated range) has `numTiles < maxTiles` with `0xFFFFFFFF` entries. Writers must reject more tiles
than `maxTiles`, a tile ref whose slot is `>= maxTiles`, and files whose offsets exceed `0xFFFFFFFE`.

A tile record's offset in the table is the running sum of `16 + dataSize` of preceding records in file order.
Records are in slot order, which is the order in which tiles were added to the Detour mesh (so unrelated to grid
position; each tile carries its own `(x, y)`).

### 9.2 Tile record

| Size | Field |
|---|---|
| 8 | tileRef (u64) = `salt << 48 \| slotIndex << 20` (salt is 1 for a freshly built mesh, higher where a tile was replaced) |
| 4 | dataSize (i32) |
| 4 | padding: 0 in shipped maps, other values seen in some saves; ignored by the game. Preserve on rewrite; write 0 for new tiles |
| dataSize | Detour tile blob |

### 9.3 Tile blob (Detour tile version 12, `DT_POLYREF64`)

This is **standard Detour tile data** (stock `dtCreateNavMeshData` layout) with three DayZ differences: tile
version 12; a 4-byte header extension; 44-byte off-mesh connections; a trailing extra section. A reimplementation can
produce it with its own code. Sections follow each other with no padding (everything is 4-aligned):

| Section | Element | Count |
|---|---|---|
| `dtMeshHeader` | 104 bytes | 1 |
| vertices (3 x f32) | 12 | `vertCount` |
| `dtPoly` | 32 | `polyCount` |
| `dtLink` | 16 | `maxLinkCount` |
| `dtPolyDetail` | 12 | `detailMeshCount` (always 0) |
| detail vertices | 12 | `detailVertCount` (0) |
| detail triangles | 4 | `detailTriCount` (0) |
| `dtBVNode` | 16 | `bvNodeCount` |
| `dtOffMeshConnection` | 44 | `offMeshConCount` |
| extra section | `extraDataSize` bytes | 1 |

Coordinates are absolute world coordinates (x east, y up, z north); the header's `y` is the grid row along world Z
(`z = orig.z + y * tileHeight`). Polygon/vertex indices fit 16 bits.

**`dtMeshHeader` (104 bytes):**

| Offset | Type | Field |
|---|---|---|
| 0 | i32 | magic `'DNAV'` (`'D'<<24 \| 'N'<<16 \| 'A'<<8 \| 'V'`; stored bytes `56 41 4E 44` = "VAND") |
| 4 | i32 | version = 12 |
| 8 | i32 | x (grid x) |
| 12 | i32 | y (grid z row) |
| 16 | i32 | layer (0) |
| 20 | u32 | userId (0) |
| 24 | i32 | polyCount (ground polys + stored off-mesh polys) |
| 28 | i32 | vertCount (ground verts + 2 * stored off-mesh) |
| 32 | i32 | maxLinkCount |
| 36 | i32 | detailMeshCount (0) |
| 40 | i32 | detailVertCount (0) |
| 44 | i32 | detailTriCount (0) |
| 48 | i32 | bvNodeCount (= 2 * ground polys) |
| 52 | i32 | offMeshConCount |
| 56 | i32 | offMeshBase (= ground poly count) |
| 60 | f32 | walkableHeight (metres, = stand height) |
| 64 | f32 | walkableRadius (= diameter * 0.5) |
| 68 | f32 | walkableClimb (= max step height) |
| 72 | f32[3] | bmin |
| 84 | f32[3] | bmax |
| 96 | f32 | bvQuantFactor (= 1 / cell size: 4.0 or 10.0 on shipped maps) |
| 100 | i32 | extraDataSize |

**`dtPoly` (32 bytes):** `u32 firstLink`, `u16 verts[6]`, `u16 neis[6]`, `u16 flags`, `u8 vertCount`, `u8 areaAndType`
(low 6 bits area, high 2 bits type: 0 ground, 1 off-mesh). Neighbour encoding `neis[j]`: `0` = solid edge, `1 + polyIndex`
= internal neighbour, `0x8000 | dir` = portal to a neighbouring tile with `dir` 0 (+x), 2 (+z), 4 (-x), 6 (-z) per the Detour
mapping of the Recast portal codes (portal code x- becomes 4, z+ becomes 2, x+ becomes 0, z- becomes 6).

**`dtLink` (16 bytes):** `u64 ref`, `u32 next`, `u8 edge`, `u8 side`, `u8 bmin`, `u8 bmax`. Content comes from `addTile`
(files are saved after linking), so `firstLink` in polygons and the link array are populated.

**`dtBVNode` (16 bytes):** `u16 bmin[3]`, `u16 bmax[3]`, `i32 i` (>= 0 leaf poly index; < 0 escape offset).

**`dtOffMeshConnection` (44 bytes):**

| Offset | Size | Field |
|---|---|---|
| 0 | 24 | `pos[6]` start and end point (after `addTile`, snapped onto the mesh polygons they connect to) |
| 24 | 4 | f32 centre height: the measured obstacle height at the midpoint relative to the start; >= class `minCenterHeight` (negative classes: <=); not read by Detour |
| 28 | 4 | f32 reserved: 0.0 from the builder; `addTile` sets 1.0 |
| 32 | 4 | f32 `rad` snap radius used by `addTile` |
| 36 | 2 | u16 `poly`: index of the off-mesh polygon in this tile |
| 38 | 1 | u8 flags (1 = bidirectional) |
| 39 | 1 | u8 side: 0xFF when both ends are in this tile, else the neighbour direction of the end point |
| 40 | 4 | u32 userId (index of the link class) |

The horizontal length of every shipped link equals the class's `jumpLength`.

**Extra section:** exactly as in 7.2 (starts with the two `u32` counts). The reader verifies the parsed size equals
`extraDataSize` and that no bytes trail the blob.

### 9.4 Reading rules (for compatibility tests)

Reject: bad magic "Invalid header format (bad magic)", set version not 7 or 9 "Invalid version N", `numTiles >
maxTiles`, a record with `tileRef == 0` or `dataSize <= 0`, a record running past the end of the file, blob magic not
`'DNAV'`, tile version not 12, any negative counts, extra size mismatch, trailing bytes.

### 9.5 Area ids and flags of the output

| Id | Name | Default poly flags |
|---|---|---|
| 1 | terrain | walk |
| 2 | water | walk, swim |
| 3 | waterdeep | swim |
| 4 | watersea | walk, swim |
| 5 | waterdeepsea | swimsea |
| 8 | building | walk, inside |
| 12 | dooropened | walk, door |
| 13 | doorclosed | walk, door |
| 15 | ladder | none (link flag 0x40 is assigned) |
| 16 | crawl | crawl |
| 17 | crouch | crouch |
| 48 + N | jumpN (`jump` = jump0) | from the link class `flags` |

### 9.6 Writing

Write to `<path>.tmp`, delete the old file, rename the temporary over it (so an interrupted checkpoint never destroys the
previous save). Header counts: `numTiles` = tiles exported; `tilesX/Z`, `tileWidth/Height`, `orig`, `maxTiles`,
`maxPolys` from the mesh parameters.

---

## 10. Recast/Detour version and DayZ modifications

Upstream: https://github.com/recastnavigation/recastnavigation. The vendored copy is documented inconsistently:
`UPSTREAM.txt` records `9f4ce64458dfae86e1239c525ddc219c4e9e06f1`, while `docs/development.md` says "Recast is at upstream
`6d1f971`, the revision DayZ's matches; Detour is newer". The code is a post-1.6.0 line (span `smin/smax` are 13-bit
fields; `rcFilterLowHangingWalkableObstacles`/`rcFilterLedgeSpans` naming; the clipped-polygon `dividePoly` rasteriser
with `-1` row/column clamp; `dtNavMesh::connectExtOffMeshLinks`; `findNearestPolyInTile`). I could not diff against
upstream from here, so the list below is what the DayZ-marked changes are; everything not listed is stock for that
vintage. The reimplementer should use a Recast/Detour of that vintage as the base and verify against the test vectors
in section 13 and any reference navmesh they have. Both libraries are built with `DT_POLYREF64`; Recast asserts are
off; allocation uses mimalloc (a performance detail only).

### 10.1 Recast changes (all marked `DAYZ:` in the source)

1. **`rcSpan.subArea` (u16)** per span; `rcHeightfield.hints` (speed only: per-column "span below the last one added" to
   resume `addSpan`; results are identical to scanning from the column start); `rcCompactHeightfield.subAreas` (u16 per
   span, filled from the solid span in `rcBuildCompactHeightfield`); `rcContour.subArea`; `rcPolyMesh.subAreas` (u16 per
   polygon, zero-initialised; stored from the contour for every polygon the contour produces).
2. **`addSpan`:** new span `subArea = 0`; merge rules as stock.
3. **`rcFilterLowHangingWalkableObstacles`:** copies `subArea` along with the area (walkable span below to the promoted
   span above), and the "previous" state keeps `subArea` too.
4. **`rcFilterLedgeSpans`:** extra optional `skipAreas[64]` argument (when non-null, spans whose area has a non-zero entry
   are skipped instead of only `RC_NULL_AREA`). A neighbour-span cursor optimisation does not change results.
5. **`calculateDistanceField`:** a span is a boundary (distance 0) unless all four neighbours are connected **and** have the
   same area **and** the same `subArea`.
6. **Watershed (`rcBuildRegions`) and monotone (`rcBuildRegionsMonotone`):** `floodRegion`, `expandRegions`, and the monotone
   sweep compare **both area and subArea** wherever stock compares area; `rcRegion` carries `subArea` (taken from the
   region's first span) and `canMergeWithRegion` also requires equal `subArea`. The minimum-region-area parameter is an
   array `minRegionAreaByArea[64]`; in `mergeAndFilterRegions` the "too small" test uses the entry of the area of the **first
   region of the connected group** (lowest region number visited first). Stock constants otherwise: `expandIters = 8`,
   final expansion `expandIters * 8` iterations, `sortCellsByLevel` with 8 level stacks, the "do not remove regions
   connecting to a tile border" rule, merge threshold `mergeRegionSize`.
7. **`rcBuildContours` / `walkContour` / `getCornerHeight`:** the edge `RC_AREA_BORDER` test compares area **and** `subArea`;
   `getCornerHeight`'s "two same interior cells" test also requires the same `subArea`; each contour stores `subArea`.
   `simplifyContour` takes a new `maxVertError` (voxels, `0` = off): after the horizontal-deviation pass computes the
   largest squared horizontal deviation `maxd` (stock), it also tracks the largest squared **vertical** deviation `maxdy` of
   raw points from the segment's linearly interpolated height (where `t` is the same clamped projection parameter); if the
   horizontal test does not trigger (not `maxi != -1 && maxd > maxError^2`) and `maxVertError > 0` and `maxdy >
   maxVertError^2`, the vertically worst raw point is inserted instead, and it is tagged
   `RC_BORDER_VERTEX | RC_AREA_BORDER` in the raw contour (these tags later feed the simplified vertices' flags through
   stock's final flag-copy loop). The "tessellate only outer edges or edges between areas" gate for this pass accepts raw
   points whose flag has `RC_AREA_BORDER` **or `RC_BORDER_VERTEX`**.
8. **`rcBuildPolyMesh`:** border vertices are never removed (the stock `vflags` marking is disabled, so the "remove edge
   vertices" loop is a no-op); `subAreas` copied from contours; otherwise stock (triangulation by ear clipping, merge of
   triangles into polygons up to nvp = 6, adjacency, portal-edge marking when `borderSize > 0`, flags array allocated
   zeroed).
9. **New file `RecastDayZ.cpp`:** `dzModeFilterWalkableAreaFlat`, `dzFilterLowHeightSpansMulti`, `dzDilateArea`,
   `dzExtractTreeObstacles` (all described in section 5), and the five-mode rasteriser
   (`dzRasterizeTriangle`, `dzRasterizePart`, described in 5.3).

### 10.2 Detour changes

* `DT_NAVMESH_VERSION = 12`; magic `'DNAV'`.
* `dtMeshHeader`: extra `int extraDataSize` at the end (104 bytes). `dtMeshTile.extraData` pointer (set to the byte after the
  off-mesh connections when `extraDataSize > 0`, else null).
* `dtOffMeshConnection`: 44 bytes: `centerHeight` and `dzReserved` floats inserted after `pos[6]`.
* `dtNavMeshCreateParams`: optional `offMeshConCenterHeight[]`, `extraData`, `extraDataSize`. Without an input detail mesh no
  dummy detail mesh is produced (`detailMeshCount = 0`, `detailMeshesSize = 0`), `extraSize = dtAlign4(extraDataSize)`.
  `dzReserved` is written 0; `centerHeight` is copied per stored connection; connection `flags` is `1` iff the direction value
  is non-zero.
* `dtNavMesh::addTile`: pointer patching includes the extra section; with `detailMeshCount == 0` the detail pointers are
  null; after all linking, obstacle Y snapping (8.3).
* `connectExtLinks`: portal limits `link->bmin/bmax` are `(unsigned char)(clamp(t,0,1) * 255.0f)`, i.e. **truncated** (stock
  rounds with `roundf`).
* `baseOffMeshLinks`: sets `dzReserved = 1.0` for every connection (before the search); the back-link from the landing polygon
  to the off-mesh polygon is only allocated if the forward link was allocated.
* `closestPointOnPoly` and `getPolyHeight`: the pre-2016 behaviour. For ground polygons with no detail mesh: clamp the point to
  the nearest polygon edge if it is outside (`dtDistancePtPolyEdgesSqr`), then compute the height at the **original** position
  from the first triangle of the polygon's fan `(v0, v[j+1], v[j+2])` containing it, using the old barycentric test: with
  `v0 = c - a`, `v1 = b - a`, `v2 = p - a` in xz, `dot00..dot12`, `invDenom = 1/(dot00*dot11 - dot01*dot01)`,
  `u = (dot11*dot02 - dot01*dot12) * invDenom`, `v = (dot00*dot12 - dot01*dot02) * invDenom`; inside if `u >= -1e-3 && v >= -1e-3 &&
  u + v <= 1.001`; height `(v0.y*u + a.y) + v1.y*v`. If no triangle contains the point `closest` keeps the clamped `y`
  (`getPolyHeight`: falls back to the closest edge point's height). Off-mesh polygons: interpolate between the two endpoints
  by distance ratio. `findNearestPolyInTile` is stock (uses `closestPointOnPoly`; "directly over polygon" favours vertical
  distance beyond `walkableClimb`).

### 10.3 Non-Recast numerical caveats

* All Recast config values and the slope test are f32 as shown in sections 4-5. Constants `0.1f`, `3.1415927f`,
  `2.3561945f`, `0.8f`, `0.5f` are f32 literals.
* `RSQRTSS` approximate reciprocal square root in the slope test (hardware dependent).
* Jump-link sample positions depend on exact f32 association (see 6.4).

---

## 11. Probe output

`dznav probe [host] [x z]`: connect to `host` (default 127.0.0.1) port 36533, request world params (once), print them.
With `x z` (and a host argument present) also request that tile (with the normal retry rules and tile checks) and print
its summary. Exit 0; errors exit 1 as in 2.4.

Human-readable output (`{:.N}` = fixed N decimals, `{:#x}` = `0x`-prefixed lowercase hex, byte values print as integers):

```
world {worldSizeX:.1} x {worldSizeZ:.1} m, tiles {tilesX} x {tilesZ} of {tileWidth:.1} x {tileHeight:.1} m
deep sea height {deepSeaHeight:.2}, deep water depth {deepWaterDepth:.2}, contour error {contourMaxError:.3}, obstacles {obstacles}, block {blockSize}
filter islands {filterIsolatedIslands}, seed ({x:.1}, {y:.1}, {z:.1})
agent: stand {standHeight:.2} crouch {crouchHeight:.2} prone {proneHeight:.2} diameter {diameter:.2} step {maxStepHeight:.2} slope {maxSlope:.1}
cell class {i}: {cells} cells over {width:.2} m (cs {width/cells:.3}) [unk {unk04} {unk0C}]      (one per class)
link {index}: typeId {typeId} len {jumpLength:.2} height {jumpHeight:.2} centre {minCenterHeight:.2} dropdown {min:.2}..{max:.2} agent h {agentHeight:.2} step {maxStepHeight:.2} r {agentRadius:.2} flags {flags:#x} area {area}     (one per class)
```

and with a tile:

```
tile ({x},{z}) cell class {cellClass} bounds ({bminx:.2} {bminy:.2} {bminz:.2})-({bmaxx:.2} {bmaxy:.2} {bmaxz:.2}), {groupCount} groups
  area {area:2}: {partCount} parts, {vertexCount} verts              (one per group; area right-aligned to width 2)
```

JSON (`--json`): one event `{"event":"world","host":..,"params":{..},"tile":{..}}` as documented in `docs/cli-json.md`
(`params` has `worldSizeX/Z, tilesX/Z, tileWidth/Height, seaLevel (= deepSeaHeight), deepWaterDepth, contourMaxError, obstacles
(bool), blockSize, filterIsolatedIslands (bool), seed, agent{...}, cellClasses[{cells,width,cellSize}], linkClasses[...]`;
`tile` has `x, z, cellClass, bmin, bmax, groups[{area, parts, verts}]`).

---

## 12. Validation, limits and error cases

### 12.1 Checks on server input (wire errors; "protocol" error kind)

World params (all checked after the full payload is read):

* the message must be at least 4 bytes (else "world params truncated"); version must be 10;
* truncated payload: "message truncated"; bytes left over: "unexpected bytes after world params";
* cell-class count `<= 64` ("implausible cell class count N"), link-class count `<= 256` ("implausible link count N");
* `tilesX > 0`, `tilesZ > 0`, `tilesX * tilesZ <= 2^28` else "invalid tile grid";
* `tileWidth > 0` and `tileHeight > 0` (NaN fails) else "invalid tile size";
* every cell class: `cells > 0` and `width > 0` else "invalid cell class".

Tile record:

* `groupCount <= 65536`; each group: `partCount <= 1048576`; `vertexCount` in `[0, 2^26]` ("implausible vertex count");
* each part: `firstVertex <= endVertex <= vertexCount` where `endVertex` is the next part's `firstVertex` or the total
  ("part vertex range out of bounds"); a part whose `firstVertex` exceeds the next part's is rejected;
* every vertex coordinate must be finite ("non-finite vertex");
* truncated: "message truncated"; trailing bytes (when exactly one tile is expected): "unexpected bytes after tile record";
* tile coordinates equal the request; `cellClass < cellClassCount` (network-class errors, see 3.6).

No range checks exist on agent values, link class values, cell counts' upper bound, or vertex magnitude beyond
finiteness. Large cell counts allocate `(cells + 2*border)^2` columns per tile.

### 12.2 Limits inside the build

* Heights are clamped to 13-bit voxel indices (0..8191); at `ch = 0.1` that is 819.1 m above the tile's minimum y.
* A tile with `>= 0xffff` mesh vertices fails ("Too many vertices per tile N (max: 65535)"). Stock `rcBuildPolyMesh`
  fails above `0xfffe` contour vertices.
* Region ids are 16 bit (overflow error "Region ID overflow").
* `dtCreateNavMeshData` fails for an empty vertex or polygon list, `nvp > 6`, or `>= 0xffff` vertices.
* `addTile`: blob smaller than a header or larger than `INT32_MAX` is rejected ("invalid tile data size"); other statuses are
  reported as "addTile failed (status 0x...)".
* The cell-class index is checked again before the build ("tile X Z has unknown cell class N").
* Jump links: at most 64 candidate segments per refinement and 32 output segments per edge.

### 12.3 The stray part id case (crash fix)

Door part lookup uses the polygon's part id as an index into the extras list. The part-id field is shared by door parts and
tree obstacles, and the sea box (5.4 step 6.1) relabels tree-tagged spans below sea level as area 5 **without** clearing
the id. On a tile with such trees and at least one door part, an area-5 polygon may carry a tree index larger than the
extras list. A naive lookup reads out of bounds (the reference tool would crash). The generator ignores out-of-range ids
(polygon gets no part, flag 0x2 untouched) and counts them as `strayPartIds`; the CLI reports "N tiles had out-of-range
door part ids, which were ignored". The part's area is also compared (`extras[p].area == poly.area`) so an in-range
mismatch is also ignored.

### 12.4 Other error behaviour

* Missing or malformed `.nm`/`.dzgeo`: error naming the problem (exit 1).
* Output paths: written atomically (9.6); failing to rename: "cannot rename to <path>".
* A tile build failure does not stop the run; the run finishes, saves, and exits 1.
* The first fetch network failure in a fetcher stops that fetcher only.

---

## 13. Test vectors

The repository has no stored reference `.nm` or golden tile bytes (game data is not redistributed); validation is by
comparison against DayZ's own tool output and against the old C++ engine. What can be re-used:

### 13.1 Constants and sizes

* Poly ref: `encodePolyRef(salt 1, tile 5, poly 7)`: salt 1, tile 5, poly 7 round-trip; `encodePolyRef(1, 1, 0) =
  0x0001000000100000`.
* World-params body size: `0x54 + 4 + n_cells*16 + 4 + n_links*0x34`; with 2 cell classes and 1 link class = 176 bytes
  (excluding the 4-byte protocol version).
* Hand-made two-tile file (`tilesX 2, tilesZ 1, tileWidth = tileHeight = 50, maxTiles 2, maxPolys 1<<20`, each tile with 3
  vertices `(0,0,0) (50,0,0) (50,0,50)`, one triangle polygon with flags 1 and area byte 1, 3 zeroed links, 2 BV nodes, one
  part `{objectId 0x8000f0000501f61f, partIndex 0, firstPoly 0}`, one part-poly `{0, -1}`, one obstacle
  `(10, 0, 10, r 0.3)`; walkable height 1.5, radius 0.3, climb 0.45, `bvQuantFactor` 4.0, `offMeshBase` 1; refs
  `encodePolyRef(1, y*2 + x, 0)`): the file starts with bytes `TESM`; byte at `48 + 8 + 16` begins `VAND`; header
  `extraDataSize = 12 + 16 + 8 + 16 = 52`; parse then serialise reproduces the bytes.
* Offset-table test: with `maxTiles 4` and two tiles whose refs have slots 3 and 1 (in that file order), the table is
  `[0xFFFFFFFF, record_size, 0xFFFFFFFF, 0]` where `record_size = 16 + dataSize` of the first tile.
* Rejections with exact messages: see 9.4 and 12.1.

### 13.2 Synthetic world used by the Rust tests (shows realistic parameter magnitudes)

World: 3 x 3 tiles of 50 x 50 m (150 x 150 m); `deepSeaHeight 0.75`, `deepWaterDepth 1.5`, `contourMaxError 0.3`,
`obstacles 1`, `blockSize 4`, `filterIsolatedIslands 1`, `seedPosition (60, 10, 60)`, agent: stand 1.5, crouch 1.0, prone
0.5, diameter 0.6, max step 0.45, max slope 45 degrees. Cell classes: `{cells 200, 200, width 50, 0}` (cs 0.25) and `{500,
500, 50, 0}` (cs 0.1). One link class: `index 0, nameHash 0x12345678, typeId -1, jumpHeight 1.2, jumpLength 1.8,
dropdownMin 0, dropdownMax 0, minCenterHeight 0.4, agentHeight 1.5, maxStep 0.45, radius 0.3, flags 0x100, area 48,
color -1`. The tile at (1,1) uses cell class 1, all others class 0. Tile input geometry per tile: terrain quads on a 2 m
grid of "rolling ground at about 10 m" (`10 + 1.5*sin(0.05x)*cos(0.07z)` minus a 3 m pond hollow within 12 m of (30, 105)),
extending 10 m beyond the tile, as group 1; a water quad (area 2) around (30, 105) at y 9.6, half-width 9.5; area-9 boxes
(a crate (64..70, 8..12.5, 64..70) and a low wall (55..65, 8..10.9, 80..80.3)); area-8 walls forming a 10 x 8 room at
(80..90, 60..68) with a doorway 84.5..85.5 in the south wall and a floor quad at y 10.6; door parts: area 12 part
`{partIndex 0, objectId 0x8000f0000501f61f}` (open: a thin box at the hinge side) and area 13 part `{partIndex 1,
same id}` (closed: a box filling the doorway); five tree parts in area 10 (hexagonal prisms of radius 0.35, 6 m tall, at
(20,20), (40,70), (110,30), (120,120), (75,130), part indices 0, object ids 0x100+i); a ladder (area 15) along the crate's
west face as one vertex pair from the ground to y 12.5 at (63.8, ., 67). Expected qualitative result: every tile builds;
the world has off-mesh links, obstacles, door-part polygons, and the island filter finds the seed. (The original compares
its two engines on this world byte for byte.) Derived config for this world: class 0 gives walkableRadius 2, border 14,
heightfield 228 x 228, minRegionArea 16, maxSimplificationError 1.2 cells, door radius 1; class 1 gives radius 3, border 33,
566 x 566, minRegionArea 100, maxSimplificationError 3.0, door radius 2. Both: walkableHeight 15, crouch 8, prone 3, climb 5,
deep water 15 voxels, sea level 8 voxels (box top 0.8 m), slope threshold cos(45 degrees) = 0.70710677, jump depth
`min(3, max(0.6*3, 1.8)) = 1.8`, trajectory `total = 2.4`.
Capture of this world: `capture-info` prints "9 tiles".

### 13.3 Reported validation (original's documents, October 2026)

* 52 tiles rebuilt from replayed captures (all that had differed from the reference in every category) and a 49-tile fixture
  (castle, doors, fences, ponds, shoreline) are byte-identical to the reference output of DayZ's tool, ignoring only the door
  "disabled" bit (random in the references), link data that depends on neighbours, obstacle Y, the reserved off-mesh float,
  snapped off-mesh endpoints, and (for partial saves) the isolated flag.
* Against reference saves of rows 0-79 (26,160 tiles, sea level pinned to each session's tide): rows 40-79 13,069 of 13,080
  byte-identical (all 13,080 with `--ignore-order`); rows 0-39 13,040 of 13,080 (13,046 with `--ignore-order`); the remaining
  34 tiles are around animated objects and differ from capture to capture.
* DayZ 1.30: a 49-tile range from the 1.30 tools, same DayZDiag session, sea level pinned to 0.60, byte-identical including
  object order and every jump link. The island filter reproduces shipped Chernarus byte for byte.
* Two bugs found this way worth testing: (1) the jump-link sample far end computed as `dir*t + (n*h + A)` (1 % of tiles);
  (2) door parts at the heightfield min edge (tile 108,76 of the validation set: 0 links expected vs 73 with the wrong
  clipping).

### 13.4 Rebuild tests

The original also rebuilds shipped tiles from their reconstructed poly meshes (`dtCreateNavMeshData` with reconstructed
inputs, off-mesh connections including those arriving from neighbours, extra section) and links all tiles, requiring
identical bytes, and re-adds shipped tiles with zeroed links requiring identical bytes (obstacle Y may differ because
re-snapping is not idempotent). A reimplementation with access to a shipped `.nm` (`DZ\worlds\<world>\navmesh\navmesh.nm`)
can run the same two experiments: they validate section 9 and Detour linking independently of the geometry pipeline. The
heights in shipped tiles are on the `ch = 0.1` grid; vertices lie on the `cs`/`ch` grid, with `cs` = `1 / bvQuantFactor`.

---

## 14. Open questions

1. **Exact Recast/Detour revision.** `UPSTREAM.txt` (`9f4ce64...`) and `development.md` ("Recast `6d1f971`, Detour newer") disagree,
   and I could not diff against upstream. Stock behaviour of watershed/monotone region building, distance-field blur,
   erosion, ear-clipping triangulation, polygon merging and the BV tree is assumed to be that of a ~1.6.x-era upstream. Bit
   exactness depends on it.
2. **`RSQRTSS` hardware differences** (Intel vs AMD) in the slope test make exact slope classification CPU dependent; the
   original accepts that.
3. **BV tree tie order. Answered (#114).** `createBVTree` sorts with the C library `qsort`, which is not stable, and the
   shipped meshes show MSVC's order for equal keys: Livonia tile 128,128 has four polygons that all have `bmin.x = 0`, and
   its nodes are the root, then `[2, 1]`, then `[0, 3]`; a stable sort gives `[0, 1]`, `[2, 3]`. `msvc_qsort` in
   `detour/create.rs` has that observable order: a selection sort for ranges of up to 8 items, else a median-of-three
   partition. Built from live captures, the inner tiles of all five validation sets then match the shipped BV trees.
4. **Cell class fields 4 and 12** (observed `u32` equal to `cells`, and `u32` 0). The doc names them `cellsZ` and `height`; the
   generator ignores both and uses only `cells` and `width`.
5. **Door state** is not sent by the server (references have random "disabled" bits for about 30 % of door parts), so the
   baked state is a policy; the game updates it at runtime.
6. **Ladders:** a comment says "one-way ladder links, unused in shipped maps" but the direction value passed is `1`
   (meaning bidirectional in the connection arrays). Which is right in DayZ is unknown. Group 15 as input is documented as
   observed.
7. **Meaning of input areas 6, 7, 9, 11, 14, 18 and 16/17 sources** is only inferred from the code: 7, 9 map to terrain (1) and
   11 to building (8) after erosion; 14 "hide parts" are dilated by the agent radius and become polygons of area 14 with flag
   0x01; 18 "WallWithHole" is a ceiling for crawl/crouch clearance; crawl (16) also grows by dilation.
8. **Reverse-merge claim vs code:** docs say each span merges at most once; the code does not enforce it exactly (validity 2
   spans are matched again). Implementation above follows the code.
9. **Server error semantics:** no error message id is known; behaviour of the server for out-of-range tiles, repeated world
   requests, or a second client is not documented in the repo beyond "serves one client at a time".
10. **Protocol version.** Verified live on DayZ 1.28 (version 10); the docs and changelog assume the same for 1.30; no 1.30
    protocol change is recorded.
11. **Slot order / determinism.** With more than one builder the tile slot order (and so link allocation order) is the
    completion order; the shipped maps' slot order (probably DayZ's own scheduling order) cannot be reproduced exactly.
12. **Stock algorithm details not restated.** This document names but does not restate stock Recast/Detour algorithms
    (watershed region building, contour tracing, hole merging, ear-clipping triangulation, `addTile` neighbour
    linking, `findConnectingPolys` slab overlap with tolerance 0.01 and `walkableClimb`, BV tree construction, off-mesh
    snapping). Those are in upstream's zlib source and are expected to be reproduced from it.
13. I did not build or run anything from the checkout; all statements come from reading the files, so behaviours that depend on
    run-time interplay (for example how DayZDiag reacts to concurrent requests) are unverified.
14. **135° facing test precision.** Whether the 135° facing test of `refineEdge` (§6.3 step 4) compares in `f32` (`cosf`)
    or in double; exact 45° grid diagonals land on the threshold. dayzmake compares in `f32` (ruling R12).
15. **Pass direction `dir` (ruling R13, #116). Superseded by R16 (14.18): the one ulp error of `(n*(-h) + B) - S0` that R13 saw came from a `n` normalised by division, and `B - A` is not the rule.** The spec text above gave `dir = (n*(-h) + B) - S0`. When `n.z*(-h)` added to
    the coordinate is a tie between two floats, the sums `n*(-h) + A` and `n*(-h) + B` round in opposite directions and `dir` is one ulp off, which
    moved both ends of one link by one ulp (Badlands tile 138,102, link 46). The same happens when A and B straddle a
    power of two, where their ulps differ. The shipped value is the single f32 subtraction `B - A`; `FMA`
    gives the same value in the observed case but is not needed. dayzmake uses `dir = B - A`; with it all five validation
    sets match the shipped meshes byte for byte (inner tiles), apart from the polygon flags and the link array.
16. **Shallow water depth (ruling R14, #117).** The first water pass (area 2) uses `waterDepth = walkableClimb`, not 0. With 0
    the shallow band reaches ground that is only a voxel under the surface, and on a sloped shore it is 1 m to 2 m too wide
    (Livonia tiles 130,220, 131,220, 131,221 and 131,222: the shipped shallow-water polygons end where the ground is 5 voxels
    below the water piece's lower limit, and with `walkableClimb` the polygons, vertices and off-mesh connections of all four
    tiles are identical to the shipped ones; the values 0 to 4 and 6 do not match). Every shipped world has 5 voxels both
    for `walkableClimb` (0.45 m) and for `ceil(proneHeight / 0.1)` (0.5 m), so the data cannot say which one DayZ means;
    dayzmake uses `walkableClimb`, the step height a walker can wade up. A world with other agent values would settle it.
17. **Slab test by reciprocal (ruling R15, #117).** The segment-box clip of `refineEdge` (§6.3 step 3) multiplies by `1/e`, as
    upstream's `isectSegAABB` does (RecastDemo `InputGeom.cpp`; demo code, not the Recast library, so it corroborates the
    ruling and the tile data proves it), instead of dividing. The two differ by one ulp for some operands (`3/7` and
    `3*(1/7)`). In Livonia tile 130,223 one clip parameter is one ulp lower, which moves a midline end point across an
    exact rounding tie of `p0 + (p1 - p0)*t` at x of about 6510, and so both end points of two links by 1 mm. With the
    reciprocal all five validation sets match the shipped meshes completely (inner tiles); a lerp `p0*(1-t) + p1*t`
    for that point fixes the tile but breaks every set, and neither FMA nor a reciprocal in `normalize` changes it.
18. **Normalisation by the reciprocal (ruling R16, #120, #121).** Every `normalize` of the link code (`refine_edge` frame and
    facing test, pass direction and normal) computes `v * (1.0f / |v|)`, not `v / |v|`. The two differ in the last bit for
    some `v` (`(-4.06, 0, -0.67)`: x is -0.98665547 by division and -0.9866554 by the reciprocal). With the reciprocal,
    the literal spec formula `dir = (n*(-h) + B) - S0` matches the shipped meshes, which settles 14.15: `B - A` agreed
    with the shipped value in the tile that gave R13 only because the division gave a slightly wrong `n`. Consequences:
    * the two passes do not share `dir` (`n` has the opposite sign, the sums round differently), so the two spans of a
      pair are not exact mirrors, the reverse merge (6.4) compares different lengths, and the exchanged end points and
      the radius differences of #120 disappear;
    * the edge end points that `cy` and the span end points come from carry the right low bits, which removes the
      one ulp `center_height` differences of #121 (they came from the end point y of the midline, which `pos` hides
      because `snap_to_solid` replaces it).
    Evidence: with the reciprocal and the literal `dir` all inner tiles of an 11 x 11 block of Badlands tiles that
    had 16 differing links match, the ten tiles with the `center_height` differences match, the five validation sets
    of 14.15 and the 14.17 tile match, and so do the tiles the previous attempt (`dir` by fused multiply-add, which
    removed most but not all differences and broke two tiles) left. Rejected on the way: `B - A`, FMA of the sums
    (also with `S0` unfused: it breaks a tile of a regression set), double precision intermediates, tie rule `<`
    in the reverse merge (breaks most tiles), FMA or other association in the span, midline or centre height
    formulas. R15 (the reciprocal in the slab test) is the same habit.
