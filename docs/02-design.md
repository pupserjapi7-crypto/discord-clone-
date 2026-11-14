# Project Nova — Designsprache

**Status:** umgesetzt in v0.4.0 (`crates/ui/assets/app.css`, `crates/ui/src/views`).
Die verbindliche Kurzfassung mit Checkliste steht im Skill
[`.claude/skills/nova-design-system/SKILL.md`](../.claude/skills/nova-design-system/SKILL.md).

## Leitidee

Ruhiges Profi-Werkzeug (Richtung Linear/Arc/Raycast) für Gaming-Gruppen –
hochwertig statt bunt und **nicht** Discord in anderen Farben. Flache Flächen,
Ebenen nur über Helligkeitsstufen, ein Akzent (Nova-Violett `#7C6CFF`),
kein Glow, keine Verläufe, kein Blur auf Flächen.

## Tokens

Alle Farben, Abstände, Radien und Zeiten sind CSS-Variablen mit Präfix `--nv-`
in `:root`. Light verwendet dieselben Namen (`data-theme="light"` oder
System-Einstellung). Im Komponentencode und außerhalb der Token-Blöcke gibt es
keine Hex-Werte.

| Gruppe | Tokens |
|---|---|
| Flächen | `--nv-bg-0` Server-Leiste · `-1` Kanäle/Mitglieder · `-2` Chat · `-3` Karten/Eingaben/Hover · `-4` aktiv/Popover |
| Linien | `--nv-line`, `--nv-line-strong` |
| Text | `--nv-text-1/2/3` |
| Akzent | `--nv-accent`, `-hover`, `-soft` |
| Status | `--nv-online`, `--nv-idle`, `--nv-danger`, `--nv-offline`, `--nv-speaking` |
| Typo | `--nv-fs-12 … --nv-fs-28` (mit `--nv-scale` 90–125 %) |
| Raster | `--nv-s-1 … --nv-s-12` (4, 8, 12, 16, 20, 24, 32, 48) |
| Radius | 6 Badges · 10 Controls · 14 Karten · 999 Pills |
| Motion | `--nv-ease` = `cubic-bezier(0.2, 0, 0, 1)`, 120/160 ms, nur `opacity`/`transform` |

## Layout

Server-Leiste 72 · Kanäle 248 · Chat flexibel · Mitglieder 248 (Thread-Spalte
380) · Header 48. Unter 1100 px liegen Mitglieder/Thread als Panel über dem
Chat und starten geschlossen; unter 760 px werden Leiste und Kanäle zum Drawer.

## Signature-Details

- Server-Icons 48 px rund → Radius 14 bei Hover/Auswahl, Pill-Marker links
  (8 / 20 / 36 px für ungelesen / Hover / ausgewählt).
- Sprecher-Ring 2 px `--nv-speaking`, eingeblendet, nie pulsierend.
- Voice-Grid mit Kacheln (`--nv-bg-3`, Radius 14), Screenshare groß mit Leiste
  darunter, Platzhalter solange das Bild lädt.
- Voice-Dock: schwebende Pill unten mittig **nur in der Voice-Ansicht**; in
  Textkanälen zeigt die „Sprache verbunden“-Leiste über dem Nutzerpanel Kanal,
  Ping und Auflegen (sonst verdeckt das Dock den Composer).
- Ping mit Farbpunkt: grün < 60 ms, gelb < 120 ms, sonst rot – plus Zahl.
- Command Palette 560 px: Kanäle, Personen, Nachrichten, Aktionen; ↑/↓/Enter.
- Erwähnungen: `--nv-accent-soft` + 2-px-Streifen; „Neu“-Trenner in `--nv-danger`.
- Composer: Enter sendet, Umschalt+Enter neue Zeile, wächst bis 40 % Höhe,
  Tipp-Anzeige darunter, Emoji-Auswahl rechts.
- Icons: eine Line-Familie im Lucide-Stil, 1,75 px Strich, Inline-SVG.

## Barrierefreiheit

Tastatur überall (Dialoge fokussieren ihr erstes Feld, Esc schließt),
Fokusring 2 px Akzent mit 2 px Abstand, Klickflächen ≥ 32 px (Voice 40 px),
Status immer Punkt **und** Text/`aria-label`, Nachrichtenliste `role="log"` mit
`aria-live`, `prefers-reduced-motion` → 0 ms.

## Galerie und Screenshots

`cargo xtask gallery` baut die UI mit `/dev/gallery`: alle Bausteine in allen
Zuständen nebeneinander in Dark und Light, sowie ganze Bildschirme mit
Testdaten (`?screen=chat|thread|voice|screenshare|palette|settings|moderation|member|login|onboarding`,
`&theme=dark|light`). Daraus stammen die Bilder in
[`docs/screenshots/`](screenshots/).

## Budget (gemessen 14.11.2026)

| Posten | Ziel | Ist |
|---|---|---|
| WASM gzip | < 2 MB | 501 KB (1.272 KB roh) |
| Externe Requests (Fonts/CDNs) | 0 | 0 |
| Handgeschriebenes JS | 0 | 0 |
