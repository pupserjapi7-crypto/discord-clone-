//! Komponenten-Galerie unter `/dev/gallery` – nur mit Feature `gallery`
//! (`cargo xtask gallery`), nie im ausgelieferten Build.
//!
//! `?screen=` zeigt einen ganzen Bildschirm mit Testdaten (für
//! `docs/screenshots/`), `?theme=light|dark` erzwingt das Theme:
//! login, chat, voice, screenshare, palette, settings, moderation, thread.

use std::collections::HashMap;

use dioxus::prelude::*;

use crate::{
    components::{Avatar, EmptyState, Icon, IconButton, Kbd, Skeleton, StatusDot},
    store::{History, Overlay, ServerTab, SettingsTab, Store},
    types::*,
    views,
};

pub fn active() -> bool {
    web_sys::window()
        .and_then(|window| window.location().pathname().ok())
        .is_some_and(|path| path.starts_with("/dev/gallery"))
}

fn query(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")).map(str::to_owned))
}

fn user(id: &str, name: &str, handle: &str) -> User {
    User {
        id: id.into(),
        handle: handle.into(),
        display_name: name.into(),
        ..Default::default()
    }
}

fn message(id: u64, author: &User, minutes_ago: i64, content: &str) -> Message {
    let now = js_sys::Date::now();
    let created = js_sys::Date::new(&(now - minutes_ago as f64 * 60_000.0).into());
    Message {
        id: id.to_string(),
        channel_id: "10".into(),
        author: author.clone(),
        content: content.into(),
        created_at: created.to_iso_string().as_string().unwrap_or_default(),
        ..Default::default()
    }
}

fn channel(id: &str, name: &str, kind: i16) -> Channel {
    Channel {
        id: id.into(),
        guild_id: "1".into(),
        name: name.into(),
        kind,
        ..Default::default()
    }
}

fn member(user: &User, status: i16, roles: &[&str]) -> Member {
    Member {
        user_id: user.id.clone(),
        handle: user.handle.clone(),
        display_name: user.display_name.clone(),
        status,
        role_ids: roles.iter().map(|r| r.to_string()).collect(),
        ..Default::default()
    }
}

