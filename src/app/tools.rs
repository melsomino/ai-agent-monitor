use crate::app::core;
use anyhow::{Context, Result, bail};
use std::{env, fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

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
    anyhow::ensure!(
        cfg!(target_os = "macos"),
        "App bundles and DMGs require macOS; on Ubuntu run the binary directly"
    );
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
