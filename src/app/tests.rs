use crate::app::core::{self, Action};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    time::Duration,
};

#[test]
fn title_uses_exact_session_and_renamed_title() {
    let home = tempfile::tempdir().unwrap();
    let db = Connection::open(home.path().join("state_5.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TABLE threads (id TEXT, name TEXT, title TEXT);
        INSERT INTO threads VALUES ('one', 'Новое название', 'Первый запрос');
        INSERT INTO threads VALUES ('two', 'Чужой чат', 'Другой запрос');",
    )
    .unwrap();
    assert_eq!(
        core::chat_title(&json!({"session_id":"one"}), home.path()),
        "Новое название"
    );
    assert_eq!(
        core::chat_title(&json!({"session_id":"unknown"}), home.path()),
        "Codex"
    );
    assert_eq!(core::chat_title(&json!({}), home.path()), "Codex");
    assert_eq!(
        core::chat_title(&json!({"session_id":"' OR 1=1 --"}), home.path()),
        "Codex"
    );
}

#[test]
fn title_supports_old_schema_missing_names_and_corrupt_databases() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("state_6.sqlite"), "corrupt").unwrap();
    let db = Connection::open(home.path().join("state_5.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TABLE threads (id TEXT, title TEXT); INSERT INTO threads VALUES ('one', '  Название
        чата  ');",
    )
    .unwrap();
    assert_eq!(
        core::chat_title(&json!({"session_id":"one"}), home.path()),
        "Название чата"
    );
}

#[test]
fn title_prefers_newest_database_version_not_lexicographic_order() {
    let home = tempfile::tempdir().unwrap();
    for (version, title) in [(9, "old"), (10, "current")] {
        let db = Connection::open(home.path().join(format!("state_{version}.sqlite"))).unwrap();
        db.execute_batch("CREATE TABLE threads (id TEXT, name TEXT, title TEXT)")
            .unwrap();
        db.execute("INSERT INTO threads VALUES ('one', '', ?1)", [title])
            .unwrap();
    }
    assert_eq!(
        core::chat_title(&json!({"session_id":"one"}), home.path()),
        "current"
    );
}

#[test]
fn unicode_text_is_shortened_on_word_boundaries() {
    let text = "Длинная причина ".repeat(30);
    let short = core::shorten(&text, 100);
    assert!(short.chars().count() <= 100);
    assert!(short.ends_with('…'));
    assert_eq!(core::shorten(" a\n b  ", 100), "a b");
}

#[test]
fn only_permission_events_produce_notifications_and_commands_are_not_shown() {
    for event in [
        json!({}),
        json!([]),
        json!({"hook_event_name":"PreToolUse"}),
    ] {
        assert!(core::notification(&event).is_none());
    }
    let note = core::notification(
        &json!({"hook_event_name":"PermissionRequest", "tool_input":{"command":"secret-token"}}),
    )
    .unwrap();
    assert!(!note.body.contains("secret-token"));
    let reason = "\" & do shell script \"touch /tmp/nope\"";
    let note = core::notification(
        &json!({"hook_event_name":"PermissionRequest", "tool_input":{"description":reason}}),
    )
    .unwrap();
    assert_eq!(note.body, reason);
    let long_reason = format!(
        "{}\n\nВторой абзац: подробное объяснение запроса.",
        "Длинная причина ".repeat(30)
    );
    let note = core::notification(
        &json!({"hook_event_name":"PermissionRequest", "tool_input":{"description":long_reason}}),
    )
    .unwrap();
    assert_eq!(note.body, long_reason);
}

fn metadata(storage: &Path, name: &str, data: Value) {
    let parent = storage.join(name);
    fs::create_dir_all(&parent).unwrap();
    fs::write(parent.join("workspace.json"), data.to_string()).unwrap();
}

fn target(path: &Path) -> String {
    format!(
        "vscode://file{}",
        url::Url::from_file_path(path)
            .unwrap()
            .as_str()
            .trim_start_matches("file://")
    )
}

#[test]
fn target_finds_workspace_parent_and_encodes_special_characters() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let project = root.join("my project #1");
    let child = project.join("src");
    fs::create_dir_all(&child).unwrap();
    let storage = root.join("storage");
    metadata(
        &storage,
        "one",
        json!({"folder":url::Url::from_file_path(&project).unwrap().as_str()}),
    );
    assert_eq!(
        core::vscode_target(&json!({"cwd":child}), &storage),
        target(&project)
    );
}

#[test]
fn target_finds_multi_root_jsonc_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let project = root.join("project");
    fs::create_dir(&project).unwrap();
    let workspace = root.join("dev.code-workspace");
    fs::write(
        &workspace,
        "{ // comment\n \"folders\": [{\"path\":\"project\"},], }",
    )
    .unwrap();
    let storage = root.join("storage");
    metadata(
        &storage,
        "one",
        json!({"workspace":url::Url::from_file_path(&workspace).unwrap().as_str()}),
    );
    assert_eq!(
        core::vscode_target(&json!({"cwd":project}), &storage),
        target(&workspace)
    );
}

#[test]
fn target_falls_back_to_cwd_and_ignores_remote_or_invalid_cache() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    metadata(
        &root,
        "remote",
        json!({"folder":"vscode-remote://ssh-remote+host/work"}),
    );
    metadata(&root, "bad", json!([]));
    assert_eq!(
        core::vscode_target(&json!({"cwd":root}), &root),
        target(&root)
    );
    assert_eq!(core::vscode_target(&json!({"cwd":"relative"}), &root), "");
    assert_eq!(core::vscode_target(&json!({}), &root), "");
}

