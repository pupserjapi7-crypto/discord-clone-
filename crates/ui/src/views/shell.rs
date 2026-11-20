//! Grundgerüst: Server-Leiste, Kanalspalte, Hauptbereich, rechte Spalte,
//! Overlays, globale Tastenkürzel.

use dioxus::prelude::*;
use wasm_bindgen::{JsCast, closure::Closure};

use crate::{
    actions,
    components::{Avatar, EmptyState, Icon, IconButton, Kbd, Skeleton, classes},
    store::{ConnStatus, Overlay, ServerTab, SettingsTab, Store, use_store},
    types::*,
    views::{chat, overlays, voice as voice_view},
};

#[component]
pub fn Shell() -> Element {
    let store = use_store();
    use_global_shortcuts(store);
    let members_open = store.members_open;
    let nav_open = store.nav_open;
    let thread_open = store.thread_id.read().is_some();
    let has_detail = store.detail.read().is_some();
    let right = (members_open() && has_detail) || thread_open;
    let class = classes(&[
        "nv-app",
        if right { "" } else { "no-members" },
        if thread_open { "has-thread" } else { "" },
        if nav_open() { "nav-open" } else { "" },
    ]);
    // Dateien über das Fenster ziehen: Zähler statt Flag, weil `dragenter`/
    // `dragleave` bei jedem Kindelement feuern.
    let drag_depth = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(0i32)));
    let enter_depth = std::rc::Rc::clone(&drag_depth);
    let leave_depth = std::rc::Rc::clone(&drag_depth);
    let drop_depth = std::rc::Rc::clone(&drag_depth);
    let mut dragging = store.dragging;
    rsx! {
        div {
            class: "{class}",
            ondragenter: move |_| {
                enter_depth.set(enter_depth.get() + 1);
                if !*dragging.peek() {
                    dragging.set(true);
                }
            },
            ondragleave: move |_| {
                leave_depth.set((leave_depth.get() - 1).max(0));
                if leave_depth.get() == 0 {
                    dragging.set(false);
                }
            },
            // Abgelegte Dateien außerhalb des Chats öffnen sonst die Datei
            // im Tab und die App ist weg.
            ondragover: move |event: DragEvent| event.prevent_default(),
            ondrop: move |event: DragEvent| {
                event.prevent_default();
                drop_depth.set(0);
                dragging.set(false);
            },
            ServerRail {}
            ChannelSidebar {}
            div { class: "nv-scrim", onclick: move |_| store.nav_open.clone().set(false) }
            MainArea {}
            if thread_open {
                chat::ThreadPanel {}
            } else if has_detail && members_open() {
                MemberList {}
            }
        }
        overlays::OverlayHost {}
        Toast {}
    }
}

/// Strg/⌘+K, Esc, Push-to-Talk, Strg+Umschalt+M (stumm); dazu der
/// Ungelesen-Stand, wenn das Fenster wieder in den Vordergrund kommt (gelesen
/// wurde vielleicht auf einem anderen Gerät).
fn use_global_shortcuts(store: Store) {
    use_hook(move || {
        let mut store = store;
        let keydown = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |event: web_sys::KeyboardEvent| {
                let ctrl = event.ctrl_key() || event.meta_key();
                if ctrl && event.key().eq_ignore_ascii_case("k") {
                    event.prevent_default();
                    let open = *store.overlay.read() == Overlay::Palette;
                    store.overlay.set(if open {
                        Overlay::None
                    } else {
                        Overlay::Palette
                    });
                    return;
                }
                if ctrl && event.shift_key() && event.key().eq_ignore_ascii_case("m") {
                    event.prevent_default();
                    if store.voice.read().joined {
                        spawn(actions::toggle_mute(store));
                    }
                    return;
                }
                if event.key() == "Escape" {
                    if *store.overlay.read() != Overlay::None {
                        store.overlay.set(Overlay::None);
                    } else if store.pins.read().is_some() {
                        store.pins.set(None);
                    } else if store.nav_open.read().to_owned() {
                        store.nav_open.set(false);
                    }
                    return;
                }
                push_to_talk(store, &event, true);
            },
        );
        let keyup = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |event: web_sys::KeyboardEvent| {
                push_to_talk(store, &event, false);
            },
        );
        let focus = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if store.user.read().is_some() {
                actions::load_unread_throttled(store);
            }
        });
        if let Some(window) = web_sys::window() {
            let _ = window
                .add_event_listener_with_callback("keydown", keydown.as_ref().unchecked_ref());
            let _ =
                window.add_event_listener_with_callback("keyup", keyup.as_ref().unchecked_ref());
            let _ =
                window.add_event_listener_with_callback("focus", focus.as_ref().unchecked_ref());
        }
        // Lebt so lange wie die App; ein Fenster, je ein Listener.
        keydown.forget();
        keyup.forget();
        focus.forget();
    });
}

