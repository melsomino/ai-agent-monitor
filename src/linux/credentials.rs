//! Ubuntu Secret Service credentials. Tokens are passed through stdin, never argv.
use anyhow::{Context, Result};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn command() -> Command {
    Command::new("secret-tool")
}
pub fn load() -> Result<Option<String>> {
    load_named("telegram-bot-token")
}
pub fn load_named(name: &str) -> Result<Option<String>> {
    let result = command()
        .args([
            "lookup",
            "application",
            "ai-agent-monitor",
            "credential",
            name,
        ])
        .output()
        .context("Install libsecret-tools to store credentials in Ubuntu's keyring")?;
    if result.status.code() == Some(1) && result.stdout.is_empty() && result.stderr.is_empty() {
        return Ok(None);
    }
    anyhow::ensure!(
        result.status.success(),
        "Could not read credential from Ubuntu's keyring"
    );
    let token = String::from_utf8(result.stdout)?.trim_end().to_owned();
    Ok((!token.is_empty()).then_some(token))
}
pub fn save(token: &str) -> Result<()> {
    save_named("telegram-bot-token", token)
}
pub fn save_named(name: &str, token: &str) -> Result<()> {
    let mut child = command()
        .args([
            "store",
            "--label=AI Agent Monitor credential",
            "application",
            "ai-agent-monitor",
            "credential",
            name,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Install libsecret-tools to store credentials in Ubuntu's keyring")?;
    child
        .stdin
        .take()
        .context("Open keyring input")?
        .write_all(token.as_bytes())?;
    anyhow::ensure!(
        child.wait()?.success(),
        "Could not save credential; unlock Ubuntu's keyring and try again"
    );
    Ok(())
}
pub fn delete() -> Result<()> {
    delete_named("telegram-bot-token")
}
pub fn delete_named(name: &str) -> Result<()> {
    let status = command()
        .args([
            "clear",
            "application",
            "ai-agent-monitor",
            "credential",
            name,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    anyhow::ensure!(
        status.success() || status.code() == Some(1),
        "Could not remove the credential"
    );
    Ok(())
}
