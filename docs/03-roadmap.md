# Project Nova — Roadmap

Schätzungen sind **Personentage für eine erfahrene Rust-Entwicklerin/einen erfahrenen Rust-Entwickler** inklusive Tests und Doku. Jeder Meilenstein endet mit Code, Tests, Messung und kurzem Entscheidungsprotokoll. Kein Meilenstein beginnt, bevor der vorherige sein Gate bestanden hat.

## Umsetzungsstand v0.2.0 (Rust-only)

| Meilenstein | Stand |
|---|---|
| **M0 Architektur/UI** | ✅ Dioxus-0.7-UI in `crates/ui` (Web-WASM + Tauri-Shell), `web/` entfernt, Docs auf Rust-only umgestellt |
| **M1 Gateway/Auth/Text** | ✅ implementiert, getestet, auf dem VPS live; 1.000 WS ohne Fehler, p99 3,3 ms |
| **M2 Voice** | ✅ eigener str0m-SFU, ICE/DTLS im Zwei-Client-Test verbunden; UDP-Freigabe des Providers und HTTPS bleiben Betriebsvoraussetzungen |
| **M3 UI** | ✅ Auth, Guilds, Kanäle, Chat, Invites, Voice-Dock, Mute, Screenshare-UI |
| **M4 Screenshare/1080p60** | ✅ Video-Pfad mit synthetischem 1920×1080@60-Track und zwei Teilnehmern verifiziert; Threads/Moderation/Bot-API noch offen |
| **M5 optional** | ⏳ nicht begonnen (bewusst nach Ressourcenmessung) |

---

## M0 — Architektur und Rust-only-Frontend (abgeschlossen)

**Lieferung:** `docs/01-architecture.md`, `docs/02-design.md`, `docs/03-roadmap.md`, `crates/ui`, Tauri-Shell.
**Gate:** erfüllt — eine UI-Codebasis, Web-Build über `dx`, Einbettung ins Server-Binary, kein Node/npm.

---

## M1 — Gateway + Auth + Text-Chat · 8–12 Tage

**Scope**

- Cargo-Workspace, CI (fmt, clippy `-D warnings`, test, audit, deny, musl-Build, Frontend-Size-Check).
- Config-Layer (TOML + Env-Override), `migrate`-Subcommand, Seed-Daten.
- Auth: Registrierung, Login, Refresh-Rotation mit Reuse-Detection, Logout, Session-Liste. Argon2id.
- Profile: Handle, Anzeigename, Avatar-Upload (Re-Encoding), Status (online/idle/dnd/offline).
- Guilds, Rollen, Permissions (Bitfield + Channel-Overrides), Invites.
- Kanäle (Text/Voice-Platzhalter), Kategorien, Sortierung.
- Nachrichten: senden, editieren, löschen, antworten, Reaktionen, Cursor-Pagination (`before`/`after`), Attachments (Upload + Auth-Download).
- Gateway: binäres Framing, HELLO/HEARTBEAT/RESUME, Fan-out mit `Bytes`, bounded Queues, Rate-Limits, Presence.
- (Suche wurde auf M3 verschoben, um M2 früher zu erreichen.)
- `/healthz`, `/readyz`, `/metrics`, strukturierte Logs.
- `loadtest` (WS): 1.000 und 2.000 Conn, Nachrichtenlast, Resume-Sturm.

**Explizit nicht in M1:** Voice, Screenshare, Threads, DMs, Command Palette, Themes, Soundboard, Bot-API, 2FA, Link-Previews, PWA-Offline-Cache.

**Gate (messbar, auf dem VPS mit laufendem PG+Caddy)**

| Kriterium | Schwelle |
|---|---|
| 1.000 WS stabil 10 min, 20 msg/s verteilt | 0 Disconnects außer geplant |
| 2.000 WS idle 5 min | RSS Gateway < 200 MB, CPU < 30 % |
| Nachrichtenlatenz lokal | p50 < 20 ms, p99 < 50 ms |
| Fehlerrate unter Last | < 0,1 % |
| RSS gesamt (ohne SFU) | < 700 MB |
| `cargo clippy -- -D warnings`, Tests | grün, keine `unwrap()` im Prod-Pfad |
| Frontend-Build | WASM+Glue < 1,5 MB gzip |

**Artefakt:** `docs/benchmarks/m1-gateway.md` mit Rohwerten und CPU/RAM-Verlauf.

---

## M2 — Voice-Prototyp (2 Teilnehmer) · 6–10 Tage, danach **hartes Gate**

**Vorgehen**

1. **Tag 1–2: str0m-Spike.** Ein Binary, zwei Browser, Audio bidirektional, keine UI. Messung der SFU-Transit-Zeit.
2. **Tag 3–6:** Signaling über Gateway (`VOICE_JOIN`, SDP-Offer/Answer, ICE-Trickle), Room-Verwaltung, Join/Leave, Mute, Sprecher-Erkennung (Audio-Level), Reconnect.
3. **Tag 7–8:** Latenz-Harness (Click-Track → Aufnahme → Kreuzkorrelation), `getStats()`-Panel im Client (Ping/Jitter/Loss/Bitrate).
4. **Tag 9–10:** 6-Teilnehmer-Test, 30-Teilnehmer-Lasttest mit synthetischen Clients, Tuning (Socket-Buffer, `sendmmsg`, CPU-Pinning prüfen).

**Gate — wenn nicht erreicht, wird auf LiveKit gewechselt (Entscheidung dokumentiert, nicht diskutiert)**

