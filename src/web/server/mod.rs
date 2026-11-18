//! Opt-in resident HTTP frontend. Management is available only over authenticated local IPC.
mod auth;
mod projection;
mod tuna;
mod tunnel;
mod updates;
use crate::{
    actors::monitor::{self, Command, HistoryClient, HistoryKey},
    domain::{interaction::Response as Answer, session::Session},
};
use anyhow::{Context, Result, bail};
use auth::{Auth, Device};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    net::{IpAddr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Mutex as AsyncMutex, Semaphore},
    task::JoinHandle,
};
use yawc::{
    CompressionLevel, HttpWebSocket, IncomingUpgrade, Options,
    close::CloseCode,
    frame::{Frame, OpCode},
};

#[derive(Clone, clap::Subcommand, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Control {
    /// Start the web frontend (use 0.0.0.0:8765 for access from your LAN).
    Start {
        #[arg(long, default_value = "127.0.0.1:8765")]
        listen: SocketAddr,
    },
    /// Start an experimental, account-free Cloudflare Quick Tunnel.
    TunnelStart,
    /// Start the built-in ngrok SDK using the saved personal authtoken.
    NgrokStart,
    /// Start Tuna using its installed official CLI and saved API key.
    TunaStart,
    /// Store a Tuna API key through authenticated local IPC.
    #[command(skip)]
    TunaConfigure { token: String },
    /// Save a personal authtoken and connect through the built-in SDK (local IPC only).
    #[command(skip)]
    NgrokConfigure { token: String },
    /// Stop all public tunnels and disconnect remote browsers.
    TunnelStop,
    /// Print the current listening address and paired devices.
    Status,
    /// Create a new single-use link, valid for two minutes.
    Pair,
    /// Revoke a paired device by its ID.
    Revoke { id: u64 },
    /// Stop web access and disconnect devices, retaining their authorization.
    Stop,
}
impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NgrokConfigure { .. } => f.write_str("NgrokConfigure { token: [REDACTED] }"),
            Self::TunaConfigure { .. } => f.write_str("TunaConfigure { token: [REDACTED] }"),
            _ => f
                .debug_tuple("WebControl")
                .field(&std::mem::discriminant(self))
                .finish(),
        }
    }
}
pub struct Controller {
    monitor: monitor::Handle,
    server: AsyncMutex<Option<Server>>,
}
struct Server {
    context: Arc<ContextState>,
    task: JoinHandle<()>,
    listen: SocketAddr,
    tunnel: Option<tunnel::QuickTunnel>,
    ngrok: Option<tunnel::QuickTunnel>,
    tuna: Option<tunnel::QuickTunnel>,
}
impl Drop for Server {
    fn drop(&mut self) {
        let mut auth = self.context.auth.lock().unwrap();
        auth.active = false;
        for device in auth.devices.values() {
            let _ = self.context.monitor.send(Command::ForgetHistory {
                client: HistoryClient::Web(device.id),
            });
        }
        self.context.revoked.send_replace(());
        self.task.abort();
    }
}
struct ContextState {
    monitor: monitor::Handle,
    auth: Mutex<Auth>,
    authorities: Mutex<Vec<String>>,
    tunnel: Mutex<tunnel::Status>,
    ngrok: Mutex<tunnel::Status>,
    tuna: Mutex<tunnel::Status>,
    sockets: Arc<Semaphore>,
    generation: AtomicU64,
    ngrok_configured: AtomicBool,
    tuna_configured: AtomicBool,
    revoked: tokio::sync::watch::Sender<()>,
}
impl ContextState {
    fn tunnel_status(&self, provider: &str) -> &Mutex<tunnel::Status> {
        if provider == "ngrok" {
            &self.ngrok
        } else if provider == "tuna" {
            &self.tuna
        } else {
            &self.tunnel
        }
    }
    fn close_provider(&self, provider: &'static str) {
        self.retire_remote(
            None,
            tunnel::Status {
                provider,
                ..Default::default()
            },
        );
    }
    fn close_remote(&self) {
        self.close_provider("cloudflare");
        self.close_provider("ngrok");
        self.close_provider("tuna");
    }
    fn ingress_available(&self, ingress: Ingress) -> bool {
        ingress.provider().is_none_or(|provider| {
            self.tunnel_status(provider)
                .lock()
                .unwrap()
                .origin
                .is_some()
        })
    }
    fn authorized_device(&self, token: &str, ingress: Ingress) -> Option<Device> {
        if !self.ingress_available(ingress) {
            return None;
        }
        self.auth
            .lock()
            .unwrap()
            .device(token)
            .filter(|d| d.ingress == ingress)
    }
    fn retire_remote(&self, generation: Option<u64>, status: tunnel::Status) {
        let provider = status.provider;
        let mut current = self.tunnel_status(provider).lock().unwrap();
        if generation.is_some_and(|generation| current.generation != generation) {
            return;
        }
        let had_origin = current.origin.is_some();
        *current = status;
        let mut auth = self.auth.lock().unwrap();
        let disconnected: Vec<_> = auth
            .devices
            .values()
            .filter(|d| d.ingress.provider() == Some(provider))
            .map(|d| d.id)
            .collect();
        if had_origin || !disconnected.is_empty() {
            auth.pair_token();
        }
        drop(auth);
        for id in disconnected {
            let _ = self.monitor.send(Command::ForgetHistory {
                client: HistoryClient::Web(id),
            });
        }
        self.revoked.send_replace(());
    }
}
impl Controller {
    pub fn new(monitor: monitor::Handle) -> Self {
        Self {
            monitor,
            server: AsyncMutex::new(None),
        }
    }
    pub async fn control(&self, action: Control) -> Result<Value> {
        if let Control::NgrokConfigure { token } = &action {
            tunnel::validate_ngrok_token(token.trim())?;
        }
        if let Control::TunaConfigure { token } = &action {
            tuna::validate_key(token.trim())?;
        }
        let mut server = self.server.lock().await;
        if let Control::Start { listen } = action {
            if server.is_some() {
                bail!("Web access is already running. Stop it before changing the address.");
            }
            #[cfg(not(test))]
            let saved_auth = tokio::task::spawn_blocking(|| {
                Auth::load(crate::app::core::state_home().join("web-devices.json"))
            })
            .await
            .context("Cannot load browser authorization")??;
            let listener = TcpListener::bind(listen)
                .await
                .context("Cannot bind web listener")?;
            let listen = listener.local_addr()?;
            let (context, task) = start_server(listener, self.monitor.clone());
            #[cfg(not(test))]
            {
                *context.auth.lock().unwrap() = saved_auth;
            }
            *server = Some(Server {
                context,
                task,
                listen,
                tunnel: None,
                ngrok: None,
                tuna: None,
            });
        }
        if matches!(action, Control::Start { .. })
            && let Some(server) = server.as_mut()
        {
            // Discover the saved credential once, off the IPC/runtime thread.
            // A missing token is a normal unconfigured state, not a failed tunnel.
            match tunnel::QuickTunnel::start_ngrok(server.context.clone(), None).await {
                Ok(connector) => server.ngrok = Some(connector),
                Err(error) => {
                    let mut status = server.context.ngrok.lock().unwrap();
                    status.phase = "failed";
                    status.error = Some(format!("{error:#}"));
                }
            }
        }
        if matches!(action, Control::Start { .. })
            && let Some(server) = server.as_mut()
        {
            match tuna::start(server.context.clone(), None).await {
                Ok(connector) => server.tuna = Some(connector),
                Err(error) => {
                    let mut status = server.context.tuna.lock().unwrap();
                    status.phase = "failed";
                    status.error = Some(format!("{error:#}"));
                }
            }
        }
        if matches!(action, Control::Stop) {
            server.take();
            return Ok(json!({"running":false}));
        }
        let Some(server) = server.as_mut() else {
            return Ok(json!({"running":false}));
        };
        if matches!(action, Control::TunnelStop) {
            server.tunnel.take();
            server.ngrok.take();
            server.tuna.take();
            server.context.close_remote();
        }
        if matches!(
            action,
            Control::TunnelStart | Control::NgrokStart | Control::NgrokConfigure { .. }
        ) {
            let ngrok = matches!(action, Control::NgrokStart | Control::NgrokConfigure { .. });
            let provider = if ngrok { "ngrok" } else { "cloudflare" };
            if ngrok {
                server.ngrok.take();
            } else {
                server.tunnel.take();
            }
            server.context.close_provider(provider);
            let started = if ngrok {
                let token = match &action {
                    Control::NgrokConfigure { token } => Some(token.trim().to_owned()),
                    _ => None,
                };
                tunnel::QuickTunnel::start_ngrok(server.context.clone(), token).await
            } else {
                tunnel::QuickTunnel::start(server.context.clone()).await
            };
            match started {
                Ok(tunnel) => {
                    if ngrok {
                        server.ngrok = Some(tunnel);
                    } else {
                        server.tunnel = Some(tunnel);
                    }
                }
                Err(error) => {
                    let mut status = server.context.tunnel_status(provider).lock().unwrap();
                    status.provider = provider;
                    status.phase = "failed";
                    status.error = Some(format!("{error:#}"));
                    return Err(error);
                }
            }
        }
        if matches!(action, Control::TunaStart | Control::TunaConfigure { .. }) {
            server.tuna.take();
            server.context.close_provider("tuna");
            let token = match &action {
                Control::TunaConfigure { token } => Some(token.trim().to_owned()),
                _ => None,
            };
            match tuna::start(server.context.clone(), token).await {
                Ok(connector) => server.tuna = Some(connector),
                Err(error) => {
                    let mut status = server.context.tuna.lock().unwrap();
                    status.phase = "failed";
                    status.error = Some(format!("{error:#}"));
                    return Err(error);
                }
            }
        }
        *server.context.authorities.lock().unwrap() = authorities(server.listen);
        let tunnel = server.context.tunnel.lock().unwrap().clone();
        let ngrok = server.context.ngrok.lock().unwrap().clone();
        let tuna = server.context.tuna.lock().unwrap().clone();
        let mut auth = server.context.auth.lock().unwrap();
        if let Control::Revoke { id } = action {
            auth.revoke(id)?;
            server.context.revoked.send_replace(());
            let _ = self.monitor.send(Command::ForgetHistory {
                client: HistoryClient::Web(id),
            });
        }
        if matches!(
            action,
            Control::Start { .. }
                | Control::Pair
                | Control::TunnelStart
                | Control::NgrokStart
                | Control::NgrokConfigure { .. }
                | Control::TunaStart
                | Control::TunaConfigure { .. }
                | Control::TunnelStop
        ) {
            auth.pair_token();
        }
        let pairing = auth
            .pairing
            .as_ref()
            .filter(|(_, until)| *until > std::time::Instant::now());
        let token = pairing.map(|(token, _)| token.clone());
        let expires = pairing.map(|(_, until)| {
            until
                .saturating_duration_since(std::time::Instant::now())
                .as_secs()
        });
        let mut urls: Vec<_> = server
            .context
            .authorities
            .lock()
            .unwrap()
            .iter()
            .filter(|a| !a.starts_with("localhost:"))
            .map(|a| format!("http://{a}/"))
            .collect();
        let mut remote_urls: Vec<_> = [&tunnel, &ngrok, &tuna]
            .into_iter()
            .filter_map(|status| status.origin.as_ref())
            .map(|origin| format!("{origin}/"))
            .collect();
        remote_urls.append(&mut urls);
        let urls = remote_urls;
        let links: Vec<_> = token
            .as_ref()
            .map(|token| {
                urls.iter()
                    .map(|url| format!("{url}#pair={token}"))
                    .collect()
            })
            .unwrap_or_default();
        let qr = links
            .first()
            .and_then(|link| qrcode::QrCode::new(link).ok())
            .map(|qr| qr.render::<qrcode::render::unicode::Dense1x2>().build());
        Ok(
            json!({"running":auth.active&&!server.task.is_finished(),"listen":server.listen,"urls":urls,"pairingLinks":links,"pairingExpiresIn":expires,"qr":qr,"tunnel":{"enabled":server.tunnel.is_some(),"provider":tunnel.provider,"tokenSaved":server.context.ngrok_configured.load(Ordering::Relaxed),"status":tunnel.phase,"url":tunnel.origin,"error":tunnel.error},"ngrok":{"enabled":server.ngrok.is_some(),"tokenSaved":server.context.ngrok_configured.load(Ordering::Relaxed),"status":ngrok.phase,"url":ngrok.origin,"error":ngrok.error},"tuna":{"clientInstalled":tuna::installed(),"enabled":server.tuna.is_some(),"tokenSaved":server.context.tuna_configured.load(Ordering::Relaxed),"status":tuna.phase,"url":tuna.origin,"error":tuna.error},"devices":auth.devices.values().map(|d|json!({"id":d.id,"name":d.name,"remote":d.remote})).collect::<Vec<_>>()}),
        )
    }
}
fn start_server(
    listener: TcpListener,
    monitor: monitor::Handle,
) -> (Arc<ContextState>, JoinHandle<()>) {
    let listen = listener.local_addr().expect("bound TCP listener");
    let context = Arc::new(ContextState {
        monitor,
        auth: Mutex::new(Auth::default()),
        authorities: Mutex::new(authorities(listen)),
        tunnel: Mutex::new(tunnel::Status::default()),
        ngrok: Mutex::new(tunnel::Status {
            provider: "ngrok",
            phase: "not_configured",
            ..Default::default()
        }),
        tuna: Mutex::new(tunnel::Status {
            provider: "tuna",
            phase: "not_configured",
            ..Default::default()
        }),
        sockets: Arc::new(Semaphore::new(8)),
        generation: AtomicU64::new(1),
        ngrok_configured: AtomicBool::new(false),
        tuna_configured: AtomicBool::new(false),
        revoked: tokio::sync::watch::channel(()).0,
    });
    let app = router(context.clone());
    let state = context.clone();
    let task = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            state.auth.lock().unwrap().active = false;
            eprintln!("Web listener stopped: {error}");
        }
    });
    (context, task)
}
fn authorities(listen: SocketAddr) -> Vec<String> {
    let mut ips = vec![listen.ip()];
    if listen.ip().is_unspecified() {
        ips = vec![IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)];
        // SAFETY: Traverse the OS-owned list only until freeifaddrs; check family
        // before casting each non-null socket address. IPv6 scope IDs are omitted.
        unsafe {
            let mut head = std::ptr::null_mut();
            if libc::getifaddrs(&mut head) == 0 {
                let mut at = head;
                while !at.is_null() {
                    let item = &*at;
                    if !item.ifa_addr.is_null()
                        && (*item.ifa_addr).sa_family as i32 == libc::AF_INET
                    {
                        let addr = &*(item.ifa_addr as *const libc::sockaddr_in);
                        ips.push(IpAddr::V4(std::net::Ipv4Addr::from(
                            addr.sin_addr.s_addr.to_ne_bytes(),
                        )));
                    }
                    at = item.ifa_next;
                }
                libc::freeifaddrs(head);
            }
        }
    }
    ips.sort();
    ips.dedup();
    let mut values: Vec<_> = ips
        .into_iter()
        .filter(|ip| !ip.is_unspecified())
        .map(|ip| SocketAddr::new(ip, listen.port()).to_string())
        .collect();
    // Prefer LAN addresses in the QR, but keep loopback usable for local checks.
    values.sort_by_key(|a| a.starts_with("127.") || a.starts_with("[::1]"));
    if listen.ip().is_loopback() || listen.ip().is_unspecified() {
        values.push(format!("localhost:{}", listen.port()));
    }
    values
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Ingress {
    Local,
    Tunnel,
    Ngrok,
    Tuna,
}
impl Ingress {
    fn provider(self) -> Option<&'static str> {
        match self {
            Self::Local => None,
            Self::Tunnel => Some("cloudflare"),
            Self::Ngrok => Some("ngrok"),
            Self::Tuna => Some("tuna"),
        }
    }
}
#[derive(Clone)]
struct Guard {
    context: Arc<ContextState>,
    ingress: Ingress,
}
fn router(context: Arc<ContextState>) -> Router {
    ingress_router(context, Ingress::Local)
}
fn ingress_router(context: Arc<ContextState>, ingress: Ingress) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/monitor.css", get(css))
        .route("/monitor.js", get(js))
        .route("/settings.js", get(settings_js))
        .route("/remote.js", get(remote_js))
        .route("/stream.js", get(stream_js))
        .route("/remote.css", get(remote_css))
        .route("/api/pair", post(pair))
        .route("/api/snapshot", get(snapshot))
        .route("/api/events", get(events))
        .route("/api/command", post(command))
        .route("/api/logout", post(logout))
        .route("/api/image/{session}/{image}", get(image))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(middleware::from_fn_with_state(
            Guard {
                context: context.clone(),
                ingress,
            },
            guard,
        ))
        .layer(Extension(ingress))
        .with_state(context)
}
type ApiResult = std::result::Result<Response, (StatusCode, Json<Value>)>;
fn error(status: StatusCode, text: impl ToString) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"error":text.to_string()})))
}
fn cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|p| p.strip_prefix("aam_session="))
}
fn device(context: &ContextState, headers: &HeaderMap, ingress: Ingress) -> Option<Device> {
    context.authorized_device(cookie(headers)?, ingress)
}
async fn guard(State(guard): State<Guard>, request: Request, next: Next) -> Response {
    let context = &guard.context;
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    // Ingress is assigned by the private listener, never by forwarded headers.
    let expected = match guard.ingress {
        Ingress::Local
            if context
                .authorities
                .lock()
                .unwrap()
                .iter()
                .any(|a| a == host) =>
        {
            format!("http://{host}")
        }
        Ingress::Tunnel | Ingress::Ngrok | Ingress::Tuna => {
            let origin = context
                .tunnel_status(guard.ingress.provider().unwrap())
                .lock()
                .unwrap()
                .origin
                .clone();
            match origin.filter(|origin| origin.strip_prefix("https://") == Some(host)) {
                Some(origin) => origin,
                None => {
                    return error(
                        StatusCode::FORBIDDEN,
                        "Remote address is unavailable or unrecognized.",
                    )
                    .into_response();
                }
            }
        }
        _ => {
            return error(
                StatusCode::FORBIDDEN,
                "Unrecognized server address. Use the link generated on your computer.",
            )
            .into_response();
        }
    };
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if origin.is_some_and(|o| o != expected)
        || (request.method() != axum::http::Method::GET && origin != Some(expected.as_str()))
        || (request.uri().path() == "/api/events" && origin != Some(expected.as_str()))
    {
        return error(StatusCode::FORBIDDEN, "Cross-origin request rejected.").into_response();
    }
    if !context.auth.lock().unwrap().active {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Web access is stopped.").into_response();
    }
    if request.uri().path().starts_with("/api/")
        && request.uri().path() != "/api/pair"
        && device(context, headers, guard.ingress).is_none()
    {
        return error(
            StatusCode::UNAUTHORIZED,
            "Pair this browser using a new link from your computer.",
        )
        .into_response();
    }
    let policy = format!(
        "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' {}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        websocket_origin(host, guard.ingress)
    );
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&policy).expect("validated origin"),
    );
    response
}
fn websocket_origin(host: &str, ingress: Ingress) -> String {
    format!(
        "{}://{host}",
        if ingress != Ingress::Local {
            "wss"
        } else {
            "ws"
        }
    )
}
async fn index(Extension(ingress): Extension<Ingress>, headers: HeaderMap) -> Response {
    let source = include_str!("../index.html");
    let split = source
        .find("<script>\n(() =>")
        .expect("shared bootstrap marker");
    let page = format!(
        "{}<script src=\"stream.js\"></script><script src=\"remote.js\"></script></body></html>",
        &source[..split]
    );
    let page = page
        .replace(
            "connect-src 'none'",
            &format!(
                "connect-src 'self' {}",
                websocket_origin(
                    headers[header::HOST].to_str().expect("validated host"),
                    ingress
                )
            ),
        )
        .replace("img-src data:", "img-src 'self' data:")
        .replace(
            "<title>",
            "<link rel=\"stylesheet\" href=\"remote.css\"><title>",
        );
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], page).into_response()
}
async fn css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../monitor.css"),
    )
        .into_response()
}
async fn js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../monitor.js"),
    )
        .into_response()
}
async fn settings_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../settings.js"),
    )
        .into_response()
}
async fn remote_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../remote.js"),
    )
        .into_response()
}
async fn stream_js() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../stream.js"),
    )
        .into_response()
}
async fn remote_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../remote.css"),
    )
        .into_response()
}
#[derive(Deserialize)]
struct Pair {
    token: String,
    #[serde(default)]
    name: String,
}
async fn pair(
    State(context): State<Arc<ContextState>>,
    Extension(ingress): Extension<Ingress>,
    Json(input): Json<Pair>,
) -> ApiResult {
    let (secret, device) = context
        .auth
        .lock()
        .unwrap()
        .pair(&input.token, &input.name, ingress)
        .map_err(|e| error(StatusCode::UNAUTHORIZED, e))?;
    let mut response = Json(json!({"device":device.id})).into_response();
    let secure = if ingress != Ingress::Local {
        "; Secure"
    } else {
        ""
    };
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "aam_session={secret}; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000{secure}"
        ))
        .unwrap(),
    );
    Ok(response)
}
async fn logout(State(context): State<Arc<ContextState>>, headers: HeaderMap) -> Response {
    let mut auth = context.auth.lock().unwrap();
    if let Some(token) = cookie(&headers)
        && let Some(device) = auth.device(token)
    {
        if auth.revoke(device.id).is_err() {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not revoke browser authorization. Try again.",
            )
            .into_response();
        }
        let _ = context.monitor.send(Command::ForgetHistory {
            client: HistoryClient::Web(device.id),
        });
        context.revoked.send_replace(());
    }
    (
        [(
            header::SET_COOKIE,
            "aam_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
        )],
        Json(json!({"ok":true})),
    )
        .into_response()
}
async fn snapshot(
    State(context): State<Arc<ContextState>>,
    Extension(ingress): Extension<Ingress>,
    headers: HeaderMap,
) -> ApiResult {
    let device = device(&context, &headers, ingress)
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Session expired"))?;
    Ok(Json(projection::snapshot(&context.monitor.snapshot(), device.id)).into_response())
}
async fn events(
    State(context): State<Arc<ContextState>>,
    headers: HeaderMap,
    Extension(ingress): Extension<Ingress>,
    upgrade: IncomingUpgrade,
) -> ApiResult {
    let token = cookie(&headers).unwrap_or_default().to_owned();
    let permit = context.sockets.clone().try_acquire_owned().map_err(|_| {
        error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many open browser connections.",
        )
    })?;
    let options = Options::default()
        .with_compression_level(CompressionLevel::fast())
        .with_compression_threshold(256)
        .server_no_context_takeover()
        .client_no_context_takeover()
        .with_max_payload_read(65537)
        .with_max_read_buffer(65537)
        .with_read_buffer_capacity(8192)
        .with_utf8();
    let (response, upgraded) = upgrade
        .upgrade(options)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid WebSocket upgrade."))?;
    tokio::spawn(async move {
        let _permit = permit;
        if let Ok(Ok(socket)) = tokio::time::timeout(Duration::from_secs(5), upgraded).await {
            stream(socket, context, token, ingress).await;
        }
    });
    Ok(response.into_response())
}
async fn send_ws(socket: &mut HttpWebSocket, message: Frame) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(5), socket.send(message)).await,
        Ok(Ok(()))
    )
}
async fn stream(
    mut socket: HttpWebSocket,
    context: Arc<ContextState>,
    token: String,
    ingress: Ingress,
) {
    let mut published = context.monitor.subscribe();
    let mut revoked = context.revoked.subscribe();
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    tick.tick().await;
    let mut update = true;
    let mut reset = false;
    let mut baseline = updates::Stream::default();
    let mut last_resync: Option<tokio::time::Instant> = None;
    let mut history_replies = std::collections::VecDeque::<(String, String, Value)>::new();
    while let Some(device) = context.authorized_device(&token, ingress) {
        if update {
            let state = published.borrow_and_update().clone();
            if state.stopping {
                break;
            }
            if let Some(mut packet) =
                baseline.prepare(projection::snapshot(&state, device.id), reset)
            {
                if !send_ws(&mut socket, Frame::text(std::mem::take(&mut packet.text))).await {
                    break;
                }
                baseline.commit(packet);
            }
            reset = false;
        }
        update = false;
        tokio::select! {
            changed=published.changed()=>{
                if changed.is_err(){break;}
                // Coalesce rapid provider updates; send the newest snapshot.
                tokio::time::sleep(Duration::from_millis(100)).await;update=true;
            },
            changed=revoked.changed()=>if changed.is_err(){break;},
            _=tick.tick()=>if !send_ws(&mut socket,Frame::ping(Vec::new())).await {break;},
            incoming=socket.next_frame()=>match incoming {
                Ok(frame) if matches!(frame.opcode(), OpCode::Ping | OpCode::Pong)=>{},
                Ok(frame) if frame.opcode()==OpCode::Text=>{
                    match serde_json::from_slice::<StreamInput>(frame.payload()) {
                        Ok(StreamInput::Resync { protocol }) if protocol==updates::PROTOCOL=>{
                            if last_resync.is_some_and(|at| at.elapsed()<Duration::from_secs(1)) {break;}
                            last_resync=Some(tokio::time::Instant::now());reset=true;update=true;
                        },
                        Ok(StreamInput::History { protocol, id, session }) if protocol==updates::PROTOCOL && valid_operation_id(&id) && session.len()<=512=>{
                            // Revalidate immediately before dispatch, not only before waiting for input.
                            let current=context.authorized_device(&token, ingress);
                            let Some(current)=current else {break;};
                            let result=if let Some((_,original,reply))=history_replies.iter().find(|(key,_,_)|key==&id) {
                                if original==&session {reply.clone()}else{json!({"error":"Request ID was already used for another session."})}
                            }else{
                                let result=match tokio::time::timeout(Duration::from_secs(5),dispatch(&context,&current,Input::History{session:session.clone()})).await {
                                    Ok(Ok(value))=>value,
                                    Ok(Err(error))=>json!({"error":error.to_string()}),
                                    Err(_)=>json!({"error":"History request timed out. Check the session before retrying."}),
                                };
                                if history_replies.len()==32 {history_replies.pop_front();}
                                history_replies.push_back((id.clone(),session,result.clone()));result
                            };
                            let mut reply=result;reply["type"]="command_result".into();reply["protocol"]=updates::PROTOCOL.into();reply["id"]=id.into();
                            if !send_ws(&mut socket,Frame::text(reply.to_string())).await {break;}
                        },
                        _=>break,
                    }
                },
                _=>break,
            },
        }
    }
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        socket.send(Frame::close(CloseCode::Normal, "")),
    )
    .await;
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum StreamInput {
    Resync {
        protocol: u64,
    },
    History {
        protocol: u64,
        id: String,
        session: String,
    },
}
fn valid_operation_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    #[serde(rename = "reset-credits")]
    UsageResets,
    Submit {
        session: String,
        context: String,
        action: Option<String>,
        answers: Option<BTreeMap<String, Vec<String>>>,
    },
    Message {
        session: String,
        text: String,
        id: String,
    },
    Seen {
        session: String,
        generation: u64,
    },
    History {
        session: String,
    },
    Open {
        session: String,
    },
}
fn visible_session(state: &monitor::State, key: &str) -> Result<Session> {
    state
        .sessions
        .iter()
        .find(|s| s.key() == key && state.preferences.applications.enabled(s.client))
        .cloned()
        .context("This session is no longer visible.")
}
async fn command(
    State(context): State<Arc<ContextState>>,
    headers: HeaderMap,
    Extension(ingress): Extension<Ingress>,
    Json(input): Json<Input>,
) -> ApiResult {
    let device = device(&context, &headers, ingress)
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Session expired"))?;
    dispatch(&context, &device, input)
        .await
        .map(Json)
        .map(IntoResponse::into_response)
        .map_err(|e| error(StatusCode::CONFLICT, e))
}
async fn dispatch(context: &ContextState, device: &Device, input: Input) -> Result<Value> {
    let state = context.monitor.snapshot();
    let command = match input {
        Input::UsageResets => Command::ReadUsageResets,
        Input::Submit {
            session,
            context: ticket,
            action,
            answers,
        } => {
            let session = visible_session(&state, &session)?;
            anyhow::ensure!(
                projection::request_context(&session).as_deref() == Some(ticket.as_str())
                    && session.authoritative()
                    && session
                        .approval()
                        .is_some_and(|a| a.expires > monitor::now()),
                "This request is stale or unavailable."
            );
            anyhow::ensure!(
                !matches!(
                    state.submissions.get(&session.approval_key().unwrap()),
                    Some(monitor::Submission::Sending | monitor::Submission::Reconciling)
                ),
                "A response is already being delivered."
            );
            let interaction = session.interaction();
            let response = match (action, answers) {
                (Some(id), None) if interaction.actions.iter().any(|a| a.id == id) => {
                    Answer::Action(id)
                }
                (None, Some(a)) if interaction.valid_answers(&a) => Answer::Answers(a),
                _ => bail!("Choose an offered action or answer every question."),
            };
            Command::Submit {
                session: Box::new(session),
                response,
            }
        }
        Input::Message { session, text, id } => {
            let current = visible_session(&state, &session)?;
            anyhow::ensure!(
                current.can_send_message()
                    && current.authoritative()
                    && current.activity == crate::domain::session::Activity::Idle
                    && current.approval().is_none(),
                "This session is not ready for a message."
            );
            anyhow::ensure!(
                !text.trim().is_empty() && text.chars().count() <= 16000,
                "Enter a message of 1–16000 characters."
            );
            anyhow::ensure!(
                id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit()),
                "Invalid message ID."
            );
            let id = format!("web:{}:{id}", device.id);
            let mut events = context.monitor.subscribe();
            context
                .monitor
                .enqueue(Command::SendMessage {
                    id: id.clone(),
                    session,
                    text,
                })
                .await
                .map_err(anyhow::Error::msg)?;
            return tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    let state = events.borrow_and_update().clone();
                    if let Some(send) = state.message_sends.get(&id) {
                        match &send.status {
                            monitor::MessageStatus::Sent => return Ok(json!({"sent":true})),
                            monitor::MessageStatus::Failed(e) => bail!("{e}"),
                            _ => {}
                        }
                    }
                    events.changed().await.context("Monitor disconnected")?;
                }
            })
            .await
            .context("Delivery is still unconfirmed. Check the session before sending again.")?;
        }
        Input::Seen {
            session,
            generation,
        } => {
            visible_session(&state, &session)?;
            Command::FinishedSeen {
                completions: HashMap::from([(session, generation)]),
            }
        }
        Input::History { session } => {
            visible_session(&state, &session)?;
            let key = HistoryKey {
                session,
                client: HistoryClient::Web(device.id),
            };
            let history = state.history.get(&key);
            anyhow::ensure!(
                !history.is_some_and(|h| h.loading || h.exhausted),
                "No history page is available right now."
            );
            anyhow::ensure!(
                history.is_none_or(|h| h.messages.len() < 30),
                "The web history limit is 30 earlier messages per session."
            );
            let count = state
                .history
                .keys()
                .filter(|k| k.client == HistoryClient::Web(device.id))
                .count();
            anyhow::ensure!(
                history.is_some() || count < 32,
                "History session limit reached. Reconnect this device to clear it."
            );
            let cursor = history.and_then(|h| h.cursor.clone());
            Command::History {
                key,
                cursor,
                generation: context.generation.fetch_add(1, Ordering::Relaxed),
            }
        }
        Input::Open { session } => {
            visible_session(&state, &session)?;
            Command::Open(session)
        }
    };
    context
        .monitor
        .enqueue(command)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(json!({"accepted":true}))
}
async fn image(
    State(context): State<Arc<ContextState>>,
    headers: HeaderMap,
    Path((key, id)): Path<(String, String)>,
    Extension(ingress): Extension<Ingress>,
) -> ApiResult {
    let device = device(&context, &headers, ingress)
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Session expired"))?;
    let decode = |s: String| {
        URL_SAFE_NO_PAD
            .decode(s)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "Image not found"))
    };
    let key = decode(key)?;
    let id = decode(id)?;
    let state = context.monitor.snapshot();
    let session = visible_session(&state, &key)
        .map_err(|_| error(StatusCode::NOT_FOUND, "Image not found"))?;
    let image = session
        .activities
        .iter()
        .filter_map(|a| {
            if let crate::domain::session_activity::SessionActivity::Message(m) = a {
                Some(&m.images)
            } else {
                None
            }
        })
        .flatten()
        .find(|i| i.id == id)
        .cloned()
        .or_else(|| {
            state
                .history
                .get(&HistoryKey {
                    session: key,
                    client: HistoryClient::Web(device.id),
                })
                .and_then(|h| {
                    h.messages
                        .iter()
                        .flat_map(|m| &m.images)
                        .find(|i| i.id == id)
                        .cloned()
                })
        })
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Image not found"))?;
    Ok((
        [(header::CONTENT_TYPE, image.mime)],
        Body::from(image.data.to_vec()),
    )
        .into_response())
}

#[cfg(test)]
mod tests;
