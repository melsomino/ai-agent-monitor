//! Local hook transport. Only the resident process executes integrations or posts notifications.
use crate::app::{core, monitor};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    cell::RefCell,
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
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 2 * 1024 * 1024;
pub type Owner = (u32, String, String);
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Sessions,
    Setup,
    NotificationSettings,
    Hook {
        event: Value,
        owner: Option<Owner>,
    },
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
thread_local! { static CONNECTION: RefCell<Option<UnixStream>> = const { RefCell::new(None) }; }

pub fn peer() -> Option<UnixStream> {
    CONNECTION.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|stream| stream.try_clone().ok())
    })
}
pub fn peer_alive(stream: &UnixStream) -> bool {
    let mut byte = 0u8;
    // SAFETY: One-byte buffer and a live owned descriptor; peeking consumes no input.
    let n = unsafe {
        libc::recv(
            stream.as_raw_fd(),
            (&mut byte as *mut u8).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    n > 0 || (n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock)
}
pub fn connected() -> bool {
    CONNECTION.with(|slot| slot.borrow().as_ref().is_none_or(peer_alive))
}

fn path() -> PathBuf {
    std::env::var_os("AI_AGENT_MONITOR_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            core::home().join("Library/Application Support/AI Agent Monitor/monitor.sock")
        })
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
    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: Valid descriptor and writable uid/gid output pointers.
    let status = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    anyhow::ensure!(
        status == 0 && uid == unsafe { libc::geteuid() },
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
            if std::env::var_os("AI_AGENT_MONITOR_SOCKET").is_none()
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
    reply.result.context("Missing monitor result")
}

/// Called only after the resident-process lock has been acquired.
pub fn start() -> Result<()> {
    let path = path();
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        anyhow::ensure!(
            metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::geteuid() },
            "Refusing to replace an untrusted monitor socket"
        );
        fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path).context("Create monitor socket")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    monitor::set_host();
    let active = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if active.fetch_add(1, Ordering::Relaxed) >= 64 {
                active.fetch_sub(1, Ordering::Relaxed);
                continue;
            }
            let active = active.clone();
            std::thread::spawn(move || {
                let _ = serve(stream);
                active.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });
    Ok(())
}
fn serve(mut stream: UnixStream) -> Result<()> {
    trusted_peer(&stream)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let request: Request = read_frame(&mut stream)?;
    CONNECTION.with(|slot| *slot.borrow_mut() = stream.try_clone().ok());
    let result = monitor::handle(request);
    CONNECTION.with(|slot| slot.borrow_mut().take());
    let reply = match result {
        Ok(result) => Reply {
            result: Some(result),
            error: None,
        },
        Err(error) => Reply {
            result: None,
            error: Some(format!("{error:#}")),
        },
    };
    write_frame(&mut stream, &reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framed_hook_round_trip_and_disconnect_detection() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let task = std::thread::spawn(move || {
            let request: Request = read_frame(&mut server).unwrap();
            assert!(matches!(request, Request::Hook { .. }));
            write_frame(
                &mut server,
                &Reply {
                    result: Some(serde_json::json!({"decision":"allow"})),
                    error: None,
                },
            )
            .unwrap();
        });
        assert_eq!(
            exchange(
                client,
                &Request::Hook {
                    event: serde_json::json!({"session_id":"one"}),
                    owner: None
                }
            )
            .unwrap()["decision"],
            "allow"
        );
        task.join().unwrap();
        let (client, server) = UnixStream::pair().unwrap();
        CONNECTION.with(|slot| *slot.borrow_mut() = Some(server));
        assert!(connected());
        drop(client);
        assert!(!connected());
        CONNECTION.with(|slot| slot.borrow_mut().take());
    }
    #[test]
    fn oversized_frames_and_error_replies_are_rejected() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client
            .write_all(&((MAX_FRAME + 1) as u32).to_le_bytes())
            .unwrap();
        assert!(read_frame::<Request>(&mut server).is_err());
        let (client, mut server) = UnixStream::pair().unwrap();
        let task = std::thread::spawn(move || {
            let _: Request = read_frame(&mut server).unwrap();
            write_frame(
                &mut server,
                &Reply {
                    result: None,
                    error: Some("request expired".into()),
                },
            )
            .unwrap();
        });
        assert!(
            exchange(
                client,
                &Request::Notice {
                    title: "Test".into(),
                    body: String::new(),
                    target: String::new()
                }
            )
            .is_err()
        );
        task.join().unwrap();
    }
}
