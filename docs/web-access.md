# Web access

The resident monitor can serve its shared HTML frontend over HTTP. It uses the same monitor actor, bounded session activities, request validation, history reads and message delivery as the native desktop and Telegram. It does not start another Codex client. No public service or tunnel is required.

## Connect a phone

On macOS, open **Settings → Web**, enable **Web access**, and scan the QR code with your phone camera. The switch starts local access and automatically attempts a Cloudflare tunnel when `cloudflared` is installed. For a local HTTP address, the computer and phone must be reachable on the same network; select the address belonging to that network. A public HTTPS address also works outside that network. Allow incoming connections if macOS asks.

The switch starts a listener on `0.0.0.0:8765`. It is off by default and is not automatically restored after restarting the monitor. A bind failure is displayed in the pane. Turning it off stops the listener and revokes every browser session. The native monitor keeps working.

Each QR contains an address and a random, single-use pairing secret in the URL fragment. The link expires after two minutes. While Settings → Web is visible, a progress bar beneath the QR shows the remaining seconds; expiration or successful use automatically generates a replacement code. **New QR code** can also replace the secret manually, and **Copy link** copies the selected address with its valid secret. Renewal errors appear inline and retry; expired links cannot be copied. The browser exchanges it for an HttpOnly, SameSite=Strict cookie and immediately removes the fragment. Browser sessions last at most 30 days and are held in memory: stopping the server or restarting the application revokes them. The displayed browser count means paired browsers, including currently closed tabs.

This is HTTP, not encrypted HTTPS. Pairing restricts application access; it does not encrypt network traffic. Use it on a network you trust. This implementation does not configure internet port forwarding, certificates, VPNs or tunnels.

## Experimental remote access with Cloudflare

On macOS, install the official `cloudflared` connector (`brew install cloudflared`), enable **Web access** in the same pane. This single switch also attempts the tunnel; no separate Remote access control is needed. No Cloudflare account, domain or provider token is required. The monitor runs a managed Cloudflare Quick Tunnel process, obtains its `https://*.trycloudflare.com` address and selects that address for the existing pairing QR. Scan it from the phone; the phone does not need a Cloudflare app or account.

Quick Tunnels are intended by Cloudflare for development/testing and have no uptime guarantee. Creating a new tunnel changes the public hostname. Pairing-token renewal keeps the same hostname and does not restart the tunnel. `cloudflared` handles transient network reconnection; a stopped or failed process requires toggling Web access off/on (which also disconnects local browsers), or using the CLI tunnel commands. The pane distinguishes starting, connected, reconnecting and failed states. The connector uses HTTP/2 transport, an isolated empty configuration and no named-tunnel credentials. Its own process and temporary config are cleaned up on disable/server stop. The application does not install or register a login daemon.

The tunnel forwards to a separate ephemeral **loopback-only listener**. That listener accepts only the exact provider-issued HTTPS Host and Origin. The LAN listener cannot be converted to public ingress by spoofing forwarded headers. Public responses use same-origin WSS in CSP, and remote pairing sets a Secure, HttpOnly, SameSite=Strict cookie. LAN and remote browser cookies have separate authority. The CLI `web tunnel-stop` revokes remote browsers and releases their history while locally paired browsers remain connected. Turning Web access off in Settings revokes both and stops the connector. Web access is not restored automatically on monitor restart. If the connector is unavailable or fails, local access remains running and the pane displays the error.

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

A disconnected page disables submissions/message entry until it receives a fresh snapshot. WebSocket reconnect and returning to the foreground obtain current state; actions are not automatically replayed. A browser needs a new QR after revocation or server restart. The mobile layout uses the dynamic viewport and supports native screen keyboard resizing; it has no service worker or background push notifications.

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
