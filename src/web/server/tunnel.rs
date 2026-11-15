//! Managed Cloudflare and ngrok connectors sharing the authenticated remote ingress.
use super::{ContextState, Ingress, ingress_router};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashSet,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
};

#[derive(Clone)]
pub(super) struct Status {
    pub generation: u64,
    pub provider: &'static str,
    pub phase: &'static str,
    pub origin: Option<String>,
    pub error: Option<String>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            provider: "cloudflare",
            generation: 0,
            phase: "off",
            origin: None,
            error: None,
        }
    }
}

pub(super) struct QuickTunnel {
    pub(super) stop: Option<oneshot::Sender<()>>,
    pub(super) listener: JoinHandle<()>,
}
impl Drop for QuickTunnel {
    fn drop(&mut self) {
        self.listener.abort();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

pub(super) fn executable_named(name: &str) -> Result<PathBuf> {
    for directory in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/snap/bin",
    ] {
        let path = PathBuf::from(directory).join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths).map(|p| p.join(name)) {
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    bail!("{name} is not installed. Install {name} and try again.")
}

/// Only accept a single provider-issued HTTPS hostname, never a log-supplied URL
/// with a user name, port, query or arbitrary domain.
fn origin(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let candidate: String = line[start..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '|' | '\\'))
        .collect();
    let url = url::Url::parse(&candidate).ok()?;
    let host = url.host_str()?;
    let label = host.strip_suffix(".trycloudflare.com")?;
    if url.scheme() != "https"
        || label.is_empty()
        || label.len() > 63
        || !label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || label.starts_with('-')
        || label.ends_with('-')
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(format!("https://{host}"))
}

fn ngrok_origin(address: &str) -> Option<String> {
    let url = url::Url::parse(address).ok()?;
    let host = url.host_str()?;
    let suffix = [
        ".ngrok-free.app",
        ".ngrok-free.dev",
        ".ngrok.app",
        ".ngrok.dev",
        ".ngrok.io",
    ]
    .into_iter()
    .find_map(|suffix| host.strip_suffix(suffix))?;
    if suffix.is_empty()
        || suffix.len() > 63
        || suffix.starts_with('-')
        || suffix.ends_with('-')
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(format!("https://{host}"))
}

pub(super) fn validate_ngrok_token(token: &str) -> Result<()> {
    anyhow::ensure!(
        !token.is_empty() && token.len() <= 4096 && token.bytes().all(|b| b.is_ascii_graphic()),
        "Enter a valid ngrok authtoken without spaces."
    );
    Ok(())
}

// Unit/renderer tests never access the user's credential store.
#[cfg(not(test))]
fn saved_ngrok_token() -> Result<Option<String>> {
    crate::platform::credentials::load_named("ngrok-authtoken")
}
#[cfg(test)]
fn saved_ngrok_token() -> Result<Option<String>> {
    Ok(None)
}

// Also closes a connected SDK session when startup is canceled while its
// endpoint is still being provisioned. Dropping an SDK forwarder alone only
// detaches its JoinHandle, so forwarding tasks are aborted explicitly below.
struct SdkSession(Option<ngrok::Session>);
impl SdkSession {
    async fn close(&mut self) {
        if let Some(mut session) = self.0.take() {
            let _ = tokio::time::timeout(Duration::from_secs(3), session.close()).await;
        }
    }
}
impl Drop for SdkSession {
    fn drop(&mut self) {
        if let Some(mut session) = self.0.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(3), session.close()).await;
            });
        }
    }
}

impl QuickTunnel {
    pub async fn start(context: Arc<ContextState>) -> Result<Self> {
        Self::launch(context, executable_named("cloudflared")?).await
    }

