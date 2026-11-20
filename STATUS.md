# Nova — Status

**Stand:** 20.11.2026 · **Version:** 0.8.0 (Uploads)
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

### Design-Durchgang nach „frontend-design“ (0.7.0)
- Keine Versalien-Labels mehr (Textkanäle, Online, Profil …), Anzahlen leise
  neben dem Label statt „ONLINE — 2“, keine „A · B“-Metazeilen.
- Texte aus Nutzersicht: „Benutzername“ statt „Handle“, Hinweise als Sätze,
  Technik-Fußzeilen („Nativ · Rust“) entfernt.
- Nativer Client verlangte bei der Registrierung 8 Zeichen Passwort, der
  Server 10: Hinweis und Prüfung angeglichen.

### Design „Funkraum“ (0.7.0, 15.11.2026)
- Neues Erscheinungsbild für Web und nativen Client, beschrieben in
  `docs/02-design.md` und im Skill `nova-design-system`: bläulich-tintige
  Flächen, **Nova-Gelb** nur für live/aktiv/ausgewählt/Primärknopf, Violett
  komplett entfernt (auch Logo, Fokusring, Erwähnungen, App-Icons).
- Schrift **Atkinson Hyperlegible Next** (OFL) statt Inter: Web eine variable
  woff2 (lateinische Teilmenge, 400–700) mit **34 KB**, selbst ausgeliefert;
  nativ dieselbe Schrift als TTF (2 × 46 KB, iced liest kein woff2).
- Status über Form (online Punkt, abwesend Halbmond, nicht stören Querbalken,
  offline Ring). Server-Icons immer abgerundete Quadrate mit gelbem Ring statt
  Kreis-zu-Quadrat und Pill-Marker; ausgewählter Kanal mit Streifen.
- Signature Voice: Avatar-Reihe unter belegten Sprachkanälen (Sprecher mit
  gelbem Ring, rückt hoch), Kacheln mit gelbem Namensbalken beim Sprechen,
  Kacheln erscheinen einmal nacheinander direkt nach dem Beitreten.
- Login: Formularspalte links, statischer Lounge-Ausschnitt rechts (≥ 900 px).
- Nativer Client nutzt exakt die Dark-Tokens; neuer Test
  `tokens_match_web_css` liest `app.css` und bricht bei Abweichungen.
- Messwerte: WASM gzip **502 KB** (vorher 500 KB, +2,9 KB), roh 1.281 KB;
  Schrift 34 KB als eigene, cachebare Datei. 20 Screenshots neu (alle
  Bildschirme in Dark und Light). Nativer Client: 35 MB Working Set nach
  10 s (vorher ~32 MB), erste Ansicht nach 20 ms.

### Uploads (0.8.0, 20.11.2026)
- **Server:** `BlobStore`-Trait mit `LocalFs` (`NOVA_UPLOAD_DIR/{hash[0..2]}/{hash}`,
  SHA-256, Dedupe). `POST /channels/{id}/attachments?filename=` und
  `/dms/{id}/attachments`: roher Body, gestreamt in eine Zwischendatei
  (20 MB Upload = +4 MB RSS), Limit `NOVA_MAX_UPLOAD_BYTES` (25 MB, auch ohne
  `Content-Length`), Quota pro Guild (2 GB) bzw. pro Person in DMs (512 MB),
  20 Uploads/min. Typ aus den ersten Bytes; JPEG/PNG/WebP ohne EXIF/GPS, XMP,
  IPTC, Textblöcke und Anhängsel nach dem Bildende, Drehung bleibt
  (`crates/api/src/media.rs`, ohne Neukodieren). SVG und alles andere nur als
  Download. `GET /attachments/{id}` mit Rechteprüfung, `Range`, `ETag`/`304`,
  `nosniff`, `CSP: sandbox`; `DELETE` für eigene, ungesendete Uploads.
  Bereinigung alle 15 min (verwaiste Uploads > 24 h, Blobs ohne Verweis).
- **Senden:** `attachment_ids` im Gateway (`message_create`) und per REST
  (Kanal, DM, Bots), Transaktion hängt nur eigene, ungesendete Uploads aus
  demselben Kanal an; mit Anhang darf der Text leer sein (Migration `0004`).
  Anhänge auch in Pins und im DM-Verlauf.
- **Web:** „+“ im Composer, Ziehen & Ablegen auf Verlauf oder Composer,
  Einfügen aus der Zwischenablage (Screenshots), Fortschritt per XHR,
  Chips mit Entfernen, Bilder inline mit festen Maßen (kein Springen),
  Bildansicht mit „Herunterladen“, Videos/Audio mit Player, Dateien als Zeile.
  Neues Recht „Dateien anhängen“ im Rollen-Editor. Galerie: `?screen=uploads`.
- **Nativ:** „+“ öffnet den Systemdialog (rfd, unter Linux XDG-Portal statt
  GTK), Chips mit Status, Bilder als verkleinerte Vorschau (EXIF-Drehung,
  höchstens 720 px, max. 96 im Speicher), Klick speichert über den
  Speichern-Dialog, Dateien mit Speichern-Knopf; in Kanälen und DMs.