/// Store mit realistischen Testdaten für die Bildschirm-Ansichten.
fn mock_store(screen: &str) -> Store {
    let mut store = Store::create();
    let me = user("100", "Mara Lenz", "mara");
    let jonas = user("101", "Jonas", "jonas");
    let ida = user("102", "Ida Krüger", "ida.k");
    let tom = user("103", "Tom", "tom_");
    let lea = user("104", "Lea", "lea");
    let mut bot = user("105", "Patchnotes", "patchbot");
    bot.bot = true;

    let mut general = channel("10", "allgemein", KIND_TEXT);
    general.topic = Some("Alles rund um die Freitagsrunde · Regeln im Pin".into());
    let mut clips = channel("11", "clips", KIND_TEXT);
    clips.slowmode_secs = 30;
    let mut thread = channel("15", "Map-Pool für Freitag", KIND_THREAD);
    thread.parent_id = Some("10".into());
    thread.root_message_id = Some("1003".into());
    thread.owner_id = Some("101".into());
    let detail = GuildDetail {
        guild: Guild {
            id: "1".into(),
            name: "Freitagsrunde".into(),
            owner_id: "100".into(),
        },
        channels: vec![
            general,
            clips,
            channel("12", "ankündigungen", KIND_TEXT),
            thread,
            channel("20", "Lounge", KIND_VOICE),
            channel("21", "Ranked", KIND_VOICE),
        ],
        members: vec![
            member(&me, 0, &["900"]),
            member(&jonas, 0, &["901"]),
            member(&ida, 1, &[]),
            member(&tom, 2, &[]),
            member(&lea, 3, &[]),
            Member {
                bot: true,
                ..member(&bot, 0, &[])
            },
        ],
        roles: vec![
            Role {
                id: "900".into(),
                name: "Owner".into(),
                position: 100,
                permissions: -1,
                ..Default::default()
            },
            Role {
                id: "901".into(),
                name: "Moderation".into(),
                position: 10,
                permissions: 0x2f0,
                ..Default::default()
            },
            Role {
                id: "902".into(),
                name: "@everyone".into(),
                position: 0,
                is_everyone: true,
                permissions: 0x9c7d80,
                ..Default::default()
            },
        ],
        permissions: u64::MAX,
        unread: vec![Unread {
            channel_id: "11".into(),
            last_message_id: Some("2000".into()),
            last_read_id: "1990".into(),
        }],
    };

    let messages = vec![
        message(
            1001,
            &jonas,
            46,
            "Wer ist heute Abend dabei? Ich wäre ab 20 Uhr da.",
        ),
        message(1002, &jonas, 45, "Hab den neuen Patch schon geladen 🎮"),
        message(
            1003,
            &ida,
            31,
            "Bin dabei! Sollen wir vorher kurz über den Map-Pool reden?",
        ),
        message(
            1004,
            &bot,
            30,
            "Patch 1.4.2 ist live: neue Map „Hafen“, Balance-Änderungen an zwei Waffen.",
        ),
        message(
            1005,
            &me,
            12,
            "Klingt gut. @jonas kannst du die Lobby aufmachen?",
        ),
        message(1006, &tom, 4, "@mara ich komme etwas später, so 20:30."),
        message(1007, &jonas, 1, "Lobby steht, Code im Sprachkanal 👍"),
    ];
    let mut history = HashMap::new();
    let mut main_history = History {
        messages,
        complete: true,
        read_marker: Some("1005".into()),
        ..Default::default()
    };
    main_history.messages[3].pinned = true;
    main_history.messages[6].reactions = vec![
        Reaction {
            emoji: "👍".into(),
            count: 3,
            me: true,
        },
        Reaction {
            emoji: "🔥".into(),
            count: 1,
            me: false,
        },
    ];
    history.insert("10".to_owned(), main_history);
    history.insert(
        "15".to_owned(),
        History {
            messages: vec![
                message(1101, &jonas, 25, "Vorschlag: Hafen, Werk, Altstadt."),
                message(
                    1102,
                    &ida,
                    22,
                    "Altstadt lieber raus, die ist nach dem Patch zu eng.",
                ),
            ],
            complete: true,
            ..Default::default()
        },
    );

    let lounge = vec![
        VoiceParticipant {
            user_id: "100".into(),
            handle: "mara".into(),
            display_name: "Mara Lenz".into(),
            ..Default::default()
        },
        VoiceParticipant {
            user_id: "101".into(),
            handle: "jonas".into(),
            display_name: "Jonas".into(),
            screen: screen == "screenshare",
            ..Default::default()
        },
        VoiceParticipant {
            user_id: "102".into(),
            handle: "ida.k".into(),
            display_name: "Ida Krüger".into(),
            muted: true,
            ..Default::default()
        },
        VoiceParticipant {
            user_id: "103".into(),
            handle: "tom_".into(),
            display_name: "Tom".into(),
            deafened: true,
            ..Default::default()
        },
    ];

    store.user.set(Some(me));
    store.booting.set(false);
    store.status.set(crate::store::ConnStatus::Online);
    store.guilds.set(vec![
        GuildSummary {
            id: "1".into(),
            name: "Freitagsrunde".into(),
        },
        GuildSummary {
            id: "2".into(),
            name: "Uni Lerngruppe".into(),
        },
        GuildSummary {
            id: "3".into(),
            name: "Speedrun DE".into(),
        },
    ]);
    store.guilds_loaded.set(true);
    store.detail.set(Some(detail));
    store.history.set(history);
    store.voice_rooms.write().insert("20".into(), lounge);

    let voice_screen = matches!(screen, "voice" | "screenshare");
    store
        .channel_id
        .set(Some(if voice_screen { "20" } else { "10" }.into()));
    if voice_screen || screen == "chat" {
        let mut voice = store.voice.write();
        voice.channel = Some(channel("20", "Lounge", KIND_VOICE));
        voice.joined = true;
        voice.ping_ms = Some(38);
        voice.speaking = vec!["101".into()];
        if screen == "screenshare" {
            voice.video_users = vec!["101".into()];
        }
    }
    if screen == "chat" {
        store.typing.write().push(crate::store::Typing {
            channel_id: "10".into(),
            user_id: "102".into(),
            name: "Ida Krüger".into(),
            until_ms: f64::MAX,
        });
    }
    match screen {
        "palette" => store.overlay.set(Overlay::Palette),
        "settings" => store.overlay.set(Overlay::Settings(SettingsTab::Voice)),
        "moderation" => store.overlay.set(Overlay::ServerSettings(ServerTab::Roles)),
        "member" => store.overlay.set(Overlay::Member("101".into())),
        "thread" => store.thread_id.set(Some("15".into())),
        _ => {}
    }
    store
}

fn apply_theme_from_query() {
    if let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        // Screenshots ohne Ein-/Ausblenden, damit sie reproduzierbar sind.
        let _ = root.class_list().add_1("nv-static");
        match query("theme").as_deref() {
            Some("light") => {
                let _ = root.set_attribute("data-theme", "light");
            }
            Some("dark") => {
                let _ = root.set_attribute("data-theme", "dark");
            }
            _ => {}
        }
    }
}