    pub async fn start_ngrok(context: Arc<ContextState>, token: Option<String>) -> Result<Self> {
        use ngrok::prelude::*;
        let socket = TcpListener::bind("127.0.0.1:0")
            .await
            .context("Cannot bind private tunnel listener")?;
        let target = url::Url::parse(&format!("http://{}", socket.local_addr()?))?;
        let generation = context.generation.fetch_add(1, Ordering::Relaxed);
        *context.ngrok.lock().unwrap() = Status {
            provider: "ngrok",
            generation,
            phase: "starting",
            origin: None,
            error: None,
        };
        let app = ingress_router(context.clone(), Ingress::Ngrok);
        let listener = tokio::spawn(async move {
            let _ = axum::serve(socket, app).await;
        });
        let abort_listener = listener.abort_handle();
        let (stop, mut stopped) = oneshot::channel();
        tokio::spawn(async move {
            // Credential APIs may show a native unlock prompt. Keep them off the
            // runtime executor; no token is ever exposed in status or logs.
            let connect = async {
                let token = tokio::task::spawn_blocking(move || -> Result<String> {
                    if let Some(token) = token {
                        validate_ngrok_token(&token)?;
                        crate::platform::credentials::save_named("ngrok-authtoken", &token)?;
                        Ok(token)
                    } else {
                        saved_ngrok_token()?.ok_or_else(|| anyhow::anyhow!("ngrok_not_configured"))
                    }
                })
                .await
                .map_err(|_| anyhow::anyhow!("Cannot access saved ngrok token."))??;
                context.ngrok_configured.store(true, Ordering::Relaxed);
                let session = ngrok::Session::builder()
                    .authtoken(token)
                    .connect()
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!(
                            "Cannot connect to ngrok. Check your token, account and network."
                        )
                    })?;
                let mut session = SdkSession(Some(session));
                let endpoint = session
                    .0
                    .as_ref()
                    .expect("connected SDK session")
                    .http_endpoint()
                    .scheme(ngrok::config::Scheme::HTTPS)
                    .listen_and_forward(target)
                    .await;
                match endpoint {
                    Ok(forwarder) => Ok((session, forwarder)),
                    Err(_) => {
                        let _ = session.close().await;
                        anyhow::bail!(
                            "Cannot open ngrok endpoint. Check your account limits and network."
                        )
                    }
                }
            };
            let outcome = tokio::select! {
                _ = &mut stopped => { abort_listener.abort(); return; },
                outcome = tokio::time::timeout(Duration::from_secs(45), connect) => outcome,
            };
            let failure = match outcome {
                Ok(Ok((mut session, mut forwarder))) => {
                    if let Some(origin) = ngrok_origin(forwarder.url()) {
                        {
                            let mut current = context.ngrok.lock().unwrap();
                            if current.generation == generation {
                                current.origin = Some(origin);
                                current.phase = "connected";
                            }
                        }
                        let stopped_explicitly = tokio::select! {
                            _ = &mut stopped => true,
                            _ = forwarder.join() => false,
                        };
                        // Closing the session ends SDK reconnect attempts and all
                        // forwarding tasks. Bound shutdown even if the link died.
                        session.close().await;
                        forwarder.join().abort();
                        if stopped_explicitly {
                            abort_listener.abort();
                            return;
                        }
                        "ngrok connection stopped. Select ngrok to retry.".to_owned()
                    } else {
                        forwarder.join().abort();
                        session.close().await;
                        "ngrok returned an unsupported public address.".to_owned()
                    }
                }
                Ok(Err(error)) => {
                    // Credential errors are owned by our store; SDK errors above
                    // are replaced with static text to avoid exposing credentials.
                    error.to_string()
                }
                Err(_) => "ngrok connection timed out. Check your token and network.".to_owned(),
            };
            abort_listener.abort();
            context.retire_remote(
                Some(generation),
                Status {
                    provider: "ngrok",
                    generation,
                    phase: if failure == "ngrok_not_configured" {
                        "not_configured"
                    } else {
                        "failed"
                    },
                    origin: None,
                    error: (failure != "ngrok_not_configured").then_some(failure),
                },
            );
        });
        Ok(Self {
            stop: Some(stop),
            listener,
        })
    }

    async fn launch(context: Arc<ContextState>, binary: PathBuf) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .context("Cannot bind private tunnel listener")?;
        let target = format!("http://{}", listener.local_addr()?);
        // Isolate Quick Tunnel from the user's existing named-tunnel config.
        let config =
            tempfile::NamedTempFile::new().context("Cannot create tunnel configuration")?;
        std::fs::write(config.path(), "{}\n")?;
        let mut child = tokio::process::Command::new(binary)
            .args(["tunnel", "--config"])
            .arg(config.path())
            .args([
                "--no-autoupdate",
                "--protocol",
                "http2",
                "--url",
                &target,
                "--output",
                "json",
            ])
            .env_remove("TUNNEL_TOKEN")
            .env_remove("TUNNEL_TOKEN_FILE")
            .env_remove("TUNNEL_ORIGIN_CERT")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Cannot start cloudflared")?;
        let stderr = child
            .stderr
            .take()
            .context("Missing cloudflared status stream")?;
        let generation = context.generation.fetch_add(1, Ordering::Relaxed);
        *context.tunnel.lock().unwrap() = Status {
            provider: "cloudflare",
            generation,
            phase: "starting",
            origin: None,
            error: None,
        };
        let app = ingress_router(context.clone(), Ingress::Tunnel);
        let listener = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let abort_listener = listener.abort_handle();
        let (stop, mut stopped) = oneshot::channel();
        tokio::spawn(async move {
            let _config = config;
            // read_until is bounded to prevent a child from growing a line indefinitely.
            let mut input = BufReader::with_capacity(8192, stderr);
            let mut bytes = Vec::new();
            let mut registered = false;
            let mut address = None;
            let mut connections = HashSet::new();
            let startup = tokio::time::sleep(Duration::from_secs(45));
            tokio::pin!(startup);
            let failure = loop {
                tokio::select! {
                    _=&mut stopped=>{let _=child.kill().await;return;},
                    _=&mut startup,if !registered||address.is_none()=>break "Cloudflare connection timed out. Try again or check your VPN/network.",
                    line=read_status(&mut input,&mut bytes)=>{
                        match line {
                            Ok(0)|Err(_)=>break "Cloudflare tunnel stopped. Turn Remote access off and on to retry.",
                            Ok(_)=>{
                                if bytes.len()>65536 {break "Invalid cloudflared status output.";}
                                let text=String::from_utf8_lossy(&bytes);
                                if address.is_none(){address=origin(&text);}
                                let fields=serde_json::from_slice::<serde_json::Value>(&bytes).ok();
                                let index=fields.as_ref().and_then(|v|v["connIndex"].as_u64()).unwrap_or(0);
                                if text.contains("Registered tunnel connection"){registered=true;connections.insert(index);}
                                if text.contains("Unregistered tunnel connection") || text.contains("Connection terminated") || text.contains("Failed to serve tunnel connection"){connections.remove(&index);}
                                if registered && let Some(origin)=&address {
                                    let mut current=context.tunnel.lock().unwrap();
                                    if current.generation==generation {
                                        current.origin=Some(origin.clone());current.phase=if connections.is_empty(){"reconnecting"}else{"connected"};
                                    }
                                }
                                bytes.clear();
                            }
                        }
                    },
                }
            };
            abort_listener.abort();
            let _ = child.kill().await;
            context.retire_remote(
                Some(generation),
                Status {
                    provider: "cloudflare",
                    generation,
                    phase: "failed",
                    origin: None,
                    error: Some(failure.into()),
                },
            );
        });
        Ok(Self {
            stop: Some(stop),
            listener,
        })
    }
}

