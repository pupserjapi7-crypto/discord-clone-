# Project Nova

Ein schlanker, latenzoptimierter Discord-Clone in Rust — gebaut für **einen** VPS mit 2 vCore / 2 GB RAM / 90 GB NVMe.

**Prioritäten:** Voice-Latenz → Performance/Ressourcen → Design/UX → Features → Sicherheit (immer Pflicht).

## Status

| Phase | Zustand |
|---|---|
| Schritt 1: Architektur, RAM-Budget, Roadmap, Designsprache | **fertig — wartet auf Freigabe** |
| M1: Gateway + Auth + Text-Chat | nicht begonnen |
| M2: Voice-Prototyp (str0m, Latenz-Gate) | nicht begonnen |
| M3: Finale UI, Command Palette, Suche, Uploads | nicht begonnen |
| M4: Screenshare, Threads, Moderation, Bot-API | nicht begonnen |
| M5: optionale Features (nur nach Messung) | nicht begonnen |

Es existiert **noch kein Produktivcode**. Dieses Repository enthält derzeit ausschließlich die Entscheidungsgrundlage.

## Dokumentation

- [`docs/01-architecture.md`](docs/01-architecture.md) — Topologie, Prozessmodell, Protokoll, RAM-/Bandbreiten-Budget, Deployment, Sicherheit, LiveKit-Alternative
- [`docs/02-design.md`](docs/02-design.md) — Farben, Typografie, Layout, Komponenten, Motion, A11y, Frontend-Budget
- [`docs/03-roadmap.md`](docs/03-roadmap.md) — Meilensteine, messbare Gates, Risiken

## Kurzfassung der Architektur

- **Ein Binary, drei systemd-Units:** `nova-api` (Axum), `nova-gateway` (WebSocket, binär/MessagePack), `nova-sfu` (str0m, ICE-lite, UDP 40000–40100).
- **PostgreSQL 17** für alles Persistente inkl. Volltextsuche (`tsvector` + GIN). Kein Redis, kein Meilisearch, kein MinIO, kein Docker auf dem Server.
- **Presence, Sessions, Rate-Limits** im Prozess (`DashMap`), **Uploads** content-addressed im Dateisystem hinter einer `BlobStore`-Abstraktion.
- **Frontend:** SvelteKit + TypeScript + Tailwind, statisch gebaut und ins Binary eingebettet.
- **Voice:** eigener SFU ohne Mixing/Transcoding, 48 kHz Opus, 20-ms-Frames, adaptiver Jitter-Buffer, 1080p60-Screenshare (8–10 Mbit/s, Simulcast, Cap 8). Fallback auf LiveKit ist als Entscheidungstor in M2 definiert.

## Voraussetzungen (lokale Entwicklung)

Auf diesem Rechner vorhanden: Node 26 / npm 12. **Fehlt noch:** Rust-Toolchain (≥ 1.89, str0m-MSRV) und PostgreSQL 17. Beides wird zu Beginn von M1 installiert; der Server-Build läuft ohnehin in CI (`x86_64-unknown-linux-musl`).

Geplant: `cargo test`, `cargo clippy -- -D warnings`, `cargo sqlx prepare` (mit lokaler DB), `npm run dev` in `web/`.

## Nächster Schritt

Freigabe von Schritt 1 (offene Punkte in [`docs/01-architecture.md`](docs/01-architecture.md#22-offene-punkte-für-dein-ok)), danach Start von M1.
