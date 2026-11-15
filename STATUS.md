# Nova — Status

**Stand:** 15.11.2026 · **Version:** 0.6.0 (Direktnachrichten, Hover-Zeichenfehler behoben)
**Live:** <https://85-215-175-202.sslip.io> (läuft noch mit 0.2.2 – Update-Befehle in `docs/06-vps-production.md`)

Fortsetzen: „mach weiter laut STATUS.md“ – der Abschnitt *Offen* ist die
Arbeitsliste, in dieser Reihenfolge.

---

## Bestandsaufnahme (Abschnitt 0)

Lauf am 14.11.2026: `dx build`, fmt, clippy (Workspace + WASM), Tests und
Server-Release-Build waren vor den Änderungen **grün**.

| Behauptung (alt) | Wirklichkeit |
|---|---|
| Voice funktioniert | Nur ICE/DTLS im Zwei-Client-Test; Audio beim Gegenüber und Mouth-to-Ear **nicht gemessen** |
| Screenshare 1080p60 | Nur synthetischer Track; echte Display-Aufnahme nie getestet |
| Uploads hinter `BlobStore` | **Gab es nicht** (nur Tabelle + URL-Schema) – README korrigiert |
| Volltextsuche (`tsvector`) | **Gab es nicht** – README korrigiert |
| Installer funktionieren | Gebaut, nie installiert, unsigniert |

### Gefundene und behobene Fehler

- **Sicherheit:** `effective_permissions` prüfte keine Mitgliedschaft – über das
  Gateway konnte jeder mit bekannter Kanal-ID fremde Kanäle abonnieren und
  beschreiben. Behoben, Regressionstest `gateway_rejects_non_members`.
- REST lieferte `role_ids` als 64-Bit-Zahlen → Web-UI konnte Guild-Details
  nicht lesen. `json_ids` behandelt jetzt auch `*_ids`, Test ergänzt.
- Web-UI: Enter schickte weder Nachrichten noch Formulare ab; Dialoge ohne
  Fokus; ganzer State bei jedem Render geklont; Auto-Scroll riss nach unten;
  eigene Nachrichten über den Text statt die Nonce zugeordnet; Passwortregel 8
  statt 10; Zeitstempel in UTC; Voice-Teilnehmer per 4-s-Polling;
  Screenshare-Stopp ließ eingefrorenes Bild; fremde Video-Elemente blieben
  stehen. Alles behoben im Neubau der UI.
- Abläufe aus Dialogen brachen beim Schließen ab (Dioxus-`spawn` gehört dem
  Dialog) → `spawn_forever`.
- CI wandte nur `0001_init.sql` an; `crates/ui/dist` sammelte alte WASM-Dateien.

---

## Fertig in 0.4.0

### P1 Stabilisieren
- `cargo xtask ui|gallery|build|check|all` (Rust statt Shell), README gekürzt.
- 7 Integrationstests gegen echten Server (Auth/Rotation, Rechte, CRUD, Pins,
  Threads, Moderation, Bots, Gateway-Roundtrip, Gateway-Abweisung), dazu
  Protokoll-Kompatibilitätstests. `CLAUDE.md` und 7 Skills.

### P2 Voice (soweit ohne echte Geräte prüfbar)
- SFU: Stream-IDs `u{user_id}`, max. 8 Videospuren/Raum, `voice/state`,
  `voice_state_update` statt Polling, Beitritt nur mit `CONNECT`.
- Client: Sprecher-Ring (AnalyserNode), Lautstärke pro Person, Push-to-Talk
  mit frei wählbarer Taste, Ping aus `getStats`, Reconnect bei ICE-Ausfall,
  Mikrofonauswahl, Rauschunterdrückung umschaltbar, Taubschalten schaltet auch
  stumm, Screenshare-Stopp nimmt die Spur aus der Verbindung.

### Native Sprache (nach 0.4.0, im nativen Client)
- `crates/client-native/src/voice.rs`: str0m als Gegenstelle zum SFU, cpal für
  Mikrofon/Lautsprecher, Opus über `unsafe-libopus` (reines Rust, keine
  C-Toolchain), 20-ms-Frames, Jitter-Puffer 40–200 ms pro Person, Mischen,
  Sprecher-Ring über Stream-ID `u{user_id}`, Neuverhandlung über den
  DataChannel, Stumm/Taub/Auflegen, Teilnehmer per `voice_state_update`.
