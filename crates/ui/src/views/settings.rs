//! Nutzer- und Server-Einstellungen.

use dioxus::{core::spawn_forever, prelude::*};
use serde_json::json;

use crate::{
    actions, api,
    components::{Icon, Kbd, Modal, classes, day_label, time_label},
    store::{ServerTab, SettingsTab, use_store},
    types::*,
};

fn save_settings(store: crate::store::Store) {
    crate::settings::save(&store.settings.read());
}

#[component]
pub fn UserSettings(tab: SettingsTab) -> Element {
    let mut store = use_store();
    let tabs = [
        (SettingsTab::Profile, "user", "Profil"),
        (SettingsTab::Voice, "mic", "Sprache & Audio"),
        (SettingsTab::Appearance, "contrast", "Darstellung"),
        (SettingsTab::Shortcuts, "keyboard", "Tastenkürzel"),
        (SettingsTab::Notifications, "bell", "Benachrichtigungen"),
    ];
    rsx! {
        Modal {
            id: "dlg-settings",
            title: "Einstellungen",
            wide: true,
            onclose: move |_| store.close_overlay(),
            div { class: "nv-tabs",
                nav { class: "nv-tabs__nav", role: "tablist", "aria-label": "Bereiche",
                    for (id, icon, label) in tabs {
                        button {
                            key: "{label}",
                            role: "tab",
                            "aria-selected": if id == tab { "true" } else { "false" },
                            onclick: move |_| store.overlay.set(crate::store::Overlay::Settings(id)),
                            Icon { name: icon, size: 16 }
                            "{label}"
                        }
                    }
                }
                div { class: "nv-tabs__panel", role: "tabpanel",
                    match tab {
                        SettingsTab::Profile => rsx! { ProfileTab {} },
                        SettingsTab::Voice => rsx! { VoiceTab {} },
                        SettingsTab::Appearance => rsx! { AppearanceTab {} },
                        SettingsTab::Shortcuts => rsx! { ShortcutsTab {} },
                        SettingsTab::Notifications => rsx! { NotificationsTab {} },
                    }
                }
            }
        }
    }
}

