# Project Nova

Ein schlanker, latenzoptimierter Discord-Clone in **100 % Rust** — gebaut für **einen** VPS mit 2 vCore / 2 GB RAM / 90 GB NVMe.

**Prioritäten:** Voice-Latenz → Performance/Ressourcen → Design/UX → Features → Sicherheit (immer Pflicht).

## Status v0.8.0

| Bereich | Zustand |
|---|---|
| Text-Chat, Auth, Gateway | **live** (0.2.2 auf dem VPS); Integrationstests gegen echten Server, Dispatch-p99 lokal 0,98 ms (120 WS) |
| Rechte | **neu abgesichert** — Nicht-Mitglieder/Gebannte haben keine Rechte mehr (Lücke bis 0.3.0, s. `STATUS.md`) |
| Moderation | **fertig** — Rollen & Kanal-Overrides (mit Rechte-Editor), Kick, Bann, Timeout, Slowmode, Löschen/Anpinnen, Audit-Log mit UI |
| Foren & Webhooks | **neu (0.8)** — Forum-Kanäle (Beiträge als Threads), eingehende Webhooks pro Textkanal |
| Threads | **fertig** — aus Nachricht starten, Seitenleiste, Archivieren, Ungelesen-Stand |
| Direktnachrichten | **neu (0.6)** — über den Benutzernamen, live über das Gateway; im nativen Client, Web-App folgt |
| Uploads | **neu (0.8)** — „+“, Ziehen & Ablegen, Einfügen (Web) bzw. Dateidialog (nativ); Bilder inline ohne EXIF/GPS, Dateien als Download; content-addressed `BlobStore`, Quota pro Server, Backup im Installer |
| Bot-API | **fertig** — `nb_`-Tokens für REST + Gateway, Doku `docs/04-bot-api.md`, Beispiel `examples/echo-bot` |
| Web-UI | **neues Nova-Design** — Tokens `--nv-*`, Dark/Light/System, Command Palette, Voice-Grid, Einstellungen, Galerie `/dev/gallery` |
| Voice | Verbindung, Sprecher-Ring, Lautstärke pro Person, Push-to-Talk, Ping, Reconnect, `voice_state_update` statt Polling; **Ende-zu-Ende-Latenz noch nicht gemessen** |
| Screenshare & Kamera | Web: Simulcast (3 Schichten, Empfänger wählt), Qualitätsstufen bis 1440p, Ton mitsenden, Kamera 720p; nativ: Bildschirm 1080p30 (H.264/OpenH264, Windows + Linux/X11); max. 8 Videospuren pro Raum |
| Desktop | Tauri-2-Hülle (Tray, native Benachrichtigungen, optionaler Autostart, CSP) + nativer iced-Client mit In-App-Update |
| Offen | Latenzmessung mit echten Geräten, Kamera im nativen Client – siehe `STATUS.md` |

Screenshots: [`docs/screenshots/`](docs/screenshots/).

## Dokumentation

- [`CLAUDE.md`](CLAUDE.md) — Regeln, Crates, Befehle; Skills unter `.claude/skills/`
- [`docs/01-architecture.md`](docs/01-architecture.md) — Topologie, Protokoll, RAM-/Bandbreiten-Budget
- [`docs/02-design.md`](docs/02-design.md) — Nova-Designsystem
- [`docs/03-roadmap.md`](docs/03-roadmap.md) — Meilensteine und Messwerte
- [`docs/04-bot-api.md`](docs/04-bot-api.md) — Bot-API
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — Entscheidungen
- [`STATUS.md`](STATUS.md) — was fertig ist, was fehlt, bekannte Probleme
- [`docs/06-vps-production.md`](docs/06-vps-production.md) — Betrieb und Update auf dem VPS
- [`docs/07-desktop-app.md`](docs/07-desktop-app.md) — Desktop-Apps und Installer

## Architektur in Kürze

- **Ein Binary:** `nova serve` startet Axum-HTTP, Gateway und SFU; Optionen über `/etc/nova/nova.env`; Migrationen laufen beim Start.
- **PostgreSQL 17** für alles Persistente. Kein Redis, kein Docker. Volltextsuche ist noch nicht gebaut (die Palette sucht in geladenen Nachrichten).
- **Presence, Sessions, Rate-Limits** im Prozess.
- **Frontend:** Dioxus 0.7 (RSX) → WASM, per `rust-embed` im Server-Binary; dieselbe UI in der Tauri-2-Hülle. Zusätzlich nativer Client `crates/client-native`.
- **Voice:** eigener str0m-SFU, ICE-lite, UDP 40000–40100, Opus, RTP-Fanout ohne Mixing/Transcoding.

## Voraussetzungen

- Rust **1.89+** (siehe `rust-toolchain.toml`), Target `wasm32-unknown-unknown`
- Dioxus-CLI **0.7.0**: `cargo install dioxus-cli --version 0.7.0`
- PostgreSQL 17 (die sqlx-Makros prüfen Queries gegen die echte DB: `DATABASE_URL` setzen, `nova migrate`)
- **Kein** Node, npm, pnpm oder Bun

## Bauen und prüfen

```bash
cargo xtask ui      # Web-UI bauen → crates/ui/dist (+ app.css)
cargo xtask build   # ui + Server (release)
cargo xtask check   # fmt, clippy (Workspace + WASM), Tests
cargo xtask gallery # Komponenten-Galerie unter /dev/gallery (nur lokal!)
```

Release für den VPS: `cargo build --release --target x86_64-unknown-linux-musl -p nova`.
## Desktop-Installer

**Nativer Client** (iced, mit Streams): Installer pro Benutzer, ohne Adminrechte.

```powershell
cargo install cargo-packager --version 0.11.8 --locked
cargo build --release -p nova-desktop
cargo packager -p nova-desktop --release --out-dir "$PWD\target\packager" --binaries-dir "$PWD\target\release"
```

Ergebnis: `target/packager/nova-desktop_<version>_x64-setup.exe`; im Release als
`nova-desktop-windows-x86_64-setup.exe`. Der Update-Knopf im Client tauscht
danach die installierte exe aus.

**Tauri-Hülle** (WebView):

```powershell
cargo install tauri-cli --version "^2"
cd desktop
cargo tauri build --bundles nsis,msi
```

Ergebnis: `desktop/src-tauri/target/release/bundle/nsis/*.exe` und `bundle/msi/*.msi`. Linux: `--bundles deb,appimage`. Die CI baut alle Pakete bei einem Tag `v*` über `.github/workflows/release.yml`.

## Lokal starten

```bash
cp .env.example .env            # Werte anpassen
cargo run -p nova -- migrate
cargo run -p nova -- serve
```

Standard: `http://127.0.0.1:8080`. Voice im Browser benötigt einen **Secure Context** (HTTPS oder `localhost`) und erreichbare **UDP-Ports 40000–40100**.