- **Installer/Betrieb:** `install.sh` legt `/var/lib/nova/uploads` an, trägt
  `NOVA_UPLOAD_DIR` auch in bestehende `nova.env` nach (ohne den Eintrag
  startete der Dienst unter `ProtectSystem=strict` nicht), installiert
  `nova-backup` (DB-Dump + Uploads) samt Cronjob, `--no-backup` überspringt
  das. systemd-Unit: `StateDirectory=nova`, Standard für `NOVA_UPLOAD_DIR`.
  `nova doctor` meldet Schreibbarkeit und Belegung.
- **Getestet** (20.11.2026, lokal): 2 Integrationstests (Rechte, Metadaten,
  Range, 304, SVG, Quota-Fehler, Verwerfen, Kanal-Override, Gateway, DMs,
  Bereinigung samt Blob), Unit-Tests für Typerkennung, JPEG/PNG/WebP-Filter,
  Range, Dateinamen. Web per Playwright: Auswahl, Fortschritt, Senden,
  Bildansicht, Download, Reload, Ziehen & Ablegen, Einfügen, Nachricht nur
  aus Anhang, Dark/Light, Galerie. Nativ unter Xvfb: Vorschau, Datei-Zeile,
  Chips, Senden mit Anhang (`NOVA_DEV_ATTACH`, da der Dateidialog dort nicht
  bedienbar ist). Backup-Skript gegen eine echte Datenbank.
- **Nicht getestet:** Dateidialoge auf echtem Windows/Linux-Desktop,
  Speichern-Dialog, Uploads über Apache auf dem VPS.
- **Messwerte:** WASM gzip 527 KB (vorher 502 KB, +25 KB), roh 1.346 KB.
- Bekannt: Bilder im nativen Client haben eckige Ecken (tiny-skia ignoriert
  den Radius), Vorschaubilder (Thumbnails) erzeugt der Server nicht – große
  Fotos lädt die Web-UI in voller Größe.

### Kanal-Rechte, Foren und Webhooks (0.8.0, 20.11.2026)
- **Rechte-Editor** (Server-Einstellungen → Kanäle → Bearbeiten): pro Rolle
  oder Mitglied und Recht „Verweigern / Standard / Erlauben“, sofort
  gespeichert; „Alles auf Standard“ löscht den Override. Neu:
  `GET /channels/{id}/overrides`. Text/Forum: Sehen, Schreiben, Verlauf,
  Reagieren, Anhängen, Threads starten, Nachrichten/Threads verwalten;
  Sprache: Sehen, Beitreten, Sprechen, Video senden.
- **Foren** (`kind = 3`): Anlegen im Kanal-Dialog, Beitragsliste (Titel,
  Vorschau, Autor, Antworten, zuletzt aktive oben), „Neuer Beitrag“, ein Klick
  öffnet den Beitrag als Thread rechts. Direkt ins Forum schreiben lehnen
  Gateway und REST ab. Nativer Client: Foren bei den Textkanälen, Beiträge
  als Threads darunter, Hinweis statt Eingabezeile (Beiträge starten nur in
  der Web-App).
- **Webhooks:** anlegen/löschen pro Textkanal (`MANAGE_WEBHOOKS`, neu im
  Rollen-Editor), URL einmal sichtbar mit Kopieren, `POST
  /webhooks/{id}/{token}` postet als Bot-Konto des Webhooks, rate-limitiert.
  Migration `0005_forums_webhooks` (nur neue Tabelle).
- **Getestet:** Integrationstest `forums_webhooks_and_channel_overrides`
  (Forum ohne Direktnachrichten, Beitrag mit Anhang, Antworten-Zähler,
  Override lesen/verbieten, Webhook-Rechte, falsches Token 404, Ausführen,
  Löschen); im Browser: Forum per Dialog, Beitrag, Antwort, Override setzen
  und zurücknehmen, Webhook anlegen, per URL auslösen, Nachricht von
  „Build-Bot“ im Kanal.
- **Nicht gebaut:** Forum-Tags und Sortierung, Webhook-Namen pro Nachricht
  überschreiben, Forum in der Komponenten-Galerie (braucht den Server).

### Screenshare, Simulcast und Kamera (0.8.0, 20.11.2026)
- **Simulcast mit Schichtwahl:** Bildschirm und Kamera senden aus dem Browser
  drei H.264-Schichten (voll, ½, ¼). Der SFU wählt pro Empfänger und Spur die
  Schicht, wechselt am Schlüsselbild (fordert es sofort an) und weicht aus,
  wenn eine Schicht 1,5 s ausbleibt. Empfänger: Bildschirme hoch (ab drei
  gleichzeitig mittel), Kameras mittel (ab fünf Personen klein);
  Einstellung „Automatisch / Immer hoch / Datensparend“.
- **Qualitätsstufen beim Teilen:** Ausgewogen (1080p30), Scharf (1440p15,
  `contentHint: text`), Flüssig (720p60, `motion`).
