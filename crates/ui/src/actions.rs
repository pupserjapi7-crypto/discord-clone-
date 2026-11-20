//! Abläufe, die Views auslösen: laden, senden, moderieren, Voice.
//! Alles hier schreibt nur in den `Store`; Views lesen daraus.

use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::{core::spawn_forever, prelude::*};

use crate::{
    api,
    gateway::Gateway,
    protocol::VoiceState,
    store::{ConnStatus, Overlay, PendingUpload, Store, UploadState},
    types::*,
    voice,
};

static NONCE: AtomicU64 = AtomicU64::new(1);

pub fn next_nonce() -> u64 {
    let base = (js_sys::Date::now() as u64) << 12;
    base | (NONCE.fetch_add(1, Ordering::Relaxed) & 0xfff)
}

// ------------------------------------------------------------ Anmeldung

pub async fn boot(mut store: Store) {
    match api::refresh().await {
        Ok(user) => finish_login(store, user).await,
        Err(_) => store.booting.set(false),
    }
}

pub async fn finish_login(mut store: Store, user: User) {
    let Some(token) = api::token() else {
        store.booting.set(false);
        return;
    };
    if let Some(previous) = store.gateway.read().clone() {
        previous.close();
    }
    store.gateway.set(Some(Gateway::start(token, store)));
    store.user.set(Some(user));
    store.booting.set(false);
    load_guilds(store).await;
}

pub async fn logout(mut store: Store) {
    voice::shutdown();
    if let Some(gateway) = store.gateway.read().clone() {
        gateway.close();
    }
    let _ = api::logout().await;
    store.gateway.set(None);
    store.user.set(None);
    store.guilds.set(Vec::new());
    store.detail.set(None);
    store.channel_id.set(None);
    store.thread_id.set(None);
    store.history.write().clear();
    store.voice.set(Default::default());
    store.voice_rooms.write().clear();
    store.status.set(ConnStatus::Offline);
    store.overlay.set(Overlay::None);
}

// -------------------------------------------------------- Server/Kanäle

/// Nur die Liste (z. B. nach `guild_update` einer anderen Guild).
pub async fn load_guild_list(mut store: Store) {
    if let Ok(list) = api::guilds().await {
        store.guilds.set(list);
    }
}

/// Liste laden und die zuletzt offene (oder erste) Guild öffnen.
pub async fn load_guilds(mut store: Store) {
    match api::guilds().await {
        Ok(list) => {
            let current = store
                .detail
                .read()
                .as_ref()
                .map(|detail| detail.guild.id.clone());
            let target = current
                .filter(|id| list.iter().any(|guild| &guild.id == id))
                .or_else(|| list.first().map(|guild| guild.id.clone()));
            store.guilds.set(list);
            store.guilds_loaded.set(true);
            match target {
                Some(id) => open_guild(store, id).await,
                None => {
                    store.detail.set(None);
                    store.channel_id.set(None);
                }
            }
        }
        Err(err) => {
            store.guilds_loaded.set(true);
            store.error(err);
        }
    }
}

pub async fn open_guild(mut store: Store, guild_id: String) {
    store.guild_loading.set(true);
    store.thread_id.set(None);
    store.pins.set(None);
    match api::guild(&guild_id).await {
        Ok(detail) => {
            let keep = store
                .channel_id
                .read()
                .clone()
                .filter(|id| detail.channel(id).is_some());
            let target = keep
                .or_else(|| {
                    detail
                        .channels
                        .iter()
                        .find(|channel| channel.kind == KIND_TEXT)
                        .map(|channel| channel.id.clone())
                })
                .or_else(|| detail.channels.first().map(|channel| channel.id.clone()));
            let voice_ids: Vec<String> = detail
                .channels
                .iter()
                .filter(|channel| channel.kind == KIND_VOICE)
                .map(|channel| channel.id.clone())
                .collect();
            store.detail.set(Some(detail));
            store.guild_loading.set(false);
            store.voice_rooms.write().clear();
            for id in voice_ids {
                spawn(load_voice_room(store, id));
            }
            if let Some(id) = target {
                open_channel(store, id).await;
            } else {
                store.channel_id.set(None);
            }
        }
        Err(err) => {
            store.guild_loading.set(false);
            store.error(err);
        }
    }
}

