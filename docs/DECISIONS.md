# Entscheidungen

Kurzbegründungen für Weichenstellungen, die den Rahmen des Auftrags
(„alles in Rust, ein VPS, 30 Nutzer, Voice zuerst“) konkretisieren. Neueste
Einträge oben.

## 2026-11-20 — Uploads: Metadaten filtern statt Bilder neu kodieren

**Entscheidung:** Bilder werden nicht mit `image` neu kodiert (wie in
`01-architecture.md` §12 geplant), sondern auf Byte-Ebene gefiltert
(`crates/api/src/media.rs`): EXIF/GPS, XMP, IPTC, Kommentare und PNG-
Textblöcke fliegen raus, Farbprofile bleiben, die EXIF-Drehung wird als
minimaler EXIF-Block wieder eingesetzt, Daten hinter dem Bildende werden
verworfen. Den Typ bestimmen die ersten Bytes; inline gibt es nur JPEG, PNG,
GIF und WebP bis 16 384 px Kante und 60 MP.

**Warum:** Neu kodieren kostet auf 2 vCores pro Foto 100–300 ms CPU und bis
zu 100 MB RAM, verschlechtert die Qualität (der reine Rust-Encoder kann nur
JPEG verlustbehaftet, WebP nur verlustfrei) und bringt die Bild-Codecs als
Angriffsfläche in den Server. Der Filter streamt, braucht konstant wenig
Speicher und entfernt, worum es geht: Ortsdaten. Polyglot-Dateien entschärfen
zusätzlich `X-Content-Type-Options: nosniff`, `Content-Security-Policy:
default-src 'none'; sandbox` und `Content-Disposition: attachment` für alles,
was kein Bild ist.

## 2026-11-20 — Uploads: roher Body, Auslieferung durch Axum

**Entscheidung:** `POST …/attachments?filename=` nimmt den rohen Body (kein
Multipart) und streamt ihn in eine Zwischendatei unter `NOVA_UPLOAD_DIR/tmp`,
gehasht und gezählt. Ausgeliefert wird durch Axum (`Range`, `ETag`, `304`),
nicht über `forward_auth` + `file_server` im Reverse-Proxy.

**Warum:** Die Produktion läuft hinter Apache, nicht Caddy; ein Weg für beide
Proxys. Gestreamt kosten 20 MB im Test 4 MB RAM. Bei unter 500 aktiven
Nutzern ist die CPU für das Ausliefern vernachlässigbar.

## 2026-11-20 — Nachrichten ohne Text (nur Anhänge)

**Entscheidung:** Migration `0004` ersetzt die Prüfung
`char_length(content) between 1 and 8000` durch `<= 8000`. Das ist eine
Lockerung, keine der in `CLAUDE.md` aufgezählten Operationen
(`add column … default`, Tabellen, Indizes) – aber genauso
vorwärtskompatibel: ältere Server schreiben weiter nur nicht-leere Texte,
ältere Clients zeigen leeren Text leer an, nichts wird gelöscht oder
umbenannt.

**Warum:** Ein Screenshot ohne Kommentar ist der häufigste Upload. Ein
Platzhaltertext in `content` hätte ältere Clients verwirrt.

## 2026-11-20 — Aufräumen der Blobs mit Sperre statt Zeitfenster

**Entscheidung:** Uploads halten eine Lesesperre (`AppState::blob_gc`) vom
Ablegen des Blobs bis zur Datenbankzeile; die Bereinigung (alle 15 min) hält
die Schreibsperre, während sie Verweise prüft und löscht. Verwaiste Uploads
verschwinden nach 24 h, Blobs ohne Verweis frühestens nach einer Stunde.

**Warum:** Gleiche Inhalte teilen sich einen Blob. Ohne Sperre könnte die
Bereinigung einen Blob löschen, auf den ein paralleler Upload gerade eine neue
Zeile anlegt. Ein Prozess, eine Sperre – billiger als Referenzzähler.

## 2026-11-15 — Streams: H.264 über OpenH264, SFU nur noch H.264 für Video

**Entscheidung:** Der SFU bietet für Video nur noch H.264 Constrained
Baseline (`42e01f`, `packetization-mode=1`, PT 108) an, Audio bleibt Opus.
Der native Client kodiert und dekodiert H.264 mit Ciscos OpenH264
(`openh264`-Crate, aus dem C++-Quelltext gebaut). Aufnahme unter Windows per
GDI (`windows-sys`), unter Linux per X11 (`x11rb`); Wayland vorerst nicht.

