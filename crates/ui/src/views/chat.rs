//! Nachrichtenliste, Composer, Thread-Seitenleiste und Pins.

use dioxus::prelude::*;

use crate::{
    actions,
    components::{
        Avatar, EmptyState, Icon, IconButton, Skeleton, classes, day_label, minutes, time_label,
    },
    store::{ConfirmAction, Overlay, use_store},
    types::*,
};

const QUICK_EMOJI: [&str; 16] = [
    "👍", "❤️", "😂", "🎉", "🔥", "👀", "😮", "😢", "🙏", "👏", "✅", "❌", "🤔", "😎", "💯", "🚀",
];

fn parse_id(id: &str) -> i64 {
    id.parse().unwrap_or(i64::MAX)
}

/// Text mit hervorgehobenen @Erwähnungen.
fn render_text(content: &str) -> Element {
    let mut parts: Vec<(bool, String)> = Vec::new();
    for word in content.split_inclusive(' ') {
        let trimmed = word.trim_end();
        let is_mention = trimmed.len() > 1
            && trimmed.starts_with('@')
            && trimmed[1..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
        match parts.last_mut() {
            Some((false, text)) if !is_mention => text.push_str(word),
            _ => parts.push((is_mention, word.to_owned())),
        }
    }
    rsx! {
        for (index, (mention, text)) in parts.into_iter().enumerate() {
            if mention {
                span { key: "{index}", class: "nv-msg__mention", "{text}" }
            } else {
                span { key: "{index}", "{text}" }
            }
        }
    }
}

#[component]
pub fn ChatPane(channel_id: String, in_thread: bool) -> Element {
    let store = use_store();
    let channel = store
        .detail
        .read()
        .as_ref()
        .and_then(|detail| detail.channel(&channel_id).cloned());
    let Some(channel) = channel else {
        return rsx! {};
    };
    rsx! {
        MessageList { channel: channel.clone(), in_thread }
        Composer { channel }
    }
}

#[component]
fn MessageList(channel: Channel, in_thread: bool) -> Element {
    let store = use_store();
    let mut stick = use_signal(|| true);
    let mut unseen = use_signal(|| 0usize);
    let list_id = if in_thread {
        format!("log-thread-{}", channel.id)
    } else {
        format!("log-{}", channel.id)
    };
    let channel_id = channel.id.clone();
    let history = store
        .history
        .read()
        .get(&channel_id)
        .cloned()
        .unwrap_or_default();
    let count = history.messages.len();

    // Unten bleiben, solange man unten ist; sonst nur einen Hinweis zeigen.
    let scroll_id = list_id.clone();
    use_effect(use_reactive!(|count| {
        let _ = count;
        let Some(element) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id(&scroll_id))
        else {
            return;
        };
        if *stick.peek() {
            element.set_scroll_top(element.scroll_height());
            unseen.set(0);
        } else {
            *unseen.write() += 1;
        }
    }));
    let reset_id = list_id.clone();
    use_effect(use_reactive!(|channel_id| {
        let _ = &channel_id;
        stick.set(true);
        unseen.set(0);
        if let Some(element) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id(&reset_id))
        {
            element.set_scroll_top(element.scroll_height());
        }
    }));

    let me = store.me_id();
    let me_handle = store.user.read().as_ref().map(|u| format!("@{}", u.handle));
    let detail = store.detail.read().clone();
    let can_manage = detail
        .as_ref()
        .map(|d| d.can(perm::MANAGE_MESSAGES))
        .unwrap_or(false);
    let can_thread = !in_thread
        && channel.kind == KIND_TEXT
        && detail
            .as_ref()
            .map(|d| d.can(perm::CREATE_THREADS))
            .unwrap_or(false);
    let threads: Vec<Channel> = detail
        .as_ref()
        .map(|d| {
            d.channels
                .iter()
                .filter(|c| c.kind == KIND_THREAD)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let marker = history.read_marker.as_deref().map(parse_id);
    let first_new = marker.and_then(|marker| {
        history
            .messages
            .iter()
            .find(|m| parse_id(&m.id) > marker && Some(&m.author.id) != me.as_ref())
            .map(|m| m.id.clone())
    });
    let scroll_handler_id = list_id.clone();
    let jump_id = list_id.clone();
    let older_channel = channel.id.clone();

    if history.loading && history.messages.is_empty() {
        return rsx! { div { class: "nv-log", Skeleton { lines: 8 } } };
    }
    if let Some(error) = history.error.clone()
        && history.messages.is_empty()
    {
        let retry = channel.id.clone();
        return rsx! {
            div { class: "nv-log",
                EmptyState {
                    icon: "alert",
                    title: "Nachrichten konnten nicht geladen werden",
                    text: "{error}",
                    button { class: "nv-btn nv-btn--primary", onclick: move |_| { spawn(actions::reload_history(store, retry.clone())); }, Icon { name: "refresh", size: 16 } "Erneut versuchen" }
                }
            }
        };
    }

    let mut previous: Option<Message> = None;
    let items: Vec<(Message, bool, Option<String>, bool)> = history
        .messages
        .iter()
        .map(|message| {
            let day = day_label(&message.created_at);
            let new_day = previous
                .as_ref()
                .map(|prev| day_label(&prev.created_at) != day)
                .unwrap_or(true);
            let grouped = !new_day
                && previous.as_ref().is_some_and(|prev| {
                    prev.author.id == message.author.id
                        && (minutes(&message.created_at) - minutes(&prev.created_at)).abs() < 5.0
                });
            let is_new = first_new.as_deref() == Some(message.id.as_str());
            previous = Some(message.clone());
            (
                message.clone(),
                grouped && !is_new,
                new_day.then_some(day),
                is_new,
            )
        })
        .collect();

    rsx! {
        div {
            id: "{list_id}",
            class: "nv-log",
            role: "log",
            "aria-live": "polite",
            "aria-label": "Nachrichten in {channel.name}",
            onscroll: move |_| {
                if let Some(element) = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| document.get_element_by_id(&scroll_handler_id))
                {
                    let bottom = element.scroll_height() - element.scroll_top() - element.client_height();
                    let at_bottom = bottom < 80;
                    if at_bottom != *stick.peek() {
                        stick.set(at_bottom);
                    }
                    if at_bottom && *unseen.peek() > 0 {
                        unseen.set(0);
                    }
                }
            },
            if history.complete || history.messages.is_empty() {
                div { class: "nv-log__start",
                    div { class: "nv-log__start-icon", Icon { name: if in_thread { "thread" } else { "hash" }, size: 22 } }
                    h2 { if in_thread { "{channel.name}" } else { "Willkommen in #{channel.name}" } }
                    p {
                        if history.messages.is_empty() {
                            "Hier ist noch nichts. Schreib die erste Nachricht."
                        } else {
                            "Das ist der Anfang dieses Kanals."
                        }
                    }
                }
            } else {
                div { class: "nv-log__top",
                    button {
                        class: "nv-btn nv-btn--ghost nv-btn--sm",
                        disabled: history.loading,
                        onclick: move |_| {
                            stick.set(false);
                            spawn(actions::load_older(store, older_channel.clone()));
                        },
                        if history.loading { "Lädt …" } else { "Ältere Nachrichten laden" }
                    }
                }
            }
            for (message, grouped, day, is_new) in items {
                Fragment { key: "{message.id}",
                if let Some(day) = day {
                    div { class: "nv-divider", role: "separator", "{day}" }
                }
                if is_new {
                    div { class: "nv-divider nv-divider--new", role: "separator", "Neu" }
                }
                MessageRow {
                    message: message.clone(),
                    grouped,
                    mine: me.as_deref() == Some(message.author.id.as_str()),
                    mention: me_handle.as_ref().is_some_and(|handle| message.content.contains(handle.as_str())),
                    can_manage,
                    can_thread,
                    thread: threads.iter().find(|t| t.root_message_id.as_deref() == Some(message.id.as_str())).cloned(),
                }
                }
            }
        }
        if unseen() > 0 && !stick() {
            button {
                class: "nv-jump",
                onclick: move |_| {
                    if let Some(element) = web_sys::window()
                        .and_then(|window| window.document())
                        .and_then(|document| document.get_element_by_id(&jump_id))
                    {
                        element.set_scroll_top(element.scroll_height());
                    }
                    stick.set(true);
                    unseen.set(0);
                },
                Icon { name: "arrow-down", size: 16 }
                "Neue Nachrichten"
            }
        }
    }
}

