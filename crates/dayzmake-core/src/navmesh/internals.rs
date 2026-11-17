//! Pipeline stages for tests and experiments; not part of the stable API; may change in any release.

pub use super::dayz::areas::{dilate_area, extract_tree_obstacles, remap_and_roof};
pub use super::dayz::filters::{filter_low_height_multi, mode_filter};
pub use super::dayz::slope::{rsqrt_approx, walkable};
pub use super::recast::compact::{
    CompactHeightfield, CompactSpan, NOT_CONNECTED, build_compact, dir_offset_x, dir_offset_y,
    erode, mark_box_area,
};
pub use super::recast::contour::{
    Contour, ContourSet, RC_AREA_BORDER, RC_BORDER_VERTEX, RC_CONTOUR_REG_MASK,
    RC_CONTOUR_TESS_AREA_EDGES, RC_CONTOUR_TESS_WALL_EDGES, build_contours,
};
pub use super::recast::filter::{filter_ledges, filter_low_hanging};
pub use super::recast::heightfield::{Heightfield, SPAN_MAX_HEIGHT, Span};
pub use super::recast::rasterize::{
    RasterMode, RasterOutcome, RasterParams, rasterize_part, rasterize_triangle,
};
pub use super::recast::region::{build_distance_field, build_regions, build_regions_monotone};
