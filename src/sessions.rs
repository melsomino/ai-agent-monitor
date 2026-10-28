//! Read the local session index without resuming conversations or running models.
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufReader, Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime},
};

pub fn open(home: &Path) -> Result<Vec<Session>> {
    // SAFETY: geteuid has no preconditions and scopes discovery to this user.
    let uid = unsafe { libc::geteuid() }.to_string();
    let mut command = Command::new("/usr/sbin/lsof");
    command.args(["-nP", "-a", "-u", &uid, "-c", "codex", "-F", "pcafn"]);
    let (status, output, errors) = output_with_timeout(&mut command, Duration::from_secs(3))?;
    anyhow::ensure!(
        status == Some(0) || (status == Some(1) && errors.trim().is_empty()),
        "Could not read open Codex sessions: {}",
        errors.trim()
    );
    let files = open_files(&output, home);
    let ids = files.keys().cloned().collect();
    let mut sessions = read_open(home, &ids)?;
    let pending = crate::pending::list();
    for session in &mut sessions {
        if let Some(path) = files.get(&session.id) {
            enrich(session, path);
        }
        session.approval = pending
            .iter()
            .filter(|request| request.session_id == session.id)
            .max_by_key(|request| request.expires)
            .cloned();
        if session.approval.is_some() {
            session.activity = Activity::Confirmation;
        }
    }
    let vscode_ids = sessions
        .iter()
        .filter(|session| session.client == Client::VSCode)
        .map(|session| session.id.clone())
        .collect();
    let storage =
        crate::core::home().join("Library/Application Support/Code/User/workspaceStorage");
    let mut workspaces = crate::vscode::workspaces(&vscode_ids, &storage);
    for session in &mut sessions {
        session.workspace = workspaces.remove(&session.id);
    }
    Ok(sessions)
}

pub(crate) fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<(Option<i32>, String, String)> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().context("missing process output")?;
    let reader = thread::spawn(move || {
        let mut output = String::new();
        stdout.read_to_string(&mut output).map(|_| output)
    });
    let mut stderr = child
        .stderr
        .take()
        .context("missing process error output")?;
    let error_reader = thread::spawn(move || {
        let mut errors = String::new();
        stderr.read_to_string(&mut errors).map(|_| errors)
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    let output = reader
        .join()
        .map_err(|_| anyhow::anyhow!("process output reader failed"))??;
    let errors = error_reader
        .join()
        .map_err(|_| anyhow::anyhow!("process error reader failed"))??;
    let status = status.context("session discovery timed out")?;
    Ok((status.code(), output, errors))
}

fn session_id(path: &Path, home: &Path) -> Option<String> {
    let relative = path.strip_prefix(home).ok()?;
    if !relative.starts_with("sessions") && !relative.starts_with("threads") {
        return None;
    }
    for component in relative.components() {
        let value = component.as_os_str().to_str()?.trim_end_matches(".jsonl");
        let Some(candidate) = value
            .len()
            .checked_sub(36)
            .and_then(|start| value.get(start..))
        else {
            continue;
        };
        if candidate.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        }) {
            return Some(candidate.to_owned());
        }
    }
    None
}

fn open_files(output: &str, home: &Path) -> HashMap<String, PathBuf> {
    let mut ids = HashMap::new();
    let mut codex = false;
    let mut writable = false;
    for line in output.lines() {
        if line.starts_with('p') {
            codex = false;
            writable = false;
        } else if let Some(command) = line.strip_prefix('c') {
            codex = command == "codex";
        } else if line.starts_with('f') {
            writable = false;
        } else if let Some(access) = line.strip_prefix('a') {
            writable = access == "w" || access == "u";
        } else if codex
            && writable
            && let Some(path) = line.strip_prefix('n')
            && let Some(id) = session_id(Path::new(path), home)
        {
            ids.insert(id, PathBuf::from(path));
        }
    }
    ids
}

