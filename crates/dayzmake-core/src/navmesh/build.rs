//! Building one navmesh tile end to end (spec 4, 5, 7).
//!
//! [`tile_config`] derives the Recast configuration of a cell class, and [`build_tile`] runs every
//! step of spec 5.4 on one tile input: terrain, objects, span filters, compact heightfield, area
//! passes, regions, contours, polygon mesh, flags, the extra section and the Detour tile blob.
//!
//! The build raises no warnings: the counts it returns (stray part ids, obstacles) are for the
//! caller to report.

use std::io;

use super::dayz::{
    areas::{dilate_area, extract_tree_obstacles, remap_and_roof},
    filters::{filter_low_height_multi, mode_filter},
    slope::walkable,
};
use super::detour::{
    CreateParams, Obstacle, OffMeshInput, TileData, TileExtra, TilePart, TilePartPoly,
    create_tile_data,
};
use super::num::{ceil_i32, i32_to_f32, trunc_i32};
use super::recast::{
    compact::{CompactHeightfield, build_compact, erode, mark_box_area},
    contour::{RC_CONTOUR_TESS_WALL_EDGES, build_contours},
    filter::{filter_ledges, filter_low_hanging},
    heightfield::Heightfield,
    mesh::{PolyMesh, build_poly_mesh},
    rasterize::{RasterMode, RasterParams, rasterize_part, rasterize_triangle},
    region::{build_distance_field, build_regions},
};
use super::wire::{CellClass, TileGroup, TileInput, WorldParams};

/// What the door states of the tiles are (spec 5.1): which door parts start disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DoorPolicy {
    /// Open doors (area 12) are disabled, closed doors (area 13) are not.
    #[default]
    Closed,
    /// Closed doors (area 13) are disabled, open doors are not.
    Open,
    /// No door is disabled.
    Enabled,
}

/// Options of a tile build.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BuildOptions {
    /// Build jump, climb and ladder links (spec 6).
    pub links: bool,
    /// The initial state of door parts.
    pub doors: DoorPolicy,
    /// Sea level in metres; `None` uses the world's `deepSeaHeight`.
    pub sea_level: Option<f32>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            links: true,
            doors: DoorPolicy::Closed,
            sea_level: None,
        }
    }
}

/// The Recast configuration derived from the world and a cell class (spec 4). Sizes in voxels
/// unless noted.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct TileConfig {
    /// Cells across the tile (`tileSize`).
    pub cells: i32,
    /// Cell size in metres.
    pub cs: f32,
    /// Cell height in metres (0.1).
    pub ch: f32,
    /// `walkableHeight`.
    pub walkable_height: i32,
    /// Crouch height.
    pub crouch_height: i32,
    /// Prone height.
    pub prone_height: i32,
    /// `walkableClimb`.
    pub walkable_climb: i32,
    /// `walkableRadius`.
    pub walkable_radius: i32,
    /// Deep water depth.
    pub deep_water: i32,
    /// Sea level.
    pub sea_level: i32,
    /// Border size in cells.
    pub border_size: i32,
    /// Heightfield width and height: `cells + 2 * border_size`.
    pub width: i32,
    /// Padding in metres: `float(border_size) * cs`.
    pub pad: f32,
    /// Contour simplification error in cells.
    pub max_simplification_error: f32,
    /// Longest contour edge in cells (120).
    pub max_edge_len: i32,
    /// Vertical simplification error in voxels (8).
    pub max_vertical_error: f32,
    /// Default minimum region area.
    pub min_region_area: i32,
    /// Region merge area (400).
    pub merge_region_area: i32,
    /// Vertices per polygon (6).
    pub nvp: usize,
    /// Door dilation radius.
    pub door_radius: i32,
    /// Slope threshold: `cos(maxSlope)`.
    pub slope_threshold: f32,
}

