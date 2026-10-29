//! Associate loaded Codex chats with live VS Code windows, not command working directories.
use crate::app::process::output_with_timeout;
use anyhow::Result;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime},
};

#[derive(Clone, Default)]
struct LogData {
    starts: HashMap<u32, String>,
    workspaces: Vec<(String, String)>,
    roles: HashMap<String, Role>,
}

#[derive(Clone)]
struct Role {
    timestamp: String,
    name: String,
}

#[derive(Clone)]
struct CachedLog {
    device: u64,
    inode: u64,
    length: u64,
    modified: Option<SystemTime>,
    offset: u64,
    anchor: [u8; 128],
    data: LogData,
}

static LOG_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedLog>>> = OnceLock::new();

fn timestamp(line: &str) -> Option<&str> {
    let stamp = line.get(..23)?;
    for (index, byte) in stamp.bytes().enumerate() {
        let expected = match index {
            4 | 7 => Some(b'-'),
            10 => Some(b' '),
            13 | 16 => Some(b':'),
            19 => Some(b'.'),
            _ => None,
        };
        if expected.is_some_and(|expected| byte != expected)
            || (expected.is_none() && !byte.is_ascii_digit())
        {
            return None;
        }
    }
    Some(stamp)
}

fn field<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=");
    body.split_whitespace()
        .find_map(|part| part.strip_prefix(&prefix))
}

fn storage_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_line(data: &mut LogData, line: &str, storage: &Path) {
    let Some(time) = timestamp(line) else {
        return;
    };
    let Some(body) = line.get(24..).and_then(|body| body.strip_prefix("[info] ")) else {
        return;
    };
    if let Some(rest) = body.strip_prefix("Extension host with pid ") {
        if let Some((pid, rest)) = rest.split_once(' ')
            && rest.starts_with("started")
            && let Ok(pid) = pid.parse::<u32>()
        {
            data.starts.insert(pid, time.to_owned());
        }
    } else if let Some(path) = body.strip_prefix("Skipping acquiring lock for ") {
        let prefix = format!("{}/", storage.display());
        if let Some(id) = path.trim_end().trim_end_matches('.').strip_prefix(&prefix)
            && storage_id(id)
        {
            data.workspaces.push((time.to_owned(), id.to_owned()));
        }
    } else if let Some(body) = body.strip_prefix("thread_stream_role_changed ")
        && let (Some(id), Some(role)) = (field(body, "conversationId"), field(body, "role"))
        && id.len() == 36
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        && data
            .roles
            .get(id)
            .is_none_or(|old| old.timestamp.as_str() <= time)
    {
        data.roles.insert(
            id.to_owned(),
            Role {
                timestamp: time.to_owned(),
                name: role.to_owned(),
            },
        );
    }
}

fn anchor(file: &mut fs::File, end: u64) -> std::io::Result<[u8; 128]> {
    let start = end.saturating_sub(128);
    let mut bytes = [0; 128];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut bytes[..(end - start) as usize])?;
    Ok(bytes)
}

fn read_log(
    cache: &mut HashMap<PathBuf, CachedLog>,
    path: &Path,
    storage: &Path,
) -> std::io::Result<LogData> {
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    let old = cache
        .get(path)
        .filter(|old| old.device == metadata.dev() && old.inode == metadata.ino());
    if let Some(old) = old
        && old.length == metadata.len()
        && old.modified == metadata.modified().ok()
    {
        return Ok(old.data.clone());
    }
    let previous = old.filter(|old| {
        metadata.len() >= old.length
            && anchor(&mut file, old.offset).is_ok_and(|bytes| bytes == old.anchor)
    });
    let mut state = previous.cloned().unwrap_or(CachedLog {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: 0,
        modified: None,
        offset: 0,
        anchor: [0; 128],
        data: LogData::default(),
    });
    file.seek(SeekFrom::Start(state.offset))?;
    let mut reader = BufReader::new((&mut file).take(metadata.len() - state.offset));
    let mut line = Vec::new();
    let mut oversized = false;
    let mut cursor = state.offset;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            break;
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let count = newline.map(|index| index + 1).unwrap_or(buffer.len());
        if !oversized && line.len() + count <= 64 * 1024 {
            line.extend_from_slice(&buffer[..count]);
        } else {
            oversized = true;
            line.clear();
        }
        reader.consume(count);
        cursor += count as u64;
        if newline.is_some() {
            if !oversized {
                parse_line(&mut state.data, &String::from_utf8_lossy(&line), storage);
            }
            line.clear();
            oversized = false;
            state.offset = cursor;
        }
    }
    drop(reader);
    state.length = metadata.len();
    state.modified = metadata.modified().ok();
    state.anchor = anchor(&mut file, state.offset)?;
    let data = state.data.clone();
    cache.insert(path.to_owned(), state);
    Ok(data)
}