#[component]
fn ProfileTab() -> Element {
    let mut store = use_store();
    let user = store.user.read().clone().unwrap_or_default();
    let mut name = use_signal(move || user.display_name.clone());
    let mut busy = use_signal(|| false);
    let handle = store
        .user
        .read()
        .as_ref()
        .map(|u| u.handle.clone())
        .unwrap_or_default();
    rsx! {
        section { class: "nv-section",
            h3 { "Profil" }
            form {
                class: "nv-form",
                onsubmit: move |event| {
                    event.prevent_default();
                    let value = name.read().trim().to_owned();
                    busy.set(true);
                    spawn_forever(async move {
                        match api::update_profile(&value).await {
                            Ok(user) => {
                                store.user.set(Some(user));
                                store.notice("Profil gespeichert.");
                            }
                            Err(err) => store.error(err),
                        }
                        busy.set(false);
                    });
                },
                div { class: "nv-field",
                    label { class: "nv-field__label", r#for: "profile-name", "Anzeigename" }
                    input { id: "profile-name", class: "nv-input", maxlength: "48", value: "{name}", oninput: move |e| name.set(e.value()) }
                    span { class: "nv-field__hint", "Handle: @{handle} – der Handle bleibt fest." }
                }
                div { class: "nv-row",
                    button { r#type: "submit", class: "nv-btn nv-btn--primary", disabled: busy(), "Speichern" }
                }
            }
        }
        section { class: "nv-section",
            h3 { "Sitzung" }
            p { class: "nv-muted", "Meldet dich auf diesem Gerät ab und trennt Sprachverbindungen." }
            div { class: "nv-row",
                button { class: "nv-btn", onclick: move |_| { spawn_forever(actions::logout(store)); }, Icon { name: "logout", size: 16 } "Abmelden" }
            }
        }
    }
}

#[component]
fn VoiceTab() -> Element {
    let mut store = use_store();
    let devices = use_resource(crate::voice::input_devices);
    let mut capturing = use_signal(|| false);
    let settings = store.settings.read().clone();
    let joined = store.voice.read().joined;
    rsx! {
        section { class: "nv-section",
            h3 { "Eingabe" }
            div { class: "nv-field",
                label { class: "nv-field__label", r#for: "input-device", "Mikrofon" }
                select {
                    id: "input-device",
                    class: "nv-select",
                    onchange: move |event| {
                        let value = event.value();
                        store.settings.write().input_device = (!value.is_empty()).then_some(value);
                        save_settings(store);
                    },
                    option { value: "", selected: settings.input_device.is_none(), "Systemstandard" }
                    if let Some(list) = devices.read().as_ref() {
                        for (id, label) in list.clone() {
                            option { key: "{id}", value: "{id}", selected: settings.input_device.as_deref() == Some(id.as_str()), "{label}" }
                        }
                    }
                }
                span { class: "nv-field__hint",
                    if joined { "Gilt ab dem nächsten Beitritt." } else { "Die Namen erscheinen, sobald der Browser einmal Mikrofonzugriff hatte." }
                }
            }
            label { class: "nv-check",
                input {
                    r#type: "checkbox",
                    checked: settings.noise_suppression,
                    onchange: move |event| {
                        store.settings.write().noise_suppression = event.checked();
                        save_settings(store);
                    },
                }
                span { class: "nv-check__text",
                    span { "Rauschunterdrückung" }
                    span { class: "nv-check__hint", "Filtert Lüfter und Tastatur heraus (über den Browser). Gilt ab dem nächsten Beitritt." }
                }
            }
        }
        section { class: "nv-section",
            h3 { "Push-to-Talk" }
            label { class: "nv-check",
                input {
                    r#type: "checkbox",
                    checked: settings.push_to_talk,
                    onchange: move |event| {
                        let on = event.checked();
                        store.settings.write().push_to_talk = on;
                        save_settings(store);
                        if store.voice.read().joined {
                            crate::voice::set_muted(on);
                            store.voice.write().muted = on;
                        }
                    },
                }
                span { class: "nv-check__text",
                    span { "Nur senden, solange die Taste gedrückt ist" }
                    span { class: "nv-check__hint", "Funktioniert, solange Nova im Vordergrund ist." }
                }
            }
            div { class: "nv-row",
                span { class: "nv-muted", "Taste:" }
                Kbd { "{settings.ptt_key.trim_start_matches(\"Key\")}" }
                button {
                    class: "nv-btn nv-btn--sm",
                    onkeydown: move |event| {
                        if capturing() {
                            event.prevent_default();
                            let code = event.code().to_string();
                            if code != "Escape" {
                                store.settings.write().ptt_key = code;
                                save_settings(store);
                            }
                            capturing.set(false);
                        }
                    },
                    onclick: move |_| capturing.set(true),
                    if capturing() { "Jetzt Taste drücken …" } else { "Ändern" }
                }
            }
        }
    }
}

#[component]
fn AppearanceTab() -> Element {
    let mut store = use_store();
    let settings = store.settings.read().clone();
    let themes = [
        (Theme::System, "System"),
        (Theme::Dark, "Dunkel"),
        (Theme::Light, "Hell"),
    ];
    rsx! {
        section { class: "nv-section",
            h3 { "Theme" }
            div { class: "nv-segment", role: "group", "aria-label": "Theme",
                for (theme, label) in themes {
                    button {
                        key: "{label}",
                        "aria-pressed": if settings.theme == theme { "true" } else { "false" },
                        onclick: move |_| {
                            store.settings.write().theme = theme;
                            save_settings(store);
                        },
                        "{label}"
                    }
                }
            }
        }
        section { class: "nv-section",
            h3 { "Schriftgröße" }
            div { class: "nv-field",
                label { class: "nv-field__label", r#for: "font-scale", "{settings.font_scale} %" }
                input {
                    id: "font-scale",
                    class: "nv-range",
                    r#type: "range",
                    min: "90",
                    max: "125",
                    step: "5",
                    value: "{settings.font_scale}",
                    oninput: move |event| {
                        store.settings.write().font_scale = event.value().parse().unwrap_or(100);
                        save_settings(store);
                    },
                }
                span { class: "nv-field__hint", "Nachrichten, Listen und Eingaben skalieren mit." }
            }
        }
    }
}

#[component]
fn ShortcutsTab() -> Element {
    let store = use_store();
    let ptt = store
        .settings
        .read()
        .ptt_key
        .trim_start_matches("Key")
        .to_owned();
    let rows = [
        ("Strg K", "Suchen und springen"),
        ("Enter", "Nachricht senden"),
        ("Umschalt Enter", "Neue Zeile"),
        ("Strg Umschalt M", "Stummschalten (im Sprachkanal)"),
        ("Esc", "Dialog oder Popover schließen"),
        ("↑ ↓", "In der Suche auswählen"),
    ];
    rsx! {
        section { class: "nv-section",
            h3 { "Tastenkürzel" }
            div { class: "nv-list",
                for (keys, text) in rows {
                    div { key: "{keys}", class: "nv-list__item",
                        span { class: "nv-list__main", "{text}" }
                        Kbd { "{keys}" }
                    }
                }
                div { class: "nv-list__item",
                    span { class: "nv-list__main", "Push-to-Talk (wenn aktiviert)" }
                    Kbd { "{ptt}" }
                }
            }
        }
    }
}

#[component]
fn NotificationsTab() -> Element {
    let mut store = use_store();
    let enabled = store.settings.read().notifications;
    rsx! {
        section { class: "nv-section",
            h3 { "Desktop-Benachrichtigungen" }
            label { class: "nv-check",
                input {
                    r#type: "checkbox",
                    checked: enabled,
                    onchange: move |event| {
                        let on = event.checked();
                        spawn_forever(async move {
                            let granted = if on { crate::settings::request_notification_permission().await } else { true };
                            if on && !granted {
                                store.error("Der Browser hat Benachrichtigungen blockiert. Erlaube sie in den Seiteneinstellungen.");
                            }
                            store.settings.write().notifications = on && granted;
                            save_settings(store);
                        });
                    },
                }
                span { class: "nv-check__text",
                    span { "Bei Erwähnungen benachrichtigen" }
                    span { class: "nv-check__hint", "Nur wenn Nova im Hintergrund ist und dich jemand mit @handle erwähnt." }
                }
            }
        }
    }
}

// --------------------------------------------------------------- Server

#[component]
pub fn ServerSettings(tab: ServerTab) -> Element {
    let mut store = use_store();
    let detail = store.detail.read().clone();
    let Some(detail) = detail else { return rsx! {} };
    let mut tabs = vec![(ServerTab::Overview, "hash", "Kanäle")];
    if detail.can(perm::MANAGE_ROLES) {
        tabs.push((ServerTab::Roles, "shield", "Rollen"));
    }
    if detail.can(perm::BAN_MEMBERS) {
        tabs.push((ServerTab::Bans, "ban", "Banns"));
    }
    tabs.push((ServerTab::Audit, "inbox", "Audit-Log"));
    rsx! {
        Modal {
            id: "dlg-server",
            title: "{detail.guild.name}",
            wide: true,
            onclose: move |_| store.close_overlay(),
            div { class: "nv-tabs",
                nav { class: "nv-tabs__nav", role: "tablist", "aria-label": "Server-Bereiche",
                    for (id, icon, label) in tabs {
                        button {
                            key: "{label}",
                            role: "tab",
                            "aria-selected": if id == tab { "true" } else { "false" },
                            onclick: move |_| store.overlay.set(crate::store::Overlay::ServerSettings(id)),
                            Icon { name: icon, size: 16 }
                            "{label}"
                        }
                    }
                }
                div { class: "nv-tabs__panel", role: "tabpanel",
                    match tab {
                        ServerTab::Overview => rsx! { ChannelsTab {} },
                        ServerTab::Roles => rsx! { RolesTab {} },
                        ServerTab::Bans => rsx! { BansTab {} },
                        ServerTab::Audit => rsx! { AuditTab {} },
                    }
                }
            }
        }
    }
}

const SLOWMODES: [(i32, &str); 8] = [
    (0, "Aus"),
    (5, "5 s"),
    (10, "10 s"),
    (30, "30 s"),
    (60, "1 Min."),
    (300, "5 Min."),
    (900, "15 Min."),
    (3600, "1 Std."),
];

#[component]
fn ChannelsTab() -> Element {
    let store = use_store();
    let detail = store.detail.read().clone().unwrap_or_default();
    let can = detail.can(perm::MANAGE_CHANNELS);
    let channels: Vec<Channel> = detail
        .channels
        .iter()
        .filter(|c| c.kind == KIND_TEXT)
        .cloned()
        .collect();
    rsx! {
        section { class: "nv-section",
            h3 { "Textkanäle" }
            p { class: "nv-muted", "Slowmode begrenzt, wie oft jemand schreiben darf. Moderation ist ausgenommen." }
            if !can {
                p { class: "nv-faint", "Zum Ändern brauchst du „Kanäle verwalten“." }
            }
            div { class: "nv-list",
                for channel in channels {
                    {
                        let id = channel.id.clone();
                        rsx! {
                            div { key: "{channel.id}", class: "nv-list__item",
                                Icon { name: "hash", size: 16 }
                                div { class: "nv-list__main",
                                    div { class: "nv-list__title", "{channel.name}" }
                                    div { class: "nv-list__sub", "{channel.topic.clone().unwrap_or_default()}" }
                                }
                                label { class: "sr-only", r#for: "slow-{channel.id}", "Slowmode für {channel.name}" }
                                select {
                                    id: "slow-{channel.id}",
                                    class: "nv-select",
                                    style: "width: 120px; min-height: 36px",
                                    disabled: !can,
                                    onchange: move |event| {
                                        let secs: i32 = event.value().parse().unwrap_or(0);
                                        let id = id.clone();
                                        spawn_forever(async move {
                                            match api::update_channel(&id, json!({ "slowmode_secs": secs })).await {
                                                Ok(_) => {
                                                    store.notice("Slowmode gespeichert.");
                                                    actions::refresh_guild(store).await;
                                                }
                                                Err(err) => store.error(err),
                                            }
                                        });
                                    },
                                    for (secs, label) in SLOWMODES {
                                        option { key: "{secs}", value: "{secs}", selected: channel.slowmode_secs == secs, "{label}" }
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

#[component]
fn RolesTab() -> Element {
    let store = use_store();
    let detail = store.detail.read().clone().unwrap_or_default();
    let mut selected = use_signal(|| Option::<String>::None);
    let roles = detail.roles.clone();
    let current = selected
        .read()
        .clone()
        .and_then(|id| roles.iter().find(|r| r.id == id).cloned())
        .or_else(|| roles.iter().find(|r| r.is_everyone).cloned());
    let guild_id = detail.guild.id.clone();
    let create_guild = guild_id.clone();
    rsx! {
        section { class: "nv-section",
            div { class: "nv-row nv-row--between",
                h3 { "Rollen" }
                button {
                    class: "nv-btn nv-btn--sm",
                    onclick: move |_| {
                        let guild = create_guild.clone();
                        spawn_forever(async move {
                            match api::create_role(&guild, "Neue Rolle").await {
                                Ok(role) => {
                                    selected.set(Some(role.id));
                                    actions::refresh_guild(store).await;
                                }
                                Err(err) => store.error(err),
                            }
                        });
                    },
                    Icon { name: "plus", size: 16 }
                    "Rolle anlegen"
                }
            }
            p { class: "nv-muted", "Höhere Rollen stehen oben. Du kannst nur Rollen unterhalb deiner eigenen bearbeiten." }
            div { class: "nv-list",
                for role in roles.clone() {
                    {
                        let id = role.id.clone();
                        let active = current.as_ref().map(|c| c.id == role.id).unwrap_or(false);
                        rsx! {
                            button {
                                key: "{role.id}",
                                class: classes(&["nv-list__item", if active { "is-selected" } else { "" }]),
                                onclick: move |_| selected.set(Some(id.clone())),
                                span { class: "nv-role-dot" }
                                span { class: "nv-list__main",
                                    span { class: "nv-list__title", "{role.name}" }
                                }
                                span { class: "nv-faint nv-num", "Pos. {role.position}" }
                            }
                        }
                    }
                }
            }
        }
        if let Some(role) = current {
            RoleEditor { key: "{role.id}", guild_id: guild_id.clone(), role }
        }
    }
}

#[component]
fn RoleEditor(guild_id: String, role: Role) -> Element {
    let store = use_store();
    let initial_name = role.name.clone();
    let mut name = use_signal(move || initial_name);
    let mut bits = use_signal(move || role.permissions as u64);
    let save_guild = guild_id.clone();
    let save_role = role.id.clone();
    let delete_guild = guild_id.clone();
    let delete_role = role.id.clone();
    rsx! {
        section { class: "nv-section",
            h3 { if role.is_everyone { "@everyone – Grundrechte aller Mitglieder" } else { "Rolle bearbeiten" } }
            if !role.is_everyone {
                div { class: "nv-field",
                    label { class: "nv-field__label", r#for: "role-name", "Name" }
                    input { id: "role-name", class: "nv-input", maxlength: "32", value: "{name}", oninput: move |e| name.set(e.value()) }
                }
            }
            div {
                for (bit, label, hint) in perm::EDITABLE.iter().copied() {
                    label { key: "{bit}", class: "nv-check",
                        input {
                            r#type: "checkbox",
                            checked: bits() & bit != 0,
                            onchange: move |event| {
                                let mut value = bits();
                                if event.checked() { value |= bit } else { value &= !bit }
                                bits.set(value);
                            },
                        }
                        span { class: "nv-check__text",
                            span { "{label}" }
                            if !hint.is_empty() {
                                span { class: "nv-check__hint", "{hint}" }
                            }
                        }
                    }
                }
            }
            div { class: "nv-row",
                button {
                    class: "nv-btn nv-btn--primary",
                    onclick: move |_| {
                        let mut patch = json!({ "permissions": bits() });
                        if !role.is_everyone {
                            patch["name"] = json!(name.read().trim());
                        }
                        let guild = save_guild.clone();
                        let id = save_role.clone();
                        spawn_forever(async move {
                            match api::update_role(&guild, &id, patch).await {
                                Ok(_) => {
                                    store.notice("Rolle gespeichert.");
                                    actions::refresh_guild(store).await;
                                }
                                Err(err) => store.error(err),
                            }
                        });
                    },
                    "Speichern"
                }
                if !role.is_everyone {
                    button {
                        class: "nv-btn nv-btn--ghost",
                        onclick: move |_| {
                            let guild = delete_guild.clone();
                            let id = delete_role.clone();
                            spawn_forever(async move {
                                match api::delete_role(&guild, &id).await {
                                    Ok(()) => {
                                        store.notice("Rolle gelöscht.");
                                        actions::refresh_guild(store).await;
                                    }
                                    Err(err) => store.error(err),
                                }
                            });
                        },
                        Icon { name: "trash", size: 16 }
                        "Löschen"
                    }
                }
            }
        }
    }
}

#[component]
fn BansTab() -> Element {
    let store = use_store();
    let guild = actions::guild_id(store).unwrap_or_default();
    let fetch_guild = guild.clone();
    let mut bans = use_resource(move || {
        let guild = fetch_guild.clone();
        async move { api::bans(&guild).await }
    });
    let list = bans.read().clone();
    rsx! {
        section { class: "nv-section",
            h3 { "Gebannte Mitglieder" }
            match list {
                None => rsx! { p { class: "nv-muted", "Lädt …" } },
                Some(Err(err)) => rsx! { p { class: "nv-field__error", "{err}" } },
                Some(Ok(list)) if list.is_empty() => rsx! { p { class: "nv-muted", "Niemand ist gebannt." } },
                Some(Ok(list)) => rsx! {
                    div { class: "nv-list",
                        for ban in list {
                            {
                                let guild = guild.clone();
                                let user = ban.user_id.clone();
                                rsx! {
                                    div { key: "{ban.user_id}", class: "nv-list__item",
                                        div { class: "nv-list__main",
                                            div { class: "nv-list__title", "{ban.display_name} (@{ban.handle})" }
                                            div { class: "nv-list__sub", "{ban.reason.clone().unwrap_or_else(|| \"Kein Grund angegeben\".into())} · {day_label(&ban.created_at)}" }
                                        }
                                        button {
                                            class: "nv-btn nv-btn--sm",
                                            onclick: move |_| {
                                                let guild = guild.clone();
                                                let user = user.clone();
                                                spawn_forever(async move {
                                                    match api::unban(&guild, &user).await {
                                                        Ok(()) => {
                                                            store.notice("Bann aufgehoben.");
                                                            bans.restart();
                                                        }
                                                        Err(err) => store.error(err),
                                                    }
                                                });
                                            },
                                            "Aufheben"
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}

fn audit_label(action: &str) -> &'static str {
    match action {
        "member_kick" => "hat entfernt",
        "member_ban" => "hat gebannt",
        "member_unban" => "hat Bann aufgehoben für",
        "member_timeout" => "hat Timeout gegeben",
        "member_timeout_clear" => "hat Timeout aufgehoben für",
        "member_role_add" => "hat Rolle vergeben an",
        "member_role_remove" => "hat Rolle entzogen bei",
        "role_create" => "hat Rolle angelegt",
        "role_update" => "hat Rolle geändert",
        "role_delete" => "hat Rolle gelöscht",
        "channel_create" => "hat Kanal angelegt",
        "channel_update" => "hat Kanal geändert",
        "channel_delete" => "hat Kanal gelöscht",
        "channel_override_set" | "channel_override_delete" => "hat Kanalrechte geändert",
        "message_delete" => "hat eine Nachricht gelöscht von",
        "message_pin" => "hat angepinnt",
        "message_unpin" => "hat Pin entfernt",
        "bot_add" => "hat einen Bot hinzugefügt",
        _ => "Aktion",
    }
}

#[component]
fn AuditTab() -> Element {
    let store = use_store();
    let guild = actions::guild_id(store).unwrap_or_default();
    let log = use_resource(move || {
        let guild = guild.clone();
        async move { api::audit_log(&guild).await }
    });
    let members = store
        .detail
        .read()
        .as_ref()
        .map(|d| d.members.clone())
        .unwrap_or_default();
    let name_of = move |id: &Option<String>| {
        id.as_ref()
            .and_then(|id| members.iter().find(|m| &m.user_id == id))
            .map(|m| m.display_name.clone())
    };
    rsx! {
        section { class: "nv-section",
            h3 { "Audit-Log" }
            p { class: "nv-muted", "Jede Moderationsaktion wird hier mit Zeit, Person und Grund festgehalten." }
            match log.read().clone() {
                None => rsx! { p { class: "nv-muted", "Lädt …" } },
                Some(Err(err)) => rsx! { p { class: "nv-field__error", "{err}" } },
                Some(Ok(entries)) if entries.is_empty() => rsx! { p { class: "nv-muted", "Noch keine Einträge." } },
                Some(Ok(entries)) => rsx! {
                    div { class: "nv-list",
                        for entry in entries {
                            div { key: "{entry.id}", class: "nv-list__item",
                                div { class: "nv-list__main",
                                    div { class: "nv-list__title",
                                        "{entry.actor_name.clone().unwrap_or_else(|| \"Unbekannt\".into())} {audit_label(&entry.action)} "
                                        if let Some(target) = name_of(&entry.target_id) {
                                            "{target}"
                                        }
                                    }
                                    div { class: "nv-list__sub",
                                        span { class: "nv-audit__action", "{entry.action}" }
                                        if let Some(detail) = entry.detail.clone() { " · {detail}" }
                                        if let Some(reason) = entry.reason.clone() { " · Grund: {reason}" }
                                    }
                                }
                                span { class: "nv-faint nv-num", "{day_label(&entry.created_at)} {time_label(&entry.created_at)}" }
                            }
                        }
                    }
                },
            }
        }
    }
}
