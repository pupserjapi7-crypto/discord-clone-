# Nova desktop app (Tauri 2)

The desktop app is a native window around the same client that runs in the
browser. It is a Rust binary using the system WebView (WebView2 on Windows,
WebKitGTK on Linux); no Electron and no bundled Chromium.

| Item | Value |
| --- | --- |
| Source | `desktop/src-tauri` (excluded from the server workspace) |
| Window | 1280×800, minimum 960×600, title “Nova” |
| Loads | `https://85-215-175-202.sslip.io` by default |
| Override | `NOVA_DESKTOP_URL=https://… nova.exe` |
| Installer | `Nova_<version>_x64-setup.exe` (NSIS, per-user, no admin) |
| Bundle id | `dev.nova.desktop` |

## Why the window loads the server URL

The web client authenticates with same-origin cookies and opens a WebSocket at
`/gateway`. Loading the public origin directly keeps cookies, CORS, session
refresh and the gateway handshake identical to the browser build. Bundling the
static UI inside the app would make every `/api` call cross-origin and break
the `SameSite` refresh cookie over plain HTTP.

The trade-off: the app needs a network connection and the server URL is fixed at
build time (override with `NOVA_DESKTOP_URL`).

## Build locally (Windows, MSVC toolchain)

Die Tauri-CLI wird als Rust-Binary installiert; es wird **kein npm** benötigt.

```powershell
cargo install tauri-cli --version "^2"
cd desktop
cargo tauri build --bundles nsis,msi
```

Output: `desktop/src-tauri/target/release/bundle/nsis/Nova_0.2.1_x64-setup.exe` und `bundle/msi/Nova_0.2.1_x64_en-US.msi`.
The GitHub release workflow builds this automatically on `windows-latest` and
attaches the installer to the release tag.

## Not configured yet

- Code signing / SmartScreen reputation: the installer is unsigned, so Windows
  may show a warning.
- Tauri auto-updater: requires a signing keypair and a hosted update manifest.
  Until then, install a newer release over the old one.
- Voice/screenshare: WebView2/WebKitGTK support WebRTC; the default origin is
  HTTPS, so microphone and display capture get a secure context. The SFU still
  needs inbound UDP 40000–40100 on the VPS firewall.
