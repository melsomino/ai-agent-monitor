//! AppKit glass presentation. No monitor, filesystem settings or interaction transport.
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
};

pub(super) const BRAND_HEIGHT: f64 = 88.0;
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

// Decorative surfaces retain native controls, hit testing and focus ownership.
pub(super) fn gradient(path: &NSBezierPath, start: &NSColor, end: &NSColor, angle: f64) {
    if let Some(fill) =
        NSGradient::initWithStartingColor_endingColor(NSGradient::alloc(), start, end)
    {
        fill.drawInBezierPath_angle(path, angle);
    }
}

fn radial(path: &NSBezierPath, color: &NSColor, center: NSPoint) {
    if let Some(fill) = NSGradient::initWithStartingColor_endingColor(
        NSGradient::alloc(),
        color,
        &NSColor::clearColor(),
    ) {
        fill.drawInBezierPath_relativeCenterPosition(path, center);
    }
}

pub(super) fn command_text(label: &NSTextField, text: &str) {
    let value = NSMutableAttributedString::initWithString(
        NSMutableAttributedString::alloc(),
        &NSString::from_str(text),
    );
    let range = NSRange::new(0, text.encode_utf16().count());
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setMinimumLineHeight(15.0);
    paragraph.setMaximumLineHeight(15.0);
    paragraph.setLineBreakMode(NSLineBreakMode::ByCharWrapping);
    // SAFETY: Values use their documented AppKit attribute types and UTF-16 range.
    unsafe {
        value.addAttribute_value_range(
            NSFontAttributeName,
            &NSFont::monospacedSystemFontOfSize_weight(10.0, 0.0),
            range,
        );
        value.addAttribute_value_range(
            NSForegroundColorAttributeName,
            &secondary_text_color(),
            range,
        );
        value.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
    }
    label.setAttributedStringValue(&value);
}

pub(super) fn draw_menu_tint(bounds: NSRect) {
    appearance_color([244.0, 240.0, 249.0], [18.0, 16.0, 32.0], 1.0).setFill();
    NSBezierPath::bezierPathWithRect(bounds).fill();
}

pub(super) fn stops(colors: &[(Retained<NSColor>, f64)]) -> Option<Retained<NSGradient>> {
    let locations: Vec<_> = colors.iter().map(|(_, position)| *position).collect();
    let colors = NSArray::from_slice(&colors.iter().map(|(color, _)| &**color).collect::<Vec<_>>());
    // SAFETY: One live location per retained color, consumed synchronously by AppKit.
    unsafe {
        NSGradient::initWithColors_atLocations_colorSpace(
            NSGradient::alloc(),
            &colors,
            locations.as_ptr(),
            &NSColorSpace::sRGBColorSpace(),
        )
    }
}

fn ellipse_light(
    clip: &NSBezierPath,
    center: NSPoint,
    radii: NSSize,
    colors: &[(Retained<NSColor>, f64)],
) {
    let Some(gradient) = stops(colors) else {
        return;
    };
    NSGraphicsContext::saveGraphicsState_class();
    clip.addClip();
    let transform = NSAffineTransform::transform();
    transform.translateXBy_yBy(center.x, center.y);
    transform.scaleXBy_yBy(radii.width.max(0.1), radii.height.max(0.1));
    transform.concat();
    gradient.drawFromCenter_radius_toCenter_radius_options(
        NSPoint::new(0.0, 0.0),
        0.0,
        NSPoint::new(0.0, 0.0),
        1.0,
        NSGradientDrawingOptions::DrawsAfterEndingLocation,
    );
    NSGraphicsContext::restoreGraphicsState_class();
}

pub(super) fn is_dark(view: &NSView) -> bool {
    // SAFETY: Standard immutable AppKit appearance names.
    unsafe {
        view.effectiveAppearance()
            .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[
                NSAppearanceNameDarkAqua,
                NSAppearanceNameAqua,
            ]))
            .is_some_and(|name| &*name == NSAppearanceNameDarkAqua)
    }
}

#[derive(Debug)]
pub(super) struct MeshState {
    texture: Rc<OnceCell<Retained<NSImage>>>,
    offset: Cell<f64>,
    document_height: Cell<f64>,
    source_x: Cell<f64>,
    source_width: Cell<f64>,
    brand_wave: Cell<bool>,
}