/// Details neu laden, ohne Kanal oder Verlauf zu verlieren.
pub async fn refresh_guild(mut store: Store) {
    let Some(id) = store.detail.read().as_ref().map(|d| d.guild.id.clone()) else {
        return;
    };
    if let Ok(detail) = api::guild(&id).await {
        let current = store.channel_id.read().clone();
        if let Some(current) = &current
            && detail.channel(current).is_none()
        {
            store.channel_id.set(None);
        }
        store.detail.set(Some(detail));
    }
}

pub async fn open_channel(mut store: Store, channel_id: String) {
    let channel = store
        .detail
        .read()
        .as_ref()
        .and_then(|detail| detail.channel(&channel_id).cloned());
    store.channel_id.set(Some(channel_id.clone()));
    store.nav_open.set(false);
    store.pins.set(None);
    if store.thread_id.read().as_deref() == Some(channel_id.as_str()) {
        store.thread_id.set(None);
    }
    if let Some(gateway) = store.gateway.read().clone() {
        gateway.subscribe(&channel_id);
    }
    // Sprachkanäle und Foren haben keinen Nachrichtenverlauf.
    let has_history = channel.is_none_or(|c| c.kind != KIND_VOICE && c.kind != KIND_FORUM);
    if has_history {
        load_history(store, channel_id).await;
    }
}

/// Thread in der Seitenleiste öffnen (Elternkanal bleibt sichtbar).
pub async fn open_thread(mut store: Store, thread_id: String) {
    store.thread_id.set(Some(thread_id.clone()));
    if let Some(gateway) = store.gateway.read().clone() {
        gateway.subscribe(&thread_id);
    }
    load_history(store, thread_id).await;
}

fn read_marker(store: Store, channel_id: &str) -> Option<String> {
    let detail = store.detail.read();
    let unread = detail
        .as_ref()?
        .unread
        .iter()
        .find(|u| u.channel_id == channel_id)?;
    unread.is_unread().then(|| unread.last_read_id.clone())
}

async fn load_history(mut store: Store, channel_id: String) {
    let cached = store
        .history
        .read()
        .get(&channel_id)
        .map(|h| !h.messages.is_empty())
        .unwrap_or(false);
    let marker = read_marker(store, &channel_id);
    {
        let mut histories = store.history.write();
        let history = histories.entry(channel_id.clone()).or_default();
        history.loading = !cached;
        history.error = None;
        if marker.is_some() {
            history.read_marker = marker;
        }
    }
    match api::messages(&channel_id, None).await {
        Ok(list) => {
            let complete = list.len() < 50;
            let last = list.last().map(|m| m.id.clone());
            let mut histories = store.history.write();
            let history = histories.entry(channel_id.clone()).or_default();
            let pending: Vec<Message> = history
                .messages
                .iter()
                .filter(|m| m.nonce.is_some())
                .cloned()
                .collect();
            history.messages = list;
            history.messages.extend(pending);
            history.loading = false;
            history.complete = complete;
            drop(histories);
            if let Some(last) = last {
                mark_read(channel_id.clone(), last.clone()).await;
                if let Some(detail) = store.detail.write().as_mut()
                    && let Some(unread) = detail
                        .unread
                        .iter_mut()
                        .find(|u| u.channel_id == channel_id)
                {
                    unread.last_read_id = last;
                }
            }
        }
        Err(err) => {
            let mut histories = store.history.write();
            let history = histories.entry(channel_id).or_default();
            history.loading = false;
            history.error = Some(err);
        }
    }
}

pub async fn reload_history(store: Store, channel_id: String) {
    load_history(store, channel_id).await;
}

