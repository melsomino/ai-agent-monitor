use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn malformed_and_unrelated_events_never_emit_permissions() {
    for event in ["invalid JSON", "{}", "[]", "{\"hook_event_name\":\"Stop\"}"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_AIAgentMonitor"))
            .arg("hook")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(event.as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(result.status.success());
        assert!(result.stdout.is_empty());
    }
}

#[test]
fn cli_help_and_errors_do_not_initialize_native_ui() {
    for args in [vec!["--help"], vec!["test", "--help"], vec!["--version"]] {
        let result = Command::new(env!("CARGO_BIN_EXE_AIAgentMonitor"))
            .args(args)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(!result.stdout.is_empty());
        assert!(result.stderr.is_empty());
    }
    let result = Command::new(env!("CARGO_BIN_EXE_AIAgentMonitor"))
        .args(["test", "--session-id"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("--session-id"));
}

#[test]
fn malformed_new_agent_hooks_never_grant_permissions_or_cancel_cline() {
    for agent in ["claude", "cline"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_AIAgentMonitor"))
            .args(["agent-hook", agent])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"not json").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        if agent == "cline" {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&stdout).unwrap(),
                serde_json::json!({"cancel":false})
            );
        } else {
            assert!(stdout.trim().is_empty());
        }
        assert!(!stdout.contains("allow"));
    }
}
