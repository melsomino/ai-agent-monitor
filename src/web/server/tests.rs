use super::*;
use crate::domain::{
    interaction::Interaction,
    session::{Activity, Approval, Client, ConnectionState},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct Http {
    status: u16,
    headers: String,
    body: Vec<u8>,
}
async fn http(
    address: SocketAddr,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    host: Option<&str>,
    body: Option<Value>,
) -> Http {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = body.map(|v| v.to_string()).unwrap_or_default();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        host.map(str::to_owned)
            .unwrap_or_else(|| address.to_string()),
        body.len()
    );
    if let Some(cookie) = cookie {
        request.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    if let Some(origin) = origin {
        request.push_str(&format!("Origin: {origin}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(&body);
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    let split = bytes.windows(4).position(|s| s == b"\r\n\r\n").unwrap();
    let headers = String::from_utf8_lossy(&bytes[..split]).to_string();
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    Http {
        status,
        headers,
        body: bytes[split + 4..].to_vec(),
    }
}
fn fixture() -> Session {
    let mut s = Session {
        agent: "codex",
        id: "web-test".into(),
        title: "Phone test".into(),
        cwd: "/demo/project".into(),
        workspace: None,
        git_branch: Some("demo".into()),
        model: None,
        model_display_name: None,
        reasoning_effort: None,
        updated_at: 0,
        client: Client::Terminal,
        activity: Activity::Confirmation,
        target: None,
        connection: ConnectionState::live(true),
        activities: Default::default(),
    };
    s.seed_snapshot(
        "**Ready**".into(),
        vec![],
        None,
        Some(Approval {
            id: "request-one".into(),
            body: "Run checks?".into(),
            expires: monitor::now() + 60,
            can_allow: true,
        }),
        Interaction::allow(),
    );
    s
}
async fn setup() -> (
    Controller,
    SocketAddr,
    String,
    String,
    tokio::sync::mpsc::Receiver<Command>,
    tokio::sync::watch::Sender<Arc<monitor::State>>,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let state = monitor::State {
        sessions: vec![fixture()],
        ..Default::default()
    };
    let (published, state) = tokio::sync::watch::channel(Arc::new(state));
    let monitor = monitor::Handle { tx, state };
    let controller = Controller::new(monitor);
    let report = controller
        .control(Control::Start {
            listen: "127.0.0.1:0".parse().unwrap(),
        })
        .await
        .unwrap();
    let address = report["listen"].as_str().unwrap().parse().unwrap();
    let token = report["pairingLinks"][0]
        .as_str()
        .unwrap()
        .split("#pair=")
        .nth(1)
        .unwrap()
        .to_owned();
    let origin = format!("http://{address}");
    let paired = http(
        address,
        "POST",
        "/api/pair",
        None,
        Some(&origin),
        None,
        Some(json!({"token":token,"name":"Test browser"})),
    )
    .await;
    assert_eq!(paired.status, 200);
    let cookie = paired
        .headers
        .lines()
        .find_map(|line| {
            line.to_lowercase().starts_with("set-cookie:").then(|| {
                line.split_once(": ")
                    .unwrap()
                    .1
                    .split(';')
                    .next()
                    .unwrap()
                    .to_owned()
            })
        })
        .unwrap();
    (controller, address, cookie, token, rx, published)
}
#[tokio::test]
async fn http_pairing_is_required_one_use_and_revocable() {
    let (controller, address, cookie, token, _rx, _published) = setup().await;
    let origin = format!("http://{address}");
    assert_eq!(
        http(address, "GET", "/api/snapshot", None, None, None, None)
            .await
            .status,
        401
    );
    let replay = http(
        address,
        "POST",
        "/api/pair",
        None,
        Some(&origin),
        None,
        Some(json!({"token":token,"name":"Other"})),
    )
    .await;
    assert_eq!(replay.status, 401);
    assert_eq!(
        http(
            address,
            "POST",
            "/api/command",
            Some(&cookie),
            Some("http://evil.invalid"),
            None,
            Some(json!({"type":"open","session":"codex:web-test"}))
        )
        .await
        .status,
        403
    );
    assert_eq!(
        http(
            address,
            "GET",
            "/api/snapshot",
            Some(&cookie),
            None,
            Some("evil.invalid"),
            None
        )
        .await
        .status,
        403
    );
    let response = http(
        address,
        "GET",
        "/api/snapshot",
        Some(&cookie),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(response.status, 200);
    let value: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(value["sessions"][0]["title"], "Phone test");
    assert!(
        !response
            .body
            .windows(token.len())
            .any(|w| w == token.as_bytes())
    );
    let page = http(address, "GET", "/", None, None, None, None).await;
    assert_eq!(page.status, 200);
    assert!(String::from_utf8_lossy(&page.body).contains("remote.js"));
    assert!(!String::from_utf8_lossy(&page.body).contains("postMessage"));
    assert!(page.headers.to_lowercase().contains("no-store"));
    assert_eq!(
        http(address, "GET", "/../../Cargo.toml", None, None, None, None)
            .await
            .status,
        404
    );
    let status = controller.control(Control::Status).await.unwrap();
    assert!(status["pairingLinks"].as_array().unwrap().is_empty());
    let id = status["devices"][0]["id"].as_u64().unwrap();
    controller.control(Control::Revoke { id }).await.unwrap();
    assert_eq!(
        http(
            address,
            "GET",
            "/api/snapshot",
            Some(&cookie),
            None,
            None,
            None
        )
        .await
        .status,
        401
    );
    controller.control(Control::Stop).await.unwrap();
}
#[tokio::test]
async fn stale_and_invalid_approvals_never_reach_monitor() {
    let (controller, address, cookie, _token, mut rx, _published) = setup().await;
    let origin = format!("http://{address}");
    let key = fixture().key();
    let context = projection::request_context(&controller.monitor.snapshot().sessions[0]).unwrap();
    for (context, action) in [("old", "allow"), (context.as_str(), "invented")] {
        let response = http(
            address,
            "POST",
            "/api/command",
            Some(&cookie),
            Some(&origin),
            None,
            Some(json!({"type":"submit","session":key,"context":context,"action":action})),
        )
        .await;
        assert_eq!(response.status, 409);
        assert!(rx.try_recv().is_err());
    }
    assert_eq!(
        http(
            address,
            "POST",
            "/api/command",
            Some(&cookie),
            Some(&origin),
            None,
            Some(json!({"type":"submit","session":key,"context":context,"action":"allow"}))
        )
        .await
        .status,
        200
    );
    assert!(
        matches!(rx.recv().await,Some(Command::Submit{response:Answer::Action(id),..}) if id=="allow")
    );
    let mut changed = fixture();
    changed.approval_mut().unwrap().body = "Different command".into();
    let next = monitor::State {
        sessions: vec![changed],
        ..Default::default()
    };
    _published.send_replace(Arc::new(next));
    assert_eq!(
        http(
            address,
            "POST",
            "/api/command",
            Some(&cookie),
            Some(&origin),
            None,
            Some(json!({"type":"submit","session":key,"context":context,"action":"allow"}))
        )
        .await
        .status,
        409
    );
    assert!(rx.try_recv().is_err());
    controller.control(Control::Stop).await.unwrap();
}
#[tokio::test]
async fn websocket_delivers_initial_state_and_later_revisions() {
    let (controller, address, cookie, _token, _rx, published) = setup().await;
    let (ready, wait) = tokio::sync::oneshot::channel();
    let task = tokio::task::spawn_blocking(move || {
        use tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://{address}/api/events")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Origin", format!("http://{address}").parse().unwrap());
        request
            .headers_mut()
            .insert("Cookie", cookie.parse().unwrap());
        let (mut ws, _) = tungstenite::connect(request).unwrap();
        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = ws.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
        }
        let first: Value = serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(first["type"], "snapshot");
        assert_eq!(
            first["state"]["sessions"]["codex:web-test"]["title"],
            "Phone test"
        );
        ready.send(()).unwrap();
        loop {
            let message = ws.read().unwrap();
            if message.is_text() {
                let value: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if value["type"] == "patch" {
                    assert_eq!(value["baseRevision"], 1);
                    assert_eq!(value["revision"], 2);
                    assert_eq!(
                        value["changes"],
                        json!([{"op":"set","path":["sessions","codex:web-test","title"],"value":"Updated"}])
                    );
                    break;
                }
            }
        }
        let _ = ws.close(None);
    });
    wait.await.unwrap();
    let mut next = (**published.borrow()).clone();
    next.revision = 42;
    next.sessions[0].title = "Updated".into();
    published.send_replace(Arc::new(next));
    task.await.unwrap();
    controller.control(Control::Stop).await.unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "Runs the hidden AppKit/WebKit browser check; invoke explicitly on macOS"]
async fn real_browser_pairs_and_submits_over_http() {
    let (controller, _address, _cookie, _token, mut rx, published) = setup().await;
    let mut initial = (**published.borrow()).clone();
    initial.revision += 1;
    initial.sessions[0]
        .interaction_mut()
        .actions
        .push(crate::domain::interaction::Action {
            id: "similar".into(),
            label: "Allow similar commands".into(),
            detail: "cargo test".into(),
        });
    published.send_replace(Arc::new(initial));
    let report = controller.control(Control::Pair).await.unwrap();
    let link = report["pairingLinks"][0].as_str().unwrap().to_owned();
    let browser = tokio::task::spawn_blocking(move || {
        std::process::Command::new("/usr/bin/swift")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/check-web-frontend.swift"
            ))
            .env("AAM_WEB_TEST_URL", link)
            .output()
            .unwrap()
    });
    let command = tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .unwrap();
    assert!(matches!(command,Some(Command::Open(id)) if id=="codex:web-test"));
    let mut next = (**published.borrow()).clone();
    next.revision += 1;
    next.sessions[0].title = "Updated over diff".into();
    published.send_replace(Arc::new(next));
    let command = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .unwrap();
    let Some(Command::History {
        key,
        cursor,
        generation,
    }) = command
    else {
        panic!("expected WebSocket history request");
    };
    assert!(cursor.is_none());
    assert!(matches!(key.client, HistoryClient::Web(_)));
    let mut next = (**published.borrow()).clone();
    next.revision += 1;
    next.history.insert(
        key,
        Arc::new(monitor::History {
            messages: vec![crate::domain::history::Message {
                cursor: "earlier".into(),
                text: "Earlier over WebSocket".into(),
                images: vec![],
            }],
            cursor: Some("earlier".into()),
            generation,
            ..Default::default()
        }),
    );
    published.send_replace(Arc::new(next));
    let command = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .unwrap();
    assert!(matches!(command,Some(Command::Submit{response:Answer::Action(id),..})if id=="allow"));
    let mut next = (**published.borrow()).clone();
    next.revision += 1;
    next.sessions[0].activity = Activity::Idle;
    next.sessions[0].set_approval(None);
    next.sessions[0].set_interaction(Interaction::default());
    published.send_replace(Arc::new(next));
    for sent in [false, true] {
        let command = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap();
        let Some(Command::SendMessage { id, session, text }) = command else {
            panic!("expected real browser message delivery");
        };
        assert_eq!(session, "codex:web-test");
        assert_eq!(text, "Please continue.\nInclude all checks.");
        let mut next = (**published.borrow()).clone();
        next.revision += 1;
        next.message_sends.insert(
            id,
            monitor::MessageSend {
                session,
                status: if sent {
                    monitor::MessageStatus::Sent
                } else {
                    monitor::MessageStatus::Failed("Test delivery failure".into())
                },
            },
        );
        published.send_replace(Arc::new(next));
    }
    let output = browser.await.unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        rx.try_recv().is_err(),
        "message sends must not be duplicated"
    );
    controller.control(Control::Stop).await.unwrap();
}

async fn raw_websocket(
    address: SocketAddr,
    cookie: &str,
    compress: bool,
) -> (tokio::net::TcpStream, String) {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let extension = if compress {
        "Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits\r\n"
    } else {
        ""
    };
    stream.write_all(format!("GET /api/events HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nCookie: {cookie}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{extension}\r\n").as_bytes()).await.unwrap();
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        header.push(stream.read_u8().await.unwrap());
        assert!(header.len() < 16384);
    }
    let header = String::from_utf8(header).unwrap();
    assert!(header.starts_with("HTTP/1.1 101"), "{header}");
    (stream, header)
}

