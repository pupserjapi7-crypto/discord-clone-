# Project Nova

Ein schlanker, latenzoptimierter Discord-Clone in **100 % Rust** — gebaut für **einen** VPS mit 2 vCore / 2 GB RAM / 90 GB NVMe.

**Prioritäten:** Voice-Latenz → Performance/Ressourcen → Design/UX → Features → Sicherheit (immer Pflicht).

## Status v0.2.2

| Phase | Zustand |
|---|---|
| M0: Architektur + Rust-only-UI | **fertig** — Dioxus 0.7 (Web-WASM + Tauri-Shell), kein Node/npm |
| M1: Gateway + Auth + Text-Chat | **live** auf `https://85-215-175-202.sslip.io`; 1.000 WS ohne Fehler, Dispatch-p99 3,3 ms |
| M2: Voice (str0m-SFU) | **implementiert** — ICE/DTLS im Zwei-Client-Test verbunden |
| M3: UI | **implementiert** — neues Clean-Design (Dark/Light), Mitgliederpanel, Command Palette, gruppierte Nachrichten, Voice-Dock mit Mute/Deafen/Screenshare |
| M4: Screenshare 1080p60 | **Code + synthetischer Video-Pfad verifiziert**; echte Display-Aufnahme im Automationsbrowser nicht testbar, Threads/Moderation/Bot-API offen |
| M5: optionale Features | nicht begonnen |

## Dokumentation

- [`docs/01-architecture.md`](docs/01-architecture.md) — Topologie, Protokoll, RAM-/Bandbreiten-Budget, Build-Pipeline, Deployment
- [`docs/02-design.md`](docs/02-design.md) — Farben, Typografie, Layout, Komponenten, Frontend-Budget
- [`docs/03-roadmap.md`](docs/03-roadmap.md) — Meilensteine, Gates, Umsetzungsstand
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — getroffene Architektur- und Designentscheidungen
- [`STATUS.md`](STATUS.md) — was fertig ist, was fehlt, bekannte Probleme
- [`docs/06-vps-production.md`](docs/06-vps-production.md) — Betrieb auf dem VPS
- [`docs/07-desktop-app.md`](docs/07-desktop-app.md) — Tauri-Shell und Installer

## Architektur in Kürze

- **Ein Binary:** `nova serve` startet Axum-HTTP, Gateway und SFU; Optionen über `/etc/nova/nova.env`.
- **PostgreSQL 17** für alles Persistente inkl. Volltextsuche (`tsvector` + GIN). Kein Redis, kein Meilisearch, kein MinIO, kein Docker.
- **Presence, Sessions, Rate-Limits** im Prozess; **Uploads** content-addressed hinter `BlobStore`.
- **Frontend:** Dioxus 0.7 (RSX), `dx build --web` → WASM, per `rust-embed` im Server-Binary. Dieselbe UI läuft in der Tauri-2-Desktop-Shell.
- **Voice:** eigener str0m-SFU, ICE-lite, UDP 40000–40100, Opus 48 kHz/20 ms, RTP-Fanout ohne Mixing/Transcoding; Screenshare als 1080p60-Schicht.

## Voraussetzungen

- Rust **1.89+** (siehe `rust-toolchain.toml`), Target `wasm32-unknown-unknown`
- Dioxus-CLI **0.7.0**: `cargo install dioxus-cli --version 0.7.0`
- PostgreSQL 17 für lokale Entwicklung
- Für Tauri-Installer: MSVC/WebView2 (Windows) bzw. WebKitGTK (Linux)
- **Kein** Node, npm, pnpm oder Bun

## Bauen

```bash
# 1. Web-UI (erzeugt crates/ui/dist)
cd crates/ui
dx build --web --release --features web
cd ../..
rm -rf crates/ui/dist
mkdir -p crates/ui/dist
cp -a target/dx/nova-ui/release/web/public/. crates/ui/dist/
cp crates/ui/assets/app.css crates/ui/dist/assets/app.css

# 2. Server (Debug lokal, Release für Deployment)
cargo build -p nova
cargo build --release --target x86_64-unknown-linux-musl -p nova

# 3. Prüfungen
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p nova-ui --target wasm32-unknown-unknown --features web -- -D warnings
cargo test --workspace
```

Der Server bettet `crates/ui/dist` ein; nach UI-Änderungen muss `crates/nova/src/main.rs` neu kompiliert werden (`touch` hilft gegen den `rust-embed`-Cache).

## Desktop-Installer

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