- **Getestet** (14.11.2026, lokal, zwei native Clients, SFU auf 127.0.0.1):
  beide ICE-verbunden, Audio in beide Richtungen (~50 Pakete/s je Richtung,
  > 7.000 Pakete ohne Abbruch), Sprecher-Ring zeigt die richtige Person.
  Stummschalten konnte die Testautomation nicht klicken (Windows sperrte den
  Fensterfokus) – bitte manuell prüfen.
- Nicht gemessen: Mouth-to-Ear-Latenz.

### Nativer Client vollständig (0.4.1)
- Server erstellen, mit Einladungscode beitreten, Einladung erzeugen und
  kopieren, Text-/Sprachkanäle anlegen (mit `MANAGE_CHANNELS`).
- Nachrichten: bearbeiten, löschen, Reaktionen (Schnellauswahl + Pillen),
  anpinnen/lösen, Pins-Liste, ältere Nachrichten laden, „… schreibt“,
  Ungelesen-Punkte inkl. Lesestand (`ReadAck`), Slowmode-Hinweis.
- Threads: unter dem Elternkanal, aus Nachricht starten, archivieren.
- Moderation in der Profilkarte: Rollen vergeben/entziehen, Timeout
  (10 min/1 h/1 Tag/aufheben), Kicken und Bannen mit Bestätigung.
- Einstellungen: Anzeigename, Mikrofon/Lautsprecher, Push-to-Talk mit frei
  wählbarer Taste (gilt bei Fenster im Vordergrund), automatisches
  Neuverbinden der Sprache (3 Versuche).
- Live: `guild_update`, `guild_member_*`, `presence_update`, `message_pin`,
  Reaktionen, Tippen.
- **Getestet** (15.11.2026, lokaler Server, zwei Konten): Laden, Pins,
  Threads, Ungelesen, Live-Kanäle und -Nachrichten, alle Dialoge per
  Screenshot (Debug-Haken `NOVA_DEV_MODAL`, `NOVA_DEV_TOOLS`). Bearbeiten,
  Reagieren und Moderationsknöpfe nicht per Klick geprüft (die Testautomation
  liefert keine Mausbewegungen) – bitte kurz manuell prüfen.

### Streams im nativen Client (0.5.0)
- Bildschirm teilen (Windows: GDI, Linux: X11) mit H.264/OpenH264, 1080p30,
  Videospur per Neuverhandlung über den DataChannel; Status „Du teilst …
  · fps“, Abbruch mit Fehlermeldung (z. B. Wayland).
- Streams ansehen: eigener Dekodier-Thread, Bühne mit Name/Auflösung,
  Vollbild (Esc beendet), LIVE-Etikett in Kacheln und Kanalliste,
  Schlüsselbild-Anforderung bei Lücken.
- SFU: Video nur noch H.264 Constrained Baseline (siehe DECISIONS).
- **Getestet** (15.11.2026, lokal): nativ → nativ (1080p, 30 fps), nativ →
  Chromium (1080p, 31 fps dekodiert), Chromium → nativ (Canvas-Spur, da
  `getDisplayMedia` im Testbrowser gesperrt). **Nicht getestet:** echtes
  Teilen aus der Web-App, Linux/X11-Aufnahme auf echter Hardware,
  Ende-zu-Ende-Latenz.
- Windows-Installer für den nativen Client (`cargo-packager`, NSIS, pro
  Benutzer nach `%LOCALAPPDATA%`, damit der Update-Knopf ohne Adminrechte
  funktioniert). Release-Asset `nova-desktop-windows-x86_64-setup.exe`.
  Lokal gebaut (6 MB); eine stille Probeinstallation ließ die Testumgebung
  nicht zu – **Installation auf echtem Rechner noch ungetestet**.

### Direktnachrichten und Hover-Fehler (0.6.0)
- Direktnachrichten zwischen zwei Personen: Migration `0003_direct_messages`
  (neue Tabellen `dm_channels`, `dm_messages`), REST `GET/POST
  /users/@me/dms` (öffnen über den Benutzernamen), `GET/POST
  /dms/{id}/messages`, Gateway-Event `dm_message_create` an alle Sessions
  beider Beteiligten (`Gateway::publish_user`). Nur Beteiligte sehen einen
  Kanal (sonst 404); Bots können keine DMs nutzen; Öffnen und Senden sind
  rate-limitiert. Integrationstest
  `direct_messages_open_send_list_and_stay_private`.