pub(super) async fn read_status<R: tokio::io::AsyncBufRead + Unpin>(
    input: &mut R,
    bytes: &mut Vec<u8>,
) -> std::io::Result<usize> {
    loop {
        let available = input.fill_buf().await?;
        if available.is_empty() {
            return Ok(0);
        }
        let end = available.iter().position(|b| *b == b'\n').map(|i| i + 1);
        let size = end.unwrap_or(available.len());
        if bytes.len() + size > 65536 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Status line too large",
            ));
        }
        bytes.extend_from_slice(&available[..size]);
        input.consume(size);
        if end.is_some() {
            return Ok(bytes.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ngrok_accepts_only_https_provider_origins() {
        for host in [
            "fixture.ngrok-free.app",
            "fixture.ngrok-free.dev",
            "fixture.ngrok.app",
            "fixture.ngrok.dev",
            "fixture.ngrok.io",
        ] {
            assert_eq!(
                ngrok_origin(&format!("https://{host}")),
                Some(format!("https://{host}"))
            );
        }
        for address in [
            "http://fixture.ngrok-free.app",
            "https://ngrok-free.app",
            "https://a.b.ngrok-free.app",
            "https://fixture.ngrok-free.app.evil.invalid",
            "https://user@fixture.ngrok-free.app",
            "https://fixture.ngrok-free.app:8123",
            "https://fixture.ngrok-free.app/path",
            "https://fixture.ngrok-free.app/?token=secret",
        ] {
            assert!(ngrok_origin(address).is_none(), "{address}");
        }
    }

    #[test]
    fn token_validation_and_debug_never_echo_credentials() {
        for token in ["", "token with spaces", "token\nline"] {
            assert!(validate_ngrok_token(token).is_err());
        }
        assert!(validate_ngrok_token("synthetic-account-token").is_ok());
        let command = super::super::Control::NgrokConfigure {
            token: "synthetic-private-token".into(),
        };
        assert!(!format!("{command:?}").contains("synthetic-private-token"));
    }

    #[test]
    fn only_exact_cloudflare_quick_origins_are_accepted_from_status_output() {
        assert_eq!(
            origin(r#"{"message":"https://four-random-words-here.trycloudflare.com"}"#).as_deref(),
            Some("https://four-random-words-here.trycloudflare.com")
        );
        for text in [
            "https://evil.invalid/",
            "https://trycloudflare.com/",
            "https://a.b.trycloudflare.com/",
            "https://a.trycloudflare.com.evil.invalid/",
            "https://user@a.trycloudflare.com/",
            "https://a.trycloudflare.com:123/",
            "https://a.trycloudflare.com/other",
            "https://a.trycloudflare.com/?secret=x",
            "https://a.trycloudflare.com/#x",
        ] {
            assert!(origin(text).is_none(), "{text}");
        }
    }

    #[tokio::test]
    async fn status_lines_are_bounded_before_allocation() {
        let data = vec![b'x'; 70000];
        let mut input = BufReader::new(&data[..]);
        let mut line = Vec::new();
        assert!(read_status(&mut input, &mut line).await.is_err());
        assert!(line.len() <= 65536);
    }

    #[tokio::test]
    async fn managed_connector_stops_its_own_child_and_stale_exit_cannot_revoke_a_new_tunnel() {
        use std::os::unix::fs::PermissionsExt;
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let (_published, state) =
            tokio::sync::watch::channel(Arc::new(crate::actors::monitor::State::default()));
        let monitor = crate::actors::monitor::Handle { tx, state };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (context, local) = super::super::start_server(listener, monitor);
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("connector");
        let pidfile = directory.path().join("pid");
        std::fs::write(&binary,format!("#!/bin/sh\necho $$ > '{}'\necho '{{\"message\":\"https://fixture.trycloudflare.com\"}}' >&2\necho '{{\"message\":\"Registered tunnel connection\",\"connIndex\":0}}' >&2\nexec /bin/sleep 60\n",pidfile.display())).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let connector = QuickTunnel::launch(context.clone(), binary).await.unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while context.tunnel.lock().unwrap().phase != "connected" {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid = std::fs::read_to_string(pidfile)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        let generation = context.tunnel.lock().unwrap().generation;
        drop(connector);
        context.close_remote();
        // SAFETY: signal 0 only probes our test-owned child; it sends no signal.
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "test connector was left running"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        *context.tunnel.lock().unwrap() = Status {
            provider: "cloudflare",
            generation: generation + 1,
            phase: "connected",
            origin: Some("https://new.trycloudflare.com".into()),
            error: None,
        };
        context.retire_remote(Some(generation), Status::default());
        assert_eq!(
            context.tunnel.lock().unwrap().origin.as_deref(),
            Some("https://new.trycloudflare.com")
        );
        local.abort();
    }
}