define_class!(
    // SAFETY: Main-thread decorative view; input always passes through.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars=MeshState]
    #[derive(Debug)]
    pub(super) struct MeshView;
    unsafe impl NSObjectProtocol for MeshView {}
    impl MeshView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self)->bool { true }
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self,_point:NSPoint)->Option<Retained<NSView>> { None }
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) { self.setNeedsDisplay(true); }
        #[unsafe(method(drawRect:))]
        fn draw_mesh(&self,_dirty:NSRect) {
            let bounds=self.bounds();
            draw_menu_tint(bounds);
            NSGraphicsContext::saveGraphicsState_class();
            NSBezierPath::bezierPathWithRect(bounds).addClip();
            let transform=NSAffineTransform::transform();
            transform.translateXBy_yBy(-self.ivars().source_x.get(),0.0);
            transform.concat();
            let width=self.ivars().source_width.get().max(1.0);
            let offset=self.ivars().offset.get();
            let height=self.ivars().document_height.get().max(1.0);
            // Native NSMenu padding extends beyond the document at both ends.
            // Paint those slices in the same document coordinates, rather than
            // clipping the mesh to the document and exposing the flat tint.
            let viewport=NSBezierPath::bezierPathWithRect(rect(self.ivars().source_x.get(),0.0,bounds.size.width,bounds.size.height));
            if let Some(fill)=stops(&[
                (appearance_color([246.0,246.0,249.0],[20.0,21.0,28.0],1.0),0.0),
                (appearance_color([239.0,240.0,244.0],[18.0,19.0,25.0],1.0),0.45),
                (appearance_color([243.0,243.0,247.0],[23.0,23.0,31.0],1.0),1.0),
            ]) {
                let angle=55.0_f64.to_radians();
                let (sin,cos)=angle.sin_cos();
                let radius=(width*cos.abs()+height*sin.abs())/2.0;
                let center=NSPoint::new(width/2.0,height/2.0-offset);
                fill.drawFromPoint_toPoint_options(
                    NSPoint::new(center.x-radius*cos,center.y-radius*sin),
                    NSPoint::new(center.x+radius*cos,center.y+radius*sin),
                    NSGradientDrawingOptions::DrawsBeforeStartingLocation | NSGradientDrawingOptions::DrawsAfterEndingLocation,
                );
            }
            for (x,y,rx,ry,light,dark,alpha) in [
                (0.08,0.0,285.0,190.0,[162.0,149.0,207.0],[116.0,98.0,179.0],0.15),
                (0.98,570.0,245.0,260.0,[180.0,140.0,204.0],[109.0,60.0,156.0],0.13),
                (0.15,940.0,330.0,230.0,[165.0,151.0,207.0],[99.0,81.0,155.0],0.14),
            ] {
                ellipse_light(&viewport,NSPoint::new(x*width,y-offset),NSSize::new(rx*0.8,ry*0.8),&[(appearance_color(light,dark,alpha),0.0),(NSColor::clearColor(),1.0)]);
            }
            let image=self.ivars().texture.get_or_init(|| {
                NSImage::initWithData(NSImage::alloc(),&NSData::with_bytes(include_bytes!("../../assets/monitor-mesh.png"))).expect("valid bundled mesh texture")
            });
            let scale=width/460.0*1.08;
            let count=(height/(1000.0*width/460.0)).ceil().max(1.0) as usize;
            let first=((offset-128.0*scale)/(1000.0*scale)).floor().max(0.0) as usize;
            let last=(((offset+bounds.size.height+128.0*scale)/(1000.0*scale)).ceil().max(0.0) as usize).min(count);
            for tile in first..last {
                NSGraphicsContext::saveGraphicsState_class();
                if tile%2==1 {
                    let flip=NSAffineTransform::transform();
                    flip.translateXBy_yBy(width,0.0);
                    flip.scaleXBy_yBy(-1.0,1.0);
                    flip.concat();
                }
                // SAFETY: No drawing hints; the immutable image has a valid bitmap representation.
                unsafe { image.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
                    rect((width-width*1.08)/2.0,(tile as f64*1000.0-128.0)*scale-offset,width*1.08,1256.0*scale),
                    NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(0.0,0.0)),NSCompositingOperation::SourceOver,
                    if is_dark(self) {0.62} else {0.32},true,None); }
                NSGraphicsContext::restoreGraphicsState_class();
            }
            if self.ivars().brand_wave.get() && offset<BRAND_HEIGHT+32.0 {
                NSGraphicsContext::saveGraphicsState_class();
                let wave=NSAffineTransform::transform();
                wave.translateXBy_yBy(0.0,BRAND_HEIGHT-offset);
                wave.scaleXBy_yBy(1.0,-1.0);
                wave.concat();
                draw_brand_wave(width, is_dark(self));
                NSGraphicsContext::restoreGraphicsState_class();
            }
            NSGraphicsContext::restoreGraphicsState_class();
        }
    }
);

impl MeshView {
    pub(super) fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        Self::with_texture(mtm, frame, Rc::new(OnceCell::new()))
    }
    fn with_texture(
        mtm: MainThreadMarker,
        frame: NSRect,
        texture: Rc<OnceCell<Retained<NSImage>>>,
    ) -> Retained<Self> {
        let state = MeshState {
            texture,
            offset: Cell::new(0.0),
            document_height: Cell::new(frame.size.height),
            source_x: Cell::new(0.0),
            source_width: Cell::new(frame.size.width),
            brand_wave: Cell::new(true),
        };
        // SAFETY: NSView initializer with fully initialized, main-thread-only state.
        unsafe { msg_send![super(Self::alloc(mtm).set_ivars(state)),initWithFrame:frame] }
    }
    pub(super) fn sibling(&self, mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        Self::with_texture(mtm, frame, self.ivars().texture.clone())
    }
    pub(super) fn set_brand_wave(&self, visible: bool) {
        if self.ivars().brand_wave.replace(visible) != visible {
            self.setNeedsDisplay(true);
        }
    }
    pub(super) fn sync(&self, x: f64, offset: f64, height: f64, width: f64) {
        let changed = [
            self.ivars().source_x.replace(x) != x,
            self.ivars().offset.replace(offset) != offset,
            self.ivars().document_height.replace(height) != height,
            self.ivars().source_width.replace(width) != width,
        ]
        .into_iter()
        .any(|changed| changed);
        if changed {
            self.setNeedsDisplay(true);
        }
    }
}

