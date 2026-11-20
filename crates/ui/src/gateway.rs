//! WebSocket-Gateway der Web-UI.

use dioxus::prelude::*;
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::stream::SplitSink;
use futures_util::{FutureExt, SinkExt, StreamExt, pin_mut, select};
use gloo_net::websocket::Message as WsMessage;
use gloo_net::websocket::futures::WebSocket;
use gloo_timers::future::TimeoutFuture;
use serde::de::DeserializeOwned;

use crate::actions;
use crate::protocol::{self, AckPayload, ClientOp, Dispatched, ErrorPayload, HelloOk};
use crate::store::{ConnStatus, Store, Typing};
use crate::types::Message;

#[derive(Clone)]
pub struct Gateway {
    tx: UnboundedSender<Command>,
}

impl PartialEq for Gateway {
    fn eq(&self, other: &Self) -> bool {
        self.tx.same_receiver(&other.tx)
    }
}

enum Command {
    Subscribe(i64),
    Send {
        nonce: u64,
        channel_id: i64,
        content: String,
        attachment_ids: Vec<i64>,
    },
    Edit {
        nonce: u64,
        message_id: i64,
        content: String,
    },
    Reaction {
        message_id: i64,
        emoji: String,
        add: bool,
    },
    Typing(i64),
    Close,
}

impl Gateway {
    pub fn start(token: String, store: Store) -> Self {
        let (tx, rx) = unbounded();
        spawn(async move {
            run(token, store, rx).await;
        });
        Self { tx }
    }

    fn command(&self, command: Command) {
        let _ = self.tx.unbounded_send(command);
    }

    pub fn subscribe(&self, channel_id: &str) {
        if let Ok(id) = channel_id.parse() {
            self.command(Command::Subscribe(id));
        }
    }

    pub fn send_message(
        &self,
        nonce: u64,
        channel_id: &str,
        content: String,
        attachment_ids: Vec<i64>,
    ) {
        if let Ok(channel_id) = channel_id.parse() {
            self.command(Command::Send {
                nonce,
                channel_id,
                content,
                attachment_ids,
            });
        }
    }

    pub fn edit_message(&self, nonce: u64, message_id: &str, content: String) {
        if let Ok(message_id) = message_id.parse() {
            self.command(Command::Edit {
                nonce,
                message_id,
                content,
            });
        }
    }

    pub fn react(&self, message_id: &str, emoji: &str, add: bool) {
        if let Ok(message_id) = message_id.parse() {
            self.command(Command::Reaction {
                message_id,
                emoji: emoji.to_owned(),
                add,
            });
        }
    }

    pub fn typing(&self, channel_id: &str) {
        if let Ok(id) = channel_id.parse() {
            self.command(Command::Typing(id));
        }
    }

    pub fn close(&self) {
        self.command(Command::Close);
    }
}

fn decode_payload<T: DeserializeOwned>(body: &[u8]) -> Option<T> {
    #[derive(serde::Deserialize)]
    struct Envelope<T> {
        d: T,
    }
    rmp_serde::from_slice::<Envelope<T>>(body)
        .ok()
        .map(|envelope| envelope.d)
}

fn websocket_url() -> String {
    let Some(window) = web_sys::window() else {
        return "ws://localhost/gateway".into();
    };
    let location = window.location();
    let protocol = location.protocol().unwrap_or_else(|_| "http:".into());
    let scheme = if protocol == "https:" { "wss" } else { "ws" };
    let host = location.host().unwrap_or_else(|_| "localhost".into());
    format!("{scheme}://{host}/gateway")
}

async fn send_op(sink: &mut SplitSink<WebSocket, WsMessage>, value: &ClientOp) -> bool {
    let op = if matches!(value, ClientOp::Hello { .. }) {
        protocol::OP_HELLO
    } else {
        protocol::OP_REQUEST
    };
    let Ok(frame) = protocol::encode(op, 0, value) else {
        return false;
    };
    sink.send(WsMessage::Bytes(frame)).await.is_ok()
}

