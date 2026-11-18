# Shared HTML frontend

The HTML/CSS/JavaScript here serves two independent hosts:

- `design/previews/monitor-ui.html` embeds `index.html` with synthetic data from `mock-data.js`.
- The resident HTTP server embeds the same shell, renderer and CSS, using `remote.js` to obtain real snapshots and send typed API commands. See [Web access](../../docs/web-access.md).

The native AppKit/GTK windows do not host a WebView. Starting the web listener is optional and does not replace native rendering or start another agent client.

`monitor.js` owns keyed session rendering, bounded activity projection, filtering, requests, drafts, command disclosure, history and focus. `AAMMonitor.mount(root, send, options)` returns `setSnapshot`, `visibility`, `reveal`, `expanded`, `messageResult` and `destroy`. Remote options hide simulated settings and retain message drafts until delivery is confirmed. Incoming message HTML uses a small allowlist; images are restricted to authenticated `/api/image/…` URLs.

Snapshots reconcile existing DOM nodes rather than replacing session articles. Session headers, disclosure clips, latest-message containers and composers keep their identity through content/status updates; request forms are keyed by request context so a different request gets fresh inputs. Unchanged markup, counters and quota content are not rebuilt. Updating during disclosure must retain the same transition, editor, selection and scroll container. `scripts/check-web-frontend.swift` verifies these contracts at a 390 px viewport against an isolated real HTTP/WebSocket host.

The decorative mesh is rasterized once per mount into a bounded 460×2000 PNG period with alternating mirrored artwork. It repeats vertically and scrolls with the document; resizing the document does not rerun its SVG blur filters or regenerate the image. Light/dark opacity remains CSS-controlled. The original SVG remains the fallback if rasterization is unavailable. Only bundled decorative artwork enters the canvas; session text and images are never captured.

The shared composer now uses an inset 28 px upward-arrow Send control: hidden for blank drafts, vertically centered beside fitting single-line text, or in a separate bottom reservation for wrapping/newline text. The empty fill is 32 px tall. The same presentation is used by the preview and live web host without a query flag. Command blocks have no Copy control and use muted 10 px text with 6×8 px padding.

`settings.js` remains the preview-only settings host with mock General, Applications, Agents, Telegram and Web states. The single Web access switch simulates enable/disable with automatic Cloudflare connection, collapsible ngrok/Tuna setup/status sections and simultaneous Cloudflare/ngrok/Tuna addresses, address selection and a120-second renewing QR using reserved example origins; it never starts a real listener or tunnel. Telegram simulates token checking, activation, replacement and cancellation without contacting Telegram. `remote.js` owns pairing, cookie-based API access, WebSocket reconnect, browser logout and delivery feedback. `remote.css` adapts viewport/safe-area sizing. The remote host never interprets mock settings events as production commands.

The mock parent/iframe bridge still checks message source windows and uses the `aam-host` / `aam-monitor` channels. It never contacts the HTTP server. The header renders `versions.current` and optional `versions.available` from its host snapshot. Only the resident monitor owns checking, caching and comparing releases. The preview supplies synthetic version state and never queries GitHub; the remote host receives the same monitor-owned data over its existing stream. A newer available release appears as an underlined link below the current version; the frontend only emits its normal link-open command. Opening the preview directly needs no server or credentials. The HTTP host replaces the inline mock bootstrap with an external script; it does not accept parent-window snapshot injection.

`scripts/render-monitor-mesh.swift` uses the prototype's decorative SVG to produce the native bitmap asset during development. WebKit remains a development-test dependency, not a native application renderer.

Preview settings verification: run `swift scripts/check-settings-preview.swift` from the repository root on macOS. It uses a hidden nonpersistent WebKit window with synthetic data and checks all five panes, preference events, Telegram replacement/cancellation/activation, unified Web access, automatic tunnel startup, address selection, QR renewal, keyboard navigation and both themes. Set `AAM_PREVIEW_WIDTH=390` to check a narrow viewport. It does not contact the running monitor or external services.
