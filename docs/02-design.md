# Project Nova — Designsprache

**Kurzfassung:** eigenständig, nicht Discord. Ruhige dunkle Flächen, eine einzige kräftige Akzentfarbe, Glas nur für schwebende Ebenen, großzügiger Weißraum, Typografie statt Trennlinien. Alles in Tokens, damit die Themes-Engine (Feature 6) später nur Variablen tauscht.

---

## 1. Leitidee

„Weltraum-Instrumententafel“: fast schwarzer Grund, präzise Typografie, ein violett-blaues Signal für Interaktion, Farbe ausschließlich für Bedeutung (Status, Präsenz, Warnung). Keine Verläufe auf Inhaltsflächen, keine Schatten als Dekoration, keine bunten Server-Icons als Standard.

Drei Themes ab Tag 1: **Dark (Default)**, **Light**, **AMOLED**. Alle drei aus demselben Token-Satz.

---

## 2. Farb-Tokens

```css
:root[data-theme="dark"] {
  --nx-bg:            #0a0b0f;
  --nx-bg-sunken:     #07080b;
  --nx-surface:       #14171e;
  --nx-surface-2:     #1a1e27;
  --nx-surface-3:     #222733;
  --nx-line:          #262b36;
  --nx-line-soft:     rgb(255 255 255 / 0.06);
  --nx-text:          #e9ecf2;
  --nx-text-2:        #a6aebf;
  --nx-text-3:        #6e7688;
  --nx-accent:        #6c5cff;   /* Flächen, Fokus, aktive Zustände */
  --nx-accent-text:   #9c90ff;   /* Akzent als Text auf dunklem Grund (AA) */
  --nx-accent-soft:   rgb(108 92 255 / 0.16);
  --nx-online:        #34d399;
  --nx-idle:          #fbbf24;
  --nx-dnd:           #f87171;
  --nx-offline:       #6e7688;
  --nx-danger:        #f4516c;
  --nx-success:       #2dd4a7;
  --nx-warn:          #f0a35e;
  --nx-glass:         rgb(20 23 30 / 0.72);
  --nx-glass-border:  rgb(255 255 255 / 0.07);
}
:root[data-theme="light"] {
  --nx-bg: #f7f8fb; --nx-surface: #ffffff; --nx-surface-2: #f1f3f8;
  --nx-line: #e3e6ed; --nx-text: #111318; --nx-text-2: #4b5364; --nx-text-3: #737b8c;
  --nx-accent: #5646e8; --nx-accent-text: #4a3ad6;
}
:root[data-theme="amoled"] {
  --nx-bg: #000000; --nx-bg-sunken: #000000; --nx-surface: #07090d;
  --nx-surface-2: #0d1117; --nx-line: #151a21; --nx-glass: rgb(7 9 13 / 0.9);
}
```

**Regeln**

- Akzent `#6c5cff` hat auf `#14171e` nur ~3,4:1 → **nie** für Fließtext, nur für Flächen, Icons ≥ 24 px, Fokusringe. Text in Akzentfarbe nutzt `--nx-accent-text` (≥ 4,6:1).
- Statusfarben nur als 8-px-Punkt, 2-px-Ring oder Icon — nie als Flächenfarbe.
- AMOLED deaktiviert Glas (Banding/Schmieren auf OLED), nutzt solide Flächen.

---

## 3. Typografie