#[cfg(test)]
fn ids_from_open_files(output: &str, home: &Path) -> HashSet<String> {
    open_files(output, home).into_keys().collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Client {
    Terminal,
    VSCode,
    Desktop,
    #[default]
    Unknown,
}
impl Client {
    pub fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::VSCode => "VS Code",
            Self::Desktop => "Codex",
            Self::Unknown => "Codex",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Activity {
    Working,
    Idle,
    Confirmation,
    #[default]
    Unknown,
}
impl Activity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "Working",
            Self::Idle => "Idle",
            Self::Confirmation => "Needs approval",
            Self::Unknown => "Unknown status",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub workspace: Option<PathBuf>,
    pub updated_at: i64,
    pub client: Client,
    pub activity: Activity,
    pub last_message: String,
    pub approval: Option<crate::pending::Request>,
}

impl Session {
    pub fn project_path(&self) -> &Path {
        self.workspace.as_deref().unwrap_or(&self.cwd)
    }

    pub fn vscode_target(&self, storage: &Path) -> String {
        if let Some(workspace) = &self.workspace {
            crate::core::vscode_file_target(workspace)
        } else {
            crate::core::vscode_target(&serde_json::json!({"cwd": self.cwd}), storage)
        }
    }
}

fn read_open(home: &Path, ids: &HashSet<String>) -> Result<Vec<Session>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut databases: Vec<_> = match fs::read_dir(home) {
        Ok(entries) => entries
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
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    databases.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
    let mut last_error = None;
    for (_, path) in databases {
        match read(&path, ids) {
            Ok(sessions) => return Ok(sessions),
            Err(error) => last_error = Some(error),
        }
    }
    match last_error {
        Some(error) => Err(error),
        None => Ok(Vec::new()),
    }
}

fn read(path: &Path, ids: &HashSet<String>) -> Result<Vec<Session>> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(150))?;
    let columns = db
        .prepare("PRAGMA table_info(threads)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has = |column: &str| columns.iter().any(|value| value == column);
    anyhow::ensure!(
        has("id") && has("title"),
        "Unsupported session database schema"
    );
    let title = if has("name") {
        "COALESCE(NULLIF(TRIM(name), ''), NULLIF(TRIM(title), ''), 'Untitled')"
    } else {
        "COALESCE(NULLIF(TRIM(title), ''), 'Untitled')"
    };
    let cwd = if has("cwd") {
        "COALESCE(cwd, '')"
    } else {
        "''"
    };
    let updated = if has("updated_at_ms") && has("updated_at") {
        "COALESCE(updated_at_ms / 1000, updated_at, 0)"
    } else if has("updated_at") {
        "COALESCE(updated_at, 0)"
    } else {
        "0"
    };
    let archived = if has("archived") {
        "AND archived = 0"
    } else {
        ""
    };
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "SELECT id, {title}, {cwd}, {updated} AS updated FROM threads WHERE id IN ({placeholders}) {archived} ORDER BY updated DESC, id"
    );
    let mut statement = db.prepare(&sql)?;
    statement
        .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            Ok(Session {
                id: row.get(0)?,
                title: row.get(1)?,
                cwd: PathBuf::from(row.get::<_, String>(2)?),
                workspace: None,
                updated_at: row.get(3)?,
                client: Client::Unknown,
                activity: Activity::Unknown,
                last_message: String::new(),
                approval: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn enrich(session: &mut Session, path: &Path) {
    let Ok(mut file) = fs::File::open(path) else {
        return;
    };
    let mut head = Vec::new();
    if (&mut file).take(131072).read_to_end(&mut head).is_ok()
        && let Some(line) = head.split(|byte| *byte == b'\n').next()
        && let Ok(value) = serde_json::from_slice::<serde_json::Value>(line)
    {
        let meta = &value["payload"];
        session.client = if meta["originator"]
            .as_str()
            .is_some_and(|s| s.contains("Desktop"))
        {
            Client::Desktop
        } else {
            match meta["source"].as_str() {
                Some("vscode") => Client::VSCode,
                Some("cli") => Client::Terminal,
                _ => Client::Unknown,
            }
        };
    }
    let Ok(metadata) = file.metadata() else {
        return;
    };
    let length = metadata.len();
    let start = length.saturating_sub(4 * 1024 * 1024);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return;
    }
    let mut tail = String::new();
    // Read bytes so a seek into a multibyte character does not invalidate the tail.
    let mut bytes = Vec::new();
    if (&mut file)
        .take(length - start)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 != length - start
    {
        return;
    }
    tail.push_str(&String::from_utf8_lossy(&bytes));
    session.activity = Activity::Unknown;
    parse_events(session, &tail);
    session.activity =
        recover_activity(&mut file, path, &metadata, start, &bytes, session.activity);
}

#[derive(Clone, Copy)]
struct ActivityCheckpoint {
    device: u64,
    inode: u64,
    length: u64,
    modified: Option<SystemTime>,
    complete_end: u64,
    anchor_end: u64,
    anchor: [u8; 128],
    activity: Activity,
}

static ACTIVITY_CACHE: OnceLock<Mutex<HashMap<PathBuf, ActivityCheckpoint>>> = OnceLock::new();

fn recover_activity(
    file: &mut fs::File,
    path: &Path,
    metadata: &fs::Metadata,
    tail_start: u64,
    tail: &[u8],
    activity: Activity,
) -> Activity {
    let cache = ACTIVITY_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let previous = cache
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(path)
        .copied();
    let previous = previous.filter(|checkpoint| {
        checkpoint.device == metadata.dev()
            && checkpoint.inode == metadata.ino()
            && (metadata.len() > checkpoint.length
                || (metadata.len() == checkpoint.length
                    && metadata.modified().ok() == checkpoint.modified))
            && activity_anchor(file, checkpoint.anchor_end)
                .is_ok_and(|bytes| bytes == checkpoint.anchor)
    });
    let lower = previous
        .map(|checkpoint| checkpoint.complete_end)
        .unwrap_or(0);
    let complete_end = tail
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| tail_start + index as u64 + 1)
        .unwrap_or(lower);
    let activity = if activity != Activity::Unknown {
        activity
    } else {
        // Include the record crossing the 4 MiB boundary: it may itself be a
        // large completion event. New data outside the tail must also be checked
        // before using a checkpoint; a single tool output can exceed 4 MiB.
        let search_end = tail
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| tail_start + index as u64 + 1)
            .unwrap_or(metadata.len());
        let found = if search_end > lower {
            match last_activity_before(file, search_end, lower) {
                Ok(activity) => activity,
                Err(_) => return Activity::Unknown,
            }
        } else {
            None
        };
        found
            .or(previous.map(|checkpoint| checkpoint.activity))
            .unwrap_or(Activity::Unknown)
    };
    let anchor_end = if complete_end > 0 {
        complete_end
    } else {
        metadata.len().min(128)
    };
    let Ok(anchor) = activity_anchor(file, anchor_end) else {
        return activity;
    };
    let checkpoint = ActivityCheckpoint {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: metadata.len(),
        modified: metadata.modified().ok(),
        complete_end,
        anchor_end,
        anchor,
        activity,
    };
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    if cache.len() >= 512 && !cache.contains_key(path) {
        cache.clear();
    }
    cache.insert(path.to_owned(), checkpoint);
    activity
}

