//! Native work-caption shimmer. Rasterize glyphs/halo once, then move a small
//! alpha band over the cached image; no WebView, per-frame blur or transcript I/O.
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained, runtime::Bool,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::cell::{Cell, RefCell};

const PAD: f64 = 8.0;
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

// CSS ease-in-out (0.42,0,0.58,1), a five-second pass and one-second pause.
fn position(seconds: f64) -> Option<f64> {
    let t = seconds.rem_euclid(6.0) / 5.0;
    if t >= 1.0 {
        return None;
    }
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..16 {
        let u = (low + high) / 2.0;
        let x = 3.0 * (1.0 - u) * (1.0 - u) * u * 0.42 + 3.0 * (1.0 - u) * u * u * 0.58 + u * u * u;
        if x < t {
            low = u;
        } else {
            high = u;
        }
    }
    let u = (low + high) / 2.0;
    let center = -0.5 + 2.0 * (3.0 * u * u - 2.0 * u * u * u);
    (-0.18..=1.18).contains(&center).then_some(center)
}

#[derive(Debug)]
pub(super) struct GlowState {
    text: String,
    width: f64,
    height: f64,
    phase: Cell<Option<f64>>,
    tick: Cell<u64>,
    sprite: RefCell<Option<Retained<NSImage>>>,
}
define_class!(
    // SAFETY: Main-thread, click-through decoration over a native selectable label.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars=GlowState]
    #[derive(Debug)]
    pub(super) struct WorkGlow;
    unsafe impl NSObjectProtocol for WorkGlow {}
    impl WorkGlow {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self,_point:NSPoint)->Option<Retained<NSView>> {None}
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {self.ivars().sprite.borrow_mut().take();self.setNeedsDisplay(true);}
        #[unsafe(method(drawRect:))]
        fn draw(&self,_dirty:NSRect) {
            let state=self.ivars();let Some(phase)=state.phase.get() else {return;};
            let mut cached=state.sprite.borrow_mut();
            if cached.is_none() {*cached=self.make_sprite();}
            let Some(sprite)=cached.as_ref() else {return;};
            let center=PAD+phase*state.width;
            let radius=state.width*0.18;
            let left=(center-radius).max(0.0);let right=(center+radius).min(self.bounds().size.width);
            if right<=left {return;}
            // Small source rectangles reuse the same cached green glyph/halo
            // texture. The base selectable label remains untouched underneath.
            let step=(right-left)/64.0;
            for index in 0..64 {
                let x=left+index as f64*step;
                let alpha=(1.0-((x+step/2.0-center)/radius).abs()).clamp(0.0,1.0);
                let strip=rect(x,0.0,step,self.bounds().size.height);
                sprite.drawInRect_fromRect_operation_fraction(strip,strip,NSCompositingOperation::SourceOver,alpha);
            }
        }
    }
);

fn label(mtm: MainThreadMarker, text: &str, width: f64) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    label.setTextColor(Some(&super::visual::tertiary_text_color()));
    label.setMaximumNumberOfLines(1);
    if let Some(cell) = label.cell() {
        cell.setWraps(false);
        cell.setScrollable(false);
        cell.setUsesSingleLineMode(true);
        cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    }
    label.setSelectable(true);
    label.setToolTip(Some(&NSString::from_str(text)));
    label.setFrame(rect(0.0, 0.0, width, 16.0));
    label
}
fn image_from_bitmap(bitmap: &NSBitmapImageRep, size: NSSize) -> Retained<NSImage> {
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    image.addRepresentation(bitmap);
    image
}
impl WorkGlow {
    fn make_sprite(&self) -> Option<Retained<NSImage>> {
        let state = self.ivars();
        let appearance = self.effectiveAppearance();
        let glyph = label(self.mtm(), &state.text, state.width);
        glyph.setAppearance(Some(&appearance));
        glyph.setTextColor(Some(&super::visual::appearance_color(
            [33.0, 156.0, 115.0],
            [176.0, 255.0, 218.0],
            1.0,
        )));
        let bitmap = glyph.bitmapImageRepForCachingDisplayInRect(glyph.bounds())?;
        glyph.cacheDisplayInRect_toBitmapImageRep(glyph.bounds(), &bitmap);
        let ink = image_from_bitmap(&bitmap, glyph.bounds().size);
        let size = self.bounds().size;
        let width = state.width;
        let height = state.height;
        let draw = block2::RcBlock::new(move |_bounds: NSRect| {
            NSGraphicsContext::saveGraphicsState_class();
            let glow = NSShadow::new();
            glow.setShadowOffset(NSSize::new(0.0, 0.0));
            glow.setShadowBlurRadius(4.0);
            glow.setShadowColor(Some(&super::visual::appearance_color(
                [22.0, 135.0, 100.0],
                [77.0, 235.0, 168.0],
                0.65,
            )));
            glow.set();
            ink.drawInRect(rect(PAD, PAD, width, height));
            NSGraphicsContext::restoreGraphicsState_class();
            Bool::YES
        });
        let image = NSImage::imageWithSize_flipped_drawingHandler(size, false, &draw);
        let view = NSImageView::initWithFrame(
            NSImageView::alloc(self.mtm()),
            rect(0.0, 0.0, size.width, size.height),
        );
        view.setAppearance(Some(&appearance));
        view.setImage(Some(&image));
        let raster = view.bitmapImageRepForCachingDisplayInRect(view.bounds())?;
        view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), &raster);
        Some(image_from_bitmap(&raster, size))
    }
    pub(super) fn advance(&self, seconds: f64, animate: bool) {
        let tick = (seconds * 30.0) as u64;
        let phase = if animate && self.window().is_some() && !self.isHiddenOrHasHiddenAncestor() {
            position(seconds)
        } else {
            None
        };
        if animate && self.ivars().tick.replace(tick) == tick {
            return;
        }
        if self.ivars().phase.replace(phase) != phase {
            self.setNeedsDisplay(true);
        }
    }
    pub(super) fn release_sprite(&self) {
        self.ivars().sprite.borrow_mut().take();
        self.ivars().phase.set(None);
        self.setNeedsDisplay(true);
    }
}
pub(super) fn new(
    mtm: MainThreadMarker,
    text: &str,
    width: f64,
) -> (Retained<NSView>, Retained<WorkGlow>) {
    let label = label(mtm, text, width);
    let size = label.frame().size;
    let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, size.width, size.height));
    view.setClipsToBounds(false);
    view.addSubview(&label);
    let state = GlowState {
        text: text.into(),
        width,
        height: size.height,
        phase: Cell::new(None),
        tick: Cell::new(u64::MAX),
        sprite: RefCell::new(None),
    };
    // SAFETY: Initialize the decorative view after all main-thread state.
    let glow: Retained<WorkGlow> = unsafe {
        msg_send![super(WorkGlow::alloc(mtm).set_ivars(state)),initWithFrame:rect(-PAD,-PAD,width+2.0*PAD,size.height+2.0*PAD)]
    };
    // SAFETY: The duplicate visual layer is not an accessibility element.
    unsafe {
        let _: () = msg_send![&*glow,setAccessibilityElement:false];
    }
    view.addSubview(&glow);
    (view, glow)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sweep_moves_left_to_right_and_rests_for_one_second() {
        let a = position(1.5).unwrap();
        let b = position(2.5).unwrap();
        let c = position(3.5).unwrap();
        assert!(a < b && b < c);
        assert!((b - 0.5).abs() < 0.001);
        assert!(position(5.0).is_none() && position(5.9).is_none());
        assert_eq!(position(2.5), position(8.5));
    }
}