async fn read_raw_frame(stream: &mut tokio::net::TcpStream) -> (u8, bool, Vec<u8>) {
    let first = stream.read_u8().await.unwrap();
    let second = stream.read_u8().await.unwrap();
    assert_eq!(second & 128, 0);
    let length = match second & 127 {
        126 => stream.read_u16().await.unwrap() as usize,
        127 => stream.read_u64().await.unwrap() as usize,
        n => n as usize,
    };
    assert!(length < 2 * 1024 * 1024);
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).await.unwrap();
    (first & 15, first & 64 != 0, payload)
}

fn inflate(payload: &[u8]) -> Vec<u8> {
    let mut input = payload.to_vec();
    input.extend([0, 0, 255, 255]);
    let mut output = vec![0; 2 * 1024 * 1024];
    let mut decoder = flate2::Decompress::new(false);
    decoder
        .decompress(&input, &mut output, flate2::FlushDecompress::Sync)
        .unwrap();
    assert_eq!(decoder.total_in(), input.len() as u64);
    output.truncate(decoder.total_out() as usize);
    output
}

async fn send_raw_text(stream: &mut tokio::net::TcpStream, text: &str, compress: bool) {
    let payload = if compress {
        let mut encoder = flate2::Compress::new(flate2::Compression::fast(), false);
        let mut bytes = vec![0; text.len() + 1024];
        encoder
            .compress(text.as_bytes(), &mut bytes, flate2::FlushCompress::Sync)
            .unwrap();
        bytes.truncate(encoder.total_out() as usize);
        assert!(bytes.ends_with(&[0, 0, 255, 255]));
        bytes.truncate(bytes.len() - 4);
        bytes
    } else {
        text.as_bytes().to_vec()
    };
    let mut frame = vec![if compress { 0xc1 } else { 0x81 }];
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else {
        frame.push(0xfe);
        frame.extend((payload.len() as u16).to_be_bytes());
    }
    let mask = [3, 1, 4, 1];
    frame.extend(mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    stream.write_all(&frame).await.unwrap();
}

#[tokio::test]
async fn compressed_stream_skips_repeats_sends_small_diffs_resyncs_and_revokes() {
    let (controller, address, cookie, _token, _rx, published) = setup().await;
    let (mut socket, header) = raw_websocket(address, &cookie, true).await;
    assert!(header.contains("permessage-deflate"));
    assert!(header.contains("server_no_context_takeover"));
    let (op, compressed, payload) = read_raw_frame(&mut socket).await;
    assert_eq!(op, 1);
    assert!(compressed);
    let decoded = inflate(&payload);
    assert!(payload.len() < decoded.len());
    let first: Value = serde_json::from_slice(&decoded).unwrap();
    assert_eq!(first["type"], "snapshot");
    let mut next = (**published.borrow()).clone();
    next.revision += 1;
    published.send_replace(Arc::new(next.clone()));
    assert!(
        tokio::time::timeout(Duration::from_millis(350), read_raw_frame(&mut socket))
            .await
            .is_err(),
        "revision-only updates must not send data"
    );
    next.sessions[0].title = "Updated".into();
    published.send_replace(Arc::new(next));
    let (op, compressed, payload) =
        tokio::time::timeout(Duration::from_secs(2), read_raw_frame(&mut socket))
            .await
            .unwrap();
    assert_eq!(op, 1);
    let patch: Value = serde_json::from_slice(&if compressed {
        inflate(&payload)
    } else {
        payload
    })
    .unwrap();
    assert_eq!(patch["type"], "patch");
    assert_eq!(patch["baseRevision"], 1);
    assert_eq!(patch["revision"], 2);
    assert_eq!(
        patch["changes"],
        json!([{"op":"set","path":["sessions","codex:web-test","title"],"value":"Updated"}])
    );
    send_raw_text(&mut socket, r#"{"type":"resync","protocol":1}"#, false).await;
    let (_, compressed, payload) = read_raw_frame(&mut socket).await;
    let reset: Value = serde_json::from_slice(&if compressed {
        inflate(&payload)
    } else {
        payload
    })
    .unwrap();
    assert_eq!(reset["type"], "snapshot");
    assert_eq!(reset["revision"], 3);
    assert_eq!(
        reset["state"]["sessions"]["codex:web-test"]["title"],
        "Updated"
    );
    let id = controller.control(Control::Status).await.unwrap()["devices"][0]["id"]
        .as_u64()
        .unwrap();
    controller.control(Control::Revoke { id }).await.unwrap();
    let (op, _, _) = tokio::time::timeout(Duration::from_secs(2), read_raw_frame(&mut socket))
        .await
        .unwrap();
    assert_eq!(op, 8);
    controller.control(Control::Stop).await.unwrap();
}

#[tokio::test]
async fn compression_is_negotiated_and_oversized_inflated_input_is_rejected() {
    let (controller, address, cookie, _token, mut rx, _published) = setup().await;
    let (mut plain, header) = raw_websocket(address, &cookie, false).await;
    assert!(!header.to_lowercase().contains("sec-websocket-extensions"));
    let (_, compressed, payload) = read_raw_frame(&mut plain).await;
    assert!(!compressed);
    assert_eq!(
        serde_json::from_slice::<Value>(&payload).unwrap()["type"],
        "snapshot"
    );
    let (mut compressed, _) = raw_websocket(address, &cookie, true).await;
    read_raw_frame(&mut compressed).await;
    // Highly compressible input must still respect the decompressed 64 KiB cap.
    send_raw_text(&mut compressed, &"x".repeat(70 * 1024), true).await;
    let closed = tokio::time::timeout(Duration::from_secs(2), compressed.read_u8())
        .await
        .unwrap();
    assert!(closed.is_err() || closed.is_ok_and(|first| first & 15 == 8));
    assert!(rx.try_recv().is_err());
    controller.control(Control::Stop).await.unwrap();
}

#[tokio::test]
async fn static_http_assets_negotiate_gzip() {
    use std::io::Read;
    let (controller, address, _cookie, _token, _rx, _published) = setup().await;
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(format!("GET /monitor.js HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nAccept-Encoding: gzip\r\n\r\n").as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    socket.read_to_end(&mut bytes).await.unwrap();
    let split = bytes.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
    let header = String::from_utf8_lossy(&bytes[..split]).to_lowercase();
    assert!(header.contains("content-encoding: gzip"));
    let mut body = &bytes[split + 4..];
    let mut data = Vec::new();
    if header.contains("transfer-encoding: chunked") {
        loop {
            let n = body.windows(2).position(|b| b == b"\r\n").unwrap();
            let length =
                usize::from_str_radix(std::str::from_utf8(&body[..n]).unwrap(), 16).unwrap();
            body = &body[n + 2..];
            if length == 0 {
                break;
            }
            data.extend_from_slice(&body[..length]);
            body = &body[length + 2..];
        }
    } else {
        data.extend_from_slice(body);
    }
    let mut decoded = String::new();
    flate2::read::GzDecoder::new(&data[..])
        .read_to_string(&mut decoded)
        .unwrap();
    assert_eq!(decoded, include_str!("../monitor.js"));
    assert!(data.len() < decoded.len() / 2);
    controller.control(Control::Stop).await.unwrap();
}

#[tokio::test]
async fn tunnel_ingress_requires_exact_https_origin_and_separates_device_authority() {
    for (hostname, provider) in [
        ("fixture.trycloudflare.com", "cloudflare"),
        ("fixture.ngrok-free.app", "ngrok"),
        ("fixture.a.2na.am", "tuna"),
        ("fixture.a.tuna.free", "tuna"),
        ("fixture.ru.tuna.am", "tuna"),
    ] {
        verify_tunnel_ingress(hostname, provider).await;
    }
}

async fn verify_tunnel_ingress(hostname: &str, provider: &'static str) {
    let (controller, local, local_cookie, _token, _rx, _published) = setup().await;
    let context = controller
        .server
        .lock()
        .await
        .as_ref()
        .unwrap()
        .context
        .clone();
    let origin = format!("https://{hostname}");
    *context.tunnel_status(provider).lock().unwrap() = tunnel::Status {
        provider,
        generation: 9,
        phase: "connected",
        origin: Some(origin.clone()),
        error: None,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = ingress_router(
        context.clone(),
        if provider == "ngrok" {
            Ingress::Ngrok
        } else if provider == "tuna" {
            Ingress::Tuna
        } else {
            Ingress::Tunnel
        },
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    assert_eq!(
        http(local, "GET", "/", None, None, Some(hostname), None)
            .await
            .status,
        403
    );
    assert_eq!(
        http(address, "GET", "/", None, None, None, None)
            .await
            .status,
        403
    );
    assert_eq!(
        http(
            address,
            "GET",
            "/api/snapshot",
            Some(&local_cookie),
            Some(&origin),
            Some(hostname),
            None
        )
        .await
        .status,
        401
    );
    let report = controller.control(Control::Pair).await.unwrap();
    assert_eq!(report["urls"][0], format!("{origin}/"));
    let token = report["pairingLinks"][0]
        .as_str()
        .unwrap()
        .split("#pair=")
        .nth(1)
        .unwrap();
    assert_eq!(
        http(
            address,
            "POST",
            "/api/pair",
            None,
            Some(&format!("http://{hostname}")),
            Some(hostname),
            Some(json!({"token":token}))
        )
        .await
        .status,
        403
    );
    let paired = http(
        address,
        "POST",
        "/api/pair",
        None,
        Some(&origin),
        Some(hostname),
        Some(json!({"token":token,"name":"Remote phone"})),
    )
    .await;
    assert_eq!(paired.status, 200);
    assert!(paired.headers.contains("; Secure"));
    let remote_cookie = paired
        .headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("set-cookie: ")
                .or_else(|| line.strip_prefix("Set-Cookie: "))
        })
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    assert_eq!(
        http(
            address,
            "GET",
            "/api/snapshot",
            Some(remote_cookie),
            Some(&origin),
            Some(hostname),
            None
        )
        .await
        .status,
        200
    );
    assert_eq!(
        http(
            local,
            "GET",
            "/api/snapshot",
            Some(remote_cookie),
            None,
            None,
            None
        )
        .await
        .status,
        401
    );
    let page = http(address, "GET", "/", None, None, Some(hostname), None).await;
    assert!(page.headers.contains(&format!("wss://{hostname}")));
    assert!(String::from_utf8_lossy(&page.body).contains(&format!("wss://{hostname}")));
    // Untrusted forwarded headers cannot grant tunnel ingress on the LAN socket.
    let spoofed = format!("{local}\r\nX-Forwarded-Host: {hostname}\r\nX-Forwarded-Proto: https");
    assert_eq!(
        http(
            local,
            "GET",
            "/api/snapshot",
            Some(remote_cookie),
            Some(&origin),
            Some(&spoofed),
            None
        )
        .await
        .status,
        403
    );
    controller.control(Control::TunnelStop).await.unwrap();
    assert_eq!(
        http(
            address,
            "GET",
            "/api/snapshot",
            Some(remote_cookie),
            Some(&origin),
            Some(hostname),
            None
        )
        .await
        .status,
        403
    );
    assert_eq!(
        http(
            local,
            "GET",
            "/api/snapshot",
            Some(&local_cookie),
            None,
            None,
            None
        )
        .await
        .status,
        200
    );
    assert!(
        !context
            .auth
            .lock()
            .unwrap()
            .devices
            .values()
            .any(|d| d.remote)
    );
    task.abort();
    controller.control(Control::Stop).await.unwrap();
}

#[tokio::test]
async fn websocket_history_uses_the_device_cursor_and_replies_without_advancing_state_revisions() {
    let (controller, address, cookie, _token, mut rx, published) = setup().await;
    let (mut socket, _) = raw_websocket(address, &cookie, false).await;
    read_raw_frame(&mut socket).await;
    let request = json!({"type":"history","protocol":1,"id":"11111111111111111111111111111111","session":"codex:web-test"});
    send_raw_text(&mut socket, &request.to_string(), false).await;
    let (_, _, body) = read_raw_frame(&mut socket).await;
    let reply: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply["type"], "command_result");
    assert_eq!(reply["id"], request["id"]);
    assert_eq!(reply["accepted"], true);
    assert!(reply.get("revision").is_none());
    let Some(Command::History { key, cursor, .. }) = rx.recv().await else {
        panic!("history command missing");
    };
    assert_eq!(key.session, "codex:web-test");
    assert!(matches!(key.client, HistoryClient::Web(_)));
    assert!(cursor.is_none());
    send_raw_text(&mut socket, &request.to_string(), false).await;
    let (_, _, body) = read_raw_frame(&mut socket).await;
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), reply);
    assert!(
        rx.try_recv().is_err(),
        "same request ID must not dispatch twice"
    );
    let mut next = (**published.borrow()).clone();
    next.history.insert(
        key.clone(),
        Arc::new(monitor::History {
            cursor: Some("older-cursor".into()),
            ..Default::default()
        }),
    );
    published.send_replace(Arc::new(next));
    read_raw_frame(&mut socket).await; // history state diff
    let mut older = request.clone();
    older["id"] = "22222222222222222222222222222222".into();
    send_raw_text(&mut socket, &older.to_string(), false).await;
    read_raw_frame(&mut socket).await;
    let Some(Command::History {
        key: received,
        cursor,
        ..
    }) = rx.recv().await
    else {
        panic!("second history command missing");
    };
    assert_eq!(received, key);
    assert_eq!(cursor.as_deref(), Some("older-cursor"));
    let mut unavailable = older;
    unavailable["id"] = "33333333333333333333333333333333".into();
    unavailable["session"] = "codex:missing".into();
    send_raw_text(&mut socket, &unavailable.to_string(), false).await;
    let (_, _, body) = read_raw_frame(&mut socket).await;
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("no longer visible")
    );
    assert!(rx.try_recv().is_err());
    let mut next = (**published.borrow()).clone();
    Arc::make_mut(next.history.get_mut(&key).unwrap()).exhausted = true;
    published.send_replace(Arc::new(next));
    read_raw_frame(&mut socket).await;
    let mut exhausted = request;
    exhausted["id"] = "44444444444444444444444444444444".into();
    send_raw_text(&mut socket, &exhausted.to_string(), false).await;
    let (_, _, body) = read_raw_frame(&mut socket).await;
    assert!(
        serde_json::from_slice::<Value>(&body).unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("No history page")
    );
    assert!(rx.try_recv().is_err());
    controller.control(Control::Stop).await.unwrap();
}