#[component]
pub fn Gallery() -> Element {
    use_hook(apply_theme_from_query);
    match query("screen") {
        Some(screen) => rsx! { Screen { screen } },
        None => rsx! { Components {} },
    }
}

#[component]
fn Screen(screen: String) -> Element {
    let name = screen.clone();
    let store = use_context_provider(move || {
        let store = mock_store(&name);
        // Nach dem Store: der lädt sonst die gespeicherten Einstellungen darüber.
        apply_theme_from_query();
        store
    });
    if screen == "login" {
        return rsx! { views::auth::AuthScreen {} };
    }
    if screen == "onboarding" {
        let mut store = store;
        store.guilds.set(Vec::new());
        store.detail.set(None);
    }
    rsx! { views::shell::Shell {} }
}

#[component]
fn Components() -> Element {
    let swatches = [
        "--nv-bg-0",
        "--nv-bg-1",
        "--nv-bg-2",
        "--nv-bg-3",
        "--nv-bg-4",
        "--nv-accent",
        "--nv-online",
        "--nv-idle",
        "--nv-danger",
        "--nv-offline",
    ];
    rsx! {
        div { class: "nv-gallery",
            h1 { "Nova – Komponenten" }
            p { class: "nv-muted",
                "Bildschirme: "
                for screen in ["login", "onboarding", "chat", "thread", "voice", "screenshare", "palette", "settings", "moderation", "member"] {
                    a { key: "{screen}", href: "/dev/gallery?screen={screen}", style: "color: var(--nv-accent); margin-right: 12px", "{screen}" }
                }
            }
            div { class: "nv-gallery__themes",
                for theme in ["dark", "light"] {
                    div { key: "{theme}", class: "nv-gallery__pane", "data-theme": "{theme}",
                        h3 { "Theme: {theme}" }
                        div { class: "nv-gallery__row",
                            for token in swatches {
                                span { key: "{token}", class: "nv-gallery__swatch", title: "{token}", style: "background: var({token})" }
                            }
                        }
                        h3 { "Buttons" }
                        div { class: "nv-gallery__row",
                            button { class: "nv-btn nv-btn--primary", "Primär" }
                            button { class: "nv-btn", "Sekundär" }
                            button { class: "nv-btn nv-btn--ghost", "Ghost" }
                            button { class: "nv-btn nv-btn--danger", "Gefahr" }
                            button { class: "nv-btn nv-btn--primary is-loading", "Lädt …" }
                            button { class: "nv-btn", disabled: true, "Deaktiviert" }
                        }
                        h3 { "Icon-Buttons" }
                        div { class: "nv-gallery__row",
                            IconButton { icon: "mic", label: "normal", onclick: |_| {} }
                            IconButton { icon: "mic-off", label: "aktiv", active: true, onclick: |_| {} }
                            IconButton { icon: "trash", label: "gefährlich", danger: true, onclick: |_| {} }
                            IconButton { icon: "screen", label: "deaktiviert", disabled: true, onclick: |_| {} }
                            IconButton { icon: "phone-off", label: "groß", large: true, onclick: |_| {} }
                        }
                        h3 { "Eingaben" }
                        input { class: "nv-input", placeholder: "Platzhalter" }
                        input { class: "nv-input", value: "Fehlerhaft", "aria-invalid": "true" }
                        h3 { "Avatare & Status" }
                        div { class: "nv-gallery__row",
                            Avatar { name: "Mara Lenz", status: Some(0) }
                            Avatar { name: "Jonas", status: Some(1) }
                            Avatar { name: "Ida Krüger", status: Some(2) }
                            Avatar { name: "Tom", status: Some(3) }
                            Avatar { name: "Lea", speaking: true, size: 40 }
                            Avatar { name: "Patchnotes", bot: true }
                            StatusDot { status: 0 }
                            span { class: "nv-badge", "Badge" }
                            span { class: "nv-badge nv-badge--accent", "Live" }
                            span { class: "nv-count", "3" }
                            Kbd { "Strg K" }
                        }
                        h3 { "Zustände" }
                        Skeleton { lines: 2 }
                        EmptyState { icon: "inbox", title: "Leer", text: "Hier ist noch nichts.",
                            button { class: "nv-btn nv-btn--primary", "Aktion" }
                        }
                        div { class: "nv-gallery__row",
                            span { class: "nv-ping is-good", "38 ms" }
                            span { class: "nv-ping is-ok", "84 ms" }
                            span { class: "nv-ping is-bad", "190 ms" }
                            Icon { name: "logo" }
                        }
                    }
                }
            }
        }
    }
}
