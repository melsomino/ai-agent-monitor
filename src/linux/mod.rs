//! Native Rust/GTK desktop. GTK owns the main thread; Tokio owns agent state.
pub(crate) mod credentials;
mod model;
mod notifications;
mod panel;
mod requests;
mod settings;
mod style;
mod ui;
mod widgets;

use crate::{
    actors::{agents, human::telegram, monitor},
    app::{cli::NativeCommand, core, ipc},
};
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    fs,
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    },
    process::{Command, Stdio},
    sync::Arc,
};
use tokio::sync::{mpsc, watch};

pub(super) fn open_session(target: &str, timestamp: u32) -> Result<()> {
    use gtk::prelude::*;
    anyhow::ensure!(
        target.starts_with("vscode://")
            || target.starts_with("file://")
            || target.starts_with("https://")
            || std::path::Path::new(target).is_absolute(),
        "Unsupported project target"
    );
    let display = gtk::gdk::Display::default().context("No desktop display")?;
    let context = display
        .app_launch_context()
        .context("No desktop launch context")?;
    context.set_timestamp(timestamp);
    let uri = if std::path::Path::new(target).is_absolute() {
        gtk::gio::File::for_path(target).uri().to_string()
    } else {
        target.to_owned()
    };
    // GDK supplies the X11 startup ID / Wayland activation token. A bare
    // xdg-open process loses the user interaction and triggers focus prevention.
    gtk::gio::AppInfo::launch_default_for_uri(&uri, Some(&context))
        .context("Switch to the session application")
}

pub(super) fn open(target: &str) -> Result<()> {
    anyhow::ensure!(
        target.starts_with("vscode://")
            || target.starts_with("file://")
            || target.starts_with("https://")
            || std::path::Path::new(target).is_absolute(),
        "Unsupported project target"
    );
    Command::new("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

pub fn run(command: NativeCommand) -> Result<()> {
    if let NativeCommand::OpenTarget(target) = command {
        return open(&target);
    }
    let directory = core::state_home();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    let metadata = fs::symlink_metadata(&directory)?;
    anyhow::ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "Monitor state directory must be private and owned by this user"
    );
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join("resident.lock"))?;
    fs::set_permissions(
        directory.join("resident.lock"),
        fs::Permissions::from_mode(0o600),
    )?;
    // SAFETY: The owned descriptor and exclusive lock live until the GUI stops.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        if !matches!(command, NativeCommand::Background) {
            ipc::request(&ipc::Request::ShowWindow).context("Contact the running monitor")?;
        }
        return Ok(());
    }
    gtk::init()
        .map_err(|e| anyhow::anyhow!("Open the monitor in an Ubuntu graphical session: {e}"))?;
    if let Ok(icon) = gtk::gdk_pixbuf::Pixbuf::from_read(std::io::Cursor::new(
        include_bytes!("../../assets/AppIcon-robots.png").as_slice(),
    )) {
        gtk::Window::set_default_icon(&icon);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (tx, rx) = mpsc::channel(256);
    let (published, state) = watch::channel(Arc::new(monitor::State::default()));
    let handle = monitor::Handle { tx, state };
    let (platform_tx, platform_rx) = mpsc::channel(128);
    let (telegram_tx, telegram_rx) = mpsc::channel(128);
    let mut ports = HashMap::new();
    for provider in crate::agents::sessions::adapters() {
        let (tx, rx) = mpsc::channel(64);
        ports.insert(provider.id(), tx);
        runtime.spawn(agents::supervise(provider, rx, handle.clone()));
    }
    let task = runtime.spawn(monitor::run(
        rx,
        published,
        ports,
        telegram_tx.clone(),
        platform_tx,
    ));
    runtime.spawn(telegram::run(telegram_rx, handle.clone(), telegram_tx));
    let _ = handle.send(monitor::Command::PreferencesObserved(
        settings::load_preferences(),
    ));
    let ipc_task = ipc::start(handle.clone(), runtime.handle())?;
    let desktop = ui::Desktop::new(handle.clone(), platform_rx)?;
    if !matches!(command, NativeCommand::Background) {
        desktop.show();
    }
    gtk::main();
    desktop.close();
    let _ = handle.send(monitor::Command::Shutdown);
    runtime.block_on(async {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), task).await;
        ipc_task.abort();
    });
    runtime.shutdown_timeout(std::time::Duration::from_secs(3));
    Ok(())
}
