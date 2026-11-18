//! Jump, climb and ladder links (spec 6): the off-mesh connections of a tile.
//!
//! [`generate_links`] runs at step 7 of the tile build. It lists the ladders (6.1); then, when the
//! world has link classes, it builds monotone regions and contours on the final compact heightfield
//! (6.2), finds facing edge segments ([`refine_edge`], 6.3) and sweeps every link class along the
//! edges ([`class_spans`], [`sample_pass`], [`walk_trajectory`], 6.4), merges reverse spans
//! ([`reverse_merge`]) and emits the connections ([`emit`]).
//!
//! All `f32` expressions are written in the association the spec gives: `dir*t + Ebase` is not
//! `dir*t + n*h + A`. The vector helpers below do one operation each, so the order stays visible.
//!
//! Limits (hostile link classes): a trajectory has at most [`MAX_TRAJECTORY_SAMPLES`] samples, a
//! pass along an edge at most [`MAX_EDGE_SAMPLES`], and one span emits at most [`MAX_SPAN_LINKS`]
//! connections; beyond them the build fails with an error instead of allocating.

use std::collections::BTreeMap;
use std::io;

use super::build::{TileConfig, min_region_table};
use super::detour::OffMeshInput;
use super::num::{ceil_i32, floor_i32, i32_to_f32, trunc_i32, wrap_u32};
use super::recast::{
    compact::CompactHeightfield,
    contour::{ContourSet, RC_AREA_BORDER, RC_CONTOUR_TESS_WALL_EDGES, build_contours},
    heightfield::Heightfield,
    region::build_regions_monotone,
};
use super::wire::{LinkClass, TileGroup, WorldParams};

/// A 3-D point or vector `[x, y, z]`.
pub(crate) type V = [f32; 3];

/// C `FLT_EPSILON`.
const FLT_EPSILON: f32 = f32::EPSILON;
/// C `FLT_MAX`.
const FLT_MAX: f32 = f32::MAX;
/// The facing limit of spec 6.3 step 4: `cosf(2.3561945f)`, compared in `f32` (ruling R12).
const COS_135: f32 = f32::from_bits(0xBF35_04F3); // cosf(2.3561945f)
/// The most trajectory samples `nT` of a link class (the check of one sample costs `nT` steps).
pub(crate) const MAX_TRAJECTORY_SAMPLES: i32 = 4096;
/// The most samples `nS` along one edge in one pass.
pub(crate) const MAX_EDGE_SAMPLES: i32 = 1_000_000;
/// The most connections one span emits.
pub(crate) const MAX_SPAN_LINKS: i32 = 1_000_000;
/// Segment records per refinement (spec 12.2).
const MAX_RECORDS: usize = 64;
/// Output segments per refinement (spec 12.2).
const MAX_OUTPUTS: usize = 32;
/// Poly flags of ladder connections.
const LADDER_FLAGS: u16 = 0x40;
/// Area of ladders.
const LADDER_AREA: u8 = 15;

/// The tree area: its spans carry a tree index in the part id field, not a part (spec 5.4).
const TREE_AREA: u8 = 10;
const STATE_VALID: u8 = 0;
const STATE_INVALID: u8 = 1;
const STATE_WATER: u8 = 2;

pub(crate) fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub(crate) fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The 3-D length `|a|`.
pub(crate) fn len(a: V) -> f32 {
    dot(a, a).sqrt()
}

/// The squared 3-D distance.
fn dist_sq(a: V, b: V) -> f32 {
    let d = sub(a, b);
    dot(d, d)
}

/// `v / |v|` when the length is at least `FLT_EPSILON`, else `(v.x, 1, v.z)` unnormalised.
fn normalize3(v: V) -> V {
    let l = len(v);
    if l >= FLT_EPSILON {
        // Multiplies by the reciprocal, as the shipped code does (R16): `v / l` is one ulp away for some v.
        let r = 1.0_f32 / l;
        [v[0] * r, v[1] * r, v[2] * r]
    } else {
        [v[0], 1.0, v[2]]
    }
}

/// C `(float)i` of an index.
fn idx_f32(i: usize) -> f32 {
    i32_to_f32(i32::try_from(i).unwrap_or(i32::MAX))
}

/// A segment between two world points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Seg {
    pub(crate) p0: V,
    pub(crate) p1: V,
}

impl Seg {
    /// The squared 3-D length.
    fn len_sq(&self) -> f32 {
        dist_sq(self.p1, self.p0)
    }
}

/// The off-mesh connections of a tile and, for each, the hide part it belongs to (spec 7.2).
#[derive(Debug, Default)]
pub(crate) struct Links {
    /// The connections: the ladders first, then the link classes in order.
    pub(crate) off_mesh: Vec<OffMeshInput>,
    /// `parts[i]`: the index in the tile's extras list of the hide part of connection `i`, if any.
    pub(crate) parts: Vec<Option<u16>>,
}

// 6.1 Ladders

/// Spec 6.1: one connection per vertex pair of every ladder group, in group order.
pub(crate) fn ladder_links(ladders: &[&TileGroup], out: &mut Links) {
    for g in ladders {
        for pair in g.verts.as_chunks::<2>().0 {
            out.parts.push(None);
            out.off_mesh.push(OffMeshInput {
                start: pair[0],
                end: pair[1],
                center_height: 0.0,
                radius: 1.0,
                flags: LADDER_FLAGS,
                area: LADDER_AREA,
                bidirectional: true,
                user_id: 0,
            });
        }
    }
}

// 6.2 Candidate edges

/// `P(v)` of spec 6.2: a contour vertex in world coordinates, with the +2 voxels on y.
fn contour_point(cset: &ContourSet, v: [i32; 4]) -> V {
    [
        i32_to_f32(v[0]) * cset.cs + cset.bmin[0],
        i32_to_f32(v[1].saturating_add(2)) * cset.ch + cset.bmin[1],
        i32_to_f32(v[2]) * cset.cs + cset.bmin[2],
    ]
}

/// Spec 6.2 step 2: the segments of the contours' area-border edges, in contour order.
fn edge_segments(cset: &ContourSet) -> Vec<Seg> {
    let mut segs = Vec::new();
    for contour in &cset.contours {
        let verts = &contour.verts;
        let nv = verts.len();
        for i in 0..nv {
            let j = (i + nv - 1) % nv;
            let (Some(&vi), Some(&vj)) = (verts.get(i), verts.get(j)) else {
                continue;
            };
            if vj[3] & RC_AREA_BORDER == 0 {
                continue;
            }
            let s = Seg {
                p0: contour_point(cset, vi),
                p1: contour_point(cset, vj),
            };
            if s.len_sq() >= 1.5 {
                segs.push(s);
            }
        }
    }
    segs
}

/// Spec 6.2 step 3: `depth = min(3, max(diameter * 3, jumpLength))` over the link classes.
pub(crate) fn link_depth(world: &WorldParams) -> f32 {
    let mut depth = 3.0_f32;
    for c in &world.link_classes {
        depth = depth.min((world.world.diameter * 3.0_f32).max(c.jump_length));
    }
    depth
}

// 6.3 refineEdge

/// One candidate of `refineEdge`.
struct Rec {
    group: u32,
    seg: usize,
    t0: f32,
    t1: f32,
    zmin: f32,
    zmax: f32,
}

/// Clips the parametric segment `a + t*e` against the box `[min, max]` (spec 6.3 step 3). Returns
/// `(tmin, tmax)` of an accepted segment.
fn clip_segment(a: V, e: V, min: V, max: V) -> Option<(f32, f32)> {
    let mut tmin = 0.0_f32;
    let mut tmax = FLT_MAX;
    for (((&a, &e), &lo), &hi) in a.iter().zip(&e).zip(&min).zip(&max) {
        if e.abs() < 1e-6_f32 {
            if a < lo || a > hi {
                return None;
            }
        } else {
            let ood = 1.0_f32 / e;
            let mut t1 = (lo - a) * ood;
            let mut t2 = (hi - a) * ood;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            tmin = tmin.max(t1);
            tmax = tmax.min(t2);
            if tmax < tmin {
                return None;
            }
        }
    }
    (tmin <= 1.0 && tmax >= 0.0).then_some((tmin, tmax))
}