fn candidate_logs(storage: &Path) -> Vec<PathBuf> {
    let Some(code) = storage.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for launch in fs::read_dir(code.join("logs"))
        .into_iter()
        .flatten()
        .flatten()
    {
        for window in fs::read_dir(launch.path()).into_iter().flatten().flatten() {
            let name = window.file_name();
            let name = name.to_string_lossy();
            if !name.strip_prefix("window").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            }) {
                continue;
            }
            let path = window.path().join("exthost/exthost.log");
            if path.is_file() {
                paths.push(path);
            }
        }
    }
    paths
}

fn live_logs(paths: &[PathBuf]) -> Result<HashMap<PathBuf, HashSet<u32>>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    // SAFETY: geteuid scopes process discovery to the current user.
    let uid = unsafe { libc::geteuid() }.to_string();
    let mut command = Command::new("/usr/sbin/lsof");
    command
        .args(["-nP", "-a", "-u", &uid, "-c", "Code Helper", "-F", "pn"])
        .args(paths);
    let (status, output, _) = output_with_timeout(&mut command, Duration::from_secs(2))?;
    // lsof may report unmatched historical paths while still returning live
    // matches. Only the returned PID/file pairs are used below.
    anyhow::ensure!(
        matches!(status, Some(0 | 1)),
        "Could not discover VS Code extension hosts"
    );
    let allowed: HashSet<_> = paths.iter().collect();
    let mut result: HashMap<PathBuf, HashSet<u32>> = HashMap::new();
    let mut pid = None;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse().ok();
        } else if let Some(value) = line.strip_prefix('n')
            && let Some(pid) = pid
        {
            let path = PathBuf::from(value);
            if allowed.contains(&path) {
                result.entry(path).or_default().insert(pid);
            }
        }
    }
    Ok(result)
}

fn workspace_path(storage: &Path, id: &str) -> Option<PathBuf> {
    if !storage_id(id) {
        return None;
    }
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(storage.join(id).join("workspace.json")).ok()?).ok()?;
    for key in ["workspace", "folder"] {
        if let Some(path) = value[key]
            .as_str()
            .and_then(|value| url::Url::parse(value).ok())
            .and_then(|url| url.to_file_path().ok())
            .filter(|path| {
                if key == "workspace" {
                    path.is_file()
                } else {
                    path.is_dir()
                }
            })
        {
            return Some(path);
        }
    }
    None
}

#[derive(Clone)]
struct Match {
    path: PathBuf,
    timestamp: String,
    owner: bool,
}

#[derive(Default)]
pub struct LiveSessions {
    pub active: HashSet<String>,
    pub workspaces: HashMap<String, PathBuf>,
    complete: bool,
}

#[cfg(test)]
fn resolve(
    storage: &Path,
    ids: &HashSet<String>,
    live: &HashMap<PathBuf, HashSet<u32>>,
    cache: &mut HashMap<PathBuf, CachedLog>,
) -> HashMap<String, PathBuf> {
    resolve_live(storage, ids, live, cache).workspaces
}

