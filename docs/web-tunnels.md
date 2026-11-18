# Remote tunnel integration

Status: **Cloudflare Quick Tunnel, embedded ngrok Rust SDK and managed Tuna CLI implemented**. All three can run concurrently. The shared server, pairing, incremental WebSocket transport and compression are documented in [Web access](web-access.md); native/preview presentation is specified in [settings](design/components/settings.md#settings-tunnel-provider).

## Access paths

- The normal HTTP listener exposes the configured local address and usable interface addresses. No tunnel is required on a reachable LAN.
- Cloudflare uses a managed installed `cloudflared` child, an isolated temporary configuration and HTTP/2 transport. Quick Tunnels need no account but are intended for development/testing and have no uptime guarantee.
- ngrok uses the official Rust SDK in-process, a personal authtoken in the platform credential store, and an HTTPS endpoint forwarded to its own ephemeral loopback listener. No ngrok executable, CLI configuration or process is used.
- Tuna uses an installed official CLI with a personal API key from the platform credential store; its inspector and ambient user configuration are disabled.
- All remote origins appear with LAN/loopback in one address list. QR and Copy link use the selected address. Pairing remains single-use and expires after two minutes.

Enabling Web access through macOS Settings starts local access, discovers/connects saved ngrok and Tuna credentials and attempts Cloudflare. An unavailable connector or credential never disables the other paths. Web access is off after restarting the monitor; stored ngrok credentials persist. The Mac must be awake and reachable to its providers. Phone and Mac provider reachability are independent.

## Lifecycle and authorization contract

Each provider owns a distinct connector, private ingress, status, public origin and generation. Starting/restarting Cloudflare leaves ngrok untouched; configuring/reconnecting ngrok leaves Cloudflare untouched. A stale completion from an old generation cannot overwrite its replacement or revoke replacement devices.

Local, Cloudflare, ngrok and Tuna browser sessions have separate ingress identities. An exact HTTPS Host/Origin match is required on each provider listener, with Secure cookies and same-origin WSS CSP. No forwarded header can promote LAN traffic to remote authority. All existing command validation, history bounds, image authorization, size/device/socket limits and idempotency rules apply to both public routes.

Provider failure removes only that origin and disconnects only that provider's browsers and releases its history cursors, retaining browser authorization. Other browser sessions remain valid. Closing unconfigured startup without a public origin does not rotate a local pairing link. Explicit `web tunnel-stop` stops all public paths; `web stop`/the settings switch stops all paths and disconnects devices while preserving authorization. An ngrok SDK session is explicitly closed on cancellation, failure and stop; its forwarding task is aborted. Cloudflare children are stopped on owner drop.

The status API retains `tunnel` for Cloudflare and adds `ngrok` and `tuna`, including `status`, `url`, `error`, and a `tokenSaved` boolean. `urls` and `pairingLinks` combine all public origins and current local interfaces. No raw provider logs or credentials are included. The public hostname is accepted only from the provider's controlled status stream/SDK and validated against supported HTTPS provider domains. Custom domains are not configured by this integration.

## Credentials and setup

ngrok account onboarding uses the collapsible settings section: Get token opens the dashboard; Connect submits a new secure-field token through authenticated local IPC. The credential is stored separately from Telegram in Keychain (Ubuntu backend: Secret Service). The saved credential is represented only by “stored securely in macOS Keychain”, with Change to open a replacement input. Failed connections can reconnect with it. There is no project-wide provider token or operator backend. SDK network/account errors use static messages rather than raw errors. The CLI accepts no token argument; `web ngrok-start` uses the saved credential.

## Privacy and provider constraints

Both standard HTTP tunnel configurations terminate public TLS at the provider. Transport encryption and QR authorization do not provide end-to-end encryption against Cloudflare/ngrok. Provider billing, quotas, account requirements and browser interstitials apply independently; the UI does not embed fixed limits. Actual cellular/VPN availability must be verified from the target networks.

References: [ngrok Rust SDK](https://docs.rs/ngrok/0.19.1/ngrok/), [ngrok HTTP endpoints](https://ngrok.com/docs/gateway/endpoints/http), [Cloudflare Quick Tunnels](https://developers.cloudflare.com/tunnel/get-started/quick-tunnels/).

## Verification and remaining work

Local tests cover exact Host/Origin, provider-specific cookies, simultaneous address projection, provider-scoped revocation, stale generation protection, pairing rotation, no-credential discovery, compression, history and stale actions. Native renderer tests and the interactive HTML check cover collapsed/expanded/not-configured/connected/failed/reconnect states and themes without accessing user credentials or provider accounts.

Live ngrok connection/forwarding, cellular/VPN reachability, account quota failure, host sleep/wake, browser interstitials and runtime reconnection still require a personal account and manual end-to-end verification. Ubuntu uses the same backend but does not yet expose native token onboarding. Custom/named tunnels, credential removal and startup persistence remain separate future work; do not treat them as implemented.

Tuna setup, supported public domains, CLI requirement and live-account verification limits: [Tuna web access](web-access.md#remote-access-with-tuna). Tuna credentials are stored separately and passed through the child environment; no provider-wide shared credential is included. TLS/privacy claims for ngrok/Cloudflare do not automatically apply to Tuna. The official Tuna CLI flags were checked locally; live account forwarding remains unverified without a key.
