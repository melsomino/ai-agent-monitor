//! Managed official Tuna CLI, isolated from user config and request inspection.
use super::{
    ContextState, Ingress, ingress_router,
    tunnel::{QuickTunnel, Status, executable_named, read_status},
};
use anyhow::{Context, Result};
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{io::BufReader, net::TcpListener, sync::oneshot};

pub(super) fn validate_key(key: &str) -> Result<()> {
    anyhow::ensure!(
        !key.is_empty() && key.len() <= 4096 && key.bytes().all(|b| b.is_ascii_graphic()),
        "Enter a valid Tuna API key without spaces."
    );
    Ok(())
}
#[cfg(not(test))]
fn saved_key() -> Result<Option<String>> {
    crate::platform::credentials::load_named("tuna-api-key")
}
#[cfg(test)]
fn saved_key() -> Result<Option<String>> {
    Ok(None)
}

pub(super) fn origin(address: &str) -> Option<String> {
    let url = url::Url::parse(address).ok()?;
    let host = url.host_str()?;
    let label = [
        ".a.2na.am",
        ".a.tuna.free",
        ".ru.tuna.am",
        ".nl.tuna.am",
        ".run.tuna.am",
    ]
    .into_iter()
    .find_map(|suffix| host.strip_suffix(suffix))?;
    if url.scheme() != "https"
        || label.is_empty()
        || label.len() > 63
        || label.starts_with('-')
        || label.ends_with('-')
        || !label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
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
fn published_origin(line: &[u8]) -> Option<String> {
    let event: serde_json::Value = serde_json::from_slice(line).ok()?;
    let level = event["level"].as_str().unwrap_or("").to_ascii_lowercase();
    if ["error", "fatal", "panic"].contains(&level.as_str()) {
        return None;
    }
    // Tuna's text/JSON logger announces the active endpoint as Forwarding.
    // Never accept an API/dashboard URL or an address inside an error message.
    let message = event["msg"]
        .as_str()
        .or_else(|| event["message"].as_str())
        .unwrap_or("");
    let announcement = message.to_ascii_lowercase();
    if announcement != "forwarding" && !announcement.starts_with("forwarding https://") {
        return None;
    }
    for field in [
        "public_url",
        "url",
        "from",
        "address",
        "forwarding",
        "msg",
        "message",
    ] {
        if let Some(text) = event[field].as_str() {
            for word in text.split(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '|' | '(' | ')' | ',')
            }) {
                if let Some(origin) = origin(word) {
                    return Some(origin);
                }
            }
        }
    }
    None
}

pub(super) async fn start(context: Arc<ContextState>, key: Option<String>) -> Result<QuickTunnel> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("Cannot bind private Tuna listener")?;
    let target = listener.local_addr()?.to_string();
    let generation = context.generation.fetch_add(1, Ordering::Relaxed);
    *context.tuna.lock().unwrap() = Status {
        provider: "tuna",
        generation,
        phase: "starting",
        origin: None,
        error: None,
    };
    let app = ingress_router(context.clone(), Ingress::Tuna);
    let listener = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let abort_listener = listener.abort_handle();
    let (stop, mut stopped) = oneshot::channel();
    tokio::spawn(async move {
        let setup = async {
            let key = tokio::task::spawn_blocking(move || -> Result<String> {
                if let Some(key) = key {
                    validate_key(&key)?;
                    crate::platform::credentials::save_named("tuna-api-key", &key)?;
                    Ok(key)
                } else {
                    saved_key()?.ok_or_else(|| anyhow::anyhow!("tuna_not_configured"))
                }
            })
            .await
            .map_err(|_| anyhow::anyhow!("Cannot access saved Tuna API key."))??;
            context.tuna_configured.store(true, Ordering::Relaxed);
            let binary=executable_named("tuna").map_err(|_|anyhow::anyhow!("Tuna CLI is not installed. Install it with: brew install yuccastream/tap/tuna (macOS), or follow tuna.am/docs/guides/install/install-cli."))?;
            launch(binary, &target, key)
        };
        let setup = tokio::select! {
            _=&mut stopped=>{abort_listener.abort();return;},
            result=tokio::time::timeout(Duration::from_secs(45),setup)=>result,
        };
        let failure = match setup {
            Ok(Ok((mut child, _directory))) => {
                let stdout = child.stdout.take().expect("piped Tuna output");
                let mut input = BufReader::new(stdout);
                let mut bytes = Vec::new();
                let mut connected = false;
                let deadline = tokio::time::sleep(Duration::from_secs(45));
                tokio::pin!(deadline);
                let failure = loop {
                    tokio::select! {
                        _=&mut stopped=>{let _=child.kill().await;abort_listener.abort();return;},
                        _=&mut deadline,if !connected=>break "Tuna connection timed out. Check your API key and network.".to_owned(),
                        line=read_status(&mut input,&mut bytes)=>match line {
                            Ok(0)|Err(_)=>break "Tuna connection stopped. Check your API key, account and network, then reconnect.".to_owned(),
                            Ok(_)=>{
                                if let Some(origin)=published_origin(&bytes){
                                    let previous=context.tuna.lock().unwrap().clone();
                                    if previous.generation==generation {
                                        let status=Status{provider:"tuna",generation,phase:"connected",origin:Some(origin),error:None};
                                        if previous.origin.is_some()&&previous.origin!=status.origin {context.retire_remote(Some(generation),status);}else{*context.tuna.lock().unwrap()=status;}
                                    }
                                    connected=true;
                                }
                                bytes.clear();
                            }
                        }
                    }
                };
                let _ = child.kill().await;
                failure
            }
            Ok(Err(error)) => error.to_string(),
            Err(_) => "Tuna setup timed out. Check access to your credential store.".to_owned(),
        };
        abort_listener.abort();
        context.retire_remote(
            Some(generation),
            Status {
                provider: "tuna",
                generation,
                phase: if failure == "tuna_not_configured" {
                    "not_configured"
                } else {
                    "failed"
                },
                origin: None,
                error: (failure != "tuna_not_configured").then_some(failure),
            },
        );
    });
    Ok(QuickTunnel {
        stop: Some(stop),
        listener,
    })
}

