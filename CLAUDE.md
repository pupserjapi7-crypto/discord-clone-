# Project Nova — Hinweise für Claude

Discord-ähnlicher Chat in **100 % Rust** für einen VPS (2 vCore, 2 GB RAM).
Prioritäten: Voice-Latenz → Performance/Ressourcen → Design/UX → Features.
Sicherheit gilt immer.

## Harte Regeln

- Kein Node, npm, pnpm, Bun, TypeScript, Tailwind oder JS-Framework. Browser-APIs
  nur über `web-sys`/`wasm-bindgen`. Unvermeidbares JS-Glue < 50 Zeilen und in
  `docs/DECISIONS.md` begründet (derzeit: keines).
- Architektur bleibt: Dioxus 0.7 (Web-WASM), Tauri 2 als Desktop-Hülle,
  zusätzlich der native iced-Client `crates/client-native`; **ein** Server-Binary
  `nova`; PostgreSQL 17; kein Redis/Docker; Uploads hinter `BlobStore`;
  str0m-SFU mit ICE-lite auf UDP 40000–40100.
- Migrationen nur vorwärtskompatibel (neue Datei in `crates/db/migrations/`,
  nur `add column … default`, neue Tabellen, neue Indizes). Nie Spalten löschen
  oder umbenennen, solange ausgelieferte Clients sie lesen.
- Keine Secrets im Repo. Kommentare und Doku auf Deutsch, Bezeichner auf Englisch.
- Kein `unwrap()` in Server-Pfaden ohne Kommentar, warum es nicht fehlschlagen kann.

## Crates

| Crate | Inhalt |
|---|---|
| `crates/common` (`nova-common`) | Config, Snowflake-IDs, Permissions-Bitfield, Wire-Protokoll (`protocol.rs`), Tokens, Validierung, Metriken |
| `crates/db` (`nova-db`) | sqlx-Repository (`repo.rs`), Modelle, Views, Migrationen |
| `crates/api` (`nova-api`) | Axum-REST unter `/api/v1`, Auth-Extractor, Rate-Limiter |
| `crates/gateway` (`nova-gateway`) | binäres WebSocket-Gateway `/gateway`, Fan-out, Resume-Ring |
| `crates/voice-sfu` (`nova-sfu`) | str0m-SFU, ein Thread + ein UDP-Port pro Raum |
| `crates/nova` | das Binary: `serve`, `migrate`, `seed`, `doctor`, `update`; bettet `crates/ui/dist` per `rust-embed` ein |
| `crates/ui` (`nova-ui`) | Dioxus-Web-UI, Styles in `assets/app.css` |
| `crates/client-native` (`nova-desktop`) | nativer Desktop-Client (iced + tiny-skia) mit Self-Update |
| `crates/loadtest` | WebSocket-Lasttest |
| `xtask` | Build-Kette in Rust (`cargo xtask …`) |
| `desktop/src-tauri` | Tauri-2-Hülle (eigener Workspace) |

## Befehle

```bash
cargo xtask ui        # dx build → crates/ui/dist → app.css kopieren
cargo xtask build     # ui + Server (release)
cargo xtask check     # fmt, clippy (Workspace + WASM), Tests
cargo run -p nova -- migrate
cargo run -p nova -- serve
```

Die sqlx-Makros prüfen gegen eine echte Datenbank: `DATABASE_URL` muss gesetzt
sein und alle Migrationen müssen angewendet sein (`nova migrate`).

**rust-embed-Falle:** Nach UI-Änderungen muss das Server-Binary neu
kompiliert werden. Wenn Cargo das nicht merkt: `touch crates/nova/src/main.rs`.

### Diese Windows-Maschine

Toolchain liegt in `%LOCALAPPDATA%\nova-tc` (mingw-Linker verträgt keine
Leerzeichen im Pfad). Umgebung in PowerShell:

```powershell
$tc="$env:LOCALAPPDATA\nova-tc"; $env:CARGO_HOME="$tc\cargo"; $env:RUSTUP_HOME="$tc\rustup"
$env:PATH="$tc\cargo\bin;$tc\mingw64\bin;$env:PATH"; $env:TMP="$PWD\.dev\tmp"; $env:TEMP=$env:TMP
$env:DATABASE_URL="postgres://nova@127.0.0.1:55432/nova"
```

Lokales Postgres: `.dev\pg\pgsql\bin\pg_ctl.exe start -D .dev\pgdata -o "-p 55432"`
(per `Start-Process`, sonst blockiert die geerbte Pipe). `.dev/check-all.ps1`
fährt den kompletten Lauf.

## Skills

Unter `.claude/skills/` — vor passender Arbeit laden, nach Lerneffekten verbessern:

- `nova-design-system` – jede UI-Änderung
- `nova-ui-component` – Dioxus-Komponenten anlegen/ändern
- `nova-ui-build` – nach jeder UI-Änderung
- `nova-protocol` – Gateway-Events, API-Endpunkte, gemeinsame Typen
- `nova-voice` – SFU, WebRTC, Audio, Screenshare
- `nova-security` – Auth, Rechte, Uploads, Moderation, Bot-API
- `nova-release-check` – Ende jedes Arbeitspakets
