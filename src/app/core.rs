use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    env, fs,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::Duration,
};
#[cfg(test)]
use std::{io, os::unix::fs::PermissionsExt, thread, time::Instant};
use url::Url;

pub const WAIT: Duration = Duration::from_secs(60);
pub const APP_NAME: &str = "AI Agent Monitor.app";
pub const VERSION: &str = env!("AI_AGENT_MONITOR_VERSION");

pub fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME is not set")
}

pub fn codex_home() -> PathBuf {
    env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".codex"))
}

pub fn bundle_executable() -> PathBuf {
    let exe = env::current_exe().expect("executable path is unavailable");
    if exe.parent().is_some_and(|p| p.ends_with("Contents/MacOS")) {
        exe
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("build")
            .join(APP_NAME)
            .join("Contents/MacOS/AIAgentMonitor")
    }
}

pub fn shorten(value: &str, width: usize) -> String {
    let words: Vec<_> = value.split_whitespace().collect();
    let normalized = words.join(" ");
    if normalized.chars().count() <= width {
        return normalized;
    }
    let mut result = String::new();
    for word in words {
        let extra = usize::from(!result.is_empty());
        if result.chars().count() + extra + word.chars().count() + 1 > width {
            break;
        }
        if extra != 0 {
            result.push(' ');
        }
        result.push_str(word);
    }
    result.push('…');
    result
}

pub fn chat_title(event: &Value, home: &Path) -> String {
    let Some(id) = event["session_id"].as_str().filter(|s| !s.is_empty()) else {
        return "Codex".into();
    };
    let mut databases: Vec<_> = fs::read_dir(home)
        .into_iter()
        .flatten()
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
    for (_, database) in databases {
        let read = || -> rusqlite::Result<Option<String>> {
            let db = Connection::open_with_flags(
                database,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            db.busy_timeout(Duration::from_millis(150))?;
            let mut statement = db.prepare("PRAGMA table_info(threads)")?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let fields: Vec<_> = ["name", "title"]
                .into_iter()
                .filter(|field| columns.iter().any(|c| c == field))
                .collect();
            if fields.is_empty() || !columns.iter().any(|c| c == "id") {
                return Ok(None);
            }
            let sql = format!("SELECT {} FROM threads WHERE id = ?1", fields.join(", "));
            db.query_row(&sql, [id], |row| {
                Ok((0..fields.len())
                    .filter_map(|index| row.get::<_, Option<String>>(index).ok().flatten())
                    .find(|name| !name.trim().is_empty()))
            })
            .optional()
            .map(Option::flatten)
        };
        if let Ok(Some(title)) = read() {
            return shorten(&title, 100);
        }
    }
    "Codex".into()
}

fn local_path(value: &Value) -> Option<PathBuf> {
    Url::parse(value.as_str()?)
        .ok()?
        .to_file_path()
        .ok()
        .map(normalize_path)
}

fn normalize_path(path: PathBuf) -> PathBuf {
    fs::canonicalize(&path).unwrap_or(path)
}

pub fn vscode_target(event: &Value, storage: &Path) -> String {
    if let Some(id) = event["session_id"].as_str() {
        let ids = HashSet::from([id.to_owned()]);
        if let Some(workspace) = crate::agents::codex::vscode::workspaces(&ids, storage).remove(id)
        {
            return vscode_file_target(&workspace);
        }
    }
    let Some(cwd) = event["cwd"]
        .as_str()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    else {
        return String::new();
    };
    let cwd = normalize_path(cwd);
    let mut candidates = Vec::new();
    for entry in fs::read_dir(storage).into_iter().flatten().flatten() {
        let metadata = entry.path().join("workspace.json");
        let mut read = || -> Option<()> {
            let data: Value = serde_json::from_str(&fs::read_to_string(&metadata).ok()?).ok()?;
            let folder = local_path(&data["folder"]);
            let workspace = local_path(&data["workspace"]).filter(|p| p.is_file());
            let mut roots: Vec<_> = folder.into_iter().collect();
            if let Some(ref workspace) = workspace {
                // VS Code workspace files support comments and trailing commas.
                let config: Value = json5::from_str(&fs::read_to_string(workspace).ok()?).ok()?;
                for item in config["folders"].as_array()? {
                    if let Some(path) = item["path"].as_str() {
                        roots.push(normalize_path(workspace.parent()?.join(path)));
                    } else if let Some(path) = local_path(&item["uri"]) {
                        roots.push(path);
                    }
                }
            }
            for root in roots {
                if root.is_dir() && cwd.starts_with(&root) {
                    candidates.push((
                        root.components().count(),
                        fs::metadata(&metadata).ok()?.modified().ok()?,
                        workspace.clone().unwrap_or(root),
                    ));
                }
            }
            Some(())
        };
        let _ = read();
    }
    let target = candidates
        .into_iter()
        .max_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)))
        .map(|item| item.2)
        .unwrap_or(cwd);
    vscode_file_target(&target)
}

