//! Local suggestions for BotFather; these values never change the connected bot.
use objc2_foundation::{
    NSFullUserName, NSString, NSStringTransformStripDiacritics, NSStringTransformToLatin,
};

/// Consume a newly issued start link once, only after an explicit Connect click.
#[derive(Default)]
pub(super) struct AutoOpen {
    waiting: bool,
    previous: Option<String>,
}
impl AutoOpen {
    pub(super) fn request(&mut self, current: Option<String>) {
        self.waiting = true;
        self.previous = current;
    }
    pub(super) fn cancel(&mut self) {
        self.waiting = false;
    }
    pub(super) fn take_link(&mut self, link: Option<&str>) -> Option<String> {
        if !self.waiting || link.is_none() || link == self.previous.as_deref() {
            return None;
        }
        self.waiting = false;
        link.map(str::to_owned)
    }
}

pub(super) fn username(regenerate: bool) -> String {
    let random = regenerate.then(crate::app::ids::nonce);
    from_name(
        &NSFullUserName().to_string(),
        random.as_deref().map(|value| &value[..4]),
    )
}

fn from_name(name: &str, random: Option<&str>) -> String {
    let name = NSString::from_str(name);
    // SAFETY: Both transforms are immutable Foundation constants.
    let latin = unsafe { name.stringByApplyingTransform_reverse(NSStringTransformToLatin, false) }
        .unwrap_or(name);
    let latin =
        unsafe { latin.stringByApplyingTransform_reverse(NSStringTransformStripDiacritics, false) }
            .unwrap_or(latin)
            .to_string();
    let mut canonical = String::new();
    for character in latin.chars() {
        if character.is_ascii_alphanumeric() {
            canonical.push(character.to_ascii_lowercase());
        } else if !canonical.is_empty() && !canonical.ends_with('_') {
            canonical.push('_');
        }
    }
    let suffix = match random {
        Some(value) => format!("_{value}_bot"),
        None => "_bot".into(),
    };
    canonical.truncate(32 - "aiagmon_".len() - suffix.len());
    let base = canonical.trim_end_matches('_');
    let base = if base.is_empty() { "my" } else { base };
    format!("aiagmon_{base}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_opens_only_the_new_link_once_and_cancel_prevents_opening() {
        let mut opener = AutoOpen::default();
        opener.request(Some("old".into()));
        assert!(opener.take_link(Some("old")).is_none());
        assert!(opener.take_link(None).is_none());
        assert_eq!(opener.take_link(Some("new")), Some("new".into()));
        assert!(opener.take_link(Some("new")).is_none());
        opener.request(None);
        opener.cancel();
        assert!(opener.take_link(Some("later")).is_none());
    }

    #[test]
    fn readable_usernames_transliterate_full_names() {
        for (name, expected) in [
            ("Alex Morgan", "aiagmon_alex_morgan_bot"),
            ("Иван Петров", "aiagmon_ivan_petrov_bot"),
            ("José García", "aiagmon_jose_garcia_bot"),
            ("John Smith", "aiagmon_john_smith_bot"),
            ("张伟", "aiagmon_zhang_wei_bot"),
            ("  John --- Smith__ ", "aiagmon_john_smith_bot"),
            ("", "aiagmon_my_bot"),
            ("___", "aiagmon_my_bot"),
        ] {
            assert_eq!(from_name(name, None), expected, "{name}");
        }
    }

    #[test]
    fn retry_suffix_and_long_names_stay_within_telegram_limits() {
        assert_eq!(
            from_name("John Smith", Some("7ab4")),
            "aiagmon_john_smith_7ab4_bot"
        );
        for name in [
            "Very long full name with many words".repeat(10),
            "___".into(),
        ] {
            for suffix in [None, Some("7ab4")] {
                let value = from_name(&name, suffix);
                assert!((5..=32).contains(&value.len()));
                assert!(value.ends_with("_bot"));
                assert!(!value.starts_with('_'));
                assert!(!value.contains("__"));
                assert!(value.bytes().all(|byte| byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || byte == b'_'));
            }
        }
    }
}