#[tokio::test]
async fn simultaneous_tunnels_have_separate_authority_and_provider_restart_preserves_other_devices()
{
    let (controller, _local, local_cookie, _, _rx, _published) = setup().await;
    let context = controller
        .server
        .lock()
        .await
        .as_ref()
        .unwrap()
        .context
        .clone();
    for provider in ["cloudflare", "ngrok", "tuna"] {
        *context.tunnel_status(provider).lock().unwrap() = tunnel::Status {
            provider,
            generation: 99,
            phase: "connected",
            origin: Some(format!(
                "https://fixture.{}",
                if provider == "ngrok" {
                    "ngrok-free.app"
                } else if provider == "tuna" {
                    "ru.tuna.am"
                } else {
                    "trycloudflare.com"
                }
            )),
            error: None,
        };
    }
    let report = controller.control(Control::Status).await.unwrap();
    let urls = report["urls"].as_array().unwrap();
    assert!(
        urls.iter()
            .any(|url| url == "https://fixture.trycloudflare.com/")
    );
    assert!(
        urls.iter()
            .any(|url| url == "https://fixture.ngrok-free.app/")
    );
    assert!(
        urls.iter()
            .any(|url| url.as_str().unwrap().starts_with("http://"))
    );
    assert!(urls.iter().any(|url| url == "https://fixture.ru.tuna.am/"));
    let (cloudflare_cookie, ngrok_cookie, tuna_cookie) = {
        let mut auth = context.auth.lock().unwrap();
        let token = auth.pair_token();
        let (cf, _) = auth
            .pair(&token, "cloudflare phone", Ingress::Tunnel)
            .unwrap();
        let token = auth.pair_token();
        let (ng, _) = auth.pair(&token, "ngrok phone", Ingress::Ngrok).unwrap();
        let token = auth.pair_token();
        let (tuna, _) = auth.pair(&token, "Tuna phone", Ingress::Tuna).unwrap();
        (cf, ng, tuna)
    };
    let headers = |cookie: &str| {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("aam_session={cookie}")).unwrap(),
        );
        h
    };
    assert!(device(&context, &headers(&cloudflare_cookie), Ingress::Tunnel).is_some());
    assert!(device(&context, &headers(&cloudflare_cookie), Ingress::Ngrok).is_none());
    assert!(device(&context, &headers(&ngrok_cookie), Ingress::Ngrok).is_some());
    assert!(device(&context, &headers(&ngrok_cookie), Ingress::Tunnel).is_none());
    assert!(device(&context, &headers(&tuna_cookie), Ingress::Tuna).is_some());
    assert!(device(&context, &headers(&tuna_cookie), Ingress::Ngrok).is_none());
    assert!(device(&context, &headers(&ngrok_cookie), Ingress::Tuna).is_none());
    context.retire_remote(
        Some(98),
        tunnel::Status {
            provider: "tuna",
            phase: "failed",
            ..Default::default()
        },
    );
    assert_eq!(context.tuna.lock().unwrap().phase, "connected");
    context.close_provider("tuna");
    assert!(device(&context, &headers(&tuna_cookie), Ingress::Tuna).is_none());
    assert!(device(&context, &headers(&ngrok_cookie), Ingress::Ngrok).is_some());
    assert!(device(&context, &headers(&cloudflare_cookie), Ingress::Tunnel).is_some());
    context.close_provider("ngrok");
    assert!(device(&context, &headers(&ngrok_cookie), Ingress::Ngrok).is_none());
    assert!(device(&context, &headers(&cloudflare_cookie), Ingress::Tunnel).is_some());
    assert!(
        device(
            &context,
            &headers(local_cookie.strip_prefix("aam_session=").unwrap()),
            Ingress::Local
        )
        .is_some()
    );
    assert_eq!(context.tunnel.lock().unwrap().phase, "connected");
    context.close_provider("cloudflare");
    assert!(device(&context, &headers(&cloudflare_cookie), Ingress::Tunnel).is_none());
    assert!(
        device(
            &context,
            &headers(local_cookie.strip_prefix("aam_session=").unwrap()),
            Ingress::Local
        )
        .is_some()
    );
    controller.control(Control::Stop).await.unwrap();
}

#[tokio::test]
async fn unconfigured_ngrok_discovery_does_not_rotate_an_existing_local_pairing_link() {
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let (_published, state) = tokio::sync::watch::channel(Arc::new(monitor::State::default()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (context, server) = start_server(listener, monitor::Handle { tx, state });
    let token = context.auth.lock().unwrap().pair_token();
    context.retire_remote(
        None,
        tunnel::Status {
            provider: "ngrok",
            phase: "not_configured",
            ..Default::default()
        },
    );
    assert!(
        context
            .auth
            .lock()
            .unwrap()
            .pair(&token, "phone", Ingress::Local)
            .is_ok()
    );
    server.abort();
}
