mod api;
mod gateway;
mod protocol;
mod types;
mod voice;

use dioxus::prelude::*;
use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::{JsCast, closure::Closure};

use gateway::Gateway;
use types::{Channel, Message, SessionState, User};

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let mut state = use_signal(SessionState::default);
    let mut gateway = use_signal(|| Option::<Gateway>::None);

    let mut auth_handle = use_signal(String::new);
    let mut auth_password = use_signal(String::new);
    let mut auth_register = use_signal(|| false);
    let mut auth_busy = use_signal(|| false);
    let mut auth_error = use_signal(String::new);

    let mut draft = use_signal(String::new);
    let mut new_guild = use_signal(String::new);
    let mut join_code = use_signal(String::new);
    let mut new_channel = use_signal(String::new);
    let mut channel_kind = use_signal(|| 0i16);
    let mut show_create_guild = use_signal(|| false);
    let mut show_join = use_signal(|| false);
    let mut show_channel_form = use_signal(|| false);
    let mut show_invite = use_signal(|| false);
    let mut invite_code = use_signal(|| Option::<String>::None);
    let mut show_members = use_signal(|| true);
    let mut show_nav = use_signal(|| false);
    let mut text_open = use_signal(|| true);
    let mut voice_open = use_signal(|| true);
    let mut palette_open = use_signal(|| false);
    let mut palette_query = use_signal(String::new);

    use_future(move || async move {
        boot(state, gateway).await;
    });

    // Strg/Cmd+K und Escape gelten für die ganze App, nicht nur für ein Feld.
    use_effect(move || {
        let mut palette = palette_open;
        let shortcut = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            move |event: web_sys::KeyboardEvent| {
                if (event.ctrl_key() || event.meta_key()) && event.key().eq_ignore_ascii_case("k") {
                    event.prevent_default();
                    palette.set(true);
                } else if event.key() == "Escape" {
                    palette.set(false);
                }
            },
        );
        if let Some(window) = web_sys::window() {
            let _ = window
                .add_event_listener_with_callback("keydown", shortcut.as_ref().unchecked_ref());
        }
        shortcut.forget();
    });

    let snapshot = state.read().clone();
    let message_count = snapshot.messages.len();
    use_effect(move || {
        if message_count == 0 {
            return;
        }
        if let Some(element) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("message-list"))
        {
            element.set_scroll_top(element.scroll_height());
        }
    });

    rsx! {
        link { rel: "stylesheet", href: "/assets/app.css" }
        if let Some(me) = snapshot.user.clone() {
            div {
                class: if show_members() { "app-shell" } else { "app-shell members-off" },
                nav { class: "rail", "aria-label": "Server",
                    div { class: "rail-logo", "N" }
                    div { class: "rail-guilds",
                        for guild in snapshot.guilds.iter().cloned() {
                            button {
                                key: "{guild.id}",
                                class: if is_current_guild(&snapshot, &guild.id) { "rail-guild active" } else { "rail-guild" },
                                title: "{guild.name}",
                                onclick: {
                                    let id = guild.id.clone();
                                    move |_| {
                                        let id = id.clone();
                                        spawn(async move { open_guild(id, state, gateway).await; });
                                    }
                                },
                                "{initials(&guild.name)}"
                            }
                        }
                        button {
                            class: "rail-add",
                            title: "Server erstellen",
                            onclick: move |_| show_create_guild.set(true),
                            "+"
                        }
                    }
                    div { class: "rail-bottom",
                        div { class: "avatar", "{initials(&me.display_name)}" }
                    }
                }

                aside {
                    id: "channel-sidebar",
                    class: if show_nav() { "sidebar mobile-open" } else { "sidebar" },
                    if let Some(detail) = snapshot.detail.clone() {
                        div { class: "guild-head",
                            h1 { "{detail.guild.name}" }
                            button {
                                class: "ghost small",
                                title: "Einladung erstellen",
                                onclick: {
                                    let guild_id = detail.guild.id.clone();
                                    move |_| {
                                        let guild_id = guild_id.clone();
                                        show_invite.set(true);
                                        invite_code.set(None);
                                        spawn(async move {
                                            match api::create_invite(&guild_id).await {
                                                Ok(response) => invite_code.set(Some(response.code)),
                                                Err(err) => state.write().error = err,
                                            }
                                        });
                                    }
                                },
                                "Einladen"
                            }
                        }
                        div { class: "channel-scroll",
                            div { class: "channel-group",
                                button {
                                    class: if text_open() { "group-label" } else { "group-label collapsed" },
                                    "aria-expanded": if text_open() { "true" } else { "false" },
                                    onclick: move |_| text_open.toggle(),
                                    Icon { name: "chevron" }
                                    span { "Textkanäle" }
                                }
                                if text_open() {
                                    for channel in detail.channels.iter().filter(|c| c.kind == 0).cloned() {
                                        button {
                                            key: "{channel.id}",
                                            class: if is_current_channel(&snapshot, &channel.id) { "channel active" } else { "channel" },
                                            onclick: {
                                                let channel = channel.clone();
                                                move |_| {
                                                    let channel = channel.clone();
                                                    show_nav.set(false);
                                                    spawn(async move { open_channel(channel, state, gateway).await; });
                                                }
                                            },
                                            span { class: "channel-icon", Icon { name: "hash" } }
                                            span { class: "channel-name", "{channel.name}" }
                                        }
                                    }
                                }
                            }
                            div { class: "channel-group",
                                button {
                                    class: if voice_open() { "group-label" } else { "group-label collapsed" },
                                    "aria-expanded": if voice_open() { "true" } else { "false" },
                                    onclick: move |_| voice_open.toggle(),
                                    Icon { name: "chevron" }
                                    span { "Sprachkanäle" }
                                }
                                if voice_open() {
                                    for channel in detail.channels.iter().filter(|c| c.kind != 0).cloned() {
                                        button {
                                            key: "{channel.id}",
                                            class: if is_current_channel(&snapshot, &channel.id) { "channel active" } else { "channel" },
                                            onclick: {
                                                let channel = channel.clone();
                                                move |_| {
                                                    let channel = channel.clone();
                                                    show_nav.set(false);
                                                    spawn(async move { open_channel(channel, state, gateway).await; });
                                                }
                                            },
                                            span { class: "channel-icon", Icon { name: "volume" } }
                                            span { class: "channel-name", "{channel.name}" }
                                            if snapshot.voice_joined && is_current_channel(&snapshot, &channel.id) {
                                                span { class: "live-dot" }
                                            }
                                        }
                                        if snapshot.voice_joined && is_current_channel(&snapshot, &channel.id) && !snapshot.voice_participants.is_empty() {
                                            div { class: "voice-presence",
                                                for participant in snapshot.voice_participants.iter().cloned() {
                                                    span { class: "mini-avatar", title: "{participant.display_name}", "{initials(&participant.display_name)}" }
                                                }
                                            }
                                        }
                                    }
                                }
                                if show_channel_form() {
                                    div { class: "side-form",
                                        input {
                                            placeholder: "Kanalname",
                                            value: new_channel(),
                                            oninput: move |event| new_channel.set(event.value()),
                                        }
                                        div { class: "kind-row",
                                            button {
                                                class: if channel_kind() == 0 { "kind active" } else { "kind" },
                                                onclick: move |_| channel_kind.set(0),
                                                "Text"
                                            }
                                            button {
                                                class: if channel_kind() == 1 { "kind active" } else { "kind" },
                                                onclick: move |_| channel_kind.set(1),
                                                "Sprache"
                                            }
                                        }
                                        button {
                                            class: "primary full",
                                            onclick: {
                                                let guild_id = detail.guild.id.clone();
                                                move |_| {
                                                    let guild_id = guild_id.clone();
                                                    let name = new_channel.read().trim().to_string();
                                                    if name.is_empty() {
                                                        return;
                                                    }
                                                    let kind = channel_kind();
                                                    spawn(async move {
                                                        match api::create_channel(&guild_id, &name, kind).await {
                                                            Ok(channel) => {
                                                                new_channel.set(String::new());
                                                                show_channel_form.set(false);
                                                                open_guild(guild_id.clone(), state, gateway).await;
                                                                if kind == 0 {
                                                                    let _ = channel;
                                                                }
                                                            }
                                                            Err(err) => state.write().error = err,
                                                        }
                                                    });
                                                }
                                            },
                                            "Erstellen"
                                        }
                                    }
                                } else {
                                    button {
                                        class: "channel add-channel",
                                        onclick: move |_| show_channel_form.set(true),
                                        span { class: "channel-icon", "+" }
                                        span { class: "channel-name", "Kanal hinzufügen" }
                                    }
                                }
                            }
                        }
                    } else {
                        div { class: "guild-head", h1 { "Nova" } }
                        div { class: "empty-side",
                            p { "Noch kein Server." }
                            button { class: "primary full", onclick: move |_| show_create_guild.set(true), "Server erstellen" }
                            button { class: "ghost full", onclick: move |_| show_join.set(true), "Einladung beitreten" }
                        }
                    }

                    div { class: "me-bar",
                        div { class: "avatar small", "{initials(&me.display_name)}" }
                        div { class: "me-info",
                            div { class: "me-name", "{me.display_name}" }
                            div { class: "me-status",
                                span { class: if snapshot.status == "online" { "status-dot online" } else { "status-dot" } }
                                "{status_label(&snapshot.status)}"
                            }
                        }
                        button {
                            class: "ghost icon",
                            title: "Abmelden",
                            onclick: move |_| {
                                spawn(async move {
                                    voice::shutdown();
                                    if let Some(handle) = gateway.read().clone() {
                                        handle.close();
                                    }
                                    let _ = api::logout().await;
                                    state.set(SessionState::default());
                                    gateway.set(None);
                                    invite_code.set(None);
                                });
                            },
                            Icon { name: "logout" }
                        }
                    }
                }

                main { class: "main",
                    if let Some(channel) = snapshot.channel.clone() {
                        header { class: "channel-header",
                            button {
                                class: "icon-button nav-toggle",
                                "aria-label": "Kanäle öffnen",
                                onclick: move |_| show_nav.set(true),
                                Icon { name: "menu" }
                            }
                            div { class: "channel-title",
                                span { class: "channel-icon large", Icon { name: if channel.kind == 0 { "hash".to_string() } else { "volume".to_string() } } }
                                div {
                                    h2 { "{channel.name}" }
                                    if let Some(topic) = channel.topic.clone().filter(|topic| !topic.is_empty()) {
                                        p { "{topic}" }
                                    }
                                }
                            }
                            div { class: "channel-actions",
                                if let Some(detail) = snapshot.detail.clone() {
                                    span { class: "member-count", "{detail.members.len()} Mitglieder" }
                                }
                                button {
                                    class: "search-trigger",
                                    "aria-label": "Befehlspalette öffnen",
                                    onclick: move |_| palette_open.set(true),
                                    Icon { name: "search" }
                                    span { "Suchen" }
                                    kbd { "Strg K" }
                                }
                                if snapshot.detail.is_some() {
                                    button {
                                        class: if show_members() { "icon-button members-toggle active" } else { "icon-button members-toggle" },
                                        "aria-label": if show_members() { "Mitglieder ausblenden" } else { "Mitglieder einblenden" },
                                        "aria-pressed": if show_members() { "true" } else { "false" },
                                        onclick: move |_| show_members.toggle(),
                                        Icon { name: "users" }
                                    }
                                    button {
                                        class: "ghost small",
                                        onclick: {
                                            let guild_id = snapshot.detail.as_ref().map(|detail| detail.guild.id.clone());
                                            move |_| {
                                                let Some(guild_id) = guild_id.clone() else { return };
                                                show_invite.set(true);
                                                invite_code.set(None);
                                                spawn(async move {
                                                    match api::create_invite(&guild_id).await {
                                                        Ok(response) => invite_code.set(Some(response.code)),
                                                        Err(err) => state.write().error = err,
                                                    }
                                                });
                                            }
                                        },
                                        Icon { name: "user-plus" }
                                        span { "Einladen" }
                                    }
                                }
                            }
                        }
                    } else {
                        header { class: "channel-header",
                            button {
                                class: "icon-button nav-toggle",
                                "aria-label": "Kanäle öffnen",
                                onclick: move |_| show_nav.set(true),
                                Icon { name: "menu" }
                            }
                            div { class: "channel-title", h2 { "Willkommen bei Nova" } }
                        }
                    }

                    div { id: "voice-videos", class: "voice-videos" }

                    if snapshot.channel.as_ref().map(|channel| channel.kind == 0).unwrap_or(true) {
                        div { id: "message-list", class: "message-list", role: "log", "aria-live": "polite",
                            if snapshot.messages.is_empty() {
                                div { class: "empty-chat",
                                    div { class: "empty-mark", "N" }
                                    h2 { if snapshot.guilds.is_empty() { "Starte deinen Server" } else { "Noch keine Nachrichten" } }
                                    p {
                                        if snapshot.guilds.is_empty() {
                                            "Erstelle einen Server oder tritt mit einem Einladungscode bei."
                                        } else {
                                            "Schreibe die erste Nachricht in diesem Kanal."
                                        }
                                    }
                                    if snapshot.guilds.is_empty() {
                                        div { class: "empty-actions",
                                            button { class: "primary", onclick: move |_| show_create_guild.set(true), "Server erstellen" }
                                            button { class: "ghost", onclick: move |_| show_join.set(true), "Einladung beitreten" }
                                        }
                                    }
                                }
                            }
                            for (index, message) in snapshot.messages.iter().cloned().enumerate() {
                                {
                                    let grouped = grouped_message(&snapshot.messages, index);
                                    rsx! {
                                        div { key: "{message.id}", class: if grouped { "message grouped" } else { "message" },
                                            if grouped {
                                                div { class: "message-avatar-spacer" }
                                            } else {
                                                div { class: "avatar small", "{initials(&message.author.display_name)}" }
                                            }
                                            div { class: "message-body",
                                                if grouped {
                                                    span { class: "sr-only", "{message.author.display_name}, {time_label(&message.created_at)}" }
                                                } else {
                                                    div { class: "message-head",
                                                        span { class: "message-author", "{message.author.display_name}" }
                                                        span { class: "message-handle", "@{message.author.handle}" }
                                                        span { class: "message-time", "{time_label(&message.created_at)}" }
                                                    }
                                                }
                                                div { class: "message-text", "{message.content}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        div { class: "voice-stage",
                            div { class: "voice-stage-card",
                                div { class: "voice-orb", Icon { name: "volume" } }
                                h2 { "{snapshot.channel.as_ref().map(|c| c.name.clone()).unwrap_or_default()}" }
                                p { "Sprache in hoher Qualität, Bildschirmfreigabe bis 1080p60." }
                                if snapshot.voice_joined && snapshot.voice_channel.as_ref().map(|c| c.id.clone()) == snapshot.channel.as_ref().map(|c| c.id.clone()) {
                                    button {
                                        class: "danger",
                                        onclick: move |_| {
                                            let channel = state.read().voice_channel.clone();
                                            spawn(async move {
                                                voice::shutdown();
                                                if let Some(channel) = channel {
                                                    let _ = api::voice_leave(&channel.id).await;
                                                }
                                                let mut current = state.write();
                                                current.voice_joined = false;
                                                current.voice_channel = None;
                                                                                                                current.voice_participants.clear();
                                                                current.screen = false;
                                                                current.muted = false;
                                                                current.deafened = false;
                                            });
                                        },
                                        "Verlassen"
                                    }
                                } else {
                                    button {
                                        class: "primary",
                                        onclick: move |_| {
                                            let selected = state.read().channel.clone();
                                            if let Some(channel) = selected {
                                                let channel_id = channel.id.clone();
                                                spawn(async move {
                                                    match voice::join(&channel_id).await {
                                                        Ok(()) => {
                                                            {
                                                                let mut current = state.write();
                                                                current.voice_joined = true;
                                                                current.voice_channel = Some(channel);
                                                                current.muted = false;
                                                                current.screen = false;
                                                            }
                                                            start_voice_poll(channel_id, state);
                                                        }
                                                        Err(err) => state.write().error = err,
                                                    }
                                                });
                                            }
                                        },
                                        "Sprachkanal beitreten"
                                    }
                                }
                            }
                        }
                    }

                    if let Some(channel) = snapshot.channel.clone().filter(|channel| channel.kind == 0) {
                        div { class: "composer-wrap",
                            div { class: "composer",
                                input {
                                    placeholder: if snapshot.status == "online" { "Nachricht an #{channel.name}" } else { "Verbinde mit dem Gateway…" },
                                    value: draft(),
                                    oninput: move |event| draft.set(event.value()),
                                }
                                button {
                                    class: "primary send",
                                    disabled: draft.read().trim().is_empty(),
                                    onclick: move |_| send_message(state, gateway, draft),
                                    "Senden"
                                }
                            }
                        }
                    }

                    if let Some(voice_channel) = snapshot.voice_channel.clone() {
                        section { class: "voice-dock", "aria-label": "Sprachsteuerung",
                            div { class: "voice-dock-main",
                                div { class: if snapshot.voice_joined { "voice-signal" } else { "voice-signal idle" } }
                                div {
                                    div { class: "voice-name", "{voice_channel.name}" }
                                    div { class: "voice-sub",
                                        if snapshot.voice_joined { "Verbunden" } else { "Bereit" }
                                        " · {snapshot.voice_participants.len()} Teilnehmer"
                                    }
                                }
                            }
                            div { class: "voice-people",
                                for participant in snapshot.voice_participants.iter().cloned() {
                                    span { class: "voice-person", title: "{participant.display_name}",
                                        span { class: "mini-avatar", "{initials(&participant.display_name)}" }
                                        if participant.screen { span { class: "badge", "1080" } }
                                    }
                                }
                            }
                            div { class: "voice-actions",
                                if snapshot.voice_joined {
                                    button {
                                        class: if snapshot.muted { "icon-button active" } else { "icon-button" },
                                        "aria-label": if snapshot.muted { "Stummschaltung aufheben" } else { "Stummschalten" },
                                        "aria-pressed": if snapshot.muted { "true" } else { "false" },
                                        onclick: move |_| {
                                            let muted = !state.read().muted;
                                            state.write().muted = muted;
                                            voice::set_muted(muted);
                                        },
                                        Icon { name: if snapshot.muted { "mic-off".to_string() } else { "mic".to_string() } }
                                    }
                                    button {
                                        class: if snapshot.deafened { "icon-button active" } else { "icon-button" },
                                        "aria-label": if snapshot.deafened { "Ton wieder einschalten" } else { "Ton stummschalten" },
                                        "aria-pressed": if snapshot.deafened { "true" } else { "false" },
                                        onclick: move |_| {
                                            let deafened = !state.read().deafened;
                                            state.write().deafened = deafened;
                                            voice::set_deafened(deafened);
                                        },
                                        Icon { name: "headphones" }
                                    }
                                    button {
                                        class: if snapshot.screen { "icon-button active" } else { "icon-button" },
                                        "aria-label": "Bildschirm teilen (1080p60)",
                                        "aria-pressed": if snapshot.screen { "true" } else { "false" },
                                        onclick: move |_| {
                                            let sharing = state.read().screen;
                                            spawn(async move {
                                                if sharing {
                                                    voice::stop_screen();
                                                    state.write().screen = false;
                                                } else {
                                                    match voice::start_screen().await {
                                                        Ok(()) => state.write().screen = true,
                                                        Err(err) => state.write().error = err,
                                                    }
                                                }
                                            });
                                        },
                                        Icon { name: "screen" }
                                    }
                                    button {
                                        class: "icon-button danger",
                                        "aria-label": "Sprachkanal verlassen",
                                        onclick: {
                                            let channel_id = voice_channel.id.clone();
                                            move |_| {
                                                let channel_id = channel_id.clone();
                                                spawn(async move {
                                                    voice::shutdown();
                                                    let _ = api::voice_leave(&channel_id).await;
                                                    let mut current = state.write();
                                                    current.voice_joined = false;
                                                    current.voice_channel = None;
                                                    current.voice_participants.clear();
                                                    current.screen = false;
                                                    current.muted = false;
                                                    current.deafened = false;
                                                });
                                            }
                                        },
                                        Icon { name: "phone-off" }
                                    }
                                } else {
                                    button {
                                        class: "primary small",
                                        onclick: {
                                            let channel = voice_channel.clone();
                                            move |_| {
                                                let channel = channel.clone();
                                                spawn(async move {
                                                    match voice::join(&channel.id).await {
                                                        Ok(()) => {
                                                            {
                                                                let mut current = state.write();
                                                                current.voice_joined = true;
                                                                current.muted = false;
                                                            }
                                                            start_voice_poll(channel.id.clone(), state);
                                                        }
                                                        Err(err) => state.write().error = err,
                                                    }
                                                });
                                            }
                                        },
                                        "Beitreten"
                                    }
                                }
                            }
                        }
                    }
                }

                aside { class: "members", "aria-label": "Mitglieder",
                    div { class: "members-head",
                        h2 { "Mitglieder" }
                        button {
                            class: "icon-button",
                            "aria-label": "Mitglieder ausblenden",
                            onclick: move |_| show_members.set(false),
                            Icon { name: "close" }
                        }
                    }
                    div { class: "members-list",
                        if let Some(detail) = snapshot.detail.clone() {
                            div { class: "member-group-label", "Online" }
                            for member in detail.members.iter().filter(|member| member.status >= 0).cloned() {
                                div { key: "{member.user_id}", class: "member",
                                    div { class: "avatar tiny", "{initials(&member.display_name)}" }
                                    div { class: "member-info",
                                        div { class: "member-name", "{member.display_name}" }
                                        div { class: "member-handle", "@{member.handle}" }
                                    }
                                    span { class: "status-dot online", "aria-label": "Online" }
                                }
                            }
                            for member in detail.members.iter().filter(|member| member.status < 0).cloned() {
                                div { key: "{member.user_id}", class: "member",
                                    div { class: "avatar tiny", "{initials(&member.display_name)}" }
                                    div { class: "member-info",
                                        div { class: "member-name", "{member.display_name}" }
                                        div { class: "member-handle", "@{member.handle}" }
                                    }
                                    span { class: "status-dot", "aria-label": "Offline" }
                                }
                            }
                        }
                    }
                }
            }

            if palette_open() {
                div { class: "palette-backdrop", onclick: move |_| palette_open.set(false),
                    div { class: "palette", role: "dialog", "aria-modal": "true", "aria-label": "Befehlspalette", onclick: move |event| event.stop_propagation(),
                        div { class: "palette-search",
                            Icon { name: "search" }
                            input {
                                autofocus: true,
                                placeholder: "Kanal, Server oder Befehl suchen…",
                                value: palette_query(),
                                oninput: move |event| palette_query.set(event.value()),
                                onkeydown: move |event| {
                                    if event.key().to_string() == "Escape" {
                                        palette_open.set(false);
                                    } else if event.key().to_string() == "Enter" {
                                        let query = palette_query.read().to_lowercase();
                                        let target = state.read().detail.as_ref().and_then(|detail| {
                                            detail.channels.iter().find(|channel| channel.name.to_lowercase().contains(&query)).cloned()
                                        });
                                        if let Some(channel) = target {
                                            palette_open.set(false);
                                            spawn(async move { open_channel(channel, state, gateway).await; });
                                        }
                                    }
                                },
                            }
                        }
                        div { class: "palette-list",
                            div { class: "palette-section", "Kanäle" }
                            if let Some(detail) = snapshot.detail.clone() {
                                for channel in detail.channels.iter().filter(|channel| channel.name.to_lowercase().contains(&palette_query().to_lowercase())).cloned() {
                                    button {
                                        key: "{channel.id}",
                                        class: "palette-item",
                                        onclick: {
                                            let channel = channel.clone();
                                            move |_| {
                                                let channel = channel.clone();
                                                palette_open.set(false);
                                                spawn(async move { open_channel(channel, state, gateway).await; });
                                            }
                                        },
                                        Icon { name: if channel.kind == 0 { "hash".to_string() } else { "volume".to_string() } }
                                        span { "{channel.name}" }
                                    }
                                }
                            }
                            div { class: "palette-section", "Aktionen" }
                            button { class: "palette-item", onclick: move |_| { palette_open.set(false); show_create_guild.set(true); }, Icon { name: "plus" }, "Server erstellen" }
                            button { class: "palette-item", onclick: move |_| { palette_open.set(false); show_join.set(true); }, Icon { name: "user-plus" }, "Einladung beitreten" }
                        }
                        div { class: "palette-hint", span { "Tippen · Tab · Enter öffnen" }, span { "Esc schließen" } }
                    }
                }
            }

            if show_nav() {
                div { class: "nav-scrim", onclick: move |_| show_nav.set(false) }
            }

            if show_create_guild() {
                div { class: "modal-backdrop", onclick: move |_| show_create_guild.set(false),
                    div { class: "modal", onclick: move |event| event.stop_propagation(),
                        h2 { "Server erstellen" }
                        p { class: "muted", "Ein eigener Ort für deine Community." }
                        input {
                            placeholder: "Name des Servers",
                            value: new_guild(),
                            oninput: move |event| new_guild.set(event.value()),
                        }
                        div { class: "modal-actions",
                            button { class: "ghost", onclick: move |_| show_create_guild.set(false), "Abbrechen" }
                            button {
                                class: "primary",
                                onclick: move |_| {
                                    let name = new_guild.read().trim().to_string();
                                    if name.is_empty() {
                                        return;
                                    }
                                    spawn(async move {
                                        match api::create_guild(&name).await {
                                            Ok(_) => {
                                                new_guild.set(String::new());
                                                show_create_guild.set(false);
                                                load_guilds(state, gateway).await;
                                            }
                                            Err(err) => state.write().error = err,
                                        }
                                    });
                                },
                                "Erstellen"
                            }
                        }
                    }
                }
            }

            if show_join() {
                div { class: "modal-backdrop", onclick: move |_| show_join.set(false),
                    div { class: "modal", onclick: move |event| event.stop_propagation(),
                        h2 { "Einladung beitreten" }
                        p { class: "muted", "Füge den Code ein, den du erhalten hast." }
                        input {
                            placeholder: "Einladungscode",
                            value: join_code(),
                            oninput: move |event| join_code.set(event.value()),
                        }
                        div { class: "modal-actions",
                            button { class: "ghost", onclick: move |_| show_join.set(false), "Abbrechen" }
                            button {
                                class: "primary",
                                onclick: move |_| {
                                    let code = join_code.read().trim().to_string();
                                    if code.is_empty() {
                                        return;
                                    }
                                    spawn(async move {
                                        match api::join_invite(&code).await {
                                            Ok(()) => {
                                                join_code.set(String::new());
                                                show_join.set(false);
                                                load_guilds(state, gateway).await;
                                            }
                                            Err(err) => state.write().error = err,
                                        }
                                    });
                                },
                                "Beitreten"
                            }
                        }
                    }
                }
            }

            if show_invite() {
                div { class: "modal-backdrop", onclick: move |_| show_invite.set(false),
                    div { class: "modal", onclick: move |event| event.stop_propagation(),
                        h2 { "Einladung" }
                        p { class: "muted", "Teile diesen Code mit deinen Freunden." }
                        if let Some(code) = invite_code() {
                            input { class: "invite-input", readonly: true, value: "{code}" }
                            p { class: "muted tiny", "Der Code läuft nach 7 Tagen ab." }
                        } else {
                            p { class: "loading-line", "Code wird erstellt…" }
                        }
                        div { class: "modal-actions",
                            button { class: "primary", onclick: move |_| show_invite.set(false), "Fertig" }
                        }
                    }
                }
            }

            if !snapshot.error.is_empty() {
                div { class: "toast",
                    span { "{snapshot.error}" }
                    button { class: "ghost icon", onclick: move |_| state.write().error.clear(), "×" }
                }
            }
        } else {
            div { class: "auth-shell",
                div { class: "auth-glow" }
                div { class: "auth-card",
                    div { class: "brand-mark", "N" }
                    h1 { "Nova" }
                    p { class: "auth-sub", "Sprich. Teile. Bleib verbunden." }
                    if snapshot.booting {
                        div { class: "loading-line", "Sitzung wird geprüft…" }
                    } else {
                        div { class: "auth-form",
                            label { "Handle" }
                            input {
                                placeholder: "dein_name",
                                value: auth_handle(),
                                autocomplete: "username",
                                oninput: move |event| auth_handle.set(event.value()),
                            }
                            label { "Passwort" }
                            input {
                                r#type: "password",
                                placeholder: "••••••••",
                                value: auth_password(),
                                autocomplete: if auth_register() { "new-password" } else { "current-password" },
                                oninput: move |event| auth_password.set(event.value()),
                            }
                            if !auth_error().is_empty() {
                                div { class: "form-error", "{auth_error}" }
                            }
                            button {
                                class: "primary full big",
                                disabled: auth_busy(),
                                onclick: move |_| {
                                    let handle = auth_handle.read().trim().to_string();
                                    let password = auth_password.read().clone();
                                    if handle.len() < 3 || password.len() < 8 {
                                        auth_error.set("Handle (min. 3) und Passwort (min. 8) prüfen.".into());
                                        return;
                                    }
                                    let register = auth_register();
                                    auth_busy.set(true);
                                    auth_error.set(String::new());
                                    spawn(async move {
                                        let result = if register {
                                            api::register(&handle, &password).await
                                        } else {
                                            api::login(&handle, &password).await
                                        };
                                        match result {
                                            Ok(user) => finish_login(user, state, gateway).await,
                                            Err(err) => auth_error.set(err),
                                        }
                                        auth_busy.set(false);
                                    });
                                },
                                if auth_busy() { "Einen Moment…" } else if auth_register() { "Konto erstellen" } else { "Anmelden" }
                            }
                            button {
                                class: "switch-auth",
                                onclick: move |_| {
                                    auth_register.toggle();
                                    auth_error.set(String::new());
                                },
                                if auth_register() { "Schon ein Konto? Anmelden" } else { "Neu hier? Konto erstellen" }
                            }
                        }
                    }
                    div { class: "auth-foot", "Rust · Axum · WebRTC · Dioxus" }
                }
            }
        }
    }
}

#[component]
fn Icon(name: String) -> Element {
    let path = icon_path(&name);
    rsx! {
        svg {
            class: "icon",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.8",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            path { d: path }
        }
    }
}

fn icon_path(name: &str) -> &'static str {
    match name {
        "hash" => "M4 9h16M4 15h16M10 3 8 21M16 3l-2 18",
        "volume" => "M11 5 6 9H3v6h3l5 4V5Zm4.54 3.46a5 5 0 0 1 0 7.07m2.82-9.89a9 9 0 0 1 0 12.72",
        "mic" => {
            "M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Zm7 8v2a7 7 0 0 1-14 0v-2m7 9v3"
        }
        "mic-off" => {
            "M3 3l18 18M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V5a3 3 0 0 0-5.94-.6M19 10v2a7 7 0 0 1-.11 1.23M5 10v2a7 7 0 0 0 7 7m0 0v3"
        }
        "screen" => "M3 5h18v12H3zM8 21h8M12 17v4",
        "phone-off" => {
            "M3 3l18 18M5 10a15 15 0 0 0 9 9l2-3 3 1v3a2 2 0 0 1-2 2A17 17 0 0 1 2 5a2 2 0 0 1 2-2h3l1 3-3 2a15 15 0 0 0 0 2"
        }
        "headphones" => {
            "M3 14v-2a9 9 0 0 1 18 0v2M3 14h3v6H4a1 1 0 0 1-1-1v-5Zm18 0h-3v6h2a1 1 0 0 0 1-1v-5Z"
        }
        "users" => {
            "M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8m14 10v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"
        }
        "plus" => "M12 5v14M5 12h14",
        "search" => "M21 21l-4.35-4.35M19 11a8 8 0 1 1-16 0 8 8 0 0 1 16 0",
        "chevron" => "M6 9l6 6 6-6",
        "close" => "M6 6l12 12M18 6 6 18",
        "menu" => "M4 7h16M4 12h16M4 17h16",
        "user-plus" => {
            "M16 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M8.5 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8M20 8v6M23 11h-6"
        }
        "logout" => "M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9",
        _ => "M12 3v18M3 12h18",
    }
}

fn grouped_message(messages: &[Message], index: usize) -> bool {
    if index == 0 {
        return false;
    }
    let previous = &messages[index - 1];
    let current = &messages[index];
    previous.author.id == current.author.id
        && previous.created_at.get(..16) == current.created_at.get(..16)
}

fn is_current_guild(state: &SessionState, guild_id: &str) -> bool {
    state
        .detail
        .as_ref()
        .map(|detail| detail.guild.id == guild_id)
        .unwrap_or(false)
}

fn is_current_channel(state: &SessionState, channel_id: &str) -> bool {
    state
        .channel
        .as_ref()
        .map(|channel| channel.id == channel_id)
        .unwrap_or(false)
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

fn time_label(value: &str) -> String {
    value.get(11..16).unwrap_or_default().to_string()
}

fn status_label(status: &str) -> &'static str {
    match status {
        "online" => "Online",
        "connecting" => "Verbinde…",
        "reconnecting" => "Neu verbinden…",
        _ => "Offline",
    }
}

async fn boot(mut state: Signal<SessionState>, gateway: Signal<Option<Gateway>>) {
    state.write().booting = true;
    match api::refresh().await {
        Ok(user) => finish_login(user, state, gateway).await,
        Err(_) => state.write().booting = false,
    }
}

async fn finish_login(
    user: User,
    mut state: Signal<SessionState>,
    mut gateway: Signal<Option<Gateway>>,
) {
    let Some(token) = api::token() else {
        state.write().booting = false;
        return;
    };
    if let Some(previous) = gateway.read().clone() {
        previous.close();
    }
    let handle = Gateway::start(token, user.id.clone(), None, state);
    gateway.set(Some(handle));
    {
        let mut current = state.write();
        current.user = Some(user);
        current.booting = false;
        current.error.clear();
    }
    load_guilds(state, gateway).await;
}

async fn load_guilds(mut state: Signal<SessionState>, gateway: Signal<Option<Gateway>>) {
    match api::guilds().await {
        Ok(list) => {
            let current_guild = state
                .read()
                .detail
                .as_ref()
                .map(|detail| detail.guild.id.clone());
            let target = current_guild
                .and_then(|id| list.iter().find(|guild| guild.id == id).cloned())
                .or_else(|| list.first().cloned());
            state.write().guilds = list;
            match target {
                Some(guild) => open_guild(guild.id, state, gateway).await,
                None => {
                    let mut current = state.write();
                    current.detail = None;
                    current.channel = None;
                    current.messages.clear();
                }
            }
        }
        Err(err) => state.write().error = err,
    }
}

async fn open_guild(
    guild_id: String,
    mut state: Signal<SessionState>,
    gateway: Signal<Option<Gateway>>,
) {
    match api::guild(&guild_id).await {
        Ok(detail) => {
            let previous = state
                .read()
                .channel
                .clone()
                .filter(|channel| channel.guild_id == guild_id);
            let target = previous
                .or_else(|| {
                    detail
                        .channels
                        .iter()
                        .find(|channel| channel.kind == 0)
                        .cloned()
                })
                .or_else(|| detail.channels.first().cloned());
            {
                let mut current = state.write();
                current.detail = Some(detail);
                current.channel = target.clone();
                current.messages.clear();
            }
            if let Some(channel) = target {
                open_channel(channel, state, gateway).await;
            }
        }
        Err(err) => state.write().error = err,
    }
}

async fn open_channel(
    channel: Channel,
    mut state: Signal<SessionState>,
    gateway: Signal<Option<Gateway>>,
) {
    let is_text = channel.kind == 0;
    {
        let mut current = state.write();
        current.channel = Some(channel.clone());
        current.messages.clear();
    }
    if let Some(handle) = gateway.read().clone()
        && let Ok(id) = channel.id.parse::<i64>()
    {
        handle.subscribe(id);
    }
    if is_text {
        match api::messages(&channel.id).await {
            Ok(history) => state.write().messages = history,
            Err(err) => state.write().error = err,
        }
    }
}

fn send_message(
    mut state: Signal<SessionState>,
    gateway: Signal<Option<Gateway>>,
    mut draft: Signal<String>,
) {
    let content = draft.read().trim().to_string();
    if content.is_empty() {
        return;
    }
    let Some(channel) = state.read().channel.clone() else {
        return;
    };
    let Some(handle) = gateway.read().clone() else {
        state.write().error = "Gateway ist nicht verbunden.".into();
        return;
    };
    let Some(user) = state.read().user.clone() else {
        return;
    };
    let nonce = (js_sys::Date::now() as u64) * 1000 + (js_sys::Math::random() * 1000.0) as u64;
    let local = Message {
        id: format!("local-{nonce}"),
        channel_id: channel.id.clone(),
        author: user,
        content: content.clone(),
        created_at: types::now_string(),
        edited_at: None,
        reply_to: None,
        reactions: Vec::new(),
    };
    state.write().messages.push(local);
    if let Ok(channel_id) = channel.id.parse::<i64>() {
        handle.send_message(nonce, channel_id, content);
    }
    draft.set(String::new());
}

fn start_voice_poll(channel_id: String, mut state: Signal<SessionState>) {
    spawn(async move {
        loop {
            let active = {
                let current = state.read();
                current.voice_joined
                    && current
                        .voice_channel
                        .as_ref()
                        .map(|channel| channel.id == channel_id)
                        .unwrap_or(false)
            };
            if !active {
                break;
            }
            if let Ok(participants) = api::voice_participants(&channel_id).await {
                state.write().voice_participants = participants;
            }
            TimeoutFuture::new(4000).await;
        }
    });
}
