//! Geometry shared by the native list's rows and pinned application icons.
pub const ROW_HEIGHT: f64 = 36.0;
pub const TOP_INSET: f64 = 6.0;
pub const GROUP_GAP: f64 = 13.0;
pub const TEXT_LEFT: f64 = 40.0;
pub const DETAIL_WIDTH: f64 = 326.0;
pub const ICON_LEFT: f64 = 8.0;
pub const ICON_SIZE: f64 = 24.0;
pub const BUTTON_HORIZONTAL_INSET: f64 = 18.0;
pub const BUTTON_VERTICAL_INSET: f64 = 4.0;
pub const BUTTON_MIN_HEIGHT: f64 = 24.0;

/// The complete row-sized anchor stays inside its application section. Using
/// section bounds, rather than the next icon, also handles one-session groups.
pub fn sticky_y(start: f64, end: f64, viewport_top: f64) -> f64 {
    // TOP_INSET already belongs to the first row's document position. Adding it
    // again when pinning shifts the app icon below a header scrolled to the top.
    start.max(viewport_top).min((end - ROW_HEIGHT).max(start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_start_at_the_first_row_pin_and_get_pushed_before_the_next_group() {
        assert_eq!(sticky_y(6.0, 206.0, 0.0), 6.0);
        assert_eq!(sticky_y(6.0, 206.0, 80.0), 80.0);
        assert_eq!(sticky_y(6.0, 206.0, 180.0), 170.0);
        let next_start = 206.0 + GROUP_GAP;
        assert!(sticky_y(6.0, 206.0, 200.0) + ROW_HEIGHT <= sticky_y(next_start, 500.0, 200.0));
    }

    #[test]
    fn single_rows_and_changing_expanded_heights_never_escape_their_section() {
        assert_eq!(sticky_y(100.0, 136.0, 125.0), 100.0);
        assert_eq!(sticky_y(6.0, 80.0, 150.0), 44.0);
        assert_eq!(sticky_y(6.0, 500.0, 150.0), 150.0);
        assert_eq!(sticky_y(6.0, 80.0, -30.0), 6.0);
    }

    #[test]
    fn scrolling_first_header_to_viewport_top_does_not_add_another_inset() {
        for start in [TOP_INSET, 120.0] {
            assert_eq!(sticky_y(start, start + 400.0, start), start);
            assert_eq!(sticky_y(start, start + 400.0, start - 1.0), start);
            assert_eq!(sticky_y(start, start + 400.0, start + 1.0), start + 1.0);
        }
    }
}