fn resolve_live(
    storage: &Path,
    ids: &HashSet<String>,
    live: &HashMap<PathBuf, HashSet<u32>>,
    cache: &mut HashMap<PathBuf, CachedLog>,
) -> LiveSessions {
    let mut result = LiveSessions {
        complete: true,
        ..LiveSessions::default()
    };
    let mut matches: HashMap<String, Match> = HashMap::new();
    let mut ambiguous = HashSet::new();
    let mut read_paths = HashSet::new();
    for (host_log, pids) in live {
        let Ok(host) = read_log(cache, host_log, storage) else {
            result.complete = false;
            continue;
        };
        read_paths.insert(host_log.clone());
        let Some((pid, start)) = host.starts.iter().max_by_key(|(_, time)| *time) else {
            continue;
        };
        if !pids.contains(pid) {
            continue;
        }
        let workspace = host
            .workspaces
            .iter()
            .filter(|(time, _)| time >= start)
            .max_by_key(|(time, _)| time)
            .and_then(|(_, id)| workspace_path(storage, id));
        let Some(directory) = host_log.parent() else {
            continue;
        };
        let mut window_roles: HashMap<String, Role> = HashMap::new();
        for entry in fs::read_dir(directory.join("openai.chatgpt"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !(name == "Codex.log" || (name.starts_with("Codex.") && name.ends_with(".log"))) {
                continue;
            }
            let path = entry.path();
            let Ok(data) = read_log(cache, &path, storage) else {
                result.complete = false;
                continue;
            };
            read_paths.insert(path);
            for (id, role) in data.roles {
                if ids.contains(&id)
                    && role.timestamp >= *start
                    && window_roles
                        .get(&id)
                        .is_none_or(|old| old.timestamp < role.timestamp)
                {
                    window_roles.insert(id, role);
                }
            }
        }
        for (id, role) in window_roles {
            if !matches!(role.name.as_str(), "owner" | "follower") {
                continue;
            }
            result.active.insert(id.clone());
            let Some(workspace) = workspace.as_ref() else {
                continue;
            };
            let candidate = Match {
                path: workspace.clone(),
                timestamp: role.timestamp,
                owner: role.name == "owner",
            };
            match matches.get(&id) {
                Some(old)
                    if (&old.timestamp, old.owner) > (&candidate.timestamp, candidate.owner) => {}
                Some(old)
                    if (&old.timestamp, old.owner) == (&candidate.timestamp, candidate.owner) =>
                {
                    if old.path != candidate.path {
                        ambiguous.insert(id);
                    }
                }
                _ => {
                    ambiguous.remove(&id);
                    matches.insert(id, candidate);
                }
            }
        }
    }
    cache.retain(|path, _| read_paths.contains(path));
    result.workspaces = matches
        .into_iter()
        .filter(|(id, _)| !ambiguous.contains(id))
        .map(|(id, value)| (id, value.path))
        .collect();
    result
}

pub fn workspaces(ids: &HashSet<String>, storage: &Path) -> HashMap<String, PathBuf> {
    live_sessions(ids, storage)
        .map(|live| live.workspaces)
        .unwrap_or_default()
}

/// None means discovery is unavailable; an empty verified set means no IDE
/// windows currently have these chats loaded. Directory existence is unrelated.
pub fn live_sessions(ids: &HashSet<String>, storage: &Path) -> Option<LiveSessions> {
    if ids.is_empty() {
        return Some(LiveSessions {
            complete: true,
            ..LiveSessions::default()
        });
    }
    let paths = candidate_logs(storage);
    if paths.is_empty() {
        return None;
    }
    let live = live_logs(&paths).ok()?;
    let mut cache = LOG_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let resolved = resolve_live(storage, ids, &live, &mut cache);
    resolved.complete.then_some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    const CHAT: &str = "01a0fc00-46fe-7141-a045-4bb5734e1fde";
    const OTHER: &str = "01a11096-73bf-79f3-b589-b07359ad1f40";

    fn line(second: u32, body: &str) -> String {
        format!("2026-10-28 11:00:{second:02}.000 [info] {body}\n")
    }
    fn role(second: u32, id: &str, name: &str) -> String {
        line(
            second,
            &format!(
                "thread_stream_role_changed conversationId={id} ownerClientId=example role={name}"
            ),
        )
    }
    fn append(path: &Path, text: &str) {
        fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    fn window(storage: &Path, index: u32, pid: u32, project: &Path) -> (PathBuf, PathBuf) {
        let host = storage
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(format!("logs/run/window{index}/exthost/exthost.log"));
        fs::create_dir_all(host.parent().unwrap().join("openai.chatgpt")).unwrap();
        let id = format!("{index:032x}");
        fs::create_dir_all(storage.join(&id)).unwrap();
        fs::write(
            storage.join(&id).join("workspace.json"),
            serde_json::json!({"folder":url::Url::from_file_path(project).unwrap().to_string()})
                .to_string(),
        )
        .unwrap();
        fs::write(
            &host,
            line(1, &format!("Extension host with pid {pid} started"))
                + &line(
                    1,
                    &format!(
                        "Skipping acquiring lock for {}.",
                        storage.join(id).display()
                    ),
                ),
        )
        .unwrap();
        let log = host.parent().unwrap().join("openai.chatgpt/Codex.log");
        fs::write(&log, "").unwrap();
        (host, log)
    }

    #[test]
    fn follows_window_workspace_across_rotation_and_only_reads_complete_appended_events() {
        let temp = tempfile::tempdir().unwrap();
        let storage = temp.path().join("Code/User/workspaceStorage");
        let project = temp.path().join("actual project #1");
        fs::create_dir(&project).unwrap();
        let (host, log) = window(&storage, 4, 123, &project);
        fs::write(&log, role(2, CHAT, "follower")).unwrap();
        let live = HashMap::from([(host, HashSet::from([123]))]);
        let ids = HashSet::from([CHAT.to_owned(), OTHER.to_owned()]);
        let mut cache = HashMap::new();
        assert_eq!(
            resolve(&storage, &ids, &live, &mut cache).get(CHAT),
            Some(&project)
        );
        let next = role(3, OTHER, "owner");
        append(&log, next.trim_end_matches('\n'));
        assert!(!resolve(&storage, &ids, &live, &mut cache).contains_key(OTHER));
        append(&log, "\n");
        assert_eq!(
            resolve(&storage, &ids, &live, &mut cache).get(OTHER),
            Some(&project)
        );
        fs::rename(&log, log.with_file_name("Codex.1.log")).unwrap();
        fs::write(&log, role(4, CHAT, "none")).unwrap();
        let resolved = resolve(&storage, &ids, &live, &mut cache);
        assert!(!resolved.contains_key(CHAT));
        assert_eq!(resolved.get(OTHER), Some(&project));
        assert!(resolve(&storage, &ids, &HashMap::new(), &mut cache).is_empty());
    }

    #[test]
    fn closed_windows_disappear_even_while_a_shared_codex_process_retains_the_rollout() {
        let temp = tempfile::tempdir().unwrap();
        let storage = temp.path().join("Code/User/workspaceStorage");
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        let (host, log) = window(&storage, 4, 123, &project);
        fs::write(&log, role(2, CHAT, "follower")).unwrap();
        let ids = HashSet::from([CHAT.to_owned()]);
        let mut cache = HashMap::new();
        let live = HashMap::from([(host.clone(), HashSet::from([123]))]);
        assert!(
            resolve_live(&storage, &ids, &live, &mut cache)
                .active
                .contains(CHAT)
        );
        // Missing/deleted worktrees do not imply that a live window is closed.
        fs::remove_dir(&project).unwrap();
        let resolved = resolve_live(&storage, &ids, &live, &mut cache);
        assert!(resolved.active.contains(CHAT));
        assert!(resolved.workspaces.is_empty());
        // Closing the extension host must remove membership, including cached roles.
        let resolved = resolve_live(&storage, &ids, &HashMap::new(), &mut cache);
        assert!(resolved.complete);
        assert!(resolved.active.is_empty());
        // Reopening the window and loading the same chat restores membership.
        append(&host, &line(3, "Extension host with pid 456 started"));
        append(&log, &role(4, CHAT, "follower"));
        let reopened = HashMap::from([(host, HashSet::from([456]))]);
        assert!(
            resolve_live(&storage, &ids, &reopened, &mut cache)
                .active
                .contains(CHAT)
        );
    }

    #[test]
    fn ignores_previous_extension_host_generations_even_if_the_old_process_is_still_alive() {
        let temp = tempfile::tempdir().unwrap();
        let storage = temp.path().join("Code/User/workspaceStorage");
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        let (host, log) = window(&storage, 4, 123, &project);
        fs::write(&log, role(2, CHAT, "owner")).unwrap();
        let ids = HashSet::from([CHAT.to_owned()]);
        let mut cache = HashMap::new();
        let old = HashMap::from([(host.clone(), HashSet::from([123]))]);
        assert!(resolve(&storage, &ids, &old, &mut cache).contains_key(CHAT));
        append(&host, &line(3, "Extension host with pid 456 started"));
        append(
            &host,
            &line(
                3,
                &format!(
                    "Skipping acquiring lock for {}.",
                    storage.join(format!("{:032x}", 4)).display()
                ),
            ),
        );
        assert!(resolve(&storage, &ids, &old, &mut cache).is_empty());
        let current = HashMap::from([(host, HashSet::from([456]))]);
        assert!(resolve(&storage, &ids, &current, &mut cache).is_empty());
        append(&log, &role(4, CHAT, "follower"));
        assert_eq!(
            resolve(&storage, &ids, &current, &mut cache).get(CHAT),
            Some(&project)
        );
    }

    #[test]
    fn selects_latest_window_and_does_not_guess_when_current_links_are_ambiguous() {
        let temp = tempfile::tempdir().unwrap();
        let storage = temp.path().join("Code/User/workspaceStorage");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let (a, alog) = window(&storage, 1, 1, &first);
        let (b, blog) = window(&storage, 2, 2, &second);
        let (c, clog) = window(&storage, 3, 3, &first);
        fs::write(&alog, role(2, CHAT, "follower")).unwrap();
        fs::write(&blog, role(2, CHAT, "follower")).unwrap();
        fs::write(&clog, role(2, CHAT, "follower")).unwrap();
        let live = HashMap::from([
            (a, HashSet::from([1])),
            (b, HashSet::from([2])),
            (c, HashSet::from([3])),
        ]);
        let ids = HashSet::from([CHAT.to_owned()]);
        let mut cache = HashMap::new();
        assert!(resolve(&storage, &ids, &live, &mut cache).is_empty());
        append(&alog, &role(3, CHAT, "owner"));
        append(&blog, &role(4, CHAT, "follower"));
        assert_eq!(
            resolve(&storage, &ids, &live, &mut cache).get(CHAT),
            Some(&second)
        );
    }

    #[test]
    fn opens_the_workspace_file_and_ignores_nested_text_and_remote_paths() {
        let temp = tempfile::tempdir().unwrap();
        let storage = temp.path().join("Code/User/workspaceStorage");
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        let (host, log) = window(&storage, 4, 123, &project);
        let workspace = temp.path().join("dev #1.code-workspace");
        fs::write(&workspace, "{}").unwrap();
        let metadata = storage.join(format!("{:032x}", 4)).join("workspace.json");
        fs::write(&metadata, serde_json::json!({"workspace":url::Url::from_file_path(&workspace).unwrap().to_string()}).to_string()).unwrap();
        fs::write(&log, role(2, CHAT, "follower") + &line(3, &format!("cli: message=\"thread_stream_role_changed conversationId={OTHER} role=owner\""))).unwrap();
        let live = HashMap::from([(host, HashSet::from([123]))]);
        let ids = HashSet::from([CHAT.to_owned(), OTHER.to_owned()]);
        let mut cache = HashMap::new();
        let result = resolve(&storage, &ids, &live, &mut cache);
        assert_eq!(result.get(CHAT), Some(&workspace));
        assert!(!result.contains_key(OTHER));
        let session = crate::agents::sessions::Session {
            agent: "codex",
            id: CHAT.into(),
            title: "Chat".into(),
            cwd: temp.path().join("deleted"),
            workspace: Some(workspace.clone()),
            git_branch: None,
            updated_at: 0,
            client: crate::agents::sessions::Client::VSCode,
            activity: crate::agents::sessions::Activity::Idle,
            last_message: String::new(),
            approval: None,
        };
        assert_eq!(
            crate::app::core::vscode_path(&session.open_target().unwrap()),
            Some(workspace)
        );
        fs::write(
            &metadata,
            r#"{"folder":"vscode-remote://ssh-remote+host/project"}"#,
        )
        .unwrap();
        assert!(resolve(&storage, &ids, &live, &mut cache).is_empty());
    }
}
