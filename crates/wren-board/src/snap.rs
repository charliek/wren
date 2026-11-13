//! The editing grid (§5.8 of the board plan). Snapping is always on for
//! creating, moving and resizing; the results are integers.

/// The grid line nearest `value`, in board units: a multiple of
/// [`crate::GRID`].
pub fn grid(value: f32) -> i32 {
    let cell = crate::GRID as f32;
    ((value / cell).round() * cell) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_goes_to_the_nearest_grid_line() {
        for (value, line) in [
            (0.0, 0),
            (9.9, 0),
            (10.1, 20),
            (29.0, 20),
            (131.0, 140),
            (-9.9, 0),
            (-10.1, -20),
            (-47.0, -40),
        ] {
            assert_eq!(grid(value), line, "{value}");
        }
    }
}
