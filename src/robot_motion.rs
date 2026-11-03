//! Platform-independent motion for working session markers (not menu-bar settings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub eyes: [f64; 2],
}

/// One 2.4-second cycle. Reduced motion keeps both eyes steadily lit.
pub fn sample(seconds: f64, animate: bool) -> Frame {
    if !animate {
        return Frame { eyes: [1.0; 2] };
    }
    let phase = seconds.rem_euclid(2.4) / 2.4;
    let levels = [
        [0.5, 0.0],
        [1.0, 0.0],
        [1.0, 0.5],
        [1.0, 1.0],
        [0.5, 1.0],
        [0.0, 1.0],
        [0.0, 0.5],
        [0.0, 0.0],
    ];
    let step = phase * 8.0;
    let index = step.floor() as usize;
    let mix = step.fract();
    Frame {
        eyes: std::array::from_fn(|eye| {
            levels[index][eye] * (1.0 - mix) + levels[(index + 1) % 8][eye] * mix
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cycle_is_bounded_periodic_and_reduced_motion_is_static() {
        for n in 0..240 {
            let t = n as f64 / 100.0;
            let f = sample(t, true);
            assert!(f.eyes.iter().all(|v| (0.0..=1.0).contains(v)));
            let next = sample(t + 2.4, true);
            assert!(
                f.eyes
                    .iter()
                    .zip(next.eyes)
                    .all(|(a, b)| (a - b).abs() < 0.000001)
            );
            assert_eq!(sample(t, false), Frame { eyes: [1.0; 2] });
        }
        assert_eq!(sample(0.0, true).eyes, [0.5, 0.0]);
        assert_eq!(sample(0.9, true).eyes, [1.0, 1.0]);
    }
}