fn draw_brand_wave(width: f64, dark: bool) {
    let upper = brand_wave(width, 0.0);
    upper.lineToPoint(NSPoint::new(width, BRAND_HEIGHT));
    upper.lineToPoint(NSPoint::new(0.0, BRAND_HEIGHT));
    upper.closePath();
    ellipse_light(
        &upper,
        NSPoint::new(252.0 * width / 460.0, 23.0 * 44.0 / 60.0),
        NSSize::new(124.0 * width / 460.0, 22.0 * 44.0 / 60.0),
        &[
            (
                appearance_color(
                    [199.0, 175.0, 224.0],
                    [219.0, 124.0, 222.0],
                    if dark { 0.5 } else { 0.16 },
                ),
                0.0,
            ),
            (NSColor::clearColor(), 1.0),
        ],
    );
    let lower = brand_wave(width, 0.0);
    lower.lineToPoint(NSPoint::new(width, 0.0));
    lower.lineToPoint(NSPoint::new(0.0, 0.0));
    lower.closePath();
    gradient(
        &lower,
        &NSColor::clearColor(),
        &appearance_color(
            [153.0, 133.0, 184.0],
            [11.0, 9.0, 20.0],
            if dark { 0.58 } else { 0.07 },
        ),
        90.0,
    );
    // One narrow contour, rather than two filled shapes sharing a baseline.
    // Coincident antialiased closing edges can leave a straight gradient seam.
    let rim = brand_wave_rim(width, 0.65);
    if let Some(fill) = stops(&[
        (
            appearance_color([57.0, 48.0, 79.0], [57.0, 48.0, 79.0], 0.0),
            0.0,
        ),
        (
            appearance_color(
                [99.0, 65.0, 111.0],
                [99.0, 65.0, 111.0],
                0.35 * if dark { 1.0 } else { 0.5 },
            ),
            0.21,
        ),
        (
            appearance_color(
                [214.0, 130.0, 198.0],
                [214.0, 130.0, 198.0],
                0.85 * if dark { 1.0 } else { 0.5 },
            ),
            0.42,
        ),
        (
            appearance_color(
                [239.0, 171.0, 235.0],
                [239.0, 171.0, 235.0],
                if dark { 1.0 } else { 0.5 },
            ),
            0.53,
        ),
        (
            appearance_color(
                [197.0, 121.0, 243.0],
                [197.0, 121.0, 243.0],
                0.9 * if dark { 1.0 } else { 0.5 },
            ),
            0.62,
        ),
        (
            appearance_color(
                [121.0, 80.0, 194.0],
                [121.0, 80.0, 194.0],
                0.65 * if dark { 1.0 } else { 0.5 },
            ),
            0.77,
        ),
        (NSColor::clearColor(), 1.0),
    ]) {
        fill.drawInBezierPath_angle(&rim, 0.0);
    }
}

type WavePoint = (f64, f64);
type WaveSegment = (WavePoint, WavePoint, WavePoint);
const BRAND_WAVE_SEGMENTS: [WaveSegment; 6] = [
    ((38.0, 44.0), (59.0, 38.0), (86.0, 34.0)),
    ((117.0, 28.0), (137.0, 47.0), (179.0, 44.0)),
    ((211.0, 43.0), (236.0, 30.0), (264.0, 29.0)),
    ((292.0, 27.0), (303.0, 43.0), (337.0, 44.0)),
    ((371.0, 46.0), (393.0, 31.0), (421.0, 35.0)),
    ((440.0, 37.0), (449.0, 40.0), (460.0, 39.0)),
];

fn brand_wave_rim(width: f64, thickness: f64) -> Retained<NSBezierPath> {
    let rim = brand_wave(width, 0.0);
    let point =
        |p: (f64, f64)| NSPoint::new(p.0 * width / 460.0, (60.0 - p.1) * 44.0 / 60.0 + thickness);
    rim.lineToPoint(point((460.0, 39.0)));
    for index in (0..BRAND_WAVE_SEGMENTS.len()).rev() {
        let (c1, c2, _) = BRAND_WAVE_SEGMENTS[index];
        let end = if index == 0 {
            (0.0, 38.0)
        } else {
            BRAND_WAVE_SEGMENTS[index - 1].2
        };
        rim.curveToPoint_controlPoint1_controlPoint2(point(end), point(c2), point(c1));
    }
    rim.closePath();
    rim
}

// The wave uses the prototype's actual control points in its 460×60 viewBox.
fn brand_wave(width: f64, offset: f64) -> Retained<NSBezierPath> {
    let point = |x: f64, y: f64| NSPoint::new(x * width / 460.0, (60.0 - y) * 44.0 / 60.0 + offset);
    let path = NSBezierPath::bezierPath();
    path.moveToPoint(point(0.0, 38.0));
    for (c1, c2, end) in BRAND_WAVE_SEGMENTS {
        path.curveToPoint_controlPoint1_controlPoint2(
            point(end.0, end.1),
            point(c1.0, c1.1),
            point(c2.0, c2.1),
        );
    }
    path
}

fn rounded_ring(bounds: NSRect, radius: f64) -> Retained<NSBezierPath> {
    let ring = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, radius, radius);
    ring.appendBezierPath(&NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
        rect(
            1.0,
            1.0,
            (bounds.size.width - 2.0).max(0.0),
            (bounds.size.height - 2.0).max(0.0),
        ),
        radius - 1.0,
        radius - 1.0,
    ));
    ring.setWindingRule(NSWindingRule::EvenOdd);
    ring
}

