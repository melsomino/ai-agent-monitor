//! Last reported account quotas from local rollouts. Never read credentials or run a model.
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub use crate::domain::usage::{Usage, Window};
#[derive(Default)]
pub struct Reader {
    cache: Option<(PathBuf, Instant, Option<Usage>)>,
}
impl Reader {
    pub fn latest(&mut self, home: &Path) -> Option<Usage> {
        if let Some((root, at, value)) = &self.cache
            && root == home
            && at.elapsed() < Duration::from_secs(10)
        {
            return value.clone();
        }
        let value = read_latest(home);
        self.cache = Some((home.to_owned(), Instant::now(), value.clone()));
        value
    }
}
fn read_latest(home: &Path) -> Option<Usage> {
    let mut databases: Vec<_> = fs::read_dir(home)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let version = path
                .file_name()?
                .to_str()?
                .strip_prefix("state_")?
                .strip_suffix(".sqlite")?
                .parse::<u32>()
                .ok()?;
            Some((version, path))
        })
        .collect();
    databases.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
    for (_, path) in databases {
        let Some(paths) = recent_rollouts(&path) else {
            continue;
        };
        let mut latest: Option<Usage> = None;
        for path in paths {
            let Ok(mut file) = fs::File::open(path) else {
                continue;
            };
            let Ok(meta) = file.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let start = meta.len().saturating_sub(512 * 1024);
            if file.seek(SeekFrom::Start(start)).is_err() {
                continue;
            }
            let mut tail = Vec::new();
            if file.take(512 * 1024).read_to_end(&mut tail).is_err() {
                continue;
            }
            if let Some(usage) = parse_tail(&tail, start > 0)
                && latest
                    .as_ref()
                    .is_none_or(|old| usage.observed_at > old.observed_at)
            {
                latest = Some(usage);
            }
        }
        if latest.is_some() {
            return latest;
        }
    }
    None
}
fn recent_rollouts(path: &Path) -> Option<Vec<PathBuf>> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    db.busy_timeout(Duration::from_millis(150)).ok()?;
    let mut statement = db
        .prepare("SELECT rollout_path FROM threads ORDER BY updated_at DESC LIMIT 16")
        .ok()?;
    statement
        .query_map([], |row| Ok(PathBuf::from(row.get::<_, String>(0)?)))
        .ok()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .ok()
}
fn parse_tail(bytes: &[u8], partial_start: bool) -> Option<Usage> {
    let mut lines: Vec<_> = bytes.split(|b| *b == b'\n').collect();
    if partial_start {
        lines.remove(0);
    }
    for line in lines.into_iter().rev() {
        let Ok(event) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if event["type"] != "event_msg" || event["payload"]["type"] != "token_count" {
            continue;
        }
        let limits = &event["payload"]["rate_limits"];
        // Other metered models have independent quotas, not the Codex account balance.
        if limits["limit_id"].as_str().is_some_and(|id| id != "codex") {
            continue;
        }
        let Some(observed_at) = event["timestamp"].as_str() else {
            continue;
        };
        let mut windows = Vec::new();
        for name in ["primary", "secondary"] {
            let window = &limits[name];
            let (Some(minutes), Some(used_percent)) = (
                window["window_minutes"].as_u64(),
                window["used_percent"].as_f64(),
            ) else {
                continue;
            };
            if minutes == 0 || !used_percent.is_finite() || used_percent < 0.0 {
                continue;
            }
            windows.push(Window {
                minutes,
                used_percent,
                resets_at: window["resets_at"].as_i64(),
            });
        }
        windows.sort_by_key(|window| window.minutes);
        windows.dedup_by_key(|window| window.minutes);
        if !windows.is_empty() {
            return Some(Usage {
                observed_at: observed_at.into(),
                windows,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn event(timestamp: &str, used: f64, id: &str) -> String {
        json!({"timestamp":timestamp,"type":"event_msg","payload":{"type":"token_count","rate_limits":{
            "limit_id":id,"primary":{"used_percent":used,"window_minutes":10080,"resets_at":2000},"secondary":null
        }}}).to_string()
    }
    #[test]
    fn uses_the_latest_codex_quota_and_displays_remaining_not_spent() {
        let tail = format!(
            "{}\n{}\n{}\n{{partial",
            event("2026-10-28T10:00:00Z", 18.0, "codex"),
            event("2026-10-29T10:00:00Z", 19.0, "codex"),
            event("2026-10-29T10:01:00Z", 90.0, "other")
        );
        let usage = parse_tail(tail.as_bytes(), false).unwrap();
        assert_eq!(usage.windows[0].label(), "Weekly");
        assert_eq!(usage.windows[0].remaining(1999), Some(81));
        assert_eq!(usage.windows[0].remaining(2000), None);
    }
    #[test]
    fn reads_recent_closed_sessions_and_orders_by_observation_not_database_order() {
        let home = tempfile::tempdir().unwrap();
        let db = Connection::open(home.path().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads (rollout_path TEXT, updated_at INTEGER)")
            .unwrap();
        for (i, timestamp, used) in [
            (1, "2026-10-29T10:00:00Z", 19.0),
            (2, "2026-10-28T10:00:00Z", 10.0),
        ] {
            let path = home.path().join(format!("{i}.jsonl"));
            fs::write(&path, event(timestamp, used, "codex")).unwrap();
            db.execute(
                "INSERT INTO threads VALUES (?1, ?2)",
                rusqlite::params![path.to_str(), i],
            )
            .unwrap();
        }
        let usage = read_latest(home.path()).unwrap();
        assert_eq!(usage.windows[0].remaining(1), Some(81));
    }
    #[test]
    fn skips_partial_lines_and_invalid_windows_without_inventing_a_balance() {
        assert!(parse_tail(b"{broken\n{}\n", true).is_none());
        assert!(read_latest(tempfile::tempdir().unwrap().path()).is_none());
        let tail = format!("junk\n{}\n", event("2026-10-29T10:00:00Z", -1.0, "codex"));
        assert!(parse_tail(tail.as_bytes(), true).is_none());
        let window = Window {
            minutes: 300,
            used_percent: 102.0,
            resets_at: None,
        };
        assert_eq!(window.remaining(1), Some(0));
        assert_eq!(window.label(), "5-hour");
    }
}
