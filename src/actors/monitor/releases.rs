//! Monitor-owned update schedule and cache. Frontends never perform release I/O.
use crate::domain::versions::{Release, Versions};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{future::Future, path::Path, pin::Pin, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, time::Instant};
const INTERVAL: Duration = Duration::from_secs(3600);
const RETRY: Duration = Duration::from_secs(900);
const LIMIT: usize = 1024 * 1024;
const ENDPOINT: &str = "https://api.github.com/repos/melsomino/ai-agent-monitor/releases/latest";
#[derive(Clone, Default, Serialize, Deserialize)]
struct Cache {
    latest: Option<Release>,
    checked_at: u64,
}
impl Cache {
    fn state(&self) -> Versions {
        Versions::checked(self.latest.clone(), self.checked_at)
    }
}
type Check = Pin<Box<dyn Future<Output = Result<Option<Release>>> + Send>>;
pub(super) struct Checker {
    enabled: bool,
    state: Versions,
    next: Instant,
    checking: Option<Check>,
}
impl Checker {
    pub async fn new() -> Self {
        #[cfg(not(test))]
        let cache = tokio::task::spawn_blocking(|| load(&cache_path(), super::now()))
            .await
            .ok()
            .flatten();
        #[cfg(test)]
        let cache: Option<Cache> = None;
        let delay = cache
            .as_ref()
            .map(|cache| {
                INTERVAL.saturating_sub(Duration::from_secs(
                    super::now().saturating_sub(cache.checked_at),
                ))
            })
            .unwrap_or_default();
        Self {
            enabled: !cfg!(test),
            state: cache.map(|cache| cache.state()).unwrap_or_default(),
            next: Instant::now() + delay,
            checking: None,
        }
    }
    pub fn state(&self) -> Versions {
        self.state.clone()
    }
    pub async fn next(&mut self) -> Versions {
        // Actor tests never contact GitHub or read/write user cache files.
        if !self.enabled {
            return std::future::pending().await;
        }
        loop {
            if self.checking.is_none() {
                tokio::time::sleep_until(self.next).await;
                self.checking = Some(Box::pin(fetch()));
            }
            // Store the in-flight future across command-driven cancellation of next().
            let result = self.checking.as_mut().unwrap().await;
            self.checking = None;
            match result {
                Ok(latest) => {
                    let cache = Cache {
                        latest,
                        checked_at: super::now(),
                    };
                    self.state = cache.state();
                    self.next = Instant::now() + INTERVAL;
                    #[cfg(not(test))]
                    tokio::task::spawn_blocking(move || {
                        let _ = save(&cache_path(), &cache);
                    });
                    return self.state.clone();
                }
                Err(_) => {
                    self.next = Instant::now() + RETRY;
                }
            }
        }
    }
}
#[cfg(not(test))]
fn cache_path() -> std::path::PathBuf {
    crate::app::core::state_home().join("release-cache.json")
}
fn load(path: &Path, now: u64) -> Option<Cache> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(8193)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 8192 {
        return None;
    }
    let mut cache: Cache = serde_json::from_slice(&bytes).ok()?;
    if cache.checked_at > now.saturating_add(60) {
        return None;
    }
    if let Some(release) = cache.latest {
        cache.latest = Some(Release::stable(&release.version)?);
    }
    Some(cache)
}
fn save(path: &Path, cache: &Cache) -> Result<()> {
    use std::io::Write;
    let directory = path.parent().context("Release cache directory")?;
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&serde_json::to_vec(cache)?)?;
    file.persist(path)?;
    Ok(())
}
fn release(bytes: &[u8]) -> Result<Option<Release>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    anyhow::ensure!(
        value["draft"] == false && value["prerelease"] == false,
        "Expected published stable release"
    );
    let tag = value["tag_name"]
        .as_str()
        .context("Missing release version")?;
    Ok(Some(
        Release::stable(tag).context("Invalid stable release version")?,
    ))
}
async fn fetch() -> Result<Option<Release>> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut child = tokio::process::Command::new("curl")
            .args([
                "--disable",
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--max-redirs",
                "3",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--connect-timeout",
                "5",
                "--max-time",
                "10",
                "--max-filesize",
                "1048576",
                "--header",
                "Accept: application/vnd.github+json",
                "--user-agent",
                "AI-Agent-Monitor-release-check",
                ENDPOINT,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        anyhow::ensure!(bytes.len() <= LIMIT, "Release response too large");
        anyhow::ensure!(child.wait().await?.success(), "Release check unavailable");
        release(&bytes)
    })
    .await
    .context("Release check timed out")?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn in_flight_check_survives_other_monitor_commands_and_failures_keep_known_release() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = started.clone();
        let mut checker = Checker {
            enabled: true,
            state: Versions::default(),
            next: Instant::now(),
            checking: Some(Box::pin(async move {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                rx.await.unwrap()
            })),
        };
        // Cancel the wait as the monitor does whenever another command arrives.
        assert!(
            tokio::time::timeout(Duration::from_millis(5), checker.next())
                .await
                .is_err()
        );
        tx.send(Ok(Release::stable("v99.0.0"))).unwrap();
        let known = checker.next().await;
        assert_eq!(started.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(known.available.as_ref().unwrap().version, "v99.0.0");
        assert!(known.checked_at.is_some());
        assert!(checker.next > Instant::now() + Duration::from_secs(3500));
        checker.checking = Some(Box::pin(async { anyhow::bail!("offline") }));
        assert!(
            tokio::time::timeout(Duration::from_millis(5), checker.next())
                .await
                .is_err()
        );
        assert_eq!(checker.state, known);
        assert!(checker.next > Instant::now() + Duration::from_secs(800));
    }
    #[test]
    fn published_metadata_uses_our_repository_and_rejects_api_failures() {
        let release=release(br#"{"tag_name":"v0.4.0","draft":false,"prerelease":false,"html_url":"https://untrusted.example/"}"#).unwrap().unwrap();
        assert_eq!(
            release.url,
            "https://github.com/melsomino/ai-agent-monitor/releases/tag/v0.4.0"
        );
        for bytes in [
            br#"{"message":"rate limited"}"#.as_slice(),
            br#"{"tag_name":"v0.4.0","draft":true,"prerelease":false}"#,
            br#"{"tag_name":"v0.4.0-rc.1","draft":false,"prerelease":true}"#,
        ] {
            assert!(super::release(bytes).is_err());
        }
    }
    #[test]
    fn cache_survives_restart_and_upgrade_without_trusting_cached_urls_or_future_times() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("release-cache.json");
        let mut latest = Release::stable("v99.0.0").unwrap();
        latest.url = "https://untrusted.example/".into();
        save(
            &path,
            &Cache {
                latest: Some(latest),
                checked_at: 100,
            },
        )
        .unwrap();
        let loaded = load(&path, 110).unwrap();
        assert_eq!(loaded.checked_at, 100);
        assert!(
            loaded
                .state()
                .available
                .unwrap()
                .url
                .starts_with("https://github.com/melsomino/ai-agent-monitor/releases/tag/")
        );
        assert!(load(&path, 0).is_none());
        save(
            &path,
            &Cache {
                latest: Release::stable("v0.0.1"),
                checked_at: 100,
            },
        )
        .unwrap();
        assert!(load(&path, 110).unwrap().state().available.is_none());
        std::fs::write(&path, b"broken").unwrap();
        assert!(load(&path, 110).is_none());
    }
}
