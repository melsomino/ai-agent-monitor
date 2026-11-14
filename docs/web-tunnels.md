# Remote tunnel integration plan

Status: **Cloudflare Quick Tunnel implemented experimentally; other providers planned**. Local HTTP access, pairing, incremental WebSocket updates and compression are implemented separately in [Web access](web-access.md). This document records options and the work required before a public tunnel can be enabled. Enabling Web access in macOS Settings authorizes local access and an automatic Quick Tunnel attempt through one switch. The CLI retains separate local and tunnel commands.

## Objective and constraints

Let a phone use the existing browser frontend outside the Mac's LAN, without opening router ports or requiring a public IP on the Mac. The Mac remains the agent/monitor host; it must be awake and connected. A provider outage or a VPN blocking the provider can still interrupt service.

The first implementation should use the user's own tunnel account. Do not distribute a project-wide provider token inside the application. A future centrally managed offering would need separate device provisioning, credential isolation, billing controls and an operator backend; it is outside this plan.

## Provider choices

| Option | User setup | Integration | Constraints |
|---|---|---|---|
| ngrok | Account, authtoken and assigned/configured domain | Official Rust SDK can forward to the existing loopback listener, or an explicitly configured external agent can be used first | Account quotas, HTTP interstitial on Free, HTTPS termination at the provider by default |
| Cloudflare Quick Tunnel | Installed `cloudflared`; no account/domain | Start a temporary connector and obtain its assigned origin | Development/test only; hostname changes; no uptime guarantee |
| Named Cloudflare Tunnel | Account, Cloudflare-managed domain and configured tunnel | Use a user-provided connector credential and explicit public origin | More onboarding; standard HTTP proxy terminates public HTTPS at Cloudflare |
| Tailscale Funnel | Tailscale account/client on host and Funnel configuration | Existing local service forwarded by Tailscale; phone uses a normal browser | Separate system client; Funnel is documented as beta; provider hostname and platform constraints |

Provider limits/pricing are external and change independently of this repository. Verify them at implementation time rather than embedding fixed quotas in UI text. References checked 2026-11-14: [ngrok Rust SDK](https://github.com/ngrok/ngrok-rust), [ngrok HTTP/WebSocket/TLS behavior](https://ngrok.com/docs/gateway/endpoints/http), [ngrok pricing](https://ngrok.com/pricing), [Cloudflare Quick Tunnels](https://developers.cloudflare.com/tunnel/get-started/quick-tunnels/), [named Cloudflare setup](https://developers.cloudflare.com/tunnel/get-started/), [Tailscale Funnel](https://tailscale.com/docs/features/tailscale-funnel).

## Suggested first path

1. Add an explicit external-origin mode, tested with a user-managed tunnel forwarding to a loopback-only listener. This isolates proxy/authentication behavior from SDK lifecycle issues.
2. Integrate ngrok as an optional provider through its Rust SDK, retaining the same HTTP/command/history server. Confirm current SDK/Axum compatibility, domain provisioning and plan behavior before choosing the dependency/configuration.
3. Keep direct LAN mode available without a provider account. The Web access settings switch enables all available access paths, including the tunnel. A connector failure leaves LAN access available.

Quick Tunnel is the first implemented experimental provider. ngrok and named-tunnel support remain recommendations, not activated features.

## Server contract and remaining provider work

Implemented for Quick Tunnel: separate loopback ingress, exact configured HTTPS Host/Origin, Secure remote cookies, ingress-scoped devices, remote-only revocation, scoped WSS CSP, managed child lifecycle and automatic selection of the remote QR. The items below remain the review checklist for adding another provider; account/credential setup is not part of Quick Tunnel.

- Model public origins as exact configured `(scheme, host, port)` values. A local interface address and a public HTTPS origin are different configuration concepts. Retain exact Host and Origin checks; do not replace them with `*` or accept arbitrary forwarded host headers.
- Trust forwarding metadata only on the designated tunnel ingress. Prefer a separate loopback-only listener or explicit ingress identity so a LAN client cannot spoof `X-Forwarded-Proto`/`X-Forwarded-Host`. Define the provider's forwarding contract and reject ambiguous multi-value headers.
- Generate pairing links from the accepted public origin. Keep the secret in the fragment and single-use with the existing two-minute expiry. Rotating a QR token must not restart the tunnel or change the public hostname.
- Use Secure cookies on the public HTTPS route. Keep local HTTP cookies separate and working; prevent host/origin collisions between LAN and tunnel sessions. Define whether disabling the tunnel revokes only its devices or all Web devices before implementation.
- Permit the exact same-origin WSS connection in the HTTP and document CSP. The client already selects `wss:` when loaded over HTTPS; its reconnect and diff-resync behavior must work through the provider.
- Apply the same request-context, submission and idempotency validation to public ingress. Preserve device/socket/input/time limits and the read-only image boundary. The external origin must never become an unauthenticated response route.
- Confirm negotiated WebSocket compression through the tunnel; do not assume a provider compresses or preserves extensions automatically. Measure billed traffic separately from logical JSON sizes and capture no message contents in diagnostics.
- Keep secrets out of command arguments, URLs in logs, monitor snapshots and diagnostics. Provider configuration belongs in the platform credential store, with the same behavior expected on Ubuntu's credential backend.

## Proposed settings behavior

- Use one Web access settings switch for local and available remote paths. A configured provider account does not itself start the server.
- A provider setup state accepts the user's own credential and origin/domain, with connection verification and an explicit enable action. Preserve the working configuration until a replacement succeeds.
- Show disabled, connecting, connected, reconnecting and failed states; display the exact active HTTPS origin and an actionable error without exposing credentials.
- Keep a single QR/address selection model, with clear distinction between LAN and remote addresses. Only show a remote QR after the public route has been verified. Retain the implemented countdown and automatic pairing-token rotation.
- On tunnel loss, show the unavailable state and reconnect with bounded backoff. On explicit disable/credential removal, stop publishing immediately and revoke access according to the chosen device policy. Local monitoring and Telegram continue to work.
- Never auto-open firewall/router configuration, disable a VPN, or weaken certificate validation to make a provider connect.

Quick Tunnel starts automatically from the single macOS Web access settings switch, with connection/error states and no credential inputs. Other provider-account interactions remain proposed.

## Privacy and provider trust

Standard ngrok HTTPS and Cloudflare HTTP tunnel configurations terminate TLS at their edge. This protects traffic in transit but does not hide application contents from that provider. Existing QR pairing is authorization, not content encryption. Do not describe it as end-to-end encryption.

If hiding content from the relay is required, evaluate an authenticated key-exchange and encrypted-message design separately. A web application delivered by the same intermediary also has a frontend-code trust boundary; simply encrypting API JSON does not make that boundary disappear. Tailscale Funnel has a different TLS termination arrangement; assess each provider's actual transport rather than applying one privacy statement to all of them.

## Acceptance and unresolved decisions

Before release, test from a phone on cellular, with the Mac behind NAT and its ordinary VPN both on and off. Cover provider outage, host sleep/wake, tunnel restart, expired/used QR, revocation, credentials replaced/removed, domain changes, quotas, interstitial behavior, stale answers, duplicate message delivery, image authorization, compression and forced diff resync. Verify an incorrect Host/Origin and spoofed proxy headers fail. Confirm the LAN frontend still works with remote access disabled.

Decisions remaining: initial supported provider; SDK versus external connector; supported plans; manual domain entry versus discovery; startup persistence; device revocation policy by ingress; whether provider-readable HTTP is acceptable; and who owns support/billing if a centrally managed service is later offered. Runtime verification on both macOS and Ubuntu is required before claiming cross-platform support.