pub fn vscode_file_target(path: &Path) -> String {
    Url::from_file_path(path)
        .map(|url| {
            format!(
                "vscode://file{}",
                url.as_str().trim_start_matches("file://")
            )
        })
        .unwrap_or_default()
}

pub fn vscode_path(value: &str) -> Option<PathBuf> {
    let url = Url::parse(value).ok()?;
    if url.scheme() != "vscode"
        || url.host_str() != Some("file")
        || !url.path().starts_with('/')
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Url::parse(&format!("file://{}", url.path()))
        .ok()?
        .to_file_path()
        .ok()
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub target: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Allow,
    #[allow(dead_code)]
    Open,
}

impl Action {
    fn text(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Open => "open",
        }
    }
}

pub fn respond(path: &Path, action: Action) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if path.file_name().is_none_or(|s| s != "response")
        || parent.file_name().is_none_or(|s| {
            !s.to_string_lossy().starts_with("codex-approval-")
                && !s.to_string_lossy().starts_with("agent-monitor-approval-")
        })
        || parent
            .parent()
            .is_none_or(|p| p != Path::new("/tmp") && p != Path::new("/private/tmp"))
    {
        return false;
    }
    let Ok(meta) = fs::symlink_metadata(parent) else {
        return false;
    };
    // SAFETY: geteuid has no preconditions.
    if !meta.is_dir() || meta.mode() & 0o777 != 0o700 || meta.uid() != unsafe { libc::geteuid() } {
        return false;
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .and_then(|mut file| file.write_all(action.text().as_bytes()))
        .is_ok()
}

#[cfg(test)]
pub fn wait_response(path: &Path, duration: Duration) -> Option<Action> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        match fs::read_to_string(path).as_deref() {
            Ok("allow") => return Some(Action::Allow),
            Ok("open") => return Some(Action::Open),
            _ => thread::sleep(Duration::from_millis(100)),
        }
    }
    None
}

#[cfg(test)]
pub fn approval_directory() -> io::Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix("agent-monitor-approval-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in("/tmp")
}

pub fn decision(action: Option<Action>, test: bool) -> Option<Value> {
    (action == Some(Action::Allow) && !test).then(|| {
        json!({"hookSpecificOutput": {
            "hookEventName": "PermissionRequest", "decision": {"behavior": "allow"}
        }})
    })
}

/// Identify this app's own hook after a move, without touching other hook commands.
pub fn monitor_hook(command: &str, arguments: &[&str]) -> bool {
    let Some(words) = shlex::split(command) else {
        return false;
    };
    let Some((executable, args)) = words.split_first() else {
        return false;
    };
    let path = Path::new(executable);
    args.iter()
        .map(String::as_str)
        .eq(arguments.iter().copied())
        && path
            .file_name()
            .is_some_and(|name| name == "AIAgentMonitor")
        && path
            .parent()
            .is_some_and(|p| p.ends_with("AI Agent Monitor.app/Contents/MacOS"))
}
