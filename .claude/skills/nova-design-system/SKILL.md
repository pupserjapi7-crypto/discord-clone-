---
name: nova-design-system
description: Das Nova-Designsystem – Farb-Tokens, Typografie, Spacing, Radius, Motion, Zustände und Barrierefreiheit für die Dioxus-Web-UI (crates/ui/assets/app.css) und den nativen Client (crates/client-native/src/style.rs). Laden vor jeder UI-Änderung.
---

# Nova-Designsystem

Gefühl: ruhiges Profi-Werkzeug (Richtung Linear/Arc/Raycast) für Gaming-Gruppen.
Hochwertig statt bunt, **kein Discord mit anderen Farben**. Flache Flächen,
Ebenen nur über Helligkeitsstufen. Kein Glow, keine Verläufe auf Flächen, keine
dicken Schatten, kein Backdrop-Blur auf großen Flächen. Ein Akzent, sparsam.

## Tokens (`crates/ui/assets/app.css`, `:root`)

| Token | Dark | Light | Verwendung |
|---|---|---|---|
| `--nv-bg-0` | `#0B0C0F` | `#E9EBEF` | Server-Leiste |
| `--nv-bg-1` | `#111317` | `#F3F4F6` | Kanal-/Mitgliederliste |
| `--nv-bg-2` | `#16181D` | `#FFFFFF` | Chat-Fläche |
| `--nv-bg-3` | `#1C1F25` | `#F1F2F5` | Karten, Eingabe, Hover |
| `--nv-bg-4` | `#23262E` | `#E6E8EE` | aktiv, Popover, Voice-Dock |
| `--nv-line` | `rgba(255,255,255,.06)` | `rgba(15,17,22,.08)` | Trennlinien |
| `--nv-line-strong` | `rgba(255,255,255,.10)` | `rgba(15,17,22,.14)` | Ränder schwebender Elemente |
| `--nv-text-1` | `#ECEDEF` | `#14161A` | Fließtext |
| `--nv-text-2` | `#A4A8B3` | `#4B5060` | sekundär |
| `--nv-text-3` | `#6B7080` | `#6B7080` | Zeitstempel, Labels |
| `--nv-accent` | `#7C6CFF` | `#6A58F5` | einziger Akzent |
| `--nv-accent-hover` | `#8F81FF` | `#5B48E8` | |
| `--nv-accent-soft` | `rgba(124,108,255,.14)` | `rgba(106,88,245,.12)` | Erwähnungen, aktive Zeilen |
| `--nv-online` / `--nv-speaking` | `#3DD68C` | `#1F9D62` | |
| `--nv-idle` | `#F5B83D` | `#B7801A` | |
| `--nv-danger` | `#F0565B` | `#D63A40` | |
| `--nv-offline` | `#5A5F6B` | `#8A8F9C` | |

Theme: `data-theme="dark|light"` auf `<html>`; ohne Attribut folgt Light
`prefers-color-scheme`. Im Komponentencode **nie Hex-Werte**, nur Tokens.

## Typografie

- `Inter` (falls installiert), Fallback `system-ui`; Code `JetBrains Mono`,
  Fallback `ui-monospace`. Keine Webfont-Requests an fremde Server.
- Skala: 12 / 13 / 14 / 15 / 18 / 22 / 28 px (`--nv-fs-12` … `--nv-fs-28`).
- Nachrichten 15 px, Zeilenhöhe 1,45. Namen 14 px/600. Zeitstempel 12 px,
  `--nv-text-3`, `font-variant-numeric: tabular-nums`.
- Nur Gewichte 400 und 600.
- Schriftgröße 90–125 % über `--nv-scale` auf `html` (Einstellungen).

## Raster, Radius, Schatten

- Abstände: 4, 8, 12, 16, 20, 24, 32, 48 (`--nv-s-1` … `--nv-s-12`).
- Radius: 6 Badges, 10 Buttons/Inputs, 14 Karten/Modals, 999 Avatare/Pills.
- Schatten nur für Schwebendes: `0 8px 24px rgba(0,0,0,.35)` + 1 px
  `--nv-line-strong` (`--nv-shadow`).
- **Nativer Client: keine Schatten** (`style::NO_SHADOW`). iced_tiny_skia
  0.14.1 malt Schatten bei Teil-Neuzeichnungen ungeclippt immer wieder
  übereinander → Flächen werden schwarz. Tiefe dort nur über Flächenfarbe und
  `BORDER_STRONG`; der Test `styles_have_no_shadows` prüft das.

## Layout

Server-Leiste 72 · Kanäle 248 · Chat flexibel · Mitglieder 248 (einklappbar) ·
Header 48. < 1100 px Mitglieder aus, < 760 px Drawer-Navigation.

## Zustände (jede interaktive Komponente)

normal · hover (`--nv-bg-3`) · active/selected (`--nv-bg-4` bzw.
`--nv-accent-soft`) · focus-visible (2 px `--nv-accent`, 2 px Offset) ·
disabled (Opazität .45, `cursor: not-allowed`) · loading (Skeleton
`--nv-bg-3`, kein Spinner auf großen Flächen).

## Motion

120–180 ms, `cubic-bezier(0.2, 0, 0, 1)` (`--nv-ease`), nur `opacity` und
`transform`. Neue Nachrichten: Fade + 4 px Slide. Modals: Fade + Scale .98.
`prefers-reduced-motion` → 0 ms.

## Do / Don't

- Do: Hierarchie über Größe/Gewicht/Textstufe; Status = Punkt **und** Text.
- Do: Icons aus einer Line-Familie (Lucide-Stil), 1,75 px Strich, 18/20 px,
  Inline-SVG über die `Icon`-Komponente.
- Don't: zweite Akzentfarbe, farbige Flächen für Deko, Pulsieren, Glow,
  `linear-gradient` auf Flächen, Hex im RSX.

## Checkliste

- [ ] Nur Tokens, kein Hex außerhalb von `:root`.
- [ ] Fokus sichtbar, Tab-Reihenfolge sinnvoll, Klickfläche ≥ 32 px (Voice 40 px).
- [ ] Kontrast AA (Text-1/2 auf bg-1…bg-3 in beiden Themes).
- [ ] `prefers-reduced-motion` greift.
- [ ] Dark **und** Light in der Galerie geprüft (`/dev/gallery`).
