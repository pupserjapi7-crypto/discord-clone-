---
name: nova-ui-component
description: Vorlage und Konventionen für Dioxus-Komponenten der Nova-Web-UI – Props, Zustände, ARIA, CSS-Klassen, Ablage und Galerie-Eintrag. Laden beim Anlegen oder Ändern einer Komponente in crates/ui/src.
---

# Nova-UI-Komponente

## Ablage

- `crates/ui/src/components/` – wiederverwendbare Bausteine (`Icon`, `Avatar`,
  `Button`, `IconButton`, `Modal`, `Toast`, `StatusDot`, `Skeleton`, …), ein
  Modul pro Familie, Re-Export in `components/mod.rs`.
- `crates/ui/src/views/` – Bildschirmteile, die State lesen (Rail, Sidebar,
  Chat, VoiceStage, Members, Settings, Moderation, ThreadPanel, Palette).
- `crates/ui/src/gallery.rs` – nur mit Feature `gallery`; jede Komponente
  erscheint dort in allen Zuständen.

## Vorlage

```rust
#[derive(Props, Clone, PartialEq)]
pub struct ThingProps {
    label: String,
    #[props(default)]
    active: bool,
    #[props(default)]
    disabled: bool,
    onclick: EventHandler<MouseEvent>,
}

#[component]
pub fn Thing(props: ThingProps) -> Element {
    rsx! {
        button {
            class: classes(&["nv-thing", if props.active { "is-active" } else { "" }]),
            disabled: props.disabled,
            "aria-pressed": if props.active { "true" } else { "false" },
            onclick: move |event| props.onclick.call(event),
            "{props.label}"
        }
    }
}
```

## Konventionen

- CSS-Klassen: Block `nv-<name>`, Element `nv-<name>__<teil>`, Zustand
  `is-active`, `is-disabled`, `is-loading`, `is-speaking`. Keine Inline-Styles
  außer dynamischen Werten (z. B. Lautstärke-Slider).
- Props mit `PartialEq`, damit Dioxus unnötige Renders spart. Große Daten als
  `ReadOnlySignal`/`Rc`, nicht klonen.
- ARIA: Icon-Buttons immer `aria-label`; Toggles `aria-pressed`; Listen mit
  Auswahl `role="listbox"`/`option`; Dialoge `role="dialog"` +
  `aria-modal="true"` + Fokus ins erste Feld; Nachrichtenliste `role="log"`
  mit `aria-live="polite"`.
- Tastatur: alles per Tab erreichbar; Esc schließt Overlays; Enter löst die
  Hauptaktion aus.
- Status nie nur über Farbe (Punkt + Text oder `aria-label`).
- Nach der Änderung: Galerie-Eintrag ergänzen, `nova-design-system`-Checkliste,
  dann `nova-ui-build`.
