# Entscheidungen

Kurzbegründungen für Weichenstellungen, die den Rahmen des Auftrags
(„alles in Rust, ein VPS, 30 Nutzer, Voice zuerst“) konkretisieren. Neueste
Einträge oben.

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
