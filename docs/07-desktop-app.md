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
| Installer | `Nova_<version>_x64-setup.exe` (NSIS, per-user, no admin), MSI; Linux deb + AppImage |
| Bundle id | `dev.nova.desktop` |
| Tray | „Nova anzeigen“, „Mit dem System starten“, „Beenden“; Linksklick zeigt das Fenster |
| Schließen | legt das Fenster in den Tray; beendet wird über das Tray-Menü |
| Benachrichtigungen | native (Windows-Toast, libnotify) über `tauri-plugin-notification` |
| Autostart | aus, bis man ihn im Tray einschaltet; startet dann versteckt im Tray (`--minimized`) |
| Zweiter Start | holt das laufende Fenster nach vorn (`tauri-plugin-single-instance`) |

## Native Funktionen (ab 0.8)

- **Benachrichtigungen ohne eigenen JS-Code:** Das Notification-Plugin ersetzt
  in der Webview `window.Notification`; die Web-App ruft weiter die
  Browser-API auf (`crates/ui/src/settings.rs`). Die Server-URL bekommt zur
  Laufzeit eine Capability mit genau drei Rechten
  (`notification:allow-is-permission-granted`, `…-request-permission`,
  `…-notify`) – sonst kann die geladene Seite nichts von Tauri aufrufen.
- **CSP:** Für die geladene Seite gilt die CSP, die der Server schickt
  (`crates/nova/src/main.rs`, `content_security_policy`): `script-src 'self'
  'wasm-unsafe-eval'`, keine fremden Verbindungen außer dem eigenen Gateway,
  `frame-ancestors 'none'`. `tauri.conf.json` setzt dieselbe Richtlinie für
  lokal ausgelieferte Seiten statt `null`.
- **Installer:** Die NSIS-Deinstallation entfernt den Autostart-Eintrag
  (`windows/hooks.nsh`, `HKCU\…\Run\Nova`). Das deb-Paket hängt von
  `libayatana-appindicator3-1` ab (Tray unter Linux); ins AppImage wird die
  Bibliothek gepackt. Die MSI-Deinstallation entfernt den Autostart-Eintrag
  noch nicht – vorher im Tray ausschalten.

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

Output: `desktop/src-tauri/target/release/bundle/nsis/Nova_<version>_x64-setup.exe` und `bundle/msi/Nova_<version>_x64_en-US.msi`.

Linux (deb, AppImage):

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf libayatana-appindicator3-dev
cd desktop && cargo tauri build --bundles deb,appimage
```
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
