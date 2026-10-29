//! Cline lifecycle and selected-task UI metadata. Approval stays in Cline:
//! its public API has no request-addressed approval operation.
use crate::{
    agents::{
        sessions::{Activity, Approval, Session, SessionProvider},
        tracking::{self, Record},
    },
    app::core,
};
use anyhow::Result;
use serde_json::Value;
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub struct Cline;
pub const EVENTS: &[&str] = &[
    "TaskStart",
    "TaskResume",
    "TaskCancel",
    "TaskComplete",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "SessionShutdown",
];
pub fn hooks_home() -> PathBuf {
    core::home().join("Documents/Cline/Hooks")
}
fn task_files(record: &Record) -> Vec<PathBuf> {
    let home = core::home();
    [
        home.join(
            "Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/tasks",
        ),
        home.join(".cline/data/tasks"),
    ]
    .into_iter()
    .map(|p| p.join(&record.id).join("ui_messages.json"))
    .collect()
}
fn read_messages(path: &Path) -> Option<Value> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 8 * 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
fn permission_body(message: &Value) -> String {
    let text = message["text"].as_str().unwrap_or("");
    if message["ask"] == "tool"
        && let Ok(tool) = serde_json::from_str::<Value>(text)
    {
        let label = match tool["tool"].as_str().unwrap_or("") {
            "readFile" => "Read file",
            "editedExistingFile" => "Edit file",
            "newFile" => "Create file",
            "listFilesTopLevel" | "listFilesRecursive" => "List files",
            _ => "Use tool",
        };
        return format!(
            "{label}: {}",
            core::shorten(
                tool["path"]
                    .as_str()
                    .unwrap_or("Open Cline to review the request"),
                500
            )
        );
    }
    let label = match message["ask"].as_str() {
        Some("command") => "Run a command",
        Some("use_mcp_server") => "Use an MCP tool",
        Some("browser_action_launch") => "Open the browser",
        _ => "Review request",
    };
    format!("{label} — confirm in Cline")
}
pub fn parse(session: &mut Session, messages: &Value) {
    let Some(messages) = messages.as_array() else {
        return;
    };
    for message in messages {
        if message["type"] == "say"
            && matches!(message["say"].as_str(), Some("text" | "completion_result"))
            && let Some(text) = message["text"].as_str().filter(|s| !s.is_empty())
        {
            session.last_message = tracking::text(text);
        }
    }
    let Some(last) = messages.last() else {
        return;
    };
    // Cancel/complete lifecycle events take precedence over a stale persisted ask.
    if session.activity == Activity::Idle {
        return;
    }
    if last["type"] == "ask" && last["partial"] != true {
        match last["ask"].as_str() {
            Some("tool" | "command" | "use_mcp_server" | "browser_action_launch") => {
                session.activity = Activity::Confirmation;
                session.approval = Some(Approval {
                    id: format!("{}:{}", session.id, last["ts"]),
                    body: permission_body(last),
                    expires: u64::MAX,
                    can_allow: false,
                });
            }
            Some(
                "followup"
                | "plan_mode_respond"
                | "completion_result"
                | "resume_task"
                | "resume_completed_task"
                | "api_req_failed",
            ) => session.activity = Activity::Idle,
            _ => {}
        }
    } else if last["partial"] == true || last["say"] == "api_req_started" {
        session.activity = Activity::Working;
    } else if last["say"] == "completion_result" {
        session.activity = Activity::Idle;
    }
}
impl SessionProvider for Cline {
    fn id(&self) -> &'static str {
        "cline"
    }
    fn sessions(&self) -> Result<Vec<Session>> {
        Ok(tracking::open_records("cline")
            .iter()
            .map(|record| {
                let mut session = tracking::snapshot(record);
                let files = task_files(record);
                // Prefer the freshest existing storage copy, never a task-history scan.
                let mut files: Vec<_> = files
                    .into_iter()
                    .filter_map(|p| Some((fs::metadata(&p).ok()?.modified().ok()?, p)))
                    .collect();
                files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
                if let Some((_, path)) = files.first()
                    && let Some(messages) = read_messages(path)
                {
                    parse(&mut session, &messages);
                }
                session
            })
            .collect())
    }
    fn pending_count(&self) -> usize {
        self.sessions()
            .unwrap_or_default()
            .iter()
            .filter(|s| s.activity == Activity::Confirmation)
            .count()
    }
    fn open_target(&self, session: &Session) -> String {
        core::vscode_file_target(session.project_path())
    }
    fn allow(&self, _: &Session) -> bool {
        false
    }
}
pub fn hook(
    event: &Value,
    kind: Option<&str>,
    root: &Path,
    owner: (u32, String, String),
) -> Result<Value> {
    let kind = kind.or_else(|| event["hookName"].as_str()).unwrap_or("");
    if !EVENTS.contains(&kind) {
        return Ok(serde_json::json!({"cancel":false}));
    }
    let id = event["taskId"]
        .as_str()
        .or_else(|| event["taskStart"]["taskMetadata"]["taskId"].as_str())
        .or_else(|| event["taskResume"]["taskMetadata"]["taskId"].as_str())
        .unwrap_or("");
    anyhow::ensure!(tracking::valid_id(id), "Invalid Cline task ID");
    if kind == "SessionShutdown" {
        tracking::remove(root, "cline", id)?;
        return Ok(serde_json::json!({"cancel":false}));
    }
    let cwd = event["workspaceRoots"][0]
        .as_str()
        .or_else(|| event["cwd"].as_str())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| anyhow::anyhow!("Missing Cline workspace"))?;
    let mut r = tracking::load(root, "cline", id).unwrap_or(Record {
        agent: "cline".into(),
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
    if matches!(kind, "TaskStart" | "TaskResume") {
        r.cwd = cwd;
    }
    r.pid = owner.0;
    r.birth = owner.1;
    r.client = owner.2;
    if kind == "TaskStart"
        && let Some(title) = event["taskStart"]["taskMetadata"]["initialTask"].as_str()
    {
        r.title = core::shorten(title.lines().next().unwrap_or(title), 100);
    }
    r.status = if matches!(kind, "TaskCancel" | "TaskComplete") {
        "idle"
    } else {
        "working"
    }
    .into();
    r.updated = tracking::now();
    tracking::save(root, &r)?;
    Ok(serde_json::json!({"cancel":false}))
}
pub fn install(directory: &Path, executable: &Path) -> Result<bool> {
    let executable = shlex::try_quote(
        executable
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Non-UTF8 binary path"))?,
    )?;
    let scripts:Vec<_>=EVENTS.iter().map(|event|(directory.join(event),format!("#!/bin/sh\n# AI Agent Monitor Cline integration\nexec {executable} agent-hook cline --event {event}\n"))).collect();
    // Validate every destination before writing; never overwrite another user's hook.
    for (path, _) in &scripts {
        anyhow::ensure!(
            !path.is_symlink(),
            "Cline hook is a symlink: {}",
            path.display()
        );
        if path.exists() {
            anyhow::ensure!(
                fs::read_to_string(path)?
                    .starts_with("#!/bin/sh\n# AI Agent Monitor Cline integration\n"),
                "An existing Cline hook must be merged manually: {}",
                path.display()
            );
        }
    }
    fs::create_dir_all(directory)?;
    let mut changed = false;
    for (path, script) in scripts {
        if fs::read_to_string(&path).ok().as_deref() == Some(&script) {
            continue;
        }
        let mut temp = tempfile::NamedTempFile::new_in(directory)?;
        temp.write_all(script.as_bytes())?;
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o700))?;
        temp.persist(path)?;
        changed = true;
    }
    Ok(changed)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_keeps_workspace_independent_of_tool_cwd_and_removes_closed_task() {
        let root = tempfile::tempdir().unwrap();
        let event = serde_json::json!({"taskId":"one", "workspaceRoots":["/workspace"], "taskStart":{"taskMetadata":{"initialTask":"Fix parsing"}}});
        let owner = || (42, "birth".into(), "vscode".into());
        assert_eq!(
            hook(&event, Some("TaskStart"), root.path(), owner()).unwrap(),
            serde_json::json!({"cancel":false})
        );
        let tool =
            serde_json::json!({"taskId":"one", "workspaceRoots":["/other-command-directory"]});
        hook(&tool, Some("PreToolUse"), root.path(), owner()).unwrap();
        let record = tracking::load(root.path(), "cline", "one").unwrap();
        assert_eq!(record.cwd, Path::new("/workspace"));
        assert_eq!(record.title, "Fix parsing");
        hook(&event, Some("SessionShutdown"), root.path(), owner()).unwrap();
        assert!(tracking::load(root.path(), "cline", "one").is_none());
    }

    #[test]
    fn waiting_tool_is_visible_but_not_exposed_as_an_unsafe_generic_allow() {
        let record = Record {
            agent: "cline".into(),
            id: "one".into(),
            title: "Task".into(),
            cwd: "/workspace".into(),
            transcript: None,
            pid: 1,
            birth: "birth".into(),
            client: "vscode".into(),
            status: "working".into(),
            last_message: String::new(),
            updated: 0,
        };
        let mut session = tracking::snapshot(&record);
        parse(
            &mut session,
            &serde_json::json!([{"type":"say","say":"text","text":"Reading files"},{"type":"ask","ask":"tool","ts":123,"partial":false,"text":"{\"tool\":\"readFile\",\"path\":\"src/main.rs\"}"}]),
        );
        assert_eq!(session.activity, Activity::Confirmation);
        assert!(!session.approval.as_ref().unwrap().can_allow);
        assert!(!session.allow());
        assert!(
            session
                .approval
                .as_ref()
                .unwrap()
                .body
                .contains("src/main.rs")
        );
        session.activity = Activity::Idle;
        session.approval = None;
        parse(
            &mut session,
            &serde_json::json!([{"type":"ask","ask":"command","ts":124,"partial":false}]),
        );
        assert_eq!(session.activity, Activity::Idle);
        assert!(session.approval.is_none());
    }
    #[test]
    fn installer_is_idempotent_and_refuses_to_overwrite_other_hooks() {
        let root = tempfile::tempdir().unwrap();
        let executable = Path::new("/path with spaces/AIAgentMonitor");
        assert!(install(root.path(), executable).unwrap());
        assert!(!install(root.path(), executable).unwrap());
        fs::write(root.path().join("TaskStart"), "#!/bin/sh\necho original\n").unwrap();
        assert!(install(root.path(), executable).is_err());
        assert!(
            fs::read_to_string(root.path().join("TaskStart"))
                .unwrap()
                .contains("original")
        );
    }
}
