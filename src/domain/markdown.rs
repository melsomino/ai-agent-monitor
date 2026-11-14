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
    pub table: Option<TableCell>,
    pub table_cell_end: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TableCell {
    pub table: usize,
    pub row: usize,
    pub column: usize,
    pub columns: usize,
    pub alignment: pulldown_cmark::Alignment,
    pub header: bool,
}
struct TableState {
    id: usize,
    row: usize,
    column: usize,
    alignments: Vec<pulldown_cmark::Alignment>,
    header: bool,
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
    let mut table: Option<TableState> = None;
    let mut table_id = 0;
    for event in Parser::new_ext(
        source.trim_end(),
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES,
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
                    Tag::Table(alignments) => {
                        newline(&mut runs, 2, &style);
                        table_id += 1;
                        table = Some(TableState {
                            id: table_id,
                            row: 0,
                            column: 0,
                            alignments,
                            header: true,
                        });
                    }
                    Tag::TableCell => {
                        if let Some(table) = &table {
                            style.table = Some(TableCell {
                                table: table.id,
                                row: table.row,
                                column: table.column,
                                columns: table.alignments.len(),
                                alignment: table.alignments[table.column],
                                header: table.header,
                            });
                            style.bold |= table.header;
                        }
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
                    TagEnd::TableCell => {
                        if let Some(table) = &mut table {
                            let mut separator = style.clone();
                            separator.table_cell_end = true;
                            push(
                                &mut runs,
                                if table.column + 1 == table.alignments.len() {
                                    "\n"
                                } else {
                                    "  │  "
                                },
                                &separator,
                            );
                            table.column += 1;
                        }
                    }
                    TagEnd::TableHead | TagEnd::TableRow => {
                        if let Some(table) = &mut table {
                            table.row += 1;
                            table.column = 0;
                            table.header = false;
                        }
                    }
                    TagEnd::Table => {
                        table = None;
                        newline(&mut runs, 2, &Style::default());
                    }
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
        if last.style.table_cell_end {
            break;
        }
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
            parse("## Summary 🤖\n\n**bold and *italic*** `a ** b`\n\n```rust\nlet x = 1;\n```\n");
        assert_eq!(
            plain(&runs),
            "Summary 🤖\n\nbold and italic a ** b\n\nlet x = 1;"
        );
        assert!(
            runs.iter()
                .any(|r| r.text == "italic" && r.style.bold && r.style.italic)
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
    #[test]
    fn tables_keep_columns_alignment_and_inline_styles() {
        let runs = parse(
            "Before\n\n| Process | Memory |\n| :--- | ---: |\n| Dart 8651 | **1.46 GB** |\n| rust-analyzer 8285 | `1.01 GB` |\n\nAfter",
        );
        assert_eq!(
            plain(&runs),
            "Before\n\nProcess  │  Memory\nDart 8651  │  1.46 GB\nrust-analyzer 8285  │  1.01 GB\n\nAfter"
        );
        assert!(runs.iter().any(|r| r.text == "Memory"
            && r.style.bold
            && r.style.table.as_ref().is_some_and(|c| c.header
                && c.row == 0
                && c.column == 1
                && c.columns == 2
                && c.alignment == pulldown_cmark::Alignment::Right)));
        assert!(runs.iter().any(|r| r.text == "1.46 GB"
            && r.style.bold
            && r.style.table.as_ref().is_some_and(|c| c.row == 1)));
        assert!(runs.iter().any(|r| r.text == "1.01 GB" && r.style.code));
        assert_eq!(runs.iter().filter(|r| r.style.table_cell_end).count(), 6);
    }
    #[test]
    fn tables_preserve_empty_cells_escaped_pipes_and_separate_table_identity() {
        let runs = parse(
            "| A | B |\n| --- | --- |\n| a \\| b | |\n\nBetween\n\n| C | D |\n| --- | --- |\n| x | y |",
        );
        assert!(plain(&runs).contains("a | b"));
        assert!(runs.iter().any(|r| {
            r.style.table_cell_end
                && r.style
                    .table
                    .as_ref()
                    .is_some_and(|c| c.table == 1 && c.row == 1 && c.column == 1)
        }));
        assert!(runs.iter().any(|r| {
            r.text == "C"
                && r.style
                    .table
                    .as_ref()
                    .is_some_and(|c| c.table == 2 && c.row == 0)
        }));
        assert!(runs.last().unwrap().style.table_cell_end);
    }
    #[test]
    fn code_and_malformed_pipes_are_not_tables() {
        for text in [
            "```text\n| A | B |\n| --- | --- |\n```",
            "| A | B |\nordinary text",
        ] {
            assert!(parse(text).iter().all(|r| r.style.table.is_none()));
        }
    }
}
