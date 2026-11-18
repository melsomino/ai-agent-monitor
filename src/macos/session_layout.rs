//! macOS glass-layout measurements; GNOME retains the shared baseline.
use crate::domain::layout as baseline;
pub use baseline::{ICON_LEFT, ICON_SIZE};
pub const ROW_HEIGHT: f64 = baseline::ROW_HEIGHT + 2.0;
pub const TOP_INSET: f64 = super::visual::BRAND_HEIGHT + baseline::TOP_INSET + 1.0;
pub const TEXT_LEFT: f64 = baseline::TEXT_LEFT - 2.0;
pub const DETAIL_WIDTH: f64 = baseline::DETAIL_WIDTH + 62.0;
pub const BUTTON_HORIZONTAL_INSET: f64 = baseline::BUTTON_HORIZONTAL_INSET - 2.0;
pub const BUTTON_VERTICAL_INSET: f64 = baseline::BUTTON_VERTICAL_INSET + 3.0;
pub const BUTTON_MIN_HEIGHT: f64 = baseline::BUTTON_MIN_HEIGHT + 8.0;