/// The configuration of tiles of `cell_class` (spec 4). All arithmetic is in `f32` as the spec
/// writes it. A degenerate cell class (no cells) gives non-finite sizes, which [`build_tile`]
/// rejects.
#[must_use]
pub fn tile_config(
    world: &WorldParams,
    cell_class: &CellClass,
    options: &BuildOptions,
) -> TileConfig {
    let b = &world.world;
    let cs = cell_class.cell_size();
    let walkable_radius = ceil_i32((b.diameter * 0.5_f32) / cs);
    let border_size = 10.max(walkable_radius.saturating_sub(trunc_i32(-3.0_f32 / cs)));
    let sea = options.sea_level.unwrap_or(b.deep_sea_height);
    let min_region = trunc_i32((1.0_f32 / cs) * (1.0_f32 / cs));
    TileConfig {
        cells: cell_class.cells,
        cs,
        ch: 0.1,
        walkable_height: ceil_i32(b.stand_height / 0.1_f32),
        crouch_height: ceil_i32(b.crouch_height / 0.1_f32).saturating_sub(2),
        prone_height: ceil_i32(b.prone_height / 0.1_f32).saturating_sub(2),
        walkable_climb: ceil_i32(b.max_step_height / 0.1_f32),
        walkable_radius,
        deep_water: ceil_i32(b.deep_water_depth / 0.1_f32),
        sea_level: ceil_i32(sea / 0.1_f32),
        border_size,
        width: cell_class
            .cells
            .saturating_add(border_size.saturating_mul(2)),
        pad: i32_to_f32(border_size) * cs,
        max_simplification_error: b.contour_max_error / cs,
        max_edge_len: 120,
        max_vertical_error: 8.0,
        min_region_area: min_region,
        merge_region_area: 400,
        nvp: 6,
        door_radius: trunc_i32(i32_to_f32(walkable_radius) * 0.8_f32),
        slope_threshold: ((b.max_slope / 180.0_f32) * std::f32::consts::PI).cos(),
    }
}

/// The result of building one tile.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
#[expect(
    clippy::large_enum_variant,
    reason = "the tile is returned by value; a box would only move it"
)]
pub enum TileOutcome {
    /// A tile blob.
    Built {
        /// The Detour tile.
        data: TileData,
        /// Polygons whose part id was beyond the extras list (spec 12.3); they were ignored.
        stray_part_ids: usize,
        /// Tree obstacles stored in the tile.
        obstacles: usize,
    },
    /// No walkable surface: no contours or no polygons.
    Empty,
}

/// One entry of the tile extras list (spec 5.1).
struct ExtraPart {
    object_id: u64,
    part_index: u32,
    area: u8,
    /// 0: the polygons of the part get the disabled flag.
    state: u8,
    polys: Vec<u32>,
}

const SEA_FLOOR: f32 = -500.0;
const NULL_AREA: u8 = 0;
/// Areas that are not rasterised as walkable objects any more (spec 5.4 step 3).
const TERRAIN_AREA: u32 = 1;
const LADDER_AREA: u32 = 15;

fn area_u8(area: u32) -> Option<u8> {
    u8::try_from(area).ok().filter(|a| *a < 64)
}

/// The extras list: all parts of all non-tree groups, and the part count of the last tree group.
fn extras_list(input: &TileInput, doors: DoorPolicy) -> (Vec<ExtraPart>, usize) {
    let mut extras = Vec::new();
    let mut tree_items = 0;
    for g in &input.groups {
        if g.area == 10 {
            tree_items = g.parts.len();
            continue;
        }
        let state = match doors {
            DoorPolicy::Closed if g.area == 12 => 0,
            DoorPolicy::Open if g.area == 13 => 0,
            _ => 1,
        };
        for p in &g.parts {
            extras.push(ExtraPart {
                object_id: p.object_id,
                part_index: p.part_index,
                area: u8::try_from(g.area & 0xff).unwrap_or(0),
                state,
                polys: Vec::new(),
            });
        }
    }
    (extras, tree_items)
}

/// Rasterises the triangles of `verts` one by one in rasteriser order `(v0, v2, v1)`. With
/// `slope` set, triangles that fail the slope test get area 0.
fn raster_triangles(
    hf: &mut Heightfield,
    verts: &[[f32; 3]],
    params: &RasterParams,
    slope: Option<f32>,
) {
    for [v0, v1, v2] in verts.as_chunks::<3>().0.iter().copied() {
        let mut p = *params;
        if let Some(threshold) = slope
            && !walkable(threshold, v0, v1, v2)
        {
            p.area = NULL_AREA;
        }
        rasterize_triangle(hf, v0, v2, v1, &p);
    }
}

/// The vertices of a part (server order); empty when the range is invalid.
fn part_verts(input: &TileInput, group: usize, part: usize) -> &[[f32; 3]] {
    input
        .part_vertex_range(group, part)
        .and_then(|range| input.groups.get(group)?.verts.get(range))
        .unwrap_or(&[])
}

/// The triangles of a part in rasteriser order, as one flat vertex list.
fn part_triangles(input: &TileInput, group: usize, part: usize) -> Vec<[f32; 3]> {
    let verts = part_verts(input, group, part);
    let mut out = Vec::with_capacity(verts.len());
    for [v0, v1, v2] in verts.as_chunks::<3>().0.iter().copied() {
        out.extend([v0, v2, v1]);
    }
    out
}

