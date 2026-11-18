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

//! `rcHeightfield` and `rcSpan` with the DayZ `subArea` (spec 5, 10.1 item 1) and `addSpan`.
//!
//! Upstream keeps the spans of a column in a linked list over a memory pool; here every column is a
//! `Vec<Span>` sorted ascending by height.

use std::io;

/// `RC_SPAN_MAX_HEIGHT`: spans store 13-bit heights.
pub const SPAN_MAX_HEIGHT: u16 = 8191;

/// Emulates the 13-bit `rcSpan` bitfield: upstream stores `smin`/`smax` into 13-bit fields, so a
/// value above [`SPAN_MAX_HEIGHT`] wraps (8192 becomes 0).
#[must_use]
pub fn store13(v: u16) -> u16 {
    v & SPAN_MAX_HEIGHT
}

/// The largest number of columns a heightfield may have (hostile size limit).
const MAX_COLUMNS: usize = 1 << 24;

/// One solid span of a column (`rcSpan`), heights in voxels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Lower limit, below `smax`.
    pub smin: u16,
    /// Upper limit, at most [`SPAN_MAX_HEIGHT`].
    pub smax: u16,
    /// Area id.
    pub area: u8,
    /// DayZ part id (door parts, tree indices).
    pub sub_area: u16,
}

impl Span {
    /// A span with `sub_area` 0.
    #[must_use]
    pub fn new(smin: u16, smax: u16, area: u8) -> Self {
        Self {
            smin,
            smax,
            area,
            sub_area: 0,
        }
    }
}

/// `rcHeightfield`: a grid of `width` x `height` columns, each a sorted list of spans.
#[derive(Clone, Debug)]
pub struct Heightfield {
    /// Columns along x.
    pub width: i32,
    /// Columns along z.
    pub height: i32,
    /// Minimum bounds in world space.
    pub bmin: [f32; 3],
    /// Maximum bounds in world space.
    pub bmax: [f32; 3],
    /// Cell size (x and z).
    pub cs: f32,
    /// Cell height (y).
    pub ch: f32,
    columns: Vec<Vec<Span>>,
    /// Spans added so far (merged spans are subtracted).
    span_count: usize,
    /// The most spans [`Self::add_span`] accepts; a span past it is refused.
    span_limit: usize,
    /// Spans refused for the limit (so the total that was wanted is reported).
    refused: usize,
}

impl Heightfield {
    /// `rcCreateHeightfield`. Errors on a negative or implausibly large grid, or a non-positive or
    /// non-finite cell size.
    ///
    /// # Errors
    /// Returns an error for such a size.
    pub fn new(
        width: i32,
        height: i32,
        bmin: [f32; 3],
        bmax: [f32; 3],
        cs: f32,
        ch: f32,
    ) -> io::Result<Self> {
        let w =
            usize::try_from(width).map_err(|_| error!("heightfield width {width} is negative"))?;
        let h = usize::try_from(height)
            .map_err(|_| error!("heightfield height {height} is negative"))?;
        let n = w
            .checked_mul(h)
            .filter(|n| *n <= MAX_COLUMNS)
            .ok_or_else(|| error!("heightfield {width}x{height} is too large"))?;
        if !(cs.is_finite() && cs > 0.0 && ch.is_finite() && ch > 0.0) {
            return Err(error!(
                "heightfield cell size {cs} / height {ch} is invalid"
            ));
        }
        Ok(Self {
            width,
            height,
            bmin,
            bmax,
            cs,
            ch,
            columns: vec![Vec::new(); n],
            span_count: 0,
            span_limit: usize::MAX,
            refused: 0,
        })
    }

    /// Sets the span budget: once the heightfield holds `limit` spans, [`Self::add_span`] refuses
    /// further ones and [`Self::check_span_budget`] fails.
    pub fn set_span_limit(&mut self, limit: usize) {
        self.span_limit = limit;
    }

    /// The number of spans added through [`Self::add_span`] (after merging).
    #[must_use]
    pub fn span_count(&self) -> usize {
        self.span_count
    }

    /// Fails when [`Self::add_span`] had to refuse a span for the budget. Deterministic: the
    /// tile fails, nothing is silently truncated.
    ///
    /// # Errors
    /// Returns "too many spans (N, limit L)" when the budget was exceeded.
    pub fn check_span_budget(&self) -> io::Result<()> {
        if self.refused == 0 {
            return Ok(());
        }
        Err(error!(
            "too many spans ({}, limit {})",
            self.span_count.saturating_add(self.refused),
            self.span_limit
        ))
    }

    fn index(&self, x: i32, z: i32) -> Option<usize> {
        if x < 0 || z < 0 || x >= self.width || z >= self.height {
            return None;
        }
        let w = usize::try_from(self.width).ok()?;
        usize::try_from(x)
            .ok()?
            .checked_add(usize::try_from(z).ok()?.checked_mul(w)?)
    }

    /// The spans of column `(x, z)`, ascending; empty when out of range.
    #[must_use]
    pub fn column(&self, x: i32, z: i32) -> &[Span] {
        self.index(x, z)
            .and_then(|i| self.columns.get(i))
            .map_or(&[], Vec::as_slice)
    }

    /// All columns, `x + z * width` order.
    #[must_use]
    pub fn columns(&self) -> &[Vec<Span>] {
        &self.columns
    }

    /// Mutable access to all columns. The caller keeps each column sorted ascending.
    pub fn columns_mut(&mut self) -> &mut [Vec<Span>] {
        &mut self.columns
    }

    pub(crate) fn column_mut(&mut self, x: i32, z: i32) -> Option<&mut Vec<Span>> {
        let i = self.index(x, z)?;
        self.columns.get_mut(i)
    }

    /// `addSpan`: insert a span, merging it with every span it overlaps. The merged area is
    /// `max(new, old)` when the merged `smax` is within `flag_merge_threshold` of the old span's
    /// `smax`; otherwise the new area is kept. The new span has `sub_area` 0 (DayZ). Out of range
    /// columns are ignored; heights are stored in 13 bits like `rcSpan` (see [`store13`]).
    pub fn add_span(
        &mut self,
        x: i32,
        z: i32,
        smin: u16,
        smax: u16,
        area: u8,
        flag_merge_threshold: i32,
    ) {
        if self.span_count >= self.span_limit {
            if self.index(x, z).is_some() {
                self.refused = self.refused.saturating_add(1);
            }
            return;
        }
        let Some(i) = self.index(x, z) else {
            return;
        };
        let Some(column) = self.columns.get_mut(i) else {
            return;
        };
        let before = column.len();
        let mut new = Span::new(store13(smin), store13(smax), area);
        let mut i = 0;
        while let Some(&cur) = column.get(i) {
            if cur.smin > new.smax {
                break;
            }
            if cur.smax < new.smin {
                i += 1;
                continue;
            }
            new.smin = new.smin.min(cur.smin);
            new.smax = new.smax.max(cur.smax);
            if (i32::from(new.smax) - i32::from(cur.smax)).abs() <= flag_merge_threshold {
                new.area = new.area.max(cur.area);
            }
            column.remove(i);
        }
        column.insert(i, new);
        let after = column.len();
        self.span_count = self.span_count.saturating_sub(before).saturating_add(after);
    }
}
