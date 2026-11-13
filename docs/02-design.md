# Project Nova — Designsprache

**Status:** umgesetzt in `crates/ui/assets/app.css` und `crates/ui/src/main.rs` (v0.2.2).
Ruhige Flächen, eine Akzentfarbe, Inhalt zuerst. Keine Verläufe, Glows,
Backdrop-Blur oder dekorativen Schatten.

---

## 1. Leitidee

Nova ist ein Werkzeug, keine Bühne. Die drei Flächen (Server-Leiste, Kanäle,
Chat) liegen nah beieinander; Struktur entsteht durch dünne Linien und
Typografie, nicht durch Farbe oder Tiefe. Ein einziges ruhiges Violett-Blau
markiert Interaktion. Semantische Farben erscheinen nur als Status.

Dark Mode ist der Standard. Light Mode folgt automatisch
`prefers-color-scheme`; es gibt keinen manuellen Theme-Schalter.

---

## 2. Tokens

Alle Werte liegen in `crates/ui/assets/app.css`. Komponenten verwenden
ausschließlich Variablen, keine Hex-Werte.

```css
:root {
    --bg-rail: #0f1115;      /* Server-Leiste */
    --bg-sidebar: #13161c;   /* Kanäle / Mitglieder */
    --bg-main: #171b22;      /* Chat */
    --bg-raised: #1d222b;    /* Inputs, Karten, Voice-Dock */
    --border: rgba(255, 255, 255, 0.07);
    --text: #e8ebf1;
    --text-muted: #a0a8b7;
    --text-faint: #7d8797;
    --accent: #7b86f8;       /* genau eine Akzentfarbe */
    --green: #49b98a;
    --yellow: #d0a44c;
    --red: #df737d;
    --gray: #78818f;
    --radius-sm: 6px;
    --radius-md: 10px;
    --radius-lg: 16px;
}
```

Light Mode überschreibt dieselben Variablen in
`@media (prefers-color-scheme: light)` (weiße Hauptfläche, dunkle Schrift,
Akzent `#4f5bd5`).

Regeln:

- Drei Hintergrundstufen plus `--bg-raised` für interaktive Flächen.
- Drei Textstufen; Fließtext erreicht WCAG AA auf seiner Fläche.
- Status nie nur über Farbe: Punkt plus Text/`aria-label`.
- Kein Hex-Wert und kein `background: linear-gradient(...)` in Komponenten.
- Fokus: `outline: 2px solid var(--accent)` mit 2 px Offset.

---

## 3. Typografie

- Familie: `Inter` falls lokal installiert, sonst `Segoe UI Variable Text`,
  `Segoe UI`, `system-ui`. Keine Webfont-Anfrage an fremde Server.
- Skala: 11 / 12 / 13,5 / 14 / 15 / 19 / 22 / 24 px.
- Chat 14 px, Zeilenhöhe 1,45; Zeitstempel 10,5–11 px,
  `font-variant-numeric: tabular-nums`.
- Gewichte: 400 Regular, 600 Semibold. Keine 700+-Ausnahmen außer Wortmarke.
- Labels in Großbuchstaben nur für Gruppenüberschriften, nie für Fließtext.

---

## 4. Layout und Abstände

```
┌──────┬──────────────┬──────────────────────────────┬──────────────┐
│ 68   │ 248          │ Chat                          │ 240          │
│ Rail │ Kanäle       │ Header / Nachrichten / Composer│ Mitglieder   │
└──────┴──────────────┴──────────────────────────────┴──────────────┘
```

- 4-px-Raster; Radius 6 / 10 / 16 px.
- Server-Icons 42 px, rund; aktiv abgerundetes Quadrat plus Marker links.
- Avatare rund, 32 px im Chat, 26 px in Listen.
- Mitgliederpanel ab 1024 px sichtbar und per Button umschaltbar; darunter
  ausgeblendet, damit der Chat lesbar bleibt. (Die ursprüngliche Vorgabe
  „unter ca. 900 px“ wird bewusst etwas früher gezogen.)
- Unter 900 px: Rail 64 px, Kanäle 232 px, Mitglieder aus.
- Unter 700 px: eine Spalte; Kanäle als Drawer mit Scrim, Header mit
  Menü-Button.
- Composer bleibt am unteren Rand; Voice-Dock sitzt direkt darüber.

---

## 5. Komponenten

| Komponente | Zustände / Verhalten |
|---|---|
| Server-Icon | normal, Hover, aktiv mit Marker |
| Kanalgruppe | offen/geschlossen, `aria-expanded`, Chevron |
| Kanal | normal, Hover, aktiv, Live-Punkt, Voice-Avatare |
| Nachricht | normal, gruppiert (gleicher Autor, gleiche Minute), Hover |
| Composer | leer/gefüllt, Fokus, deaktivierter Senden-Button |
| Voice-Dock | verbunden/bereit, Mute, Deafen, Screenshare, Verlassen |
| Mitgliederpanel | Online/Offline, Statuspunkt mit Label, einklappbar |
| Command Palette | Strg/Cmd+K, Esc, Tippen, Enter, Aktionen |
| Modals | Fokus, Backdrop, Abbrechen/primäre Aktion |
| Toast | Fehler mit Schließen-Button |
| Auth | Login/Registrierung, Busy-Zustand, Fehlertext |

Icons sind Inline-SVG (`Icon`-Komponente in `main.rs`), keine Icon-Fonts und
keine externen Assets.

---

## 6. Bewegung

- 150 ms, `ease-out`; nur Hintergrund, Farbe, Transform und Opazität.
- Kein Scroll- oder Voice-Animationseffekt.
- `@media (prefers-reduced-motion: reduce)` setzt Übergänge und Animationen
  auf 0,01 ms.

---

## 7. Barrierefreiheit

- Vollständige Tab-Reihenfolge, sichtbarer Fokusring, Mindesthöhe 32 px.
- `Ctrl/Cmd+K` öffnet die Palette global; `Esc` schließt sie.
- Icon-Buttons haben `aria-label`; Mute/Deafen/Screenshare nutzen
  `aria-pressed`.
- Nachrichtenliste ist `role="log"` mit `aria-live="polite"`.
- Status wird als Punkt **und** Text (`Online`/`Offline`) ausgegeben.
- Farbe ist nie der einzige Bedeutungsträger.

---

## 8. Budget

| Posten | Ziel |
|---|---|
| WASM (unkomprimiert) | < 1 MB; aktuell ca. 763 KB |
| Handgeschriebenes CSS | eine Datei, ca. 25 KB |
| Externe Requests | 0 (keine Fonts/CDNs) |
| Handgeschriebenes JS | 0 |
| Interaktion | sofort; Palette arbeitet rein im Speicher |

---

## 9. Bekannte Design-Lücken

- Ungelesen-Badges und Reply-/Reaktionsdarstellung sind noch nicht an die
  vorhandenen Daten angebunden.
- Ein echter Ping-Wert wird im Voice-Dock noch nicht angezeigt.
- Die Sprecher-Hervorhebung nutzt noch keinen Audiopegel; sie ist für einen
  späteren RTP-Level-Ausbau vorbereitet.
