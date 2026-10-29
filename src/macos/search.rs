//! Case-insensitive AND matching. Highlight ranges use AppKit's UTF-16 offsets.
use std::ops::Range;

pub fn matches(title: &str, query: &str) -> bool {
    let title = title.to_lowercase();
    query
        .split_whitespace()
        .all(|word| title.contains(&word.to_lowercase()))
}

pub fn highlights(title: &str, query: &str) -> Vec<Range<usize>> {
    let mut folded = String::new();
    let mut offsets = Vec::new();
    let mut position = 0;
    for ch in title.chars() {
        let end = position + ch.len_utf16();
        for lower in ch.to_lowercase() {
            folded.push(lower);
            offsets.extend(std::iter::repeat_n(position..end, lower.len_utf8()));
        }
        position = end;
    }
    let mut ranges = Vec::new();
    for word in query.split_whitespace().map(str::to_lowercase) {
        for (start, _) in folded.char_indices() {
            if folded[start..].starts_with(&word) {
                ranges.push(offsets[start].start..offsets[start + word.len() - 1].end);
            }
        }
    }
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

pub fn printable(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| !ch.is_control() && !('\u{f700}'..='\u{f8ff}').contains(&ch))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_word_must_match_regardless_of_case_order_or_repeated_spaces() {
        assert!(matches("Message DELIVERY", "  delivery  Mess "));
        assert!(!matches("message", "message delivery"));
        assert!(matches("Исправить доставку", "ДОСТ испр"));
        assert!(matches("Anything", "   "));
    }
    #[test]
    fn highlights_all_matches_and_merges_overlapping_words() {
        assert_eq!(
            highlights("Message message delivery", "mess sage"),
            vec![0..7, 8..15]
        );
        assert_eq!(highlights("banana", "ana"), vec![1..6]);
        assert_eq!(
            highlights("Message delivery", ""),
            Vec::<Range<usize>>::new()
        );
        assert_eq!(
            highlights("Message delivery", "missing"),
            Vec::<Range<usize>>::new()
        );
    }
    #[test]
    fn highlight_offsets_survive_emoji_cyrillic_and_case_expansion() {
        assert_eq!(highlights("🤖 Доставка", "ДОСТ"), vec![3..7]);
        assert_eq!(highlights("İstanbul", "i"), vec![0..1]);
        assert!(printable(" delivery Привет"));
        assert!(!printable("\u{f700}"));
        assert!(!printable("\r"));
        assert!(!printable(""));
    }
}
