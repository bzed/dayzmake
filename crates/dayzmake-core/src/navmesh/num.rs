//! Float to integer conversions with C semantics on in-range values, in one place.
//!
//! They saturate like Rust `as` does (NaN gives 0). The spec's `(int)` casts of computed floats use
//! these helpers rather than scattered `as`.

/// C `(int)x`: truncation towards zero.
#[expect(dead_code, reason = "used by the tile builder tasks that follow")]
#[expect(
    clippy::cast_possible_truncation,
    reason = "C truncation as in Recast; saturating on out-of-range values"
)]
pub(crate) fn trunc_i32(x: f32) -> i32 {
    x as i32
}

/// C `(int)floorf(x)`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "C truncation of an integral value as in Recast; saturating on out-of-range values"
)]
pub(crate) fn floor_i32(x: f32) -> i32 {
    x.floor() as i32
}

/// C `(int)ceilf(x)`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "C truncation of an integral value as in Recast; saturating on out-of-range values"
)]
pub(crate) fn ceil_i32(x: f32) -> i32 {
    x.ceil() as i32
}

/// C `(float)i`: the nearest `f32` (round to nearest even).
#[expect(
    clippy::cast_precision_loss,
    reason = "C int to float conversion; values above 2^24 round as in C"
)]
pub(crate) fn i32_to_f32(i: i32) -> f32 {
    i as f32
}

/// C `(unsigned short)x`: truncation towards zero, saturating on out-of-range values.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "C truncation as in Detour; saturating on out-of-range values"
)]
pub(crate) fn trunc_u16(x: f32) -> u16 {
    x as u16
}

/// C `(unsigned char)x`: truncation towards zero, saturating on out-of-range values.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "C truncation as in Detour; saturating on out-of-range values"
)]
pub(crate) fn trunc_u8(x: f32) -> u8 {
    x as u8
}