fn draw_quota(bounds: NSRect) {
    let w = bounds.size.width;
    let h = bounds.size.height;
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 18.0, 18.0);
    if let Some(fill) = stops(&[
        (
            appearance_color([229.0, 222.0, 238.0], [40.0, 35.0, 62.0], 1.0),
            0.0,
        ),
        (
            appearance_color([235.0, 229.0, 242.0], [33.0, 28.0, 48.0], 1.0),
            0.72,
        ),
        (
            appearance_color([235.0, 229.0, 242.0], [33.0, 28.0, 48.0], 1.0),
            1.0,
        ),
    ]) {
        fill.drawInBezierPath_angle(&path, 0.0);
    }
    ellipse_light(
        &path,
        NSPoint::new(0.0, h * 1.1),
        NSSize::new(w * 0.9, h * 1.7),
        &[
            (
                appearance_color([163.0, 148.0, 211.0], [130.0, 118.0, 197.0], 0.18),
                0.0,
            ),
            (NSColor::clearColor(), 0.7),
        ],
    );
    ellipse_light(
        &path,
        NSPoint::new(w, -h * 0.1),
        NSSize::new(w * 0.7, h * 1.5),
        &[
            (
                appearance_color([177.0, 140.0, 204.0], [139.0, 88.0, 180.0], 0.078),
                0.0,
            ),
            (NSColor::clearColor(), 0.75),
        ],
    );
    gradient(
        &path,
        &NSColor::clearColor(),
        &appearance_color([181.0, 164.0, 230.0], [181.0, 164.0, 230.0], 0.027),
        90.0,
    );
    let rim = rounded_ring(bounds, 18.0);
    gradient(
        &rim,
        &appearance_color([84.0, 68.0, 102.0], [84.0, 68.0, 102.0], 0.145),
        &appearance_color([75.0, 59.0, 97.0], [75.0, 59.0, 97.0], 0.125),
        0.0,
    );
    ellipse_light(
        &rim,
        NSPoint::new(0.0, h),
        NSSize::new(w * 0.7, h * 1.5),
        &[
            (
                appearance_color([150.0, 132.0, 193.0], [163.0, 141.0, 229.0], 0.65),
                0.0,
            ),
            (
                appearance_color([127.0, 105.0, 188.0], [127.0, 105.0, 188.0], 0.22),
                0.4,
            ),
            (NSColor::clearColor(), 0.75),
        ],
    );
    ellipse_light(
        &rim,
        NSPoint::new(w, 0.0),
        NSSize::new(w * 0.85, h * 1.8),
        &[
            (
                appearance_color([165.0, 120.0, 191.0], [180.0, 124.0, 206.0], 0.57),
                0.0,
            ),
            (
                appearance_color([127.0, 85.0, 154.0], [127.0, 85.0, 154.0], 0.22),
                0.45,
            ),
            (NSColor::clearColor(), 0.8),
        ],
    );
}

pub(super) fn quota_value_color() -> Retained<NSColor> {
    appearance_color([119.0, 99.0, 133.0], [192.0, 175.0, 216.0], 1.0)
}
pub(super) fn tertiary_text_color() -> Retained<NSColor> {
    appearance_color([138.0, 126.0, 155.0], [147.0, 136.0, 175.0], 1.0)
}

pub(super) fn draw_thread_line(
    x: f64,
    top: f64,
    bottom: f64,
    gaps: &[(f64, f64)],
    request: bool,
    working: bool,
) {
    if top <= bottom {
        return;
    }
    let mask = NSBezierPath::bezierPath();
    let mut cursor = top;
    for &(center, radius) in gaps {
        let lower = (center + radius).max(bottom);
        if cursor > lower {
            mask.appendBezierPathWithRect(rect(x - 0.5, lower, 1.0, cursor - lower));
        }
        cursor = cursor.min(center - radius);
    }
    if cursor > bottom {
        mask.appendBezierPathWithRect(rect(x - 0.5, bottom, 1.0, cursor - bottom));
    }
    let colors = if request {
        vec![
            (
                appearance_color([255.0, 182.0, 78.0], [255.0, 182.0, 78.0], 0.322),
                0.0,
            ),
            (
                appearance_color([255.0, 168.0, 71.0], [255.0, 168.0, 71.0], 0.651),
                0.3,
            ),
            (
                appearance_color([184.0, 128.0, 221.0], [184.0, 128.0, 221.0], 0.145),
                0.7,
            ),
            (NSColor::clearColor(), 1.0),
        ]
    } else {
        vec![
            (
                if working {
                    appearance_color([70.0, 140.0, 155.0], [140.0, 203.0, 234.0], 0.34)
                } else {
                    appearance_color([109.0, 90.0, 159.0], [163.0, 163.0, 230.0], 0.36)
                },
                0.0,
            ),
            (
                appearance_color([109.0, 90.0, 159.0], [139.0, 127.0, 189.0], 0.18),
                if working { 0.42 } else { 0.45 },
            ),
            (
                appearance_color([139.0, 127.0, 189.0], [139.0, 127.0, 189.0], 0.051),
                0.75,
            ),
            (NSColor::clearColor(), 1.0),
        ]
    };
    if let Some(fill) = stops(&colors) {
        NSGraphicsContext::saveGraphicsState_class();
        mask.addClip();
        fill.drawFromPoint_toPoint_options(
            NSPoint::new(x, top),
            NSPoint::new(x, bottom),
            NSGradientDrawingOptions::empty(),
        );
        NSGraphicsContext::restoreGraphicsState_class();
    }
}

