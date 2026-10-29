//! Hook-owned snapshots bound to the lifetime of an actual agent/extension host.
use crate::{
    agents::sessions::{Activity, Client, Session},
    app::{core, process::output_with_timeout},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub agent: String,
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub transcript: Option<PathBuf>,
    pub pid: u32,
    pub birth: String,
    pub client: String,
    pub status: String,
    pub last_message: String,
    pub updated: u64,
}

pub fn root() -> PathBuf {
    core::home().join("Library/Application Support/AI Agent Monitor/agents")
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub fn text(value: &str) -> String {
    value.chars().take(32_000).collect()
}
fn directory(root: &Path, agent: &str) -> PathBuf {
    root.join(agent)
}
fn path(root: &Path, agent: &str, id: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        matches!(agent, "claude" | "cline") && valid_id(id),
        "Invalid agent session identity"
    );
    Ok(directory(root, agent).join(format!("{id}.json")))
}
fn private(meta: &fs::Metadata) -> bool {
    // SAFETY: geteuid has no preconditions.
    meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0
}
pub fn save(root: &Path, record: &Record) -> Result<()> {
    let path = path(root, &record.agent, &record.id)?;
    let directory = path.parent().unwrap();
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let meta = fs::symlink_metadata(directory)?;
    anyhow::ensure!(
        meta.is_dir() && private(&meta),
        "Session directory must be private and owned by this user"
    );
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(temp.as_file_mut(), record)?;
    temp.write_all(b"\n")?;
    temp.persist(path)?;
    Ok(())
}
pub fn load(root: &Path, agent: &str, id: &str) -> Option<Record> {
    let path = path(root, agent, id).ok()?;
    let parent = fs::symlink_metadata(path.parent()?).ok()?;
    let meta = fs::symlink_metadata(&path).ok()?;
    if !parent.is_dir()
        || !private(&parent)
        || !meta.is_file()
        || !private(&meta)
        || meta.len() > 1_048_576
    {
        return None;
    }
    let record: Record = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (record.agent == agent && record.id == id).then_some(record)
}
pub fn remove(root: &Path, agent: &str, id: &str) -> Result<()> {
    let path = path(root, agent, id)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub fn records(root: &Path, agent: &str) -> Vec<Record> {
    fs::read_dir(directory(root, agent))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let id = entry.path().file_stem()?.to_str()?.to_owned();
            load(root, agent, &id)
        })
        .collect()
}