#[component]
fn MessageRow(
    message: Message,
    grouped: bool,
    mine: bool,
    mention: bool,
    can_manage: bool,
    can_thread: bool,
    thread: Option<Channel>,
) -> Element {
    let mut store = use_store();
    let mut editing = use_signal(|| Option::<String>::None);
    let failed = message.failed;
    let pending = message.pending();
    let class = classes(&[
        "nv-msg",
        if grouped { "is-grouped" } else { "" },
        if mention { "is-mention" } else { "" },
        if pending { "is-pending" } else { "" },
        if failed { "is-failed" } else { "" },
    ]);
    let time = time_label(&message.created_at);
    let day = day_label(&message.created_at);
    let local = message.nonce.is_some();
    let msg = message.clone();
    let react = move |emoji: String, mine: bool| {
        let id = msg.id.clone();
        move |_| {
            if let Some(gateway) = store.gateway.read().clone() {
                gateway.react(&id, &emoji, !mine);
            }
        }
    };
    rsx! {
        article { class: "{class}", "aria-label": "{message.author.display_name}, {time}",
            if grouped {
                span { class: "nv-msg__gutter", title: "{day} {time}", "{time}" }
            } else {
                span { class: "nv-msg__avatar",
                    Avatar { name: message.author.display_name.clone(), size: 40 }
                }
            }
            div {
                if !grouped {
                    div { class: "nv-msg__head",
                        span { class: "nv-msg__author", "{message.author.display_name}" }
                        if message.author.bot {
                            span { class: "nv-badge", "Bot" }
                        }
                        time { class: "nv-msg__time", title: "{day}, {time} Uhr", datetime: "{message.created_at}", "{time}" }
                    }
                }
                if let Some(text) = editing() {
                    div { class: "nv-composer__box",
                        textarea {
                            class: "nv-composer__input",
                            "aria-label": "Nachricht bearbeiten",
                            rows: "{text.lines().count().clamp(1, 8)}",
                            autofocus: true,
                            value: "{text}",
                            oninput: move |event| editing.set(Some(event.value())),
                            onkeydown: {
                                let id = message.id.clone();
                                let channel = message.channel_id.clone();
                                move |event: KeyboardEvent| {
                                    if event.key() == Key::Escape {
                                        event.stop_propagation();
                                        editing.set(None);
                                    } else if event.key() == Key::Enter && !event.modifiers().shift() {
                                        event.prevent_default();
                                        let text = editing.read().clone().unwrap_or_default().trim().to_owned();
                                        if text.is_empty() {
                                            return;
                                        }
                                        if let Some(gateway) = store.gateway.read().clone() {
                                            gateway.edit_message(actions::next_nonce(), &id, text.clone());
                                        }
                                        if let Some(history) = store.history.write().get_mut(&channel)
                                            && let Some(local) = history.messages.iter_mut().find(|m| m.id == id)
                                        {
                                            local.content = text;
                                            local.edited_at = Some(now_string());
                                        }
                                        editing.set(None);
                                    }
                                }
                            },
                        }
                    }
                    span { class: "nv-faint", style: "font-size: 12px", "Enter speichert, Esc bricht ab" }
                } else {
                    div { class: "nv-msg__text", {render_text(&message.content)} }
                }
                if message.edited_at.is_some() || message.pinned || !message.reactions.is_empty() || failed {
                    div { class: "nv-msg__meta",
                        for reaction in message.reactions.clone() {
                            button {
                                key: "{reaction.emoji}",
                                class: classes(&["nv-reaction", if reaction.me { "is-mine" } else { "" }]),
                                "aria-pressed": if reaction.me { "true" } else { "false" },
                                "aria-label": "{reaction.emoji} {reaction.count}",
                                onclick: react(reaction.emoji.clone(), reaction.me),
                                "{reaction.emoji}"
                                span { "{reaction.count}" }
                            }
                        }
                        if message.pinned {
                            span { class: "nv-msg__pin", Icon { name: "pin", size: 12 } "angepinnt" }
                        }
                        if message.edited_at.is_some() {
                            span { class: "nv-msg__edited", "(bearbeitet)" }
                        }
                        if failed {
                            span { class: "nv-msg__retry",
                                Icon { name: "alert", size: 14 }
                                "Nicht gesendet."
                                button {
                                    onclick: {
                                        let channel = message.channel_id.clone();
                                        let id = message.id.clone();
                                        move |_| actions::retry_message(store, channel.clone(), id.clone())
                                    },
                                    "Erneut senden"
                                }
                            }
                        }
                    }
                }
                if let Some(thread) = thread {
                    button {
                        class: "nv-msg__thread",
                        onclick: move |_| {
                            let id = thread.id.clone();
                            spawn(actions::open_thread(store, id));
                        },
                        Icon { name: "thread", size: 14 }
                        "{thread.name}"
                        if thread.archived {
                            span { class: "nv-badge", "archiviert" }
                        }
                    }
                }
            }
            if !local {
                div { class: "nv-msg__tools", role: "toolbar", "aria-label": "Aktionen für diese Nachricht",
                    IconButton { icon: "smile", label: "Mit 👍 reagieren", onclick: react("👍".into(), message.reactions.iter().any(|r| r.emoji == "👍" && r.me)) }
                    if can_thread {
                        IconButton {
                            icon: "thread",
                            label: "Thread starten",
                            onclick: {
                                let channel_id = message.channel_id.clone();
                                let message_id = message.id.clone();
                                move |_| store.overlay.set(Overlay::NewThread { channel_id: channel_id.clone(), message_id: message_id.clone() })
                            },
                        }
                    }
                    if can_manage {
                        IconButton {
                            icon: "pin",
                            label: if message.pinned { "Nicht mehr anpinnen" } else { "Anpinnen" },
                            active: message.pinned,
                            onclick: {
                                let channel = message.channel_id.clone();
                                let id = message.id.clone();
                                let pinned = message.pinned;
                                move |_| {
                                    let channel = channel.clone();
                                    let id = id.clone();
                                    spawn(async move {
                                        if let Err(err) = crate::api::set_pin(&channel, &id, !pinned).await {
                                            store.error(err);
                                        }
                                    });
                                }
                            },
                        }
                    }
                    if mine {
                        IconButton {
                            icon: "edit",
                            label: "Bearbeiten",
                            onclick: {
                                let content = message.content.clone();
                                move |_| editing.set(Some(content.clone()))
                            },
                        }
                    }
                    if mine || can_manage {
                        IconButton {
                            icon: "trash",
                            label: "Löschen",
                            danger: true,
                            onclick: {
                                let channel_id = message.channel_id.clone();
                                let message_id = message.id.clone();
                                move |_| store.overlay.set(Overlay::Confirm(ConfirmAction::DeleteMessage {
                                    channel_id: channel_id.clone(),
                                    message_id: message_id.clone(),
                                }))
                            },
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Composer(channel: Channel) -> Element {
    let store = use_store();
    let mut draft = use_signal(String::new);
    let mut emoji_open = use_signal(|| false);
    let mut last_typing = use_signal(|| 0f64);
    let detail = store.detail.read().clone();
    let can_send = detail
        .as_ref()
        .map(|d| d.can(perm::SEND_MESSAGES))
        .unwrap_or(false);
    let me = store.me_id();
    let timed_out = detail
        .as_ref()
        .zip(me.as_ref())
        .and_then(|(d, me)| d.member(me))
        .and_then(|m| m.timeout_until.clone());
    let typing: Vec<String> = store
        .typing
        .read()
        .iter()
        .filter(|t| t.channel_id == channel.id)
        .map(|t| t.name.clone())
        .collect();
    let channel_id = channel.id.clone();

    if channel.archived {
        return rsx! {
            div { class: "nv-composer",
                div { class: "nv-composer__locked", Icon { name: "archive", size: 16 } "Dieser Thread ist archiviert." }
            }
        };
    }
    if let Some(until) = timed_out {
        let until = time_label(&until);
        return rsx! {
            div { class: "nv-composer",
                div { class: "nv-composer__locked", Icon { name: "clock", size: 16 } "Du hast einen Timeout bis {until} Uhr." }
            }
        };
    }
    if !can_send {
        return rsx! {
            div { class: "nv-composer",
                div { class: "nv-composer__locked", "In diesem Kanal kannst du nicht schreiben." }
            }
        };
    }

    let send = {
        let channel_id = channel_id.clone();
        move || {
            let text = draft.read().clone();
            if text.trim().is_empty() {
                return;
            }
            actions::send_message(store, channel_id.clone(), text);
            draft.set(String::new());
        }
    };
    let mut send_key = send.clone();
    let mut send_click = send;
    let rows = draft.read().lines().count().clamp(1, 12);
    let placeholder = if channel.kind == KIND_THREAD {
        format!("Antworten in „{}“", channel.name)
    } else {
        format!("Nachricht an #{}", channel.name)
    };
    let typing_text = match typing.len() {
        0 => None,
        1 => Some(format!("{} schreibt …", typing[0])),
        2 => Some(format!("{} und {} schreiben …", typing[0], typing[1])),
        n => Some(format!("{n} Personen schreiben …")),
    };
    let typing_channel = channel_id.clone();
    rsx! {
        div { class: "nv-composer",
            if emoji_open() {
                div { class: "nv-emoji", role: "dialog", "aria-label": "Emoji einfügen",
                    for emoji in QUICK_EMOJI {
                        button {
                            key: "{emoji}",
                            "aria-label": "{emoji} einfügen",
                            onclick: move |_| {
                                draft.write().push_str(emoji);
                                emoji_open.set(false);
                            },
                            "{emoji}"
                        }
                    }
                }
            }
            div { class: "nv-composer__box",
                label { class: "sr-only", r#for: "composer-{channel_id}", "{placeholder}" }
                textarea {
                    id: "composer-{channel_id}",
                    class: "nv-composer__input",
                    rows: "{rows}",
                    placeholder: "{placeholder}",
                    value: "{draft}",
                    oninput: move |event| {
                        draft.set(event.value());
                        let now = js_sys::Date::now();
                        if now - *last_typing.peek() > 4000.0 {
                            last_typing.set(now);
                            if let Some(gateway) = store.gateway.read().clone() {
                                gateway.typing(&typing_channel);
                            }
                        }
                    },
                    onkeydown: move |event| {
                        if event.key() == Key::Enter && !event.modifiers().shift() {
                            event.prevent_default();
                            send_key();
                        }
                    },
                }
                IconButton {
                    icon: "smile",
                    label: "Emoji",
                    active: emoji_open(),
                    pressed: Some(emoji_open()),
                    onclick: move |_| emoji_open.toggle(),
                }
            }
            div { class: "nv-composer__foot",
                span {
                    if let Some(text) = typing_text {
                        span { class: "nv-typing", "aria-live": "polite",
                            span { class: "nv-typing__dots", span {} span {} span {} }
                            "{text}"
                        }
                    }
                }
                span { class: "nv-faint",
                    if channel.slowmode_secs > 0 { "Slowmode {channel.slowmode_secs} s. " }
                    "Enter sendet, Umschalt+Enter macht eine neue Zeile."
                }
            }
            button { class: "sr-only", onclick: move |_| send_click(), "Senden" }
        }
    }
}

// ------------------------------------------------------------ Threads

#[component]
pub fn ThreadPanel() -> Element {
    let mut store = use_store();
    let thread_id = store.thread_id.read().clone();
    let thread = thread_id.as_ref().and_then(|id| {
        store
            .detail
            .read()
            .as_ref()
            .and_then(|d| d.channel(id).cloned())
    });
    let Some(thread) = thread else {
        return rsx! {
            aside { class: "nv-thread",
                div { class: "nv-thread__head",
                    h2 { "Thread" }
                    IconButton { icon: "close", label: "Thread schließen", onclick: move |_| store.thread_id.set(None) }
                }
                Skeleton { lines: 4 }
            }
        };
    };
    let me = store.me_id();
    let can_archive =
        thread.owner_id.as_deref() == me.as_deref() || store.can(perm::MANAGE_THREADS);
    let archived = thread.archived;
    let id = thread.id.clone();
    rsx! {
        aside { class: "nv-thread", "aria-label": "Thread {thread.name}",
            div { class: "nv-thread__head",
                Icon { name: "thread", size: 18 }
                h2 { "{thread.name}" }
                if can_archive {
                    IconButton {
                        icon: "archive",
                        label: if archived { "Thread wieder öffnen" } else { "Thread archivieren" },
                        active: archived,
                        onclick: move |_| {
                            let id = id.clone();
                            spawn(async move {
                                match crate::api::update_channel(&id, serde_json::json!({ "archived": !archived })).await {
                                    Ok(_) => actions::refresh_guild(store).await,
                                    Err(err) => store.error(err),
                                }
                            });
                        },
                    }
                }
                IconButton { icon: "close", label: "Thread schließen", onclick: move |_| store.thread_id.set(None) }
            }
            ChatPane { channel_id: thread.id.clone(), in_thread: true }
        }
    }
}

// ---------------------------------------------------------------- Pins

#[component]
pub fn PinsPopover() -> Element {
    let mut store = use_store();
    let pins = store.pins.read().clone().unwrap_or_default();
    rsx! {
        section { class: "nv-popover", role: "dialog", "aria-label": "Angepinnte Nachrichten",
            div { class: "nv-popover__head",
                h3 { "Angepinnt" }
                IconButton { icon: "close", label: "Schließen", onclick: move |_| store.pins.set(None) }
            }
            if pins.is_empty() {
                p { class: "nv-muted", style: "padding: 8px 4px",
                    "Noch nichts angepinnt. Moderatoren pinnen wichtige Nachrichten über das Stecknadel-Symbol an der Nachricht."
                }
            }
            for pin in pins {
                div { key: "{pin.id}", class: "nv-pin",
                    div { class: "nv-pin__head",
                        strong { "{pin.author.display_name}" }
                        span { class: "nv-faint", "{day_label(&pin.created_at)}, {time_label(&pin.created_at)}" }
                    }
                    div { class: "nv-msg__text", "{pin.content}" }
                }
            }
        }
    }
}
