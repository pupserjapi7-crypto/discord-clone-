//! Wiederverwendbare Bausteine. Klassen-Konvention: `nv-<block>`,
//! `nv-<block>__<teil>`, Zustände `is-…` (siehe Skill `nova-ui-component`).

use dioxus::prelude::*;
use wasm_bindgen::JsCast;

/// Klassen zusammensetzen, leere Teile überspringen.
pub fn classes(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn initials(name: &str) -> String {
    let letters: String = name
        .split(|c: char| c.is_whitespace() || c == '_' || c == '.')
        .filter_map(|part| part.chars().next())
        .take(2)
        .collect();
    if letters.is_empty() {
        "?".into()
    } else {
        letters.to_uppercase()
    }
}

/// Stabile, ruhige Avatar-Tönung aus dem Namen (eine von sechs Stufen).
fn tone(seed: &str) -> u32 {
    seed.bytes().fold(7u32, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as u32)
    }) % 6
}

/// Lucide-artige Linien-Icons, 24er-Raster, 1,75 px Strich.
pub fn icon_path(name: &str) -> &'static str {
    match name {
        "hash" => "M4 9h16M4 15h16M10 3 8 21M16 3l-2 18",
        "volume" => "M11 5 6 9H2v6h4l5 4V5zM15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14",
        "mic" => {
            "M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3zM19 10v2a7 7 0 0 1-14 0v-2M12 19v3"
        }
        "mic-off" => {
            "M2 2l20 20M18.9 13.2A7 7 0 0 0 19 12v-2M5 10v2a7 7 0 0 0 12 5M15 9.3V5a3 3 0 0 0-5.7-1.3M9 9v3a3 3 0 0 0 5.1 2.1M12 19v3"
        }
        "headphones" => {
            "M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"
        }
        "headphones-off" => {
            "M2 2l20 20M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 15.3-6.4M21 12v7a2 2 0 0 1-.4 1.2M16 16v-.5"
        }
        "screen" => "M2 4h20v12H2zM8 20h8M12 16v4M9 10l3-3 3 3M12 7v6",
        "video" => "M15 10l5-3v10l-5-3zM2 6h13v12H2z",
        "phone-off" => {
            "M10.7 13.3a16 16 0 0 0 3.4 2.6l1.3-1.3a2 2 0 0 1 2.1-.4 12.8 12.8 0 0 0 2.8.7 2 2 0 0 1 1.7 2v3a2 2 0 0 1-2.2 2 19.8 19.8 0 0 1-8.6-3.1 19.4 19.4 0 0 1-3.3-2.7m-2.7-3.3a19.8 19.8 0 0 1-3.1-8.6A2 2 0 0 1 4.1 2h3a2 2 0 0 1 2 1.7 12.8 12.8 0 0 0 .7 2.8 2 2 0 0 1-.4 2.1L8.1 9.9M22 2 2 22"
        }
        "settings" => {
            "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1A1.7 1.7 0 0 0 9 19.4a1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1A1.7 1.7 0 0 0 4.6 9a1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"
        }
        "users" => {
            "M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8M23 21v-2a4 4 0 0 0-3-3.9M16 3.1a4 4 0 0 1 0 7.8"
        }
        "user" => "M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2M12 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8z",
        "plus" => "M12 5v14M5 12h14",
        "search" => "M21 21l-4.3-4.3M19 11a8 8 0 1 1-16 0 8 8 0 0 1 16 0",
        "chevron" => "M6 9l6 6 6-6",
        "close" => "M18 6 6 18M6 6l12 12",
        "menu" => "M4 7h16M4 12h16M4 17h16",
        "user-plus" => {
            "M16 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M8.5 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8M20 8v6M23 11h-6"
        }
        "logout" => "M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9",
        "pin" => {
            "M12 17v5M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.2v.8a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.8a2 2 0 0 0-1.1-1.8l-1.8-.9A2 2 0 0 1 15 10.8V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"
        }
        "thread" => "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2zM8 9h8M8 13h5",
        "smile" => {
            "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM8 14s1.5 2 4 2 4-2 4-2M9 9h.01M15 9h.01"
        }
        "archive" => "M3 3h18v5H3zM5 8v11a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8M10 12h4",
        "shield" => "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z",
        "trash" => {
            "M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"
        }
        "more" => {
            "M12 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2zM19 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2zM5 13a1 1 0 1 0 0-2 1 1 0 0 0 0 2z"
        }
        "clock" => "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 6v6l4 2",
        "ban" => "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM4.9 4.9l14.2 14.2",
        "check" => "M20 6 9 17l-5-5",
        "alert" => "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 8v4M12 16h.01",
        "bell" => "M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10.3 21a1.9 1.9 0 0 0 3.4 0",
        "keyboard" => "M2 6h20v12H2zM6 10h.01M10 10h.01M14 10h.01M18 10h.01M8 14h8",
        "contrast" => "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 2v20",
        "logo" => "M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z",
        "arrow-down" => "M12 5v14M19 12l-7 7-7-7",
        "reply" => "M9 17l-5-5 5-5M20 18v-2a4 4 0 0 0-4-4H4",
        "signal" => "M2 20h.01M7 20v-4M12 20v-8M17 20V8",
        "refresh" => "M21 12a9 9 0 0 1-15 6.7L3 16M3 12a9 9 0 0 1 15-6.7L21 8M21 3v5h-5M3 21v-5h5",
        "inbox" => {
            "M22 12h-6l-2 3h-4l-2-3H2M5.5 5.1 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.5-6.9A2 2 0 0 0 16.8 4H7.2a2 2 0 0 0-1.7 1.1z"
        }
        "edit" => "M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z",
        "bot" => "M12 8V4H8M4 8h16v12H4zM2 14h2M20 14h2M9 13v2M15 13v2",
        "file" => {
            "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zM14 2v6h6M8 13h8M8 17h5"
        }
        "download" => "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3",
        "upload" => "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12",
        "image" => "M3 3h18v18H3zM8.5 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 15l-5-5L5 21",
        _ => "M12 3v18M3 12h18",
    }
}