/// Spec 5.4 step 3: the objects of every group but the terrain. Returns the ladder groups, which
/// the link stage uses.
fn rasterize_objects<'a>(
    hf: &mut Heightfield,
    input: &'a TileInput,
    cfg: &TileConfig,
    obstacles: bool,
) -> Vec<&'a TileGroup> {
    let mut ladders = Vec::new();
    let mut part_counter = 0_usize;
    let slope = Some(cfg.slope_threshold);
    for (gi, g) in input.groups.iter().enumerate() {
        if g.area == TERRAIN_AREA {
            continue;
        }
        if g.area == LADDER_AREA {
            ladders.push(g);
            continue;
        }
        let Some(area) = area_u8(g.area) else {
            continue;
        };
        let mut params = RasterParams {
            area,
            walkable_climb: cfg.walkable_climb,
            walkable_height: cfg.walkable_height,
            ..RasterParams::default()
        };
        match g.area {
            6 | 7 | 8 | 9 | 11 => raster_triangles(hf, &g.verts, &params, slope),
            2 => {
                params.mode = RasterMode::Water;
                params.water_depth = 0;
                raster_triangles(hf, &g.verts, &params, None);
                params.area = 3;
                params.water_depth = cfg.deep_water;
                raster_triangles(hf, &g.verts, &params, None);
            }
            10 => {
                if g.parts.is_empty() {
                    params.mode = RasterMode::Tree;
                    raster_triangles(hf, &g.verts, &params, None);
                    continue;
                }
                for k in 0..g.parts.len() {
                    if obstacles {
                        params.mode = RasterMode::Tree;
                        params.sub_area = u16::try_from(k).unwrap_or(u16::MAX);
                    } else {
                        params.mode = RasterMode::Add;
                        params.area = NULL_AREA;
                        params.sub_area = 0;
                    }
                    raster_triangles(hf, part_verts(input, gi, k), &params, None);
                }
            }
            12..=14 => {
                params.mode = if g.area == 14 {
                    RasterMode::PaintOrAdd
                } else {
                    RasterMode::PaintBelow
                };
                if g.area == 14 {
                    params.flag_merge_threshold = 1;
                } else {
                    params.flag_merge_threshold = cfg.walkable_height;
                }
                if g.parts.is_empty() {
                    raster_triangles(hf, &g.verts, &params, None);
                    continue;
                }
                for k in 0..g.parts.len() {
                    params.sub_area = u16::try_from(part_counter).unwrap_or(u16::MAX);
                    part_counter = part_counter.saturating_add(1);
                    rasterize_part(hf, &part_triangles(input, gi, k), &params);
                }
            }
            18 => raster_triangles(hf, &g.verts, &params, None),
            _ => {}
        }
    }
    ladders
}

/// The area table of spec 4: the default minimum region area, except for areas 10 and 12 to 14.
fn min_region_table(cfg: &TileConfig) -> [i32; 64] {
    let mut table = [cfg.min_region_area; 64];
    for id in [10, 12, 13, 14] {
        if let Some(e) = table.get_mut(id) {
            *e = i32::MAX;
        }
    }
    table
}

fn areas_flag(mask: &mut [bool; 64], ids: &[usize], value: bool) {
    for &id in ids {
        if let Some(m) = mask.get_mut(id) {
            *m = value;
        }
    }
}

/// Spec 5.4 step 6 (`postCompact`): sea box, dilations, tree obstacles, erosion, remap and roof.
fn post_compact(
    chf: &mut CompactHeightfield,
    cfg: &TileConfig,
    pbmin: [f32; 3],
    pbmax: [f32; 3],
    tree_items: usize,
    obstacles: bool,
) -> io::Result<Vec<Obstacle>> {
    // 6.1 The sea box.
    mark_box_area(
        chf,
        [pbmin[0], SEA_FLOOR, pbmin[2]],
        [pbmax[0], i32_to_f32(cfg.sea_level) * cfg.ch, pbmax[2]],
        5,
    );
    // 6.2 Dilations: every area but the door and hide areas takes part.
    let mut mask = [true; 64];
    areas_flag(&mut mask, &[12, 13, 14], false);
    dilate_area(chf, cfg.door_radius, 13, &mask);
    dilate_area(chf, cfg.door_radius, 12, &mask);
    dilate_area(chf, cfg.walkable_radius, 14, &mask);
    dilate_area(chf, cfg.walkable_radius, 16, &mask);
    // 6.3 Tree obstacles.
    let mut found = Vec::new();
    if obstacles && tree_items > 0 {
        let mut seeds = [false; 64];
        areas_flag(&mut seeds, &[0, 10, 12, 13, 14], true);
        found = extract_tree_obstacles(chf, 10, 1, tree_items, &seeds, 2.0, 3.0);
    }
    // 6.4 to 6.6.
    erode(chf, cfg.walkable_radius).map_err(|e| error!("could not erode: {e}"))?;
    remap_and_roof(chf);
    Ok(found)
}

