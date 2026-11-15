---
name: nova-design-system
description: Das Nova-Designsystem „Funkraum“ – Farb-Tokens, Typografie, Spacing, Radius, Motion, Zustände und Barrierefreiheit für die Dioxus-Web-UI (crates/ui/assets/app.css) und den nativen Client (crates/client-native/src/style.rs). Laden vor jeder UI-Änderung.
---

# Nova-Designsystem „Funkraum“

Ein abendlicher, bläulich-tintiger Raum, in dem die Stimmen leuchten. Nova ist
der Ort, an dem sich eine kleine Gruppe Freunde zum Zocken trifft; die
wichtigste Frage ist: **wer ist da, wer redet gerade.** Flache Flächen, Ebenen
nur über Helligkeitsstufen. Kein Glow, keine Verläufe auf Flächen, kein Blur.

**Nova-Gelb heißt genau eins: live / aktiv / jetzt.** Wer spricht, leuchtet
gelb. Was ausgewählt ist, ist gelb markiert. Der wichtigste Knopf ist gelb.
Sonst nichts.

## Tokens (`crates/ui/assets/app.css`, `:root`)

| Token | Dark | Light | Verwendung |
|---|---|---|---|
| `--nv-bg-0` | `#10151E` | `#DDE3EC` | Server-Leiste |
| `--nv-bg-1` | `#151B26` | `#E9EDF3` | Kanäle, Mitglieder |
| `--nv-bg-2` | `#1A2130` | `#F7F9FC` | Chat |
| `--nv-bg-3` | `#212938` | `#EEF2F7` | Eingaben, Voice-Kacheln, Hover |
| `--nv-bg-4` | `#2A3446` | `#E1E7F0` | aktiv, Popover, Dock |
| `--nv-line` | `rgba(200,215,240,.08)` | `rgba(18,24,34,.08)` | Trennlinien |
| `--nv-line-strong` | `rgba(200,215,240,.14)` | `rgba(18,24,34,.14)` | Ränder schwebender Elemente |
| `--nv-text-1` | `#E8ECF3` | `#121822` | Fließtext |
| `--nv-text-2` | `#A9B2C3` | `#45506A` | sekundär |
| `--nv-text-3` | `#8692A7` | `#5A6579` | Zeitstempel, Hinweise |
| `--nv-accent` | `#FFC34D` | `#F2B12E` | Flächen: Primärknopf, Sprecher-Balken |
| `--nv-accent-hover` | `#FFD27A` | `#E0A01C` | |
| `--nv-accent-text` | `#FFC34D` | `#8A5B00` | Links, Erwähnungs-Text, aktive Icons |
| `--nv-accent-soft` | `rgba(255,195,77,.13)` | `rgba(242,177,46,.18)` | Erwähnungen, ausgewählte Zeile |
| `--nv-accent-line` | `#FFC34D` | `#A86F00` | Streifen, Auswahl-Ring, Fokusring (≥ 3:1) |
| `--nv-text-on-accent` | `#1B1405` | `#1B1405` | Text auf Gelb, nie Weiß |
| `--nv-text-on-danger` | `#1B1405` | `#FFFFFF` | Text auf Rot |
| `--nv-speaking` | `#FFC34D` | `#A86F00` | Sprecher-Ring |
| `--nv-online` | `#4FD18B` | `#137A45` | |
| `--nv-danger` | `#FF6B6B` | `#B83232` | |
| `--nv-offline` | `#5D6779` | `#8A93A5` | |
| `--nv-tone-0…5` | Blau, Grau, Grün, Rost, Petrol, Pflaume | hell | Avatare, nie gelb |

Theme: `data-theme="dark|light"` auf `<html>`; ohne Attribut folgt Light
`prefers-color-scheme`. Im Komponentencode **nie Hex-Werte**, nur Tokens.
Kontraste: Text-1/2/3 ≥ 4,5:1 auf bg-1…bg-3 in beiden Themes, Ringe/Streifen
≥ 3:1 (deshalb in Light `--nv-accent-line` statt `--nv-accent`).

**Nativer Client:** `style.rs` trägt exakt die Dark-Werte (Kommentar mit
Token-Namen). Der Test `tokens_match_web_css` liest `app.css` und bricht, wenn
beide auseinanderlaufen – bei neuen Tokens dort ergänzen.

## Typografie

- **Atkinson Hyperlegible Next** (OFL) für alles: eindeutige Formen (l/I/1,
  0/O) für Namen wie `ida.k` oder `tom_`. Web: variable woff2, lateinische
  Teilmenge, 34 KB (`assets/fonts/`, `font-display: swap`). Nativ: dieselbe
  Schrift als TTF (Regular + Bold, je 46 KB), weil iced kein woff2 liest.
