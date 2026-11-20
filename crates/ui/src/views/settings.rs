//! Nutzer- und Server-Einstellungen.

use dioxus::{core::spawn_forever, prelude::*};
use serde_json::json;

use crate::{
    actions, api,
    components::{Icon, Kbd, Modal, Skeleton, classes, day_label, time_label},
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
                    span { class: "nv-field__hint", "Dein Benutzername @{handle} lässt sich nicht ändern." }
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
        VideoSection {}
    }
}

/// Bildschirm teilen, Kamera und Empfangsqualität.
#[component]
fn VideoSection() -> Element {
    let mut store = use_store();
    let cameras = use_resource(crate::voice::camera_devices);
    let settings = store.settings.read().clone();
    rsx! {
        section { class: "nv-section",
            h3 { "Bildschirm teilen" }
            div { class: "nv-field",
                label { class: "nv-field__label", r#for: "screen-quality", "Qualität" }
                select {
                    id: "screen-quality",
                    class: "nv-select",
                    onchange: move |event| {
                        if let Some(quality) = ScreenQuality::ALL.into_iter().find(|q| q.key() == event.value()) {
                            store.settings.write().screen_quality = quality;
                            save_settings(store);
                        }
                    },
                    for quality in ScreenQuality::ALL {
                        option { key: "{quality.key()}", value: "{quality.key()}", selected: settings.screen_quality == quality, "{quality.label()}" }
                    }
                }
                span { class: "nv-field__hint", "Gilt ab der nächsten Freigabe. Andere bekommen je nach Ansicht automatisch eine kleinere Stufe." }
            }
            label { class: "nv-check",
                input {
                    r#type: "checkbox",
                    checked: settings.screen_audio,
                    onchange: move |event| {
                        store.settings.write().screen_audio = event.checked();
                        save_settings(store);
                    },
                }
                span { class: "nv-check__text",
                    span { "Ton mitsenden" }
                    span { class: "nv-check__hint", "Ton eines Tabs, unter Windows auch der Systemton – sofern der Browser es anbietet." }
                }
            }
        }
        section { class: "nv-section",
            h3 { "Kamera" }
            div { class: "nv-field",
                label { class: "nv-field__label", r#for: "camera-device", "Gerät" }
                select {
                    id: "camera-device",
                    class: "nv-select",
                    onchange: move |event| {
                        let value = event.value();
                        store.settings.write().camera_device = (!value.is_empty()).then_some(value);
                        save_settings(store);
                    },
                    option { value: "", selected: settings.camera_device.is_none(), "Systemstandard" }
                    if let Some(list) = cameras.read().as_ref() {
                        for (id, label) in list.clone() {
                            option { key: "{id}", value: "{id}", selected: settings.camera_device.as_deref() == Some(id.as_str()), "{label}" }
                        }
                    }
                }
            }
            div { class: "nv-field",
                label { class: "nv-field__label", r#for: "receive-quality", "Empfang von Bildschirmen und Kameras" }
                select {
                    id: "receive-quality",
                    class: "nv-select",
                    onchange: move |event| {
                        if let Some(quality) = ReceiveQuality::ALL.into_iter().find(|q| q.key() == event.value()) {
                            store.settings.write().receive_quality = quality;
                            save_settings(store);
                            crate::voice::apply_layers(store);
                        }
                    },
                    for quality in ReceiveQuality::ALL {
                        option { key: "{quality.key()}", value: "{quality.key()}", selected: settings.receive_quality == quality, "{quality.label()}" }
                    }
                }
                span { class: "nv-field__hint", "„Datensparend“ holt kleinere Bilder, gut für mobile Daten." }
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

fn channel_icon(kind: i16) -> &'static str {
    match kind {
        KIND_VOICE => "volume",
        KIND_FORUM => "forum",
        _ => "hash",
    }
}

fn channel_kind_label(kind: i16) -> &'static str {
    match kind {
        KIND_VOICE => "Sprachkanal",
        KIND_FORUM => "Forum",
        _ => "Textkanal",
    }
}

#[component]
fn ChannelsTab() -> Element {
    let store = use_store();
    let mut selected = use_signal(|| Option::<String>::None);
    let detail = store.detail.read().clone().unwrap_or_default();
    let can = detail.can(perm::MANAGE_CHANNELS);
    let can_edit = (detail.can(perm::MANAGE_ROLES) && can) || detail.can(perm::MANAGE_WEBHOOKS);
    if let Some(channel) = selected().and_then(|id| detail.channel(&id).cloned()) {
        return rsx! { ChannelDetail { channel, onback: move |_| selected.set(None) } };
    }
    let mut channels: Vec<Channel> = detail
        .channels
        .iter()
        .filter(|c| c.kind != KIND_THREAD)
        .cloned()
        .collect();
    channels.sort_by_key(|c| (c.kind == KIND_VOICE, c.position));
    rsx! {
        section { class: "nv-section",
            h3 { "Kanäle" }
            p { class: "nv-muted", "Slowmode begrenzt, wie oft jemand schreiben darf (Moderation ist ausgenommen). „Bearbeiten“ öffnet Berechtigungen und Webhooks." }
            if !can {
                p { class: "nv-faint", "Zum Ändern brauchst du „Kanäle verwalten“." }
            }
            div { class: "nv-list",
                for channel in channels {
                    {
                        let id = channel.id.clone();
                        let open_id = channel.id.clone();
                        let has_slowmode = channel.kind != KIND_VOICE;
                        rsx! {
                            div { key: "{channel.id}", class: "nv-list__item",
                                Icon { name: channel_icon(channel.kind), size: 16 }
                                div { class: "nv-list__main",
                                    div { class: "nv-list__title", "{channel.name}" }
                                    div { class: "nv-list__sub",
                                        "{channel.topic.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| channel_kind_label(channel.kind).to_owned())}"
                                    }
                                }
                                if has_slowmode {
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
                                if can_edit {
                                    button {
                                        class: "nv-btn nv-btn--sm",
                                        "aria-label": "{channel.name} bearbeiten",
                                        onclick: move |_| selected.set(Some(open_id.clone())),
                                        "Bearbeiten"
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

/// Ein Kanal im Detail: Berechtigungen (Overrides) und Webhooks.
#[component]
fn ChannelDetail(channel: Channel, onback: EventHandler<()>) -> Element {
    let store = use_store();
    let can_overrides = store.can(perm::MANAGE_ROLES) && store.can(perm::MANAGE_CHANNELS);
    let can_webhooks = store.can(perm::MANAGE_WEBHOOKS) && channel.kind == KIND_TEXT;
    rsx! {
        section { class: "nv-section",
            div { class: "nv-row",
                button { class: "nv-btn nv-btn--ghost nv-btn--sm", onclick: move |_| onback.call(()), "← Alle Kanäle" }
            }
            div { class: "nv-row",
                Icon { name: channel_icon(channel.kind), size: 18 }
                h3 { "{channel.name}" }
                span { class: "nv-faint", "{channel_kind_label(channel.kind)}" }
            }
        }
        if can_overrides {
            OverrideEditor { key: "{channel.id}", channel: channel.clone() }
        } else {
            section { class: "nv-section",
                h3 { "Berechtigungen" }
                p { class: "nv-faint", "Dafür brauchst du „Rollen verwalten“ und „Kanäle verwalten“." }
            }
        }
        if can_webhooks {
            WebhookSection { key: "wh-{channel.id}", channel_id: channel.id.clone() }
        }
    }
}

/// Wer in einem Override gemeint ist: (Art, ID, Anzeigename).
fn override_target_name(detail: &GuildDetail, kind: &str, id: &str) -> String {
    if kind == "role" {
        detail
            .roles
            .iter()
            .find(|role| role.id == id)
            .map(|role| role.name.clone())
            .unwrap_or_else(|| "Gelöschte Rolle".into())
    } else {
        detail
            .member(id)
            .map(|member| member.display_name.clone())
            .unwrap_or_else(|| "Ehemaliges Mitglied".into())
    }
}

/// Rechte-Editor für Kanal-Overrides: pro Rolle bzw. Mitglied und Recht
/// „Verweigern“, „Standard“ (von der Rolle geerbt) oder „Erlauben“. Jede
/// Änderung wird sofort gespeichert.
#[component]
fn OverrideEditor(channel: Channel) -> Element {
    let store = use_store();
    let detail = store.detail.read().clone().unwrap_or_default();
    let channel_id = channel.id.clone();
    let loaded = use_resource(use_reactive!(|channel_id| async move {
        api::overrides(&channel_id).await
    }));
    let mut entries = use_signal(Vec::<ChannelOverride>::new);
    let mut ready = use_signal(|| false);
    use_effect(move || {
        if let Some(Ok(list)) = &*loaded.read() {
            entries.set(list.clone());
            ready.set(true);
        }
    });
    let everyone = detail
        .roles
        .iter()
        .find(|role| role.is_everyone)
        .map(|role| role.id.clone())
        .unwrap_or_default();
    let mut selected = use_signal(|| ("role".to_owned(), String::new()));
    let current = {
        let (kind, id) = selected.read().clone();
        if id.is_empty() {
            ("role".to_owned(), everyone.clone())
        } else {
            (kind, id)
        }
    };
    let rows: &[(u64, &str, &str)] = if channel.kind == KIND_VOICE {
        VOICE_OVERRIDES
    } else {
        TEXT_OVERRIDES
    };
    // Liste: @everyone immer, dann alle mit Override.
    let mut targets: Vec<(String, String)> = vec![("role".into(), everyone.clone())];
    for entry in entries.read().iter() {
        if !(entry.kind == "role" && entry.target_id == everyone) {
            targets.push((entry.kind.clone(), entry.target_id.clone()));
        }
    }
    if !targets.contains(&current) {
        targets.push(current.clone());
    }
    let addable_roles: Vec<Role> = detail
        .roles
        .iter()
        .filter(|role| {
            !role.is_everyone && !targets.iter().any(|(k, id)| k == "role" && *id == role.id)
        })
        .cloned()
        .collect();
    let addable_members: Vec<Member> = detail
        .members
        .iter()
        .filter(|m| {
            !targets
                .iter()
                .any(|(k, id)| k == "member" && *id == m.user_id)
        })
        .cloned()
        .collect();
    let entry = entries
        .read()
        .iter()
        .find(|e| e.kind == current.0 && e.target_id == current.1)
        .cloned()
        .unwrap_or(ChannelOverride {
            kind: current.0.clone(),
            target_id: current.1.clone(),
            allow: 0,
            deny: 0,
        });
    let current_name = override_target_name(&detail, &current.0, &current.1);
    let save_channel = channel.id.clone();

    if !ready() {
        return rsx! {
            section { class: "nv-section",
                h3 { "Berechtigungen" }
                if let Some(Err(err)) = &*loaded.read() {
                    p { class: "nv-field__error", role: "alert", "{err}" }
                } else {
                    Skeleton { lines: 3 }
                }
            }
        };
    }

    rsx! {
        section { class: "nv-section",
            h3 { "Berechtigungen" }
            p { class: "nv-muted", "Überschreibt die Rollenrechte nur in diesem Kanal. „Standard“ übernimmt, was die Rolle ohnehin darf; Verweigern schlägt Erlauben auf derselben Stufe, das Mitglied schlägt die Rolle." }
            div { class: "nv-overrides",
                div { class: "nv-overrides__targets", role: "listbox", "aria-label": "Rolle oder Mitglied",
                    for (kind, id) in targets.clone() {
                        {
                            let active = (kind.clone(), id.clone()) == current;
                            let name = if kind == "role" && id == everyone {
                                "@everyone".to_owned()
                            } else {
                                override_target_name(&detail, &kind, &id)
                            };
                            let pick = (kind.clone(), id.clone());
                            rsx! {
                                button {
                                    key: "{kind}-{id}",
                                    class: classes(&["nv-overrides__target", if active { "is-selected" } else { "" }]),
                                    role: "option",
                                    "aria-selected": if active { "true" } else { "false" },
                                    onclick: move |_| selected.set(pick.clone()),
                                    Icon { name: if kind == "role" { "shield" } else { "user" }, size: 14 }
                                    span { "{name}" }
                                }
                            }
                        }
                    }
                    label { class: "sr-only", r#for: "override-add", "Rolle oder Mitglied hinzufügen" }
                    select {
                        id: "override-add",
                        class: "nv-select",
                        onchange: move |event| {
                            let value = event.value();
                            if let Some((kind, id)) = value.split_once(':') {
                                selected.set((kind.to_owned(), id.to_owned()));
                            }
                        },
                        option { value: "", selected: true, "+ Hinzufügen …" }
                        if !addable_roles.is_empty() {
                            optgroup { label: "Rollen",
                                for role in addable_roles {
                                    option { key: "r{role.id}", value: "role:{role.id}", "{role.name}" }
                                }
                            }
                        }
                        if !addable_members.is_empty() {
                            optgroup { label: "Mitglieder",
                                for member in addable_members {
                                    option { key: "m{member.user_id}", value: "member:{member.user_id}", "{member.display_name}" }
                                }
                            }
                        }
                    }
                }
                div { class: "nv-overrides__perms",
                    div { class: "nv-row nv-row--between",
                        strong { if current.0 == "role" && current.1 == everyone { "@everyone" } else { "{current_name}" } }
                        if entry.allow != 0 || entry.deny != 0 {
                            button {
                                class: "nv-btn nv-btn--ghost nv-btn--sm",
                                onclick: {
                                    let channel = save_channel.clone();
                                    let entry = entry.clone();
                                    move |_| {
                                        let channel = channel.clone();
                                        let entry = entry.clone();
                                        entries.write().retain(|e| !(e.kind == entry.kind && e.target_id == entry.target_id));
                                        spawn_forever(async move {
                                            if let Err(err) = api::delete_override(&channel, &entry.kind, &entry.target_id).await {
                                                store.error(err);
                                            }
                                        });
                                    }
                                },
                                "Alles auf Standard"
                            }
                        }
                    }
                    for (bit, label, hint) in rows.iter().copied() {
                        {
                            let state = entry.state(bit);
                            let base = entry.clone();
                            let channel = save_channel.clone();
                            let choose = move |next: OverrideState| {
                                let mut changed = base.clone();
                                changed.set(bit, next);
                                let channel = channel.clone();
                                {
                                    let mut list = entries.write();
                                    list.retain(|e| !(e.kind == changed.kind && e.target_id == changed.target_id));
                                    if changed.allow != 0 || changed.deny != 0 {
                                        list.push(changed.clone());
                                    }
                                }
                                spawn_forever(async move {
                                    let result = if changed.allow == 0 && changed.deny == 0 {
                                        api::delete_override(&channel, &changed.kind, &changed.target_id).await
                                    } else {
                                        api::set_override(&channel, &changed).await
                                    };
                                    if let Err(err) = result {
                                        store.error(err);
                                    }
                                });
                            };
                            let mut deny = choose.clone();
                            let mut inherit = choose.clone();
                            let mut allow = choose;
                            rsx! {
                                div { key: "{bit}", class: "nv-perm",
                                    div { class: "nv-perm__text",
                                        span { class: "nv-perm__label", "{label}" }
                                        if !hint.is_empty() {
                                            span { class: "nv-perm__hint", "{hint}" }
                                        }
                                    }
                                    div { class: "nv-tristate", role: "radiogroup", "aria-label": "{label}",
                                        button {
                                            class: "is-deny",
                                            role: "radio",
                                            "aria-checked": if state == OverrideState::Deny { "true" } else { "false" },
                                            title: "Verweigern",
                                            onclick: move |_| deny(OverrideState::Deny),
                                            Icon { name: "close", size: 14 }
                                            span { class: "sr-only", "Verweigern" }
                                        }
                                        button {
                                            role: "radio",
                                            "aria-checked": if state == OverrideState::Inherit { "true" } else { "false" },
                                            title: "Standard",
                                            onclick: move |_| inherit(OverrideState::Inherit),
                                            "/"
                                            span { class: "sr-only", "Standard" }
                                        }
                                        button {
                                            class: "is-allow",
                                            role: "radio",
                                            "aria-checked": if state == OverrideState::Allow { "true" } else { "false" },
                                            title: "Erlauben",
                                            onclick: move |_| allow(OverrideState::Allow),
                                            Icon { name: "check", size: 14 }
                                            span { class: "sr-only", "Erlauben" }
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

/// Webhooks eines Textkanals: anlegen (URL nur einmal sichtbar), löschen.
#[component]
fn WebhookSection(channel_id: String) -> Element {
    let store = use_store();
    let id = channel_id.clone();
    let mut hooks = use_resource(use_reactive!(|id| async move { api::webhooks(&id).await }));
    let mut name = use_signal(String::new);
    let mut created = use_signal(|| Option::<Webhook>::None);
    let mut busy = use_signal(|| false);
    let create_channel = channel_id.clone();
    rsx! {
        section { class: "nv-section",
            h3 { "Webhooks" }
            p { class: "nv-muted", "Andere Dienste (CI, Monitoring, Kalender) posten über eine geheime URL in diesen Kanal – ohne Konto." }
            form {
                class: "nv-row",
                onsubmit: move |event| {
                    event.prevent_default();
                    let value = name.read().trim().to_owned();
                    if value.is_empty() {
                        return;
                    }
                    let channel = create_channel.clone();
                    busy.set(true);
                    spawn_forever(async move {
                        match api::create_webhook(&channel, &value).await {
                            Ok(hook) => {
                                created.set(Some(hook));
                                name.set(String::new());
                                hooks.restart();
                            }
                            Err(err) => store.error(err),
                        }
                        busy.set(false);
                    });
                },
                label { class: "sr-only", r#for: "webhook-name", "Name des Webhooks" }
                input { id: "webhook-name", class: "nv-input", maxlength: "48", placeholder: "Name, z. B. Build-Bot", value: "{name}", oninput: move |e| name.set(e.value()) }
                button { r#type: "submit", class: "nv-btn", disabled: busy() || name.read().trim().is_empty(), Icon { name: "plus", size: 16 } "Anlegen" }
            }
            if let Some(hook) = created() {
                div { class: "nv-secret", role: "status",
                    strong { "URL für „{hook.name}“" }
                    p { class: "nv-faint", "Nur jetzt sichtbar. Behandle sie wie ein Passwort – wer sie kennt, kann in diesen Kanal schreiben." }
                    div { class: "nv-row",
                        input { class: "nv-input", readonly: true, value: "{hook.url.clone().unwrap_or_default()}", "aria-label": "Webhook-URL" }
                        button {
                            class: "nv-btn",
                            onclick: {
                                let url = hook.url.clone().unwrap_or_default();
                                move |_| {
                                    if let Some(window) = web_sys::window() {
                                        let _ = window.navigator().clipboard().write_text(&url);
                                        store.notice("URL kopiert.");
                                    }
                                }
                            },
                            "Kopieren"
                        }
                    }
                    code { class: "nv-code", "curl -X POST -H 'Content-Type: application/json' -d '{{\"content\":\"Hallo\"}}' <URL>" }
                }
            }
            match &*hooks.read() {
                None => rsx! { Skeleton { lines: 2 } },
                Some(Err(err)) => rsx! { p { class: "nv-field__error", role: "alert", "{err}" } },
                Some(Ok(list)) if list.is_empty() => rsx! { p { class: "nv-faint", "Noch keine Webhooks." } },
                Some(Ok(list)) => rsx! {
                    div { class: "nv-list",
                        for hook in list.clone() {
                            {
                                let hook_id = hook.id.clone();
                                rsx! {
                                    div { key: "{hook.id}", class: "nv-list__item",
                                        Icon { name: "webhook", size: 16 }
                                        div { class: "nv-list__main",
                                            div { class: "nv-list__title", "{hook.name}" }
                                            div { class: "nv-list__sub", "angelegt {crate::components::day_label(&hook.created_at)}" }
                                        }
                                        button {
                                            class: "nv-btn nv-btn--danger nv-btn--sm",
                                            onclick: move |_| {
                                                let id = hook_id.clone();
                                                spawn_forever(async move {
                                                    match api::delete_webhook(&id).await {
                                                        Ok(()) => {
                                                            store.notice("Webhook gelöscht.");
                                                            hooks.restart();
                                                        }
                                                        Err(err) => store.error(err),
                                                    }
                                                });
                                            },
                                            "Löschen"
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
                                            div { class: "nv-list__sub", "{ban.reason.clone().unwrap_or_else(|| \"Kein Grund angegeben\".into())}, gebannt am {day_label(&ban.created_at)}" }
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
                                        if let Some(detail) = entry.detail.clone() { ": {detail}" }
                                        if let Some(reason) = entry.reason.clone() { " (Grund: {reason})" }
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
