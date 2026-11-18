# Navmesh live-validation fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make tiles built from live DayZDiag data byte-identical to the meshes shipped with DayZ, and correct the DayZDiag recipe in the docs, before PR #113 is merged.

**Architecture:** The fixes go into the existing `navmesh` module of `dayzmake-core` (branch `feat/navmesh`). An ignored test compares a mesh built from a capture with a shipped mesh. It is the acceptance check for tasks 1–3 and runs locally against game data.

**Tech Stack:** Rust 1.99, edition 2024. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-17-navmesh-cleanroom-spec.md`. This plan resolves §14 item 3 (BV tree tie order). Read it together with the plan `docs/superpowers/plans/2026-11-17-navmesh.md`.

## Global Constraints

- AGENTS.md applies in full:
  - tests first (red, then green);
  - no `unsafe`;
  - no panics on untrusted input;
  - docs updated with each behaviour change;
  - one logical change per commit, with `make check` passing before the task is done.
- Clean room: never open Flare Navgen (`/tmp/flare-navgen.*`, its GitHub). References allowed:
  - the clean-room spec;
  - upstream recastnavigation (zlib) at `/tmp/recastnav.rfH3dD/src`;
  - the data listed below.
- Game data never goes into the repo. The captures and shipped meshes are read only through environment variables in `#[ignore]` tests.
- Temporary files only under `/tmp` or `/var/tmp`. Run long or memory-heavy test runs under `prlimit --as=24000000000`.
- Total coverage must not drop. PR #113 is at 95.79 % and its base at 95.80 %, and Codecov's `project` check fails on any drop. New code comes with tests that cover it.
- Commit messages end with the attribution lines from the session (Co-Authored-By, Claude-Session).

## Validation data (local, read-only)

| What | Path |
|---|---|
| Livonia shipped mesh | `/var/tmp/dzshipnav.wV1HN5/enoch/navmesh.nm` |
| Badlands shipped mesh | `/var/tmp/dzshipnav-nasdara.KWqkww/nav/navmesh.nm` |
| Livonia, open terrain, tiles 128–131 × 128–131 | `/var/tmp/dznavlive2.YfAmoq/r128.dzgeo` |
| Livonia, dense town, tiles 128–131 × 220–223 (no margin) | `/var/tmp/dznavlive2.YfAmoq/dense.dzgeo` |
| Badlands, dense, 135–140 × 99–104 (inner 136–139 × 100–103) | `/var/tmp/dznavbad.AX9NDF/d1m.dzgeo` |
| Badlands, irrigation tunnel, 179–184 × 185–190 (inner 180–183 × 186–189) | `/var/tmp/dznavbad.AX9NDF/t1m.dzgeo` |
| Badlands, irrigation tunnel, 182–187 × 197–202 (inner 183–186 × 198–201) | `/var/tmp/dznavbad.AX9NDF/t2m.dzgeo` |

Build from a capture without the island filter: the seed is outside these ranges, so the polygon flags are not
comparable and are ignored. A link's far end that lies in a tile outside the range is snapped only when that
neighbour is loaded, so only **inner** tiles (one tile from the range edge) can match completely.