- Nativer Client: Knopf „Direktnachrichten“ oben in der Serverleiste,
  Benutzername eingeben → Unterhaltung, Liste mit Ungelesen-Punkt, Verlauf
  mit „Ältere laden“, „Nachricht senden“ in der Profilkarte.
- **Getestet** (15.11.2026, lokal, zwei native Clients): REST → Client live,
  Client → Client live, kein Doppel-Eintrag beim Absender.
- **Nicht gebaut:** DMs in der Web-App (sie ignoriert das neue Event),
  Bearbeiten/Löschen/Reaktionen in DMs, Blockieren.
- Hover-Zeichenfehler im nativen Client (schwarze Flächen beim Überfahren):
  **Ursache:** iced_tiny_skia 0.14.1 malt Schatten ohne Clip-Maske über das
  ganze Schattenrechteck, auch bei Teil-Neuzeichnungen. Jeder Hover in einem
  Dialog (oder neben einem Tooltip) legte den halbtransparenten Schatten
  erneut über die unveränderten Pixel; nach ein paar Durchgängen war die
  Dialogkarte schwarz. Nachgestellt im Einstellungsdialog: Kartenfarbe
  RGB(30,32,38) → (12,13,17) nach vier Hover-Runden. **Fix:** keine Schatten
  im nativen Client (`style::NO_SHADOW`, Test `styles_have_no_shadows`);
  danach nach acht Runden kein Pixel verändert. Der erste Versuch (jede
  Mausaktion durch `update` schicken) half nicht und ist wieder entfernt.

### P3 Features
- Moderation: Rollen-Editor, Rollen zuweisen, Kanal-Overrides (API), Kick,
  Bann/Entbannen, Timeout, Slowmode, Löschen/Anpinnen, Audit-Log – mit
  Hierarchie-Prüfung, UI in Profilkarte und Server-Einstellungen.
- Threads: aus Nachricht starten, Seitenleiste, Archivieren, Ungelesen-Stand.
- Bot-API + `docs/04-bot-api.md` + `examples/echo-bot`.

### P4 Design
- Nova-Tokens `--nv-*`, Dark/Light/System, Layout 72/248/flex/248, Pill-Marker,
  Voice-Grid, Dock-Pill, Palette 560 px, Erwähnungen, „Neu“-Trenner,
  Lucide-Icons. Galerie `/dev/gallery` (nur Dev-Build). Screenshots:
  `docs/screenshots/` (Chat, Thread, Voice, Screenshare, Palette,
  Einstellungen, Moderation, Profilkarte, Login, Onboarding).
- Design mit dem Plugin „UI Design Review“ geprüft, Funde behoben.

### P5 Komfort
- Einstellungen: Profil, Audio, Theme, Schriftgröße, Tastenkürzel,
  Benachrichtigungen bei Erwähnung (Browser-API).
- Skeletons, Fehlerzustände mit „Erneut versuchen“, Offline-Banner,
  fehlgeschlagene Nachrichten mit „Erneut senden“, Onboarding.
- Nachrichten bearbeiten, Reaktionen, Emoji-Auswahl, ältere Nachrichten laden.

### Messwerte

Siehe `docs/03-roadmap.md`: Dispatch-p99 0,98 ms, Server-RAM 11,5 MB Leerlauf,
WASM 501 KB gzip.

---

## Offen

1. **Voice messen:** Ende-zu-Ende-Latenz und SFU-Weiterleitung mit zwei echten
   Geräten (Click-Track-Harness), 30-Nutzer-Test. Ohne Mikrofon nicht
   automatisierbar.
2. **Uploads:** `BlobStore` (content-addressed, SHA-256), Upload-Endpunkt mit
   Größenlimit, Download nur für Kanalmitglieder, „+“ im Composer.
3. **Tauri:** CSP statt `null`, Tray (Anzeigen/Beenden), native
   Benachrichtigungen, Autostart optional. Lokal nicht baubar ohne MSVC/WebView2-SDK;
   die CI baut die Installer.
4. **Screenshare:** echtes Simulcast mit Layer-Wahl, Qualitätsstufen,
   Systemaudio; Kamera.
5. Rollen-UI für Kanal-Overrides (API vorhanden), Forum-Kanäle, Webhooks.
6. Inter/JetBrains Mono lokal einbetten (Entscheidung offen, s. DECISIONS).
7. P6 (Custom-Emojis, Soundboard, Status-Texte) erst nach Voice-Messung.

---

## Starten

```bash
cargo run -p nova -- migrate
cargo run -p nova -- serve
cargo xtask check
```