fn activity_anchor(file: &mut fs::File, end: u64) -> std::io::Result<[u8; 128]> {
    let start = end.saturating_sub(128);
    let mut bytes = [0; 128];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut bytes[..(end - start) as usize])?;
    Ok(bytes)
}

#[derive(serde::Deserialize)]
struct ActivityRecord {
    #[serde(rename = "type")]
    kind: String,
    payload: ActivityPayload,
}

#[derive(serde::Deserialize)]
struct ActivityPayload {
    #[serde(rename = "type", default)]
    kind: String,
}

fn activity_record(file: &mut fs::File, start: u64, end: u64) -> std::io::Result<Option<Activity>> {
    file.seek(SeekFrom::Start(start))?;
    // The small typed envelope ignores message/tool contents while streaming;
    // even an oversized JSONL record does not allocate its output as a String.
    let reader = BufReader::new((&mut *file).take(end - start));
    let Ok(record) = serde_json::from_reader::<_, ActivityRecord>(reader) else {
        return Ok(None);
    };
    Ok(if record.kind == "event_msg" {
        match record.payload.kind.as_str() {
            "task_started" | "turn_started" => Some(Activity::Working),
            "task_complete" | "turn_complete" | "turn_aborted" => Some(Activity::Idle),
            _ => None,
        }
    } else {
        None
    })
}

