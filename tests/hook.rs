use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn removed_codex_hook_commands_are_rejected() {
    for args in [vec!["hook"], vec!["install"], vec!["check-hooks"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_AIAgentMonitor"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
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

#[cfg(target_os = "macos")]
#[test]
fn claude_hook_is_only_an_ipc_client_and_prints_the_monitors_reply() {
    use std::{
        fs,
        io::Read,
        os::unix::{fs::PermissionsExt, net::UnixListener},
    };
    let home = tempfile::Builder::new()
        .prefix("aim-ipc-test-")
        .tempdir_in("/tmp")
        .unwrap();
    let directory = home
        .path()
        .join("Library/Application Support/AI Agent Monitor");
    fs::create_dir_all(&directory).unwrap();
    let socket = directory.join("monitor.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let expected = serde_json::json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}});
    let reply = expected.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut header = [0; 4];
        stream.read_exact(&mut header).unwrap();
        let mut data = vec![0; u32::from_le_bytes(header) as usize];
        stream.read_exact(&mut data).unwrap();
        let request: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(request["type"], "agent_hook");
        assert_eq!(request["agent"], "claude");
        assert_eq!(request["input"]["session_id"], "ipc-test");
        let reply = serde_json::to_vec(&serde_json::json!({"result":reply,"error":null})).unwrap();
        stream
            .write_all(&(reply.len() as u32).to_le_bytes())
            .unwrap();
        stream.write_all(&reply).unwrap();
    });
    let mut hook = Command::new(env!("CARGO_BIN_EXE_AIAgentMonitor"))
        .args(["agent-hook", "claude"])
        .env("AI_AGENT_MONITOR_SOCKET", &socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    hook.stdin
        .take()
        .unwrap()
        .write_all(b"{\"hook_event_name\":\"PermissionRequest\",\"session_id\":\"ipc-test\"}")
        .unwrap();
    let output = hook.wait_with_output().unwrap();
    server.join().unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        expected
    );
    assert!(!home.path().join(".codex").exists());
    assert!(!home.path().join(".claude").exists());
}