/// Spec 5.4 step 7 hook: jump, climb and ladder links. Task 12 fills it in; no links for now.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the link stage is fallible once it exists"
)]
pub(crate) fn links(
    _chf: &mut CompactHeightfield,
    _hf: &Heightfield,
    _ladders: &[&TileGroup],
    _cfg: &TileConfig,
    _world: &WorldParams,
) -> io::Result<Vec<OffMeshInput>> {
    Ok(Vec::new())
}

/// The poly flags of an area (spec 7.1).
fn area_flags(area: u8) -> u16 {
    match area {
        1 | 6 | 7 | 9 | 14 => 0x01,
        2 | 4 => 0x11,
        3 => 0x10,
        5 => 0x20,
        8 => 0x09,
        12 | 13 => 0x05,
        16 => 0x0400,
        17 => 0x0800,
        _ => 0,
    }
}

/// Spec 7.2: attaches polygons to parts, sets or clears the disabled flag and counts the stray part
/// ids. Returns the stray count.
fn attach_parts(
    extras: &mut [ExtraPart],
    sub_areas: &[u16],
    areas: &[u8],
    flags: &mut [u16],
) -> usize {
    let mut stray = 0;
    if extras.is_empty() {
        return stray;
    }
    for (i, (&sub, &area)) in sub_areas.iter().zip(areas).enumerate() {
        let Some(part) = extras.get_mut(usize::from(sub)) else {
            stray += 1;
            continue;
        };
        if part.area != area {
            continue;
        }
        if let Some(f) = flags.get_mut(i) {
            if part.state == 0 {
                *f |= 0x2;
            } else {
                *f &= !0x2;
            }
        }
        part.polys.push(u32::try_from(i).unwrap_or(u32::MAX));
    }
    stray
}

/// The extra section from the extras list (spec 7.2).
fn extra_section(extras: &[ExtraPart], obstacles: Vec<Obstacle>) -> io::Result<TileExtra> {
    let mut extra = TileExtra::default();
    for part in extras {
        let first = if part.polys.is_empty() {
            -1
        } else {
            i32::try_from(extra.part_polys.len()).map_err(|_| error!("too many part polygons"))?
        };
        extra.parts.push(TilePart {
            object_id: part.object_id,
            part_index: part.part_index,
            first_poly: first,
        });
        let base = extra.part_polys.len();
        for (k, poly) in part.polys.iter().enumerate() {
            let next = if k + 1 < part.polys.len() {
                i32::try_from(base + k + 1).map_err(|_| error!("too many part polygons"))?
            } else {
                -1
            };
            extra.part_polys.push(TilePartPoly { poly: *poly, next });
        }
    }
    extra.obstacles = obstacles;
    Ok(extra)
}