/// Exponentiell bis 15 s plus Zufall, damit nach einem Server-Neustart nicht
/// alle Clients gleichzeitig anklopfen.
fn backoff_ms(attempt: u32) -> u32 {
    let base = 500u32.saturating_mul(1 << attempt.min(5)).min(15_000);
    let jitter = (js_sys::Math::random() * f64::from(base) / 2.0) as u32;
    base / 2 + jitter
}

async fn run(token: String, mut store: Store, mut rx: UnboundedReceiver<Command>) {
    let mut token = token;
    let mut session: Option<String> = None;
    let mut last_seq = 0u32;
    let mut channel: Option<i64> = None;
    let mut closed = false;
    let mut attempt = 0u32;

    while !closed {
        store.status.set(ConnStatus::Connecting);
        if attempt > 0 {
            // Das Access-Token läuft nach 15 min ab; vor dem Neuverbinden erneuern.
            if let Ok(user) = crate::api::refresh().await {
                store.user.set(Some(user));
            }
            if let Some(fresh) = crate::api::token() {
                token = fresh;
            }
        }
        let Ok(ws) = WebSocket::open(&websocket_url()) else {
            store.status.set(ConnStatus::Offline);
            TimeoutFuture::new(backoff_ms(attempt)).await;
            attempt += 1;
            continue;
        };
        let (mut sink, mut stream) = ws.split();
        let hello = ClientOp::Hello {
            token: token.clone(),
            resume: session.as_ref().map(|id| protocol::ResumeRequest {
                session_id: id.clone(),
                last_seq,
            }),
        };
        if !send_op(&mut sink, &hello).await {
            store.status.set(ConnStatus::Offline);
            TimeoutFuture::new(backoff_ms(attempt)).await;
            attempt += 1;
            continue;
        }
        let mut online = false;
        let mut heartbeat_ms = 25_000u32;
        let mut connected = true;
        while connected {
            let tick = TimeoutFuture::new(heartbeat_ms).fuse();
            pin_mut!(tick);
            select! {
                incoming = stream.next().fuse() => match incoming {
                    Some(Ok(WsMessage::Bytes(bytes))) => {
                        let Ok(frame) = protocol::decode(&bytes) else { continue };
                        match frame.op {
                            protocol::OP_HELLO_OK | protocol::OP_RESUMED => {
                                if let Some(hello) = decode_payload::<HelloOk>(&frame.body) {
                                    session = Some(hello.session_id);
                                    last_seq = hello.seq;
                                    heartbeat_ms = (hello.heartbeat_secs.max(5) * 1000).min(30_000);
                                }
                                online = true;
                                attempt = 0;
                                store.status.set(ConnStatus::Online);
                                if let Some(id) = channel {
                                    send_op(&mut sink, &ClientOp::Subscribe { channel_id: id }).await;
                                }
                            }
                            protocol::OP_INVALID_SESSION => {
                                session = None;
                                last_seq = 0;
                                connected = false;
                            }
                            protocol::OP_DISPATCH => {
                                last_seq = frame.seq;
                                if let Some(event) = protocol::parse_dispatch(&frame.body) {
                                    handle_dispatch(event, store);
                                }
                            }
                            protocol::OP_ACK => {
                                if let Some(ack) = decode_payload::<AckPayload>(&frame.body) {
                                    settle_ack(ack, store);
                                }
                            }
                            protocol::OP_ERROR => {
                                if let Some(err) = decode_payload::<ErrorPayload>(&frame.body) {
                                    report_error(err, store);
                                }
                            }
                            protocol::OP_RESYNC => {
                                last_seq = 0;
                                if let Some(id) = store.channel_id.read().clone() {
                                    spawn(actions::reload_history(store, id));
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => connected = false,
                },
                _ = tick => {
                    if online {
                        send_op(&mut sink, &ClientOp::Heartbeat { seq: last_seq }).await;
                    }
                    prune_typing(store);
                }
                command = rx.next().fuse() => {
                    let op = match command {
                        Some(Command::Subscribe(id)) => {
                            channel = Some(id);
                            Some(ClientOp::Subscribe { channel_id: id })
                        }
                        Some(Command::Send { nonce, channel_id, content, attachment_ids }) => Some(ClientOp::MessageCreate {
                            nonce,
                            channel_id,
                            content,
                            reply_to: None,
                            attachment_ids,
                        }),
                        Some(Command::Edit { nonce, message_id, content }) => {
                            Some(ClientOp::MessageEdit { nonce, message_id, content })
                        }
                        Some(Command::Reaction { message_id, emoji, add: true }) => {
                            Some(ClientOp::ReactionAdd { message_id, emoji })
                        }
                        Some(Command::Reaction { message_id, emoji, add: false }) => {
                            Some(ClientOp::ReactionRemove { message_id, emoji })
                        }
                        Some(Command::Typing(id)) => Some(ClientOp::Typing { channel_id: id }),
                        Some(Command::Close) | None => {
                            closed = true;
                            connected = false;
                            None
                        }
                    };
                    if let Some(op) = op
                        && online
                        && !send_op(&mut sink, &op).await
                    {
                        connected = false;
                    }
                }
            }
        }
        store.status.set(ConnStatus::Offline);
        if !closed {
            TimeoutFuture::new(backoff_ms(attempt)).await;
            attempt += 1;
        }
    }
    store.status.set(ConnStatus::Offline);
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

fn prune_typing(mut store: Store) {
    let now = now_ms();
    if store.typing.read().iter().any(|t| t.until_ms < now) {
        store.typing.write().retain(|t| t.until_ms >= now);
    }
}

fn report_error(err: ErrorPayload, mut store: Store) {
    // Abgelehnte eigene Nachricht markieren statt sie stumm verschwinden zu lassen.
    if let Some(nonce) = err.nonce {
        for history in store.history.write().values_mut() {
            if let Some(message) = history.messages.iter_mut().find(|m| m.nonce == Some(nonce)) {
                message.failed = true;
            }
        }
    }
    let text = match err.code.as_str() {
        "slowmode" => format!(
            "Slowmode aktiv – {}",
            err.message.trim_start_matches("slowmode: ")
        ),
        "rate_limited" => "Etwas langsamer bitte.".to_owned(),
        "forbidden" => "Dafür fehlen dir die Rechte (oder du hast einen Timeout).".to_owned(),
        "archived" => "Dieser Thread ist archiviert.".to_owned(),
        _ => err.message,
    };
    store.error(text);
}

fn mentions_me(message: &Message, store: Store) -> bool {
    let Some(me) = store.user.read().clone() else {
        return false;
    };
    message.content.contains(&format!("@{}", me.handle))
}

fn handle_dispatch(event: Dispatched, mut store: Store) {
    let current = store.channel_id.read().clone();
    let thread = store.thread_id.read().clone();
    match event {
        Dispatched::Created(message) => {
            let me = store.me_id();
            let channel_id = message.channel_id.clone();
            let visible = current.as_deref() == Some(channel_id.as_str())
                || thread.as_deref() == Some(channel_id.as_str());
            if !visible && me.as_deref() != Some(message.author.id.as_str()) {
                if let Some(detail) = store.detail.write().as_mut()
                    && let Some(unread) = detail
                        .unread
                        .iter_mut()
                        .find(|u| u.channel_id == channel_id)
                {
                    unread.last_message_id = Some(message.id.clone());
                }
                if mentions_me(&message, store) && store.settings.read().notifications {
                    crate::settings::notify(&message.author.display_name, &message.content);
                }
            }
            store
                .typing
                .write()
                .retain(|t| !(t.channel_id == channel_id && t.user_id == message.author.id));
            let mut histories = store.history.write();
            let history = histories.entry(channel_id.clone()).or_default();
            if history.messages.iter().any(|m| m.id == message.id) {
                return;
            }
            history.messages.push(message.clone());
            drop(histories);
            if visible {
                spawn(actions::mark_read(channel_id, message.id));
            }
        }
        Dispatched::Updated(message) => {
            if let Some(history) = store.history.write().get_mut(&message.channel_id)
                && let Some(existing) = history.messages.iter_mut().find(|m| m.id == message.id)
            {
                *existing = message;
            }
        }
        Dispatched::Deleted {
            channel_id,
            message_id,
        } => {
            if let Some(history) = store.history.write().get_mut(&channel_id) {
                history.messages.retain(|m| m.id != message_id);
            }
        }
        Dispatched::Typing {
            channel_id,
            user_id,
            display_name,
        } => {
            let mut typing = store.typing.write();
            typing.retain(|t| !(t.channel_id == channel_id && t.user_id == user_id));
            typing.push(Typing {
                channel_id,
                user_id,
                name: display_name,
                until_ms: now_ms() + 6_000.0,
            });
        }
        Dispatched::Reaction {
            channel_id,
            message_id,
            user_id,
            emoji,
            added,
        } => {
            let me = store.me_id();
            if let Some(history) = store.history.write().get_mut(&channel_id)
                && let Some(message) = history.messages.iter_mut().find(|m| m.id == message_id)
            {
                let mine = me.as_deref() == Some(user_id.as_str());
                match message.reactions.iter_mut().find(|r| r.emoji == emoji) {
                    Some(reaction) if added => {
                        reaction.count += 1;
                        reaction.me |= mine;
                    }
                    Some(reaction) => {
                        reaction.count -= 1;
                        if mine {
                            reaction.me = false;
                        }
                    }
                    None if added => message.reactions.push(crate::types::Reaction {
                        emoji,
                        count: 1,
                        me: mine,
                    }),
                    None => {}
                }
                message.reactions.retain(|r| r.count > 0);
            }
        }
        Dispatched::Presence { user_id, status } => {
            if let Some(detail) = store.detail.write().as_mut()
                && let Some(member) = detail.members.iter_mut().find(|m| m.user_id == user_id)
            {
                member.status = status;
            }
        }
        Dispatched::GuildUpdate { guild_id } | Dispatched::MemberUpdate { guild_id } => {
            let open = store
                .detail
                .read()
                .as_ref()
                .map(|detail| detail.guild.id == guild_id)
                .unwrap_or(false);
            if open {
                spawn(actions::refresh_guild(store));
            } else {
                spawn(actions::load_guild_list(store));
            }
        }
        Dispatched::MemberRemove { guild_id, user_id } => {
            if store.me_id().as_deref() == Some(user_id.as_str()) {
                store.notice("Du wurdest aus einem Server entfernt.");
                spawn(actions::load_guilds(store));
                return;
            }
            if let Some(detail) = store.detail.write().as_mut()
                && detail.guild.id == guild_id
            {
                detail.members.retain(|m| m.user_id != user_id);
            }
        }
        Dispatched::Pin {
            channel_id,
            message_id,
            pinned,
        } => {
            if let Some(history) = store.history.write().get_mut(&channel_id)
                && let Some(message) = history.messages.iter_mut().find(|m| m.id == message_id)
            {
                message.pinned = pinned;
            }
            if store.pins.read().is_some() {
                spawn(actions::load_pins(store));
            }
        }
        Dispatched::Voice(state) => actions::apply_voice_state(store, state),
        Dispatched::Other => {}
    }
}

fn settle_ack(ack: AckPayload, mut store: Store) {
    let mut histories = store.history.write();
    for history in histories.values_mut() {
        let Some(pos) = history
            .messages
            .iter()
            .position(|m| m.nonce == Some(ack.nonce))
        else {
            continue;
        };
        match ack.id {
            Some(id) => {
                let id = id.to_string();
                if history.messages.iter().any(|m| m.id == id) {
                    // Dispatch war schneller: lokale Kopie entfernen.
                    history.messages.remove(pos);
                } else {
                    let message = &mut history.messages[pos];
                    message.id = id;
                    message.nonce = None;
                    if let Some(created) = &ack.created_at {
                        message.created_at = created.clone();
                    }
                }
            }
            None => history.messages[pos].failed = true,
        }
        return;
    }
}
