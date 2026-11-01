//! Telegram entities over rendered text, never over Markdown source or byte offsets.
use crate::domain::markdown::parse;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize)]
pub(super) struct Entity {
    #[serde(rename = "type")]
    kind: &'static str,
    offset: usize,
    length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Text {
    pub text: String,
    entities: Vec<Entity>,
    units: usize,
}
impl Text {
    pub fn plain(&mut self, text: &str) {
        self.units += text.encode_utf16().count();
        self.text.push_str(text);
    }
    pub fn bold(&mut self, text: &str) {
        self.literal(text, "bold");
    }
    pub fn code(&mut self, text: &str) {
        self.literal(text, "code");
    }
    pub fn code_block(&mut self, text: &str, language: Option<&str>) {
        let offset = self.units;
        self.plain(text);
        if self.units > offset {
            self.entities.push(Entity {
                kind: "pre",
                offset,
                length: self.units - offset,
                url: None,
                language: language
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
            });
        }
    }
    fn literal(&mut self, text: &str, kind: &'static str) {
        let offset = self.units;
        self.plain(text);
        if self.units > offset {
            self.entities.push(Entity {
                kind,
                offset,
                length: self.units - offset,
                url: None,
                language: None,
            });
        }
    }
    pub fn append(&mut self, other: Self) {
        let offset = self.units;
        self.plain(&other.text);
        self.entities
            .extend(other.entities.into_iter().map(|mut entity| {
                entity.offset += offset;
                entity
            }));
    }
    pub fn append_limited(&mut self, other: Self, limit: usize) {
        self.append(other.bounded(limit));
    }
    pub fn markdown(&mut self, source: &str, limit: usize) {
        self.append(Self::from_markdown(source).bounded(limit));
    }
    fn from_markdown(source: &str) -> Self {
        let mut text = Self::default();
        let mut last = BTreeMap::new();
        for run in parse(source) {
            if run.style.depth > 1
                && !run.style.code
                && (text.text.is_empty() || text.text.ends_with('\n'))
            {
                text.plain(&"  ".repeat(run.style.depth.saturating_sub(1).min(20)));
            }
            let offset = text.units;
            text.plain(&run.text);
            let length = text.units - offset;
            if length == 0 {
                continue;
            }
            let style = run.style;
            let mut formats = Vec::new();
            if style.quote {
                formats.push(("blockquote", None, None));
            }
            if style.code {
                formats.push(if let Some(language) = style.block_code {
                    ("pre", None, (!language.is_empty()).then_some(language))
                } else {
                    ("code", None, None)
                });
            } else {
                if style.bold {
                    formats.push(("bold", None, None));
                }
                if style.italic {
                    formats.push(("italic", None, None));
                }
                if style.strike {
                    formats.push(("strikethrough", None, None));
                }
                if let Some(url) = style.link {
                    formats.push(("text_link", Some(url), None));
                }
            }
            for (kind, url, language) in formats {
                let key = (kind, url.clone(), language.clone());
                if let Some(&index) = last.get(&key) {
                    let previous: &mut Entity = &mut text.entities[index];
                    if previous.offset + previous.length == offset {
                        previous.length += length;
                        continue;
                    }
                }
                last.insert(key, text.entities.len());
                text.entities.push(Entity {
                    kind,
                    offset,
                    length,
                    url,
                    language,
                });
            }
        }
        // Telegram disallows quote nesting with code/pre/link entities. Keep the
        // inner content's useful formatting when that combination can't be expressed.
        let exclusive: Vec<_> = text
            .entities
            .iter()
            .filter(|e| matches!(e.kind, "code" | "pre" | "text_link"))
            .map(|e| (e.offset, e.offset + e.length))
            .collect();
        text.entities.retain(|e| {
            e.kind != "blockquote"
                || !exclusive
                    .iter()
                    .any(|(start, end)| e.offset < *end && *start < e.offset + e.length)
        });
        text
    }
    fn bounded(mut self, limit: usize) -> Self {
        if self.units <= limit {
            return self;
        }
        let mut units = 0;
        let end = self
            .text
            .char_indices()
            .find_map(|(byte, ch)| {
                if units + ch.len_utf16() > limit.saturating_sub(1) {
                    Some(byte)
                } else {
                    units += ch.len_utf16();
                    None
                }
            })
            .unwrap_or(self.text.len());
        self.text.truncate(end);
        self.entities.retain_mut(|entity| {
            if entity.offset >= units {
                return false;
            }
            entity.length = entity.length.min(units - entity.offset);
            entity.length > 0
        });
        if limit > 0 {
            self.text.push('…');
            units += 1;
        }
        self.units = units;
        self
    }
    pub fn payload(self, keyboard: Value) -> Value {
        let mut text = self.bounded(3900);
        text.entities.sort_by(|a, b| {
            a.offset
                .cmp(&b.offset)
                .then_with(|| b.length.cmp(&a.length))
                .then_with(|| a.kind.cmp(b.kind))
        });
        text.entities.truncate(100);
        json!({"text":text.text,"entities":text.entities,"reply_markup":keyboard})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn span(text: &Text, entity: &Entity) -> String {
        String::from_utf16(
            &text
                .text
                .encode_utf16()
                .skip(entity.offset)
                .take(entity.length)
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }
    #[test]
    fn nested_styles_and_links_use_utf16_after_removing_markup() {
        let text = Text::from_markdown("🤖 **Привет *мир*** и [ссылка](https://example.com)");
        assert_eq!(text.text, "🤖 Привет мир и ссылка");
        let bold = text.entities.iter().find(|e| e.kind == "bold").unwrap();
        assert_eq!(bold.offset, 3);
        assert_eq!(span(&text, bold), "Привет мир");
        assert_eq!(
            span(
                &text,
                text.entities.iter().find(|e| e.kind == "italic").unwrap()
            ),
            "мир"
        );
        let link = text
            .entities
            .iter()
            .find(|e| e.kind == "text_link")
            .unwrap();
        assert_eq!(span(&text, link), "ссылка");
        assert_eq!(link.url.as_deref(), Some("https://example.com"));
    }
    #[test]
    fn code_is_literal_and_code_blocks_keep_their_language() {
        let text = Text::from_markdown("**`a * b`**\n\n```rust\nlet x = \"🤖\";\n```\n");
        let code = text.entities.iter().find(|e| e.kind == "code").unwrap();
        assert_eq!(span(&text, code), "a * b");
        let pre = text.entities.iter().find(|e| e.kind == "pre").unwrap();
        assert_eq!(pre.language.as_deref(), Some("rust"));
        assert!(span(&text, pre).contains("let x"));
        assert!(!text.entities.iter().any(|e| e.kind == "bold"));
    }
    #[test]
    fn truncation_keeps_unicode_and_entities_inside_the_actual_text() {
        let text = Text::from_markdown("**🤖привет🤖**").bounded(6);
        assert_eq!(text.text, "🤖при…");
        assert_eq!(text.units, 6);
        assert_eq!(span(&text, &text.entities[0]), "🤖при");
        let mut wrapped = Text::default();
        wrapped.plain("Заголовок 🤖\n");
        wrapped.append(text);
        wrapped.plain("\nFooter");
        assert_eq!(span(&wrapped, &wrapped.entities[0]), "🤖при");
        let payload = wrapped.payload(json!({"inline_keyboard":[]}));
        assert!(payload.get("parse_mode").is_none());
        assert_eq!(payload["entities"][0]["length"], 5);
    }
    #[test]
    fn entity_nesting_respects_telegram_rules_and_dense_text_is_bounded() {
        for source in [
            "**bold [link *italic*](https://example.com) end**",
            "> **bold** and *italic*\n> second line",
            "> quote `code` [link](https://example.com)",
            "**before `code` after**",
            "[before `code` after](https://example.com)",
        ] {
            let text = Text::from_markdown(source);
            for (i, a) in text.entities.iter().enumerate() {
                for b in text.entities.iter().skip(i + 1) {
                    let ae = a.offset + a.length;
                    let be = b.offset + b.length;
                    if a.offset < be && b.offset < ae {
                        assert!(
                            (a.offset <= b.offset && ae >= be)
                                || (b.offset <= a.offset && be >= ae)
                        );
                        assert!(
                            !matches!(a.kind, "code" | "pre") && !matches!(b.kind, "code" | "pre")
                        );
                        if !matches!(a.kind, "bold" | "italic" | "strikethrough") {
                            assert!(matches!(b.kind, "bold" | "italic" | "strikethrough"));
                        }
                    }
                }
            }
        }
        let payload = Text::from_markdown(&"**a** plain *b* `c` ".repeat(1000)).payload(json!({}));
        assert!(payload["text"].as_str().unwrap().encode_utf16().count() <= 3900);
        assert!(payload["entities"].as_array().unwrap().len() <= 100);
    }
    #[test]
    fn headers_lists_quotes_and_unsupported_markup_stay_readable() {
        let text = Text::from_markdown(
            "## Heading\n\n> **Quote**\n\n- item\n  - nested\n\ntext <script>x</script> [run](command:run)",
        );
        assert!(text.text.contains("Heading\n\nQuote"));
        assert!(text.text.contains("• item\n  • nested"));
        assert!(text.text.contains("<script>x</script> run"));
        assert!(text.entities.iter().any(|e| e.kind == "blockquote"));
        assert!(!text.entities.iter().any(|e| e.kind == "text_link"));
        let mixed = Text::from_markdown("> **quote** `code` [link](https://example.com)");
        assert!(!mixed.entities.iter().any(|e| e.kind == "blockquote"));
        assert!(mixed.entities.iter().any(|e| e.kind == "code"));
    }
}
