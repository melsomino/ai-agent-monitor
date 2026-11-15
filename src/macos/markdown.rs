//! Markdown messages rendered as native attributed text. No HTML or remote assets.
use crate::domain::markdown::parse;
use objc2::{
    AllocAnyThread, ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::collections::HashMap;

fn run_font(run: &crate::domain::markdown::Run, body_weight: f64) -> Retained<NSFont> {
    let size = match run.style.heading {
        1 => 16.0,
        2 => 15.0,
        3 => 14.0,
        _ => 13.0,
    };
    let mut font = if run.style.code {
        NSFont::monospacedSystemFontOfSize_weight(12.0, if run.style.bold { 0.3 } else { 0.0 })
    } else if run.style.bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize_weight(size, body_weight)
    };
    if run.style.italic {
        let traits =
            font.fontDescriptor().symbolicTraits() | NSFontDescriptorSymbolicTraits::TraitItalic;
        let descriptor = font
            .fontDescriptor()
            .fontDescriptorWithSymbolicTraits(traits);
        font = NSFont::fontWithDescriptor_size(&descriptor, size).unwrap_or(font);
    }
    font
}

/// Measure each unwrapped cell using the same fonts as its displayed runs.
fn column_widths(
    runs: &[crate::domain::markdown::Run],
    weight: f64,
    mtm: MainThreadMarker,
) -> HashMap<(usize, usize), f64> {
    let mut cells = HashMap::<(usize, usize, usize), f64>::new();
    for run in runs {
        if let Some(cell) = &run.style.table {
            let width = cells
                .entry((cell.table, cell.row, cell.column))
                .or_default();
            if run.style.table_cell_end {
                continue;
            }
            let measure = NSTextField::labelWithString(&NSString::from_str(&run.text), mtm);
            measure.setFont(Some(&run_font(run, weight)));
            measure.sizeToFit();
            *width += (measure.frame().size.width - 4.0).max(0.0);
        }
    }
    let mut columns = HashMap::<(usize, usize), f64>::new();
    for ((table, _, column), width) in cells {
        let entry = columns.entry((table, column)).or_insert(36.0);
        *entry = entry.max(width + 12.0);
    }
    columns
}

define_class!(
    // SAFETY: Main-thread document with top-to-bottom message layout.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    struct MessageDocument;
    unsafe impl NSObjectProtocol for MessageDocument {}
    impl MessageDocument {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
    }
);
define_class!(
    // SAFETY: Horizontal input stays local; vertical input belongs to the session list.
    #[unsafe(super(NSScrollView))]
    #[thread_kind = MainThreadOnly]
    struct TableScroll;
    unsafe impl NSObjectProtocol for TableScroll {}
    impl TableScroll {
        #[unsafe(method(scrollWheel:))]
        fn wheel(&self, event: &NSEvent) {
            if event.scrollingDeltaX().abs() > event.scrollingDeltaY().abs()
                || event.modifierFlags().contains(NSEventModifierFlags::Shift) {
                // SAFETY: Let NSScrollView handle its native horizontal scrolling.
                unsafe { let _: () = msg_send![super(self), scrollWheel: event]; }
            } else {
                super::menu::route_wheel(self, event);
            }
        }
    }
);

