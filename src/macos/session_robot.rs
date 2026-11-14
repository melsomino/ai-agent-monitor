//! Small session marker using the front robot from the menu bar artwork.
use crate::agents::sessions::Activity;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSAffineTransformNSAppKitAdditions, NSBezierPath, NSGraphicsContext, NSShadow, NSView,
    NSWindingRule,
};
use objc2_foundation::{NSAffineTransform, NSObjectProtocol, NSPoint, NSRect, NSSize};
use std::cell::Cell;

/// Monochrome toolbar artwork; AppKit supplies selected/disabled tinting.
pub(super) fn settings_icon() -> Retained<objc2_app_kit::NSImage> {
    use objc2_app_kit::{NSColor, NSImage};
    let draw = block2::RcBlock::new(|_bounds: NSRect| {
        NSColor::blackColor().setStroke();
        let head = rounded(6.0, 7.0, 20.0, 17.0, 5.0);
        head.setLineWidth(2.0);
        head.stroke();
        for x in [2.5, 26.0] {
            let ear = rounded(x, 12.0, 3.5, 7.0, 1.5);
            ear.setLineWidth(1.5);
            ear.stroke();
        }
        NSColor::blackColor().setFill();
        for x in [11.0, 18.0] {
            NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                NSPoint::new(x, 14.0), NSSize::new(3.0, 4.0),
            )).fill();
        }
        let antenna = NSBezierPath::bezierPath();
        antenna.moveToPoint(NSPoint::new(16.0, 24.0));
        antenna.lineToPoint(NSPoint::new(16.0, 28.0));
        antenna.setLineWidth(2.0);
        antenna.stroke();
        objc2::runtime::Bool::YES
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(32.0, 32.0), false, &draw);
    image.setTemplate(true);
    image.setAccessibilityDescription(Some(objc2_foundation::ns_string!("Agents")));
    image
}

#[derive(Debug)]
pub(super) struct State {
    activity: Activity,
    motion: Cell<crate::robot_motion::Frame>,
    hero: bool,
    artwork_width: f64,
}

define_class!(
    // SAFETY: NSView supports subclassing; state and drawing stay on the main thread.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    #[derive(Debug)]
    pub(super) struct SessionRobot;
    unsafe impl NSObjectProtocol for SessionRobot {}
    impl SessionRobot {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> { None }

        #[unsafe(method(drawRect:))]
        fn draw_robot(&self, _dirty: NSRect) {
            NSGraphicsContext::saveGraphicsState_class();
            if self.ivars().hero {
                draw_hero(self.bounds());
                NSGraphicsContext::restoreGraphicsState_class();
                return;
            }
            let scale = self.ivars().artwork_width / 22.0;
            let transform = NSAffineTransform::transform();
            transform.translateXBy_yBy(self.bounds().size.width / 2.0 - 10.2 * scale, (self.bounds().size.height - 16.0 * scale) / 2.0);
            transform.scaleBy(scale);
            transform.concat();
            let neutral = if self.ivars().activity == Activity::Confirmation { super::visual::request_color() } else { super::visual::neutral_robot_color() };
            let working = self.ivars().activity == Activity::Working;
            if working {
                let glow=NSShadow::new();
                glow.setShadowColor(Some(&super::visual::appearance_color([25.0,151.0,111.0],[68.0,244.0,186.0],0.65)));
                glow.setShadowBlurRadius(9.0/scale);
                glow.setShadowOffset(NSSize::new(0.0,0.0));
                glow.set();
            }
            if working {
                super::visual::appearance_color([24.0,132.0,104.0],[140.0,255.0,213.0],1.0).setFill();
            } else { neutral.setFill(); }
            let head = rounded(1.4, 0.4, 17.2, 15.2, 4.8);
            head.setWindingRule(NSWindingRule::EvenOdd);
            head.appendBezierPath(&rounded(3.0, 3.4, 14.0, 9.2, 3.2));
            if self.ivars().activity == Activity::Confirmation {
                head.fill();
            } else if self.ivars().activity != Activity::Working {
                neutral.setFill();
                head.fill();
            } else {
                // Paint the silhouette first so the shadow is not clipped by the gradient.
                head.fill();
                super::visual::gradient(&head,
                    &super::visual::appearance_color([24.0,132.0,104.0],[75.0,235.0,177.0],1.0),
                    &super::visual::appearance_color([48.0,164.0,141.0],[182.0,255.0,230.0],1.0),90.0);
                super::visual::appearance_color([24.0,132.0,104.0],[140.0,255.0,213.0],1.0).setFill();
            }
            rounded(0.0, 4.9, 3.2, 6.2, 1.3).fill();
            rounded(17.2, 4.9, 3.2, 6.2, 1.3).fill();
            if working {
                let eye_glow = NSShadow::new();
                eye_glow.setShadowColor(Some(&super::visual::working_status_color().colorWithAlphaComponent(0.7)));
                eye_glow.setShadowBlurRadius(3.0 / scale);
                eye_glow.setShadowOffset(NSSize::new(0.0,0.0));
                eye_glow.set();
            }
            let eyes = match self.ivars().activity {
                Activity::Working => super::visual::working_status_color(),
                Activity::Confirmation => super::visual::request_color(),
                _ => if self.bounds().size.width > 20.0 { super::visual::appearance_color([75.0,147.0,194.0],[127.0,231.0,255.0],1.0) } else { neutral.clone() },
            };
            for (index, x) in [7.45, 12.55].into_iter().enumerate() {
                let level = if self.ivars().activity == Activity::Working { self.ivars().motion.get().eyes[index] } else { 1.0 };
                eyes.colorWithAlphaComponent(eyes.alphaComponent() * level).setFill();
                NSBezierPath::bezierPathWithOvalInRect(frame(x - 1.8, 6.2, 3.6, 3.6)).fill();
            }
            NSGraphicsContext::restoreGraphicsState_class();
        }
    }
);

