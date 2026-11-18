# Web access

The resident monitor can serve its shared HTML frontend over HTTP. It uses the same monitor actor, bounded session activities, request validation, history reads and message delivery as the native desktop and Telegram. It does not start another Codex client. No public service or tunnel is required.

## Connect a phone

On macOS, open **Settings → Web**, enable **Web access**, and scan the QR code with your phone camera. The switch starts local access and automatically attempts a Cloudflare tunnel when `cloudflared` is installed. For a local HTTP address, the computer and phone must be reachable on the same network; select the address belonging to that network. A public HTTPS address also works outside that network. Allow incoming connections if macOS asks.

The switch starts a listener on `0.0.0.0:8765`. It is off by default and is not automatically restored after restarting the monitor. A bind failure is displayed in the pane. Turning it off stops the listener and disconnects browsers, retaining their authorization. The native monitor keeps working.

Each QR contains an address and a random, single-use pairing secret in the URL fragment. The link expires after two minutes. While Settings → Web is visible, a progress bar beneath the QR shows the remaining seconds; expiration or successful use automatically generates a replacement code. **New QR code** can also replace the secret manually, and **Copy link** copies the selected address with its valid secret. Renewal errors appear inline and retry; expired links cannot be copied. The browser exchanges it for an HttpOnly, SameSite=Strict cookie and immediately removes the fragment. Browser sessions last at most 30 days from pairing. Authorization survives listener and application restarts in a versioned `web-devices.json` file under the per-user state directory (macOS Application Support; Linux XDG state). Only SHA-256 token fingerprints, device IDs/names, ingress and expiry are saved; raw cookies and temporary QR secrets are not saved. Atomic replacement uses a private 0600 file. Expired devices are ignored on load; restart does not renew their lifetime. Storage errors prevent startup/pairing or report failed revocation rather than silently losing trust changes. Logout and `web revoke` persist removal. Browsers paired by an older, memory-only build need one new pairing after upgrading. The displayed browser count means paired browsers, including currently closed tabs.

This is HTTP, not encrypted HTTPS. Pairing restricts application access; it does not encrypt network traffic. Use it on a network you trust. This implementation does not configure internet port forwarding, certificates, VPNs or tunnels.

## Browser links from Telegram

In your paired bot, send `/web_access` (`/web` is also accepted). The bot sends fresh links for all currently available Cloudflare, ngrok, Tuna and local-interface addresses, including loopback. They share one single-use pairing code valid for two minutes. Open the appropriate link on your phone; public tunnel links work outside your LAN when reachable, while local addresses require network connectivity to the computer. Loopback works only on the monitor host. The command preserves the current chat/session selection and does not automatically turn Web access on. If disabled, enable **Settings → Web → Web access** first. Generating these links refreshes the QR pairing code, without disconnecting already paired browsers.

## Remote access with ngrok

Settings → Web shows a collapsible **ngrok: not configured / connected / connection failed** section while Web access is enabled. Cloudflare and ngrok run independently and can publish addresses at the same time. The common address list includes all connected public origins and all usable addresses of the bound local interfaces, including loopback. Selecting an address changes the QR and Copy link destination. Enabling Web access attempts Cloudflare and discovers the saved ngrok credential; ngrok connects automatically when a credential exists. Missing ngrok credentials are a normal unconfigured state and do not affect Cloudflare or LAN access.

ngrok is embedded through the official Rust SDK; no separate agent, CLI configuration or ngrok installation is required. Expand the **ngrok** section and click the **ngrok/browser-arrow** button to open your account's authtoken page, paste the token and click **Connect**. Credentials are stored under a separate `ngrok-authtoken` entry in macOS Keychain (Ubuntu's Secret Service for the shared backend). They are never included in status reports, command arguments or Debug output. The SDK does not read an existing ngrok CLI configuration. Once stored, the expanded section shows **Token: stored securely in macOS Keychain** and **Change**, without revealing the token. Change opens an empty replacement field; Cancel restores the saved-token row. The current connection remains active until Connect submits the replacement. A connected section has no Connect button. A failed connection with a saved token shows **Reconnect**, which reuses that credential. Before configuration **Token** appears above an editable **Paste token** field. An ngrok-lettermark/browser-arrow button beside the field opens the dashboard; Connect is disabled until the field contains non-whitespace text. **Will be stored securely in macOS Keychain** is aligned below the input. The disclosure preserves its expanded state during status polling. Web access remains off on monitor restart. The saved token persists independently of the listener.