- **Ton mitsenden:** Tab-Ton bzw. Systemton (Chrome/Windows), eigene Spur
  `s{id}` mit eigener Lautstärke/Taubschaltung, ohne Sprecher-Ring.
- **Kamera:** Knopf im Dock, 720p30 mit Simulcast, Bild in der Kachel statt
  Avatar, eigene Vorschau gespiegelt, Gerätewahl in den Einstellungen; Recht
  „Video senden“ (bisher „Bildschirm teilen“). Video-Limit zählt Bildschirme
  und Kameras (8 pro Raum).
- **SFU räumt auf und meldet:** Beendete Spuren fallen weg (kein
  eingefrorenes Bild mehr), Bildschirm/Kamera an/aus und Verbindungsabrisse
  gehen sofort als `voice_state_update` an alle (vorher blieben Abgerissene
  sichtbar, `screen` kam nur zufällig rechtzeitig an).
- **Kompatibel:** ältere Web-Clients und der native Client bekommen weiter
  `u{id}` für Mikrofon und Bildschirm und die hohe Schicht; `c…`/`s…`
  übergehen sie.
- **Getestet** (lokal, zwei Chromium mit Fake-Geräten, SFU auf der
  Container-IP): Kamera A sendet 1280×720/640×360/320×180, B empfängt
  640×360 (automatisch), 1280×720 (hoch), 320×180 (datensparend), Kamera aus
  → Kachel weg; Bildschirm 1920×1080/960×540/480×270, B empfängt 1920×1080,
  Ton als zweites Audio-Element, Freigabe aus → Bühne weg; eigene Vorschau
  bleibt beim Beitritt anderer erhalten. Unit-Tests für Schichtwechsel und
  Rückfall. **Nicht getestet:** echte Kameras/Bildschirme, Systemton unter
  Windows, Bandbreiten-Engpässe (Chrome schaltet dann selbst Schichten ab),
  Kamera im nativen Client (nicht gebaut).

### Desktop-Hülle und Web-Sicherheit (0.8.0, 20.11.2026)
- **CSP für die Web-App** (auch in Tauri, das die Server-URL lädt): `nova
  serve` schickt für `index.html` `script-src 'self' 'wasm-unsafe-eval'`,
  `connect-src 'self'` + eigenes Gateway, `frame-ancestors 'none'`,
  `object-src 'none'`, dazu `X-Frame-Options`, `Referrer-Policy`,
  `Permissions-Policy`, `nosniff`. Geprüft per Playwright: Anmeldung, Senden,
  Palette, Upload, Bildansicht und Voice-Beitritt ohne einen CSP-Verstoß.
  `tauri.conf.json`: dieselbe Richtlinie statt `csp: null`.
- **Caching:** gehashte Dateien (`assets/*-dxh*`) ein Jahr `immutable`, alles
  andere mit `ETag`/`304`. Vorher lud jeder Aufruf das 1,3-MB-WASM neu.
- **Tauri:** Tray („Nova anzeigen“, „Mit dem System starten“, „Beenden“,
  Linksklick zeigt das Fenster), Schließen legt in den Tray, native
  Benachrichtigungen über `tauri-plugin-notification` (ersetzt
  `window.Notification`, die Web-App bleibt unverändert; die Server-URL bekommt
  nur drei Benachrichtigungsrechte), Autostart optional über den Tray (startet
  versteckt), eine Instanz.
- **Installer:** NSIS-Deinstallation entfernt den Autostart-Eintrag
  (`desktop/src-tauri/windows/hooks.nsh`), deb hängt von
  `libayatana-appindicator3-1` ab, Release-Workflow installiert die
  AppIndicator-Header. `Cargo.lock` der Hülle ist jetzt eingecheckt.
- **Getestet** (lokal, Linux/WebKitGTK unter Xvfb): Build, Fenster lädt die
  App mit CSP, Tray-Aufbau ohne Absturz. **Nicht getestet:** Tray-Menü,
  Benachrichtigungen und Autostart auf echtem Windows/Linux-Desktop (hier kein
  D-Bus), MSI-Deinstallation (entfernt den Autostart-Eintrag noch nicht).

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
2. Server-Leiste: Ungelesen-Punkt und Erwähnungs-Zahl pro Server brauchen
   Ungelesen-Daten für alle Server (die API liefert sie nur für den offenen).
3. P6 (Custom-Emojis, Soundboard, Status-Texte) erst nach Voice-Messung.
4. Uploads: serverseitige Vorschaubilder für große Fotos; DMs in der Web-App.
5. Desktop: Tray, Benachrichtigungen und Autostart auf echten Rechnern prüfen;
   Code-Signing, Tauri-Updater.
6. Video: Kamera im nativen Client (Senden und Anzeigen), Schichtwahl im
   nativen Client (empfängt immer „h“).
7. Foren: Tags, Sortierung; Beiträge im nativen Client starten.

---

## Starten

```bash
cargo run -p nova -- migrate
cargo run -p nova -- serve
cargo xtask check
```