- **UI/Schrift:** Inter Variable, selbst gehostet, Subset `latin` + `latin-ext` (~48 KB woff2, `font-display: swap`, Preload der einen Datei). Fallback `system-ui, "Segoe UI", sans-serif`.
- **Code/Mono:** JetBrains Mono, Subset, 400/500, nur für Codeblöcke und Snowflake-IDs (~22 KB, lazy).
- **Skala (px / line-height):** 11/16 (Meta, Zeitstempel), 13/20 (Sidebar, Sekundär), 14/22 (Chat-Body, Standard), 16/24 (Untertitel), 20/28, 24/32, 30/38 (Seitentitel). Eine Skala, keine Ausnahmen.
- **Gewichte:** 400 Body, 500 Namen/Buttons, 600 Titel. Kein 700+ (wirkt in Inter schnell grob).
- **Details:** `letter-spacing: -0.011em` ab 16 px; Zeitstempel `font-variant-numeric: tabular-nums`; Zeilenlänge im Chat max. 72 ch (Lesbarkeit auf Ultrawide).
- **Deutsch:** keine feste Großschreibung von UI-Labels per CSS, echte Texte (Umlaute, ß). Keine Text-Transformation außer bei Badges.

---

## 4. Raum, Form, Tiefe

| Token | Wert | Verwendung |
|---|---|---|
| `--nx-space-1..8` | 4, 8, 12, 16, 24, 32, 48, 64 | alles |
| `--nx-radius-sm/md/lg/pill` | 8, 12, 18, 999 | Controls / Cards / Panels / Avatare |
| `--nx-shadow-1` | `0 8px 24px rgb(0 0 0 / .35)` | nur Overlays |
| `--nx-focus` | `0 0 0 2px var(--nx-bg), 0 0 0 4px var(--nx-accent)` | Tastaturfokus, immer sichtbar |
| Border | `1px solid var(--nx-line)` | Struktur statt Schatten |

**Glas** ausschließlich für: Command Palette, Modals/Sheets, Voice-Dock, Kontextmenüs. `backdrop-filter: blur(16px) saturate(120%)` + 1-px-Innenkante. Nie für Listen, nie für den Composer (Lesbarkeit).

---

## 5. Layout

```
┌────┬──────────────┬────────────────────────────────┬──────────────┐
│ 72 │ 260          │ Header (Kanal, Topic, Suche)   │ 240          │
│ R  │ Server-Header│────────────────────────────────│ Mitglieder   │
│ a  │ Kategorien   │ Nachrichten (virtualisiert)    │ (toggle)     │
│ i  │  # kanal     │                                │              │
│ l  │  # kanal  •3 │                                │              │
│    │              │────────────────────────────────│              │
│    │ Voice-Dock   │ Composer                       │              │
│    │ User-Bar     │                                │              │
└────┴──────────────┴────────────────────────────────┴──────────────┘
```

- **Desktop ≥ 1280:** 3 Spalten, Memberliste ein-/ausblendbar (merkt sich der Client).
- **Tablet 768–1279:** Memberliste als Overlay, Server-Rail bleibt.
- **Mobile < 768:** eine Spalte; Navigation als Bottom-Sheet mit Geste; Voice als kollabierbarer Streifen über dem Composer; Composer `position: sticky` mit `env(safe-area-inset-bottom)`.
- **Grid:** Sidebar `260px` (resize 220–320), Content `1fr` mit `max-width: 1200px` und Zentrierung, Memberliste `240px`.
- **Voice-UI:** Kacheln in `auto-fill minmax(180px, 1fr)`; Sprecher-Ring animiert nur `opacity`/`transform` (kein Layout-Shift, keine `box-shadow`-Animation).
- **Zustaende:** Skeleton in Flächenform (kein Spinner für Listen), leere Zustände mit einer Zeile Erklärung + einer primären Aktion, Fehler mit Retry.

---

## 6. Komponenten (M3)

Buttons (primary/ghost/danger/icon), Input, Textarea mit Autosize und Zeichenzähler, Select, Switch, Avatar mit Status-Dot, Badge/Mention-Pill, Tooltip (150 ms Delay), Message-Row (gruppiert bei gleichem Autor < 7 min), Reply-Quote, Reactions, Attachment-Card mit Thumbnail, Codeblock mit Copy, Channel-Item (mit Unread-Punkt/Badge), Voice-Tile mit Qualitäts-Pill, Toast, Modal/Sheet, Command Palette, Context-Menu, Skeleton, Empty State.