#[derive(Debug)]
pub(super) struct UsageTrackState {
    pub(super) remaining: Option<f64>,
}
define_class!(
    // SAFETY: Immutable progress decoration, always drawn on the main thread.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = UsageTrackState]
    #[derive(Debug)]
    pub(super) struct UsageTrack;
    unsafe impl NSObjectProtocol for UsageTrack {}
    impl UsageTrack {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> { None }
        #[unsafe(method(drawRect:))]
        fn draw_track(&self, _dirty: NSRect) {
            let bounds = self.bounds();
            let track = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds,4.0,4.0);
            appearance_color([153.0,128.0,181.0],[121.0,99.0,156.0],0.24).setFill();
            track.fill();
            if let Some(remaining) = self.ivars().remaining.filter(|value| *value > 0.0) {
                let fill = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    rect(0.0,0.0,bounds.size.width*remaining.clamp(0.0,100.0)/100.0,bounds.size.height),4.0,4.0);
                let rgb=crate::domain::layout::quota_rgb(remaining).map(f64::from);
                gradient(&fill,&appearance_color(rgb,rgb,0.85),&appearance_color(rgb,rgb,1.0),0.0);
                NSGraphicsContext::saveGraphicsState_class();fill.addClip();
                gradient(&NSBezierPath::bezierPathWithRect(rect(0.0,bounds.size.height-1.0,bounds.size.width,1.0)),&NSColor::clearColor(),&appearance_color([239.0,213.0,255.0],[239.0,213.0,255.0],0.27),90.0);
                NSGraphicsContext::restoreGraphicsState_class();
            }
        }
    }
);

#[derive(Clone, Copy, Debug)]
pub(super) enum DecisionGlyph {
    Check,
    Cross,
    Up,
}
#[derive(Debug)]
pub(super) struct DecisionState {
    label: Retained<NSTextField>,
    glyph: DecisionGlyph,
}
define_class!(
    // SAFETY: Click-through decoration follows its retained caption's native color.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars=DecisionState]
    #[derive(Debug)]
    pub(super) struct DecisionIcon;
    unsafe impl NSObjectProtocol for DecisionIcon {}
    impl DecisionIcon {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self,_point:NSPoint)->Option<Retained<NSView>> {None}
        #[unsafe(method(drawRect:))]
        fn draw_glyph(&self,_dirty:NSRect) {
            NSGraphicsContext::saveGraphicsState_class();
            let scale=NSAffineTransform::transform();scale.scaleBy(self.bounds().size.width/24.0);scale.concat();
            self.ivars().label.textColor().unwrap_or_else(primary_text_color).setStroke();
            let path=NSBezierPath::bezierPath();
            match self.ivars().glyph {
                DecisionGlyph::Check=>{path.moveToPoint(NSPoint::new(20.0,18.0));path.lineToPoint(NSPoint::new(9.0,7.0));path.lineToPoint(NSPoint::new(4.0,12.0));},
                DecisionGlyph::Up=>{path.moveToPoint(NSPoint::new(12.0,5.0));path.lineToPoint(NSPoint::new(12.0,19.0));path.moveToPoint(NSPoint::new(6.0,13.0));path.lineToPoint(NSPoint::new(12.0,19.0));path.lineToPoint(NSPoint::new(18.0,13.0));},
                DecisionGlyph::Cross=>{path.moveToPoint(NSPoint::new(18.0,18.0));path.lineToPoint(NSPoint::new(6.0,6.0));path.moveToPoint(NSPoint::new(6.0,18.0));path.lineToPoint(NSPoint::new(18.0,6.0));},
            }
            path.setLineWidth(1.8);path.setLineCapStyle(NSLineCapStyle::Round);path.setLineJoinStyle(NSLineJoinStyle::Round);path.stroke();
            NSGraphicsContext::restoreGraphicsState_class();
        }
    }
);
impl DecisionIcon {
    pub(super) fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        label: Retained<NSTextField>,
        glyph: DecisionGlyph,
    ) -> Retained<Self> {
        // SAFETY: The caption and glyph are initialized before the NSView initializer.
        unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(DecisionState {label,glyph})),initWithFrame:frame]
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct GlassState {
    pub(super) kind: Cell<u8>,
    pub(super) focused: Cell<bool>,
    pub(super) hovered: Cell<bool>,
    pub(super) enabled: Cell<bool>,
}