**Warum:** Der SFU leitet RTP unverändert weiter, also müssen alle
Teilnehmer denselben Codec sprechen. Für VP8/H.264 gibt es keinen fertigen
Echtzeit-Encoder in reinem Rust; AV1 (rav1e) wäre zu CPU-hungrig und in
Safari kaum nutzbar. Jeder Browser spielt H.264 Constrained Baseline.
OpenH264 baut mit dem vorhandenen mingw-`g++` bzw. MSVC – das ist die eine
bewusste Ausnahme von „keine C-Toolchain“ (vom Auftraggeber so gewählt).

**Gemessen (15.11.2026, lokal, Release):** 1920×1080 bei 30 fps; Aufnahme
19 ms + Kodierung 15 ms pro Bild in getrennten Threads; Sender ~42 %,
Zuschauer ~47 % eines Kerns. Nativ → Chromium: 1920×1080, 31 fps dekodiert.
Chromium → nativ: dekodiert und angezeigt. Ende-zu-Ende-Latenz nicht gemessen.

## 2026-11-15 — Nativer Client zeichnet jedes Bild vollständig

**Entscheidung:** `App::style` liefert pro Bild einen minimal anderen
Hintergrund; tiny-skia zeichnet dadurch immer das ganze Fenster.
Aufklappmenüs (`pick_list`) in Dialogen sind durch Auswahllisten ersetzt.

**Warum:** Die Teil-Neuzeichnung von iced/tiny-skia 0.14 ließ bei umgebauten
Listen (neuer Kanal) und Overlays Reste stehen bzw. löschte die Dialogkarte.
Gezeichnet wird nur bei Ereignissen; ein volles Bild kostet auf der CPU
wenige Millisekunden. Entfernen, sobald iced das behebt.

## 2026-11-14 — Native Sprache mit str0m, cpal und `unsafe-libopus`

**Entscheidung:** Der native Client spricht WebRTC selbst über str0m (dieselbe
Bibliothek wie der SFU), nutzt cpal für Audio und Opus über `unsafe-libopus`
(per c2rust übersetzte Referenzbibliothek).

**Warum:** „Nur Rust“ ohne C-Toolchain: libopus-Bindings bräuchten cmake/CC,
die auf dem Ziel-Windows-Rechner (mingw) fehlen. Eine eigene
Echo-Unterdrückung gibt es nicht; Kopfhörer werden empfohlen. Gepuffert wird
40–200 ms pro Person, Latenz geht vor Lückenlosigkeit.

## 2026-11-14 — Web-UI: Signale statt Snapshot, Abläufe aus Dialogen mit `spawn_forever`

**Entscheidung:** Der UI-Zustand ist ein `Store` aus einzelnen Signalen
(`crates/ui/src/store.rs`) statt eines großen `SessionState`, der bei jedem
Render geklont wurde. Abläufe, die aus Dialogen oder der Palette starten und
den Dialog schließen, laufen über `spawn_forever`.

**Warum:** Eine neue Nachricht zeichnet nur noch die Nachrichtenliste neu. Mit
`spawn` gehört eine Aufgabe dem Dialog; schließt er sich, bricht der Rest ab
(gefunden: Thread öffnete sich nach dem Anlegen nicht, Löschen hätte abbrechen
können).

## 2026-11-14 — Voice-Dock nur in der Voice-Ansicht

**Entscheidung:** Die schwebende Dock-Pill erscheint nur, wenn ein
Sprachkanal geöffnet ist. In Textkanälen übernimmt die „Sprache
verbunden“-Leiste in der Kanalspalte (Kanal, Ping, Auflegen).

**Warum:** Unten mittig verdeckte die Pill im Textkanal den Composer.

## 2026-11-14 — Schrift: Inter nur, wenn installiert

**Entscheidung:** `Inter` steht vorn im Font-Stack, wird aber nicht
eingebettet; Fallback `system-ui`. JetBrains Mono ebenso.

**Warum:** Eine eingebettete Variable-Font kostet 300+ KB und müsste aus einer
externen Quelle geladen werden; das Budget gilt der Voice-Latenz und dem
WASM-Bundle. Nachrüstbar über `crates/ui/assets/` + `@font-face`.

## 2026-11-14 — Uploads auf später verschoben

**Entscheidung:** Der `BlobStore` und die Upload-Endpunkte sind nicht Teil von
0.4.0. Der Composer hat deshalb (noch) keinen „+“-Knopf.

**Warum:** Lieber vollständig fertige Moderation, Threads und Bot-API als einen
halben Upload-Pfad (Re-Encoding, Größenlimits, Rechte beim Download). Die
README behauptete Uploads bereits; das ist korrigiert.

## 2026-11-14 — Timeout als Rechte-Entzug statt eigener Prüfung