fn in_text_field(event: &web_sys::KeyboardEvent) -> bool {
    event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .map(|element| {
            let tag = element.tag_name().to_ascii_lowercase();
            tag == "input" || tag == "textarea" || element.has_attribute("contenteditable")
        })
        .unwrap_or(false)
}

fn push_to_talk(mut store: Store, event: &web_sys::KeyboardEvent, down: bool) {
    let (enabled, key) = {
        let settings = store.settings.read();
        (settings.push_to_talk, settings.ptt_key.clone())
    };
    if !enabled || event.code() != key || !store.voice.read().joined {
        return;
    }
    if down && in_text_field(event) {
        return;
    }
    if event.repeat() {
        return;
    }
    event.prevent_default();
    let deafened = store.voice.read().deafened;
    store.voice.write().ptt_down = down;
    if !deafened {
        crate::voice::set_muted(!down);
    }
}

// ---------------------------------------------------------------- Leiste

#[component]
fn ServerRail() -> Element {
    let mut store = use_store();
    let guilds = store.guilds.read().clone();
    let current = store.detail.read().as_ref().map(|d| d.guild.id.clone());
    let unread = store.guild_unread.read().clone();
    rsx! {
        nav { class: "nv-rail", "aria-label": "Server",
            span { class: "nv-rail__home", title: "Nova", Icon { name: "logo", size: 22 } }
            span { class: "nv-rail__sep" }
            for guild in guilds {
                {
                    let active = current.as_deref() == Some(guild.id.as_str());
                    let id = guild.id.clone();
                    let state = unread.get(&guild.id);
                    let mentions = state.map_or(0, |state| state.mentions);
                    // Im offenen Server zeigt die Kanalliste, was neu ist.
                    let dot = !active && mentions == 0 && state.is_some_and(|state| state.unread);
                    let label = rail_label(&guild.name, mentions, dot);
                    rsx! {
                        div { key: "{guild.id}", class: classes(&["nv-rail__item", if active { "is-active" } else { "" }]),
                            button {
                                class: "nv-rail__btn",
                                title: "{label}",
                                "aria-label": "{label}",
                                "aria-current": if active { "page" } else { "false" },
                                onclick: move |_| {
                                    let id = id.clone();
                                    spawn(actions::open_guild(store, id));
                                },
                                "{crate::components::initials(&guild.name)}"
                            }
                            if mentions > 0 {
                                span { class: "nv-count nv-rail__count", "aria-hidden": "true",
                                    if mentions > 99 { "99+" } else { "{mentions}" }
                                }
                            } else if dot {
                                span { class: "nv-rail__dot", "aria-hidden": "true" }
                            }
                        }
                    }
                }
            }
            div { class: "nv-rail__item",
                button {
                    class: "nv-rail__btn nv-rail__btn--add",
                    title: "Server erstellen oder beitreten",
                    "aria-label": "Server erstellen oder beitreten",
                    onclick: move |_| store.overlay.set(Overlay::CreateGuild),
                    Icon { name: "plus", size: 20 }
                }
            }
        }
    }
}

/// Name plus Ungelesen-Stand für Screenreader und Tooltip (Punkt und Zahl
/// selbst sind `aria-hidden`).
fn rail_label(name: &str, mentions: i64, unread: bool) -> String {
    match mentions {
        0 if unread => format!("{name}, ungelesen"),
        0 => name.to_owned(),
        1 => format!("{name}, 1 Erwähnung"),
        n => format!("{name}, {n} Erwähnungen"),
    }
}