fn attributed(
    runs: &[crate::domain::markdown::Run],
    color: &NSColor,
    body_weight: f64,
    mtm: MainThreadMarker,
) -> Retained<NSMutableAttributedString> {
    let plain: String = runs
        .iter()
        .map(|run| {
            if run.style.table_cell_end {
                "\n"
            } else {
                run.text.as_str()
            }
        })
        .collect();
    let value = NSMutableAttributedString::initWithString(
        NSMutableAttributedString::alloc(),
        &NSString::from_str(&plain),
    );
    let widths = column_widths(runs, body_weight, mtm);
    let mut tables = HashMap::<usize, Retained<NSTextTable>>::new();
    let mut cells = HashMap::<(usize, usize, usize), Retained<NSTextTableBlock>>::new();
    let mut offset = 0;
    for run in runs {
        let range = NSRange::new(
            offset,
            if run.style.table_cell_end {
                1
            } else {
                run.text.encode_utf16().count()
            },
        );
        offset += range.length;
        let font = run_font(run, body_weight);
        let paragraph = NSMutableParagraphStyle::new();
        paragraph.setLineSpacing(3.0);
        if let Some(cell) = &run.style.table {
            let table = tables.entry(cell.table).or_insert_with(|| {
                let table = NSTextTable::new();
                table.setNumberOfColumns(cell.columns);
                table.setLayoutAlgorithm(NSTextTableLayoutAlgorithm::FixedLayoutAlgorithm);
                table.setCollapsesBorders(true);
                table.setContentWidth_type(100.0, NSTextBlockValueType::PercentageValueType);
                table
            });
            let block = cells.entry((cell.table,cell.row,cell.column)).or_insert_with(|| {
                let block = NSTextTableBlock::initWithTable_startingRow_rowSpan_startingColumn_columnSpan(
                    NSTextTableBlock::alloc(),table,cell.row as isize,1,cell.column as isize,1);
                let total: f64 = (0..cell.columns).map(|column| *widths.get(&(cell.table,column)).unwrap_or(&36.0)).sum();
                block.setContentWidth_type(100.0 * *widths.get(&(cell.table,cell.column)).unwrap_or(&36.0) / total.max(1.0), NSTextBlockValueType::PercentageValueType);
                block.setWidth_type_forLayer(5.0,NSTextBlockValueType::AbsoluteValueType,NSTextBlockLayer::Padding);
                block.setWidth_type_forLayer(0.5,NSTextBlockValueType::AbsoluteValueType,NSTextBlockLayer::Border);
                block.setBorderColor(Some(&super::visual::appearance_color([111.0,124.0,141.0],[157.0,169.0,191.0],0.35)));
                block.setVerticalAlignment(NSTextBlockVerticalAlignment::TopAlignment);
                if cell.header { block.setBackgroundColor(Some(&super::visual::appearance_color([85.0,97.0,117.0],[198.0,210.0,234.0],0.08))); }
                block
            });
            paragraph.setTextBlocks(&NSArray::from_slice(&[block.as_super()]));
            paragraph.setLineBreakMode(NSLineBreakMode::ByCharWrapping);
            paragraph.setAlignment(match cell.alignment {
                pulldown_cmark::Alignment::Right => NSTextAlignment::Right,
                pulldown_cmark::Alignment::Center => NSTextAlignment::Center,
                _ => NSTextAlignment::Left,
            });
        }

        let indent = run.style.depth as f64 * 12.0 + if run.style.quote { 10.0 } else { 0.0 };
        paragraph.setFirstLineHeadIndent(indent);
        paragraph.setHeadIndent(indent + if run.style.depth > 0 { 14.0 } else { 0.0 });
        // SAFETY: AppKit attributes have their documented types; offsets count UTF-16,
        // including surrogate pairs, in the exact string used to create `value`.
        unsafe {
            value.addAttribute_value_range(NSFontAttributeName, &font, range);
            value.addAttribute_value_range(NSForegroundColorAttributeName, color, range);
            value.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
            if run.style.code {
                value.addAttribute_value_range(
                    NSBackgroundColorAttributeName,
                    &NSColor::secondaryLabelColor().colorWithAlphaComponent(0.10),
                    range,
                );
            }
            if run.style.strike {
                value.addAttribute_value_range(
                    NSStrikethroughStyleAttributeName,
                    &NSNumber::new_i32(1),
                    range,
                );
            }
            if let Some(link) = &run.style.link {
                value.addAttribute_value_range(
                    NSLinkAttributeName,
                    &NSString::from_str(link),
                    range,
                );
                value.addAttribute_value_range(
                    NSForegroundColorAttributeName,
                    &NSColor::linkColor(),
                    range,
                );
            }
        }
    }
    value
}

fn prose(
    runs: &[crate::domain::markdown::Run],
    width: f64,
    color: &NSColor,
    request: bool,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let label = NSTextField::wrappingLabelWithString(ns_string!(""), mtm);
    label.setSelectable(true);
    label.setMaximumNumberOfLines(0);
    label.setAllowsEditingTextAttributes(true);
    label.setAttributedStringValue(&attributed(
        runs,
        color,
        if request { 0.3 } else { 0.0 },
        mtm,
    ));
    let height = label
        .cell()
        .map(|cell| {
            cell.cellSizeForBounds(NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(width, 1000000.0),
            ))
            .height
            .ceil()
        })
        .unwrap_or(22.0)
        .max(22.0);
    label.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(width, height),
    ));
    label.into_super().into_super()
}

