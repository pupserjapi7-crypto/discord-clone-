# Nova — Status

**Stand:** 13.11.2026 · **Version:** 0.2.2 · **Live:** <https://85-215-175-202.sslip.io>

Kurzfassung: Der Rust-Stack läuft live. Text-Chat, Auth, Gateway, Voice-SFU
und HTTPS sind verifiziert. Das UI-Redesign ist umgesetzt und lokal geprüft;
Screenshare-Code ist vorhanden, echte Display-Aufnahme konnte in der
Automatisierung nicht getestet werden.

---

## Fertig und verifiziert

### M0 — Rust-only-Frontend
- Dioxus 0.7 für Web (WASM) und Tauri 2 als Desktop-Hülle; eine UI-Codebasis.
- `web/` (SvelteKit) entfernt; kein Node/npm/TypeScript im Build.
- CSS als eine handgeschriebene Datei mit Tokens; Inline-SVG-Icons; keine
  externen Fonts.
- CI baut Rust-Workspace, WASM-Clippy und Tests (Run `43515447012` grün).

### M1 — Gateway, Auth, Text
- Axum-API, binäres WebSocket-Gateway (MessagePack), Sessions/Presence/Limits
  im Prozess, PostgreSQL 17 mit `tsvector`-Suche.
- Argon2id, sichere Cookies, Rate-Limits, Validierung, Prepared Statements.
- 1.000 WebSocket-Verbindungen getestet, Dispatch-p99 3,3 ms.
- HTTPS live über `https://85-215-175-202.sslip.io` (Let's Encrypt bis
  11.02.2027), WSS `/gateway` liefert 101.

### M2 — Voice
- Eigener str0m-SFU, ICE-lite, UDP 40000–40100, Opus 48 kHz/20 ms ohne
  Mixing/Transcoding.
- **UDP-Probe erfolgreich:** Antwort von `93.225.205.80:59448` auf
  VPS-Port 40000.
- **Zwei-Client-Test über HTTPS erfolgreich:** beide Teilnehmer im Kanal
  „Stream“ (Guild `247126938131369984`, Voice `247126938424971264`),
  `SFU ICE connected ... state=Completed`, `clients=2`.
- Ursache eines früheren Fehlschlags war die fehlende öffentliche IP im
  SFU-Advertisement; `NOVA_SFU_PUBLIC_IP=85.215.175.202` ist jetzt in
  `/etc/nova/nova.env` gesetzt und wird beim Start geloggt.

### M3 — UI / Design
- Neues Token-System, Dark first plus Light über `prefers-color-scheme`.
- Server-Rail, Kanalgruppen (einklappbar), gruppierte Nachrichten,
  Mitgliederpanel (ab 1024 px, umschaltbar), Voice-Bereich mit
  Mikro/Deafen/Screenshare/Auflegen, Command Palette mit `Strg/Cmd+K`,
  Modals, Toasts, leere Zustände.
- Responsive: Rail 64 px unter 900 px, Drawer unter 700 px.
- Tastaturfokus sichtbar, Klickflächen ≥ 32 px, `aria-label`/`aria-pressed`,
  `prefers-reduced-motion`.
- Lokal bei 1280×800, 600×800 sowie im Light Mode geprüft; `Strg+K` und
  `Esc` funktionieren im Browser, Palette-Suche filtert Kanäle/Aktionen.
- WASM ca. 763 KB unkomprimiert.

### M4 — Teilweise
- Screenshare-UI, ein 1080p60-Layer, Bitraten-Deckel und sauberes Beenden
  sind implementiert.
- Synthetischer 1920×1080@60-Videopfad mit zwei Teilnehmern wurde verifiziert.
- **Echte Bildschirmaufnahme** (`getDisplayMedia`) ist im automatisierten
  Chromium nicht verfügbar (`Not supported`); ein echter Chrome/Edge mit
  Display-Capture muss das abschließend prüfen.

### Release / Betrieb
- Server 0.2.2 auf dem VPS (`/opt/nova/releases/v0.2.2`), systemd-Dienst,
  Apache-Reverse-Proxy, IONOS-UDP-Regel 40000–40100 gespeichert.
- Desktop-Artefakte 0.2.1: Linux-Binary, AppImage, deb, NSIS-EXE, MSI.
- Installer sind nicht signiert; SmartScreen kann warnen. Ein echter
  Installationslauf wurde nicht getestet.

---

## Bekannte Lücken

| Lücke | Wirkung | Nächster Schritt |
|---|---|---|
| Screenshare nicht mit realem Display-Capture getestet | keine Aussage über echtes 1080p60 | manueller Test in Chrome/Edge, Werte aus `getStats()` |
| Kein TURN | Voice scheitert in Netzen mit UDP-Blockade | dokumentierte Grenze; TURN nur bei Bedarf |
| Keine Latenz-/Bitratenmessung End-to-End | Mouth-to-Ear unbekannt | Click-Track-Harness, Werte in Roadmap |
| Ungelesen-Badges, Ping, Sprecher-Ring | Design-/Feature-Lücke | M4-Ausbau |
| Threads, Moderation, Bot-API | nicht implementiert | M4 |
| Kein manueller Theme-Schalter | Light folgt nur dem System | bewusst, siehe `docs/DECISIONS.md` |
| Installer unsigniert | SmartScreen-Warnung möglich | Codesigning-Zertifikat |

---

## Nächste Schritte

1. Dieses Redesign als v0.2.2 deployen und die Live-UI damit prüfen.
2. Screenshare in einem echten Browser mit Display-Capture testen und
   `getStats()` (Framerate, Bitrate, Loss) dokumentieren.
3. Latenz-Harness für Mouth-to-Ear aufsetzen.
4. Danach M4: Threads, Moderation, Bot-API — jeweils mit Gate und Doku.

---

## Starten

Siehe [`README.md`](README.md). Lokal:

```bash
cargo run -p nova -- migrate
cargo run -p nova -- serve
```

Die UI wird separat mit `dx build --web --release --features web` gebaut und
nach `crates/ui/dist` kopiert; der Server bettet dieses Verzeichnis ein.
