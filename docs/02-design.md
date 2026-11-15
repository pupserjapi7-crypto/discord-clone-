# Project Nova — Designsprache „Funkraum“

**Status:** umgesetzt in v0.7.0 (`crates/ui/assets/app.css`, `crates/ui/src/views`,
`crates/client-native/src/style.rs`). Die verbindliche Kurzfassung mit Tokens
und Checkliste steht im Skill
[`.claude/skills/nova-design-system/SKILL.md`](../.claude/skills/nova-design-system/SKILL.md).

## Leitidee

Nova ist kein Büro-Werkzeug, sondern der Ort, an dem sich eine kleine Gruppe
Freunde am Freitagabend zum Zocken trifft („Freitagsrunde“, „Lounge“,
„Ranked“). Die wichtigste Funktion ist Voice: **wer ist da, wer redet gerade.**

Das Bild dazu heißt **Funkraum**: ein abendlicher, bläulich-tintiger Raum, in
dem die Stimmen leuchten. Der Name Nova (ein Stern, der plötzlich hell wird)
liefert die eine Signalfarbe, **Nova-Gelb**. Gelb bedeutet in der ganzen App
genau eine Sache: *live / aktiv / jetzt*. Wer spricht, leuchtet gelb. Was
ausgewählt ist, ist gelb markiert. Der wichtigste Knopf ist gelb. Sonst nichts.

Flache Flächen, Ebenen nur über Helligkeitsstufen, kein Glow, keine Verläufe,
kein Blur. Violett (das alte Nova-Violett `#7C6CFF`) gibt es nicht mehr.

## Tokens

Alle Farben, Abstände, Radien und Zeiten sind CSS-Variablen mit Präfix `--nv-`
in `:root`. Light verwendet dieselben Namen (`data-theme="light"` oder
System-Einstellung). Im Komponentencode gibt es keine Hex-Werte.

| Gruppe | Tokens |
|---|---|
| Flächen | `--nv-bg-0` Server-Leiste · `-1` Kanäle/Mitglieder · `-2` Chat · `-3` Eingaben/Kacheln/Hover · `-4` aktiv/Popover/Dock |
| Linien | `--nv-line`, `--nv-line-strong` |
| Text | `--nv-text-1/2/3`, `--nv-text-on-accent`, `--nv-text-on-danger` |
| Akzent | `--nv-accent` (Flächen), `-hover`, `-text` (Schrift), `-soft` (Zeilen), `-line` (Streifen, Ringe, Fokus) |
| Status | `--nv-speaking`, `--nv-online`, `--nv-danger`, `--nv-offline` |
| Avatare | `--nv-tone-0…5` (Blau, Grau, Grün, Rost, Petrol, Pflaume; nie gelb) |
| Typo | `--nv-fs-12 … --nv-fs-28` (mit `--nv-scale` 90–125 %) |
| Raster | `--nv-s-1 … --nv-s-12` (4, 8, 12, 16, 20, 24, 32, 48) |
| Radius | 4 Badges · 8 Buttons/Inputs · 12 Voice-Kacheln/Modals · rund Avatare |
| Motion | `--nv-ease` = `cubic-bezier(0.2, 0, 0, 1)`, 120/160 ms, nur `opacity`/`transform` |

Kontraste geprüft: Text-1/2/3 ≥ 4,5:1 auf bg-1…bg-3 in beiden Themes,
Sprecher-Ring, Auswahl-Ring und Fokusring ≥ 3:1 (in Light über das dunklere
Gelb `--nv-accent-line`/`--nv-speaking`).

Der native Client übernimmt die Dark-Werte 1:1 als Konstanten in `style.rs`;
der Test `tokens_match_web_css` liest `app.css` und hält beide gleich.

## Schrift

**Atkinson Hyperlegible Next** (SIL OFL, `crates/ui/assets/fonts/OFL.txt`) für
UI und Text. Eindeutige Formen (l/I/1, 0/O) helfen bei Benutzernamen wie
`ida.k` oder `tom_` und wirken eigenständig statt nach Tool.

- Web: eine variable woff2-Datei (lateinische Teilmenge, 400–700), 34 KB,
  selbst ausgeliefert mit `font-display: swap`, kein fremder Server.
- Nativ: dieselbe Schrift als statische TTF (Regular 46 KB, Bold 46 KB) per
  `include_bytes!`, weil iced kein woff2 lesen kann.
- Code: `ui-monospace` aus dem System; Monospace nur für Code. Zahlen (Ping,
  Zeiten, Zähler) über `tabular-nums` der Hauptschrift.
- Gewichte 400 und 700. Nachrichten 15 px, Zeilenhöhe 1,5. Überschriften
  28 px/700, Laufweite `-0.01em`. Keine Versalien-Labels.

## Layout

Server-Leiste 72 · Kanäle 248 · Chat flexibel · Mitglieder 248 (Thread-Spalte
380) · Header 48. Unter 1100 px liegen Mitglieder/Thread als Panel über dem
Chat und starten geschlossen; unter 760 px werden Leiste und Kanäle zum Drawer.
Login: ab 900 px links die Formularspalte, rechts ein statischer Ausschnitt
einer Voice-Lounge in Echtgröße; darunter nur das Formular.

