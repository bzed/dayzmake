//
// Copyright (c) 2009-2010 Mikko Mononen memon@inside.org
//
// This software is provided 'as-is', without any express or implied
// warranty.  In no event will the authors be held liable for any damages
// arising from the use of this software.
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
// 1. The origin of this software must not be misrepresented; you must not
//    claim that you wrote the original software. If you use this software
//    in a product, an acknowledgment in the product documentation would be
//    appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
//    misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Ported to Rust and altered for DayZ (see the dayzmake navmesh spec); this is not the original software.
//

//! Port of the `DetourCommon` helpers the tile builder and the tile linker need.
//!
//! Vectors are `[f32; 3]`; the arithmetic keeps the operation order of the C++ original.

#![expect(
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "the names of the upstream functions are kept for comparison"
)]
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "helpers for the tile linking and query tasks that follow"
    )
)]

/// `dtVsub`.
pub(crate) fn vsub(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `dtVadd`.
pub(crate) fn vadd(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// `dtVmad`: `a + b * s`.
pub(crate) fn vmad(a: &[f32; 3], b: &[f32; 3], s: f32) -> [f32; 3] {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

/// `dtVlerp`: `a + (b - a) * t`.
pub(crate) fn vlerp(a: &[f32; 3], b: &[f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// `dtVdistSqr`.
pub(crate) fn vdist_sqr(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let dz = b[2] - a[2];
    dx * dx + dy * dy + dz * dz
}

/// `dtVdist`.
pub(crate) fn vdist(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    vdist_sqr(a, b).sqrt()
}

/// `dtVdist2DSqr`.
pub(crate) fn vdist_2d_sqr(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let dx = b[0] - a[0];
    let dz = b[2] - a[2];
    dx * dx + dz * dz
}

/// `dtVdist2D`.
pub(crate) fn vdist_2d(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    vdist_2d_sqr(a, b).sqrt()
}

/// `dtVdot2D`.
pub(crate) fn vdot_2d(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[0] * b[0] + a[2] * b[2]
}

/// `dtVperp2D`.
pub(crate) fn vperp_2d(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[2] * b[0] - a[0] * b[2]
}

/// `dtTriArea2D`: twice the signed area of the triangle projected onto the xz plane.
pub(crate) fn tri_area_2d(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3]) -> f32 {
    let abx = b[0] - a[0];
    let abz = b[2] - a[2];
    let acx = c[0] - a[0];
    let acz = c[2] - a[2];
    acx * abz - abx * acz
}

/// `dtOverlapQuantBounds`.
pub(crate) fn overlap_quant_bounds(
    amin: [u16; 3],
    amax: [u16; 3],
    bmin: [u16; 3],
    bmax: [u16; 3],
) -> bool {
    !(amin[0] > bmax[0]
        || amax[0] < bmin[0]
        || amin[1] > bmax[1]
        || amax[1] < bmin[1]
        || amin[2] > bmax[2]
        || amax[2] < bmin[2])
}

/// `dtOverlapBounds`.
pub(crate) fn overlap_bounds(
    amin: &[f32; 3],
    amax: &[f32; 3],
    bmin: &[f32; 3],
    bmax: &[f32; 3],
) -> bool {
    !(amin[0] > bmax[0]
        || amax[0] < bmin[0]
        || amin[1] > bmax[1]
        || amax[1] < bmin[1]
        || amin[2] > bmax[2]
        || amax[2] < bmin[2])
}

/// `dtOppositeTile`.
pub(crate) fn opposite_tile(side: i32) -> i32 {
    (side + 4) & 7
}

/// `dtDistancePtSegSqr2D`: the squared xz distance from `pt` to the segment `p`-`q`, and the
/// parameter `t` of the closest point.
pub(crate) fn distance_pt_seg_sqr_2d(pt: &[f32; 3], p: &[f32; 3], q: &[f32; 3]) -> (f32, f32) {
    let pqx = q[0] - p[0];
    let pqz = q[2] - p[2];
    let mut dx = pt[0] - p[0];
    let mut dz = pt[2] - p[2];
    let d = pqx * pqx + pqz * pqz;
    let mut t = pqx * dx + pqz * dz;
    if d > 0.0 {
        t /= d;
    }
    t = t.clamp(0.0, 1.0);
    dx = p[0] + t * pqx - pt[0];
    dz = p[2] + t * pqz - pt[2];
    (dx * dx + dz * dz, t)
}

/// The even-odd test shared by `dtPointInPolygon` and `dtDistancePtPolyEdgesSqr`.
fn crossing(pt: &[f32; 3], vi: &[f32; 3], vj: &[f32; 3]) -> bool {
    ((vi[2] > pt[2]) != (vj[2] > pt[2]))
        && (pt[0] < (vj[0] - vi[0]) * (pt[2] - vi[2]) / (vj[2] - vi[2]) + vi[0])
}

/// `dtPointInPolygon`: the point is tested in the xz plane.
pub(crate) fn point_in_polygon(pt: &[f32; 3], verts: &[[f32; 3]]) -> bool {
    let mut c = false;
    let mut prev = verts.last();
    for vi in verts {
        if let Some(vj) = prev
            && crossing(pt, vi, vj)
        {
            c = !c;
        }
        prev = Some(vi);
    }
    c
}

/// `dtDistancePtPolyEdgesSqr`: whether `pt` is inside the polygon (xz), and for every edge `j`
/// (from vertex `j - 1` to `j`, stored at the index of the first vertex as upstream does) the
/// squared distance `ed[j]` and parameter `et[j]`.
///
/// `ed` and `et` must have at least `verts.len()` entries; extra entries are left alone and a
/// shorter slice stops the loop.
pub(crate) fn distance_pt_poly_edges_sqr(
    pt: &[f32; 3],
    verts: &[[f32; 3]],
    ed: &mut [f32],
    et: &mut [f32],
) -> bool {
    let mut c = false;
    let n = verts.len();
    for (i, vi) in verts.iter().enumerate() {
        let j = if i == 0 { n.saturating_sub(1) } else { i - 1 };
        let Some(vj) = verts.get(j) else { break };
        if crossing(pt, vi, vj) {
            c = !c;
        }
        let (d, t) = distance_pt_seg_sqr_2d(pt, vj, vi);
        let (Some(e), Some(s)) = (ed.get_mut(j), et.get_mut(j)) else {
            break;
        };
        *e = d;
        *s = t;
    }
    c
}

/// The result of [`intersect_segment_poly_2d`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SegPolyHit {
    /// Entry parameter.
    pub tmin: f32,
    /// Exit parameter.
    pub tmax: f32,
    /// Edge the segment enters through, or -1.
    pub seg_min: i32,
    /// Edge the segment leaves through, or -1.
    pub seg_max: i32,
}

/// `dtIntersectSegmentPoly2D`: `None` when the segment misses the polygon.
pub(crate) fn intersect_segment_poly_2d(
    p0: &[f32; 3],
    p1: &[f32; 3],
    verts: &[[f32; 3]],
) -> Option<SegPolyHit> {
    const EPS: f32 = 0.000_001;
    let mut tmin = 0.0_f32;
    let mut tmax = 1.0_f32;
    let mut seg_min = -1_i32;
    let mut seg_max = -1_i32;
    let dir = vsub(p1, p0);
    let n = verts.len();
    for (i, vi) in verts.iter().enumerate() {
        let j = if i == 0 { n.saturating_sub(1) } else { i - 1 };
        let vj = verts.get(j)?;
        let edge = vsub(vi, vj);
        let diff = vsub(p0, vj);
        let nn = vperp_2d(&edge, &diff);
        let d = vperp_2d(&dir, &edge);
        if d.abs() < EPS {
            // The segment is nearly parallel to this edge.
            if nn < 0.0 {
                return None;
            }
            continue;
        }
        let t = nn / d;
        let ji = i32::try_from(j).ok()?;
        if d < 0.0 {
            // The segment enters across this edge.
            if t > tmin {
                tmin = t;
                seg_min = ji;
                if tmin > tmax {
                    return None;
                }
            }
        } else if t < tmax {
            // The segment leaves across this edge.
            tmax = t;
            seg_max = ji;
            if tmax < tmin {
                return None;
            }
        }
    }
    Some(SegPolyHit {
        tmin,
        tmax,
        seg_min,
        seg_max,
    })
}

/// `dtIntersectSegSeg2D`: the parameters `(s, t)` of the intersection of the lines `ap`-`aq` and
/// `bp`-`bq`, or `None` when they are parallel.
pub(crate) fn intersect_seg_seg_2d(
    ap: &[f32; 3],
    aq: &[f32; 3],
    bp: &[f32; 3],
    bq: &[f32; 3],
) -> Option<(f32, f32)> {
    let u = vsub(aq, ap);
    let v = vsub(bq, bp);
    let w = vsub(ap, bp);
    let d = u[0] * v[2] - u[2] * v[0];
    if d.abs() < 1e-6 {
        return None;
    }
    let s = (v[0] * w[2] - v[2] * w[0]) / d;
    let t = (u[0] * w[2] - u[2] * w[0]) / d;
    Some((s, t))
}

fn project_poly(axis: &[f32; 3], poly: &[[f32; 3]]) -> (f32, f32) {
    let mut it = poly.iter();
    let Some(first) = it.next() else {
        return (0.0, 0.0);
    };
    let mut rmin = vdot_2d(axis, first);
    let mut rmax = rmin;
    for v in it {
        let d = vdot_2d(axis, v);
        rmin = if rmin < d { rmin } else { d };
        rmax = if rmax > d { rmax } else { d };
    }
    (rmin, rmax)
}

fn overlap_range(amin: f32, amax: f32, bmin: f32, bmax: f32, eps: f32) -> bool {
    !((amin + eps) > bmax || (amax - eps) < bmin)
}

/// `dtOverlapPolyPoly2D`: separating axis test of two convex polygons in the xz plane.
pub(crate) fn overlap_poly_poly_2d(polya: &[[f32; 3]], polyb: &[[f32; 3]]) -> bool {
    const EPS: f32 = 1e-4;
    for poly in [polya, polyb] {
        let n = poly.len();
        for (i, vb) in poly.iter().enumerate() {
            let j = if i == 0 { n.saturating_sub(1) } else { i - 1 };
            let Some(va) = poly.get(j) else { continue };
            let axis = [vb[2] - va[2], 0.0, -(vb[0] - va[0])];
            let (amin, amax) = project_poly(&axis, polya);
            let (bmin, bmax) = project_poly(&axis, polyb);
            if !overlap_range(amin, amax, bmin, bmax, EPS) {
                return false;
            }
        }
    }
    true
}

/// `dtCalcPolyCenter` over the polygon's own vertices.
pub(crate) fn calc_poly_center(verts: &[[f32; 3]]) -> [f32; 3] {
    let mut tc = [0.0_f32; 3];
    for v in verts {
        tc[0] += v[0];
        tc[1] += v[1];
        tc[2] += v[2];
    }
    // `1.0f / nidx` with the count converted as C does.
    let s = 1.0 / crate::navmesh::num::i32_to_f32(i32::try_from(verts.len()).unwrap_or(i32::MAX));
    [tc[0] * s, tc[1] * s, tc[2] * s]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_distance_clamps() {
        let (d, t) = distance_pt_seg_sqr_2d(&[5.0, 0.0, 3.0], &[0.0; 3], &[10.0, 0.0, 0.0]);
        assert!((d - 9.0).abs() < 1e-6 && (t - 0.5).abs() < 1e-6);
        let (d, t) = distance_pt_seg_sqr_2d(&[-3.0, 0.0, 4.0], &[0.0; 3], &[10.0, 0.0, 0.0]);
        assert!((d - 25.0).abs() < 1e-6 && t.abs() < 1e-6);
    }

    const SQUARE: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 10.0],
        [10.0, 0.0, 10.0],
        [10.0, 0.0, 0.0],
    ];

    #[test]
    fn point_in_polygon_and_edges() {
        assert!(point_in_polygon(&[5.0, 0.0, 5.0], &SQUARE));
        assert!(!point_in_polygon(&[15.0, 0.0, 5.0], &SQUARE));
        let mut ed = [0.0; 4];
        let mut et = [0.0; 4];
        assert!(!distance_pt_poly_edges_sqr(
            &[15.0, 0.0, 5.0],
            &SQUARE,
            &mut ed,
            &mut et
        ));
        // Edge 2 runs from vertex 2 to 3, at x = 10; the point is 5 away.
        assert!((ed[2] - 25.0).abs() < 1e-6);
        assert!((et[2] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn segment_through_polygon() {
        let hit = intersect_segment_poly_2d(&[-5.0, 0.0, 5.0], &[15.0, 0.0, 5.0], &SQUARE)
            .expect("crosses");
        assert!((hit.tmin - 0.25).abs() < 1e-6 && (hit.tmax - 0.75).abs() < 1e-6);
        assert!(
            intersect_segment_poly_2d(&[-5.0, 0.0, 20.0], &[15.0, 0.0, 20.0], &SQUARE).is_none()
        );
    }

    #[test]
    fn seg_seg_and_overlap() {
        let (s, t) = intersect_seg_seg_2d(
            &[0.0; 3],
            &[10.0, 0.0, 10.0],
            &[0.0, 0.0, 10.0],
            &[10.0, 0.0, 0.0],
        )
        .expect("cross");
        assert!((s - 0.5).abs() < 1e-6 && (t - 0.5).abs() < 1e-6);
        assert!(
            intersect_seg_seg_2d(
                &[0.0; 3],
                &[1.0, 0.0, 0.0],
                &[0.0, 0.0, 1.0],
                &[1.0, 0.0, 1.0]
            )
            .is_none()
        );
        let shifted: Vec<[f32; 3]> = SQUARE.iter().map(|v| [v[0] + 5.0, v[1], v[2]]).collect();
        let far: Vec<[f32; 3]> = SQUARE.iter().map(|v| [v[0] + 30.0, v[1], v[2]]).collect();
        assert!(overlap_poly_poly_2d(&SQUARE, &shifted));
        assert!(!overlap_poly_poly_2d(&SQUARE, &far));
    }

    #[test]
    fn bounds_and_vectors() {
        assert!(overlap_quant_bounds([0; 3], [5; 3], [5; 3], [9; 3]));
        assert!(!overlap_quant_bounds([0; 3], [4; 3], [5; 3], [9; 3]));
        assert!(overlap_bounds(&[0.0; 3], &[1.0; 3], &[1.0; 3], &[2.0; 3]));
        assert_eq!(opposite_tile(1), 5);
        assert_eq!(opposite_tile(6), 2);
        assert_eq!(vlerp(&[0.0; 3], &[2.0, 4.0, 6.0], 0.5), [1.0, 2.0, 3.0]);
        assert_eq!(vmad(&[1.0; 3], &[1.0, 2.0, 3.0], 2.0), [3.0, 5.0, 7.0]);
        assert_eq!(vadd(&[1.0; 3], &[1.0; 3]), [2.0; 3]);
        assert!((vdist(&[0.0; 3], &[3.0, 4.0, 0.0]) - 5.0).abs() < 1e-6);
        assert!((vdist_2d(&[0.0; 3], &[3.0, 9.0, 4.0]) - 5.0).abs() < 1e-6);
        assert!(tri_area_2d(&[0.0; 3], &[1.0, 0.0, 0.0], &[0.0, 0.0, 1.0]) < 0.0);
        let c = calc_poly_center(&SQUARE);
        assert_eq!(c, [5.0, 0.0, 5.0]);
    }
}
