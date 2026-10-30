#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub minutes: u64,
    pub used_percent: f64,
    pub resets_at: Option<i64>,
}
impl Window {
    pub fn label(&self) -> String {
        match self.minutes {
            10080 => "Weekly".into(),
            1440 => "Daily".into(),
            n if n % 60 == 0 => format!("{}-hour", n / 60),
            n => format!("{n}-minute"),
        }
    }
    pub fn remaining(&self, now: i64) -> Option<u32> {
        // An expired observation cannot tell us the balance in the new window.
        if self.resets_at.is_some_and(|reset| reset <= now) {
            return None;
        }
        Some((100.0 - self.used_percent).clamp(0.0, 100.0).round() as u32)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Usage {
    pub observed_at: String,
    pub windows: Vec<Window>,
}
