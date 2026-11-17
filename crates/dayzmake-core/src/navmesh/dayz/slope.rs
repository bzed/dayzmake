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

//! The server's walkability test (spec 5.2).

/// `1 / sqrt(d)` as the game computes it: the x86 `RSQRTSS` approximation (a relative error of about
/// `1.5 * 2^-12`) where SSE is in the build's baseline, which includes every `x86_64` target. On other
/// targets an exact `1.0 / d.sqrt()` is used, so slope classification there can differ from DayZ's.
#[cfg(all(
    any(target_arch = "x86", target_arch = "x86_64"),
    target_feature = "sse"
))]
#[must_use]
pub fn rsqrt_approx(d: f32) -> f32 {
    safe_arch::get_f32_from_m128_s(safe_arch::reciprocal_sqrt_m128_s(safe_arch::set_m128_s(d)))
}

/// `1 / sqrt(d)` as the game computes it; see the x86 variant. This target has no `RSQRTSS`, so the
/// result is exact and can differ from DayZ's in the low bits.
#[cfg(not(all(
    any(target_arch = "x86", target_arch = "x86_64"),
    target_feature = "sse"
)))]
#[must_use]
pub fn rsqrt_approx(d: f32) -> f32 {
    1.0 / d.sqrt()
}

/// Whether the triangle `(v0, v1, v2)` as the server sends it is walkable: its normal
/// `cross(v2 - v0, v1 - v0)`, normalised with one Newton step after `RSQRTSS`, has a `y` above
/// `slope_threshold`. On targets without `RSQRTSS` the result can differ from DayZ's. The caller passes `(v0, v2, v1)` to the rasteriser.
#[must_use]
pub fn walkable(slope_threshold: f32, v0: [f32; 3], v1: [f32; 3], v2: [f32; 3]) -> bool {
    let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
    let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
    let nx = e2[1] * e1[2] - e2[2] * e1[1];
    let mut ny = e2[2] * e1[0] - e2[0] * e1[2];
    let nz = e2[0] * e1[1] - e2[1] * e1[0];
    let d = (nx * nx + ny * ny) + nz * nz;
    if d > f32::MIN_POSITIVE {
        let r = rsqrt_approx(d);
        let inv = (0.5 * r) * (3.0 - (d * r) * r);
        ny *= inv;
    }
    slope_threshold < ny
}