define_class!(
    // SAFETY: Main-thread decoration; NSBox remains the layout and color API.
    #[unsafe(super(NSBox))]
    #[thread_kind = MainThreadOnly]
    #[ivars = GlassState]
    #[derive(Debug)]
    pub(super) struct GlassBox;
    unsafe impl NSObjectProtocol for GlassBox {}
    impl GlassBox {
        #[unsafe(method(drawRect:))]
        fn draw_glass(&self, _dirty: NSRect) {
            let bounds = self.bounds();
            let radius = self.cornerRadius();
            let kind = self.ivars().kind.get();
            if kind==3 { draw_quota(bounds);return; }
            let fill_bounds = if kind == 4 { rect(4.0,4.0,(bounds.size.width-8.0).max(0.0),(bounds.size.height-8.0).max(0.0)) } else { bounds };
            let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(fill_bounds, radius, radius);
            if matches!(kind, 0 | 5 | 6) && self.ivars().enabled.get()
                && (self.ivars().focused.get() || self.ivars().hovered.get())
            {
                draw_title_hover(fill_bounds, radius, is_dark(self));
                if self.ivars().focused.get() {
                    draw_keyboard_outline(fill_bounds, radius);
                }
                return;
            }
            if kind == 4 {
                let enabled = self.ivars().enabled.get();
                let hovered = self.ivars().hovered.get();
                let alpha = if enabled { 1.0 } else { 0.45 };
                NSGraphicsContext::saveGraphicsState_class();
                let glow=NSShadow::new();
                glow.setShadowColor(Some(&appearance_color([126.0,104.0,255.0],[126.0,104.0,255.0],0.45*alpha)));
                glow.setShadowBlurRadius(6.0);
                glow.setShadowOffset(NSSize::new(0.0,0.0));
                glow.set();
                appearance_color([116.0,85.0,238.0],[116.0,85.0,238.0],alpha).setFill();
                path.fill();
                NSGraphicsContext::restoreGraphicsState_class();
                gradient(&path,
                    &appearance_color([if hovered {132.0} else {116.0},85.0,238.0],[if hovered {132.0} else {116.0},85.0,238.0],alpha),
                    &appearance_color([65.0,77.0,200.0],[65.0,77.0,200.0],alpha),0.0);
                NSGraphicsContext::saveGraphicsState_class();path.addClip();
                gradient(&NSBezierPath::bezierPathWithRect(rect(fill_bounds.origin.x,fill_bounds.origin.y+fill_bounds.size.height*0.76,fill_bounds.size.width,fill_bounds.size.height*0.24)),&NSColor::clearColor(),&NSColor::whiteColor().colorWithAlphaComponent(0.23*alpha),90.0);
                NSGraphicsContext::restoreGraphicsState_class();
                appearance_color([191.0,186.0,255.0],[191.0,186.0,255.0],alpha).setStroke();
                path.setLineWidth(1.0);
                path.stroke();
                if self.ivars().focused.get() && enabled {
                    draw_keyboard_outline(fill_bounds, radius);
                }
                return;
            }
            if kind == 6 {
                if self.ivars().focused.get() {
                    self.fillColor().setFill();path.fill();
                } else {
                    draw_title_hover(bounds, radius, is_dark(self));
                }
                return;
            }
            if kind == 0 || kind == 4 || kind == 5 {
                // Use the supplied semantic fill: keyboard selection must retain
                // the system accent rather than acquiring a second focus color.
                let fill = self.fillColor();
                fill.setFill();
                path.fill();
                if !self.ivars().focused.get() {
                    if self.ivars().hovered.get() {
                        radial(&path,&appearance_color([178.0,232.0,255.0],[178.0,232.0,255.0],0.14),NSPoint::new(-0.8,0.8));
                        radial(&path,&appearance_color([161.0,138.0,255.0],[161.0,138.0,255.0],0.14),NSPoint::new(1.0,-1.0));
                    }
                    gradient(&path, &NSColor::clearColor(),
                        &NSColor::colorWithSRGBRed_green_blue_alpha(0.82, 0.78, 1.0, 0.12), 90.0);
                }
                if kind==5 && !self.ivars().focused.get() {
                    let rim=NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(0.5,0.5,bounds.size.width-1.0,bounds.size.height-1.0),radius,radius);
                    appearance_color([187.0,167.0,245.0],[187.0,167.0,245.0],0.16).setStroke();rim.setLineWidth(1.0);rim.stroke();
                }
                return;
            }
            let (light_start, dark_start, light_end, dark_end) = if kind == 1 {
                ([232.0,224.0,238.0], [33.0,28.0,46.0], [235.0,228.0,240.0], [36.0,31.0,50.0])
            } else if kind == 3 {
                ([229.0,221.0,239.0], [48.0,39.0,63.0], [238.0,232.0,244.0], [33.0,27.0,46.0])
            } else {
                ([244.0,237.0,247.0], [45.0,39.0,56.0], [238.0,227.0,246.0], [44.0,37.0,64.0])
            };
            gradient(&path, &appearance_color(light_start, dark_start, 1.0),
                &appearance_color(light_end, dark_end, 1.0), 0.0);
            if kind == 2 {
                radial(&path,&appearance_color([219.0,161.0,103.0],[217.0,152.0,90.0],0.12),NSPoint::new(-1.0,0.35));
                radial(&path,&appearance_color([171.0,126.0,220.0],[153.0,86.0,206.0],0.16),NSPoint::new(1.0,-0.4));
            }
            // Draw the rim inside the bounds, avoiding clipped half-pixel edges.
            let rim = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                rect(0.5, 0.5, (bounds.size.width-1.0).max(0.0), (bounds.size.height-1.0).max(0.0)), radius, radius);
            rim.setLineWidth(1.0);
            if kind == 2 || kind == 3 {
                NSGraphicsContext::saveGraphicsState_class();
                let inner = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    rect(1.0, 1.0, (bounds.size.width-2.0).max(0.0), (bounds.size.height-2.0).max(0.0)),
                    (radius-1.0).max(0.0), (radius-1.0).max(0.0));
                let ring = NSBezierPath::bezierPath();
                ring.appendBezierPath(&path);
                ring.appendBezierPath(&inner);
                ring.setWindingRule(NSWindingRule::EvenOdd);
                ring.addClip();
                let left_rim = if kind == 3 {
                    appearance_color([157.0,130.0,180.0], [172.0,145.0,209.0], 0.42)
                } else {
                    appearance_color([194.0,145.0,94.0], [237.0,189.0,129.0], 0.65)
                };
                gradient(&path, &left_rim,
                    &appearance_color([157.0,108.0,183.0], [194.0,139.0,214.0], 0.65), 0.0);
                NSGraphicsContext::restoreGraphicsState_class();
            } else {
                appearance_color([145.0,116.0,172.0], [163.0,138.0,203.0], 0.18).setStroke();
                rim.stroke();
            }
        }
    }
);

