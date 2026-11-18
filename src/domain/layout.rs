//! Geometry and quota colors shared by native session lists.
pub const ROW_HEIGHT: f64 = 36.0;
pub const TOP_INSET: f64 = 6.0;
pub const TEXT_LEFT: f64 = 40.0;
pub const DETAIL_WIDTH: f64 = 326.0;
pub const ICON_LEFT: f64 = 8.0;
pub const ICON_SIZE: f64 = 24.0;
pub const BUTTON_HORIZONTAL_INSET: f64 = 18.0;
pub const BUTTON_VERTICAL_INSET: f64 = 4.0;
pub const BUTTON_MIN_HEIGHT: f64 = 24.0;

/// Remaining-quota tint shared by native frontends, matching the web palette.
pub fn quota_rgb(remaining: f64) -> [u8; 3] {
    const STOPS: [(f64, [f64; 3]); 6] = [
        (0.0, [255.0, 48.0, 61.0]),
        (30.0, [255.0, 126.0, 52.0]),
        (40.0, [246.0, 198.0, 58.0]),
        (50.0, [146.0, 180.0, 221.0]),
        (60.0, [130.0, 177.0, 230.0]),
        (100.0, [68.0, 143.0, 247.0]),
    ];
    let remaining = if remaining.is_finite() {
        remaining.clamp(0.0, 100.0)
    } else {
        0.0
    };
    let pair = STOPS
        .windows(2)
        .find(|pair| remaining <= pair[1].0)
        .unwrap();
    let t = (remaining - pair[0].0) / (pair[1].0 - pair[0].0);
    std::array::from_fn(|i| (pair[0].1[i] + (pair[1].1[i] - pair[0].1[i]) * t).round() as u8)
}
