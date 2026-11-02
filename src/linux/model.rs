//! Guide-defined sorting, filtering, markup and transient disclosure state.
use crate::{
    actors::monitor::State,
    domain::session::{Activity, Client, Session},
};
use gtk::glib;
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;

pub fn eligible(state: &State, query: &str) -> Vec<Session> {
    let words = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    let mut sessions = state
        .sessions
        .iter()
        .filter(|s| {
            state.preferences.applications.enabled(s.client)
                && words
                    .iter()
                    .all(|word| s.title.to_lowercase().contains(word))
        })
        .cloned()
        .collect::<Vec<_>>();
    sessions.sort_by(|a, b| {
        rank(a.client)
            .cmp(&rank(b.client))
            .then(a.client.application_id().cmp(&b.client.application_id()))
            .then(
                glib::FilenameCollationKey::from(a.title.to_lowercase())
                    .cmp(&glib::FilenameCollationKey::from(b.title.to_lowercase())),
            )
            .then(a.key().cmp(&b.key()))
    });
    sessions
}
fn rank(client: Client) -> u8 {
    match client {
        Client::VSCode => 0,
        Client::Terminal => 1,
        Client::Desktop { .. } => 2,
        Client::Unknown => 3,
    }
}
pub fn pop_grapheme(text: &mut String) {
    if let Some((index, _)) = text.grapheme_indices(true).next_back() {
        text.truncate(index);
    }
}
pub fn empty_text(state: &State, query: &str) -> &'static str {
    if state.sessions.is_empty() {
        "No open sessions"
    } else if state
        .sessions
        .iter()
        .all(|s| !state.preferences.applications.enabled(s.client))
    {
        "No visible sessions"
    } else if !query.is_empty() {
        "No matching sessions"
    } else {
        "No visible sessions"
    }
}
#[derive(Default)]
pub struct Disclosure {
    pub open: bool,
    pub expanded: HashMap<String, bool>,
    requests: HashMap<String, String>,
}
impl Disclosure {
    pub fn show(&mut self, sessions: &[Session]) {
        self.open = true;
        self.expanded.clear();
        self.requests.clear();
        self.observe(sessions);
    }
    pub fn observe(&mut self, sessions: &[Session]) {
        self.expanded
            .retain(|key, _| sessions.iter().any(|s| s.key() == *key));
        for s in sessions {
            let key = s.key();
            let token = format!("{:?}:{:?}", s.approval, s.interaction_data);
            let new_request = s.approval.is_some() && self.requests.get(&key) != Some(&token);
            self.expanded
                .entry(key.clone())
                .or_insert(s.activity == Activity::Working || s.approval.is_some());
            if new_request {
                self.expanded.insert(key.clone(), true);
            }
            self.requests.insert(key, token);
        }
    }
    pub fn close(&mut self) {
        self.open = false;
        self.expanded.clear();
        self.requests.clear();
    }
}
pub fn markup(source: &str, dark: bool) -> String {
    crate::domain::markdown::parse(source.trim_end())
        .into_iter()
        .map(|run| {
            let mut text = glib::markup_escape_text(&run.text).to_string();
            if run.style.code {
                text = format!(
                    "<span font_family=\"monospace\" font_desc=\"12px\" background=\"{}\">{text}</span>",
                    if dark { "#3b3b3b" } else { "#e5e5e5" }
                );
            }
            if run.style.bold {
                text = format!("<b>{text}</b>");
            }
            if run.style.italic {
                text = format!("<i>{text}</i>");
            }
            if run.style.strike {
                text = format!("<s>{text}</s>");
            }
            if run.style.heading > 0 {
                let size = match run.style.heading {
                    1 => 16,
                    2 => 15,
                    _ => 14,
                };
                text = format!("<span font_desc=\"{size}px\">{text}</span>");
            }
            if let Some(link) = run.style.link {
                text = format!("<a href=\"{}\">{text}</a>", glib::markup_escape_text(&link));
            }
            text
        })
        .collect()
}
pub fn title_markup(title: &str, query: &str) -> String {
    let words = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    let folded = title.to_lowercase();
    // Byte offsets in the original title are retained through Unicode case expansion.
    let mut offsets = Vec::new();
    for (start, ch) in title.char_indices() {
        for _ in ch
            .to_lowercase()
            .flat_map(|c| std::iter::repeat_n((), c.len_utf8()))
        {
            offsets.push((start, start + ch.len_utf8()));
        }
    }
    let mut ranges = Vec::new();
    for word in words {
        for (start, _) in folded.match_indices(&word) {
            if !word.is_empty() {
                ranges.push((offsets[start].0, offsets[start + word.len() - 1].1));
            }
        }
    }
    let mut out = String::new();
    for (start, ch) in title.char_indices() {
        let escaped = glib::markup_escape_text(&ch.to_string()).to_string();
        if ranges.iter().any(|(a, b)| start >= *a && start < *b) {
            out.push_str(&format!("<u>{escaped}</u>"));
        } else {
            out.push_str(&escaped);
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_filter_backspace_keeps_composed_characters() {
        let mut s = "a👩‍💻".to_string();
        pop_grapheme(&mut s);
        assert_eq!(s, "a");
    }
    #[test]
    fn heading_and_links_render_natively_and_escape_markup() {
        let m = markup(
            "# Ready\n\n[Docs](https://example.com) and `<unsafe>`",
            false,
        );
        assert!(m.contains("font_desc=\"16px\""));
        assert!(gtk::pango::parse_markup(&markup("# Ready\n\n`<unsafe>`", false), '\0').is_ok());
        assert!(m.contains("href=\"https://example.com\""));
        assert!(m.contains("&lt;unsafe&gt;"));
        assert!(!markup("[run](command:unsafe)", false).contains("href"));
    }
    #[test]
    fn highlight_handles_unicode_expansion() {
        assert!(title_markup("İstanbul Доставка", "i дост").contains("<u>Д</u>"));
    }
}

#[cfg(test)]
mod behavior_tests {
    use super::*;
    use crate::domain::{interaction::Interaction, session::Approval};
    fn session(id: &str, title: &str, activity: Activity) -> Session {
        Session {
            agent: "codex",
            id: id.into(),
            title: title.into(),
            cwd: "/tmp".into(),
            workspace: None,
            git_branch: None,
            updated_at: 0,
            client: Client::VSCode,
            activity,
            last_message: String::new(),
            interaction_data: Interaction::default(),
            target: None,
            authoritative: true,
            approval: None,
        }
    }
    #[test]
    fn disclosure_respects_manual_collapse_until_a_new_request_and_resets_on_close() {
        let mut s = session("one", "Task", Activity::Working);
        let mut d = Disclosure::default();
        d.show(&[s.clone()]);
        assert_eq!(d.expanded[&s.key()], true);
        d.expanded.insert(s.key(), false);
        d.observe(&[s.clone()]);
        assert_eq!(d.expanded[&s.key()], false);
        s.approval = Some(Approval {
            id: "r".into(),
            body: "Allow?".into(),
            expires: u64::MAX,
            can_allow: true,
        });
        d.observe(&[s.clone()]);
        assert_eq!(d.expanded[&s.key()], true);
        d.expanded.insert(s.key(), false);
        d.observe(&[s.clone()]);
        assert_eq!(d.expanded[&s.key()], false);
        d.close();
        assert!(d.expanded.is_empty());
        d.show(&[s.clone()]);
        assert_eq!(d.expanded[&s.key()], true);
    }
    #[test]
    fn eligibility_uses_natural_titles_and_distinct_empty_states() {
        let mut state = State::default();
        assert_eq!(empty_text(&state, ""), "No open sessions");
        state.sessions = vec![
            session("ten", "Task 10", Activity::Idle),
            session("two", "Task 2", Activity::Idle),
        ];
        state.preferences.applications.observe([Client::VSCode]);
        assert_eq!(eligible(&state, "")[0].id, "two");
        assert_eq!(eligible(&state, "TASK 2").len(), 1);
        assert!(eligible(&state, "task absent").is_empty());
        assert_eq!(empty_text(&state, "absent"), "No matching sessions");
        state
            .preferences
            .applications
            .set_enabled(&Client::VSCode.application_id(), false);
        assert_eq!(empty_text(&state, ""), "No visible sessions");
        assert!(eligible(&state, "").is_empty());
    }
}