/// Builds the Detour tile of one tile input (spec 5).
///
/// # Errors
///
/// Returns an error for an unknown cell class ("tile X Z has unknown cell class N"), an invalid
/// grid, or a failure of one of the pipeline stages.
pub fn build_tile(
    world: &WorldParams,
    input: &TileInput,
    options: &BuildOptions,
) -> io::Result<TileOutcome> {
    let class = usize::try_from(input.cell_class)
        .ok()
        .and_then(|i| world.cell_classes.get(i))
        .ok_or_else(|| {
            error!(
                "tile {} {} has unknown cell class {}",
                input.x, input.z, input.cell_class
            )
        })?;
    let cfg = tile_config(world, class, options);
    if cfg.cells <= 0 || !cfg.cs.is_finite() || cfg.cs <= 0.0 {
        return Err(error!(
            "tile {} {}: cell class has {} cells of {} m",
            input.x, input.z, cfg.cells, cfg.cs
        ));
    }
    let obstacles_on = world.world.obstacles;

    // 5.1 setup.
    let (mut extras, tree_items) = extras_list(input, options.doors);
    let pad = cfg.pad;
    let pbmin = [input.bmin[0] - pad, input.bmin[1], input.bmin[2] - pad];
    let pbmax = [input.bmax[0] + pad, input.bmax[1], input.bmax[2] + pad];
    let mut hf = Heightfield::new(cfg.width, cfg.width, pbmin, pbmax, cfg.cs, cfg.ch)
        .map_err(|e| error!("could not create solid heightfield: {e}"))?;

    // 5.4 step 1: terrain.
    if let Some(g) = input.groups.first()
        && let Some(area) = area_u8(g.area)
    {
        let p = RasterParams {
            area,
            ..RasterParams::default()
        };
        raster_triangles(&mut hf, &g.verts, &p, Some(cfg.slope_threshold));
    }
    // Step 2: mode filter (it only runs for a border above 10).
    mode_filter(&mut hf, cfg.border_size);
    // Step 3: objects.
    let ladders = rasterize_objects(&mut hf, input, &cfg, obstacles_on);
    // Step 4: span filters.
    filter_low_height_multi(
        &mut hf,
        [cfg.prone_height, cfg.crouch_height, cfg.walkable_height],
        [0, 16, 17],
        18,
    );
    filter_low_hanging(&mut hf, cfg.walkable_climb);
    let mut skip = [false; 64];
    areas_flag(&mut skip, &[0, 1], true);
    filter_ledges(&mut hf, cfg.walkable_height, cfg.walkable_climb, &skip);
    // Step 5: compact heightfield.
    let mut chf = build_compact(cfg.prone_height, cfg.walkable_climb, &hf)
        .map_err(|e| error!("could not build compact data: {e}"))?;
    // Step 6: area passes.
    let obstacles = post_compact(&mut chf, &cfg, pbmin, pbmax, tree_items, obstacles_on)?;
    // Step 7: links.
    let off_mesh = if options.links {
        links(&mut chf, &hf, &ladders, &cfg, world)?
    } else {
        Vec::new()
    };
    drop(hf);
    // Step 8: distance field and regions.
    build_distance_field(&mut chf).map_err(|e| error!("could not build distance field: {e}"))?;
    build_regions(
        &mut chf,
        cfg.border_size,
        &min_region_table(&cfg),
        cfg.merge_region_area,
    )
    .map_err(|e| error!("could not build regions: {e}"))?;
    // Step 9: contours.
    let cset = build_contours(
        &chf,
        cfg.max_simplification_error,
        cfg.max_vertical_error,
        cfg.max_edge_len,
        RC_CONTOUR_TESS_WALL_EDGES,
    )
    .map_err(|e| error!("could not create contours: {e}"))?;
    if cset.contours.is_empty() {
        return Ok(TileOutcome::Empty);
    }
    // Step 10: polygon mesh.
    let pm = build_poly_mesh(&cset, cfg.nvp)
        .map_err(|e| error!("could not triangulate contours: {e}"))?;
    if pm.areas.is_empty() {
        return Ok(TileOutcome::Empty);
    }

    finish_tile(world, input, &cfg, &pm, &mut extras, obstacles, &off_mesh)
}

/// Spec 5.4 step 11 and 7: flags, extras and the Detour tile.
fn finish_tile(
    world: &WorldParams,
    input: &TileInput,
    cfg: &TileConfig,
    pm: &PolyMesh,
    extras: &mut [ExtraPart],
    obstacles: Vec<Obstacle>,
    off_mesh: &[OffMeshInput],
) -> io::Result<TileOutcome> {
    let pad = cfg.pad;
    let mut flags: Vec<u16> = pm.areas.iter().map(|a| area_flags(*a)).collect();
    let stray = attach_parts(extras, &pm.sub_areas, &pm.areas, &mut flags);
    let obstacle_count = obstacles.len();
    let extra = extra_section(extras, obstacles)?;

    let mut params = CreateParams::new(
        &pm.verts, &pm.polys, &flags, &pm.areas, cfg.nvp, off_mesh, &extra,
    );
    params.tile_x = input.x;
    params.tile_y = input.z;
    params.bmin = [
        (input.bmin[0] - pad) + pad,
        input.bmin[1],
        (input.bmin[2] - pad) + pad,
    ];
    params.bmax = [
        (input.bmax[0] + pad) - pad,
        input.bmax[1] + i32_to_f32(cfg.prone_height) * 0.1_f32,
        (input.bmax[2] + pad) - pad,
    ];
    params.walkable_height = world.world.stand_height;
    params.walkable_radius = world.world.diameter * 0.5_f32;
    params.walkable_climb = world.world.max_step_height;
    params.cs = cfg.cs;
    params.ch = cfg.ch;
    let data =
        create_tile_data(&params).map_err(|e| error!("could not build Detour navmesh: {e}"))?;
    Ok(TileOutcome::Built {
        data,
        stray_part_ids: stray,
        obstacles: obstacle_count,
    })
}
