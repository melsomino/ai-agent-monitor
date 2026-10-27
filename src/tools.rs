use crate::core;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub fn bundle() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let contents = root.join("build").join(core::APP_NAME).join("Contents");
    let executable = contents.join("MacOS/CodexNotify");
    fs::create_dir_all(executable.parent().unwrap())?;
    fs::create_dir_all(contents.join("Resources"))?;
    let source = env::current_exe()?;
    if source == executable {
        bail!("Run bundle from target/release/CodexNotify, not from the app being replaced");
    }
    if executable.exists() {
        let backup = root.join("build/CodexNotify.previous");
        // Keep the original sender as a rollback artifact, outside the app bundle.
        if !backup.exists() {
            fs::copy(&executable, backup)?;
        }
    }
    let temporary = tempfile::NamedTempFile::new_in(executable.parent().unwrap())?;
    fs::copy(source, temporary.path())?;
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o755))?;
    temporary.persist(&executable)?;
    fs::write(
        contents.join("Info.plist"),
        include_bytes!("../assets/Info.plist"),
    )?;
    fs::write(
        contents.join("Resources/AppIcon.icns"),
        include_bytes!("../assets/AppIcon.icns"),
    )?;
    let result = Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(contents.parent().unwrap())
        .output()?;
    if !result.status.success() {
        bail!("codesign: {}", String::from_utf8_lossy(&result.stderr));
    }
    println!("Built: {}", contents.parent().unwrap().display());
    Ok(())
}

pub fn check_hooks(binary: &str) -> Result<()> {
    let mut child = Command::new(binary)
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().context("missing Codex stdout")?;
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let result = (|| -> Result<()> {
        let mut stdin = child.stdin.take().context("missing Codex stdin")?;
        let mut send = |message: Value| -> Result<()> {
            writeln!(stdin, "{message}")?;
            stdin.flush()?;
            Ok(())
        };
        let response = |id: u64| -> Result<Value> {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                let line =
                    receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))??;
                let message: Value = serde_json::from_str(&line)?;
                if message["id"] == id {
                    if let Some(error) = message.get("error") {
                        bail!("{error}");
                    }
                    return Ok(message["result"].clone());
                }
            }
        };
        send(
            json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"codex_notify_check","version":env!("CARGO_PKG_VERSION")}}}),
        )?;
        response(1)?;
        send(json!({"method":"initialized","params":{}}))?;
        send(json!({"id":2,"method":"hooks/list","params":{"cwds":[env::current_dir()?]}}))?;
        let result = response(2)?;
        for entry in result["data"].as_array().context("missing hooks list")? {
            let hooks: Vec<_> = entry["hooks"]
                .as_array()
                .context("missing hooks")?
                .iter()
                .filter(|hook| {
                    hook["command"].as_str().is_some_and(|command| {
                        command.contains(&core::bundle_executable().to_string_lossy().to_string())
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"binary":binary,"hooks":hooks,"errors":entry["errors"],"warnings":entry["warnings"]})
                )?
            );
        }
        Ok(())
    })();
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    result
}