Status before this plan:
- every inner Badlands tile matches except its BV tree (#114) and one off-mesh link in 138,102 (#116);
- Livonia 128–131 × 128–131 matches except the BV tree;
- Livonia tiles 130,220, 131,220, 131,221 and 131,222 differ in structure (#117).

## Review Focus

- BV trees of tiles with more than 8 polygons. These use the partition path of the sort, not the short sort, and are the most common case in towns.
- A sort comparator that returns "equal" for every pair (all bmin equal on the axis). The tree must still be complete and valid, and must not loop.
- Build determinism: the same capture built twice, with 1 and with 8 threads, gives byte-identical output.
- Ignored tests with an unset environment variable must fail with a clear message naming the variable, and must not panic elsewhere.

---

### Task 1: BV tree tie order like MSVC `qsort` (#114) and the live comparison test

**Files:**
- Modify: `crates/dayzmake-core/src/navmesh/detour/create.rs` (`subdivide`, new private `msvc_qsort`)
- Test: `crates/dayzmake-core/tests/navmesh_create.rs` (unit-level behaviour through `create_nav_mesh_data`, or a `#[cfg(test)]` module for the sort)
- Create: `crates/dayzmake-core/tests/navmesh_live.rs` (ignored test)
- Modify: the spec, §14 item 3: answered, with the observation below

**Observation:** Livonia tile 128,128 has 4 polygons, all with `bmin.x = 0`. The axis is x, since x and z tie and the tie goes to x. Shipped node order: root, then `[2, 1]`, then `[0, 3]`. A stable sort gives `[0, 1]`, `[2, 3]`.

**Sort to implement:** `msvc_qsort(items: &mut [BvItem], cmp: impl Fn(&BvItem, &BvItem) -> Ordering)`. It is an in-place sort with the observable element order of the MSVC C runtime's `qsort`. Detour's comparator returns less, greater or equal on `bmin[axis]` only, with no tie-break.
- `n < 2`: nothing.
- Work on index ranges `[lo, hi]`. The order in which ranges are processed does not change the result.
- Range size ≤ 8, the short sort: while `hi > lo`, set `max = lo`; for `p` in `lo+1..=hi`, if `cmp(p, max) == Greater` then `max = p`; then `swap(max, hi)` and `hi -= 1`.
- Range size > 8:
  - `mid = lo + size / 2`.
  - Median of three: if `cmp(lo, mid) == Greater` swap them; if `cmp(lo, hi) == Greater` swap them; if `cmp(mid, hi) == Greater` swap them.
  - `loguy = lo`, `higuy = hi`. Loop:
    - if `mid > loguy`: advance `loguy` (`loguy += 1` first) while `loguy < mid` and `cmp(loguy, mid) != Greater`;
    - if `mid <= loguy`: advance `loguy` while `loguy <= hi` and `cmp(loguy, mid) != Greater`;
    - move `higuy` down (`higuy -= 1` first) while `higuy > mid` and `cmp(higuy, mid) == Greater`;
    - if `higuy < loguy`, break;
    - `swap(loguy, higuy)`; if `mid == higuy`, then `mid = loguy`.
  - Then `higuy += 1`;
    - if `mid < higuy`: move `higuy` down (`higuy -= 1` first) while `higuy > mid` and `cmp(higuy, mid) == Equal`;
    - if `mid >= higuy`: move `higuy` down while `higuy > lo` and `cmp(higuy, mid) == Equal`.
  - Continue with the sub-ranges `[lo, higuy]` (if `lo < higuy`) and `[loguy, hi]` (if `loguy < hi`).

  Every "advance while" and "move down while" is a do-while: step first, then test. Use checked or saturating index arithmetic and no recursion on unbounded depth (an explicit stack). `mid` is the index of the pivot element and follows it when it is swapped.

The validation data is the authority. If the shipped trees disagree with this description, the data wins; record what differed in the report.

- [ ] **Step 1: Write the failing tests.**
  - `bv_tree_tie_order_matches_shipped_small`: 4 polygons whose boxes all have `bmin = 0` on the split axis give the leaf order `2, 1, 0, 3`, with the node array `[-7, -3, 2, 1, -3, 0, 3]`. Escape indices: root −7 (7 nodes); inner nodes −3.
  - `msvc_qsort_matches_reference_orders`: hand-checked cases for 1, 2, 3, 8, 9 and 20 elements with many equal keys. For the ≤ 8 cases, derive the expected orders by hand from the short-sort rule above.
  - `msvc_qsort_all_equal_terminates`: 1,000 equal keys finish and give a permutation.
- [ ] **Step 2:** Run them and see them fail.
- [ ] **Step 3:** Implement `msvc_qsort` and use it in `subdivide`, in place of the stable `sort_by`. Remove the "stable sort, ruling 7" comment and say what the order is now.
- [ ] **Step 4: Live comparison test** `crates/dayzmake-core/tests/navmesh_live.rs`, `#[ignore = "needs DayZ navmesh data"]`, `fn capture_matches_shipped_inner_tiles()`.
  - Environment: `DAYZ_NAVMESH` (shipped `.nm`), `DAYZ_NAVMESH_CAPTURE` (`.dzgeo`) and `DAYZ_NAVMESH_INNER` (`x0,z0,x1,z1`).
  - Build the capture through the public generate API, with the island filter off and 1 thread.
  - For each inner tile, compare with the shipped tile of the same x,y:
    - the header, with `max_link_count` ignored;
    - vertices, by f32 bits;
    - polygons, ignoring `first_link` and `flags`;
    - detail sections, BV tree, off-mesh connections and the extra section.
  - On a mismatch, report tile, section, index, ours and shipped, as the relink test in `navmesh_mesh.rs` does.
  - Also check the build is deterministic: run again with 8 threads and require byte-identical output.
- [ ] **Step 5:** Run the live test on all five validation sets.
  - The Badlands inner ranges must now match everywhere, except Badlands 138,102 off-mesh connection 46 (#116; list it as expected-failing in the report, not in the test).
  - Livonia r128 must match on all 16 tiles; use `DAYZ_NAVMESH_INNER=128,128,131,131`, since that range has no links.
  - Report the results per set.
- [ ] **Step 6:** `make check`, then commit (`fix(navmesh): BV tree tie order like MSVC qsort (#114)`).

### Task 2: Jump-link endpoint rounding (#116)

**Files:** `crates/dayzmake-core/src/navmesh/links.rs` (spec §6), plus tests in `crates/dayzmake-core/tests/navmesh_links.rs`.

**Data:** Badlands tile 138,102, off-mesh connection 46 (area 48, user_id 2). Ours: `pos [6915.898, 191.6772, 5117.1255, 6914.0317, 191.3772, 5113.7007]`. Shipped: `[6915.898, 191.6772, 5117.125, 6914.0317, 191.3772, 5113.7]`. The z values of both ends differ by a few ULP, and the link polygon's vertices differ the same way. Find the step in §6 where we round or order the operations differently. Candidates are f32 against f64 intermediates, `a + t*(b-a)` against `a*(1-t) + b*t`, fused or unfused operations, and the cell-to-world conversion order. Fix it, staying within the spec.

- [ ] **Step 1:** Extract the inputs of that link from `d1m.dzgeo` and write a synthetic test: same geometry shape, minimal, no game data. It must reproduce the rounding and fail.
- [ ] **Step 2:** Fix, run green, then run the Task 1 live test on all sets: no regressions, and 138,102 now matches.
- [ ] **Step 3:** `make check`, then commit (`fix(navmesh): link endpoint rounding (#116)`).

### Task 3: Livonia town tiles with a different structure (#117)

**Files:** found by the investigation; add tests to the test file of the stage at fault.

**Data:** Livonia `dense.dzgeo`, tiles 130,220, 131,220, 131,221 and 131,222. Use superpowers:systematic-debugging.

The counts:

| Tile | Polys (ours / shipped) | Verts (ours / shipped) | Off-mesh (ours / shipped) | Other |
|---|---|---|---|---|
| 130,220 | 229 / 226 | 462 / 461 | 102 / 102 | parts list differs |
| 131,220 | 111 / 117 | 203 / 205 | 56 / 56 | |
| 131,221 | 320 / 326 | 636 / 648 | 240 / 246 | ground polygons equal; one link has area 114 against 112 |
| 131,222 | 110 / 108 | 200 / 205 | 52 / 56 | |

Compare the inputs of those tiles (objects, parts, areas, obstacle and tree records) with the output. Find the first stage whose output can't produce the shipped tile, then the spec rule we got wrong. If the spec itself is wrong or silent, record a ruling in the report and fix the spec text as well.

- [ ] **Step 1:** Diagnose; write down the root cause with evidence.
- [ ] **Step 2:** Write a synthetic failing test for that rule.
- [ ] **Step 3:** Fix it, then rerun the live test on all five sets with no regressions. For `dense.dzgeo` use `DAYZ_NAVMESH_INNER=129,221,130,222`, and report the structure of the edge tiles separately.
- [ ] **Step 4:** `make check`, then commit (`fix(navmesh): <rule> (#117)`).

If no root cause is found after a thorough investigation, report BLOCKED with the evidence. Do not guess a fix.

### Task 4: DayZDiag recipe and validation status in the docs (#115)

**Files:**
- `docs/book/src/guides/navmesh.md`
- `docs/book/src/reference/navmesh.md` (if it repeats the recipe)
- `README.md` / `crates/dayzmake-core/README.md` (wherever they say the builder is unvalidated)
- `docs/superpowers/specs/2026-11-17-navmesh-cleanroom-spec.md` §14 item 3 (if Task 1 didn't already update it)

**Content:**
- The mission must be a **complete copy** of a stock mission (for example `DayZServer/mpmissions/dayzOffline.enoch` or `.nasdara`, or the mod's own full mission), with only `init.c` replaced by an empty `main()`. A folder containing only `init.c` makes the server log `[Server] :: termination in: 10` and exit before the data server starts.
- Server config: a copy of the stock `serverDZ.cfg` with `verifySignatures = 0;`, `BattlEye = 0;` and the mission's `template`.
- `-profiles=` **works** for the server (`Z:\...` under Proton) and puts the RPT and script logs into that folder. Replace the "leave out `-profiles=`" advice.
- Add `-nosplash -nopause -dologs -adminlog` to the examples. A DLC map (Badlands, `dayzOffline.nasdara`) needs no `-mod`.
- The data server opens port 36533 about 25 s after start. Wait until `navmesh probe` answers, because a connection made while the world is still loading is reset.
- Validation status: replace the "not yet compared with live DayZDiag output" statement with what was compared and the result:
  - Livonia and Badlands, DayZ 1.30, including the Badlands irrigation tunnels;
  - inner tiles byte-identical apart from polygon flags, which come from the island filter;
  - only inner tiles can match, because of the far-end snap of links into neighbouring tiles;
  - any issue that is still open after tasks 1–3, named by number.
- Re-run every changed example command, or say in the report that it can't be run (no DayZ in CI).

- [ ] **Step 1:** Update the docs; `make book` and `cargo test --test book_sources`.
- [ ] **Step 2:** `make check`; check coverage with `make coverage-check` and report the total.
- [ ] **Step 3:** Commit (`docs(navmesh): working DayZDiag recipe and live validation (#115)`).