fn table_text(
    runs: &[crate::domain::markdown::Run],
    width: f64,
    color: &NSColor,
    request: bool,
    mtm: MainThreadMarker,
) -> Retained<NSTextView> {
    let view = NSTextView::initWithFrame(
        NSTextView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, 1000000.0)),
    );
    view.setEditable(false);
    view.setSelectable(true);
    view.setDrawsBackground(false);
    view.setVerticallyResizable(false);
    view.setHorizontallyResizable(false);
    view.setTextContainerInset(NSSize::new(0.0, 0.0));
    // SAFETY: This fresh NSTextView owns its TextKit 1 storage/manager/container;
    // they are used only on the main thread and retained by the view.
    let height = unsafe {
        if let (Some(storage), Some(manager), Some(container)) = (
            view.textStorage(),
            view.layoutManager(),
            view.textContainer(),
        ) {
            container.setLineFragmentPadding(0.0);
            storage.setAttributedString(&attributed(
                runs,
                color,
                if request { 0.3 } else { 0.0 },
                mtm,
            ));
            manager.ensureLayoutForTextContainer(&container);
            let used = manager.usedRectForTextContainer(&container);
            (used.origin.y + used.size.height).ceil() + 2.0
        } else {
            22.0
        }
    };
    view.setFrameSize(NSSize::new(width, height.max(22.0)));
    view
}

/// Tables scroll independently; surrounding prose retains the message's wrapping width.
pub(super) fn message(
    source: &str,
    width: f64,
    color: &NSColor,
    request: bool,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let runs = parse(source);
    if !runs.iter().any(|r| r.style.table.is_some()) {
        return prose(&runs, width, color, request, mtm);
    }
    // SAFETY: Standard initialization of an ivar-free document view.
    let document: Retained<MessageDocument> = unsafe {
        msg_send![MessageDocument::alloc(mtm), initWithFrame: NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(width,1.0))]
    };
    let mut y = 0.0;
    for group in runs.chunk_by(|a, b| {
        a.style.table.as_ref().map(|c| c.table) == b.style.table.as_ref().map(|c| c.table)
    }) {
        let child: Retained<NSView> = if group[0].style.table.is_some() {
            let natural: f64 = column_widths(group, if request { 0.3 } else { 0.0 }, mtm)
                .values()
                .sum();
            let table_width = natural.ceil().clamp(width, width * 2.0);
            let text = table_text(group, table_width, color, request, mtm);
            let overflow = table_width > width + 0.5;
            let height = text.frame().size.height + if overflow { 14.0 } else { 0.0 };
            // SAFETY: Standard NSScrollView initialization for the horizontal-only subclass.
            let scroll: Retained<TableScroll> = unsafe {
                msg_send![TableScroll::alloc(mtm), initWithFrame: NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(width,height))]
            };
            scroll.setDrawsBackground(false);
            scroll.contentView().setDrawsBackground(false);
            scroll.setHasVerticalScroller(false);
            scroll.setHasHorizontalScroller(overflow);
            scroll.setAutohidesScrollers(true);
            scroll.setScrollerStyle(NSScrollerStyle::Overlay);
            scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
            scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
            scroll.setDocumentView(Some(&text));
            scroll.into_super().into_super()
        } else {
            let mut trimmed: Vec<_> = group
                .iter()
                .map(|r| crate::domain::markdown::Run {
                    text: r.text.clone(),
                    style: r.style.clone(),
                })
                .collect();
            if let Some(first) = trimmed.first_mut() {
                first.text = first.text.trim_start_matches('\n').into();
            }
            if let Some(last) = trimmed.last_mut() {
                last.text = last.text.trim_end_matches('\n').into();
            }
            if trimmed.iter().all(|r| r.text.trim().is_empty()) {
                continue;
            }
            prose(&trimmed, width, color, request, mtm)
        };
        if y > 0.0 {
            y += 10.0;
        }
        child.setFrameOrigin(NSPoint::new(0.0, y));
        y += child.frame().size.height;
        document.addSubview(&child);
    }
    document.setFrameSize(NSSize::new(width, y.max(22.0)));
    document.into_super()
}
