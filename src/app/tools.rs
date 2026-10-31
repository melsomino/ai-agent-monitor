use crate::app::core;
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

const DEFAULT_SIGNING_IDENTITY: &str = "-";

fn sign_app(bundle: &Path) -> Result<()> {
    let identity = env::var("AI_AGENT_MONITOR_SIGN_IDENTITY")
        .unwrap_or_else(|_| DEFAULT_SIGNING_IDENTITY.into());
    anyhow::ensure!(
        !identity.trim().is_empty(),
        "AI_AGENT_MONITOR_SIGN_IDENTITY must not be empty"
    );
    let mut command = Command::new("/usr/bin/codesign");
    command.args(["--force", "--sign", &identity]);
    if identity != "-" {
        command.args(["--options", "runtime", "--timestamp"]);
    }
    let result = command.arg(bundle).output()?;
    if !result.status.success() {
        bail!("codesign: {}", String::from_utf8_lossy(&result.stderr));
    }
    let verified = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict"])
        .arg(bundle)
        .output()?;
    anyhow::ensure!(
        verified.status.success(),
        "codesign verification: {}",
        String::from_utf8_lossy(&verified.stderr)
    );
    Ok(())
}

pub fn bundle() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let contents = root.join("build").join(core::APP_NAME).join("Contents");
    let executable = contents.join("MacOS/AIAgentMonitor");
    fs::create_dir_all(executable.parent().unwrap())?;
    fs::create_dir_all(contents.join("Resources"))?;
    let source = env::current_exe()?;
    if source == executable {
        bail!("Run bundle from target/release/AIAgentMonitor, not from the app being replaced");
    }
    if executable.exists() {
        let backup = root.join("build/AIAgentMonitor.previous");
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
        include_str!("../../assets/Info.plist")
            .replace("@BUILD_NUMBER@", env!("AI_AGENT_MONITOR_BUILD"))
            .replace("@VERSION@", env!("CARGO_PKG_VERSION")),
    )?;
    fs::write(
        contents.join("Resources/AppIcon-robots.icns"),
        include_bytes!("../../assets/AppIcon.icns"),
    )?;
    // Remove the legacy resource name so Launch Services cannot reuse its old icon.
    match fs::remove_file(contents.join("Resources/AppIcon.icns")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    sign_app(contents.parent().unwrap())?;
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
            json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"ai_agent_monitor_check","version":env!("CARGO_PKG_VERSION")}}}),
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

/// Package the current architecture. This intentionally uses the running binary,
/// so a release asset's version cannot drift from the bundled executable.
pub fn dmg() -> Result<()> {
    bundle()?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = root.join("build/release");
    fs::create_dir_all(&output)?;
    let stage = tempfile::tempdir()?;
    let result = Command::new("/usr/bin/ditto")
        .arg(root.join("build").join(core::APP_NAME))
        .arg(stage.path().join(core::APP_NAME))
        .output()?;
    if !result.status.success() {
        bail!("ditto: {}", String::from_utf8_lossy(&result.stderr));
    }
    std::os::unix::fs::symlink("/Applications", stage.path().join("Applications"))?;
    let architecture = match env::consts::ARCH {
        "aarch64" => "apple-silicon",
        "x86_64" => "intel",
        other => bail!("Unsupported macOS architecture: {other}"),
    };
    let image = output.join(format!("AI-Agent-Monitor-{architecture}.dmg"));
    let result = Command::new("/usr/bin/hdiutil")
        .args(["create", "-volname", "AI Agent Monitor", "-srcfolder"])
        .arg(stage.path())
        .args(["-fs", "HFS+", "-format", "UDZO", "-ov"])
        .arg(&image)
        .output()?;
    if !result.status.success() {
        bail!("hdiutil: {}", String::from_utf8_lossy(&result.stderr));
    }
    let checksum = Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .arg(image.file_name().context("DMG filename")?)
        .current_dir(&output)
        .output()?;
    if !checksum.status.success() {
        bail!("Could not compute the DMG checksum");
    }
    fs::write(output.join("SHA256SUMS"), &checksum.stdout)?;
    println!("Version: {}", core::VERSION);
    println!("Disk image: {}", image.display());
    println!(
        "Checksum: {}",
        String::from_utf8_lossy(&checksum.stdout).trim()
    );
    Ok(())
}