**Entscheidung:** Ein laufender Timeout entfernt in `effective_permissions`
die Bits `TIMEOUT_REVOKED` (Senden, Reagieren, Verbinden, Sprechen, Streamen,
Threads). Es gibt keine zweite Timeout-Prüfung in Gateway, REST oder Voice.

**Warum:** Eine Stelle statt vier; jeder Pfad, der Rechte prüft, respektiert
Timeouts automatisch. Der 30-s-Rechte-Cache des Gateways wird bei jeder
Moderationsaktion geleert.

## 2026-11-14 — Threads sind Kanäle mit `kind = 2`

**Entscheidung:** Ein Thread ist eine Zeile in `channels` mit `parent_id`,
`root_message_id`, `owner_id` und `archived`. Nachrichten, Lesestand, Pins und
Gateway-Abos funktionieren ohne neue Tabellen. Rechte erbt der Thread vom
Elternkanal.

**Warum:** Kein zweites Nachrichtenmodell, keine Migration bestehender Daten;
ältere Clients zeigen Threads schlicht nicht an.

## 2026-11-14 — Bot-Tokens mit Präfix, nur als Hash gespeichert

**Entscheidung:** Bot-Tokens sind `nb_` + 64 Hex-Zeichen, gespeichert wird nur
SHA-256, Header `Authorization: Bot …`. Bots sind Nutzerzeilen mit `is_bot`.

**Warum:** Gleiche Rechteprüfung wie für Menschen; das Präfix erlaubt, Tokens in
Logs und Secret-Scannern zu erkennen; ein Datenbank-Leak verrät kein Token.

## 2026-11-14 — Nativer Client bleibt neben Tauri

**Entscheidung:** Der Master-Prompt schreibt Tauri 2 als Desktop-Hülle fest.
Tauri bleibt (Release-Workflow, Installer), der native iced-Client
(`crates/client-native`, seit v0.3.0) läuft daneben weiter.

**Warum:** Tauri nutzt WebView2/Chromium; der native Client ist der Weg zu
„schneller als Chromium“ (gemessen ~22–30 MB statt 130–250 MB). Beide teilen
Server und Protokoll, entfernen hätte nur Nachteile.

## 2026-11-14 — Integrationstests ohne `migrate`

**Entscheidung:** `crates/nova/tests` setzt ein migriertes Schema voraus und
ruft `Db::migrate` nicht auf. Die CI wendet alle Dateien aus
`crates/db/migrations` per `psql` an.

**Warum:** Die sqlx-Makros brauchen das Schema ohnehin schon beim Kompilieren;
ein zweites `migrate` würde mit dem per `psql` angelegten Schema kollidieren.

## 2026-11-13 — UI-Redesign als Token-System

**Entscheidung:** Das komplette Erscheinungsbild liegt in einer
handgeschriebenen CSS-Datei (`crates/ui/assets/app.css`); Dioxus-Markup nutzt
nur Klassen. Dark ist Standard, Light folgt dem System.

**Warum:** Eine Quelle für Farben, Abstände und Zustände; kein
Inline-Styling, kein Tailwind, kein Build-Schritt für Styles. Die
Kontrastwerte sind zentral prüfbar.

**Folge:** Ein manueller Theme-Schalter fehlt bewusst; er wäre ein eigener
Persistenz- und Testpfad ohne funktionalen Mehrwert.

## 2026-11-13 — Mitgliederpanel ab 1024 px statt 900 px

**Entscheidung:** Das Panel verschwindet unter 1024 px.

**Warum:** Zwischen 900 und 1024 px bleiben sonst nur ca. 390 px für den
Chat; Nachrichten und Composer werden unruhig. Die Vorgabe nannte „ca.
900 px“, die frühere Schwelle ist die robustere Variante.

## 2026-11-13 — Screenshare: ein 1080p-Layer, kein erzwungenes Simulcast

**Entscheidung:** Der Sender erzeugt eine 1080p60-Spur; die Leitung wählt die
Auflösung über `contentHint` und Bitraten-Deckel. Simulcast wird nicht
erzwungen, weil der str0m-SFU derzeit eine Spur pro Publisher weiterreicht.

**Warum:** Ein zusätzlicher Simulcast-Layer würde ohne SFU-seitige
Layer-Auswahl nur Bandbreite kosten. Ein Layer ist auf dem Ziel-VPS die
ehrlichere Zusage.

**Offen:** Echtes Simulcast mit Layer-Selektion ist ein eigener Ausbau; siehe
`docs/03-roadmap.md`.

## 2026-11-13 — SFU advertiset explizit die öffentliche IP