**Zustandsregeln:** Hover nur bei `(hover: hover)`, Fokus immer sichtbar, Active = `scale(.98)` + 1-px-Verschiebung, Disabled = 45 % Opazität und `cursor: not-allowed`, Ladezustand im Button mit fixer Breite (kein Springen).

---

## 7. Motion

- Dauer: 120 ms (Mikro), 160 ms (Standard), 200 ms (Overlays). Nichts darüber.
- Easing: `cubic-bezier(.22,.8,.24,1)` für Ein, `cubic-bezier(.4,0,1,1)` für Aus.
- Ausschließlich `transform` und `opacity`. Keine Animation von `height`, `top`, `box-shadow`, `filter`.
- `@media (prefers-reduced-motion: reduce)` → Dauer 1 ms, keine Slides, keine Sprecher-Pulse.
- Skeletons: Opacity-Puls 1,6 s, nicht bei `reduced-motion`.

---

## 8. Barrierefreiheit (WCAG AA)

- Kontrast Text ≥ 4,5:1, große Text/Icons ≥ 3:1 (Tokens oben erfüllen das; wird in M3 mit axe geprüft).
- Vollständige Tastaturbedienung: `Ctrl/Cmd+K` Palette, `Esc` schließt, `↑/↓` in Listen, `Alt+↑/↓` Kanäle, `Ctrl+Shift+M` Mute, Fokus-Falle in Modals, Fokus-Rückgabe.
- Screenreader: `aria-live="polite"` für eingehende Nachrichten (gebündelt, nicht pro Nachricht bei Burst), `role="log"` für die Nachrichtenliste, Labels für alle Icon-Buttons, `aria-pressed` für Mute/Deafen.
- Voice: nie nur Farbe — Sprecher hat Ring **und** `aria-label="spricht"`.
- Formulare: Label immer sichtbar, Fehlertext per `aria-describedby`, kein Placeholder als Label.
- `lang="de"` am Dokument, Datumsformate über `Intl`.

---

## 9. Performance-Budget (Frontend)

| Posten | Budget |
|---|---|
| JS Kern-Route (gzip) | < 200 KB, Ziel 150 KB |
| CSS (gzip) | < 25 KB |
| Fonts | 48 KB (Inter) + 22 KB (Mono, lazy) |
| TTI auf Mid-Range-Mobile (4G) | < 1 s bis interaktiv mit Cache-First-Render |
| Nachrichten-Scroll | 60 fps bei 10.000 Zeilen im virtuellen Fenster |
| Command Palette öffnen | < 50 ms (kein Netzwerk, Index im Speicher) |

**Mittel:** `adapter-static`, Route-Splitting, Icons als Inline-SVG-Sprite (nur genutzte Icons), Virtualizer selbst (~120 Zeilen, kein Fremd-Paket), IndexedDB-Cache rendert vor dem ersten Netzwerk-Roundtrip, Bilder mit `loading="lazy"`, `content-visibility: auto` für Offscreen-Kacheln, kein Runtime-CSS, kein Polyfill-Bundle (nur moderne Browser).

---

## 10. Themes-Engine (Feature 6)

Themes sind JSON mit Token-Overrides (`--nx-accent`, `--nx-bg`, …), validiert gegen eine Allowlist (nur `--nx-*`, nur Farb-/Längenwerte), angewendet über `document.documentElement.style.setProperty`. Kein `innerHTML`, kein freies CSS von Nutzern (XSS/Exfiltrationsrisiko). Live-Vorschau zeigt Chat-Ausschnitt + Voice-Tile in einem 200 ms-Umschaltzyklus. Mitgeliefert: Dark, Light, AMOLED, „Nebula“ (Magenta-Akzent), „Forest“ (Grün-Akzent), „Paper“ (warmes Hell).
