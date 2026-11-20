//! The island filter (spec 8.5): flags every polygon that cannot be reached from a seed position
//! with the "isolated" bit `0x8000`.

use std::collections::BTreeMap;

use crate::navmesh::detour::{NavMesh, decode_poly_ref, find_nearest_poly};

/// `DT_NULL_LINK`.
const NULL_LINK: u32 = 0xffff_ffff;
/// The polygon flag of the island filter.
const FLAG_ISOLATED: u16 = 0x8000;
/// The polygon flag of a disabled polygon (a closed door).
const FLAG_DISABLED: u16 = 0x0002;
/// The search box around the seed (`x`, `y`, `z` half extents).
const SEED_EXTENTS: [f32; 3] = [30.0, 30.0, 3.0];

/// What [`filter_islands`] found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct IslandReport {
    /// Polygons reachable from the seed.
    pub reachable: usize,
    /// Polygons flagged `0x8000`: every other polygon of every tile, off-mesh polygons included.
    pub isolated: usize,
}

/// Runs the island filter on `mesh`.
///
/// Clears `0x8000` on every polygon, finds the polygon nearest to `seed` (search box
/// `(30, 30, 3)`, include flags `0xffff`, exclude flags `0x8002`) and floods the mesh from it
/// depth-first over every link, ignoring flags (so closed doors are traversed) and following
/// one-way off-mesh links in their direction only. Every polygon not visited, off-mesh polygons
/// included, gets `0x8000`.
///
/// Returns `None` when no polygon is found near the seed. The flags are cleared in that case
/// too, and nothing is flagged.
pub fn filter_islands(mesh: &mut NavMesh, seed: [f32; 3]) -> Option<IslandReport> {
    for (_, polys) in mesh.polys_mut() {
        for p in polys {
            p.flags &= !FLAG_ISOLATED;
        }
    }
    let (start, _) = find_nearest_poly(
        mesh,
        seed,
        SEED_EXTENTS,
        0xffff,
        FLAG_ISOLATED | FLAG_DISABLED,
    )?;

    // Visited polygons per tile slot. Marked when pushed, so each polygon is on the stack once.
    let mut visited: BTreeMap<u32, Vec<bool>> = mesh
        .tiles()
        .map(|(r, t)| (decode_poly_ref(r).1, vec![false; t.polys.len()]))
        .collect();
    let mark = |visited: &mut BTreeMap<u32, Vec<bool>>, slot: u32, poly: usize| -> bool {
        match visited.get_mut(&slot).and_then(|v| v.get_mut(poly)) {
            Some(flag) if !*flag => {
                *flag = true;
                true
            }
            _ => false,
        }
    };
    let mut reachable = 0_usize;
    let mut stack = Vec::new();
    {
        let (_, slot, poly) = decode_poly_ref(start);
        if let Ok(poly) = usize::try_from(poly)
            && mark(&mut visited, slot, poly)
        {
            stack.push(start);
        }
    }
    while let Some(r) = stack.pop() {
        reachable += 1;
        let Some((_, tile)) = mesh.tile_by_ref(r) else {
            continue;
        };
        let (_, _, poly) = decode_poly_ref(r);
        let Some(p) = usize::try_from(poly).ok().and_then(|i| tile.polys.get(i)) else {
            continue;
        };
        let mut at = p.first_link;
        // A hostile link list may loop: a chain cannot be longer than the link array.
        let mut budget = tile.links.len();
        while at != NULL_LINK && budget > 0 {
            budget -= 1;
            let Some(link) = usize::try_from(at).ok().and_then(|i| tile.links.get(i)) else {
                break;
            };
            at = link.next;
            if let Some((slot, _)) = mesh.tile_by_ref(link.reference) {
                let (_, _, npoly) = decode_poly_ref(link.reference);
                if let Ok(npoly) = usize::try_from(npoly)
                    && mark(&mut visited, slot, npoly)
                {
                    stack.push(link.reference);
                }
            }
        }
    }

    let mut isolated = 0_usize;
    for (slot, polys) in mesh.polys_mut() {
        let row = visited.get(&slot);
        for (i, p) in polys.iter_mut().enumerate() {
            if !row.and_then(|v| v.get(i)).copied().unwrap_or(false) {
                p.flags |= FLAG_ISOLATED;
                isolated += 1;
            }
        }
    }
    Some(IslandReport {
        reachable,
        isolated,
    })
}