/// Ältere Nachrichten nachladen (Cursor `before`).
pub async fn load_older(mut store: Store, channel_id: String) {
    let oldest = store.history.read().get(&channel_id).and_then(|h| {
        h.messages
            .iter()
            .find(|m| m.nonce.is_none())
            .map(|m| m.id.clone())
    });
    let Some(oldest) = oldest else { return };
    if let Some(history) = store.history.write().get_mut(&channel_id) {
        history.loading = true;
    }
    match api::messages(&channel_id, Some(&oldest)).await {
        Ok(mut older) => {
            if let Some(history) = store.history.write().get_mut(&channel_id) {
                history.complete = older.len() < 50;
                older.append(&mut history.messages);
                history.messages = older;
                history.loading = false;
            }
        }
        Err(err) => {
            if let Some(history) = store.history.write().get_mut(&channel_id) {
                history.loading = false;
            }
            store.error(err);
        }
    }
}

pub async fn mark_read(channel_id: String, message_id: String) {
    let _ = api::ack(&channel_id, &message_id).await;
}

pub fn send_message(
    mut store: Store,
    channel_id: String,
    content: String,
    attachments: Vec<Attachment>,
) {
    let content = content.trim().to_owned();
    if content.is_empty() && attachments.is_empty() {
        return;
    }
    let Some(user) = store.user.read().clone() else {
        return;
    };
    let gateway = store.gateway.read().clone();
    let online = *store.status.read() == ConnStatus::Online;
    let Some(gateway) = gateway.filter(|_| online) else {
        store.error("Keine Verbindung – Nachricht wurde nicht gesendet.");
        return;
    };
    let nonce = next_nonce();
    let attachment_ids = attachments
        .iter()
        .filter_map(|attachment| attachment.id.parse().ok())
        .collect();
    store
        .history
        .write()
        .entry(channel_id.clone())
        .or_default()
        .messages
        .push(Message {
            id: format!("local-{nonce}"),
            channel_id: channel_id.clone(),
            author: user,
            content: content.clone(),
            created_at: now_string(),
            attachments,
            nonce: Some(nonce),
            ..Default::default()
        });
    gateway.send_message(nonce, &channel_id, content, attachment_ids);
}

/// Fehlgeschlagene Nachricht erneut senden.
pub fn retry_message(mut store: Store, channel_id: String, local_id: String) {
    let message = {
        let mut histories = store.history.write();
        let Some(history) = histories.get_mut(&channel_id) else {
            return;
        };
        let Some(pos) = history.messages.iter().position(|m| m.id == local_id) else {
            return;
        };
        history.messages.remove(pos)
    };
    send_message(store, channel_id, message.content, message.attachments);
}

/// Höchstens so viele Anhänge pro Nachricht (wie der Server).
pub const MAX_ATTACHMENTS: usize = 10;

/// Dateien hochladen und im Composer des Kanals anzeigen. Läuft unabhängig
/// von der Komponente weiter (Kanalwechsel bricht den Upload nicht ab).
pub fn attach_files(mut store: Store, channel_id: String, files: Vec<web_sys::File>) {
    for file in files {
        let count = store.uploads.read().get(&channel_id).map_or(0, Vec::len);
        if count >= MAX_ATTACHMENTS {
            store.error("Höchstens 10 Anhänge pro Nachricht.");
            break;
        }
        let key = next_nonce();
        let name = file.name();
        store
            .uploads
            .write()
            .entry(channel_id.clone())
            .or_default()
            .push(PendingUpload {
                key,
                name: name.clone(),
                size: file.size() as i64,
                progress: 0.0,
                state: UploadState::Uploading,
            });
        let channel = channel_id.clone();
        spawn_forever(async move {
            let progress_channel = channel.clone();
            let result = api::upload(&channel, &file, move |value| {
                let mut uploads = store.uploads.write();
                if let Some(item) = uploads
                    .get_mut(&progress_channel)
                    .and_then(|list| list.iter_mut().find(|item| item.key == key))
                    && (value - item.progress >= 0.02 || value >= 1.0)
                {
                    item.progress = value;
                }
            })
            .await;
            let mut uploads = store.uploads.write();
            let item = uploads
                .get_mut(&channel)
                .and_then(|list| list.iter_mut().find(|item| item.key == key));
            match (item, result) {
                (Some(item), Ok(attachment)) => {
                    item.progress = 1.0;
                    item.state = UploadState::Done(attachment);
                }
                (Some(item), Err(err)) => {
                    item.state = UploadState::Failed(err.clone());
                    drop(uploads);
                    store.error(format!("{name}: {err}"));
                }
                // Inzwischen entfernt: Upload gleich wieder verwerfen.
                (None, Ok(attachment)) => {
                    drop(uploads);
                    let _ = api::discard_attachment(&attachment.id).await;
                }
                (None, Err(_)) => {}
            }
        });
    }
}

