//! Pending hook requests shared with the resident UI. No permission is granted by discovery.
use crate::app::core;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::{
    fs, io,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub agent: String,
    pub session_id: String,
    pub body: String,
    pub response: PathBuf,
    pub expires: u64,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn publish(directory: &Path, event: &Value, body: &str, lifetime: u64) -> io::Result<()> {
    let Some(id) = event["session_id"].as_str().filter(|id| !id.is_empty()) else {
        return Ok(());
    };
    let data = json!({"agent":event["_agent"].as_str().unwrap_or("codex"),"session_id":id,"body":body,"expires":now()+lifetime});
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join("request.json"))?
        .write_all(data.to_string().as_bytes())
}

pub fn list_for(agent: &str) -> Vec<Request> {
    list()
        .into_iter()
        .filter(|request| request.agent == agent)
        .collect()
}

pub fn list() -> Vec<Request> {
    read_directory(Path::new("/tmp"))
}
fn read_directory(root: &Path) -> Vec<Request> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| read_request(&entry.path()))
        .collect()
}
fn read_request(directory: &Path) -> Option<Request> {
    let name = directory.file_name()?.to_str()?;
    if !name.starts_with("codex-approval-") && !name.starts_with("agent-monitor-approval-") {
        return None;
    }
    let meta = fs::symlink_metadata(directory).ok()?;
    // SAFETY: geteuid has no preconditions.
    if !meta.is_dir() || meta.mode() & 0o777 != 0o700 || meta.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    let response = directory.join("response");
    if fs::symlink_metadata(&response).is_ok() {
        return None;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join("request.json"))
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        return None;
    }
    let mut data = String::new();
    file.take(65537).read_to_string(&mut data).ok()?;
    if data.len() > 65536 {
        return None;
    }
    let data: Value = serde_json::from_str(&data).ok()?;
    let expires = data["expires"].as_u64()?;
    if expires <= now() {
        return None;
    }
    let agent = data["agent"].as_str().unwrap_or("codex");
    if !matches!(agent, "codex" | "claude" | "cline") {
        return None;
    }
    Some(Request {
        agent: agent.into(),
        session_id: data["session_id"].as_str()?.to_owned(),
        body: data["body"].as_str()?.to_owned(),
        response,
        expires,
    })
}

pub fn allow(request: &Request) -> bool {
    // Re-read the exact request: stale UI and concurrent notification clicks cannot reuse it.
    request.response.parent().and_then(read_request).as_ref() == Some(request)
        && core::respond(&request.response, core::Action::Allow)
}

/// Stable across sender/UI processes and /tmp's macOS path aliases.
pub fn notification_id(response: &Path) -> Option<String> {
    if response.file_name()? != "response" {
        return None;
    }
    let directory = response.parent()?;
    if !matches!(directory.parent()?.to_str()?, "/tmp" | "/private/tmp") {
        return None;
    }
    let name = directory.file_name()?.to_str()?;
    name.strip_prefix("codex-approval-")
        .or_else(|| name.strip_prefix("agent-monitor-approval-"))
        .filter(|suffix| !suffix.is_empty())?;
    Some(format!("approval-{name}"))
}

pub fn is_resolved(response: &Path) -> bool {
    notification_id(response).is_some()
        && (response.exists()
            || response
                .parent()
                .is_none_or(|directory| !directory.exists()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_ids_match_path_aliases_and_isolate_requests() {
        let first = Path::new("/tmp/codex-approval-one/response");
        let alias = Path::new("/private/tmp/codex-approval-one/response");
        let second = Path::new("/tmp/codex-approval-two/response");
        assert_eq!(notification_id(first), notification_id(alias));
        assert_ne!(notification_id(first), notification_id(second));
        assert!(notification_id(Path::new("/tmp/other/response")).is_none());
        assert!(notification_id(Path::new("/tmp/codex-approval-one/request.json")).is_none());
    }

    #[test]
    fn late_notification_delivery_detects_answered_and_cleaned_up_requests() {
        let directory = core::approval_directory().unwrap();
        let response = directory.path().join("response");
        assert!(!is_resolved(&response));
        assert!(core::respond(&response, core::Action::Allow));
        assert!(is_resolved(&response));
        drop(directory);
        assert!(is_resolved(&response));
    }

    #[test]
    fn requests_expire_and_allow_is_single_use() {
        let directory = core::approval_directory().unwrap();
        publish(
            directory.path(),
            &json!({"session_id":"one"}),
            "Approve this",
            60,
        )
        .unwrap();
        let request = read_request(directory.path()).unwrap();
        assert_eq!(request.session_id, "one");
        assert!(allow(&request));
        assert!(!allow(&request));
        assert_eq!(fs::read_to_string(&request.response).unwrap(), "allow");
        assert!(read_request(directory.path()).is_none());
        let expired = core::approval_directory().unwrap();
        publish(expired.path(), &json!({"session_id":"one"}), "Expired", 0).unwrap();
        assert!(read_request(expired.path()).is_none());
    }
    #[test]
    fn changed_request_cannot_be_approved_from_stale_ui() {
        let directory = core::approval_directory().unwrap();
        publish(
            directory.path(),
            &json!({"session_id":"one"}),
            "Original",
            60,
        )
        .unwrap();
        let old = read_request(directory.path()).unwrap();
        let metadata = directory.path().join("request.json");
        let mut value: Value =
            serde_json::from_str(&fs::read_to_string(&metadata).unwrap()).unwrap();
        value["body"] = json!("Different request");
        fs::write(metadata, value.to_string()).unwrap();
        assert!(!allow(&old));
        assert!(!old.response.exists());
    }

    #[test]
    fn symlinked_metadata_is_ignored() {
        let directory = core::approval_directory().unwrap();
        let target = directory.path().join("target");
        fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(target, directory.path().join("request.json")).unwrap();
        assert!(read_request(directory.path()).is_none());
    }
}

#[cfg(test)]
mod agent_tests {
    use super::*;
    #[test]
    fn isolates_equal_session_ids_between_agents_and_accepts_legacy_codex_requests() {
        let codex = core::approval_directory().unwrap();
        let claude = core::approval_directory().unwrap();
        publish(codex.path(), &json!({"session_id":"same"}), "Codex", 60).unwrap();
        publish(
            claude.path(),
            &json!({"session_id":"same","_agent":"claude"}),
            "Claude",
            60,
        )
        .unwrap();
        let claude_path = claude.path().join("response");
        assert!(list_for("claude").iter().any(|r| r.response == claude_path));
        assert!(!list_for("codex").iter().any(|r| r.response == claude_path));
        let path = codex.path().join("request.json");
        let mut data: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        data.as_object_mut().unwrap().remove("agent");
        fs::write(&path, data.to_string()).unwrap();
        assert_eq!(read_request(codex.path()).unwrap().agent, "codex");
    }
}
