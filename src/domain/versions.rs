//! Release metadata, independent of transports and frontend presentation.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub url: String,
}
impl Release {
    pub fn stable(tag: &str) -> Option<Self> {
        version(tag, true)?;
        Some(Self {
            version: tag.into(),
            url: format!("https://github.com/melsomino/ai-agent-monitor/releases/tag/{tag}"),
        })
    }
    pub fn newer_than(&self, current: &str) -> bool {
        match (version(&self.version, true), version(current, false)) {
            (Some(latest), Some(current)) => latest > current,
            _ => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versions {
    pub current: String,
    pub available: Option<Release>,
    pub checked_at: Option<u64>,
}
impl Default for Versions {
    fn default() -> Self {
        Self {
            current: env!("AI_AGENT_MONITOR_VERSION").into(),
            available: None,
            checked_at: None,
        }
    }
}
impl Versions {
    pub fn checked(latest: Option<Release>, checked_at: u64) -> Self {
        let mut state = Self::default();
        state.available = latest.filter(|release| release.newer_than(&state.current));
        state.checked_at = Some(checked_at);
        state
    }
}
fn version(text: &str, strict: bool) -> Option<[u64; 3]> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let base = if strict {
        text
    } else {
        text.split(['+', '-', ' ', '·']).next()?
    };
    let numbers: Vec<_> = base.split('.').collect();
    if numbers.len() != 3 {
        return None;
    }
    let mut result = [0; 3];
    for (index, number) in numbers.iter().enumerate() {
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        result[index] = number.parse().ok()?;
    }
    Some(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_versions_ignore_installed_builds_but_reject_unpublished_tags() {
        let release = Release::stable("v0.3.10").unwrap();
        assert!(release.newer_than("0.3.9+1322"));
        assert!(!release.newer_than("0.3.10+1"));
        assert!(!release.newer_than("1.0.0"));
        assert!(!release.newer_than("preview"));
        for tag in [
            "v0.4.0-rc.1",
            "v0.4.0+1",
            "x0.4.0",
            "0.4",
            "../other",
            "0.4.0/else",
        ] {
            assert!(Release::stable(tag).is_none());
        }
    }
}