| Kriterium | Schwelle |
|---|---|
| Mouth-to-Ear lokal (2 TN) | < 100 ms, gemessen, nicht geschätzt |
| Mouth-to-Ear Internet (2 TN) | < 150 ms typisch |
| 6 TN, 10 min | keine Aussetzer > 100 ms, Loss < 1 % |
| 30 TN audio-only | CPU-SFU < 80 % von 1 Kern, RSS < 150 MB |
| Jitter-Buffer adaptiv | 20–60 ms, kein Knacksen bei 2 % Loss |
| Reconnect nach Netzwechsel | < 1,5 s, Call bleibt für andere stabil |

**Artefakt:** `docs/benchmarks/m2-voice.md` inkl. Latenz-Histogramm und Vergleich LiveKit (Stichprobe).

---

## M3 — Finale UI, Command Palette, Suche, Uploads · 10–14 Tage

- Design-Tokens, drei Themes, Layout (Desktop/Tablet/Mobile), alle Komponenten aus `02-design.md`.
- Virtualisierte Nachrichtenliste, Optimistic UI mit Nonce→ID-Ersetzung, Skeleton, Empty States.
- Command Palette (`Ctrl+K`): Server, Kanäle, Nutzer, Aktionen, Recent-Sprungliste, Fuzzy-Match im Speicher.
- Suche mit Filter-Chips und Sprung zur Trefferstelle.
- Uploads: Drag & Drop, Fortschritt, Thumbnails, Größenfehler, Bild-Re-Encoding serverseitig.
- Voice-UI im finalen Design (Sprecher-Ring, Qualitäts-Pill, Mute/Deafen, PTT, Lautstärke pro Nutzer).
- PWA: Manifest, Service Worker (App-Shell), IndexedDB-Cache (Feature 8) mit LRU-Cap.
- A11y-Pass: Tastatur, Fokus, `aria-live`, axe-Scan, Kontrastprüfung.
- Performance-Pass: Bundle-Analyse, Code-Splitting, Font-Subset, TTI-Messung.

**Gate:** Dioxus-Web-Build < 1,5 MB gzip, Tastaturbedienung vollständig, axe ohne kritische Findings, Screenshare-UI auf einem echten Chrome/Edge mit Display-Capture getestet.

---

## M4 — Screenshare, Threads, Moderation, Bot-API · 12–18 Tage

- Simulcast + Layer-Auswahl pro Subscriber, **1080p60** (8–10 Mbit/s, Cap konfigurierbar), Zuschauer-Limit 8, PLI/FIR-Drosselung.
- Threads (Nachrichten als Thread-Wurzel, eigene Pagination), Forum-Kanäle mit Tags und Sortierung.
- Moderation: Timeouts, Kick/Ban, Audit-Log, Report-System, Rate-Limits pro Kanal.
- Bot-API: REST mit Bot-Tokens, Webhook-Endpoints, Slash-Commands, Gateway-Events für Bots.
- Link-Previews: SSRF-sicherer Fetcher, Cache-Tabelle, Opt-out pro Kanal.
- Soundboard, Voice-Nachrichten (Opus + Waveform), Themes-Engine, Smart-Notifications.
- 2FA (TOTP) + Recovery-Codes.

**Gate:** Screenshare 8 Zuschauer bei < 60 % eines Kerns, kein Audio-Dropout während Screenshare; Audit-Log vollständig; Bot-API mit Referenz-Bot getestet; SSRF-Testfälle (Metadata-IP, Redirect auf localhost) blockiert.

---

## M5 — Nur nach Ressourcenmessung · separat zu bewerten

Reihenfolge nach erwartetem Nutzen/Aufwand: Watch Together → Clips → Stage-Kanäle → räumliches Audio → STT → WASM-Plugins → E2EE (Insertable Streams/MLS).

**Vorbedingung für jedes Feature:** M4-Messung zeigt ≥ 30 % Reserve bei CPU und RAM im 30-Tage-Schnitt, **und** das Feature verursacht keinen zusätzlichen Dauerdienst. Sonst: nicht bauen.

---

## Querschnitt pro Meilenstein

| Aufgabe | Wann |
|---|---|
| `cargo fmt`, `clippy -D warnings`, Tests | jeder Commit (CI) |
| Lasttest-Erweiterung | M1, M2, M4 |
| Benchmark-Dokument | am Gate |
| README-Abschnitt „Setup/Betrieb“ | am Gate |
| Sicherheitsreview (CSP, Limits, SSRF, Auth) | M1, M3, M4 |
| Backup-Restore-Test | vor Produktivstart, dann monatlich |

---

## Risiken (Top 5)

| Risiko | Wirkung | Gegenmaßnahme |
|---|---|---|
| SFU wird zum Fass ohne Boden | M2 platzt, alles verschiebt sich | 10-Tage-Gate, LiveKit-Fallback hinter Trait |
| Traffic-Limit des VPS | Voice bricht mitten im Monat weg | Budget-Tabelle, Caps, `/metrics`-Zähler, Tarif mit ≥ 5 TB |
| 2 GB RAM zu knapp bei 2.000 WS + SFU | OOM, Calls sterben | cgroup-Deckel, Queue-Limits, Messung in M1 |
| sqlx-Compile-Time-Checks ohne lokale DB | Build blockiert | `.sqlx`-Offline-Cache + CI-Prepare-Job, lokale PG-Installation |
| Screenshare-Erwartung vs. Bandbreite | Enttäuschung | 1080p60 nur mit Simulcast + 8-Zuschauer-Cap; Safari/Firefox-Grenzen dokumentiert |
