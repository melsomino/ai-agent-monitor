//! Explicit read-only account request; never start/resume a turn or redeem a reset.
use crate::domain::usage::{ResetCredit, ResetPhase, Resets};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn executable() -> Result<PathBuf> {
    let home = crate::app::core::home();
    let mut paths = std::env::var_os("PATH")
        .map(|value| {
            std::env::split_paths(&value)
                .map(|p| p.join("codex"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    paths.extend([
        home.join(".local/bin/codex"),
        home.join(".cargo/bin/codex"),
        crate::app::core::codex_home().join("packages/standalone/current/bin/codex"),
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
    ]);
    paths
        .into_iter()
        .find(|path| {
            path.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .context("Codex CLI is unavailable. Install or update Codex to read usage limit resets.")
}
pub async fn read() -> Result<Resets> {
    read_from(&executable()?).await
}
async fn read_from(path: &Path) -> Result<Resets> {
    let mut child = tokio::process::Command::new(path)
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("Cannot start Codex's read-only account connection")?;
    let result=tokio::time::timeout(Duration::from_secs(25),async {
        let mut input=child.stdin.take().context("Codex input unavailable")?;
        let mut output=BufReader::new(child.stdout.take().context("Codex output unavailable")?);
        input.write_all(format!("{}\n",json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"ai_agent_monitor","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}})).as_bytes()).await?;
        reply(&mut output,1).await?;
        input.write_all(format!("{}\n{}\n",json!({"method":"initialized"}),json!({"id":2,"method":"account/rateLimits/read","params":{"supportsLunaReserve":false,"excludeResetCreditDetails":false}})).as_bytes()).await?;
        parse(&reply(&mut output,2).await?)
    }).await.context("Codex usage limit resets request timed out. Try again.")?;
    // Closing this private stdio connection never touches an existing agent server.
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}
async fn reply(reader: &mut BufReader<tokio::process::ChildStdout>, id: u64) -> Result<Value> {
    loop {
        let mut bytes = Vec::new();
        // Bound each response including notifications without logging account payloads.
        use tokio::io::AsyncReadExt;
        let count = reader
            .take(2 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut bytes)
            .await?;
        ensure!(
            count > 0 && count <= 2 * 1024 * 1024,
            "Invalid Codex account response"
        );
        let value: Value =
            serde_json::from_slice(&bytes).context("Invalid Codex account response")?;
        if value["id"] == id {
            ensure!(
                value.get("error").is_none(),
                "Codex could not read usage limit resets. Check that Codex is signed in and up to date."
            );
            return Ok(value["result"].clone());
        }
    }
}
fn parse(value: &Value) -> Result<Resets> {
    let summary = &value["rateLimitResetCredits"];
    if summary.is_null() {
        bail!("Codex did not provide usage limit resets for this account.");
    }
    let count = summary["availableCount"]
        .as_u64()
        .context("Invalid reset count from Codex")?;
    let credits = if summary["credits"].is_null() {
        None
    } else {
        let rows = summary["credits"]
            .as_array()
            .context("Invalid reset details from Codex")?;
        ensure!(rows.len() <= 128, "Too many reset details from Codex");
        Some(
            rows.iter()
                .filter(|row| row["status"] == "available")
                .map(|row| {
                    Ok(ResetCredit {
                        title: row["title"]
                            .as_str()
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("Usage reset")
                            .chars()
                            .take(160)
                            .collect(),
                        expires_at: if row["expiresAt"].is_null() {
                            None
                        } else {
                            Some(
                                row["expiresAt"]
                                    .as_i64()
                                    .context("Invalid reset expiration from Codex")?,
                            )
                        },
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        )
    };
    Ok(Resets {
        generation: 0,
        phase: ResetPhase::Ready,
        available_count: Some(count),
        credits,
        error: None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_authoritative_count_and_optional_details_without_inventing_resets() {
        assert!(parse(&json!({})).is_err());
        let value =
            parse(&json!({"rateLimitResetCredits":{"availableCount":3,"credits":null}})).unwrap();
        assert_eq!(value.available_count, Some(3));
        assert!(value.credits.is_none());
        let value=parse(&json!({"rateLimitResetCredits":{"availableCount":3,"credits":[{"status":"available","title":"Full reset","expiresAt":100},{"status":"redeemed","title":"Old","expiresAt":null}]}})).unwrap();
        assert_eq!(value.available_count, Some(3));
        assert_eq!(
            value.credits.unwrap(),
            vec![ResetCredit {
                title: "Full reset".into(),
                expires_at: Some(100)
            }]
        );
        assert_eq!(
            parse(&json!({"rateLimitResetCredits":{"availableCount":0,"credits":[]}}))
                .unwrap()
                .credits,
            Some(vec![])
        );
    }
    #[tokio::test]
    async fn stdio_reader_only_initializes_and_reads_account_limits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codex");
        std::fs::write(&path,r#"#!/bin/sh
read -r initialize
case "$initialize" in *'"method":"initialize"'*) ;; *) exit 1;; esac
printf '%s\n' '{"id":1,"result":{}}'
read -r initialized
read -r request
case "$request" in *'"method":"account/rateLimits/read"'*) ;; *) exit 2;; esac
printf '%s\n' '{"id":2,"result":{"rateLimitResetCredits":{"availableCount":1,"credits":[{"status":"available","title":"Full reset","expiresAt":2000}]}}}'
"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = read_from(&path).await.unwrap();
        assert_eq!(result.phase, ResetPhase::Ready);
        assert_eq!(result.available_count, Some(1));
    }
}
