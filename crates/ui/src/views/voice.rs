//! Voice-Grid und schwebendes Voice-Dock.

use dioxus::prelude::*;

use crate::{
    actions,
    components::{Avatar, EmptyState, Icon, IconButton, classes},
    store::{Overlay, SettingsTab, use_store},
    types::*,
    voice,
};

#[component]
pub fn VoiceStage(channel: Channel) -> Element {
    let store = use_store();
    let voice_state = store.voice.read().clone();
    let people = store
        .voice_rooms
        .read()
        .get(&channel.id)
        .cloned()
        .unwrap_or_default();
    let here = voice_state
        .channel
        .as_ref()
        .map(|c| c.id == channel.id)
        .unwrap_or(false);
    let can_connect = store.can(perm::CONNECT);
    let me = store.me_id();
    // Wessen Kamerabild gerade zu sehen ist (fremde, sobald es ankam; die
    // eigene Vorschau, solange die Kamera läuft).
    let cameras: Vec<String> = people
        .iter()
        .filter(|p| {
            p.camera && voice_state.camera_users.contains(&p.user_id)
                || (here && voice_state.camera && me.as_deref() == Some(p.user_id.as_str()))
        })
        .map(|p| p.user_id.clone())
        .collect();
    let streamers: Vec<VoiceParticipant> = people
        .iter()
        .filter(|p| p.screen && voice_state.video_users.contains(&p.user_id))
        .cloned()
        .collect();

    // Videos in ihre Kacheln hängen, sobald sie gerendert sind – nach jeder
    // Änderung von Voice-Zustand oder Teilnehmerliste, weil Dioxus dabei
    // Kacheln neu anlegen kann (die Video-Elemente hängen wir selbst ein).
    let mount: Vec<String> = streamers.iter().map(|p| p.user_id.clone()).collect();
    let mount_cameras = cameras.clone();
    use_effect(use_reactive!(|mount, mount_cameras| {
        let _ = (
            store.voice.read().video_users.len(),
            store.voice_rooms.read().len(),
        );
        for user in &mount {
            voice::mount_video(user);
        }
        for user in &mount_cameras {
            voice::mount_camera(user);
        }
    }));

    // Beitritts-Moment: nur direkt nach dem Beitreten erscheinen die Kacheln
    // einmal nacheinander (CSS `.is-entering`), nicht beim bloßen Hinsehen.
    let mut entering = use_signal(|| false);
    let mut was_here = use_signal(|| here);
    use_effect(use_reactive!(|here| {
        if here && !*was_here.peek() {
            entering.set(true);
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(400).await;
                entering.set(false);
            });
        }
        was_here.set(here);
    }));

    if people.is_empty() && !here {
        let join_channel = channel.clone();
        return rsx! {
            div { class: "nv-stage",
                EmptyState {
                    icon: "volume",
                    title: "{channel.name}",
                    text: if can_connect {
                        "Noch niemand hier. Tritt bei – die anderen sehen dich sofort in der Kanalliste."
                    } else {
                        "Dir fehlt das Recht, diesem Sprachkanal beizutreten."
                    },
                    if can_connect {
                        button {
                            class: if voice_state.connecting { "nv-btn nv-btn--primary is-loading" } else { "nv-btn nv-btn--primary" },
                            disabled: voice_state.connecting,
                            onclick: move |_| { spawn(actions::join_voice(store, join_channel.clone())); },
                            Icon { name: "volume", size: 16 }
                            if voice_state.connecting { "Verbinde …" } else { "Beitreten" }
                        }
                    }
                }
            }
        };
    }

    let tile = |index: usize, person: &VoiceParticipant, feature: bool| {
        let speaking = voice_state.speaking.contains(&person.user_id);
        let has_video = feature;
        // Kamera nur in der Personen-Kachel, nicht in der Bildschirm-Kachel.
        let has_camera = !feature && cameras.contains(&person.user_id);
        let is_me = me.as_deref() == Some(person.user_id.as_str());
        rsx! {
            div {
                key: "{person.user_id}-{feature}",
                class: classes(&["nv-tile", if feature { "nv-tile--feature" } else { "" }, if has_camera { "nv-tile--camera" } else { "" }, if speaking { "is-speaking" } else { "" }]),
                style: "--i: {index}",
                if has_video {
                    span { class: "nv-tile__wait",
                        Icon { name: "screen", size: 28 }
                        "Bildschirm von {person.display_name} wird geladen …"
                    }
                    div { class: "nv-tile__slot", id: "vslot-{person.user_id}" }
                } else if has_camera {
                    div {
                        class: "nv-tile__slot",
                        id: "cslot-{person.user_id}",
                        "data-me": if is_me { "true" } else { "false" },
                    }
                } else {
                    Avatar { name: person.display_name.clone(), size: 72, speaking }
                }
                span { class: "nv-tile__name",
                    if person.screen && !feature {
                        Icon { name: "screen", size: 14 }
                    }
                    "{person.display_name}"
                }
                if person.muted || person.deafened {
                    div { class: "nv-tile__icons",
                        if person.deafened {
                            span { title: "Taub geschaltet", Icon { name: "headphones-off", size: 14 } }
                        } else {
                            span { title: "Stumm", Icon { name: "mic-off", size: 14 } }
                        }
                    }
                }
            }
        }
    };

    let join_channel = channel.clone();
    rsx! {
        div { class: if entering() { "nv-stage is-entering" } else { "nv-stage" }, "aria-label": "Sprachkanal {channel.name}",
            if !streamers.is_empty() {
                div { class: "nv-stage__feature",
                    for (index, person) in streamers.iter().enumerate() {
                        {tile(index, person, true)}
                    }
                }
                div { class: "nv-stage__strip",
                    for (index, person) in people.iter().enumerate() {
                        {tile(index, person, false)}
                    }
                }
            } else {
                div { class: "nv-stage__grid",
                    for (index, person) in people.iter().enumerate() {
                        {tile(index, person, false)}
                    }
                }
            }
            if !here && can_connect {
                div { class: "nv-stage__intro",
                    button {
                        class: if voice_state.connecting { "nv-btn nv-btn--primary is-loading" } else { "nv-btn nv-btn--primary" },
                        disabled: voice_state.connecting,
                        onclick: move |_| { spawn(actions::join_voice(store, join_channel.clone())); },
                        Icon { name: "volume", size: 16 }
                        if voice_state.channel.is_some() { "Hierher wechseln" } else { "Beitreten" }
                    }
                }
            }
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

/// Schwebende Pill unten mittig: Mikro, Kopfhörer, Bildschirm, Einstellungen, Auflegen.
#[component]
pub fn VoiceDock() -> Element {
    let mut store = use_store();
    let state = store.voice.read().clone();
    let ptt = store.settings.read().push_to_talk;
    let can_stream = store.can(perm::STREAM);
    let Some(channel) = state.channel.clone() else {
        return rsx! {};
    };
    let mic_label = if state.muted {
        "Mikrofon einschalten"
    } else {
        "Stummschalten (Strg+Umschalt+M)"
    };
    rsx! {
        div { class: "nv-dock", role: "toolbar", "aria-label": "Sprachsteuerung",
            div { class: "nv-dock__info",
                span { class: "nv-dock__channel", "{channel.name}" }
                span { class: ping_class(state.ping_ms),
                    if state.connecting { "verbinde …" } else if let Some(ms) = state.ping_ms { "{ms} ms" } else { "– ms" }
                }
            }
            if ptt {
                span { class: if state.ptt_down { "nv-ptt is-on" } else { "nv-ptt" }, "PTT" }
            }
            button {
                class: classes(&["nv-iconbtn", "nv-iconbtn--lg", if state.muted && !ptt { "is-active" } else { "" }]),
                "aria-label": mic_label,
                title: mic_label,
                "aria-pressed": if state.muted { "true" } else { "false" },
                disabled: !state.joined || ptt,
                onclick: move |_| { spawn(actions::toggle_mute(store)); },
                Icon { name: if state.muted { "mic-off" } else { "mic" }, size: 20 }
            }
            button {
                class: classes(&["nv-iconbtn", "nv-iconbtn--lg", if state.deafened { "is-active" } else { "" }]),
                "aria-label": if state.deafened { "Ton einschalten" } else { "Taub schalten" },
                title: if state.deafened { "Ton einschalten" } else { "Taub schalten" },
                "aria-pressed": if state.deafened { "true" } else { "false" },
                disabled: !state.joined,
                onclick: move |_| { spawn(actions::toggle_deafen(store)); },
                Icon { name: if state.deafened { "headphones-off" } else { "headphones" }, size: 20 }
            }
            if can_stream {
                button {
                    class: classes(&["nv-iconbtn", "nv-iconbtn--lg", if state.camera { "is-on" } else { "" }]),
                    "aria-label": if state.camera { "Kamera ausschalten" } else { "Kamera einschalten" },
                    title: if state.camera { "Kamera ausschalten" } else { "Kamera einschalten" },
                    "aria-pressed": if state.camera { "true" } else { "false" },
                    disabled: !state.joined,
                    onclick: move |_| { spawn(actions::toggle_camera(store)); },
                    Icon { name: "video", size: 20 }
                }
            }
            if can_stream {
                button {
                    class: classes(&["nv-iconbtn", "nv-iconbtn--lg", if state.screen { "is-on" } else { "" }]),
                    "aria-label": if state.screen { "Bildschirmfreigabe beenden" } else { "Bildschirm teilen (bis 1080p60)" },
                    title: if state.screen { "Bildschirmfreigabe beenden" } else { "Bildschirm teilen (bis 1080p60)" },
                    "aria-pressed": if state.screen { "true" } else { "false" },
                    disabled: !state.joined,
                    onclick: move |_| { spawn(actions::toggle_screen(store)); },
                    Icon { name: "screen", size: 20 }
                }
            }
            IconButton {
                icon: "settings",
                label: "Audio-Einstellungen",
                large: true,
                onclick: move |_| store.overlay.set(Overlay::Settings(SettingsTab::Voice)),
            }
            span { class: "nv-dock__sep" }
            button {
                class: "nv-iconbtn nv-iconbtn--lg nv-hangup",
                "aria-label": "Auflegen",
                title: "Auflegen",
                onclick: move |_| { spawn(actions::leave_voice(store)); },
                Icon { name: "phone-off", size: 20 }
            }
        }
    }
}