pub(super) fn glass_box(mtm: MainThreadMarker, frame: NSRect, kind: u8) -> Retained<NSBox> {
    let frame = if kind == 4 {
        rect(
            frame.origin.x - 4.0,
            frame.origin.y - 4.0,
            frame.size.width + 8.0,
            frame.size.height + 8.0,
        )
    } else {
        frame
    };
    let view: Retained<GlassBox> = unsafe {
        msg_send![super(GlassBox::alloc(mtm).set_ivars(GlassState { kind: Cell::new(kind), focused: Cell::new(false), hovered: Cell::new(false), enabled: Cell::new(true) })), initWithFrame: frame]
    };
    view.into_super()
}

pub(super) fn working_status_color() -> Retained<NSColor> {
    // Darker green on light material; keep the brighter green in dark mode.
    appearance_color([22.0, 129.0, 79.0], [70.0, 244.0, 161.0], 1.0)
}

pub(super) fn hover_fill_color() -> Retained<NSColor> {
    // Explicit appearance variants remain visible over the denser menu tint,
    // including when controls were first created in the opposite appearance.
    appearance_color([145.0, 155.0, 193.0], [102.0, 87.0, 139.0], 0.34)
}

pub(super) fn primary_text_color() -> Retained<NSColor> {
    appearance_color([41.0, 36.0, 59.0], [245.0, 242.0, 255.0], 1.0)
}
pub(super) fn secondary_text_color() -> Retained<NSColor> {
    appearance_color([112.0, 101.0, 127.0], [186.0, 178.0, 209.0], 1.0)
}
pub(super) fn request_ink_color() -> Retained<NSColor> {
    appearance_color([255.0; 3], [51.0, 31.875, 0.0], 1.0)
}

pub(super) fn request_color() -> Retained<NSColor> {
    appearance_color([203.0, 119.0, 12.0], [255.0, 182.0, 78.0], 1.0)
}

pub(super) fn appearance_color(
    light_rgb: [f64; 3],
    dark_rgb: [f64; 3],
    alpha: f64,
) -> Retained<NSColor> {
    let color = |rgb: [f64; 3]| {
        NSColor::colorWithSRGBRed_green_blue_alpha(
            rgb[0] / 255.0,
            rgb[1] / 255.0,
            rgb[2] / 255.0,
            alpha,
        )
    };
    let dark = color(dark_rgb);
    let light = color(light_rgb);
    let provider = block2::RcBlock::new(move |appearance: std::ptr::NonNull<NSAppearance>| {
        // SAFETY: AppKit supplies a live appearance; framework name constants
        // are immutable, valid NSStrings for the lifetime of the process.
        let is_dark = unsafe {
            appearance
                .as_ref()
                .bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[
                    NSAppearanceNameDarkAqua,
                    NSAppearanceNameAqua,
                ]))
                .is_some_and(|name| &*name == NSAppearanceNameDarkAqua)
        };
        std::ptr::NonNull::from(if is_dark { &*dark } else { &*light })
    });
    // SAFETY: Both returned colors are retained by the provider's captures.
    // NSColor copies and retains the provider for subsequent appearance changes.
    unsafe { NSColor::colorWithName_dynamicProvider(None, &provider) }
}

#[derive(Debug)]
pub(super) struct MetadataState {
    kind: u8,
}
define_class!(
    // SAFETY: Main-thread, click-through vector decoration for session metadata.
    #[unsafe(super(NSView))]
    #[thread_kind=MainThreadOnly]
    #[ivars=MetadataState]
    #[derive(Debug)]
    pub(super) struct MetadataIcon;
    unsafe impl NSObjectProtocol for MetadataIcon {}
    impl MetadataIcon {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self,_point:NSPoint)->Option<Retained<NSView>> { None }
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self)->bool {true}
        #[unsafe(method(drawRect:))]
        fn draw_metadata(&self,_dirty:NSRect) {
            NSGraphicsContext::saveGraphicsState_class();
            let transform=NSAffineTransform::transform();
            transform.scaleBy(self.bounds().size.width/24.0);transform.concat();
            tertiary_text_color().setStroke();
            let path=NSBezierPath::bezierPath();
            path.setLineWidth(2.0);
            path.setLineCapStyle(NSLineCapStyle::Round);
            path.setLineJoinStyle(NSLineJoinStyle::Round);
            match self.ivars().kind {
                0=>{
                    path.moveToPoint(NSPoint::new(20.0,20.0));
                    path.lineToPoint(NSPoint::new(4.0,20.0));
                    path.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(2.0,18.0),NSPoint::new(2.895,20.0),NSPoint::new(2.0,19.105));
                    path.lineToPoint(NSPoint::new(2.0,6.0));
                    path.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(4.0,4.0),NSPoint::new(2.0,4.895),NSPoint::new(2.895,4.0));
                    path.lineToPoint(NSPoint::new(7.9,4.0));
                    path.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(9.59,4.9),NSPoint::new(8.57,4.0),NSPoint::new(9.19,4.34));
                    path.lineToPoint(NSPoint::new(10.4,6.1));
                    path.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(12.06,7.0),NSPoint::new(10.77,6.66),NSPoint::new(11.39,7.0));
                    path.lineToPoint(NSPoint::new(20.0,7.0));
                    path.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(22.0,9.0),NSPoint::new(21.105,7.0),NSPoint::new(22.0,7.895));
                    path.lineToPoint(NSPoint::new(22.0,18.0));
                    path.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(20.0,20.0),NSPoint::new(22.0,19.105),NSPoint::new(21.105,20.0));
                },
                1=>{
                    path.moveToPoint(NSPoint::new(6.0,3.0));path.lineToPoint(NSPoint::new(6.0,15.0));
                    path.moveToPoint(NSPoint::new(18.0,9.0));
                    path.curveToPoint_controlPoint1_controlPoint2(NSPoint::new(9.0,18.0),NSPoint::new(18.0,13.97),NSPoint::new(13.97,18.0));
                    path.appendBezierPath(&NSBezierPath::bezierPathWithOvalInRect(rect(3.0,15.0,6.0,6.0)));
                    path.appendBezierPath(&NSBezierPath::bezierPathWithOvalInRect(rect(15.0,3.0,6.0,6.0)));
                },
                _=>{
                    path.appendBezierPath(&NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(4.0,4.0,16.0,16.0),2.0,2.0));
                    path.appendBezierPath(&NSBezierPath::bezierPathWithRect(rect(9.0,9.0,6.0,6.0)));
                    for position in [8.0,12.0,16.0] {
                        for (a,b) in [((position,1.0),(position,4.0)),((position,20.0),(position,23.0)),((1.0,position),(4.0,position)),((20.0,position),(23.0,position))] {
                            path.moveToPoint(NSPoint::new(a.0,a.1));path.lineToPoint(NSPoint::new(b.0,b.1));
                        }
                    }
                }
            }
            path.stroke();
            NSGraphicsContext::restoreGraphicsState_class();
        }
    }
);
impl MetadataIcon {
    pub(super) fn new(mtm: MainThreadMarker, frame: NSRect, symbol: &str) -> Retained<Self> {
        let kind = match symbol {
            "folder" => 0,
            "git-branch" => 1,
            _ => 2,
        };
        // SAFETY: NSView initializer with fully initialized immutable state.
        unsafe {
            msg_send![super(Self::alloc(mtm).set_ivars(MetadataState{kind})),initWithFrame:frame]
        }
    }
}