## Formen und Details

- Server-Icons: immer abgerundete Quadrate, 44 px, Radius 12. Ausgewählt:
  `--nv-bg-4` plus 2-px-Ring in Gelb. Kein Kreis-zu-Quadrat, kein Pill-Marker.
- Ausgewählter Kanal: `--nv-accent-soft`, Text `--nv-text-1`, 2-px-Streifen links.
- Status über Form: online gefüllter Kreis, abwesend Halbmond (`--nv-text-3`),
  nicht stören Kreis mit Querbalken, offline hohler Ring; immer mit Text oder
  `aria-label`.
- Erwähnung von dir: Zeile `--nv-accent-soft` + 2-px-Streifen, `@name` in
  `--nv-accent-text`/700. „Neu“-Trenner in `--nv-danger`.
- Bot-Badge neutral (`--nv-bg-4`, `--nv-text-2`). Reaktionen auf `--nv-bg-3`,
  eigene mit 1-px-Rand in Gelb.
- Composer: `--nv-bg-3`, Radius 8, Fokus = `--nv-line-strong` plus Fokusring.
- Kanal-Willkommen: `#` in einer 56-px-Fläche, „Willkommen in #allgemein“ 28/700.
- Voice-Dock: schwebende Pill unten mittig nur in der Voice-Ansicht; in
  Textkanälen zeigt die „Sprache verbunden“-Leiste Kanal, Ping und Auflegen.
- Command Palette 560 px: Kanäle, Personen, Nachrichten, Aktionen; ↑/↓/Enter.
- Icons: eine Line-Familie im Lucide-Stil, 1,75 px Strich, Inline-SVG.

## Signature-Element: die Stimme

Kühnheit kommt an genau eine Stelle, alles andere bleibt ruhig.

- **Kanalliste:** Ein Sprachkanal mit Leuten zeigt darunter eine Reihe kleiner
  Avatare (20 px, 4 px überlappend, max. 6, dann „+n“), darunter die Namen.
  Wer spricht, bekommt einen 2-px-Ring `--nv-speaking` und rückt 2 px nach oben
  (120 ms). So sieht man, ohne den Kanal zu öffnen, ob in der Lounge geredet wird.
- **Voice-Ansicht:** Kacheln auf `--nv-bg-3`, Radius 12. Spricht jemand:
  Ring 2 px (eingeblendet, nie pulsierend) und der Namensbalken füllt sich mit
  `--nv-accent`, Text `--nv-text-on-accent`. Das ist die einzige größere gelbe
  Fläche – genau deshalb erinnert man sich daran.
- **Beitreten:** Die einzige Animation ohne direkten Klick: direkt nach dem
  Beitreten erscheinen die Kacheln einmal nacheinander (je 40 ms versetzt,
  Fade + Scale .98 → 1, < 300 ms). Sonst keine Einblend-Animationen auf Listen.
- Ping: grün < 60 ms, `--nv-text-3` < 120 ms, rot darüber, immer mit Zahl.

## Do / Don't

- Don't: Gelb für etwas, das nicht live oder aktiv ist (auch nicht Logo oder Deko).
- Don't: Violett, eine zweite Akzentfarbe, Glow, Pulsieren, Verläufe.
- Don't: Monospace für Labels oder Zahlen, Versalien-Labels.
- Don't: Kreis-zu-Quadrat-Server-Icons und Pill-Marker (Discord-Geste).
- Do: Hierarchie über Größe, Gewicht und Textstufe; Status über Form und Text.

## Barrierefreiheit

Tastatur überall (Dialoge fokussieren ihr erstes Feld, Esc schließt),
Fokusring 2 px `--nv-accent-line` mit 2 px Abstand, Klickflächen ≥ 32 px
(Voice 40 px), Status immer Form **und** Text/`aria-label`, Nachrichtenliste
`role="log"` mit `aria-live`, `prefers-reduced-motion` → 0 ms, Schriftgröße
90–125 % über `--nv-scale`.

## Galerie und Screenshots

`cargo xtask gallery` baut die UI mit `/dev/gallery`: alle Bausteine in allen
Zuständen nebeneinander in Dark und Light, sowie ganze Bildschirme mit
Testdaten (`?screen=chat|thread|voice|screenshare|palette|settings|moderation|member|login|onboarding`,
`&theme=dark|light`). Daraus stammen die Bilder in
[`docs/screenshots/`](screenshots/) (`<screen>-<theme>.jpg`, 1440 × 900).

## Budget (gemessen 15.11.2026)

| Posten | Ziel | Ist |
|---|---|---|
| WASM gzip | < 2 MB | 502 KB (1.281 KB roh; vor Funkraum 500 KB) |
| Schrift (woff2, einmal geladen) | < 120 KB | 34 KB |
| Externe Requests (Fonts/CDNs) | 0 | 0 |
| Handgeschriebenes JS | 0 | 0 |