impl SessionRobot {
    pub(super) fn advance(&self, seconds: f64, animate: bool) {
        if self.ivars().activity == Activity::Working {
            self.ivars()
                .motion
                .set(crate::robot_motion::sample(seconds, animate));
            self.setNeedsDisplay(true);
        }
    }

    pub(super) fn bottom_contour_in(&self, parent: &NSView) -> NSPoint {
        let scale = self.ivars().artwork_width / 22.0;
        let bottom = (self.bounds().size.height - 16.0 * scale) / 2.0 + 0.4 * scale;
        parent.convertPoint_fromView(
            NSPoint::new(self.bounds().size.width / 2.0, bottom),
            Some(self),
        )
    }

    pub(super) fn new(mtm: MainThreadMarker, frame: NSRect, activity: Activity) -> Retained<Self> {
        let hero = frame.size.width > 20.0;
        let artwork_width = frame.size.width;
        // Reserve the actual CSS shadow spread without clipping it to the artwork box.
        let frame = if hero {
            NSRect::new(
                NSPoint::new(frame.origin.x - 24.0, frame.origin.y - 24.0),
                NSSize::new(frame.size.width + 48.0, frame.size.height + 48.0),
            )
        } else if activity == Activity::Working {
            NSRect::new(
                NSPoint::new(frame.origin.x - 12.0, frame.origin.y - 12.0),
                NSSize::new(frame.size.width + 24.0, frame.size.height + 24.0),
            )
        } else {
            frame
        };
        let allocated = Self::alloc(mtm).set_ivars(State {
            activity,
            hero,
            artwork_width,
            motion: Cell::new(crate::robot_motion::sample(0.0, false)),
        });
        // SAFETY: NSView's designated initializer with initialized subclass state.
        unsafe { msg_send![super(allocated), initWithFrame: frame] }
    }
}

fn frame(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn rounded(x: f64, y: f64, width: f64, height: f64, radius: f64) -> Retained<NSBezierPath> {
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
        frame(x, y, width, height),
        radius,
        radius,
    )
}

// Header artwork follows the prototype mask, gradient and two ambient halos.
// The small session markers retain their status colors and eye animation.
fn draw_hero(bounds: NSRect) {
    let scale = 40.0 / 22.0;
    let transform = NSAffineTransform::transform();
    transform.translateXBy_yBy(
        bounds.size.width / 2.0 - 10.2 * scale,
        (bounds.size.height - 16.0 * scale) / 2.0,
    );
    transform.scaleBy(scale);
    transform.concat();
    let body = rounded(1.4, 0.4, 17.2, 15.2, 4.8);
    body.appendBezierPath(&rounded(3.0, 3.4, 14.0, 9.2, 3.2).bezierPathByReversingPath());
    body.appendBezierPath(&rounded(0.0, 4.9, 3.2, 6.2, 1.3));
    body.appendBezierPath(&rounded(17.2, 4.9, 3.2, 6.2, 1.3));
    let eyes = NSBezierPath::bezierPath();
    for x in [7.45, 12.55] {
        eyes.appendBezierPath(&NSBezierPath::bezierPathWithOvalInRect(frame(
            x - 1.8,
            6.2,
            3.6,
            3.6,
        )));
    }
    for (blur, rgb, alpha) in [
        (12.0, [101.0, 80.0, 237.0], 0.53),
        (3.0, [165.0, 184.0, 255.0], 0.60),
    ] {
        NSGraphicsContext::saveGraphicsState_class();
        let shadow = NSShadow::new();
        shadow.setShadowOffset(NSSize::new(0.0, 0.0));
        shadow.setShadowBlurRadius(blur);
        shadow.setShadowColor(Some(&super::visual::appearance_color(rgb, rgb, alpha)));
        shadow.set();
        super::visual::appearance_color([140.0, 117.0, 255.0], [140.0, 117.0, 255.0], 1.0)
            .setFill();
        body.fill();
        NSGraphicsContext::restoreGraphicsState_class();
    }
    if let Some(gradient) = super::visual::stops(&[
        (
            super::visual::appearance_color([162.0, 228.0, 255.0], [162.0, 228.0, 255.0], 1.0),
            0.0,
        ),
        (
            super::visual::appearance_color([140.0, 117.0, 255.0], [140.0, 117.0, 255.0], 1.0),
            0.75,
        ),
        (
            super::visual::appearance_color([140.0, 117.0, 255.0], [140.0, 117.0, 255.0], 1.0),
            1.0,
        ),
    ]) {
        gradient.drawInBezierPath_angle(&body, -60.0);
    }
    let cyan = super::visual::appearance_color([127.0, 231.0, 255.0], [127.0, 231.0, 255.0], 1.0);
    let shadow = NSShadow::new();
    shadow.setShadowOffset(NSSize::new(0.0, 0.0));
    shadow.setShadowBlurRadius(4.0);
    shadow.setShadowColor(Some(&cyan));
    shadow.set();
    cyan.setFill();
    eyes.fill();
}