- Code: `ui-monospace` aus dem System. Monospace nur für Code, nie für Zahlen
  oder Labels; Zahlen über `font-variant-numeric: tabular-nums`.
- Skala: 12 / 13 / 14 / 15 / 18 / 22 / 28 px (`--nv-fs-*`, `--nv-scale` 90–125 %).
- Nur Gewichte 400 und 700. Nachrichten 15 px, Zeilenhöhe 1,5.
- Überschriften (Kanal-Willkommen, Login, Einstellungen): 28 px/700, `-0.01em`.
- Keine Versalien-Labels, keine Wörter in Akzentfarbe hervorheben.

## Formen

- Radius mit Hierarchie: 4 Badges · 8 Buttons/Inputs · 12 Voice-Kacheln und
  Modals · rund Avatare. Karten im Chat gibt es nicht.
- **Server-Leiste:** immer abgerundete Quadrate (44 px, Radius 12), kein
  Kreis-zu-Quadrat, kein Marker am Rand. Ausgewählt: `--nv-bg-4` + 2-px-Ring
  `--nv-accent-line`.
- Ausgewählter Kanal: `--nv-accent-soft`, Text `--nv-text-1`, 2-px-Streifen links.
- Schatten nur für Schwebendes (`--nv-shadow`). **Nativer Client: keine
  Schatten** (`style::NO_SHADOW`, Test `styles_have_no_shadows`): iced_tiny_skia
  0.14.1 malt sie bei Teil-Neuzeichnungen immer wieder übereinander.

## Status über Form, nicht nur Farbe

10 px, 2 px Ring in der Fläche dahinter (`--nv-ring-bg`), immer mit Text oder
`aria-label`: online gefüllter Kreis · abwesend Halbmond in `--nv-text-3` ·
nicht stören Kreis mit Querbalken · offline hohler Ring. Gelb ist kein Status.

## Signature-Element: die Stimme

Kühnheit nur hier, alles andere bleibt ruhig.

- **Kanalliste:** unter einem belegten Sprachkanal eine Reihe 20-px-Avatare
  (4 px überlappend, max. 6, dann „+n“), darunter die Namen. Wer spricht:
  2-px-Ring `--nv-speaking` und `translateY(-2px)` in 120 ms.
- **Voice-Kacheln** (`--nv-bg-3`, Radius 12): Sprecher-Ring 2 px (Outline,
  liegt auch über Video), Namensbalken füllt sich mit `--nv-accent` und
  `--nv-text-on-accent` – die einzige größere gelbe Fläche.
- **Beitreten:** einzige Animation ohne Klick – Kacheln einmal nacheinander
  (je 40 ms, Fade + Scale .98 → 1, < 300 ms), nur direkt nach dem Beitreten.
- Ping: grün < 60 ms, `--nv-text-3` < 120 ms, rot darüber, immer mit Zahl.
  Auflegen bleibt `--nv-danger`.

## Zustände (jede interaktive Komponente)

normal · hover (`--nv-bg-3`) · ausgewählt (`--nv-accent-soft` bzw. `--nv-bg-4`)
· focus-visible (2 px `--nv-accent-line`, 2 px Abstand) · disabled (Opazität
.45) · loading (Skeleton `--nv-bg-3`).

## Motion

120–180 ms, `--nv-ease`, nur `opacity`/`transform`. Keine Einblend-Animationen
auf Listen oder Karten (außer dem Beitritts-Moment). Modals: Fade + Scale .98.
`prefers-reduced-motion` → alles sofort.

## Do / Don't

- Do: Hierarchie über Größe/Gewicht/Textstufe; Status = Form **und** Text.
- Do: Icons aus einer Line-Familie (Lucide-Stil), 1,75 px Strich.
- Don't: Gelb für etwas, das nicht live oder aktiv ist (Logo, Deko, Hinweise).
- Don't: Violett; zweite Akzentfarbe; Glow, Pulsieren, Verläufe; Hex im RSX.
- Don't: Monospace für Labels oder Zahlen; Versalien-Labels.
- Don't: Kreis-zu-Quadrat-Server-Icons oder Pill-Marker (das ist Discord).

## Checkliste

- [ ] Nur Tokens, kein Hex außerhalb von `:root`; nativ: `tokens_match_web_css` grün.
- [ ] Gelb nur bei live/aktiv/ausgewählt/Primärknopf.
- [ ] Fokus sichtbar, Klickfläche ≥ 32 px (Voice 40 px), Tastatur überall.
- [ ] Kontrast AA in beiden Themes; 125 % Schrift bricht nichts.
- [ ] `prefers-reduced-motion` greift.
- [ ] Dark **und** Light in der Galerie geprüft (`/dev/gallery`).