**Entscheidung:** `NOVA_SFU_PUBLIC_IP` ist in der Produktion Pflicht. Fehlt
sie, startet der SFU zwar, advertiset aber `127.0.0.1`; ICE scheitert dann
still.

**Warum:** Der Server sitzt hinter NAT (IONOS). Ein automatisches Ermitteln
der öffentlichen IP beim Start ist unzuverlässig und würde einen externen
Dienst voraussetzen.

**Folge:** `/etc/nova/nova.env` enthält die IP; die Startprüfung dokumentiert
sie im Log (`public_ip=...`).

## 2026-11-13 — UDP-Portbereich 40000–40100, kein TURN

**Entscheidung:** Der SFU nutzt einen UDP-Port pro Raum aus dem Bereich
40000–40100. Es gibt keinen TURN-Server.

**Warum:** Ein TURN-Server würde einen zweiten Dienst, Credentials und
zusätzliche Latenz bedeuten. Für die Zielgruppe ist offenes UDP üblich; die
Firewall ist entsprechend dokumentiert.

**Grenze:** In Netzen, die UDP vollständig blockieren, ist Voice nicht
möglich. Das ist eine bewusste Betriebsgrenze, keine stille Fehlfunktion.

## 2026-11-13 — HTTPS über sslip.io und Let's Encrypt

**Entscheidung:** Die öffentliche Origin ist
`https://85-215-175-202.sslip.io`, Zertifikat über acme.sh, Apache als
Reverse Proxy (HTTP/2, WebSocket-Upgrade).

**Warum:** Kein eigener DNS-Zone-Eintrag nötig; sslip.io löst die IP als
Hostname auf. Damit sind sichere Cookies, `wss://` und Mikrofonzugriff
(Secure Context) ohne Zusatzdienst möglich.

## 2026-11-13 — Dioxus 0.7 statt Leptos

**Entscheidung:** Web- und Desktop-UI teilen sich ein Dioxus-Crate; Desktop
ist eine Tauri-2-Hülle um denselben WASM-Build.

**Warum:** Ein Komponentenmodell für beide Ziele, RSX ohne separates
Template-System, kleine WASM-Artefakte. Leptos wäre gleichwertig möglich
gewesen; Dioxus war im bestehenden Workspace bereits verdrahtet.

## 2026-11-13 — Kein handgeschriebenes JavaScript

**Entscheidung:** WebRTC, Mikrofon, Audio, Clipboard und Tastatur laufen über
`web-sys`/`wasm-bindgen`. Es gibt kein eigenes JS-Glue.

**Warum:** Der Auftrag verbietet handgeschriebenes JS. Die benötigten
Browser-APIs sind in `web-sys` verfügbar; ein AudioWorklet ist für den
aktuellen Opus-Pfad nicht nötig.

## 2026-11-12 — Presence und Rate-Limits im Prozess

**Entscheidung:** `DashMap` für Sessions, Presence und Limits; kein Redis.

**Warum:** Ein VPS, ein Serverprozess. Ein zusätzlicher Dienst würde
Speicher und Betriebsaufwand kosten, ohne bei dieser Größe einen Vorteil zu
bringen.

## 2026-11-12 — Uploads content-addressed im Dateisystem

**Entscheidung:** Blobs liegen unter `NOVA_UPLOAD_DIR` mit SHA-256 als Namen
hinter der `BlobStore`-Abstraktion; Metadaten in PostgreSQL.

**Warum:** Deduplizierung, einfache Backups, kein S3-Zwang. Die Abstraktion
erlaubt später einen Object-Store, ohne API oder Datenbank zu ändern.

## 2026-11-15 — Schrift: Atkinson Hyperlegible Next eingebettet (ersetzt „Inter nur, wenn installiert“)

**Entscheidung:** Web und nativer Client nutzen Atkinson Hyperlegible Next
(SIL OFL). Web: eine variable woff2 (lateinische Teilmenge, 400–700, 34 KB)
unter `crates/ui/assets/fonts/`, per `@font-face` mit `font-display: swap`,
über `rust-embed` ausgeliefert. Nativ: dieselbe Schrift als statische TTF
(Regular/Bold, je 46 KB) per `include_bytes!`. Code bleibt `ui-monospace`.

**Warum:** Teil des Designs „Funkraum“: eindeutige Formen (l/I/1, 0/O) für
Benutzernamen, eigenständiger Charakter. Die frühere Sorge (300+ KB, externe
Quelle) trifft nicht zu: die Teilmenge ist 34 KB, kommt vom eigenen Server und
lädt getrennt vom WASM. iced liest kein woff2, deshalb die TTF im nativen Client.