#[component]
pub fn Icon(name: String, #[props(default = 18)] size: u32) -> Element {
    let path = icon_path(&name);
    rsx! {
        svg {
            class: "nv-icon",
            width: "{size}",
            height: "{size}",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.75",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: path }
        }
    }
}

#[component]
pub fn Avatar(
    name: String,
    #[props(default = 32)] size: u32,
    #[props(default)] speaking: bool,
    /// `None` = kein Statuspunkt.
    #[props(default)]
    status: Option<i16>,
    #[props(default)] bot: bool,
) -> Element {
    let class = classes(&[
        "nv-avatar",
        &format!("nv-avatar--t{}", tone(&name)),
        if speaking { "is-speaking" } else { "" },
    ]);
    let font = (size as f32 * 0.38).round();
    rsx! {
        span {
            class: "{class}",
            style: "--size: {size}px; font-size: {font}px",
            "aria-hidden": "true",
            "{initials(&name)}"
            if let Some(status) = status {
                StatusDot { status, ring: true }
            }
            if bot {
                span { class: "nv-avatar__bot", "Bot" }
            }
        }
    }
}

/// Statuspunkt. Bedeutung steht zusätzlich im `aria-label` bzw. im Text daneben.
#[component]
pub fn StatusDot(status: i16, #[props(default)] ring: bool) -> Element {
    let (class, label) = status_info(status);
    rsx! {
        span {
            class: classes(&["nv-status", class, if ring { "nv-status--ring" } else { "" }]),
            role: "img",
            "aria-label": label,
        }
    }
}

pub fn status_info(status: i16) -> (&'static str, &'static str) {
    // Wie `PATCH /users/@me`: 0 online, 1 abwesend, 2 nicht stören, 3 unsichtbar.
    match status {
        0 => ("is-online", "Online"),
        1 => ("is-idle", "Abwesend"),
        2 => ("is-dnd", "Bitte nicht stören"),
        _ => ("is-offline", "Offline"),
    }
}

#[component]
pub fn IconButton(
    icon: String,
    label: String,
    #[props(default)] active: bool,
    #[props(default)] danger: bool,
    #[props(default)] large: bool,
    #[props(default)] pressed: Option<bool>,
    #[props(default)] disabled: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let class = classes(&[
        "nv-iconbtn",
        if active { "is-active" } else { "" },
        if danger { "is-danger" } else { "" },
        if large { "nv-iconbtn--lg" } else { "" },
    ]);
    let size = if large { 20 } else { 18 };
    rsx! {
        button {
            class: "{class}",
            r#type: "button",
            title: "{label}",
            "aria-label": "{label}",
            "aria-pressed": pressed.map(|p| if p { "true" } else { "false" }),
            disabled,
            onclick: move |event| onclick.call(event),
            Icon { name: icon, size }
        }
    }
}

