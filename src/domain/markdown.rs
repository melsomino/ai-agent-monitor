//! Shared Markdown projection for native and remote message renderers.
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, Default)]
pub(crate) struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub block_code: Option<String>,
    pub strike: bool,
    pub heading: u8,
    pub depth: usize,
    pub quote: bool,
    pub link: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Run {
    pub text: String,
    pub style: Style,
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

pub(crate) fn parse(source: &str) -> Vec<Run> {
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
                    Tag::CodeBlock(kind) => {
                        newline(&mut runs, 2, &style);
                        style.code = true;
                        style.block_code = Some(match kind {
                            pulldown_cmark::CodeBlockKind::Fenced(info) => info
                                .split_whitespace()
                                .next()
                                .unwrap_or_default()
                                .chars()
                                .take(64)
                                .collect(),
                            pulldown_cmark::CodeBlockKind::Indented => String::new(),
                        });
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
