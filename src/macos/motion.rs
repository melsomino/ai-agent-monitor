//! Disclosure timing independent of AppKit, including interrupted transitions.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub const DURATION: Duration = Duration::from_millis(240);

pub struct Disclosure {
    from: HashMap<String, f64>,
    to: HashMap<String, f64>,
    started: Instant,
}
impl Disclosure {
    pub fn new(from: HashMap<String, f64>, to: HashMap<String, f64>, started: Instant) -> Self {
        Self { from, to, started }
    }
    pub fn show(&mut self, id: &str) {
        self.from.insert(id.to_owned(), 1.0);
        self.to.insert(id.to_owned(), 1.0);
    }
    pub fn sample(&self, now: Instant) -> (HashMap<String, f64>, bool) {
        let t = (now.saturating_duration_since(self.started).as_secs_f64()
            / DURATION.as_secs_f64())
        .min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        let values = self
            .to
            .iter()
            .map(|(id, target)| {
                let start = self.from.get(id).copied().unwrap_or(0.0);
                (id.clone(), start + (target - start) * eased)
            })
            .collect();
        (values, t >= 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn values(value: f64) -> HashMap<String, f64> {
        HashMap::from([("chat".into(), value)])
    }
    #[test]
    fn disclosure_is_monotonic_bounded_and_finishes_at_its_target() {
        let now = Instant::now();
        for (from, to) in [(0.0, 1.0), (1.0, 0.0)] {
            let motion = Disclosure::new(values(from), values(to), now);
            assert_eq!(motion.sample(now).0["chat"], from);
            let mut previous = from;
            for frame in 1..=30 {
                let (sample, done) = motion.sample(now + Duration::from_millis(frame * 16));
                let value = sample["chat"];
                assert!((0.0..=1.0).contains(&value));
                assert!((value - previous) * (to - from) >= 0.0);
                if done {
                    assert_eq!(value, to);
                }
                previous = value;
            }
            assert_eq!(previous, to);
        }
    }
    #[test]
    fn reversing_midway_starts_at_the_visible_position_without_a_jump() {
        let now = Instant::now();
        let opening = Disclosure::new(values(0.0), values(1.0), now);
        let halfway = now + DURATION / 2;
        let (visible, done) = opening.sample(halfway);
        assert!(!done);
        assert!(visible["chat"] > 0.0 && visible["chat"] < 1.0);
        let closing = Disclosure::new(visible.clone(), values(0.0), halfway);
        assert_eq!(closing.sample(halfway).0, visible);
        assert_eq!(closing.sample(halfway + DURATION).0["chat"], 0.0);
    }
}

/// Eight brightness steps for the left and right menu bar eyes.
/// Repeated discovery updates must not restart the sequence.
#[derive(Default)]
pub struct WorkingEyes {
    active: bool,
    animate: bool,
    phase: usize,
}
impl WorkingEyes {
    pub fn update(&mut self, active: bool, animate: bool) {
        if active && !self.active {
            self.phase = 0;
        }
        self.active = active;
        self.animate = active && animate;
    }

    pub fn tick(&mut self) {
        if self.animate {
            self.phase = (self.phase + 1) % 8;
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn levels(&self) -> [f64; 2] {
        const STEPS: [[f64; 2]; 8] = [
            [0.5, 0.0],
            [1.0, 0.0],
            [1.0, 0.5],
            [1.0, 1.0],
            [0.5, 1.0],
            [0.0, 1.0],
            [0.0, 0.5],
            [0.0, 0.0],
        ];
        if !self.active {
            [0.0, 0.0]
        } else if !self.animate {
            [1.0, 1.0]
        } else {
            STEPS[self.phase]
        }
    }
}

#[cfg(test)]
mod eyes_tests {
    use super::WorkingEyes;

    #[test]
    fn cycles_eight_brightness_steps_without_resetting_on_refresh() {
        let mut eyes = WorkingEyes::default();
        assert_eq!(eyes.levels(), [0.0, 0.0]);
        eyes.update(true, true);
        for expected in [
            [0.5, 0.0],
            [1.0, 0.0],
            [1.0, 0.5],
            [1.0, 1.0],
            [0.5, 1.0],
            [0.0, 1.0],
            [0.0, 0.5],
            [0.0, 0.0],
            [0.5, 0.0],
        ] {
            eyes.update(true, true);
            assert!(eyes.active());
            assert_eq!(eyes.levels(), expected);
            eyes.tick();
        }
        eyes.update(false, false);
        assert!(!eyes.active());
        eyes.tick();
        assert_eq!(eyes.levels(), [0.0, 0.0]);
        eyes.update(true, true);
        assert_eq!(eyes.levels(), [0.5, 0.0]);
    }

    #[test]
    fn disabled_animation_keeps_both_eyes_green_only_while_working() {
        let mut eyes = WorkingEyes::default();
        eyes.update(true, false);
        for _ in 0..10 {
            eyes.tick();
            assert_eq!(eyes.levels(), [1.0, 1.0]);
        }
        eyes.update(false, false);
        assert!(!eyes.active());
        assert_eq!(eyes.levels(), [0.0, 0.0]);
    }
}
