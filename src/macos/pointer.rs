//! Hit testing in the scroll document's flipped coordinate system.
#[derive(Clone, Copy)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Area {
    pub fn intersected(self, clip: Self) -> Self {
        let x = self.x.max(clip.x);
        let y = self.y.max(clip.y);
        Self {
            x,
            y,
            width: ((self.x + self.width).min(clip.x + clip.width) - x).max(0.0),
            height: ((self.y + self.height).min(clip.y + clip.height) - y).max(0.0),
        }
    }

    pub fn contains(self, point: (f64, f64)) -> bool {
        point.0 >= self.x
            && point.0 < self.x + self.width
            && point.1 >= self.y
            && point.1 < self.y + self.height
    }
}
pub fn header_at(point: (f64, f64), viewport: Area, headers: &[Area]) -> Option<usize> {
    viewport
        .contains(point)
        .then(|| headers.iter().position(|header| header.contains(point)))
        .flatten()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moving_below_an_expanded_header_does_not_leave_it_highlighted() {
        let viewport = Area {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 800.0,
        };
        let headers = [
            Area {
                x: 20.0,
                y: 0.0,
                width: 350.0,
                height: 48.0,
            },
            Area {
                x: 20.0,
                y: 400.0,
                width: 350.0,
                height: 48.0,
            },
        ];
        assert_eq!(header_at((100.0, 20.0), viewport, &headers), Some(0));
        assert_eq!(header_at((100.0, 200.0), viewport, &headers), None);
        assert_eq!(header_at((100.0, 420.0), viewport, &headers), Some(1));
        assert_eq!(header_at((100.0, 448.0), viewport, &headers), None);
        assert_eq!(header_at((5.0, 420.0), viewport, &headers), None);
    }
    #[test]
    fn clipped_headers_and_window_margins_cannot_be_hovered() {
        let viewport = Area {
            x: 0.0,
            y: 40.0,
            width: 400.0,
            height: 400.0,
        };
        let headers = [
            Area {
                x: 20.0,
                y: 0.0,
                width: 350.0,
                height: 48.0,
            },
            Area {
                x: 20.0,
                y: 420.0,
                width: 350.0,
                height: 48.0,
            },
        ];
        assert_eq!(header_at((100.0, 20.0), viewport, &headers), None);
        assert_eq!(header_at((100.0, 44.0), viewport, &headers), Some(0));
        assert_eq!(header_at((100.0, 444.0), viewport, &headers), None);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Row(usize),
    Button(usize),
}

/// Every rectangle is in the same document coordinate system. Overlay controls
/// win, then row controls, then row headers; outside the window nothing wins.
pub fn target_at(
    point: (f64, f64),
    window: Area,
    viewport: Area,
    toolbar: &[Area],
    buttons: &[Area],
    headers: &[Area],
) -> Option<Target> {
    if !window.contains(point) {
        return None;
    }
    if let Some(index) = toolbar.iter().position(|area| area.contains(point)) {
        return Some(Target::Button(index));
    }
    if !viewport.contains(point) {
        return None;
    }
    if let Some(index) = buttons.iter().position(|area| area.contains(point)) {
        return Some(Target::Button(toolbar.len() + index));
    }
    header_at(point, viewport, headers).map(Target::Row)
}

#[cfg(test)]
mod target_tests {
    use super::*;
    #[test]
    fn drawing_visibility_cannot_expand_a_buttons_hover_target() {
        let window = Area {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 800.0,
        };
        let bounds = Area {
            x: 324.0,
            y: 8.0,
            width: 24.0,
            height: 24.0,
        };
        // AppKit's visible drawing rectangle can extend beyond an unclipped view.
        let toolbar = [bounds.intersected(window)];
        let resolve = |point| target_at(point, window, window, &toolbar, &[], &[]);
        assert_eq!(resolve((330.0, 16.0)), Some(Target::Button(0)));
        assert_eq!(resolve((100.0, 16.0)), None);
        assert_eq!(resolve((330.0, 200.0)), None);
        assert_eq!(resolve((350.0, 16.0)), None);

        let footer = Area {
            x: 274.0,
            y: 300.0,
            width: 70.0,
            height: 28.0,
        };
        let reveal = Area {
            x: 0.0,
            y: 48.0,
            width: 372.0,
            height: 264.0,
        };
        let buttons = [footer.intersected(reveal)];
        assert_eq!(
            target_at((300.0, 305.0), window, window, &[], &buttons, &[]),
            Some(Target::Button(0))
        );
        assert_eq!(
            target_at((300.0, 320.0), window, window, &[], &buttons, &[]),
            None
        );
    }

    #[test]
    fn a_single_target_moves_between_row_button_arrow_and_outside() {
        let window = Area {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 800.0,
        };
        let headers = [
            Area {
                x: 20.0,
                y: 50.0,
                width: 350.0,
                height: 48.0,
            },
            Area {
                x: 20.0,
                y: 400.0,
                width: 350.0,
                height: 48.0,
            },
        ];
        let toolbar = [Area {
            x: 350.0,
            y: 0.0,
            width: 24.0,
            height: 24.0,
        }];
        let buttons = [Area {
            x: 330.0,
            y: 62.0,
            width: 24.0,
            height: 24.0,
        }];
        let resolve = |point| target_at(point, window, window, &toolbar, &buttons, &headers);
        assert_eq!(resolve((100.0, 60.0)), Some(Target::Row(0)));
        assert_eq!(resolve((340.0, 70.0)), Some(Target::Button(1)));
        assert_eq!(resolve((100.0, 420.0)), Some(Target::Row(1)));
        assert_eq!(resolve((360.0, 12.0)), Some(Target::Button(0)));
        assert_eq!(resolve((500.0, 420.0)), None);
        assert_eq!(resolve((100.0, 200.0)), None);
    }
}

/// Reveal the history affordance only over the first message or its gutter.
pub fn gutter_visible(
    point: (f64, f64),
    window: Area,
    viewport: Area,
    message: Area,
    gutter: Area,
) -> bool {
    window.contains(point)
        && viewport.contains(point)
        && (message.contains(point) || gutter.contains(point))
}

#[cfg(test)]
mod gutter_tests {
    use super::*;
    #[test]
    fn first_message_and_full_gutter_reveal_icon_without_hovering_other_messages() {
        let window = Area {
            x: 0.0,
            y: 0.0,
            width: 384.0,
            height: 400.0,
        };
        let viewport = Area {
            y: 20.0,
            height: 300.0,
            ..window
        };
        let message = Area {
            x: 18.0,
            y: 30.0,
            width: 338.0,
            height: 100.0,
        };
        let gutter = Area {
            x: 1.0,
            width: 16.0,
            ..message
        };
        let visible = |p| gutter_visible(p, window, viewport, message, gutter);
        assert!(visible((100.0, 40.0)));
        assert!(visible((8.0, 120.0)));
        assert!(!visible((0.0, 40.0)));
        assert!(!visible((17.0, 40.0)));
        assert!(!visible((8.0, 130.0)));
        assert!(!visible((100.0, 140.0)));
        assert!(!visible((400.0, 40.0)));
        let clipped = Area {
            y: 60.0,
            ..viewport
        };
        assert!(!gutter_visible(
            (8.0, 40.0),
            window,
            clipped,
            message,
            gutter
        ));
        assert_eq!(
            target_at((8.0, 120.0), window, viewport, &[], &[gutter], &[]),
            Some(Target::Button(0))
        );
    }
}