/// `refineEdge(s, depth, width, segs, cs)` (spec 6.3): the midlines between `s` and the segments
/// that face it across an obstacle within `depth`.
#[expect(
    clippy::many_single_char_names,
    reason = "the variable names of spec 6.3"
)]
pub(crate) fn refine_edge(s: &Seg, depth: f32, width: f32, segs: &[Seg], cs: f32) -> Vec<Seg> {
    // 1. The frame of s.
    let p0 = s.p0;
    let d = sub(s.p1, s.p0);
    let length = len(d);
    let u = normalize3(d);
    let n = normalize3([u[2], 0.0, -u[0]]);
    // 2. Local coordinates and the box.
    let local = |q: V| -> V {
        let dq = sub(q, p0);
        [
            u[0] * dq[0] + n[0] * dq[2],
            u[1] * dq[0] + dq[1] + n[1] * dq[2],
            u[2] * dq[0] + n[2] * dq[2],
        ]
    };
    let box_min = [0.0, -width * 0.5, 0.0];
    let box_max = [length, width * 0.5, depth];

    // 3. and 4. The candidates.
    let mut recs: Vec<Rec> = Vec::new();
    for (k, q) in segs.iter().enumerate() {
        let a = local(q.p0);
        let b = local(q.p1);
        let Some((tmin, tmax)) = clip_segment(a, sub(b, a), box_min, box_max) else {
            continue;
        };
        let v = normalize3([q.p1[0] - q.p0[0], 0.0, q.p1[2] - q.p0[2]]);
        if dot(v, u) <= COS_135 && recs.len() < MAX_RECORDS {
            recs.push(Rec {
                group: 0,
                seg: k,
                t0: tmin.clamp(0.0, 1.0),
                t1: tmax.clamp(0.0, 1.0),
                zmin: a[2].min(b[2]),
                zmax: a[2].max(b[2]),
            });
        }
    }

    // 5. Grouping.
    let mut g = 1_u32;
    for i in 0..recs.len() {
        if recs.get(i).is_none_or(|r| r.group != 0) {
            continue;
        }
        let (zmin_i, zmax_i) = recs.get(i).map_or((0.0, 0.0), |r| (r.zmin, r.zmax));
        if let Some(r) = recs.get_mut(i) {
            r.group = g;
        }
        for r in recs.iter_mut().skip(i + 1) {
            if zmin_i - cs <= cs + r.zmax && r.zmin - cs <= cs + zmax_i {
                r.group = g;
            }
        }
        g += 1;
    }

    // 6. One midline per group.
    let dd = dot(d, d);
    let mut out = Vec::new();
    for group in 1..g {
        let mut smin = 10.0_f32;
        let mut smax = -10.0_f32;
        let mut min_pt = [0.0; 3];
        let mut max_pt = [0.0; 3];
        for r in recs.iter().filter(|r| r.group == group) {
            let Some(q) = segs.get(r.seg) else {
                continue;
            };
            let q_d = sub(q.p1, q.p0);
            for t_param in [r.t0, r.t1] {
                let c = add(q.p0, scale(q_d, t_param));
                let mut t = (c[1] - p0[1]) * d[1] + (c[0] - p0[0]) * d[0] + (c[2] - p0[2]) * d[2];
                if t <= 0.0 {
                    t = 0.0;
                } else if t < dd {
                    t /= dd;
                } else {
                    t = 1.0;
                }
                if t < smin {
                    smin = t;
                    min_pt = c;
                }
                if smax < t {
                    smax = t;
                    max_pt = c;
                }
            }
        }
        if smin <= smax && out.len() < MAX_OUTPUTS {
            let e0 = add(p0, scale(d, smin));
            let e1 = add(p0, scale(d, smax));
            out.push(Seg {
                p0: add(e0, scale(sub(min_pt, e0), 0.5)),
                p1: add(e1, scale(sub(max_pt, e1), 0.5)),
            });
        }
    }
    out
}

/// Spec 6.2 step 4: for each segment the longest refined segment (squared length above 0, ties
/// keep the first).
fn candidate_edges(segs: &[Seg], depth: f32, cs: f32) -> Vec<Seg> {
    let mut edges = Vec::new();
    for s in segs {
        let mut best: Option<Seg> = None;
        let mut best_len = 0.0_f32;
        for r in refine_edge(s, depth, 1.0, segs, cs) {
            let l = r.len_sq();
            if l > best_len {
                best_len = l;
                best = Some(r);
            }
        }
        edges.extend(best);
    }
    edges
}

// 6.4 Per link class

/// One trajectory sample (spec 6.4, *Trajectory*): the height profile `y` and the window extremes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TrajSample {
    pub(crate) y: f32,
    pub(crate) ymin: f32,
    pub(crate) ymax: f32,
}

/// The trajectory of a link class.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Trajectory {
    /// `(r + r) + jumpLength`.
    pub(crate) total: f32,
    /// `nT` samples (none when `nT <= 0`).
    pub(crate) samples: Vec<TrajSample>,
}

/// Spec 6.4 *Trajectory*.
///
/// # Errors
/// Returns an error when `nT` exceeds [`MAX_TRAJECTORY_SAMPLES`].
pub(crate) fn trajectory(class: &LinkClass, cs: f32) -> io::Result<Trajectory> {
    let r = class.agent_radius;
    let total = (r + r) + class.jump_length;
    let n_t = ceil_i32(total / cs);
    if n_t > MAX_TRAJECTORY_SAMPLES {
        return Err(error!(
            "link class {}: the trajectory needs {n_t} samples (max {MAX_TRAJECTORY_SAMPLES})",
            class.index
        ));
    }
    let count = usize::try_from(n_t).unwrap_or(0);
    let xs: Vec<f32> = (0..n_t).map(|i| i32_to_f32(i) * cs).collect();
    let ys: Vec<f32> = xs
        .iter()
        .map(|&x| {
            if x < r {
                class.max_step_height
            } else if x >= total - r {
                class.jump_dropdown_min
            } else {
                class.jump_height
            }
        })
        .collect();
    let x_first = xs.first().copied().unwrap_or(0.0);
    let x_last = xs.last().copied().unwrap_or(0.0);
    let mut samples = Vec::with_capacity(count);
    for (&x, &y) in xs.iter().zip(&ys) {
        let lo = (x - r).max(x_first);
        let hi = (x + r).min(x_last);
        let mut ymin = y;
        let mut ymax = y;
        for (&xj, &yj) in xs.iter().zip(&ys) {
            if lo <= xj && xj < hi {
                ymin = ymin.min(yj);
                ymax = ymax.max(yj);
            }
        }
        samples.push(TrajSample {
            y,
            ymin,
            ymax: ymax + class.agent_height,
        });
    }
    Ok(Trajectory { total, samples })
}

/// One link span of a class: a run of valid samples along an edge (spec 6.4, *Run extraction*).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LinkSpan {
    pub(crate) s0: V,
    pub(crate) s1: V,
    pub(crate) e0: V,
    pub(crate) e1: V,
    pub(crate) center_h: f32,
    /// 0 merged away, 1 one-way, 2 bidirectional.
    pub(crate) validity: u8,
    /// The hide part the span belongs to: [`RunVisits::part`] over its samples.
    pub(crate) part: Option<u16>,
}

/// The constants and fields of one link class sweep (spec 6.4, *Sampling*).
pub(crate) struct ClassCtx<'a> {
    chf: &'a CompactHeightfield,
    hf: &'a Heightfield,
    class: &'a LinkClass,
    traj: Trajectory,
    r: f32,
    cs: f32,
    ch: f32,
    h: f32,
    l2: f32,
    mch: f32,
    inv_nt: f32,
    min_run: i32,
    /// `hide[i]`: part `i` of the tile's extras list is a part of a hide group (area 14).
    hide: &'a [bool],
}

impl<'a> ClassCtx<'a> {
    /// The context of `class`, or `None` when its trajectory has no samples (`nT <= 0`).
    ///
    /// # Errors
    /// Returns an error when the trajectory is too long ([`trajectory`]).
    pub(crate) fn new(
        chf: &'a CompactHeightfield,
        hf: &'a Heightfield,
        class: &'a LinkClass,
        hide: &'a [bool],
    ) -> io::Result<Option<Self>> {
        let cs = chf.cs;
        let traj = trajectory(class, cs)?;
        if traj.samples.is_empty() {
            return Ok(None);
        }
        let r = class.agent_radius;
        let n_t = i32::try_from(traj.samples.len()).unwrap_or(i32::MAX);
        Ok(Some(Self {
            chf,
            hf,
            class,
            r,
            cs,
            ch: chf.ch,
            h: traj.total * 0.5,
            l2: class.jump_length * 0.5,
            mch: class.min_center_height,
            inv_nt: 1.0_f32 / i32_to_f32(n_t),
            min_run: ceil_i32((r + r) / cs),
            traj,
            hide,
        }))
    }

    /// True when `sub` is the part id of a hide part of this tile (id 0 is no part).
    fn is_hide_part(&self, sub: u16) -> bool {
        sub != 0 && self.hide.get(usize::from(sub)).copied().unwrap_or(false)
    }
}

/// The geometry of one pass along an edge (spec 6.4, *Sampling* steps 1 and 2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PassGeometry {
    pub(crate) a: V,
    pub(crate) n: V,
    pub(crate) s0: V,
    pub(crate) dir: V,
    pub(crate) p0: V,
    pub(crate) p1: V,
    /// `n*h + A`, computed once.
    pub(crate) ebase: V,
    pub(crate) dir_len: f32,
}

/// Spec 6.4 *Sampling* steps 1 and 2: `side` is +1 (pass 0) or -1 (pass 1).
pub(crate) fn pass_geometry(edge: &Seg, side: f32, h: f32, l2: f32) -> PassGeometry {
    let a = edge.p0;
    let b = edge.p1;
    let dn = scale(normalize3(sub(b, a)), side);
    let n = normalize3([dn[2], 0.0, -dn[0]]);
    let s0 = add(scale(n, -h), a);
    // The literal `(n*(-h) + B) - S0`; `n` is normalised by the reciprocal (R16), and then this matches the
    // shipped meshes where `B - A` does not (#116, #120).
    let dir = sub(add(scale(n, -h), b), s0);
    PassGeometry {
        a,
        n,
        s0,
        dir,
        p0: add(scale(n, -l2), a),
        p1: add(scale(n, l2), a),
        ebase: add(scale(n, h), a),
        dir_len: len(dir),
    }
}

