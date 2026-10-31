//! Markdown messages rendered as native attributed text. No HTML or remote assets.
use objc2::AllocAnyThread;
use objc2_app_kit::*;
use objc2_foundation::*;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, Default)]
struct Style {
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    heading: u8,
    depth: usize,
    quote: bool,
    link: Option<String>,
}

#[derive(Debug)]
struct Run {
    text: String,
    style: Style,
}

fn push(runs: &mut Vec<Run>, text: impl Into<String>, style: &Style) {
    runs.push(Run {
        text: text.into(),
        style: style.clone(),
    });
}

fn newline(runs: &mut Vec<Run>, count: usize, style: &Style) {
    let existing = runs
        .iter()
        .rev()
        .flat_map(|r| r.text.chars().rev())
        .take_while(|c| *c == '\n')
        .count();
    if !runs.is_empty() && existing < count {
        push(runs, "\n".repeat(count - existing), style);
    }
}

fn parse(source: &str) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut style = Style::default();
    let mut stack = Vec::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    for event in Parser::new_ext(
        source.trim_end(),
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    ) {
        match event {
            Event::Start(tag) => {
                stack.push(style.clone());
                match tag {
                    Tag::Heading { level, .. } => {
                        newline(&mut runs, 2, &style);
                        style.heading = level as u8;
                        style.bold = true;
                    }
                    Tag::Paragraph => {}
                    Tag::Strong => style.bold = true,
                    Tag::Emphasis => style.italic = true,
                    Tag::Strikethrough => style.strike = true,
                    Tag::CodeBlock(_) => {
                        newline(&mut runs, 2, &style);
                        style.code = true;
                    }
                    Tag::BlockQuote(_) => {
                        newline(&mut runs, 1, &style);
                        style.quote = true;
                    }
                    Tag::Link { dest_url, .. } => {
                        // Local paths and command URLs remain text, never executable links.
                        if let Ok(url) = url::Url::parse(&dest_url)
                            && matches!(url.scheme(), "http" | "https" | "mailto")
                        {
                            style.link = Some(dest_url.into_string());
                        }
                    }
                    Tag::List(start) => {
                        newline(&mut runs, if lists.is_empty() { 2 } else { 1 }, &style);
                        lists.push(start);
                        style.depth = lists.len();
                    }
                    Tag::Item => {
                        newline(&mut runs, 1, &style);
                        let marker = match lists.last_mut() {
                            Some(Some(n)) => {
                                let marker = format!("{n}. ");
                                *n = n.saturating_add(1);
                                marker
                            }
                            _ => "• ".to_owned(),
                        };
                        push(&mut runs, marker, &style);
                    }
                    _ => {}
                }
            }
            Event::End(tag) => {
                match tag {
                    TagEnd::Heading(_) | TagEnd::CodeBlock => newline(&mut runs, 2, &style),
                    TagEnd::Paragraph => {
                        newline(&mut runs, if lists.is_empty() { 2 } else { 1 }, &style)
                    }
                    TagEnd::Item | TagEnd::BlockQuote(_) => newline(&mut runs, 1, &style),
                    TagEnd::List(_) => {
                        lists.pop();
                        newline(&mut runs, if lists.is_empty() { 2 } else { 1 }, &style);
                    }
                    _ => {}
                }
                style = stack.pop().unwrap_or_default();
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                push(&mut runs, text.into_string(), &style)
            }
            Event::Code(text) => {
                let mut code = style.clone();
                code.code = true;
                push(&mut runs, text.into_string(), &code);
            }
            Event::SoftBreak => push(&mut runs, " ", &style),
            Event::HardBreak => push(&mut runs, "\n", &style),
            Event::Rule => {
                newline(&mut runs, 2, &style);
                push(&mut runs, "────────", &style);
                newline(&mut runs, 2, &style);
            }
            Event::TaskListMarker(done) => push(&mut runs, if done { "☑ " } else { "☐ " }, &style),
            _ => {}
        }
    }
    while let Some(last) = runs.last_mut() {
        last.text.truncate(last.text.trim_end().len());
        if !last.text.is_empty() {
            break;
        }
        runs.pop();
    }
    runs
}

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

#[cfg(test)]
mod tests {
    use super::*;
    fn plain(runs: &[Run]) -> String {
        runs.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn nested_styles_unicode_and_literal_code() {
        let runs =
            parse("## Итог 🤖\n\n**жирный и *курсив*** `a ** b`\n\n```rust\nlet x = 1;\n```\n");
        assert_eq!(
            plain(&runs),
            "Итог 🤖\n\nжирный и курсив a ** b\n\nlet x = 1;"
        );
        assert!(
            runs.iter()
                .any(|r| r.text == "курсив" && r.style.bold && r.style.italic)
        );
        assert!(runs.iter().any(|r| r.text == "a ** b" && r.style.code));
        assert_eq!(runs[0].style.heading, 2);
    }

    #[test]
    fn lists_keep_order_and_nesting_without_trailing_space() {
        let runs = parse("3. first\n4. second\n   - child\n");
        assert_eq!(plain(&runs), "3. first\n4. second\n• child");
        assert!(runs.iter().any(|r| r.text == "child" && r.style.depth == 2));
    }

    #[test]
    fn unsafe_links_and_html_are_inert_text() {
        let runs =
            parse("[site](https://example.com) [run](command:run) <script>alert(1)</script>");
        assert!(
            runs.iter()
                .any(|r| r.text == "site" && r.style.link.is_some())
        );
        assert!(
            runs.iter()
                .any(|r| r.text == "run" && r.style.link.is_none())
        );
        assert!(plain(&runs).contains("<script>"));
        assert_eq!(plain(&parse("**incomplete")), "**incomplete");
    }
}