The SDK forwards HTTPS traffic to a private ephemeral loopback ingress with the existing exact Host/Origin checks, browser authorization, WSS and compression. Only HTTPS subdomains of ngrok-free.app, ngrok-free.dev, ngrok.app, ngrok.dev and ngrok.io are accepted; custom domains are not configured by this integration. Startup has a 45-second deadline. The SDK handles transient reconnection; if forwarding terminates, the origin is cleared and remote browsers disconnect, retaining authorization. Reconnecting ngrok closes only its previous SDK session, disconnects ngrok browsers without forgetting their authorization and leaves Cloudflare and LAN browsers connected. Disabling Web access closes all remote paths and disconnects all browsers without forgetting their authorization. CLI `web ngrok-start` uses the saved credential; initial token entry is available through the macOS settings pane's authenticated local IPC, not command arguments. `web tunnel-start` starts/restarts Cloudflare without stopping ngrok; `web tunnel-stop` stops all public tunnels. Status has independent `tunnel` (Cloudflare) and `ngrok` objects and a combined `urls`/`pairingLinks` list. `ngrok.tokenSaved` is a boolean; the token itself is never returned.

ngrok account limits and Free-plan browser interstitials apply. A phone may need to accept ngrok's browser warning before pairing. Actual cellular reachability depends on both networks. Local tests cover validation, secret redaction and both providers' remote authorization; live SDK connectivity requires a personal account and must be verified separately. HTTPS terminates at ngrok, so this is not end-to-end encryption against the provider. See [ngrok Rust SDK](https://docs.rs/ngrok/0.19.1/ngrok/).

## Remote access with Tuna

Tuna is a third, independent public access path. Its address appears alongside Cloudflare, ngrok, local interfaces and loopback in Settings and Telegram `/web_access`. The macOS pane and HTML prototype provide a separate **Tuna** disclosure with the same status, API-key entry, Change/Cancel and Reconnect conventions as ngrok.

Install the official Tuna CLI on macOS:

```sh
brew install yuccastream/tap/tuna
```

For Ubuntu use [Tuna's official installation instructions](https://tuna.am/en/docs/guides/install/install-cli/). The monitor searches standard executable directories and PATH; it does not install or bundle the Tuna binary. The CLI's documented HTTP/JSON logging flags were checked against the official downloaded macOS build during implementation. No Tuna SDK is linked.

Create a personal API key in [Tuna's dashboard](https://my.tuna.am/api_keys?lang=en) with the **tunnels:connect** scope. Expand Tuna, paste the key and Connect. The key is stored separately in Keychain/Secret Service as `tuna-api-key`. It is passed to the managed child through `TUNA_API_KEY`, never as a command-line argument or in status/Debug output. The monitor does not reuse Tuna CLI login credentials or configuration. Change keeps the current connection until Connect; Cancel discards the replacement input.

Enabling Web access discovers a saved key and attempts Tuna independently. Without a key, the section remains not configured. A missing executable produces an installation message; other paths remain working. CLI `web tuna-start` reconnects with the saved key. `web tunnel-stop` stops all public providers; `web stop` stops all web access.

The child runs `tuna http` against its own ephemeral loopback ingress with a private empty configuration, no user traffic-policy file, no TUI and request inspection disabled. Standard proxy/certificate environment settings are retained; ambient `TUNA_*` settings are not inherited. Startup waits up to 45 seconds for a structured Forwarding announcement with a validated HTTPS public URL. Supported origins are one-label subdomains of current `a.2na.am` and `a.tuna.free` zones, plus legacy `ru.tuna.am`, `nl.tuna.am` and `run.tuna.am`; API/dashboard URLs, custom domains, embedded credentials, paths and queries are rejected. Raw client logs are bounded and never returned to the UI. Client exit/startup failure removes the origin and disconnects Tuna browsers without forgetting authorization. A changed public origin requires opening/pairing at the new address because browser cookies are scoped to the old hostname. Stop closes only the monitor-owned child and private listener; unrelated Tuna processes are untouched.

Public requests use the same exact Host/Origin checks, Secure browser cookies, WSS, compression and command validation as the other providers, with a separate Tuna ingress identity. Status adds a `tuna` object with connection phase, public URL, error, `tokenSaved` and `clientInstalled`; no credential is exposed. UI instructions name Tuna when its address is selected. Provider transport/TLS privacy guarantees must be assessed separately; the implementation does not claim end-to-end encryption against Tuna.

Local fixture tests cover the managed process arguments/environment, public-URL parsing, provider-specific authentication and revocation, simultaneous addresses, and native/prototype setup states. A real CLI0.40.0 account connection was verified to emit a structured `Forwarding` event using the `url` field and a public `a.2na.am` origin. Authentication and endpoint publication succeeded; full browser pairing and reconnection still require end-to-end verification. Current public zones are documented in [Tuna’s high-availability announcement](https://tuna.am/en/blog/high-availability/). See [Tuna HTTP documentation](https://tuna.am/en/docs/tunnels/http/) and [environment variables](https://tuna.am/en/docs/guides/environment-variables/).

## Experimental remote access with Cloudflare

On macOS, install the official `cloudflared` connector (`brew install cloudflared`), enable **Web access** in the same pane. This single switch also attempts the tunnel; no separate Remote access control is needed. No Cloudflare account, domain or provider token is required. The monitor runs a managed Cloudflare Quick Tunnel process, obtains its `https://*.trycloudflare.com` address and selects that address for the existing pairing QR. Scan it from the phone; the phone does not need a Cloudflare app or account.

Quick Tunnels are intended by Cloudflare for development/testing and have no uptime guarantee. Creating a new tunnel changes the public hostname. Pairing-token renewal keeps the same hostname and does not restart the tunnel. `cloudflared` handles transient network reconnection; a stopped or failed process requires toggling Web access off/on (which also disconnects local browsers), or using the CLI tunnel commands. Restarting Cloudflare leaves ngrok browsers connected. The pane distinguishes starting, connected, reconnecting and failed states. The connector uses HTTP/2 transport, an isolated empty configuration and no named-tunnel credentials. Its own process and temporary config are cleaned up on disable/server stop. The application does not install or register a login daemon.

The tunnel forwards to a separate ephemeral **loopback-only listener**. That listener accepts only the exact provider-issued HTTPS Host and Origin. The LAN listener cannot be converted to public ingress by spoofing forwarded headers. Public responses use same-origin WSS in CSP, and remote pairing sets a Secure, HttpOnly, SameSite=Strict cookie. LAN, Cloudflare and ngrok browser cookies each have separate ingress authority; a cookie issued through one cannot authenticate through another. The CLI `web tunnel-stop` disconnects remote browsers and releases their history while locally paired browsers remain connected. Turning Web access off in Settings disconnects both and stops the connector. Web access is not restored automatically on monitor restart. If the connector is unavailable or fails, local access remains running and the pane displays the error.

Ordinary Cloudflare HTTP proxying terminates TLS at Cloudflare: traffic is encrypted in transit, but this is not end-to-end encryption against Cloudflare. The local pairing/command/image authorization remains required. Only the unauthenticated UI shell is public without pairing. No public URL or token is logged by the monitor. Details for future providers and remaining decisions: [Tunnel integration plan](web-tunnels.md).

## CLI and Ubuntu

The same resident server is available through local IPC on both supported platforms. The GNOME settings window does not yet expose the Web pane; use the CLI there.

```sh
AIAgentMonitor web start --listen 0.0.0.0:8765
AIAgentMonitor web status
AIAgentMonitor web pair
AIAgentMonitor web tunnel-start
AIAgentMonitor web tunnel-stop
AIAgentMonitor web revoke DEVICE_ID
AIAgentMonitor web stop
```

Without `--listen`, CLI Start binds only `127.0.0.1:8765`. Start and Pair print a terminal QR and pairing links. Status returns addresses, the current unconsumed pairing link and paired device IDs. These management commands use the existing same-user Unix socket, never the network API. Links are credentials while valid; do not publish their output.

## Browser capabilities

- View grouped sessions, status/completion markers, current actions, Markdown messages, metadata and usage.
- Expand/collapse, filter titles, acknowledge completions, and read earlier messages.
- Answer offered approvals and structured questions. The request context is revalidated against the current snapshot and again by the monitor actor.
- Send messages to idle sessions with a live message connection. Delivery errors retain the draft; timeout means delivery is unconfirmed, so check the session before sending again.
- View images already published by the agent adapter. The server serves those payloads, not arbitrary files or filesystem paths.
- Switch the session application on the host computer by selecting its title. It does not open a phone-native agent app.
- Disconnect the browser using the header power icon; it does not quit the desktop monitor.

The remote host hides the prototype settings gear. Simulated Telegram/settings actions are never exposed as real controls. The design preview remains a separate mock host, with no network access to the monitor.

A disconnected page disables submissions/message entry until it receives a fresh snapshot. WebSocket reconnect and returning to the foreground obtain current state; actions are not automatically replayed. A browser needs a new QR after explicit revocation or expiry. At the same origin it can reconnect after server restart using the saved authorization once Web access is enabled again. A changed tunnel hostname still requires opening/pairing at the new address; saved authorization cannot transfer a browser cookie between hostnames. The mobile layout uses the dynamic viewport and supports native screen keyboard resizing; it has no service worker or background push notifications.

## Transport and bounds

Embedded assets: `/`, `/monitor.css`, `/monitor.js`, `/settings.js`, `/stream.js`, `/remote.js`, `/remote.css`. HTTP responses negotiate gzip through `Accept-Encoding`; the browser decompresses them transparently. Already compressed image formats are not recompressed by the HTTP layer.

| Endpoint | Behavior |
|---|---|
| `POST /api/pair` | Exchange single-use secret and optional device name for a browser cookie |
| `GET /api/snapshot` | Authenticated current UI projection |
| `GET /api/events` | Authenticated WebSocket; initial keyed snapshot, then revisioned diffs; negotiated `permessage-deflate` |
| `POST /api/command` | Tagged Submit, Message, Seen, History or Open; no arbitrary commands |
| `GET /api/image/{session}/{image}` | Authenticated, base64url-encoded IDs into published session/history images |
| `POST /api/logout` | Revoke the requesting browser cookie |

All API routes except Pair require a valid device session. Mutations and WebSocket upgrades require the matching Origin; the Host must match a listener address. No permissive CORS is configured. Responses use no-store and a restrictive content security policy. Static assets are compiled into the binary; arbitrary paths are not served. Pairing secrets and cookies are excluded from browser state snapshots.

Limits: 16 paired browsers, 8 simultaneous WebSockets, 64 KiB incoming JSON/WebSocket frames, 16000 message characters, 32 history sessions per browser and at most 30 earlier messages per session. History consumers are distinct from native/Telegram consumers and are released on logout, revoke and server stop. The server never loads the whole agent transcript into the UI snapshot.

### Incremental stream protocol (version 1)

This is the web transport protocol, independent of Codex's owner-stream protocol. Each WebSocket owns one last-successfully-sent, device-specific projected document and its own revision counter. Keeping the baseline does not hydrate additional transcript history or copy image payloads into the document. Projection includes only the visible sessions and this device's loaded history.

The initial message is:

```json
{"type":"snapshot","protocol":1,"revision":1,"state":{"sessions":{},"sessionOrder":[],"usage":[],"error":null}}
```

The example omits other ordinary projection fields for clarity. `sessions` is an object keyed by session ID, and each session's `activities` is keyed by activity ID. `sessionOrder` and per-session `activityOrder` are arrays of IDs defining display order. Request choices and historical message lists retain their ordinary array structure. Image payloads remain separate authenticated HTTP resources.

A change is sent as:

```json
{"type":"patch","protocol":1,"baseRevision":1,"revision":2,"changes":[{"op":"set","path":["sessions","codex:example","title"],"value":"Updated title"}]}
```

`set` adds or replaces a key; `remove` removes an existing key and has no value. Paths are arrays of literal object keys, not JSON Pointer strings or executable expressions. Arrays and scalars are atomic values. Null is a value, distinct from removing a key. Entity keys may contain slashes or other punctuation. Updating one activity does not replace other activities; adding/removing/reordering entities also updates the corresponding order array.

The server excludes the monitor's internal `revision` from content comparison. Repeated projected content sends no data message and does not advance the stream revision. Rapid changes are coalesced for 100 ms. Each successful data message advances the connection's stream revision by one; the baseline is committed only after the send succeeds. If a diff's uncompressed JSON is not smaller than a complete packet, a complete snapshot is sent instead. A slow or failed send times out and closes the connection; no unbounded queue of intermediate revisions is retained.

Rendering is incremental as well: changed snapshots update existing session DOM, retaining disclosure clips, editor identity, drafts and selection. A status/message update during expansion or collapse must not replace the article or restart its transition. A new request context replaces only that request's form, preventing old answers from leaking into a different request. Identical snapshots must not remove/reinsert content, including quota and overview counters. This applies equally to full stream packets and diffs; receiving a complete packet does not require rebuilding the interface.

### Browser diagnostics

Append `?diagnostics=1` to the browser URL (before a pairing fragment, if present). The opt-in panel sits above the monitor and offers:

- **Live:** apply the newest received state normally.
- **Freeze:** retain the displayed snapshot while continuing to receive and validate WebSocket updates.
- **Repeat 4/s:** pass the same frozen snapshot to the renderer every 250 ms. Switching between Freeze and Repeat retains the same captured state; returning to Live displays the newest received state.

Freeze and Repeat disable agent responses and message sending. Session disclosure and local browsing remain available, but history, seen acknowledgements, host-app navigation and agent commands are not sent in these modes. Connection loss and authorization failure still disable controls or show the connection gate; diagnostics never bypass transport safety. Repeat pauses while the document is hidden.

The panel shows connection status, reconnect attempts after the first connection, unexpected closes, valid state packets, completed renderer calls, stream/protocol errors and rendering/uncaught JavaScript errors. The latest error text is local to this browser; nothing is uploaded. **Reset counters** starts a new observation period without reconnecting or changing the displayed snapshot. The panel is absent from ordinary URLs and does not persist its mode across reloads.

The browser checks the protocol, base revision, next revision and entity/order consistency, and applies a patch atomically. Missing, malformed or out-of-sequence updates disable stale response controls and cause `{"type":"resync","protocol":1}` to be sent over the same socket. The server replies with a complete snapshot, even if the state has not changed. Repeated resync messages within one second close the connection. The browser waits up to five seconds for the replacement, then reconnects. A new connection starts with a complete independent baseline. Approval/message/navigation commands continue to use the validated HTTP API. Previous-message requests use the authenticated WebSocket as described below; both routes share the same command validation. Data diffs cannot authorize a response.

### Previous-message requests over WebSocket

The browser sends history requests on its existing synchronized WebSocket, not as a new HTTP request:

```json
{"type":"history","protocol":1,"id":"0123456789abcdef0123456789abcdef","session":"codex:example"}
```

The request ID is32 hexadecimal characters. The server revalidates the device and ingress authority immediately before dispatch, then uses the same session visibility, per-device history cursor, loading/exhaustion and history limits as the HTTP dispatcher. It never accepts a client-supplied cursor or another device's history key.

An admission result is a separate message, outside the state revision chain:

```json
{"type":"command_result","protocol":1,"id":"0123456789abcdef0123456789abcdef","accepted":true}
```

A rejected request has `error` instead of `accepted`. Admission means queued, not that a page has finished loading. Loading/result/error state still arrives as an ordinary revisioned diff. The last32 history admission replies are cached per connection; replaying one request ID for the same session returns its cached result without a second dispatch. Reusing an ID for a different session is rejected. This cache is bounded and is not a persistent idempotency guarantee across reconnects.

The browser shares one pending history request per session, shows loading while awaiting admission, and waits up to ten seconds for its result. Server enqueue is bounded to five seconds. Disconnect rejects outstanding waits and does not replay requests or fall back to HTTP. During reconnect or diff resync, a new history request is refused until the stream is synchronized. The HTTP history command remains accepted for compatibility, but the web frontend does not use it.

### WebSocket compression

The server negotiates standard RFC 7692 `permessage-deflate` using `yawc`; browsers compress/decompress automatically, and JavaScript continues to receive ordinary JSON strings. Clients not offering the extension receive the same protocol without compression. Compression uses the fast level, a 256-byte minimum outgoing message size and no context takeover in either direction. Small packets and ping/pong are sent without compression; compressed and uncompressed data packets may coexist on a negotiated connection.

The 64 KiB incoming message/frame bound applies to decompressed input as well as compressed wire bytes, including fragmented input. UTF-8 validation and the existing private-cookie, Host, Origin, device and command validation remain in force. Ping is sent every ten seconds and browser pong is automatic. With no visible state changes, only connection maintenance remains. HTTP gzip does not compress WebSocket messages; these are separate layers. Image requests and reconnect/bootstrap snapshots still consume bandwidth. No fixed reduction ratio is guaranteed for different workloads.

Sources: `src/web/server/{mod,auth,projection,updates}.rs`, `src/web/{stream,remote}.js`, `src/web/remote.css`, `src/macos/web_settings.rs`, `src/app/ipc.rs`.

### Remote tunnels

Cloudflare Quick Tunnel is implemented as the opt-in experimental mode described above. The macOS Web access switch automatically attempts the tunnel alongside the LAN listener. The low-level CLI `web start` still starts only the local listener; `web tunnel-start` manages the tunnel separately. Other providers, named Cloudflare tunnels and bundled connector installation remain [planned](web-tunnels.md).

## Verification

`cargo test --offline` covers pairing expiry/rotation/replay, cookie authorization, Host/Origin checks, stale/invalid approval rejection, keyed diff/revision behavior and actual HTTP/WebSocket traffic against an isolated server. Raw-frame tests verify negotiated compression, inflate received bytes, reject oversized inflated input, check gzip assets, suppress unchanged state and exercise resync/revocation. `node --test tests/web_stream.js` checks atomic browser-side patches, opaque keys, revision mismatch and remote-host resync handling. Native visual tests render Web settings, off/on states and QR in both themes. For an explicit hidden macOS browser check:

```sh
cargo test --offline real_browser_pairs_and_submits_over_http -- --ignored
```

That check starts an isolated fixture server and hidden WebKit view at phone width, verifies pairing, HttpOnly cookie, token removal, `permessage-deflate` negotiation, an applied live patch, previous-message loading over WebSocket (with HTTP history deliberately rejected by the check), layout and an approval submission. It does not connect to actual agents or restart the resident app. A physical phone, network firewall behavior and GNOME runtime remain separate checks.

## Usage limit reset inventory

The quota capsule's ellipsis control expands upwards and requests reset inventory only when opened. Authenticated `POST /api/command` with `{"type":"reset-credits"}` dispatches `ReadUsageResets`; it does not redeem a reset. The monitor publishes `usageResets` through the normal snapshot/diff stream (`phase`: idle/loading/ready/error; request `generation`; nullable `availableCount`, `credits`, `error`). Each detail contains a display `title` and nullable Unix-seconds `expiresAt`; opaque redemption IDs and account authentication are excluded. Multiple clients share one in-flight read; each page keeps its own disclosure state. Opening a page or receiving a snapshot never starts the read. Loading disables no unrelated session actions. Error, unavailable detail rows, zero resets and unknown expiration are shown explicitly. Offline pages and diagnostic Freeze/Repeat disable the control.

The reader opens a short-lived private Codex CLI App Server stdio connection, sends initialize/initialized and only `account/rateLimits/read` with reset details requested. It never resumes/starts a thread, uses a reset or changes Codex settings. A 25-second deadline terminates the private child connection; existing Codex clients and servers are not stopped. It requires an installed, signed-in Codex CLI supporting reset-credit fields. Provider summary count is authoritative, even when details are null or capped. See the [Codex App Server protocol](https://learn.chatgpt.com/docs/app-server) and [quota inventory UI](design/components/glass-surfaces.md#quota-reset-inventory).