/// `S`, `E_k` and `cy` of the sample at parameter `t` (spec 6.4 step 3). `E_k` is `dir*t + Ebase`.
pub(crate) fn sample_at(g: &PassGeometry, t: f32) -> (V, V, f32) {
    let s = add(scale(g.dir, t), g.s0);
    let e = add(scale(g.dir, t), g.ebase);
    let cy = g.dir[1] * t + g.a[1];
    (s, e, cy)
}

/// The floor of `p` in the compact heightfield (spec 6.4 step 3.1): among the spans of its cell with
/// an area, the height closest to `p.y`.
fn compact_floor(chf: &CompactHeightfield, p: V) -> Option<f32> {
    let cx = floor_i32((p[0] - chf.bmin[0]) / chf.cs);
    let cz = floor_i32((p[2] - chf.bmin[2]) / chf.cs);
    let mut best: Option<(f32, f32)> = None;
    for i in chf.column_range(cx, cz) {
        if chf.areas.get(i).is_none_or(|&a| a == 0) {
            continue;
        }
        let Some(span) = chf.spans.get(i) else {
            continue;
        };
        let y = f32::from(span.y) * chf.ch + chf.bmin[1];
        let d = (y - p[1]).abs();
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((y, d));
        }
    }
    best.map(|(y, _)| y)
}

/// True for the water areas 2 to 6.
fn is_water(area: u8) -> bool {
    (2..=6).contains(&area)
}

/// The outcome of walking one trajectory (spec 6.4 step 3.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Walk {
    /// A failure state: 1 blocked, 2 water.
    pub(crate) fail: Option<u8>,
    /// The highest `last` over the steps.
    pub(crate) hi_u: u16,
    /// The lowest `last` over the steps.
    pub(crate) lo_u: u16,
    /// The hide part ids (spec 7.2) of the spans the walk visited, with the number of spans.
    pub(crate) parts: Visits,
}

/// Visits per hide part id; empty for a walk that met none (no allocation).
pub(crate) type Visits = BTreeMap<u16, u32>;

/// A hide part must have been met by at least `MET_NUM / MET_DEN` of the samples of a run (ruling
/// R17). The value is a fit: the data does not pin it down (spec 14.19).
const MET_NUM: usize = 1;
const MET_DEN: usize = 4;

/// The visits of the samples of one run of valid samples, totalled while the samples are scanned.
#[derive(Debug, Default)]
pub(crate) struct RunVisits {
    len: usize,
    met: usize,
    total: Visits,
}

impl RunVisits {
    /// Adds the walk of the next valid sample.
    pub(crate) fn add(&mut self, walk: &Visits) {
        self.len += 1;
        if walk.is_empty() {
            return;
        }
        self.met += 1;
        for (&part, &n) in walk {
            let e = self.total.entry(part).or_insert(0);
            *e = e.saturating_add(n);
        }
    }

    /// The hide part of the run (ruling R17): the part with the most visits, the lowest id on a tie,
    /// when enough of the samples met a hide part; otherwise none.
    pub(crate) fn part(&self) -> Option<u16> {
        if self.met.saturating_mul(MET_DEN) < self.len.saturating_mul(MET_NUM) {
            return None;
        }
        self.total
            .iter()
            .max_by_key(|&(&part, &n)| (n, std::cmp::Reverse(part)))
            .map(|(&part, _)| part)
    }
}

/// Spec 6.4 step 3.6: the trajectory from `S` (floor `ys`) to `E_k` (floor `ye`) through the solid
/// heightfield.
///
/// `last` belongs to the sample, not to a step: it is reset to 0 only when the step's solid column
/// has spans.
#[expect(
    clippy::many_single_char_names,
    reason = "the variable names of spec 6.4"
)]
pub(crate) fn walk_trajectory(ctx: &ClassCtx<'_>, s: V, e: V, ys: f32, ye: f32) -> Walk {
    let (hf, chf) = (ctx.hf, ctx.chf);
    let n_t = ctx.traj.samples.len();
    let mut hi_u = 0_u16;
    let mut lo_u = 10000_u16;
    let mut last = 0_u16;
    let mut parts = Visits::new();
    for (j, t) in ctx.traj.samples.iter().enumerate() {
        let u = idx_f32(j) * ctx.inv_nt;
        let y = ys + (ye - ys) * u;
        let x = s[0] + (e[0] - s[0]) * u;
        let z = s[2] + (e[2] - s[2]) * u;
        let cx = trunc_i32((x - hf.bmin[0]) / ctx.cs);
        let cz = trunc_i32((z - hf.bmin[2]) / ctx.cs);
        let in_range = cx >= 0 && cx < hf.width && cz >= 0 && cz < hf.height;
        let fail = |state, parts| Walk {
            fail: Some(state),
            hi_u,
            lo_u,
            parts,
        };

        if (j == 0 || j + 1 == n_t) && in_range {
            let first = chf.column_range(cx, cz).next();
            if let Some(area) = first.and_then(|i| chf.areas.get(i))
                && is_water(*area)
            {
                return fail(STATE_WATER, parts);
            }
        }
        let column = hf.column(cx, cz);
        if in_range && !column.is_empty() {
            last = 0;
            let top = wrap_u32((y + t.ymax - hf.bmin[1]) / ctx.ch);
            let bot = wrap_u32((y + t.ymin - hf.bmin[1]) / ctx.ch);
            let mut prev = None;
            for sp in column {
                if top < u32::from(sp.smin) {
                    break;
                }
                last = sp.smax;
                if sp.area != TREE_AREA && ctx.is_hide_part(sp.sub_area) {
                    {
                        let n = parts.entry(sp.sub_area).or_insert(0_u32);
                        *n = n.saturating_add(1);
                    }
                }
                if bot <= u32::from(sp.smax) && u32::from(sp.smin) <= top {
                    return fail(STATE_INVALID, parts);
                }
                prev = Some(sp);
            }
            if prev.is_some_and(|sp| is_water(sp.area)) {
                return fail(STATE_WATER, parts);
            }
        }
        hi_u = hi_u.max(last);
        lo_u = lo_u.min(last);
    }
    Walk {
        fail: None,
        hi_u,
        lo_u,
        parts,
    }
}

/// Spec 6.4 step 3: the state (0 valid, 1 invalid, 2 water) and obstacle height of sample `k`.
fn sample_state(ctx: &ClassCtx<'_>, g: &PassGeometry, t: f32) -> (u8, f32, Visits) {
    let class = ctx.class;
    let (s, e, cy) = sample_at(g, t);
    // 3.1 and 3.2: the floor under the start.
    let Some(y_s) = compact_floor(ctx.chf, s) else {
        return (STATE_INVALID, 0.0, Visits::new());
    };
    if !(y_s <= class.jump_dropdown_min + class.max_step_height + s[1]
        && s[1] - (class.max_step_height - class.jump_dropdown_max) <= y_s)
    {
        return (STATE_INVALID, 0.0, Visits::new());
    }
    // 3.3 and 3.4: the floor under the end.
    let ys = y_s + ctx.ch;
    let y_top = ys + ctx.traj.samples.last().map_or(0.0, |t| t.ymin);
    let Some(y_e) = compact_floor(ctx.chf, e) else {
        return (STATE_INVALID, 0.0, Visits::new());
    };
    if !(y_e <= class.max_step_height + y_top
        && (y_top - class.jump_dropdown_min + class.jump_dropdown_max) - class.max_step_height
            <= y_e)
    {
        return (STATE_INVALID, 0.0, Visits::new());
    }
    let ye = y_e + ctx.ch;
    // 3.6 and 3.7.
    let walk = walk_trajectory(ctx, s, e, ys, ye);
    if let Some(state) = walk.fail {
        return (state, 0.0, Visits::new());
    }
    let hsel = if ctx.mch >= 0.0 { walk.hi_u } else { walk.lo_u };
    let center_h = i32_to_f32(i32::from(hsel) + 1) * ctx.ch + ctx.hf.bmin[1] - cy;
    let invalid = (ctx.mch >= 0.0 && center_h < ctx.mch) || (ctx.mch < 0.0 && center_h > ctx.mch);
    (
        if invalid { STATE_INVALID } else { STATE_VALID },
        center_h,
        walk.parts,
    )
}

/// Snaps `p.y` to the top of the solid span of its column closest to it (spec 6.4, *Run
/// extraction*); out of range or empty columns leave it.
fn snap_to_solid(hf: &Heightfield, p: &mut V) {
    let cx = trunc_i32((p[0] - hf.bmin[0]) / hf.cs);
    let cz = trunc_i32((p[2] - hf.bmin[2]) / hf.cs);
    let mut best: Option<(f32, f32)> = None;
    for sp in hf.column(cx, cz) {
        let top = f32::from(sp.smax) * hf.ch + hf.bmin[1];
        let d = (top - p[1]).abs();
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((top, d));
        }
    }
    if let Some((top, _)) = best {
        p[1] = top;
    }
}