/// Anhang aus dem Composer entfernen; fertige Uploads verwirft der Server.
pub fn remove_upload(mut store: Store, channel_id: &str, key: u64) {
    let removed = {
        let mut uploads = store.uploads.write();
        let Some(list) = uploads.get_mut(channel_id) else {
            return;
        };
        let Some(pos) = list.iter().position(|item| item.key == key) else {
            return;
        };
        list.remove(pos)
    };
    if let UploadState::Done(attachment) = removed.state {
        spawn_forever(async move {
            let _ = api::discard_attachment(&attachment.id).await;
        });
    }
}

pub async fn load_pins(mut store: Store) {
    let Some(channel_id) = store.channel_id.read().clone() else {
        return;
    };
    match api::pins(&channel_id).await {
        Ok(pins) => store.pins.set(Some(pins)),
        Err(err) => store.error(err),
    }
}

// --------------------------------------------------------------- Voice

pub async fn load_voice_room(mut store: Store, channel_id: String) {
    if let Ok(list) = api::voice_participants(&channel_id).await {
        if list.is_empty() {
            store.voice_rooms.write().remove(&channel_id);
        } else {
            store.voice_rooms.write().insert(channel_id, list);
        }
    }
}

/// `voice_state_update` aus dem Gateway: Teilnehmerliste, Status-Icons und
/// Aufräumen der Medien-Elemente von Personen, die gegangen sind.
pub fn apply_voice_state(mut store: Store, state: VoiceState) {
    let in_guild = store
        .detail
        .read()
        .as_ref()
        .map(|detail| detail.guild.id == state.guild_id)
        .unwrap_or(false);
    if !in_guild {
        return;
    }
    let name = store
        .detail
        .read()
        .as_ref()
        .and_then(|detail| detail.member(&state.user_id).cloned());
    {
        let mut rooms = store.voice_rooms.write();
        // In anderen Kanälen entfernen; im eigenen an Ort und Stelle
        // aktualisieren, damit die Kacheln nicht springen.
        for (channel_id, list) in rooms.iter_mut() {
            if !state.connected || *channel_id != state.channel_id {
                list.retain(|p| p.user_id != state.user_id);
            }
        }
        if state.connected {
            let list = rooms.entry(state.channel_id.clone()).or_default();
            match list.iter_mut().find(|p| p.user_id == state.user_id) {
                Some(existing) => {
                    existing.muted = state.muted;
                    existing.screen = state.screen;
                    existing.deafened = state.deafened;
                    existing.camera = state.camera;
                }
                None => {
                    let member = name.unwrap_or_default();
                    list.push(VoiceParticipant {
                        user_id: state.user_id.clone(),
                        handle: member.handle,
                        display_name: member.display_name,
                        muted: state.muted,
                        screen: state.screen,
                        deafened: state.deafened,
                        camera: state.camera,
                    });
                }
            }
        }
        rooms.retain(|_, list| !list.is_empty());
    }
    if !state.connected || !state.screen {
        voice::drop_remote_video(&state.user_id);
        store
            .voice
            .write()
            .video_users
            .retain(|id| id != &state.user_id);
    }
    if !state.connected || !state.camera {
        voice::drop_remote_camera(&state.user_id);
        store
            .voice
            .write()
            .camera_users
            .retain(|id| id != &state.user_id);
    }
    if !state.connected {
        voice::drop_remote_user(&state.user_id);
        store
            .voice
            .write()
            .speaking
            .retain(|id| id != &state.user_id);
        // Selbst entfernt (Kick, Timeout): lokale Verbindung beenden.
        if store.me_id().as_deref() == Some(state.user_id.as_str()) && store.voice.read().joined {
            voice::shutdown();
            store.voice.set(Default::default());
        }
    }
}

