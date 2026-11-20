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

---

## Part 2: central-half Badlands comparison

Badlands tiles 48–278 on both axes were built live (53,361 tiles; capture `/var/tmp/dznavbad.AX9NDF/big.dzgeo`, 50 GB; mesh `big.nm`), and inner tiles 49–277 were compared with the shipped mesh. About 98 % match. The classes of difference are issues #119–#124; #118 came from the final review, and #125/#126 are generator bugs. Per-class lists with examples are in `/var/tmp/dznavbad.AX9NDF/classes.txt`, and the raw comparison is in `big.cmp`.

The acceptance check for tasks 6–11 is the live test (`navmesh_live.rs`) on the five original sets plus margin blocks cut from `big.dzgeo`, which works once Task 5 is done. At the end, the whole central half is re-run from the capture (Task 12).

### Task 5: Stream ranges from large captures (#125) and retry refused connections (#126)

**Files:** `crates/dayzmake-core/src/navmesh/capture.rs`, `generate.rs`, `client.rs`; tests in `tests/navmesh_generate.rs` and `tests/navmesh_client.rs`.

- **#125:** building a range from a capture needs memory in proportion to the range, not to the file. Index or seek tile records lazily; never hold more than the tiles in flight. Test: a synthetic capture with many large tiles built with a small `--range` under a memory budget asserted through the existing span or byte limits. Also run, under `prlimit --as=8000000000`, a 3 × 3 range from `big.dzgeo`.
- **#126:** retry a refused or reset connect with a bounded backoff, at most 5 tries in total, at most 10 s overall, no sleeps in tests (inject the clock or delay). A tile whose connection died goes back to the queue for the live connections. It fails only when no connection is left. Test against the fake server: refuse the first N connects.

### Task 6: Off-mesh connection endpoint direction (#120) and field rounding (#121)

**Files:** `links.rs`; spec §6. Reproduce with margin blocks from `big.dzgeo` (examples: 81,59 #16; 163,52 #34; 163,54 #89 and #105; 176,59 #21/#45/#69). Find the rule for which end is `pos[0..3]`, and the rounding of `center_height` and `rad`. Synthetic tests, then rulings in spec §14.

### Task 7: Off-mesh connection count (#119)

**Files:** `links.rs`; spec §6. Examples: 185,60 (55/51), 223,50 (64/63), 225,51 (68/67). Diagnose which links are added or missing and why. Do this after Task 6, because the direction rule may change which duplicates are removed.

### Task 8: Tile parts list (#122)

**Files:** the parts or extra-section code (`dayz/areas.rs`, `build.rs`, `detour/create.rs`); spec §5. Examples: 183,62 (1700/1724), 185,60, 183,63.

### Task 8b: Off-mesh hide-part assignment residuals (#127)

R17 (Task 8) is a placeholder rule. Find the missing condition behind the residual cases listed in #127: the 27 wrong-part cases (the shipped mesh uses the next part index), the class-4 false positives, the remaining extra and missing connections, and the 14 tiles whose ground-polygon part lists differ. The data is `/var/tmp/dznavbad.AX9NDF/t8c.cmp`, and the reviewer's sweep scripts are in `/var/tmp/rv8`. Run it after Task 9, because #124 overlaps.

### Task 9: Header bmax y (#123) and ground polygon cluster (#124)

**Files:** found by investigation. Examples: 171,210, 172,210 (bmax); 172,211, 191,188, 202,187 (ground). Find out what is special in the 171–177 × 210–229 area first.

### Task 10: Contour sort tie order (#118)

Make `msvc_qsort` generic and use it for the hole and diagonal sorts in `recast/contour.rs`; pin the orders with tests; update spec §14 item 3.

### Task 11: Doc outputs and final-review minors

Re-run the synthetic-world examples in `docs/book/src/guides/navmesh.md` and `reference/navmesh.md` and update the outputs (final-review I1). Pin the `info` totals in a CLI test. Then the minors from `.superpowers/sdd/2026-11-18-navmesh-live-fixes/final-review.md` and the carried Task 4 minors. Finally, the validation section with the central-half result after Task 12.

### Task 12: Whole central half from the capture

Rebuild the binary from HEAD first; shared binaries in /var/tmp must never be left as experiment builds. Then rebuild `48,48,278,278` from `big.dzgeo` under `systemd-run --user --scope -p MemoryMax=16G -p MemorySwapMax=0` and compare the inner tiles. Target: every inner tile matches, apart from polygon flags and the link array. Report anything left over as issues.

## Part 3: before the push (user decision, 2026-11-20)

### Task 13: Deterministic coverage (#111)

> Note (history): this is the task text as it was given. The maintainer then asked to relax the pull-request check
> (`project` threshold 1 % on pull requests, 0 % on `main`; see `codecov.yml` and AGENTS.md, section Coverage), so the
> "do not relax its thresholds" and "threshold 0 %" wording below is superseded. It is left as it was.

**Files:** found by investigation: the Makefile's coverage targets, `cargo llvm-cov` settings, CLI tests that spawn the binary (`tests/`), and `codecov.yml` (do not relax its thresholds).

The same commit gave different line coverage: `src/commands/pbo.rs:51` was covered on main and missed on PR #110, with no code change, so Codecov's `project` check (threshold 0 %) failed.
- Reproduce: run `make coverage` (or the CI coverage step) twice on the same commit, export lcov both times, and diff the line hits. Repeat until a flip shows; report how many runs it took.
- Find the cause. Candidates: subprocess profiles lost (the `LLVM_PROFILE_FILE` pattern, `%p`/`%m`, merge races), a test killing the binary before it flushes its profile, timing-dependent code paths, parallel test interference.
- Fix the cause, not the symptom. Prove it: N consecutive coverage runs (N ≥ 5) give identical lcov line data.
- Never lower the threshold or exclude code.

### Task 14: Tree obstacle order (#128)

**Files:** navmesh obstacle and tree code; spec §14.

Badlands tile 146,83 differs from the shipped mesh only in the order of two tree obstacle records. Decide between data (the server's answer order, as R20 for doors) and a generator sort rule. Measure candidate sort keys on every tile with two or more tree obstacles that already matches, as R20 did for doors. Fix it if a rule fits everything without regressions; otherwise record it as data in a ruling.