#[component]
pub fn Kbd(children: Element) -> Element {
    rsx! { kbd { class: "nv-kbd", {children} } }
}

#[component]
pub fn Skeleton(#[props(default = 6)] lines: usize) -> Element {
    rsx! {
        div { class: "nv-skeleton", "aria-busy": "true", "aria-label": "Lädt",
            for index in 0..lines {
                div { key: "{index}", class: "nv-skeleton__row",
                    span { class: "nv-skeleton__avatar" }
                    span { class: "nv-skeleton__lines",
                        span { class: "nv-skeleton__line", style: "width: {28 + (index * 17) % 30}%" }
                        span { class: "nv-skeleton__line", style: "width: {55 + (index * 23) % 40}%" }
                    }
                }
            }
        }
    }
}

/// Leer-/Fehlerzustand mit klarer nächster Handlung.
#[component]
pub fn EmptyState(
    icon: String,
    title: String,
    text: String,
    #[props(default)] children: Element,
) -> Element {
    rsx! {
        div { class: "nv-empty",
            span { class: "nv-empty__icon", Icon { name: icon, size: 22 } }
            h2 { class: "nv-empty__title", "{title}" }
            p { class: "nv-empty__text", "{text}" }
            div { class: "nv-empty__actions", {children} }
        }
    }
}

/// Fokus auf das erste Eingabefeld eines Containers setzen.
pub fn focus_first(container_id: &str) {
    let Some(element) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(container_id))
    else {
        return;
    };
    let target = element
        .query_selector("input, textarea, select, button.nv-btn--primary")
        .ok()
        .flatten()
        .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok());
    if let Some(target) = target {
        let _ = target.focus();
    }
}

/// Hinweis für Overlays: Abläufe, die den Dialog schließen, mit
/// `spawn_forever` starten – mit `spawn` gehört die Aufgabe dem Dialog und
/// bricht ab, sobald er sich schließt.
#[component]
pub fn Modal(
    id: String,
    title: String,
    #[props(default)] description: Option<String>,
    #[props(default)] wide: bool,
    onclose: EventHandler<()>,
    children: Element,
) -> Element {
    let focus_id = id.clone();
    use_effect(move || focus_first(&focus_id));
    let title_id = format!("{id}-title");
    rsx! {
        div {
            class: "nv-backdrop",
            onclick: move |_| onclose.call(()),
            div {
                id: "{id}",
                class: classes(&["nv-modal", if wide { "nv-modal--wide" } else { "" }]),
                role: "dialog",
                "aria-modal": "true",
                "aria-labelledby": "{title_id}",
                onclick: move |event| event.stop_propagation(),
                onkeydown: move |event| {
                    if event.key() == Key::Escape {
                        onclose.call(());
                    }
                },
                header { class: "nv-modal__head",
                    h2 { id: "{title_id}", class: "nv-modal__title", "{title}" }
                    IconButton { icon: "close", label: "Schließen", onclick: move |_| onclose.call(()) }
                }
                if let Some(text) = description {
                    p { class: "nv-modal__text", "{text}" }
                }
                {children}
            }
        }
    }
}

/// Uhrzeit in Ortszeit („14:05“) aus einem ISO-Zeitstempel.
pub fn time_label(iso: &str) -> String {
    let date = js_sys::Date::new(&iso.into());
    if date.get_time().is_nan() {
        return String::new();
    }
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}

/// „Heute“, „Gestern“ oder Datum – für Trenner und Tooltips.
pub fn day_label(iso: &str) -> String {
    let date = js_sys::Date::new(&iso.into());
    if date.get_time().is_nan() {
        return String::new();
    }
    let today = js_sys::Date::new_0();
    let start = |d: &js_sys::Date| {
        js_sys::Date::new_with_year_month_day(
            d.get_full_year(),
            d.get_month() as i32,
            d.get_date() as i32,
        )
        .get_time()
    };
    let days = ((start(&today) - start(&date)) / 86_400_000.0).round() as i64;
    match days {
        0 => "Heute".into(),
        1 => "Gestern".into(),
        _ => format!(
            "{:02}.{:02}.{}",
            date.get_date(),
            date.get_month() + 1,
            date.get_full_year()
        ),
    }
}

pub fn minutes(iso: &str) -> f64 {
    js_sys::Date::new(&iso.into()).get_time() / 60_000.0
}
