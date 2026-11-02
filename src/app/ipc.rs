//! Local hook transport. Only the resident process executes integrations or posts notifications.
use crate::{actors::monitor, app::core};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 2 * 1024 * 1024;
pub type Owner = (u32, String, String);
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Sessions,
    ShowWindow,
    Setup,
    NotificationSettings,
    AgentHook {
        agent: String,
        event: Option<String>,
        input: Value,
        owner: Owner,
    },
    Test {
        approval: bool,
        session_id: Option<String>,
    },
    Notice {
        title: String,
        body: String,
        target: String,
    },
}
#[derive(Serialize, Deserialize)]
struct Reply {
    result: Option<Value>,
    error: Option<String>,
}
fn path() -> PathBuf {
    std::env::var_os("AI_AGENT_MONITOR_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| core::state_home().join("monitor.sock"))
}
fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    anyhow::ensure!(bytes.len() <= MAX_FRAME, "Monitor message is too large");
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(&bytes)?;
    Ok(())
}
fn read_frame<T: DeserializeOwned>(stream: &mut UnixStream) -> Result<T> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header)?;
    let size = u32::from_le_bytes(header) as usize;
    anyhow::ensure!(size <= MAX_FRAME, "Monitor message is too large");
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).context("Invalid monitor message")
}
fn trusted_peer(stream: &UnixStream) -> Result<()> {
    trusted_fd(stream.as_raw_fd())?;
    Ok(())
}
fn trusted_fd(fd: std::os::fd::RawFd) -> Result<()> {
    #[cfg(target_os = "linux")]
    let uid = {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: valid descriptor, correctly sized credential buffer and length.
        anyhow::ensure!(
            unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    &mut cred as *mut _ as *mut libc::c_void,
                    &mut size,
                )
            } == 0,
            "Cannot authenticate monitor peer"
        );
        cred.uid
    };
    #[cfg(not(target_os = "linux"))]
    let uid = {
        let mut uid = 0;
        let mut gid = 0;
        anyhow::ensure!(
            unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } == 0,
            "Cannot authenticate monitor peer"
        );
        uid
    };
    anyhow::ensure!(
        uid == unsafe { libc::geteuid() },
        "Monitor peer belongs to another user"
    );
    Ok(())
}
fn connect(path: &Path) -> Result<UnixStream> {
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.file_type().is_socket()
            && metadata.mode() & 0o077 == 0
            && metadata.uid() == unsafe { libc::geteuid() },
        "Untrusted monitor socket"
    );
    let stream = UnixStream::connect(path)?;
    trusted_peer(&stream)?;
    Ok(stream)
}

pub fn request(request: &Request) -> Result<Value> {
    let path = path();
    let stream = match connect(&path) {
        Ok(stream) => stream,
        Err(error)
            if cfg!(target_os = "macos")
                && std::env::var_os("AI_AGENT_MONITOR_SOCKET").is_none()
                && error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                    matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    )
                }) =>
        {
            let executable = core::bundle_executable();
            let bundle = executable
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .context("Missing app bundle")?;
            let status = Command::new("/usr/bin/open")
                .args(["-g"])
                .arg(bundle)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()?;
            anyhow::ensure!(status.success(), "Could not start AI Agent Monitor");
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match connect(&path) {
                    Ok(stream) => break stream,
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50))
                    }
                    Err(error) => {
                        return Err(error).context(
                            "AI Agent Monitor did not start; open the installed app first",
                        );
                    }
                }
            }
        }
        Err(error) => return Err(error),
    };
    exchange(stream, request)
}
fn exchange(mut stream: UnixStream, request: &Request) -> Result<Value> {
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.set_read_timeout(Some(Duration::from_secs(70)))?;
    write_frame(&mut stream, request)?;
    let reply: Reply = read_frame(&mut stream)?;
    if let Some(error) = reply.error {
        bail!("{error}");
    }
    // JSON null is a successful result for passive notifications and hooks
    // that return control without a decision. Option<Value> decodes it as None.
    Ok(reply.result.unwrap_or(Value::Null))
}