// --------------------------------------------------------- Kanalspalte

#[component]
fn ChannelSidebar() -> Element {
    let mut store = use_store();
    let mut text_open = use_signal(|| true);
    let mut voice_open = use_signal(|| true);
    let detail = store.detail.read().clone();
    let current = store.channel_id.read().clone();

    let Some(detail) = detail else {
        return rsx! {
            aside { class: "nv-side",
                div { class: "nv-side__head", h1 { class: "nv-side__title", "Nova" } }
                div { class: "nv-side__scroll" }
                VoiceConnected {}
                UserPanel {}
            }
        };
    };
    let can_manage = detail.can(perm::MANAGE_CHANNELS);
    let can_invite = detail.can(perm::CREATE_INVITE);
    let can_admin = detail.can(perm::MANAGE_GUILD)
        || detail.can(perm::MANAGE_ROLES)
        || detail.can(perm::BAN_MEMBERS)
        || detail.can(perm::KICK_MEMBERS);
    let text: Vec<Channel> = detail
        .channels
        .iter()
        .filter(|c| c.kind == KIND_TEXT || c.kind == KIND_FORUM)
        .cloned()
        .collect();
    let voice: Vec<Channel> = detail
        .channels
        .iter()
        .filter(|c| c.kind == KIND_VOICE)
        .cloned()
        .collect();
    let threads: Vec<Channel> = detail
        .channels
        .iter()
        .filter(|c| c.kind == KIND_THREAD && !c.archived)
        .cloned()
        .collect();
    let unread = detail.unread.clone();
    let is_unread = move |id: &str| unread.iter().any(|u| u.channel_id == id && u.is_unread());
    let rooms = store.voice_rooms.read().clone();
    let speaking = store.voice.read().speaking.clone();
    let thread_id = store.thread_id.read().clone();

    rsx! {
        aside { class: "nv-side", "aria-label": "Kanäle",
            div { class: "nv-side__head",
                h1 { class: "nv-side__title", "{detail.guild.name}" }
                div { class: "nv-row",
                    if can_invite {
                        IconButton {
                            icon: "user-plus",
                            label: "Leute einladen",
                            onclick: move |_| {
                                store.overlay.set(Overlay::Invite(None));
                                let id = actions::guild_id(store);
                                spawn(async move {
                                    let Some(id) = id else { return };
                                    match crate::api::create_invite(&id).await {
                                        Ok(invite) => {
                                            if matches!(*store.overlay.read(), Overlay::Invite(_)) {
                                                store.overlay.set(Overlay::Invite(Some(invite.code)));
                                            }
                                        }
                                        Err(err) => store.error(err),
                                    }
                                });
                            },
                        }
                    }
                    if can_admin {
                        IconButton {
                            icon: "shield",
                            label: "Server-Einstellungen",
                            onclick: move |_| store.overlay.set(Overlay::ServerSettings(ServerTab::Overview)),
                        }
                    }
                }
            }
            div { class: "nv-side__scroll",
                div { class: "nv-group",
                    div { class: "nv-group__head",
                        button {
                            class: classes(&["nv-group__label", if text_open() { "" } else { "is-collapsed" }]),
                            "aria-expanded": if text_open() { "true" } else { "false" },
                            onclick: move |_| text_open.toggle(),
                            Icon { name: "chevron", size: 14 }
                            "Text"
                        }
                        if can_manage {
                            IconButton { icon: "plus", label: "Textkanal anlegen", onclick: move |_| store.overlay.set(Overlay::CreateChannel(KIND_TEXT)) }
                        }
                    }
                    if text_open() {
                        for channel in text {
                            {
                                let active = current.as_deref() == Some(channel.id.as_str());
                                let unread = !active && is_unread(&channel.id);
                                let id = channel.id.clone();
                                // Forum-Beiträge stehen im Forum selbst, nicht in der Leiste.
                                let child_threads: Vec<Channel> = if channel.kind == KIND_FORUM {
                                    Vec::new()
                                } else {
                                    threads.iter().filter(|t| t.parent_id.as_deref() == Some(channel.id.as_str())).cloned().collect()
                                };
                                let icon = if channel.kind == KIND_FORUM { "forum" } else { "hash" };
                                rsx! {
                                    button {
                                        key: "{channel.id}",
                                        class: classes(&["nv-channel", if active { "is-active" } else { "" }, if unread { "is-unread" } else { "" }]),
                                        "aria-current": if active { "page" } else { "false" },
                                        onclick: move |_| {
                                            let id = id.clone();
                                            spawn(actions::open_channel(store, id));
                                        },
                                        Icon { name: icon, size: 18 }
                                        span { class: "nv-channel__name", "{channel.name}" }
                                        if unread {
                                            span { class: "nv-channel__dot", role: "img", "aria-label": "Ungelesen" }
                                        }
                                    }
                                    for thread in child_threads {
                                        {
                                            let open = thread_id.as_deref() == Some(thread.id.as_str());
                                            let tid = thread.id.clone();
                                            rsx! {
                                                button {
                                                    key: "{thread.id}",
                                                    class: classes(&["nv-channel", "nv-channel--thread", if open { "is-active" } else { "" }]),
                                                    onclick: move |_| {
                                                        let tid = tid.clone();
                                                        spawn(actions::open_thread(store, tid));
                                                    },
                                                    Icon { name: "thread", size: 16 }
                                                    span { class: "nv-channel__name", "{thread.name}" }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "nv-group",
                    div { class: "nv-group__head",
                        button {
                            class: classes(&["nv-group__label", if voice_open() { "" } else { "is-collapsed" }]),
                            "aria-expanded": if voice_open() { "true" } else { "false" },
                            onclick: move |_| voice_open.toggle(),
                            Icon { name: "chevron", size: 14 }
                            "Sprache"
                        }
                        if can_manage {
                            IconButton { icon: "plus", label: "Sprachkanal anlegen", onclick: move |_| store.overlay.set(Overlay::CreateChannel(KIND_VOICE)) }
                        }
                    }
                    if voice_open() {
                        for channel in voice {
                            {
                                let active = current.as_deref() == Some(channel.id.as_str());
                                let id = channel.id.clone();
                                let people = rooms.get(&channel.id).cloned().unwrap_or_default();
                                rsx! {
                                    button {
                                        key: "{channel.id}",
                                        class: classes(&["nv-channel", if active { "is-active" } else { "" }]),
                                        onclick: move |_| {
                                            let id = id.clone();
                                            spawn(actions::open_channel(store, id));
                                        },
                                        Icon { name: "volume", size: 18 }
                                        span { class: "nv-channel__name", "{channel.name}" }
                                        if !people.is_empty() {
                                            span { class: "nv-faint nv-num", "{people.len()}" }
                                        }
                                    }
                                    if !people.is_empty() {
                                        // Auf einen Blick: wer da ist und wer gerade redet.
                                        div { class: "nv-voice-row", "aria-hidden": "true",
                                            for person in people.iter().take(6) {
                                                Avatar { key: "{person.user_id}", name: person.display_name.clone(), size: 20, speaking: speaking.contains(&person.user_id) }
                                            }
                                            if people.len() > 6 {
                                                span { class: "nv-voice-row__more", "+{people.len() - 6}" }
                                            }
                                        }
                                        div { class: "nv-voice-people",
                                            for person in people {
                                                div {
                                                    key: "{person.user_id}",
                                                    class: classes(&["nv-voice-person", if speaking.contains(&person.user_id) { "is-speaking" } else { "" }]),
                                                    span { "{person.display_name}" }
                                                    if speaking.contains(&person.user_id) {
                                                        span { class: "sr-only", "spricht" }
                                                    }
                                                    if person.deafened {
                                                        Icon { name: "headphones-off", size: 14 }
                                                    } else if person.muted {
                                                        Icon { name: "mic-off", size: 14 }
                                                    }
                                                    if person.screen {
                                                        span { class: "nv-badge nv-badge--accent", "Live" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            VoiceConnected {}
            UserPanel {}
        }
    }
}

fn ping_class(ping: Option<u32>) -> &'static str {
    match ping {
        Some(ms) if ms < 60 => "nv-ping is-good",
        Some(ms) if ms < 120 => "nv-ping is-ok",
        Some(_) => "nv-ping is-bad",
        None => "nv-ping",
    }
}

/// Kompakte „Verbunden“-Leiste über dem Nutzerpanel.
#[component]
fn VoiceConnected() -> Element {
    let store = use_store();
    let voice = store.voice.read().clone();
    let Some(channel) = voice.channel.clone() else {
        return rsx! {};
    };
    let ping = voice.ping_ms;
    rsx! {
        div { class: "nv-connected", role: "status",
            div { class: "nv-connected__text",
                div { class: if voice.connecting { "nv-connected__state is-connecting" } else { "nv-connected__state" },
                    if voice.connecting { "Verbinde …" } else { "Sprache verbunden" }
                    if !voice.connecting {
                        span { class: ping_class(ping), title: "Ping",
                            if let Some(ms) = ping { "{ms} ms" } else { "– ms" }
                        }
                    }
                }
                button {
                    class: "nv-connected__channel",
                    onclick: move |_| {
                        let id = channel.id.clone();
                        spawn(actions::open_channel(store, id));
                    },
                    "{channel.name}"
                }
            }
            IconButton {
                icon: "phone-off",
                label: "Auflegen",
                danger: true,
                onclick: move |_| { spawn(actions::leave_voice(store)); },
            }
        }
    }
}

#[component]
fn UserPanel() -> Element {
    let mut store = use_store();
    let user = store.user.read().clone();
    let status = *store.status.read();
    let voice = store.voice.read().clone();
    let Some(user) = user else { return rsx! {} };
    let (dot, label) = match status {
        ConnStatus::Online => (0, "Online"),
        ConnStatus::Connecting => (1, "Verbinde …"),
        ConnStatus::Offline => (3, "Offline"),
    };
    rsx! {
        div { class: "nv-me",
            Avatar { name: user.display_name.clone(), size: 32, status: Some(dot), speaking: voice.speaking.contains(&user.id) }
            div { class: "nv-me__info",
                div { class: "nv-me__name", "{user.display_name}" }
                div { class: "nv-me__status", "{label}" }
            }
            if voice.joined {
                IconButton {
                    icon: if voice.muted { "mic-off" } else { "mic" },
                    label: if voice.muted { "Stummschaltung aufheben" } else { "Stummschalten" },
                    active: voice.muted,
                    pressed: Some(voice.muted),
                    onclick: move |_| { spawn(actions::toggle_mute(store)); },
                }
            }
            IconButton {
                icon: "settings",
                label: "Einstellungen",
                onclick: move |_| store.overlay.set(Overlay::Settings(SettingsTab::Profile)),
            }
        }
    }
}

// ---------------------------------------------------------- Hauptbereich

#[component]
fn MainArea() -> Element {
    let mut store = use_store();
    let loaded = *store.guilds_loaded.read();
    let has_guilds = !store.guilds.read().is_empty();
    let loading = *store.guild_loading.read();
    let channel = store.current_channel();
    let status = *store.status.read();
    // Das Dock schwebt nur über der Voice-Ansicht; im Textkanal genügt die
    // „Verbunden“-Leiste in der Kanalspalte (sonst verdeckt es den Composer).
    let voice_joined = store.voice.read().channel.is_some()
        && channel.as_ref().is_some_and(|c| c.kind == KIND_VOICE);

    let content = if !loaded || (loading && channel.is_none()) {
        rsx! {
            Header { channel: None }
            Skeleton { lines: 7 }
        }
    } else if !has_guilds {
        rsx! { Onboarding {} }
    } else {
        match channel {
            Some(channel) if channel.kind == KIND_VOICE => rsx! {
                Header { channel: Some(channel.clone()) }
                voice_view::VoiceStage { channel }
            },
            Some(channel) if channel.kind == KIND_FORUM => rsx! {
                Header { channel: Some(channel.clone()) }
                super::forum::ForumView { channel }
            },
            Some(channel) => rsx! {
                Header { channel: Some(channel.clone()) }
                chat::ChatPane { channel_id: channel.id.clone(), in_thread: false }
            },
            None => rsx! {
                Header { channel: None }
                EmptyState {
                    icon: "hash",
                    title: "Kein Kanal ausgewählt",
                    text: "Wähle links einen Kanal – oder lege einen neuen an.",
                    button { class: "nv-btn nv-btn--primary", onclick: move |_| store.overlay.set(Overlay::CreateChannel(KIND_TEXT)), "Kanal anlegen" }
                }
            },
        }
    };
    rsx! {
        main { class: "nv-main",
            if status == ConnStatus::Offline && store.user.read().is_some() {
                div { class: "nv-banner is-warning", role: "status",
                    Icon { name: "alert", size: 16 }
                    "Verbindung getrennt – Nova verbindet sich automatisch neu."
                }
            }
            {content}
            if voice_joined {
                voice_view::VoiceDock {}
            }
        }
    }
}

#[component]
fn Header(channel: Option<Channel>) -> Element {
    let mut store = use_store();
    let members_open = *store.members_open.read();
    let has_detail = store.detail.read().is_some();
    let pins_open = store.pins.read().is_some();
    let is_text = channel
        .as_ref()
        .map(|c| c.kind == KIND_TEXT || c.kind == KIND_THREAD)
        .unwrap_or(false);
    rsx! {
        header { class: "nv-header",
            button {
                class: "nv-iconbtn nv-nav-toggle",
                "aria-label": "Navigation öffnen",
                onclick: move |_| store.nav_open.set(true),
                Icon { name: "menu" }
            }
            if let Some(channel) = &channel {
                span { class: "nv-header__icon",
                    Icon { name: match channel.kind { KIND_VOICE => "volume", KIND_FORUM => "forum", _ => "hash" }, size: 20 }
                }
                h2 { class: "nv-header__title", "{channel.name}" }
                if channel.slowmode_secs > 0 {
                    span { class: "nv-badge", title: "Slowmode", Icon { name: "clock", size: 12 } " {channel.slowmode_secs}s" }
                }
                if let Some(topic) = channel.topic.clone().filter(|t| !t.is_empty()) {
                    span { class: "nv-header__topic", title: "{topic}", "{topic}" }
                } else {
                    span { class: "nv-header__spacer" }
                }
            } else {
                span { class: "nv-header__spacer" }
            }
            div { class: "nv-header__actions",
                if is_text {
                    IconButton {
                        icon: "pin",
                        label: "Angepinnte Nachrichten",
                        active: pins_open,
                        pressed: Some(pins_open),
                        onclick: move |_| {
                            if store.pins.read().is_some() {
                                store.pins.set(None);
                            } else {
                                spawn(actions::load_pins(store));
                            }
                        },
                    }
                }
                if has_detail {
                    IconButton {
                        icon: "users",
                        label: if members_open { "Mitglieder ausblenden" } else { "Mitglieder einblenden" },
                        active: members_open,
                        pressed: Some(members_open),
                        onclick: move |_| {
                            let open = !*store.members_open.read();
                            store.members_open.set(open);
                            store.thread_id.set(None);
                            store.settings.write().show_members = open;
                            crate::settings::save(&store.settings.read());
                        },
                    }
                }
                button {
                    class: "nv-search",
                    "aria-label": "Suchen und springen (Strg+K)",
                    onclick: move |_| store.overlay.set(Overlay::Palette),
                    Icon { name: "search", size: 16 }
                    span { "Suchen" }
                    Kbd { "Strg K" }
                }
            }
        }
        if pins_open {
            chat::PinsPopover {}
        }
    }
}

#[component]
fn Onboarding() -> Element {
    let mut store = use_store();
    let name = store
        .user
        .read()
        .as_ref()
        .map(|u| u.display_name.clone())
        .unwrap_or_default();
    rsx! {
        div { class: "nv-onboard",
            h1 { "Hallo {name} 👋" }
            p { "Nova ist für Gruppen gebaut, die zusammen spielen und reden. Fang mit einem Server an:" }
            div { class: "nv-onboard__cards",
                button { class: "nv-onboard__card", onclick: move |_| store.overlay.set(Overlay::CreateGuild),
                    Icon { name: "plus", size: 22 }
                    strong { "Eigenen Server erstellen" }
                    span { "Ein Ort für deine Leute – mit Text- und Sprachkanälen. Du bist Owner." }
                }
                button { class: "nv-onboard__card", onclick: move |_| store.overlay.set(Overlay::JoinGuild),
                    Icon { name: "user-plus", size: 22 }
                    strong { "Mit Einladung beitreten" }
                    span { "Du hast einen Code von Freunden bekommen? Hier einfügen." }
                }
            }
        }
    }
}

// --------------------------------------------------------- Mitglieder

#[component]
fn MemberList() -> Element {
    let mut store = use_store();
    let detail = store.detail.read().clone();
    let speaking = store.voice.read().speaking.clone();
    let Some(detail) = detail else { return rsx! {} };
    let role_name = |member: &Member| {
        detail
            .roles
            .iter()
            .filter(|role| !role.is_everyone && member.role_ids.contains(&role.id))
            .max_by_key(|role| role.position)
            .map(|role| role.name.clone())
    };
    let online: Vec<Member> = detail
        .members
        .iter()
        .filter(|m| m.status != 3)
        .cloned()
        .collect();
    let offline: Vec<Member> = detail
        .members
        .iter()
        .filter(|m| m.status == 3)
        .cloned()
        .collect();
    let groups = [("Online", online), ("Offline", offline)];
    rsx! {
        aside { class: "nv-members", "aria-label": "Mitglieder",
            div { class: "nv-members__head",
                h2 { "Mitglieder" }
                span { class: "nv-faint nv-num", "{detail.members.len()}" }
                IconButton { icon: "close", label: "Mitglieder ausblenden", onclick: move |_| store.members_open.set(false) }
            }
            div { class: "nv-members__scroll",
                for (label, list) in groups {
                    if !list.is_empty() {
                        div { key: "{label}", class: "nv-members__label",
                            "{label}"
                            span { class: "nv-members__count", "{list.len()}" }
                        }
                        for member in list {
                            {
                                let offline = label == "Offline";
                                let id = member.user_id.clone();
                                let sub = if member.timeout_until.is_some() {
                                    Some("Im Timeout".to_string())
                                } else {
                                    role_name(&member)
                                };
                                rsx! {
                                    button {
                                        key: "{member.user_id}",
                                        class: classes(&["nv-member", if offline { "is-offline" } else { "" }]),
                                        onclick: move |_| store.overlay.set(Overlay::Member(id.clone())),
                                        Avatar {
                                            name: member.display_name.clone(),
                                            size: 32,
                                            status: Some(if offline { 3 } else { member.status }),
                                            speaking: speaking.contains(&member.user_id),
                                            bot: member.bot,
                                        }
                                        span { class: "nv-member__text",
                                            span { class: "nv-member__name", "{member.display_name}" }
                                            if let Some(sub) = sub {
                                                span { class: "nv-member__sub", "{sub}" }
                                            } else {
                                                span { class: "nv-member__sub", "@{member.handle}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Toast

#[component]
fn Toast() -> Element {
    let mut store = use_store();
    let toast = store.toast.read().clone();
    // Hinweise verschwinden nach 5 s, Fehler bleiben bis zum Schließen.
    use_effect(move || {
        let current = store.toast.read().clone();
        if let Some(toast) = current
            && !toast.error
        {
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(5000).await;
                if store.toast.read().as_ref() == Some(&toast) {
                    store.toast.set(None);
                }
            });
        }
    });
    let Some(toast) = toast else { return rsx! {} };
    rsx! {
        div {
            class: if toast.error { "nv-toast is-error" } else { "nv-toast" },
            role: if toast.error { "alert" } else { "status" },
            span { class: "nv-toast__text", "{toast.text}" }
            IconButton { icon: "close", label: "Meldung schließen", onclick: move |_| store.toast.set(None) }
        }
    }
}