#[test]
fn vscode_paths_decode_local_targets_and_distinguish_deleted_folders_from_invalid_urls() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("removed worktree #1 %");
    let value = target(&path);
    assert_eq!(core::vscode_path(&value), Some(path.clone()));
    assert!(!core::vscode_path(&value).unwrap().exists());
    fs::create_dir(&path).unwrap();
    assert!(core::vscode_path(&value).unwrap().is_dir());
    fs::remove_dir(&path).unwrap();
    assert!(!core::vscode_path(&value).unwrap().exists());
    for invalid in [
        "",
        "https://example.com",
        "vscode://remote/work",
        "vscode://file",
        "vscode://user@file/work",
        "vscode://file/work?command=example",
        "vscode://file/work#fragment",
    ] {
        assert!(core::vscode_path(invalid).is_none());
    }
}

#[test]
fn only_explicit_allow_emits_one_time_decision_and_tests_never_grant_access() {
    assert!(core::decision(None, false).is_none());
    assert!(core::decision(Some(Action::Open), false).is_none());
    assert!(core::decision(Some(Action::Allow), true).is_none());
    assert_eq!(
        core::decision(Some(Action::Allow), false).unwrap(),
        json!({"hookSpecificOutput":{
            "hookEventName":"PermissionRequest", "decision":{"behavior":"allow"}
        }})
    );
}

#[test]
fn response_is_single_use_isolated_and_expired_requests_cannot_be_approved() {
    let one = core::approval_directory().unwrap();
    let two = core::approval_directory().unwrap();
    let one_path = one.path().join("response");
    let two_path = two.path().join("response");
    assert!(core::respond(&one_path, Action::Allow));
    assert!(!core::respond(&one_path, Action::Open));
    assert_eq!(
        core::wait_response(&one_path, Duration::from_millis(10)),
        Some(Action::Allow)
    );
    assert_eq!(
        core::wait_response(&two_path, Duration::from_millis(1)),
        None
    );
    drop(one);
    assert!(!core::respond(&one_path, Action::Allow));
    assert!(!one_path.parent().unwrap().exists());
}

#[test]
fn response_rejects_symlinks_and_public_directories() {
    let temp = core::approval_directory().unwrap();
    let response = temp.path().join("response");
    let other = temp.path().join("other");
    fs::write(&other, "untouched").unwrap();
    symlink(&other, &response).unwrap();
    assert!(!core::respond(&response, Action::Allow));
    assert_eq!(fs::read_to_string(&other).unwrap(), "untouched");
    fs::remove_file(&response).unwrap();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(!core::respond(&response, Action::Allow));
}

#[test]
fn install_migrates_python_preserves_other_hooks_and_is_idempotent() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("hooks.json");
    let old = json!({"description":"Existing", "hooks":{
        "Stop":[{"hooks":[{"type":"command", "command":"true"}]}],
        "PermissionRequest":[{"hooks":[{"type":"command", "command":"python3 '/old project/notify.py' --approval", "timeout":70}]}]
    }});
    let before = old.to_string();
    fs::write(&path, &before).unwrap();
    let executable = Path::new("/new project/AIAgentMonitor");
    assert!(core::install(home.path(), executable, Path::new("/old project/notify.py")).unwrap());
    let once = fs::read(&path).unwrap();
    assert!(!core::install(home.path(), executable, Path::new("/old project/notify.py")).unwrap());
    assert_eq!(fs::read(&path).unwrap(), once);
    let config: Value = serde_json::from_slice(&once).unwrap();
    assert_eq!(config["hooks"]["Stop"], old["hooks"]["Stop"]);
    let groups = config["hooks"]["PermissionRequest"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    let args = shlex::split(groups[0]["hooks"][0]["command"].as_str().unwrap()).unwrap();
    assert_eq!(args, vec!["/new project/AIAgentMonitor", "hook"]);
    let backup = fs::read_dir(home.path())
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("hooks.json.backup-")
        })
        .unwrap();
    assert_eq!(fs::read_to_string(backup.path()).unwrap(), before);
}

#[test]
fn install_refuses_invalid_or_symlinked_config() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("hooks.json");
    for invalid in ["invalid JSON", "[]", "{\"hooks\":[]}"] {
        fs::write(&path, invalid).unwrap();
        assert!(core::install(home.path(), Path::new("/new"), Path::new("/old")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
    }
    fs::remove_file(&path).unwrap();
    symlink(home.path().join("missing"), &path).unwrap();
    assert!(core::install(home.path(), Path::new("/new"), Path::new("/old")).is_err());
}

#[test]
fn install_migrates_the_old_app_path_without_replacing_unrelated_codex_hooks() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("hooks.json");
    let executable = Path::new("/project/build/AI Agent Monitor.app/Contents/MacOS/AIAgentMonitor");
    let unrelated = "'/other/Codex Notify.app/Contents/MacOS/CodexNotify' hook";
    fs::write(&path, json!({"hooks":{"PermissionRequest":[{"hooks":[
        {"type":"command","command":"'/project/build/Codex Notify.app/Contents/MacOS/CodexNotify' hook"},
        {"type":"command","command":unrelated}
    ]}]}}).to_string()).unwrap();
    assert!(core::install(home.path(), executable, Path::new("/project/notify.py")).unwrap());
    let data = fs::read(&path).unwrap();
    let config: Value = serde_json::from_slice(&data).unwrap();
    let handlers = config["hooks"]["PermissionRequest"][0]["hooks"]
        .as_array()
        .unwrap();
    assert_eq!(handlers.len(), 2);
    assert_eq!(
        shlex::split(handlers[0]["command"].as_str().unwrap()).unwrap(),
        vec![executable.to_str().unwrap(), "hook"]
    );
    assert_eq!(handlers[1]["command"], unrelated);
    assert!(!core::install(home.path(), executable, Path::new("/project/notify.py")).unwrap());
    assert_eq!(fs::read(path).unwrap(), data);
}