fn last_activity_before(
    file: &mut fs::File,
    end: u64,
    lower: u64,
) -> std::io::Result<Option<Activity>> {
    let mut buffer = vec![0; 64 * 1024];
    let mut position = end;
    let mut record_end = end;
    while position > lower {
        let start = position.saturating_sub(buffer.len() as u64).max(lower);
        let count = (position - start) as usize;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..count])?;
        for index in (0..count).rev().filter(|index| buffer[*index] == b'\n') {
            let newline = start + index as u64;
            if newline + 1 < record_end
                && let Some(activity) = activity_record(file, newline + 1, record_end)?
            {
                return Ok(Some(activity));
            }
            record_end = newline;
        }
        position = start;
    }
    if record_end > lower {
        activity_record(file, lower, record_end)
    } else {
        Ok(None)
    }
}

fn parse_events(session: &mut Session, tail: &str) {
    for line in tail.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let payload = &value["payload"];
        match value["type"].as_str() {
            Some("event_msg") => match payload["type"].as_str() {
                Some("task_started" | "turn_started") => session.activity = Activity::Working,
                Some("task_complete" | "turn_complete" | "turn_aborted") => {
                    session.activity = Activity::Idle
                }
                Some("agent_message") => {
                    if let Some(text) = payload["message"].as_str().filter(|s| !s.is_empty()) {
                        session.last_message = text.to_owned();
                    }
                }
                _ => {}
            },
            Some("response_item")
                if payload["type"] == "message" && payload["role"] == "assistant" =>
            {
                if let Some(content) = payload["content"].as_array() {
                    let text = content
                        .iter()
                        .filter_map(|item| item["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.is_empty() {
                        session.last_message = text;
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn activity_survives_oversized_records_and_cached_gaps_without_guessing_from_tool_output() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("large.jsonl");
        let mut session = Session {
            id: "one".into(),
            title: "Chat".into(),
            cwd: PathBuf::new(),
            workspace: None,
            updated_at: 0,
            client: Client::Unknown,
            activity: Activity::Unknown,
            last_message: String::new(),
            approval: None,
        };
        let oversized = serde_json::json!({"type":"response_item", "payload":{
            "type":"function_call_output", "output": "x".repeat(5 * 1024 * 1024)
                + r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#
        }})
        .to_string();
        let mut file = fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"type":"event_msg","payload":{{"type":"task_started"}}}}"#
        )
        .unwrap();
        writeln!(file, "{oversized}").unwrap();
        file.flush().unwrap();
        enrich(&mut session, &path);
        assert_eq!(session.activity, Activity::Working);

        // A new completion also falls outside the tail. A previous cached
        // Working state must not hide that completion after a large append.
        writeln!(
            file,
            r#"{{"type":"event_msg","payload":{{"type":"task_complete"}}}}"#
        )
        .unwrap();
        writeln!(file, "{oversized}").unwrap();
        file.flush().unwrap();
        enrich(&mut session, &path);
        assert_eq!(session.activity, Activity::Idle);

        // Truncation invalidates the checkpoint rather than reusing Idle.
        fs::write(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"turn_started\"}}\n",
        )
        .unwrap();
        enrich(&mut session, &path);
        assert_eq!(session.activity, Activity::Working);
        fs::write(&path, format!("{oversized}\n")).unwrap();
        enrich(&mut session, &path);
        assert_eq!(session.activity, Activity::Unknown);
    }

    #[test]
    fn reads_activity_and_last_assistant_message_without_tool_output() {
        let mut session = Session {
            id: "one".into(),
            title: "Chat".into(),
            cwd: PathBuf::new(),
            workspace: None,
            updated_at: 0,
            client: Client::Unknown,
            activity: Activity::Unknown,
            last_message: String::new(),
            approval: None,
        };
        parse_events(
            &mut session,
            r#"{"type":"event_msg","payload":{"type":"task_started"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Last answer"}]}}
{"type":"response_item","payload":{"type":"function_call_output","output":"SECRET"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"User prompt"}]}}
{"type":"event_msg","payload":{"type":"token_count"}}
"#,
        );
        assert_eq!(session.activity, Activity::Working);
        assert_eq!(session.last_message, "Last answer");
        parse_events(
            &mut session,
            r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#,
        );
        assert_eq!(session.activity, Activity::Idle);
        parse_events(
            &mut session,
            r#"partial json
{"type":"event_msg","payload":{"type":"turn_started"}}
{"type":"event_msg","payload":{"type":"turn_aborted"}}
"#,
        );
        assert_eq!(session.activity, Activity::Idle);
    }

    #[test]
    fn session_metadata_distinguishes_clients() {
        let home = tempfile::tempdir().unwrap();
        let db = Connection::open(home.path().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads (id TEXT, title TEXT); INSERT INTO threads VALUES ('one', 'Chat');").unwrap();
        let mut session = read_open(home.path(), &HashSet::from(["one".into()]))
            .unwrap()
            .remove(0);
        let transcript = home.path().join("rollout.jsonl");
        for (source, originator, client) in [
            ("cli", "codex-tui", Client::Terminal),
            ("vscode", "codex_vscode", Client::VSCode),
            ("vscode", "Codex Desktop", Client::Desktop),
        ] {
            fs::write(&transcript, serde_json::json!({"type":"session_meta","payload":{"source":source,"originator":originator}}).to_string()+"\n").unwrap();
            enrich(&mut session, &transcript);
            assert_eq!(session.client, client);
            assert_eq!(session.activity, Activity::Unknown);
        }
    }

    #[test]
    fn only_open_sessions_are_shown_including_old_idle_sessions() {
        let home = tempfile::tempdir().unwrap();
        let db = Connection::open(home.path().join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads (id TEXT, name TEXT, title TEXT, cwd TEXT, updated_at INT, updated_at_ms INT, archived INT);
            INSERT INTO threads VALUES ('old', NULL, 'Старый', '/work/one', 10, NULL, 0);
            INSERT INTO threads VALUES ('new', 'Переименованный', 'Initial', '/work/two', 20, 25000, 0);
            INSERT INTO threads VALUES ('archived', NULL, 'Archived', '/work', 100, NULL, 1);").unwrap();
        let sessions = read_open(home.path(), &HashSet::from(["new".into()])).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "new");
        assert_eq!(sessions[0].title, "Переименованный");
        assert_eq!(sessions[0].updated_at, 25);
        assert_eq!(
            read_open(
                home.path(),
                &HashSet::from(["old".into(), "new".into(), "archived".into()])
            )
            .unwrap()
            .len(),
            2
        );
        let idle = read_open(home.path(), &HashSet::from(["old".into()])).unwrap();
        assert_eq!(idle[0].id, "old");
        assert!(read_open(home.path(), &HashSet::new()).unwrap().is_empty());
    }

    #[test]
    fn supports_legacy_schema_and_absent_or_corrupt_databases() {
        let home = tempfile::tempdir().unwrap();
        let ids = HashSet::from(["one".into()]);
        assert!(read_open(home.path(), &ids).unwrap().is_empty());
        fs::write(home.path().join("state_10.sqlite"), "invalid").unwrap();
        let db = Connection::open(home.path().join("state_9.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE threads (id TEXT, title TEXT); INSERT INTO threads VALUES ('one', '');",
        )
        .unwrap();
        let sessions = read_open(home.path(), &ids).unwrap();
        assert_eq!(sessions[0].title, "Untitled");
        assert_eq!(sessions[0].updated_at, 0);
        assert!(read_open(home.path(), &HashSet::new()).unwrap().is_empty());
    }

    #[test]
    fn discovers_only_writable_session_files_in_real_codex_processes() {
        let id = "01a17d31-76f4-7a83-be7f-ed970362a17a";
        let other = "01a16491-6904-7a21-8c2e-06e9b3e9b445";
        let home = Path::new("/home/user/.codex");
        let output = format!(
            "p10\nccodex\nf5\nau\nn/home/user/.codex/sessions/2026/rollout-date-{id}.jsonl\nf6\nar\nn/home/user/.codex/sessions/rollout-date-{other}.jsonl\np11\nccodex-code-mode-host\nf8\nau\nn/home/user/.codex/sessions/rollout-date-{other}.jsonl\np12\nccodex\nf9\nau\nn/other/home/sessions/rollout-date-{other}.jsonl\n"
        );
        assert_eq!(
            ids_from_open_files(&output, home),
            HashSet::from([id.into()])
        );
        assert!(ids_from_open_files("", home).is_empty());
        assert_eq!(
            session_id(
                Path::new(&format!("/home/user/.codex/threads/{id}/data.sqlite")),
                home
            ),
            Some(id.into())
        );
    }
}
