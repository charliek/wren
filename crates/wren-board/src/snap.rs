//! The editing grid (§5.8 of the board plan). Snapping is always on for
//! creating, moving and resizing; the results are integers.

/// The grid line nearest `value`, in board units: a multiple of
/// [`crate::GRID`].
pub fn grid(value: f32) -> i32 {
    let _ = value;
    unimplemented!("snapping arrives with item E3")
}
