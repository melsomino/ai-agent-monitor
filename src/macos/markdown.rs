//! Markdown messages rendered as native attributed text. No HTML or remote assets.
use crate::domain::markdown::parse;
use objc2::AllocAnyThread;
use objc2_app_kit::*;
use objc2_foundation::*;

pub fn apply(label: &NSTextField, color: &NSColor) {
    let runs = parse(&label.stringValue().to_string());
    let plain: String = runs.iter().map(|run| run.text.as_str()).collect();
    let value = NSMutableAttributedString::initWithString(
        NSMutableAttributedString::alloc(),
        &NSString::from_str(&plain),
    );
    let mut offset = 0;
    for run in runs {
        let range = NSRange::new(offset, run.text.encode_utf16().count());
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
            NSFont::systemFontOfSize(size)
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
    label.setAllowsEditingTextAttributes(true);
    label.setAttributedStringValue(&value);
}