/// pid + process birth time avoids treating a reused PID as a live conversation.
pub fn live(records: Vec<Record>, processes: &HashMap<u32, String>) -> Vec<Record> {
    let mut records: Vec<_> = records
        .into_iter()
        .filter(|r| processes.get(&r.pid) == Some(&r.birth))
        .collect();
    // Cline has one selected task per extension host. Never show its whole history.
    let mut latest: HashMap<u32, u64> = HashMap::new();
    for r in &records {
        if r.agent == "cline" {
            latest
                .entry(r.pid)
                .and_modify(|v| *v = (*v).max(r.updated))
                .or_insert(r.updated);
        }
    }
    records.retain(|r| r.agent != "cline" || latest.get(&r.pid) == Some(&r.updated));
    records
}
pub fn open_records(agent: &str) -> Vec<Record> {
    let records = records(&root(), agent);
    if records.is_empty() {
        return records;
    }
    let pids = records
        .iter()
        .map(|r| r.pid.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut command = Command::new("/bin/ps");
    command
        .env("LC_ALL", "C")
        .args(["-ww", "-p", &pids, "-o", "pid=,uid=,lstart="]);
    let Ok((_, output, _)) = output_with_timeout(&mut command, Duration::from_secs(2)) else {
        return Vec::new();
    };
    let mut processes = HashMap::new();
    for line in output.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() == 7
            && fields[1].parse::<u32>().ok() == Some(unsafe { libc::geteuid() })
            && let Ok(pid) = fields[0].parse()
        {
            processes.insert(pid, fields[2..].join(" "));
        }
    }
    live(records, &processes)
}
#[derive(Clone)]
struct ParentProcess {
    parent: u32,
    birth: String,
    command: String,
}
fn owner_from(
    agent: &str,
    mut pid: u32,
    mut read: impl FnMut(u32) -> Result<Option<ParentProcess>>,
) -> Result<(u32, String, String)> {
    let mut candidate = None;
    for _ in 0..12 {
        let process = match read(pid) {
            Ok(Some(process)) => process,
            _ if candidate.is_some() => break,
            _ => anyhow::bail!("Agent parent process is not available"),
        };
        let name = Path::new(&process.command)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        if process.command.contains("Code Helper (Plugin)") {
            let (pid, birth) = candidate.unwrap_or((pid, process.birth));
            return Ok((pid, birth, "vscode".into()));
        }
        if candidate.is_none()
            && (name == agent
                || (agent == "claude" && process.command.contains("/claude/"))
                || name == "node")
        {
            candidate = Some((pid, process.birth));
        }
        pid = process.parent;
        if pid <= 1 {
            break;
        }
    }
    candidate
        .map(|(pid, birth)| (pid, birth, "terminal".into()))
        .ok_or_else(|| anyhow::anyhow!("Could not identify the live {agent} process"))
}
pub fn owner(agent: &str) -> Result<(u32, String, String)> {
    // SAFETY: getppid/geteuid have no preconditions.
    owner_from(agent, unsafe { libc::getppid() } as u32, |pid| {
        let mut command = Command::new("/bin/ps");
        command.env("LC_ALL", "C").args([
            "-ww",
            "-p",
            &pid.to_string(),
            "-o",
            "ppid=,uid=,lstart=,comm=",
        ]);
        let (_, output, _) = output_with_timeout(&mut command, Duration::from_secs(1))?;
        let fields: Vec<_> = output.split_whitespace().collect();
        if fields.len() < 8 || fields[1].parse::<u32>().ok() != Some(unsafe { libc::geteuid() }) {
            return Ok(None);
        }
        Ok(Some(ParentProcess {
            parent: fields[0].parse().context("Invalid parent PID")?,
            birth: fields[2..7].join(" "),
            command: fields[7..].join(" "),
        }))
    })
}
pub fn snapshot(r: &Record) -> Session {
    Session {
        agent: if r.agent == "claude" {
            "claude"
        } else {
            "cline"
        },
        id: r.id.clone(),
        title: r.title.clone(),
        cwd: r.cwd.clone(),
        workspace: None,
        git_branch: None,
        updated_at: (r.updated / 1_000_000_000) as i64,
        client: if r.client == "vscode" {
            Client::VSCode
        } else {
            Client::Terminal
        },
        activity: match r.status.as_str() {
            "working" => Activity::Working,
            "confirmation" => Activity::Confirmation,
            "idle" => Activity::Idle,
            _ => Activity::Unknown,
        },
        last_message: r.last_message.clone(),
        approval: None,
    }
}
/// Read a bounded transcript tail. Never trust command output as lifecycle events.
pub fn tail(path: &Path) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(2 * 1024 * 1024);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(2 * 1024 * 1024).read_to_end(&mut bytes).ok()?;
    if start > 0 {
        let first = bytes.iter().position(|b| *b == b'\n')?;
        bytes.drain(..=first);
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(agent: &str, id: &str, pid: u32, updated: u64) -> Record {
        Record {
            agent: agent.into(),
            id: id.into(),
            title: id.into(),
            cwd: "/project".into(),
            transcript: None,
            pid,
            birth: "birth".into(),
            client: "vscode".into(),
            status: "idle".into(),
            last_message: String::new(),
            updated,
        }
    }
    #[test]
    fn distinguishes_ide_agents_without_binding_their_lifetime_to_the_whole_editor() {
        let processes = HashMap::from([
            (
                10,
                ParentProcess {
                    parent: 20,
                    birth: "shell".into(),
                    command: "/bin/sh".into(),
                },
            ),
            (
                20,
                ParentProcess {
                    parent: 30,
                    birth: "agent".into(),
                    command: "/usr/local/bin/claude".into(),
                },
            ),
            (
                30,
                ParentProcess {
                    parent: 1,
                    birth: "editor".into(),
                    command: "/Code Helper (Plugin)".into(),
                },
            ),
        ]);
        let owner = owner_from("claude", 10, |pid| Ok(processes.get(&pid).cloned())).unwrap();
        assert_eq!(owner, (20, "agent".into(), "vscode".into()));
        let owner = owner_from("cline", 30, |pid| Ok(processes.get(&pid).cloned())).unwrap();
        assert_eq!(owner, (30, "editor".into(), "vscode".into()));
        let owner = owner_from("claude", 20, |pid| {
            Ok((pid == 20).then(|| ParentProcess {
                parent: 1,
                birth: "agent".into(),
                command: "/usr/local/bin/claude".into(),
            }))
        })
        .unwrap();
        assert_eq!(owner.2, "terminal");
    }

    #[test]
    fn filters_dead_or_reused_processes_and_only_keeps_loaded_cline_task() {
        let records = vec![
            record("claude", "a", 1, 1),
            record("claude", "b", 1, 2),
            record("cline", "old", 2, 3),
            record("cline", "current", 2, 4),
            record("claude", "dead", 3, 5),
        ];
        let records = live(
            records,
            &HashMap::from([
                (1, "birth".into()),
                (2, "birth".into()),
                (3, "reused".into()),
            ]),
        );
        assert_eq!(
            records.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a", "b", "current"]
        );
    }
    #[test]
    fn private_snapshots_are_atomic_and_reject_path_traversal_and_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let mut r = record("claude", "one", 1, 1);
        save(root.path(), &r).unwrap();
        assert_eq!(load(root.path(), "claude", "one").unwrap().id, "one");
        r.id = "../other".into();
        assert!(save(root.path(), &r).is_err());
        let target = root.path().join("other.json");
        fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(target, root.path().join("claude/link.json")).unwrap();
        assert!(load(root.path(), "claude", "link").is_none());
    }
}
