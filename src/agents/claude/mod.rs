//! Claude Code lifecycle hooks and one-time PermissionRequest decisions.
use crate::{
    agents::{
        pending,
        sessions::{Approval, Session, SessionProvider},
        tracking::{self, Record},
    },
    app::core,
};
use anyhow::Result;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Claude;
pub fn home() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| core::home().join(".claude"))
}
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Notification",
    "Stop",
    "StopFailure",
];
fn approval(r: &pending::Request) -> Approval {
    Approval {
        can_allow: true,
        id: r.response.to_string_lossy().into_owned(),
        body: r.body.clone(),
        expires: r.expires,
    }
}
impl SessionProvider for Claude {
    fn id(&self) -> &'static str {
        "claude"
    }
    fn sessions(&self) -> Result<Vec<Session>> {
        let requests = pending::list_for("claude");
        Ok(tracking::open_records("claude")
            .into_iter()
            .map(|mut r| {
                enrich(&mut r);
                let mut session = tracking::snapshot(&r);
                session.approval = requests
                    .iter()
                    .filter(|request| request.session_id == r.id)
                    .max_by_key(|request| request.expires)
                    .map(approval);
                if session.approval.is_some() {
                    session.activity = crate::agents::sessions::Activity::Confirmation;
                }
                session
            })
            .collect())
    }
    fn pending_count(&self) -> usize {
        pending::list_for("claude").len()
    }
    fn open_target(&self, session: &Session) -> String {
        core::vscode_file_target(session.project_path())
    }
    fn allow(&self, session: &Session) -> bool {
        let Some(snapshot) = &session.approval else {
            return false;
        };
        let Some(request) = pending::list_for("claude")
            .into_iter()
            .find(|r| r.session_id == session.id && approval(r) == *snapshot)
        else {
            return false;
        };
        if !pending::allow(&request) {
            return false;
        }
        #[cfg(target_os = "macos")]
        crate::macos::native::dismiss_approval(&request.response);
        true
    }
}
pub fn enrich(record: &mut Record) {
    if let Some(path) = &record.transcript
        && let Some(tail) = tracking::tail(path)
    {
        parse(record, &tail);
    }
}
fn parse(record: &mut Record, input: &str) {
    for line in input.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["isSidechain"] == true {
            continue;
        }
        if v["type"] == "custom-title" {
            if let Some(title) = v["customTitle"]
                .as_str()
                .or_else(|| v["custom_title"].as_str())
            {
                record.title = core::shorten(title, 100);
            }
        } else if v["type"] == "assistant"
            && let Some(parts) = v["message"]["content"].as_array()
        {
            let text = parts
                .iter()
                .filter(|p| p["type"] == "text")
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                record.last_message = tracking::text(&text);
            }
        }
    }
}
pub fn hook(
    event: &Value,
    root: &Path,
    owner: (u32, String, String),
    notify: bool,
) -> Result<Option<Value>> {
    let kind = event["hook_event_name"].as_str().unwrap_or("");
    if !EVENTS.contains(&kind) {
        return Ok(None);
    }
    let id = event["session_id"].as_str().unwrap_or("");
    anyhow::ensure!(tracking::valid_id(id), "Invalid Claude session ID");
    if kind == "SessionEnd" {
        tracking::remove(root, "claude", id)?;
        return Ok(None);
    }
    let cwd = event["cwd"]
        .as_str()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| anyhow::anyhow!("Missing Claude project directory"))?;
    let mut r = tracking::load(root, "claude", id).unwrap_or(Record {
        agent: "claude".into(),
        id: id.into(),
        title: cwd
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        cwd: cwd.clone(),
        transcript: None,
        pid: owner.0,
        birth: owner.1.clone(),
        client: owner.2.clone(),
        status: "idle".into(),
        last_message: String::new(),
        updated: 0,
    });
    r.pid = owner.0;
    r.birth = owner.1;
    r.client = owner.2;
    // SessionStart pins the project; tool CWD changes do not relocate the window.
    if kind == "SessionStart" {
        r.cwd = cwd;
    }
    if let Some(path) = event["transcript_path"]
        .as_str()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        r.transcript = Some(path);
    }
    if kind == "UserPromptSubmit"
        && let Some(prompt) = event["prompt"].as_str()
        && r.title == r.cwd.file_name().unwrap_or_default().to_string_lossy()
    {
        r.title = core::shorten(prompt.lines().next().unwrap_or(prompt), 100);
    }
    r.status = match kind {
        "PermissionRequest" => "confirmation",
        "Stop" | "StopFailure" | "SessionStart" => "idle",
        "Notification" if event["notification_type"] == "permission_prompt" => "confirmation",
        "Notification" => "idle",
        _ => "working",
    }
    .into();
    enrich(&mut r);
    if let Some(message) = event["last_assistant_message"].as_str() {
        r.last_message = tracking::text(message);
    }
    r.updated = tracking::now();
    tracking::save(root, &r)?;
    if notify && kind == "PermissionRequest" {
        let tool = event["tool_name"].as_str().unwrap_or("tool");
        let body = event["tool_input"]["description"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(|s| core::shorten(s, 1000))
            .unwrap_or_else(|| format!("Allow {tool} in {}?", r.cwd.display()));
        let content = core::Notification {
            title: r.title.clone(),
            body,
            target: core::vscode_file_target(&r.cwd),
        };
        let mut request = event.clone();
        request["_agent"] = "claude".into();
        let action = core::deliver_content(&content, &request, true)?;
        if action == Some(core::Action::Allow) {
            r.status = "working".into();
            r.updated = tracking::now();
            tracking::save(root, &r)?;
        }
        return Ok(core::decision(action, false));
    }
    Ok(None)
}

