//! Small session marker using the front robot from the menu bar artwork.
use crate::agents::sessions::Activity;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSAffineTransformNSAppKitAdditions, NSBezierPath, NSColor, NSGraphicsContext, NSView,
    NSWindingRule,
};
use objc2_foundation::{NSAffineTransform, NSObjectProtocol, NSPoint, NSRect, NSSize};

#[derive(Debug)]
pub(super) struct State {
    activity: Activity,
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
            let scale = self.bounds().size.width / 22.0;
            let transform = NSAffineTransform::transform();
            transform.translateXBy_yBy(0.8 * scale, (self.bounds().size.height - 16.0 * scale) / 2.0);
            transform.scaleBy(scale);
            transform.concat();
            let neutral = NSColor::secondaryLabelColor();
            neutral.colorWithAlphaComponent(0.6).setFill();
            let head = rounded(1.4, 0.4, 17.2, 15.2, 4.8);
            head.setWindingRule(NSWindingRule::EvenOdd);
            head.appendBezierPath(&rounded(3.0, 3.4, 14.0, 9.2, 3.2));
            head.fill();
            rounded(0.0, 4.9, 3.2, 6.2, 1.3).fill();
            rounded(17.2, 4.9, 3.2, 6.2, 1.3).fill();
            let eyes = match self.ivars().activity {
                Activity::Working => NSColor::systemGreenColor(),
                Activity::Confirmation => NSColor::systemOrangeColor(),
                _ => neutral.colorWithAlphaComponent(0.6),
            };
            eyes.setFill();
            for x in [7.45, 12.55] {
                NSBezierPath::bezierPathWithOvalInRect(frame(x - 1.8, 6.2, 3.6, 3.6)).fill();
            }
            NSGraphicsContext::restoreGraphicsState_class();
        }
    }
);

impl SessionRobot {
    pub(super) fn bottom_contour_in(&self, parent: &NSView) -> NSPoint {
        let scale = self.bounds().size.width / 22.0;
        let bottom = (self.bounds().size.height - 16.0 * scale) / 2.0 + 0.4 * scale;
        parent.convertPoint_fromView(
            NSPoint::new(self.bounds().size.width / 2.0, bottom),
            Some(self),
        )
    }

    pub(super) fn new(mtm: MainThreadMarker, frame: NSRect, activity: Activity) -> Retained<Self> {
        let allocated = Self::alloc(mtm).set_ivars(State { activity });
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