fn launch(
    binary: PathBuf,
    target: &str,
    key: String,
) -> Result<(tokio::process::Child, tempfile::TempDir)> {
    let directory = tempfile::tempdir().context("Cannot create Tuna configuration directory")?;
    let config = directory.path().join("tuna.yml");
    std::fs::write(&config, "version: \"1\"\n")?;
    let mut command = tokio::process::Command::new(binary);
    command.env_clear();
    for name in [
        "HOME",
        "PATH",
        "TMPDIR",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let child = command
        .current_dir(directory.path())
        .args(["http", target, "--config"])
        .arg(config)
        .args([
            "--inspect=false",
            "--log=stdout",
            "--log-format=json",
            "--no-colors",
        ])
        .env("TUNA_API_KEY", key)
        .env("TUNA_TUI", "0")
        .env("TUNA_NO_PROMPT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("Cannot start Tuna CLI")?;
    Ok((child, directory))
}

pub(super) fn installed() -> bool {
    executable_named("tuna").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_parser_rejects_untrusted_or_error_addresses() {
        for host in [
            "fixture.a.2na.am",
            "fixture.a.tuna.free",
            "fixture.ru.tuna.am",
            "fixture.nl.tuna.am",
            "fixture.run.tuna.am",
        ] {
            let line = serde_json::json!({"level":"info","msg":"Forwarding","public_url":format!("https://{host}"),"to":"http://127.0.0.1:8765"});
            assert_eq!(
                published_origin(line.to_string().as_bytes()),
                Some(format!("https://{host}"))
            );
        }
        for url in [
            "https://api.tuna.am",
            "https://a.2na.am",
            "https://a.tuna.free",
            "https://fixture.a.2na.am.evil.invalid",
            "https://fixture.a.tuna.free.evil.invalid",
            "https://nested.fixture.a.2na.am",
            "https://fixture.2na.am",
            "https://fixture.tuna.free",
            "http://fixture.a.2na.am",
            "https://user@fixture.a.2na.am",
            "https://fixture.a.2na.am/path",
            "https://fixture.a.2na.am/?token=synthetic",
            "https://my.tuna.am",
            "https://fixture.ru.tuna.am.evil.invalid",
            "http://fixture.ru.tuna.am",
            "https://user@fixture.ru.tuna.am",
            "https://fixture.ru.tuna.am/path",
            "https://fixture.ru.tuna.am/?token=secret",
            "https://nested.fixture.ru.tuna.am",
        ] {
            assert!(origin(url).is_none(), "{url}");
        }
        assert!(published_origin(br#"{"level":"error","msg":"Forwarding failed","from":"https://fixture.ru.tuna.am"}"#).is_none());
    }
    #[test]
    fn current_cli_forwarding_announcement_accepts_new_public_zones_but_not_account_messages() {
        assert_eq!(published_origin(br#"{"addr":"127.0.0.1:8765","level":"info","msg":"Forwarding","time":"2026-11-15T12:00:00Z","url":"https://synthetic-endpoint.a.2na.am"}"#).as_deref(),Some("https://synthetic-endpoint.a.2na.am"));
        assert_eq!(published_origin(br#"{"level":"info","msg":"Forwarding","url":"https://synthetic-endpoint.a.tuna.free"}"#).as_deref(),Some("https://synthetic-endpoint.a.tuna.free"));
        assert!(published_origin(br#"{"level":"info","msg":"Your permanent free domain: synthetic-endpoint.a.tuna.free. Use it with --subdomain=synthetic-endpoint"}"#).is_none());
        assert!(published_origin(br#"{"level":"error","msg":"Forwarding","url":"https://synthetic-endpoint.a.2na.am"}"#).is_none());
    }
    #[tokio::test]
    async fn launch_isolates_configuration_keeps_key_out_of_argv_and_disables_inspection() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = tempfile::tempdir().unwrap();
        let binary = fixture.path().join("tuna");
        std::fs::write(&binary,"#!/bin/sh\n[ \"$TUNA_API_KEY\" = 'synthetic-api-key' ] || exit 2\n[ \"$TUNA_TUI\" = '0' ] || exit 3\ncase \"$*\" in *synthetic-api-key*) exit 4;; esac\ncase \"$*\" in *--inspect=false*) ;; *) exit 5;; esac\necho '{\"level\":\"info\",\"msg\":\"Forwarding\",\"from\":\"https://fixture.ru.tuna.am\"}'\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (child, _directory) =
            launch(binary, "127.0.0.1:8765", "synthetic-api-key".into()).unwrap();
        let output = child.wait_with_output().await.unwrap();
        assert!(output.status.success());
        assert_eq!(
            published_origin(&output.stdout).as_deref(),
            Some("https://fixture.ru.tuna.am")
        );
    }
}
