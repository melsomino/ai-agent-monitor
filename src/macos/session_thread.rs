//! Expanded-session gutter. One drawing owns the line and request contour so
//! their joints share the same coordinates during resizing and disclosure.
use super::session_layout;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{NSBezierPath, NSColor, NSGraphicsContext, NSView};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize};

#[derive(Debug)]
pub(super) struct State {
    /// Distance from the row's top to the robot's actual bottom contour.
    start: f64,
    /// Distance from the row's top to the request indicator's center.
    request: Option<f64>,
    messages: Vec<f64>,
}

define_class!(
    // SAFETY: Decoration-only NSView; all state and drawing are main-thread confined.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    #[derive(Debug)]
    pub(super) struct SessionThread;
    unsafe impl NSObjectProtocol for SessionThread {}
    impl SessionThread {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> { None }

        #[unsafe(method(drawRect:))]
        fn draw_thread(&self, _dirty: NSRect) {
            let height = self.bounds().size.height;
            let x = session_layout::ICON_LEFT + session_layout::ICON_SIZE / 2.0;
            let top = height - self.ivars().start;
            let bottom = 12.0;
            if top <= bottom { return; }
            NSGraphicsContext::saveGraphicsState_class();
            NSBezierPath::bezierPathWithRect(self.bounds()).addClip();
            NSColor::labelColor().colorWithAlphaComponent(0.10).setFill();
            let request = self.ivars().request.map(|distance| height - distance);
            let mut gaps: Vec<_> = self.ivars().messages.iter()
                .map(|distance| (height - distance, 2.5)).collect();
            if let Some(center) = request { gaps.push((center, 7.5)); }
            gaps.sort_by(|a, b| b.0.total_cmp(&a.0));
            let mut cursor = top;
            for (center, radius) in gaps {
                segment(x, (center + radius).max(bottom), cursor);
                cursor = cursor.min(center - radius);
            }
            segment(x, bottom, cursor);
            NSColor::labelColor().colorWithAlphaComponent(0.25).setFill();
            for distance in &self.ivars().messages {
                let center = height - distance;
                NSBezierPath::bezierPathWithOvalInRect(rect(x - 2.5, center - 2.5, 5.0, 5.0)).fill();
            }
            if let Some(center) = request {
                NSColor::systemOrangeColor().setFill();
                NSBezierPath::bezierPathWithOvalInRect(rect(x - 7.5, center - 7.5, 15.0, 15.0)).fill();
                let ink = NSColor::colorWithSRGBRed_green_blue_alpha(0.2, 0.125, 0.0, 1.0);
                ink.setStroke();
                ink.setFill();
                let stem = NSBezierPath::bezierPath();
                stem.setLineWidth(1.7);
                stem.moveToPoint(NSPoint::new(x, center + 3.0));
                stem.lineToPoint(NSPoint::new(x, center - 0.5));
                stem.stroke();
                NSBezierPath::bezierPathWithOvalInRect(rect(x - 0.7, center - 3.5, 1.4, 1.4)).fill();
            }
            NSGraphicsContext::restoreGraphicsState_class();
        }
    }
);

impl SessionThread {
    pub(super) fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        start: f64,
        request: Option<f64>,
        messages: Vec<f64>,
    ) -> Retained<Self> {
        let allocated = Self::alloc(mtm).set_ivars(State {
            start,
            request,
            messages,
        });
        // SAFETY: NSView's designated initializer, with initialized subclass state.
        unsafe { msg_send![super(allocated), initWithFrame: frame] }
    }
}

fn segment(x: f64, bottom: f64, top: f64) {
    if top > bottom {
        NSBezierPath::bezierPathWithRect(rect(x - 0.5, bottom, 1.0, top - bottom)).fill();
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}