pub fn install(directory: &Path, executable: &Path) -> Result<bool> {
    fs::create_dir_all(directory)?;
    let path = directory.join("settings.json");
    anyhow::ensure!(
        !path.is_symlink(),
        "Claude settings.json must not be a symlink"
    );
    let old = match fs::read(&path) {
        Ok(v) => Some(v),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let mut config: Value = old
        .as_deref()
        .map(serde_json::from_slice)
        .transpose()?
        .unwrap_or_else(|| serde_json::json!({}));
    anyhow::ensure!(
        config["disableAllHooks"] != true,
        "Claude hooks are disabled in settings; enable them before installing the integration"
    );
    let object = config
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Claude settings must be an object"))?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Claude hooks must be an object"))?;
    let command = format!(
        "{} agent-hook claude",
        shlex::try_quote(
            executable
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Non-UTF8 binary path"))?
        )?
    );
    for event in EVENTS {
        let groups = hooks
            .entry(*event)
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("Invalid {event} hooks"))?;
        let mut found = false;
        for group in groups.iter_mut() {
            if let Some(handlers) = group["hooks"].as_array_mut() {
                handlers.retain(|handler| {
                    if handler["command"] == command && handler["type"] == "command" {
                        if found {
                            false
                        } else {
                            found = true;
                            true
                        }
                    } else {
                        true
                    }
                });
            }
        }
        if !found {
            groups.push(serde_json::json!({"hooks":[{"type":"command","command":command,"timeout":if *event=="PermissionRequest" {70} else {10}}]}));
        }
    }
    let mut data = serde_json::to_vec_pretty(&config)?;
    data.push(b'\n');
    if old.as_deref() == Some(data.as_slice()) {
        return Ok(false);
    }
    if let Some(old) = old {
        use std::os::unix::fs::OpenOptionsExt;
        let mut backup = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join(format!("settings.json.backup-{}", tracking::now())))?;
        backup.write_all(&old)?;
    }
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    use std::io::Write;
    temp.write_all(&data)?;
    temp.persist(path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(kind: &str) -> Value {
        serde_json::json!({"session_id":"chat","hook_event_name":kind,"cwd":"/project","prompt":"Fix the parser"})
    }
    #[test]
    fn transcript_uses_renamed_title_and_assistant_text_without_tool_results() {
        let temp = tempfile::tempdir().unwrap();
        hook(
            &event("SessionStart"),
            temp.path(),
            (42, "birth".into(), "terminal".into()),
            false,
        )
        .unwrap();
        let mut record = tracking::load(temp.path(), "claude", "chat").unwrap();
        parse(
            &mut record,
            r#"{"type":"custom-title","customTitle":"Renamed chat"}
{"type":"assistant","message":{"content":[{"type":"text","text":"Latest answer"},{"type":"tool_use","name":"Bash"}]}}
{"type":"user","message":{"content":[{"type":"tool_result","content":"not assistant text"}]}}
{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"subagent answer"}]}}"#,
        );
        assert_eq!(record.title, "Renamed chat");
        assert_eq!(record.last_message, "Latest answer");
    }

    #[test]
    fn hook_tracks_open_idle_working_and_confirmation_without_granting_on_observation() {
        let temp = tempfile::tempdir().unwrap();
        let owner = || (42, "birth".into(), "terminal".into());
        for (kind, status) in [
            ("SessionStart", "idle"),
            ("UserPromptSubmit", "working"),
            ("PermissionRequest", "confirmation"),
            ("PostToolUse", "working"),
            ("Stop", "idle"),
        ] {
            assert!(
                hook(&event(kind), temp.path(), owner(), false)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                tracking::load(temp.path(), "claude", "chat")
                    .unwrap()
                    .status,
                status
            );
        }
        hook(&event("SessionEnd"), temp.path(), owner(), false).unwrap();
        assert!(tracking::load(temp.path(), "claude", "chat").is_none());
    }
    #[test]
    fn installation_preserves_other_hooks_and_permissions_and_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let p = temp.path().join("settings.json");
        fs::write(&p,r#"{"permissions":{"deny":["Bash(rm *)"]},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"existing"}]}]}}"#).unwrap();
        assert!(install(temp.path(), Path::new("/app monitor/binary")).unwrap());
        assert!(!install(temp.path(), Path::new("/app monitor/binary")).unwrap());
        let v: Value = serde_json::from_slice(&fs::read(p).unwrap()).unwrap();
        assert_eq!(v["permissions"]["deny"][0], "Bash(rm *)");
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "existing");
    }
}
