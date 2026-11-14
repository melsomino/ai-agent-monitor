//! Markdown messages rendered as native attributed text. No HTML or remote assets.
use crate::domain::markdown::parse;
use objc2::{AllocAnyThread, ClassType, MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::*;
use objc2_foundation::*;
use std::collections::HashMap;

fn attributed(
    runs: &[crate::domain::markdown::Run],
    color: &NSColor,
    body_weight: f64,
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
    let mut totals = HashMap::<(usize, usize, usize), usize>::new();
    for run in runs {
        if let Some(cell) = &run.style.table
            && !run.style.table_cell_end
        {
            *totals
                .entry((cell.table, cell.row, cell.column))
                .or_default() += run.text.chars().count();
        }
    }
    let mut widths = HashMap::<(usize, usize), usize>::new();
    for ((table, _, column), length) in totals {
        let value = widths.entry((table, column)).or_insert(8);
        *value = (*value).max(length.clamp(8, 40));
    }
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
            let traits = font.fontDescriptor().symbolicTraits()
                | NSFontDescriptorSymbolicTraits::TraitItalic;
            let descriptor = font
                .fontDescriptor()
                .fontDescriptorWithSymbolicTraits(traits);
            font = NSFont::fontWithDescriptor_size(&descriptor, size).unwrap_or(font);
        }
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
                let total: usize = (0..cell.columns).map(|column| *widths.get(&(cell.table,column)).unwrap_or(&8)).sum();
                block.setContentWidth_type(100.0 * *widths.get(&(cell.table,cell.column)).unwrap_or(&8) as f64 / total.max(1) as f64, NSTextBlockValueType::PercentageValueType);
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

/// Keep ordinary messages on NSTextField; use TextKit only when native tables are needed.
pub(super) fn message(
    source: &str,
    width: f64,
    color: &NSColor,
    request: bool,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let runs = parse(source);
    if !runs.iter().any(|run| run.style.table.is_some()) {
        let label = NSTextField::wrappingLabelWithString(&NSString::from_str(source), mtm);
        label.setSelectable(true);
        label.setMaximumNumberOfLines(0);
        label.setAllowsEditingTextAttributes(true);
        label.setAttributedStringValue(&attributed(&runs, color, if request { 0.3 } else { 0.0 }));
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
        return label.into_super().into_super();
    }
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
            storage.setAttributedString(&attributed(&runs, color, if request { 0.3 } else { 0.0 }));
            manager.ensureLayoutForTextContainer(&container);
            let used = manager.usedRectForTextContainer(&container);
            (used.origin.y + used.size.height).ceil() + 2.0
        } else {
            22.0
        }
    };
    view.setFrameSize(NSSize::new(width, height.max(22.0)));
    view.into_super().into_super()
}
