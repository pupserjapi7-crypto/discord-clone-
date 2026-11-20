//! Dialoge, Mitgliederkarte und Command Palette.

use dioxus::{core::spawn_forever, prelude::*};

use crate::{
    actions, api,
    components::{Avatar, Icon, Kbd, Modal, classes, focus_first},
    store::{ConfirmAction, Overlay, ServerTab, SettingsTab, use_store},
    types::*,
    views::settings::{ServerSettings, UserSettings},
};

#[component]
pub fn OverlayHost() -> Element {
    let store = use_store();
    let overlay = store.overlay.read().clone();
    match overlay {
        Overlay::None => rsx! {},
        Overlay::CreateGuild => rsx! { CreateGuild {} },
        Overlay::JoinGuild => rsx! { JoinGuild {} },
        Overlay::Invite(code) => rsx! { Invite { code } },
        Overlay::CreateChannel(kind) => rsx! { CreateChannel { kind } },
        Overlay::NewThread {
            channel_id,
            message_id,
        } => rsx! { NewThread { channel_id, message_id } },
        Overlay::Settings(tab) => rsx! { UserSettings { tab } },
        Overlay::ServerSettings(tab) => rsx! { ServerSettings { tab } },
        Overlay::Palette => rsx! { Palette {} },
        Overlay::Member(user_id) => rsx! { MemberCard { user_id } },
        Overlay::Confirm(action) => rsx! { Confirm { action } },
        Overlay::Image(attachment) => rsx! { ImageViewer { attachment } },
    }
}