/// Spec 6.4 *Run extraction*: the runs of valid samples longer than `minRun` become spans.
fn extract_runs(
    ctx: &ClassCtx<'_>,
    g: &PassGeometry,
    step: f32,
    state: &[u8],
    value: &[f32],
    run_parts: &[Option<u16>],
    spans: &mut Vec<LinkSpan>,
) {
    let n_s = state.len();
    let mut start: Option<usize> = None;
    let mut run_index = 0_usize;
    for k in 0..=n_s {
        // k == nS is an invalid sentinel.
        let valid = state.get(k).is_some_and(|&s| s == STATE_VALID);
        if valid {
            start.get_or_insert(k);
            continue;
        }
        let Some(first) = start.take() else {
            continue;
        };
        let part = run_parts.get(run_index).copied().flatten();
        run_index += 1;
        let run = i32::try_from(k - first).unwrap_or(i32::MAX);
        if run <= ctx.min_run {
            continue;
        }
        let t0 = idx_f32(first) * step;
        let t1 = idx_f32(k - 1) * step;
        let mut s0 = add(g.p0, scale(g.dir, t0));
        let mut s1 = add(g.p0, scale(g.dir, t1));
        let mut e0 = add(g.p1, scale(g.dir, t0));
        let mut e1 = add(g.p1, scale(g.dir, t1));
        let center_h = value.get(k - 1).copied().unwrap_or(0.0);
        for p in [&mut s0, &mut s1, &mut e0, &mut e1] {
            snap_to_solid(ctx.hf, p);
        }
        spans.push(LinkSpan {
            s0,
            s1,
            e0,
            e1,
            center_h,
            validity: 1,
            part,
        });
    }
}

/// One pass of spec 6.4 *Sampling* along `edge`: `side` is +1 or -1.
///
/// # Errors
/// Returns an error when the edge needs more than [`MAX_EDGE_SAMPLES`] samples.
pub(crate) fn sample_pass(ctx: &ClassCtx<'_>, edge: &Seg, side: f32) -> io::Result<Vec<LinkSpan>> {
    let g = pass_geometry(edge, side, ctx.h, ctx.l2);
    let n_s = ceil_i32(g.dir_len / ctx.cs);
    if n_s <= 0 {
        return Ok(Vec::new());
    }
    if n_s > MAX_EDGE_SAMPLES {
        return Err(error!(
            "link class {}: an edge needs {n_s} samples (max {MAX_EDGE_SAMPLES})",
            ctx.class.index
        ));
    }
    let count = usize::try_from(n_s).unwrap_or(0);
    let step = if n_s == 1 {
        0.0
    } else {
        1.0_f32 / i32_to_f32(n_s - 1)
    };
    let mut state = vec![STATE_INVALID; count];
    let mut value = vec![0.0_f32; count];
    // The part of every maximal run of valid samples, in order; the visits of the samples are
    // totalled as they come, so nothing per sample is kept.
    let mut run_parts: Vec<Option<u16>> = Vec::new();
    let mut run = RunVisits::default();
    for (k, (st, val)) in state.iter_mut().zip(&mut value).enumerate() {
        let t = idx_f32(k) * step;
        let walk;
        (*st, *val, walk) = sample_state(ctx, &g, t);
        if *st == STATE_VALID {
            run.add(&walk);
        } else if run.len > 0 {
            run_parts.push(run.part());
            run = RunVisits::default();
        }
    }
    if run.len > 0 {
        run_parts.push(run.part());
    }
    let mut spans = Vec::new();
    extract_runs(ctx, &g, step, &state, &value, &run_parts, &mut spans);
    Ok(spans)
}

/// All spans of a class over `edges`: two passes per edge, `+n` then `-n`, in edge order.
///
/// # Errors
/// Returns the errors of [`sample_pass`].
pub(crate) fn class_spans(ctx: &ClassCtx<'_>, edges: &[Seg]) -> io::Result<Vec<LinkSpan>> {
    let mut spans = Vec::new();
    for edge in edges {
        spans.extend(sample_pass(ctx, edge, 1.0)?);
        spans.extend(sample_pass(ctx, edge, -1.0)?);
    }
    Ok(spans)
}

/// Spec 6.4 *Reverse merge*, literally: a span with validity 2 can match again as `a`.
pub(crate) fn reverse_merge(spans: &mut [LinkSpan], cs: f32) {
    let thr = (cs * 5.0_f32) * (cs * 5.0_f32);
    for i in 0..spans.len() {
        if spans.get(i).is_none_or(|s| s.validity == 0) {
            continue;
        }
        for j in (i + 1)..spans.len() {
            let (Some(&a), Some(&b)) = (spans.get(i), spans.get(j)) else {
                continue;
            };
            if b.validity == 0 {
                continue;
            }
            if dist_sq(b.e0, a.s0) < thr
                && dist_sq(b.e1, a.s1) < thr
                && dist_sq(b.s0, a.e0) < thr
                && dist_sq(b.s1, a.e1) < thr
            {
                let la = dist_sq(a.s1, a.s0);
                let lb = dist_sq(b.s1, b.s0);
                if la <= lb {
                    if let Some(a) = spans.get_mut(i) {
                        a.validity = 0;
                    }
                    if let Some(b) = spans.get_mut(j) {
                        b.validity = 2;
                    }
                } else {
                    if let Some(a) = spans.get_mut(i) {
                        a.validity = 2;
                        a.center_h = b.center_h;
                    }
                    if let Some(b) = spans.get_mut(j) {
                        b.validity = 0;
                    }
                }
                break;
            }
        }
    }
}

/// Spec 6.4 *Emission*: the connections of one span, one per ~2 m, centred.
///
/// # Errors
/// Returns an error when the span would emit more than [`MAX_SPAN_LINKS`] connections.
pub(crate) fn emit(span: &LinkSpan, class: &LinkClass, r: f32, out: &mut Links) -> io::Result<()> {
    let d = sub(span.s1, span.s0);
    let length = len(d);
    let d2 = 2.0_f32 * r;
    let (rem, mut off) = if length >= 2.0 * d2 {
        let rem = length - 2.0 * d2;
        let f = (rem * 0.5_f32).floor();
        (rem, (rem - (f + f)) * 0.5_f32 + d2)
    } else {
        (0.0, length * 0.5_f32)
    };
    let count = 1_i32.saturating_sub(trunc_i32(rem * -0.5_f32));
    if count > MAX_SPAN_LINKS {
        return Err(error!(
            "link class {}: a span would emit {count} connections (max {MAX_SPAN_LINKS})",
            class.index
        ));
    }
    for _ in 0..count {
        let w = if length > 0.0 { off / length } else { 0.0 };
        out.parts.push(span.part);
        out.off_mesh.push(OffMeshInput {
            start: add(span.s0, scale(d, w)),
            end: add(span.e0, scale(d, w)),
            center_height: span.center_h,
            radius: if count == 1 { off } else { 1.0 },
            flags: class.flags,
            area: class.area,
            bidirectional: span.validity != 1,
            user_id: class.index,
        });
        off += 2.0;
    }
    Ok(())
}