/// Called only after the resident-process lock has been acquired.
pub fn start(
    handle: monitor::Handle,
    runtime: &tokio::runtime::Handle,
) -> Result<tokio::task::JoinHandle<()>> {
    let socket_path = path();
    if let Ok(metadata) = fs::symlink_metadata(&socket_path) {
        anyhow::ensure!(
            metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::geteuid() },
            "Untrusted monitor socket"
        );
        fs::remove_file(&socket_path)?;
    }
    let listener = UnixListener::bind(path())?;
    fs::set_permissions(path(), fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let _entered = runtime.enter();
    let listener = tokio::net::UnixListener::from_std(listener)?;
    Ok(runtime.spawn(async move {
        let capacity = std::sync::Arc::new(tokio::sync::Semaphore::new(64));
        while let Ok((stream, _)) = listener.accept().await {
            let Ok(permit) = capacity.clone().try_acquire_owned() else {
                continue;
            };
            let handle = handle.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let _ = serve(stream, handle).await;
            });
        }
    }))
}
async fn serve(mut stream: tokio::net::UnixStream, monitor: monitor::Handle) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    trusted_fd(stream.as_raw_fd())?;
    let request = tokio::time::timeout(Duration::from_secs(3), async {
        let length = stream.read_u32_le().await? as usize;
        anyhow::ensure!(length <= MAX_FRAME, "Oversized monitor request");
        let mut data = vec![0; length];
        stream.read_exact(&mut data).await?;
        Ok::<Request, anyhow::Error>(serde_json::from_slice(&data)?)
    })
    .await??;
    let mut hook = None;
    let result = handle_request(request, &monitor, &mut stream, &mut hook).await;
    let reply = match result {
        Ok(value) => Reply {
            result: Some(value),
            error: None,
        },
        Err(e) => Reply {
            result: None,
            error: Some(e.to_string()),
        },
    };
    let data = serde_json::to_vec(&reply)?;
    let written = tokio::time::timeout(Duration::from_secs(3), async {
        stream.write_u32_le(data.len() as u32).await?;
        stream.write_all(&data).await
    })
    .await;
    if let Some(key) = hook {
        let _ = monitor.enqueue(monitor::Command::HookGone(key)).await;
    }
    written??;
    Ok(())
}
async fn ask(
    content: core::Notification,
    event: Value,
    owner: Option<Owner>,
    approval: bool,
    monitor: &monitor::Handle,
    stream: &mut tokio::net::UnixStream,
    hook: &mut Option<String>,
) -> Result<Option<core::Action>> {
    use tokio::io::AsyncReadExt;
    if !approval {
        monitor
            .enqueue(monitor::Command::Passive(content))
            .await
            .map_err(anyhow::Error::msg)?;
        return Ok(None);
    }
    let key = format!("hook:{}:{}", std::process::id(), crate::app::ids::nonce());
    let session = monitor::hook_session(&content, &event, owner.as_ref(), &key);
    let mut state = monitor.subscribe();
    monitor
        .enqueue(monitor::Command::Hook {
            key: key.clone(),
            session: Box::new(session),
            target: content.target,
        })
        .await
        .map_err(anyhow::Error::msg)?;
    *hook = Some(key.clone());
    let mut registered = false;
    let timeout = tokio::time::sleep(core::WAIT);
    tokio::pin!(timeout);
    loop {
        let snapshot = state.borrow_and_update().clone();
        if let Some(h) = snapshot.hooks.get(&key) {
            registered = true;
            if let Some(answer) = &h.decision {
                return Ok(match answer {
                    crate::agents::interaction::Response::Action(id) if id == "allow" => {
                        Some(core::Action::Allow)
                    }
                    _ => None,
                });
            }
        } else if registered || snapshot.stopping {
            return Ok(None);
        }
        let mut byte = [0];
        tokio::select! {
            changed=state.changed()=>if changed.is_err(){return Ok(None);},
            _=&mut timeout=>return Ok(None),
            _=stream.read(&mut byte)=>return Ok(None),
        }
    }
}
async fn handle_request(
    request: Request,
    monitor: &monitor::Handle,
    stream: &mut tokio::net::UnixStream,
    hook: &mut Option<String>,
) -> Result<Value> {
    use serde_json::json;
    match request {
        Request::ShowWindow => {
            monitor
                .enqueue(monitor::Command::ShowWindow)
                .await
                .map_err(anyhow::Error::msg)?;
            Ok(Value::Null)
        }
        Request::Sessions => {
            let state = monitor.snapshot();
            let telegram = crate::actors::human::telegram::diagnostics(&state.telegram);
            Ok(
                json!({"monitor":{"pid":std::process::id(),"version":core::VERSION},"telegram":telegram,"sessions":state.sessions.iter().map(|s|json!({"agent":s.agent,"id":s.id,"title":s.title,"cwd":s.cwd,"workspace":s.workspace,"gitBranch":s.git_branch,"client":s.client.label(),"status":s.activity.label(),"lastMessage":s.last_message,"updatedAt":s.updated_at,"approval":s.approval.as_ref().map(|a|json!({"id":a.id,"body":a.body,"expiresAt":a.expires,"canAllow":a.can_allow}))})).collect::<Vec<_>>()}),
            )
        }
        Request::Test {
            approval,
            session_id,
        } => {
            let mut event = serde_json::json!({"cwd":core::home(),"_agent":"codex"});
            if let Some(id) = session_id {
                event["session_id"] = id.into();
            }
            let title = if event["session_id"].is_string() {
                core::chat_title(&event, &core::codex_home())
            } else {
                "AI Agent Monitor · Test".into()
            };
            let content = core::Notification {
                title,
                body: "Click Allow to test the response path. No agent command will be approved."
                    .into(),
                target: String::new(),
            };
            Ok(
                json!({"test":true,"allowed":ask(content,event,None,approval,monitor,stream,hook).await?==Some(core::Action::Allow)}),
            )
        }
        Request::Notice {
            title,
            body,
            target,
        } => {
            monitor
                .enqueue(monitor::Command::Passive(core::Notification {
                    title,
                    body,
                    target,
                }))
                .await
                .map_err(anyhow::Error::msg)?;
            Ok(Value::Null)
        }
        Request::AgentHook {
            agent,
            event,
            input,
            owner,
        } => {
            let root = crate::agents::tracking::root();
            let data = input.clone();
            let agent_kind = agent.clone();
            let source = owner.clone();
            let result = tokio::task::spawn_blocking(move || match agent_kind.as_str() {
                "claude" => crate::agents::claude::hook(&data, &root, source, false)
                    .map(|v| v.unwrap_or(Value::Null)),
                "cline" => crate::agents::cline::hook(&data, event.as_deref(), &root, source),
                _ => anyhow::bail!("Unknown agent"),
            })
            .await??;
            if agent == "claude" {
                if let Some(id) = input["session_id"].as_str()
                    && matches!(
                        input["hook_event_name"].as_str(),
                        Some(
                            "SessionEnd"
                                | "Stop"
                                | "StopFailure"
                                | "PostToolUse"
                                | "PostToolUseFailure"
                                | "UserPromptSubmit"
                        )
                    )
                {
                    let _ = monitor
                        .enqueue(monitor::Command::CancelSession {
                            agent: agent.clone(),
                            id: id.into(),
                        })
                        .await;
                }
                if input["hook_event_name"] == "PermissionRequest" {
                    let mut input = input;
                    input["_agent"] = "claude".into();
                    let id = input["session_id"].as_str().unwrap_or_default();
                    let record = crate::agents::tracking::load(
                        &crate::agents::tracking::root(),
                        "claude",
                        id,
                    );
                    let title = record
                        .as_ref()
                        .map(|r| r.title.clone())
                        .unwrap_or_else(|| "Claude Code".into());
                    let body = input["tool_input"]["description"]
                        .as_str()
                        .map(|s| core::shorten(s, 1000))
                        .unwrap_or_else(|| {
                            format!("Allow {}?", input["tool_name"].as_str().unwrap_or("tool"))
                        });
                    let content = core::Notification {
                        title,
                        body,
                        target: record
                            .map(|r| core::vscode_file_target(&r.cwd))
                            .unwrap_or_default(),
                    };
                    return Ok(core::decision(
                        ask(content, input, Some(owner), true, monitor, stream, hook).await?,
                        false,
                    )
                    .unwrap_or(Value::Null));
                }
            }
            let _ = monitor.send(monitor::Command::Refresh);
            Ok(result)
        }
        Request::Setup => {
            #[cfg(target_os = "macos")]
            {
                Ok(
                    json!({"allowed":tokio::task::spawn_blocking(crate::macos::native::setup_notifications).await??}),
                )
            }
            #[cfg(not(target_os = "macos"))]
            {
                anyhow::bail!("Notifications require macOS")
            }
        }
        Request::NotificationSettings => {
            #[cfg(target_os = "macos")]
            {
                tokio::task::spawn_blocking(crate::macos::native::notification_settings).await?
            }
            #[cfg(not(target_os = "macos"))]
            {
                anyhow::bail!("Notifications require macOS")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_socket_peer_is_authenticated_as_current_user() {
        let (left, right) = std::os::unix::net::UnixStream::pair().unwrap();
        super::trusted_peer(&left).unwrap();
        super::trusted_peer(&right).unwrap();
        assert!(super::trusted_fd(-1).is_err());
    }

    use super::*;
    #[test]
    fn null_replies_are_successful_but_server_errors_still_fail() {
        for error in [None, Some("Rejected".to_owned())] {
            let (client, mut server) = UnixStream::pair().unwrap();
            let failed = error.is_some();
            let task = std::thread::spawn(move || {
                let _: Request = read_frame(&mut server).unwrap();
                write_frame(
                    &mut server,
                    &Reply {
                        result: Some(Value::Null),
                        error,
                    },
                )
                .unwrap();
            });
            let result = exchange(client, &Request::Sessions);
            if failed {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), Value::Null);
            }
            task.join().unwrap();
        }
    }
}