// Shared prototype hover layers, bottom to top; focus adds a theme outline.
fn draw_title_hover(bounds: NSRect, radius: f64, dark: bool) {
    NSGraphicsContext::saveGraphicsState_class();
    let transform = NSAffineTransform::transform();
    transform.translateXBy_yBy(bounds.origin.x, bounds.origin.y);
    transform.concat();
    let bounds = rect(0.0, 0.0, bounds.size.width, bounds.size.height);
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, radius, radius);
    gradient(
        &path,
        &appearance_color(
            [187.0, 201.0, 231.0],
            [69.0, 76.0, 115.0],
            if dark { 158.0 / 255.0 } else { 133.0 / 255.0 },
        ),
        &appearance_color(
            [206.0, 192.0, 234.0],
            [81.0, 65.0, 108.0],
            if dark { 158.0 / 255.0 } else { 133.0 / 255.0 },
        ),
        -20.0,
    );
    ellipse_light(
        &path,
        NSPoint::new(bounds.size.width * 0.96, 0.0),
        NSSize::new(bounds.size.width * 0.842, bounds.size.height * 0.877),
        &[
            (
                appearance_color([161.0, 138.0, 255.0], [161.0, 138.0, 255.0], 48.0 / 255.0),
                0.0,
            ),
            (NSColor::clearColor(), 1.0),
        ],
    );
    ellipse_light(
        &path,
        NSPoint::new(bounds.size.width * 0.09, bounds.size.height * 0.92),
        NSSize::new(bounds.size.width * 0.618, bounds.size.height * 0.625),
        &[
            (
                appearance_color([178.0, 232.0, 255.0], [178.0, 232.0, 255.0], 48.0 / 255.0),
                0.0,
            ),
            (NSColor::clearColor(), 1.0),
        ],
    );
    NSGraphicsContext::saveGraphicsState_class();
    path.addClip();
    // Difference of translated rounded paths reproduces the curved inset edge.
    for (dy, color) in [
        (
            -1.0,
            appearance_color([225.0, 231.0, 255.0], [225.0, 231.0, 255.0], 40.0 / 255.0),
        ),
        (
            1.0,
            appearance_color([169.0, 146.0, 229.0], [169.0, 146.0, 229.0], 28.0 / 255.0),
        ),
    ] {
        let ring = NSBezierPath::bezierPath();
        ring.appendBezierPath(&path);
        ring.appendBezierPath(&NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
            rect(0.0, dy, bounds.size.width, bounds.size.height),
            radius,
            radius,
        ));
        ring.setWindingRule(NSWindingRule::EvenOdd);
        color.setFill();
        ring.fill();
    }
    NSGraphicsContext::restoreGraphicsState_class();
    NSGraphicsContext::restoreGraphicsState_class();
}

/// Shared theme outline; text fields call this without painting a hover fill.
pub(super) fn draw_keyboard_outline(bounds: NSRect, radius: f64) {
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
        rect(
            bounds.origin.x - 2.0,
            bounds.origin.y - 2.0,
            bounds.size.width + 4.0,
            bounds.size.height + 4.0,
        ),
        radius + 2.0,
        radius + 2.0,
    );
    appearance_color([120.0, 96.0, 188.0], [199.0, 181.0, 255.0], 1.0).setStroke();
    path.setLineWidth(2.0);
    path.stroke();
}

pub(super) fn neutral_robot_color() -> Retained<NSColor> {
    appearance_color([112.0, 128.0, 145.0], [148.0, 167.0, 187.0], 1.0)
}