/// Step 7 of the tile build (spec 5.4, 6): the ladders, then the jump and climb links of every
/// link class. Modifies only the region ids of `chf`; the regular region build overwrites them.
///
/// # Errors
/// Returns an error when a stage of the region or contour build fails or a link class exceeds the
/// limits of this module.
pub(crate) fn generate_links(
    chf: &mut CompactHeightfield,
    hf: &Heightfield,
    ladders: &[&TileGroup],
    cfg: &TileConfig,
    world: &WorldParams,
    hide: &[bool],
) -> io::Result<Links> {
    let mut out = Links::default();
    ladder_links(ladders, &mut out);
    if world.link_classes.is_empty() {
        return Ok(out);
    }
    // 6.2 step 1.
    build_regions_monotone(chf, 1, &min_region_table(cfg), cfg.merge_region_area)
        .map_err(|e| error!("could not build link regions: {e}"))?;
    let cset = build_contours(
        chf,
        cfg.max_simplification_error,
        cfg.max_vertical_error,
        cfg.max_edge_len,
        RC_CONTOUR_TESS_WALL_EDGES,
    )
    .map_err(|e| error!("could not create link contours: {e}"))?;
    // 6.2 steps 2 to 4.
    let segs = edge_segments(&cset);
    let edges = candidate_edges(&segs, link_depth(world), chf.cs);
    // 6.4.
    for class in &world.link_classes {
        let Some(ctx) = ClassCtx::new(chf, hf, class, hide)? else {
            continue;
        };
        let mut spans = class_spans(&ctx, &edges)?;
        reverse_merge(&mut spans, ctx.cs);
        for span in spans.iter().filter(|s| s.validity != 0) {
            emit(span, class, ctx.r, &mut out)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navmesh::recast::compact::build_compact;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn close3(a: V, b: V) -> bool {
        close(a[0], b[0]) && close(a[1], b[1]) && close(a[2], b[2])
    }

    /// The link class of spec 13.2.
    fn synthetic_class() -> LinkClass {
        LinkClass {
            index: 0,
            jump_height: 1.2,
            jump_length: 1.8,
            min_center_height: 0.4,
            agent_height: 1.5,
            max_step_height: 0.45,
            agent_radius: 0.3,
            flags: 0x100,
            area: 48,
            ..LinkClass::default()
        }
    }

    fn seg(p0: V, p1: V) -> Seg {
        Seg { p0, p1 }
    }

    /// A 10 x 10 m field of 0.25 m cells with the floor top at `floor(x)` voxels (if any) and a wall
    /// top at `wall(x)` voxels (if any) in the cell columns `x`, whole rows.
    fn field(
        floor: impl Fn(i32) -> Option<u16>,
        wall: impl Fn(i32) -> Option<u16>,
        area: u8,
    ) -> (Heightfield, CompactHeightfield) {
        let mut hf =
            Heightfield::new(40, 40, [0.0, 0.0, 0.0], [10.0, 5.0, 10.0], 0.25, 0.1).unwrap();
        for x in 0..40 {
            for z in 0..40 {
                if let Some(top) = floor(x) {
                    hf.add_span(x, z, 0, top, area, 1);
                }
                if let Some(top) = wall(x) {
                    hf.add_span(x, z, 0, top, area, 1);
                }
            }
        }
        let chf = build_compact(3, 5, &hf).unwrap();
        (hf, chf)
    }

    #[test]
    fn trajectory_synthetic_class() {
        let t = trajectory(&synthetic_class(), 0.25).unwrap();
        assert!(close(t.total, 2.4));
        assert_eq!(t.samples.len(), 10);
        let y: Vec<f32> = t.samples.iter().map(|s| s.y).collect();
        // x < r (0.3): the first two samples; x >= total - r (2.1): the last one.
        assert_eq!(y, [0.45, 0.45, 1.2, 1.2, 1.2, 1.2, 1.2, 1.2, 1.2, 0.0]);
        // Windows: the first sample sees only the step height; the last one starts at the dropdown.
        assert!(close(t.samples[0].ymin, 0.45));
        assert!(close(t.samples[0].ymax, 0.45 + 1.5));
        assert!(close(t.samples[9].ymin, 0.0));
        assert!(close(t.samples[9].ymax, 1.2 + 1.5));
        // A middle sample sees 1.2 only.
        assert!(close(t.samples[5].ymin, 1.2));
        assert!(close(t.samples[5].ymax, 1.2 + 1.5));
    }

    #[test]
    fn trajectory_is_bounded() {
        let mut c = synthetic_class();
        c.jump_length = 1.0e9;
        let e = trajectory(&c, 0.1).unwrap_err();
        assert!(e.to_string().contains("samples"), "{e}");
        c.jump_length = 0.0;
        c.agent_radius = 0.0;
        assert_eq!(trajectory(&c, 0.1).unwrap().samples.len(), 0);
    }

    #[test]
    fn jump_depth() {
        let mut p = WorldParams::default();
        p.world.diameter = 0.6;
        p.link_classes = vec![synthetic_class()];
        // min(3, max(0.6 * 3, 1.8)) = 1.8.
        assert!(close(link_depth(&p), 1.8));
        p.link_classes[0].jump_length = 2.5;
        assert!(close(link_depth(&p), 2.5));
        p.link_classes[0].jump_length = 9.0;
        assert!(close(link_depth(&p), 3.0));
    }

    #[test]
    fn wrap_negative_heights() {
        assert_eq!(wrap_u32(-1.0), u32::MAX);
        assert_eq!(wrap_u32(-3.9), u32::MAX - 2);
        assert_eq!(wrap_u32(12.7), 12);
    }

    #[test]
    fn refine_edge_facing_segments() {
        let s = seg([0.0, 0.0, 0.0], [4.0, 0.0, 0.0]);
        // Faces s across 1.5 m: opposite direction, on the side of n = (0, 0, -1).
        let facing = seg([4.0, 0.0, -1.5], [0.0, 0.0, -1.5]);
        let out = refine_edge(&s, 1.8, 1.0, &[s, facing], 0.1);
        assert_eq!(out.len(), 1);
        assert!(close3(out[0].p0, [0.0, 0.0, -0.75]), "{:?}", out[0]);
        assert!(close3(out[0].p1, [4.0, 0.0, -0.75]), "{:?}", out[0]);
        // The same direction is not found.
        let same = seg([0.0, 0.0, -1.5], [4.0, 0.0, -1.5]);
        assert_eq!(refine_edge(&s, 1.8, 1.0, &[s, same], 0.1).len(), 0);
        // Farther than depth.
        let far = seg([4.0, 0.0, -2.0], [0.0, 0.0, -2.0]);
        assert_eq!(refine_edge(&s, 1.8, 1.0, &[s, far], 0.1).len(), 0);
        // On the other side of s (positive z) is outside the box.
        let behind = seg([4.0, 0.0, 1.5], [0.0, 0.0, 1.5]);
        assert_eq!(refine_edge(&s, 1.8, 1.0, &[s, behind], 0.1).len(), 0);
    }

    #[test]
    #[expect(
        clippy::many_single_char_names,
        reason = "short names of a geometry test"
    )]
    fn refine_edge_groups_and_limits() {
        let s = seg([0.0, 0.0, 0.0], [4.0, 0.0, 0.0]);
        // Two facing segments at the same distance form one group: the midline spans both.
        let a = seg([2.0, 0.0, -1.0], [0.0, 0.0, -1.0]);
        let b = seg([4.0, 0.0, -1.0], [3.0, 0.0, -1.0]);
        let out = refine_edge(&s, 1.8, 1.0, &[s, a, b], 0.1);
        assert_eq!(out.len(), 1);
        assert!(close3(out[0].p0, [0.0, 0.0, -0.5]), "{:?}", out[0]);
        assert!(close3(out[0].p1, [4.0, 0.0, -0.5]), "{:?}", out[0]);
        // Far apart in local z (more than 2 cs): separate groups.
        let c = seg([2.0, 0.0, -1.7], [0.0, 0.0, -1.7]);
        let d = seg([4.0, 0.0, -0.2], [3.0, 0.0, -0.2]);
        let out = refine_edge(&s, 1.8, 1.0, &[s, c, d], 0.1);
        assert_eq!(out.len(), 2);
        // 100 facing segments 0.5 m apart: 64 records are kept, forming 64 groups, of which 32 are output.
        let spread: Vec<Seg> = (0_i16..100)
            .map(|i| {
                let z = -0.1 - 0.5 * f32::from(i);
                seg([4.0, 0.0, z], [0.0, 0.0, z])
            })
            .collect();
        assert_eq!(refine_edge(&s, 100.0, 1.0, &spread, 0.0001).len(), 32);
        // 100 segments 0.05 m apart collapse into one group (the grouping is transitive from the first).
        let dense: Vec<Seg> = (0_i16..100)
            .map(|i| {
                let z = -0.5 - 0.0001 * f32::from(i);
                seg([4.0, 0.0, z], [0.0, 0.0, z])
            })
            .collect();
        assert_eq!(refine_edge(&s, 100.0, 1.0, &dense, 0.1).len(), 1);
    }

    #[test]
    fn edge_segments_flags_direction_height_and_length() {
        use crate::navmesh::recast::contour::{Contour, RC_BORDER_VERTEX};
        let b = RC_AREA_BORDER;
        let cset = ContourSet {
            cs: 0.5,
            ch: 0.5,
            bmin: [10.0, 20.0, 30.0],
            contours: vec![
                Contour {
                    // The flag of the previous vertex `vj` decides, not the flag of `vi`: edge
                    // v0 -> v3 (vj = v3, flagged) and v2 -> v1 (vj = v1, flagged) come out; v1 -> v0 and
                    // v3 -> v2 do not, although v1 and v3 carry the flag themselves.
                    verts: vec![[0, 0, 0, 0], [4, 1, 0, b], [4, 2, 4, 0], [0, 3, 4, b]],
                    ..Contour::default()
                },
                Contour {
                    // Squared length exactly 1.5 (dx = dz = 1 cell, dy = 2 voxels: 0.25 * 6) is kept;
                    // with dy = 1 it is 0.75 and dropped. Other flag bits do not count.
                    verts: vec![[0, 0, 0, b], [1, 2, 1, 0], [1, 3, 1, RC_BORDER_VERTEX]],
                    ..Contour::default()
                },
            ],
            ..ContourSet::default()
        };
        let segs = edge_segments(&cset);
        // Heights: (y + 2) * 0.5 + 20.
        let at = |x: i32, y: i32, z: i32| -> V {
            [
                10.0 + 0.5 * i32_to_f32(x),
                20.0 + 0.5 * i32_to_f32(y + 2),
                30.0 + 0.5 * i32_to_f32(z),
            ]
        };
        let got: Vec<(V, V)> = segs.iter().map(|s| (s.p0, s.p1)).collect();
        assert_eq!(
            got,
            vec![
                (at(0, 0, 0), at(0, 3, 4)),
                (at(4, 2, 4), at(4, 1, 0)),
                // Contour 2: i = 1, vj = v0 (flagged): P(v1) -> P(v0), dy = 2.
                (at(1, 2, 1), at(0, 0, 0)),
            ]
        );
        assert_eq!(contour_point(&cset, [4, 1, 0, 0]), at(4, 1, 0));
        // With dy = 1 the same edge is too short.
        let mut short = cset.clone();
        short.contours.remove(0);
        if let Some(v) = short.contours[0].verts.get_mut(1) {
            *v = [1, 1, 1, 0];
        }
        assert_eq!(edge_segments(&short).len(), 0);
    }

    /// Ruling R15: the slab test multiplies by the reciprocal `1 / e` (as upstream's segment-box test
    /// does) instead of dividing; `3 / 7` and `3 * (1 / 7)` are different `f32` values.
    #[test]
    fn clip_multiplies_by_the_reciprocal() {
        let (_, tmax) = clip_segment(
            [0.0, 0.0, 0.0],
            [7.0, 0.0, 0.0],
            [0.0, -1.0, -1.0],
            [3.0, 1.0, 1.0],
        )
        .expect("the segment crosses the box");
        let by_reciprocal = 3.0_f32 * (1.0_f32 / 7.0);
        assert_ne!(by_reciprocal.to_bits(), (3.0_f32 / 7.0).to_bits());
        assert_eq!(tmax.to_bits(), by_reciprocal.to_bits());
    }

    #[test]
    fn refine_edge_45_degree_facing_is_accepted() {
        // A grid-diagonal segment faces `s` at exactly 135 degrees: its normalised direction dotted
        // with `s`'s is `-0.70710677`, the `f32` value of `COS_135`, and `<=` accepts it (ruling R12).
        let s = seg([0.0, 0.0, 0.0], [4.0, 0.0, 0.0]);
        let diag = seg([3.0, 0.0, -0.5], [2.0, 0.0, -1.5]);
        let out = refine_edge(&s, 1.8, 1.0, &[s, diag], 0.1);
        assert_eq!(out.len(), 1, "{out:?}");
        // A slightly less opposed direction (dot above the limit) is rejected.
        let steep = seg([3.0, 0.0, -0.5], [2.0, 0.0, -1.5 - 0.1]);
        assert_eq!(refine_edge(&s, 1.8, 1.0, &[s, steep], 0.1).len(), 0);
        assert_eq!(COS_135.to_bits(), 0xBF35_04F3);
    }

    /// Values for which `dir*t + (n*h + A)` and `(dir*t + n*h) + A` differ in `f32` (found by a search; such edges
    /// are not common).
    fn bug_one_edge() -> (Seg, f32, f32) {
        (seg([2.59, 10.3, 8.71], [3.2, 10.3, 10.1]), 1.2, 0.5)
    }

    /// `v / |v|` and `v * (1 / |v|)` differ for some `v`; the shipped code multiplies (R16, #120, #121).
    /// For (-4.06, 0, -0.67) the quotient has x = -0.98665547 and the product -0.9866554.
    #[test]
    fn normalize_multiplies_by_the_reciprocal() {
        let v = [-4.06_f32, 0.0, -0.67];
        let n = normalize3(v);
        let l = len(v);
        assert_eq!(n[0].to_bits(), 0xbf7c_9573);
        assert_eq!((v[0] / l).to_bits(), 0xbf7c_9574);
        assert_eq!(n[2].to_bits(), 0xbe26_baee);
    }

    /// With the reciprocal-normalised `n`, the literal `(n*(-h) + B) - S0` is the direction. Found by
    /// a search over random edges: here pass 0 gives `dir.z` = -0.28369141, pass 1 -0.28417969, and the
    /// difference of the end points is -0.28393555. So the two passes do not share `dir`, and the spans of
    /// a pair are not exact mirrors (#120). Inputs are bit patterns of their exact values.
    #[test]
    fn dir_is_the_difference_of_the_sums_and_differs_between_the_passes() {
        let v = f32::from_bits;
        let edge = seg(
            [v(0x45e7_166f), 10.0, v(0x452b_587f)],
            [v(0x45e7_00fc), 10.0, v(0x452b_53f4)],
        );
        let h = v(0x4007_0a3d);
        let g0 = pass_geometry(&edge, 1.0, h, 0.9);
        let g1 = pass_geometry(&edge, -1.0, h, 0.9);
        let exact = sub(edge.p1, edge.p0);
        assert_eq!(g0.dir[2].to_bits(), 0xbe91_4000);
        assert_eq!(g1.dir[2].to_bits(), 0xbe91_8000);
        assert_ne!(g0.dir[2].to_bits(), exact[2].to_bits());
        assert_ne!(g1.dir[2].to_bits(), exact[2].to_bits());
        assert_eq!(g0.dir_len.to_bits(), len(g0.dir).to_bits());
        assert_eq!(g0.s0, add(scale(g0.n, -h), edge.p0));
    }

    /// The two spans of one edge are mirrors (`s` of one is `e` of the other) but, because `dir`
    /// differs between the passes, not exact ones: the reverse merge sees different lengths and keeps
    /// the longer span, and its radius follows that span (#120).
    #[test]
    fn passes_give_spans_of_different_length_and_the_longer_is_kept() {
        let v = f32::from_bits;
        let edge = seg(
            [v(0x45e7_166f), 10.0, v(0x452b_587f)],
            [v(0x45e7_00fc), 10.0, v(0x452b_53f4)],
        );
        let h = v(0x4007_0a3d);
        let span = |side: f32| {
            let g = pass_geometry(&edge, side, h, 0.9);
            LinkSpan {
                s0: g.p0,
                s1: add(g.p0, g.dir),
                e0: g.p1,
                e1: add(g.p1, g.dir),
                center_h: 0.5,
                validity: 1,
                part: None,
            }
        };
        let mut spans = [span(1.0), span(-1.0)];
        let (la, lb) = (
            dist_sq(spans[0].s1, spans[0].s0),
            dist_sq(spans[1].s1, spans[1].s0),
        );
        assert_ne!(la.to_bits(), lb.to_bits());
        reverse_merge(&mut spans, 0.5);
        let kept = usize::from(la <= lb);
        assert_eq!(spans[kept].validity, 2);
        assert_eq!(spans[1 - kept].validity, 0);
    }

    #[test]
    fn ebase_computed_once() {
        let (edge, h, t) = bug_one_edge();
        let g = pass_geometry(&edge, 1.0, h, 0.9);
        // Ebase = n*h + A, computed once.
        assert_eq!(g.ebase, add(scale(g.n, h), edge.p0));
        let (_, e, _) = sample_at(&g, t);
        let good = add(scale(g.dir, t), g.ebase);
        let bad = add(add(scale(g.dir, t), scale(g.n, h)), edge.p0);
        // The test values are chosen so the two orders round differently...
        assert_ne!(good.map(f32::to_bits), bad.map(f32::to_bits));
        // ...and the sample uses the first.
        assert_eq!(e.map(f32::to_bits), good.map(f32::to_bits));
    }

    #[test]
    fn ebase_order_changes_the_cell_lookup() {
        // Found by a search: here the two association orders land `E.x` in different cells of a
        // 0.25 m grid (25 against 24), so the floor lookup of the sample depends on the order.
        let edge = seg(
            [4.917_223_5, 10.3, 5.542_892_5],
            [6.168_562, 10.3, 6.794_231_4],
        );
        let (h, t, cs) = (1.0_f32, 0.5_f32, 0.25_f32);
        let g = pass_geometry(&edge, 1.0, h, 0.9);
        let (_, e, _) = sample_at(&g, t);
        let bad = add(add(scale(g.dir, t), scale(g.n, h)), edge.p0);
        assert_eq!(floor_i32(e[0] / cs), 25);
        assert_eq!(floor_i32(bad[0] / cs), 24);
    }

    #[test]
    fn pass_geometry_sides() {
        let edge = seg([4.5, 1.0, 2.0], [4.5, 1.0, 8.0]);
        let g0 = pass_geometry(&edge, 1.0, 1.2, 0.9);
        assert!(close3(g0.n, [1.0, 0.0, 0.0]));
        assert!(close3(g0.s0, [3.3, 1.0, 2.0]));
        assert!(close3(g0.p0, [3.6, 1.0, 2.0]));
        assert!(close3(g0.p1, [5.4, 1.0, 2.0]));
        assert!(close3(g0.ebase, [5.7, 1.0, 2.0]));
        assert!(close3(g0.dir, [0.0, 0.0, 6.0]));
        let g1 = pass_geometry(&edge, -1.0, 1.2, 0.9);
        assert!(close3(g1.n, [-1.0, 0.0, 0.0]));
        assert!(close3(g1.p0, [5.4, 1.0, 2.0]));
        assert!(close3(g1.dir, [0.0, 0.0, 6.0]));
    }

    #[test]
    fn walk_keeps_last_between_steps() {
        // Floor only in cell columns 0..=15 (x < 4.0 m); beyond them the solid columns are empty. The
        // sample starts on the floor and `last` keeps the floor's top (10) over the empty columns.
        let (hf, chf) = field(|x| (x <= 15).then_some(10), |_| None, 1);
        let class = synthetic_class();
        let ctx = ClassCtx::new(&chf, &hf, &class, &[]).unwrap().unwrap();
        let s = [3.3, 1.1, 5.0];
        let e = [5.7, 1.1, 5.0];
        let w = walk_trajectory(&ctx, s, e, 1.1, 1.1);
        assert_eq!(w.fail, None);
        // Reset to 0 on every step would give lo_u == 0.
        assert_eq!((w.lo_u, w.hi_u), (10, 10));
        // Before any floor `last` starts at 0: a start outside the solid field.
        let w = walk_trajectory(&ctx, [-5.0, 1.1, 5.0], [-4.0, 1.1, 5.0], 1.1, 1.1);
        assert_eq!((w.lo_u, w.hi_u), (0, 0));
    }

    #[test]
    fn walk_blocked_and_water() {
        // A tall wall (top 4.0 m) in cell columns 19 and 20 blocks the jump.
        let (hf, chf) = field(|_| Some(10), |x| (x == 19 || x == 20).then_some(40), 1);
        let class = synthetic_class();
        let ctx = ClassCtx::new(&chf, &hf, &class, &[]).unwrap().unwrap();
        let w = walk_trajectory(&ctx, [3.3, 1.1, 5.0], [5.7, 1.1, 5.0], 1.1, 1.1);
        assert_eq!(w.fail, Some(1));
        // Water (area 3) under the first sample: state 2.
        let (hf, chf) = field(|_| Some(10), |_| None, 3);
        let ctx = ClassCtx::new(&chf, &hf, &class, &[]).unwrap().unwrap();
        let w = walk_trajectory(&ctx, [3.3, 1.1, 5.0], [5.7, 1.1, 5.0], 1.1, 1.1);
        assert_eq!(w.fail, Some(2));
    }

    /// A 0.5 m wall (cells 17 and 18, centred on x = 4.5, top at 1.9 m) on a floor at 1.0 m.
    fn wall_field() -> (Heightfield, CompactHeightfield) {
        field(|_| Some(10), |x| (x == 17 || x == 18).then_some(19), 1)
    }

    /// The wall field with the part id `sub` on the wall's columns.
    fn hide_wall_field(sub: u16) -> (Heightfield, CompactHeightfield) {
        let (mut hf, chf) = wall_field();
        let width = usize::try_from(hf.width).unwrap();
        for (i, column) in hf.columns_mut().iter_mut().enumerate() {
            if matches!(i % width, 17 | 18) {
                for sp in column.iter_mut() {
                    sp.sub_area = sub;
                }
            }
        }
        (hf, chf)
    }

    /// Ruling R17: a link span over a hide part carries the part's index; other part ids and 0 do not
    /// count.
    #[test]
    fn spans_over_a_hide_part_carry_it() {
        let (hf, chf) = hide_wall_field(3);
        let class = synthetic_class();
        let edge = seg([4.5, 1.0, 2.0], [4.5, 1.0, 8.0]);
        let hide = [false, false, false, true];
        let ctx = ClassCtx::new(&chf, &hf, &class, &hide).unwrap().unwrap();
        let spans = class_spans(&ctx, &[edge]).unwrap();
        assert_eq!(spans.len(), 2);
        assert!(spans.iter().all(|s| s.part == Some(3)), "{spans:?}");
        // Part 3 is a door part: not a hide part.
        let no_hide = [false; 4];
        let ctx = ClassCtx::new(&chf, &hf, &class, &no_hide).unwrap().unwrap();
        assert!(
            class_spans(&ctx, &[edge])
                .unwrap()
                .iter()
                .all(|s| s.part.is_none())
        );
        // The part list of the tile is shorter than the id: no part either, and no panic.
        let ctx = ClassCtx::new(&chf, &hf, &class, &[true]).unwrap().unwrap();
        assert!(
            class_spans(&ctx, &[edge])
                .unwrap()
                .iter()
                .all(|s| s.part.is_none())
        );
        // Id 0 is no part, even when part 0 is a hide part.
        let (hf0, chf0) = hide_wall_field(0);
        let ctx = ClassCtx::new(&chf0, &hf0, &class, &[true, true])
            .unwrap()
            .unwrap();
        assert!(
            class_spans(&ctx, &[edge])
                .unwrap()
                .iter()
                .all(|s| s.part.is_none())
        );
    }

    /// Ruling R17: a hide part that fewer than a quarter of the samples of a run meet gives no part.
    /// Tree spans (area 10) carry a tree index, not a part.
    #[test]
    fn a_hide_part_needs_a_quarter_of_the_samples_and_no_trees_count() {
        let class = synthetic_class();
        let edge = seg([4.5, 1.0, 2.0], [4.5, 1.0, 8.0]);
        let hide = [false, false, false, true];
        // The part id on the wall's columns for the z cells `zs` only; the run has 24 or so samples.
        let spans_with = |zs: std::ops::Range<usize>| {
            let (mut hf, chf) = wall_field();
            let width = usize::try_from(hf.width).unwrap();
            for (i, column) in hf.columns_mut().iter_mut().enumerate() {
                if matches!(i % width, 17 | 18) && zs.contains(&(i / width)) {
                    for sp in column.iter_mut() {
                        sp.sub_area = 3;
                    }
                }
            }
            let ctx = ClassCtx::new(&chf, &hf, &class, &hide).unwrap().unwrap();
            class_spans(&ctx, &[edge]).unwrap()
        };
        // Four of 24 cells (one sixth): no part. Twelve of 24 (a half): the part.
        assert!(spans_with(18..22).iter().all(|s| s.part.is_none()));
        let half = spans_with(16..28);
        assert_eq!(half.len(), 2);
        assert!(half.iter().all(|s| s.part == Some(3)), "{half:?}");
        // The same ids on spans of the tree area do not count either.
        let (mut hf, chf) = hide_wall_field(3);
        for column in hf.columns_mut() {
            for sp in column.iter_mut().filter(|sp| sp.sub_area == 3) {
                sp.area = TREE_AREA;
            }
        }
        let ctx = ClassCtx::new(&chf, &hf, &class, &hide).unwrap().unwrap();
        assert!(
            class_spans(&ctx, &[edge])
                .unwrap()
                .iter()
                .all(|s| s.part.is_none())
        );
    }

    fn visits(list: &[(u16, u32)]) -> Visits {
        list.iter().copied().collect()
    }

    fn run_of(walks: &[Visits]) -> Option<u16> {
        let mut run = RunVisits::default();
        for w in walks {
            run.add(w);
        }
        run.part()
    }

    #[test]
    fn the_run_part_needs_a_quarter_of_the_samples() {
        let met = visits(&[(4, 2), (9, 1)]);
        let none = Visits::new();
        assert_eq!(run_of(&[]), None);
        assert_eq!(run_of(&[none.clone(), none.clone()]), None);
        // 1 of 4 samples: just enough. 1 of 5: too few.
        assert_eq!(
            run_of(&[met.clone(), none.clone(), none.clone(), none.clone()]),
            Some(4)
        );
        assert_eq!(
            run_of(&[met, none.clone(), none.clone(), none.clone(), none]),
            None
        );
    }

    /// The span that ends a column's walk, the first one above the trajectory window, does not count.
    #[test]
    fn a_hide_part_above_the_window_does_not_count() {
        let (hf, chf) = hide_wall_field(1);
        let class = synthetic_class();
        let ctx = ClassCtx::new(&chf, &hf, &class, &[false, true])
            .unwrap()
            .unwrap();
        // The jump clears the wall (top 1.9 m): its spans lie below the window and are visited.
        let (s, e) = ([3.3, 1.1, 5.0], [5.7, 1.1, 5.0]);
        let below = walk_trajectory(&ctx, s, e, 1.1, 1.1);
        assert_eq!(below.fail, None);
        assert!(
            below.parts.keys().all(|p| *p == 1) && !below.parts.is_empty(),
            "{below:?}"
        );
        // The same part raised well above the window (6.0 to 7.0 m): it ends the walk of its
        // columns instead of being visited.
        let mut raised = hf.clone();
        for column in raised.columns_mut() {
            for sp in column.iter_mut().filter(|sp| sp.sub_area == 1) {
                sp.smin = 60;
                sp.smax = 70;
            }
        }
        let ctx = ClassCtx::new(&chf, &raised, &class, &[false, true])
            .unwrap()
            .unwrap();
        let above = walk_trajectory(&ctx, s, e, 1.1, 1.1);
        assert_eq!(above.fail, None);
        assert!(above.parts.is_empty(), "{above:?}");
    }

    #[test]
    fn the_dominant_part_is_the_most_visited_and_the_lowest_on_a_tie() {
        // 7 seen 3 times in total, 5 twice.
        assert_eq!(
            run_of(&[visits(&[(5, 1), (7, 2)]), visits(&[(5, 1), (7, 1)])]),
            Some(7)
        );
        // A tie goes to the lower id, whatever the order.
        assert_eq!(run_of(&[visits(&[(9, 2), (4, 2)])]), Some(4));
        assert_eq!(run_of(&[visits(&[(4, 2), (9, 2)])]), Some(4));
        // Counts add up across walks and cannot overflow.
        assert_eq!(
            run_of(&[
                visits(&[(1, u32::MAX)]),
                visits(&[(1, 5)]),
                visits(&[(2, u32::MAX)])
            ]),
            Some(1)
        );
    }

    /// Many hide parts: the totals of a run hold one entry per part met, not per visit, and the
    /// result does not depend on how many there are.
    #[test]
    fn many_hide_parts_stay_bounded() {
        let mut run = RunVisits::default();
        for round in 0..3_u32 {
            let walk: Visits = (1..=60_000_u16).map(|p| (p, 1 + round)).collect();
            run.add(&walk);
        }
        assert_eq!(run.total.len(), 60_000);
        // All equal: the lowest id.
        assert_eq!(run.part(), Some(1));
    }

    /// Spec 6.4: a reverse merge does not move parts; each span keeps its own.
    #[test]
    fn reverse_merge_keeps_the_parts() {
        let mut a = span(
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 0.0, 1.8],
            [2.0, 0.0, 1.8],
            0.5,
        );
        let mut b = span(
            [0.0, 0.0, 1.8],
            [2.0, 0.0, 1.8],
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            0.6,
        );
        a.part = Some(3);
        b.part = Some(5);
        let mut spans = [a, b];
        reverse_merge(&mut spans, 0.1);
        // Equal lengths: a is merged away, b is bidirectional; neither takes the other's part.
        assert_eq!(spans.map(|s| s.validity), [0, 2]);
        assert_eq!(spans.map(|s| s.part), [Some(3), Some(5)]);
        // The longer span a stays and takes b's centre height, not its part.
        let mut long_a = a;
        long_a.s1 = [2.4, 0.0, 0.0];
        long_a.e1 = [2.4, 0.0, 1.8];
        let mut spans = [long_a, b];
        reverse_merge(&mut spans, 0.1);
        assert_eq!(spans.map(|s| s.validity), [2, 0]);
        assert!(close(spans[0].center_h, 0.6));
        assert_eq!(spans.map(|s| s.part), [Some(3), Some(5)]);
    }

    #[test]
    fn both_passes_give_spans_in_order() {
        let (hf, chf) = wall_field();
        let class = synthetic_class();
        let ctx = ClassCtx::new(&chf, &hf, &class, &[]).unwrap().unwrap();
        // The edge runs along the middle of the wall.
        let edge = seg([4.5, 1.0, 2.0], [4.5, 1.0, 8.0]);
        let mut spans = class_spans(&ctx, &[edge]).unwrap();
        assert_eq!(spans.len(), 2, "{spans:?}");
        // Pass 0 jumps along +n (east), pass 1 along -n (west).
        assert!(close(spans[0].s0[0], 3.6), "{:?}", spans[0]);
        assert!(close(spans[0].e0[0], 5.4), "{:?}", spans[0]);
        assert!(close(spans[1].s0[0], 5.4), "{:?}", spans[1]);
        assert!(close(spans[1].e0[0], 3.6), "{:?}", spans[1]);
        // The measured obstacle height: the wall top (2.0 m) over the start at 1.0 m.
        assert!(close(spans[0].center_h, 1.0), "{:?}", spans[0]);
        assert_eq!((spans[0].validity, spans[1].validity), (1, 1));
        // The span ends are on the floor, y snapped to its top.
        assert!(close(spans[0].s0[1], 1.0) && close(spans[0].e0[1], 1.0));
        // Run extraction: the run covers the whole 6 m edge.
        assert!(close(spans[0].s0[2], 2.0), "{:?}", spans[0]);
        assert!(close(spans[0].s1[2], 8.0), "{:?}", spans[0]);
        // The two spans are reverses of each other: one bidirectional span after the merge.
        reverse_merge(&mut spans, 0.25);
        assert_eq!((spans[0].validity, spans[1].validity), (0, 2));
    }

    #[test]
    fn short_runs_are_dropped() {
        let (hf, chf) = wall_field();
        let class = synthetic_class();
        let ctx = ClassCtx::new(&chf, &hf, &class, &[]).unwrap().unwrap();
        // minRun = ceil(0.6 / 0.25) = 3: an edge of 0.5 m (nS = 2) has no span.
        let edge = seg([4.5, 1.0, 2.0], [4.5, 1.0, 2.5]);
        assert_eq!(class_spans(&ctx, &[edge]).unwrap().len(), 0);
    }

    fn span(s0: V, s1: V, e0: V, e1: V, center_h: f32) -> LinkSpan {
        LinkSpan {
            s0,
            s1,
            e0,
            e1,
            center_h,
            validity: 1,
            part: None,
        }
    }

    #[test]
    fn reverse_merge_literal() {
        let a = span(
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 0.0, 1.8],
            [2.0, 0.0, 1.8],
            0.5,
        );
        // B is the reverse of A.
        let b = span(
            [0.0, 0.0, 1.8],
            [2.0, 0.0, 1.8],
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            0.6,
        );
        // C is the reverse of B but longer: a validity-2 span matches again as `a`.
        let c_long = span(
            [0.0, 0.0, 0.0],
            [2.3, 0.0, 0.0],
            [0.0, 0.0, 1.8],
            [2.3, 0.0, 1.8],
            0.7,
        );
        let mut spans = [a, b, c_long];
        reverse_merge(&mut spans, 0.1);
        // A matches B (equal lengths: a <= b): A is 0, B is 2. Then B as `a` matches C (B shorter): B
        // is 0, C is 2. A is not matched with C because it stopped at B.
        assert_eq!(spans.map(|s| s.validity), [0, 0, 2]);
        // C shorter than B: the literal rule keeps B (2) and drops C, and B takes C's centre height.
        let c_short = span(
            [0.0, 0.0, 0.0],
            [1.8, 0.0, 0.0],
            [0.0, 0.0, 1.8],
            [1.8, 0.0, 1.8],
            0.7,
        );
        let mut spans = [a, b, c_short];
        reverse_merge(&mut spans, 0.1);
        assert_eq!(spans.map(|s| s.validity), [0, 2, 0]);
        assert!(close(spans[1].center_h, 0.7));
        // The centre height only moves in the `a > b` case: with a longer first span it moves from b.
        let mut long_a = a;
        long_a.s1 = [2.4, 0.0, 0.0];
        long_a.e1 = [2.4, 0.0, 1.8];
        let mut spans = [long_a, b];
        reverse_merge(&mut spans, 0.1);
        assert_eq!(spans.map(|s| s.validity), [2, 0]);
        assert!(close(spans[0].center_h, 0.6));
        // Spans farther apart than 5 cells are left alone.
        let mut far = b;
        far.s0 = [0.0, 0.0, 9.0];
        let mut spans = [a, far];
        reverse_merge(&mut spans, 0.1);
        assert_eq!(spans.map(|s| s.validity), [1, 1]);
    }

    #[test]
    fn emit_count_and_spacing() {
        let class = synthetic_class();
        let s = span(
            [0.0, 0.0, 0.0],
            [7.0, 0.0, 0.0],
            [0.0, 0.0, 1.8],
            [7.0, 0.0, 1.8],
            0.8,
        );
        let mut out = Links::default();
        emit(&s, &class, 0.3, &mut out).unwrap();
        let out = out.off_mesh;
        // rem = 7 - 1.2, count = 1 - (int)(rem * -0.5) = 3, centred and 2 m apart.
        assert_eq!(out.len(), 3);
        let xs: Vec<f32> = out.iter().map(|c| c.start[0]).collect();
        assert!(
            close(xs[0], 1.5) && close(xs[1], 3.5) && close(xs[2], 5.5),
            "{xs:?}"
        );
        for c in &out {
            assert!(close(c.end[0], c.start[0]) && close(c.end[2], 1.8));
            assert!(close(c.radius, 1.0) && close(c.center_height, 0.8));
            assert_eq!((c.flags, c.area, c.user_id), (0x100, 48, 0));
            assert!(!c.bidirectional);
        }
        // A bidirectional span, and a short one: one link with radius `off`.
        let mut one = span(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.8],
            [1.0, 0.0, 1.8],
            0.8,
        );
        one.validity = 2;
        let mut class = class;
        class.index = 5;
        let mut out = Links::default();
        emit(&one, &class, 0.3, &mut out).unwrap();
        let out = out.off_mesh;
        assert_eq!(out.len(), 1);
        assert!(close(out[0].radius, 0.5) && close(out[0].start[0], 0.5));
        assert!(out[0].bidirectional);
        assert_eq!(out[0].user_id, 5);
        // The count is bounded.
        let huge = span(
            [0.0, 0.0, 0.0],
            [1.0e12, 0.0, 0.0],
            [0.0, 0.0, 1.8],
            [1.0e12, 0.0, 1.8],
            0.8,
        );
        assert!(emit(&huge, &class, 0.3, &mut Links::default()).is_err());
    }

    #[test]
    fn ladders_come_first() {
        let g = TileGroup {
            area: 15,
            verts: vec![
                [1.0, 2.0, 3.0],
                [1.0, 5.0, 3.0],
                [9.0, 9.0, 9.0], // an odd vertex without a partner
            ],
            ..TileGroup::default()
        };
        let mut out = Links::default();
        ladder_links(&[&g], &mut out);
        assert_eq!(out.parts, [None]);
        let out = out.off_mesh;
        assert_eq!(out.len(), 1);
        let l = &out[0];
        assert_eq!((l.start, l.end), ([1.0, 2.0, 3.0], [1.0, 5.0, 3.0]));
        assert_eq!((l.flags, l.area, l.user_id), (0x40, 15, 0));
        assert!(l.bidirectional && close(l.radius, 1.0) && close(l.center_height, 0.0));
    }
}