#[component]
fn CreateGuild() -> Element {
    let mut store = use_store();
    let mut name = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);
    rsx! {
        Modal {
            id: "dlg-create-guild",
            title: "Server erstellen",
            description: "Ein eigener Ort für deine Gruppe. Du kannst Kanäle, Rollen und Einladungen danach anpassen.",
            onclose: move |_| store.close_overlay(),
            form {
                class: "nv-form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let value = name.read().trim().to_owned();
                    if value.chars().count() < 2 {
                        error.set(Some("Der Name braucht mindestens 2 Zeichen.".into()));
                        return;
                    }
                    busy.set(true);
                    spawn_forever(async move {
                        match api::create_guild(&value).await {
                            Ok(guild) => {
                                store.close_overlay();
                                actions::load_guild_list(store).await;
                                actions::open_guild(store, guild.id).await;
                            }
                            Err(err) => error.set(Some(err)),
                        }
                        busy.set(false);
                    });
                },
                div { class: "nv-field",
                    label { class: "nv-field__label", r#for: "guild-name", "Name" }
                    input { id: "guild-name", class: "nv-input", maxlength: "64", placeholder: "z. B. Freitagsrunde", value: "{name}", oninput: move |e| name.set(e.value()) }
                    if let Some(text) = error() {
                        p { class: "nv-field__error", role: "alert", Icon { name: "alert", size: 16 } "{text}" }
                    }
                }
                div { class: "nv-modal__actions",
                    button { r#type: "button", class: "nv-btn nv-btn--ghost", onclick: move |_| store.overlay.set(Overlay::JoinGuild), "Stattdessen beitreten" }
                    button { r#type: "submit", class: "nv-btn nv-btn--primary", disabled: busy(), if busy() { "Erstelle …" } else { "Erstellen" } }
                }
            }
        }
    }
}

#[component]
fn JoinGuild() -> Element {
    let store = use_store();
    let mut code = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| Option::<String>::None);
    rsx! {
        Modal {
            id: "dlg-join",
            title: "Einladung annehmen",
            description: "Füge den Code ein, den du bekommen hast.",
            onclose: move |_| store.close_overlay(),
            form {
                class: "nv-form",
                onsubmit: move |event| {
                    event.prevent_default();
                    // Ganze Links akzeptieren: nur das letzte Segment ist der Code.
                    let value = code.read().trim().rsplit('/').next().unwrap_or_default().to_owned();
                    if value.is_empty() {
                        error.set(Some("Bitte einen Einladungscode eingeben.".into()));
                        return;
                    }
                    busy.set(true);
                    spawn_forever(async move {
                        match api::join_invite(&value).await {
                            Ok(()) => {
                                store.close_overlay();
                                actions::load_guilds(store).await;
                            }
                            Err(_) => error.set(Some("Dieser Code ist ungültig, abgelaufen oder du bist dort gebannt.".into())),
                        }
                        busy.set(false);
                    });
                },
                div { class: "nv-field",
                    label { class: "nv-field__label", r#for: "invite-code", "Einladungscode" }
                    input { id: "invite-code", class: "nv-input", spellcheck: "false", placeholder: "z. B. a1b2c3d4e5", value: "{code}", oninput: move |e| code.set(e.value()) }
                    if let Some(text) = error() {
                        p { class: "nv-field__error", role: "alert", Icon { name: "alert", size: 16 } "{text}" }
                    }
                }
                div { class: "nv-modal__actions",
                    button { r#type: "button", class: "nv-btn nv-btn--ghost", onclick: move |_| store.close_overlay(), "Abbrechen" }
                    button { r#type: "submit", class: "nv-btn nv-btn--primary", disabled: busy(), if busy() { "Trete bei …" } else { "Beitreten" } }
                }
            }
        }
    }
}

#[component]
fn Invite(code: Option<String>) -> Element {
    let store = use_store();
    let mut copied = use_signal(|| false);
    rsx! {
        Modal {
            id: "dlg-invite",
            title: "Leute einladen",
            description: "Teile diesen Code. Er gilt 7 Tage.",
            onclose: move |_| store.close_overlay(),
            if let Some(code) = code {
                div { class: "nv-row",
                    input { class: "nv-input nv-num", readonly: true, value: "{code}", "aria-label": "Einladungscode" }
                    button {
                        class: "nv-btn nv-btn--primary",
                        onclick: move |_| {
                            if let Some(window) = web_sys::window() {
                                let _ = window.navigator().clipboard().write_text(&code);
                                copied.set(true);
                            }
                        },
                        if copied() { Icon { name: "check", size: 16 } "Kopiert" } else { "Kopieren" }
                    }
                }
            } else {
                p { class: "nv-muted", "Code wird erstellt …" }
            }
        }
    }
}

#[component]
fn CreateChannel(kind: i16) -> Element {
    let store = use_store();
    let mut name = use_signal(String::new);
    let mut chosen = use_signal(move || kind);
    let mut error = use_signal(|| Option::<String>::None);
    rsx! {
        Modal {
            id: "dlg-channel",
            title: "Kanal anlegen",
            onclose: move |_| store.close_overlay(),
            form {
                class: "nv-form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let value = name.read().trim().to_lowercase().replace(' ', "-");
                    if value.is_empty() {
                        error.set(Some("Bitte einen Namen eingeben.".into()));
                        return;
                    }
                    let Some(guild) = actions::guild_id(store) else { return };
                    let kind = chosen();
                    spawn_forever(async move {
                        match api::create_channel(&guild, &value, kind).await {
                            Ok(channel) => {
                                store.close_overlay();
                                actions::refresh_guild(store).await;
                                actions::open_channel(store, channel.id).await;
                            }
                            Err(err) => error.set(Some(err)),
                        }
                    });
                },
                div { class: "nv-segment", role: "group", "aria-label": "Kanaltyp",
                    button { r#type: "button", "aria-pressed": if chosen() == KIND_TEXT { "true" } else { "false" }, onclick: move |_| chosen.set(KIND_TEXT), "# Text" }
                    button { r#type: "button", "aria-pressed": if chosen() == KIND_VOICE { "true" } else { "false" }, onclick: move |_| chosen.set(KIND_VOICE), "Sprache" }
                }
                div { class: "nv-field",
                    label { class: "nv-field__label", r#for: "channel-name", "Name" }
                    input { id: "channel-name", class: "nv-input", maxlength: "48", placeholder: if chosen() == KIND_TEXT { "z. B. ankündigungen" } else { "z. B. lounge" }, value: "{name}", oninput: move |e| name.set(e.value()) }
                    span { class: "nv-field__hint", "Kleinbuchstaben, Leerzeichen werden zu Bindestrichen." }
                    if let Some(text) = error() {
                        p { class: "nv-field__error", role: "alert", Icon { name: "alert", size: 16 } "{text}" }
                    }
                }
                div { class: "nv-modal__actions",
                    button { r#type: "button", class: "nv-btn nv-btn--ghost", onclick: move |_| store.close_overlay(), "Abbrechen" }
                    button { r#type: "submit", class: "nv-btn nv-btn--primary", "Anlegen" }
                }
            }
        }
    }
}

#[component]
fn NewThread(channel_id: String, message_id: String) -> Element {
    let store = use_store();
    let preview = store
        .history
        .read()
        .get(&channel_id)
        .and_then(|h| h.messages.iter().find(|m| m.id == message_id).cloned());
    let initial: String = preview
        .as_ref()
        .map(|m| m.content.chars().take(40).collect())
        .unwrap_or_default();
    let mut name = use_signal(move || initial);
    let mut error = use_signal(|| Option::<String>::None);
    rsx! {
        Modal {
            id: "dlg-thread",
            title: "Thread starten",
            description: "Eine Nebenunterhaltung zu dieser Nachricht – der Kanal bleibt übersichtlich.",
            onclose: move |_| store.close_overlay(),
            if let Some(message) = preview {
                div { class: "nv-pin",
                    div { class: "nv-pin__head", strong { "{message.author.display_name}" } }
                    div { class: "nv-msg__text", "{message.content}" }
                }
            }
            form {
                class: "nv-form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let value = name.read().trim().to_owned();
                    if value.is_empty() {
                        error.set(Some("Bitte einen Namen eingeben.".into()));
                        return;
                    }
                    let channel_id = channel_id.clone();
                    let message_id = message_id.clone();
                    spawn_forever(async move {
                        match api::create_thread(&channel_id, &message_id, &value).await {
                            Ok(thread) => {
                                store.close_overlay();
                                actions::refresh_guild(store).await;
                                actions::open_thread(store, thread.id).await;
                            }
                            Err(err) => error.set(Some(err)),
                        }
                    });
                },
                div { class: "nv-field",
                    label { class: "nv-field__label", r#for: "thread-name", "Name" }
                    input { id: "thread-name", class: "nv-input", maxlength: "64", value: "{name}", oninput: move |e| name.set(e.value()) }
                    if let Some(text) = error() {
                        p { class: "nv-field__error", role: "alert", Icon { name: "alert", size: 16 } "{text}" }
                    }
                }
                div { class: "nv-modal__actions",
                    button { r#type: "button", class: "nv-btn nv-btn--ghost", onclick: move |_| store.close_overlay(), "Abbrechen" }
                    button { r#type: "submit", class: "nv-btn nv-btn--primary", "Thread starten" }
                }
            }
        }
    }
}

#[component]
fn Confirm(action: ConfirmAction) -> Element {
    let store = use_store();
    let mut reason = use_signal(String::new);
    let (title, text, button, needs_reason) = match &action {
        ConfirmAction::Kick { name, .. } => (
            format!("{name} entfernen?"),
            "Die Person verlässt den Server, kann aber mit einer neuen Einladung zurückkommen.".to_string(),
            "Entfernen",
            true,
        ),
        ConfirmAction::Ban { name, .. } => (
            format!("{name} bannen?"),
            "Die Person wird entfernt und kann nicht wieder beitreten, bis der Bann aufgehoben ist.".to_string(),
            "Bannen",
            true,
        ),
        ConfirmAction::DeleteMessage { .. } => (
            "Nachricht löschen?".to_string(),
            "Das lässt sich nicht rückgängig machen.".to_string(),
            "Löschen",
            false,
        ),
    };
    rsx! {
        Modal {
            id: "dlg-confirm",
            title: "{title}",
            description: "{text}",
            onclose: move |_| store.close_overlay(),
            form {
                class: "nv-form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let reason = reason.read().clone();
                    let action = action.clone();
                    spawn_forever(async move {
                        match action {
                            ConfirmAction::Kick { user_id, .. } => {
                                let Some(guild) = actions::guild_id(store) else { return };
                                actions::moderate(store, async move { api::kick(&guild, &user_id, &reason).await }, "Mitglied entfernt.").await;
                            }
                            ConfirmAction::Ban { user_id, .. } => {
                                let Some(guild) = actions::guild_id(store) else { return };
                                actions::moderate(store, async move { api::ban(&guild, &user_id, &reason).await }, "Mitglied gebannt.").await;
                            }
                            ConfirmAction::DeleteMessage { channel_id, message_id } => {
                                store.close_overlay();
                                if let Err(err) = api::delete_message(&channel_id, &message_id).await {
                                    store.error(err);
                                }
                            }
                        }
                    });
                },
                if needs_reason {
                    div { class: "nv-field",
                        label { class: "nv-field__label", r#for: "mod-reason", "Grund (optional, steht im Audit-Log)" }
                        input { id: "mod-reason", class: "nv-input", maxlength: "512", value: "{reason}", oninput: move |e| reason.set(e.value()) }
                    }
                }
                div { class: "nv-modal__actions",
                    button { r#type: "button", class: "nv-btn nv-btn--ghost", onclick: move |_| store.close_overlay(), "Abbrechen" }
                    button { r#type: "submit", class: "nv-btn nv-btn--danger", "{button}" }
                }
            }
        }
    }
}

// --------------------------------------------------------- Mitgliederkarte

#[component]
fn MemberCard(user_id: String) -> Element {
    let mut store = use_store();
    let detail = store.detail.read().clone();
    let Some(detail) = detail else { return rsx! {} };
    let Some(member) = detail.member(&user_id).cloned() else {
        return rsx! {};
    };
    let me = store.me_id();
    let is_me = me.as_deref() == Some(user_id.as_str());
    let is_owner = detail.guild.owner_id == user_id;
    let moderatable = !is_me && !is_owner;
    let roles: Vec<Role> = detail
        .roles
        .iter()
        .filter(|r| !r.is_everyone)
        .cloned()
        .collect();
    let in_voice = store
        .voice_rooms
        .read()
        .values()
        .any(|list| list.iter().any(|p| p.user_id == user_id));
    let volume = store.settings.read().volume(&user_id);
    let guild_id = detail.guild.id.clone();

    let timeout = {
        let guild_id = guild_id.clone();
        let user_id = user_id.clone();
        move |seconds: i64| {
            let guild_id = guild_id.clone();
            let user_id = user_id.clone();
            move |_| {
                let guild_id = guild_id.clone();
                let user_id = user_id.clone();
                let text = if seconds == 0 {
                    "Timeout aufgehoben."
                } else {
                    "Timeout vergeben."
                };
                spawn_forever(actions::moderate(
                    store,
                    async move { api::timeout(&guild_id, &user_id, seconds).await },
                    text,
                ));
            }
        }
    };

    rsx! {
        Modal {
            id: "dlg-member",
            title: "Profil",
            onclose: move |_| store.close_overlay(),
            div { class: "nv-profile",
                div { class: "nv-profile__head",
                    Avatar { name: member.display_name.clone(), size: 56, status: Some(member.status), bot: member.bot }
                    div {
                        div { class: "nv-profile__name", "{member.display_name}" }
                        div { class: "nv-profile__handle", "@{member.handle}" }
                        if is_owner {
                            span { class: "nv-badge", "Owner" }
                        }
                        if member.timeout_until.is_some() {
                            span { class: "nv-badge nv-badge--danger", "Timeout" }
                        }
                    }
                }
                if in_voice && !is_me {
                    div { class: "nv-field",
                        label { class: "nv-field__label", r#for: "member-volume", "Lautstärke für dich: {(volume * 100.0).round()} %" }
                        input {
                            id: "member-volume",
                            class: "nv-range",
                            r#type: "range",
                            min: "0",
                            max: "100",
                            value: "{(volume * 100.0).round()}",
                            oninput: {
                                let user_id = user_id.clone();
                                move |event: FormEvent| {
                                    let value = event.value().parse::<f64>().unwrap_or(100.0) / 100.0;
                                    store.settings.write().set_volume(&user_id, value);
                                    crate::voice::set_volume(&user_id, value);
                                    crate::settings::save(&store.settings.read());
                                }
                            },
                        }
                    }
                }
                if detail.can(perm::MANAGE_ROLES) && !roles.is_empty() && moderatable {
                    div { class: "nv-field",
                        span { class: "nv-field__label", "Rollen" }
                        div { class: "nv-chips",
                            for role in roles {
                                {
                                    let has = member.role_ids.contains(&role.id);
                                    let guild_id = guild_id.clone();
                                    let user_id = user_id.clone();
                                    let role_id = role.id.clone();
                                    rsx! {
                                        button {
                                            key: "{role.id}",
                                            class: "nv-chip",
                                            "aria-pressed": if has { "true" } else { "false" },
                                            onclick: move |_| {
                                                let guild_id = guild_id.clone();
                                                let user_id = user_id.clone();
                                                let role_id = role_id.clone();
                                                spawn_forever(async move {
                                                    match api::set_member_role(&guild_id, &user_id, &role_id, !has).await {
                                                        Ok(()) => actions::refresh_guild(store).await,
                                                        Err(err) => store.error(err),
                                                    }
                                                });
                                            },
                                            if has { Icon { name: "check", size: 14 } }
                                            "{role.name}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if moderatable && detail.can(perm::TIMEOUT_MEMBERS) {
                    div { class: "nv-field",
                        span { class: "nv-field__label", "Timeout" }
                        div { class: "nv-chips",
                            button { class: "nv-chip", onclick: timeout(600), Icon { name: "clock", size: 14 } "10 Min." }
                            button { class: "nv-chip", onclick: timeout(3600), "1 Std." }
                            button { class: "nv-chip", onclick: timeout(86_400), "1 Tag" }
                            if member.timeout_until.is_some() {
                                button { class: "nv-chip", onclick: timeout(0), "Aufheben" }
                            }
                        }
                    }
                }
                if moderatable && (detail.can(perm::KICK_MEMBERS) || detail.can(perm::BAN_MEMBERS)) {
                    div { class: "nv-profile__actions",
                        if detail.can(perm::KICK_MEMBERS) {
                            button {
                                class: "nv-btn",
                                onclick: {
                                    let user_id = user_id.clone();
                                    let name = member.display_name.clone();
                                    move |_| store.overlay.set(Overlay::Confirm(ConfirmAction::Kick { user_id: user_id.clone(), name: name.clone() }))
                                },
                                "Entfernen"
                            }
                        }
                        if detail.can(perm::BAN_MEMBERS) {
                            button {
                                class: "nv-btn nv-btn--danger",
                                onclick: {
                                    let user_id = user_id.clone();
                                    let name = member.display_name.clone();
                                    move |_| store.overlay.set(Overlay::Confirm(ConfirmAction::Ban { user_id: user_id.clone(), name: name.clone() }))
                                },
                                Icon { name: "ban", size: 16 }
                                "Bannen"
                            }
                        }
                    }
                }
            }
        }
    }
}

// ----------------------------------------------------------- Palette

#[derive(Clone, PartialEq)]
enum PaletteAction {
    Channel(String),
    Thread(String),
    Member(String),
    Message(String),
    CreateGuild,
    JoinGuild,
    Settings,
    ServerSettings,
    ToggleTheme,
    ToggleMembers,
}

#[derive(Clone, PartialEq)]
struct Entry {
    group: &'static str,
    icon: &'static str,
    label: String,
    hint: String,
    action: PaletteAction,
}

fn fuzzy(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let haystack = haystack.to_lowercase();
    let mut chars = haystack.chars();
    needle.to_lowercase().chars().all(|c| chars.any(|h| h == c))
}

#[component]
fn Palette() -> Element {
    let mut store = use_store();
    let mut query = use_signal(String::new);
    let mut selected = use_signal(|| 0usize);
    use_effect(|| focus_first("dlg-palette"));

    let detail = store.detail.read().clone();
    let current = store.channel_id.read().clone();
    let q = query.read().trim().to_owned();
    let mut entries: Vec<Entry> = Vec::new();
    if let Some(detail) = &detail {
        for channel in &detail.channels {
            if channel.kind == KIND_THREAD && channel.archived {
                continue;
            }
            if fuzzy(&channel.name, &q) {
                let (icon, group, action) = match channel.kind {
                    KIND_VOICE => (
                        "volume",
                        "Kanäle",
                        PaletteAction::Channel(channel.id.clone()),
                    ),
                    KIND_THREAD => (
                        "thread",
                        "Kanäle",
                        PaletteAction::Thread(channel.id.clone()),
                    ),
                    _ => ("hash", "Kanäle", PaletteAction::Channel(channel.id.clone())),
                };
                entries.push(Entry {
                    group,
                    icon,
                    label: channel.name.clone(),
                    hint: if current.as_deref() == Some(channel.id.as_str()) {
                        "aktuell".into()
                    } else {
                        String::new()
                    },
                    action,
                });
            }
        }
        for member in detail
            .members
            .iter()
            .filter(|m| fuzzy(&m.display_name, &q) || fuzzy(&m.handle, &q))
            .take(6)
        {
            entries.push(Entry {
                group: "Personen",
                icon: "user",
                label: member.display_name.clone(),
                hint: format!("@{}", member.handle),
                action: PaletteAction::Member(member.user_id.clone()),
            });
        }
    }
    if q.chars().count() >= 2 {
        let needle = q.to_lowercase();
        let histories = store.history.read();
        let mut found = 0;
        for (channel_id, history) in histories.iter() {
            for message in history.messages.iter().rev() {
                if found >= 5 {
                    break;
                }
                if message.content.to_lowercase().contains(&needle) {
                    let channel_name = detail
                        .as_ref()
                        .and_then(|d| d.channel(channel_id))
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    entries.push(Entry {
                        group: "Nachrichten",
                        icon: "inbox",
                        label: message.content.chars().take(80).collect(),
                        hint: format!("{} · #{channel_name}", message.author.display_name),
                        action: PaletteAction::Message(channel_id.clone()),
                    });
                    found += 1;
                }
            }
        }
    }
    let actions_list = [
        ("plus", "Server erstellen", PaletteAction::CreateGuild),
        ("user-plus", "Einladung annehmen", PaletteAction::JoinGuild),
        ("settings", "Einstellungen", PaletteAction::Settings),
        (
            "shield",
            "Server-Einstellungen",
            PaletteAction::ServerSettings,
        ),
        (
            "contrast",
            "Hell/Dunkel umschalten",
            PaletteAction::ToggleTheme,
        ),
        (
            "users",
            "Mitgliederliste ein/aus",
            PaletteAction::ToggleMembers,
        ),
    ];
    for (icon, label, action) in actions_list {
        if action == PaletteAction::ServerSettings && detail.is_none() {
            continue;
        }
        if fuzzy(label, &q) {
            entries.push(Entry {
                group: "Aktionen",
                icon,
                label: label.into(),
                hint: String::new(),
                action,
            });
        }
    }
    let count = entries.len();
    let index = selected().min(count.saturating_sub(1));

    let mut run = move |action: PaletteAction| {
        store.overlay.set(Overlay::None);
        match action {
            PaletteAction::Channel(id) | PaletteAction::Message(id) => {
                spawn_forever(actions::open_channel(store, id));
            }
            PaletteAction::Thread(id) => {
                spawn_forever(actions::open_thread(store, id));
            }
            PaletteAction::Member(id) => store.overlay.set(Overlay::Member(id)),
            PaletteAction::CreateGuild => store.overlay.set(Overlay::CreateGuild),
            PaletteAction::JoinGuild => store.overlay.set(Overlay::JoinGuild),
            PaletteAction::Settings => store.overlay.set(Overlay::Settings(SettingsTab::Profile)),
            PaletteAction::ServerSettings => store
                .overlay
                .set(Overlay::ServerSettings(ServerTab::Overview)),
            PaletteAction::ToggleTheme => {
                let dark = web_sys::window()
                    .and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok().flatten())
                    .map(|m| m.matches())
                    .unwrap_or(true);
                let next = match store.settings.read().theme {
                    Theme::Dark => Theme::Light,
                    Theme::Light => Theme::Dark,
                    Theme::System => {
                        if dark {
                            Theme::Light
                        } else {
                            Theme::Dark
                        }
                    }
                };
                store.settings.write().theme = next;
                crate::settings::save(&store.settings.read());
            }
            PaletteAction::ToggleMembers => {
                let open = !*store.members_open.read();
                store.members_open.set(open);
            }
        }
    };
    let entries_for_keys = entries.clone();
    let mut last_group = "";
    rsx! {
        div { class: "nv-backdrop nv-palette-backdrop", onclick: move |_| store.close_overlay(),
            div {
                id: "dlg-palette",
                class: "nv-palette",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "Suchen und springen",
                onclick: move |e| e.stop_propagation(),
                div { class: "nv-palette__search",
                    Icon { name: "search" }
                    input {
                        placeholder: "Kanal, Person, Nachricht oder Aktion …",
                        "aria-controls": "palette-list",
                        "aria-activedescendant": "palette-{index}",
                        value: "{query}",
                        oninput: move |e| {
                            query.set(e.value());
                            selected.set(0);
                        },
                        onkeydown: move |event| match event.key() {
                            Key::ArrowDown => {
                                event.prevent_default();
                                selected.set((index + 1) % count.max(1));
                            }
                            Key::ArrowUp => {
                                event.prevent_default();
                                selected.set((index + count.max(1) - 1) % count.max(1));
                            }
                            Key::Enter => {
                                event.prevent_default();
                                if let Some(entry) = entries_for_keys.get(index) {
                                    run(entry.action.clone());
                                }
                            }
                            Key::Escape => store.close_overlay(),
                            _ => {}
                        },
                    }
                    Kbd { "Esc" }
                }
                div { id: "palette-list", class: "nv-palette__list", role: "listbox",
                    if entries.is_empty() {
                        p { class: "nv-palette__empty", "Nichts gefunden für „{q}“." }
                    }
                    for (position, entry) in entries.into_iter().enumerate() {
                        Fragment { key: "{position}",
                        if entry.group != last_group {
                            {
                                last_group = entry.group;
                                rsx! { div { class: "nv-palette__group", "{entry.group}" } }
                            }
                        }
                        button {
                            id: "palette-{position}",
                            role: "option",
                            "aria-selected": if position == index { "true" } else { "false" },
                            class: classes(&["nv-palette__item", if position == index { "is-selected" } else { "" }]),
                            onmouseenter: move |_| selected.set(position),
                            onclick: {
                                let action = entry.action.clone();
                                move |_| run(action.clone())
                            },
                            Icon { name: entry.icon, size: 18 }
                            span { class: "nv-palette__label", "{entry.label}" }
                            if !entry.hint.is_empty() {
                                span { class: "nv-palette__hint", "{entry.hint}" }
                            }
                        }
                        }
                    }
                }
                div { class: "nv-palette__foot",
                    span { Kbd { "↑" } Kbd { "↓" } "auswählen" }
                    span { Kbd { "Enter" } "öffnen" }
                    span { Kbd { "Strg K" } "schließen" }
                }
            }
        }
    }
}

/// Bild in voller Größe; Esc, Klick daneben oder „Schließen“ beendet.
#[component]
fn ImageViewer(attachment: Attachment) -> Element {
    let store = use_store();
    use_effect(|| focus_first("image-viewer"));
    let size = attachment
        .width
        .zip(attachment.height)
        .map(|(w, h)| format!("{w} × {h} · {}", size_label(attachment.size)))
        .unwrap_or_else(|| size_label(attachment.size));
    rsx! {
        div {
            class: "nv-backdrop nv-viewer",
            onclick: move |_| store.close_overlay(),
            div {
                id: "image-viewer",
                class: "nv-viewer__frame",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "Bild {attachment.filename}",
                onclick: move |event| event.stop_propagation(),
                onkeydown: move |event| {
                    if event.key() == Key::Escape {
                        store.close_overlay();
                    }
                },
                img {
                    class: "nv-viewer__image",
                    src: "{attachment.url()}",
                    alt: "{attachment.filename}",
                }
                div { class: "nv-viewer__bar",
                    div { class: "nv-viewer__name",
                        strong { "{attachment.filename}" }
                        span { class: "nv-faint", "{size}" }
                    }
                    a {
                        class: "nv-btn nv-btn--ghost",
                        href: "{attachment.download_url()}",
                        download: "{attachment.filename}",
                        Icon { name: "download", size: 16 }
                        "Herunterladen"
                    }
                    button { class: "nv-btn", r#type: "button", onclick: move |_| store.close_overlay(), "Schließen" }
                }
            }
        }
    }
}