pub async fn join_voice(mut store: Store, channel: Channel) {
    if store.voice.read().connecting {
        return;
    }
    {
        let mut voice = store.voice.write();
        voice.channel = Some(channel.clone());
        voice.connecting = true;
        voice.joined = false;
    }
    let settings = store.settings.read().clone();
    match voice::join(store, &channel.id, &settings).await {
        Ok(()) => {
            {
                let mut voice = store.voice.write();
                voice.connecting = false;
                voice.joined = true;
                voice.muted = settings.push_to_talk;
                voice.deafened = false;
                voice.screen = false;
                voice.camera = false;
            }
            if settings.push_to_talk {
                voice::set_muted(true);
            }
            report_voice_state(store).await;
            load_voice_room(store, channel.id).await;
        }
        Err(err) => {
            store.voice.set(Default::default());
            store.error(err);
        }
    }
}

/// ICE ist ausgefallen (Netzwechsel, WLAN weg): still neu beitreten.
pub async fn reconnect_voice(mut store: Store) {
    let Some(channel) = store.voice.read().channel.clone() else {
        return;
    };
    voice::shutdown();
    store.voice.set(Default::default());
    store.notice("Sprachverbindung unterbrochen – verbinde neu …");
    join_voice(store, channel).await;
}

pub async fn leave_voice(mut store: Store) {
    let channel = store.voice.read().channel.clone();
    voice::shutdown();
    store.voice.set(Default::default());
    if let Some(channel) = channel {
        let _ = api::voice_leave(&channel.id).await;
    }
}

pub async fn report_voice_state(store: Store) {
    let (channel, muted, deafened, screen, camera) = {
        let voice = store.voice.read();
        (
            voice.channel.clone(),
            voice.muted,
            voice.deafened,
            voice.screen,
            voice.camera,
        )
    };
    if let Some(channel) = channel {
        let _ = api::voice_state(&channel.id, muted, deafened, screen, camera).await;
    }
}

pub async fn toggle_mute(mut store: Store) {
    let muted = !store.voice.read().muted;
    store.voice.write().muted = muted;
    voice::set_muted(muted);
    report_voice_state(store).await;
}

pub async fn toggle_deafen(mut store: Store) {
    let deafened = !store.voice.read().deafened;
    {
        let mut voice = store.voice.write();
        voice.deafened = deafened;
        // Wer nichts hört, spricht auch nicht – wie man es erwartet.
        if deafened {
            voice.muted = true;
        }
    }
    voice::set_deafened(deafened);
    if deafened {
        voice::set_muted(true);
    }
    report_voice_state(store).await;
}

pub async fn toggle_screen(mut store: Store) {
    let sharing = store.voice.read().screen;
    if sharing {
        voice::stop_screen();
        store.voice.write().screen = false;
    } else {
        let settings = store.settings.read().clone();
        match voice::start_screen(store, &settings).await {
            Ok(()) => store.voice.write().screen = true,
            Err(err) => {
                store.error(err);
                return;
            }
        }
    }
    report_voice_state(store).await;
}

pub async fn toggle_camera(mut store: Store) {
    let on = store.voice.read().camera;
    if on {
        voice::stop_camera();
        store.voice.write().camera = false;
    } else {
        let settings = store.settings.read().clone();
        match voice::start_camera(store, &settings).await {
            Ok(()) => store.voice.write().camera = true,
            Err(err) => {
                store.error(err);
                return;
            }
        }
    }
    report_voice_state(store).await;
}

// ---------------------------------------------------------- Moderation

pub async fn moderate(
    store: Store,
    action: impl std::future::Future<Output = Result<(), String>>,
    success: &str,
) {
    match action.await {
        Ok(()) => {
            store.notice(success);
            store.close_overlay();
            refresh_guild(store).await;
        }
        Err(err) => store.error(err),
    }
}

pub fn guild_id(store: Store) -> Option<String> {
    store
        .detail
        .read()
        .as_ref()
        .map(|detail| detail.guild.id.clone())
}
