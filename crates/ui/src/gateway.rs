use dioxus::prelude::*;
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::stream::SplitSink;
use futures_util::{FutureExt, SinkExt, StreamExt, pin_mut, select};
use gloo_net::websocket::Message;
use gloo_net::websocket::futures::WebSocket;
use gloo_timers::future::TimeoutFuture;
use serde::de::DeserializeOwned;

use crate::api;
use crate::protocol::{self, AckPayload, ClientOp, Dispatched, ErrorPayload, HelloOk};
use crate::types::SessionState;

#[derive(Clone)]
pub struct Gateway {
    tx: UnboundedSender<Command>,
}

enum Command {
    Subscribe(i64),
    SendMessage {
        nonce: u64,
        channel_id: i64,
        content: String,
    },
    Close,
}

impl Gateway {
    pub fn start(
        token: String,
        my_id: String,
        initial_channel: Option<i64>,
        state: Signal<SessionState>,
    ) -> Self {
        let (tx, rx) = unbounded();
        spawn(async move {
            run(token, my_id, initial_channel, state, rx).await;
        });
        Self { tx }
    }

    pub fn subscribe(&self, channel_id: i64) {
        let _ = self.tx.unbounded_send(Command::Subscribe(channel_id));
    }

    pub fn send_message(&self, nonce: u64, channel_id: i64, content: String) {
        let _ = self.tx.unbounded_send(Command::SendMessage {
            nonce,
            channel_id,
            content,
        });
    }

    pub fn close(&self) {
        let _ = self.tx.unbounded_send(Command::Close);
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

async fn send_op(
    sink: &mut SplitSink<WebSocket, Message>,
    op: u8,
    seq: u32,
    value: &ClientOp,
) -> bool {
    let Ok(frame) = protocol::encode(op, seq, value) else {
        return false;
    };
    sink.send(Message::Bytes(frame)).await.is_ok()
}

async fn run(
    token: String,
    my_id: String,
    initial_channel: Option<i64>,
    mut state: Signal<SessionState>,
    mut rx: UnboundedReceiver<Command>,
) {
    let mut session: Option<String> = None;
    let mut last_seq = 0u32;
    let mut channel = initial_channel;
    let mut closed = false;

    while !closed {
        state.write().status = "connecting".into();
        let ws = match WebSocket::open(&websocket_url()) {
            Ok(ws) => ws,
            Err(err) => {
                state.write().error = format!("Gateway: {err}");
                state.write().status = "offline".into();
                TimeoutFuture::new(900).await;
                continue;
            }
        };
        let (mut sink, mut stream) = ws.split();
        let hello = ClientOp::Hello {
            token: token.clone(),
            resume: session.as_ref().map(|id| protocol::ResumeRequest {
                session_id: id.clone(),
                last_seq,
            }),
        };
        if !send_op(&mut sink, protocol::OP_HELLO, 0, &hello).await {
            state.write().status = "offline".into();
            TimeoutFuture::new(600).await;
            continue;
        }
        let mut online = false;
        let mut heartbeat_ms = 25_000u32;
        let mut reconnect = true;
        while reconnect {
            let tick = TimeoutFuture::new(heartbeat_ms).fuse();
            pin_mut!(tick);
            select! {
                incoming = stream.next().fuse() => {
                    match incoming {
                        Some(Ok(Message::Bytes(bytes))) => {
                            let Ok(frame) = protocol::decode(&bytes) else { continue };
                            match frame.op {
                                protocol::OP_HELLO_OK => {
                                    if let Some(hello) = decode_payload::<HelloOk>(&frame.body) {
                                        session = Some(hello.session_id);
                                        last_seq = hello.seq;
                                        heartbeat_ms = (hello.heartbeat_secs.max(5) * 1000).min(60_000);
                                        online = true;
                                        state.write().status = "online".into();
                                        state.write().error = String::new();
                                        if let Some(id) = channel {
                                            let _ = send_op(&mut sink, protocol::OP_REQUEST, 0, &ClientOp::Subscribe { channel_id: id }).await;
                                        }
                                    }
                                }
                                protocol::OP_RESUMED => {
                                    online = true;
                                    state.write().status = "online".into();
                                    if let Some(id) = channel {
                                        let _ = send_op(&mut sink, protocol::OP_REQUEST, 0, &ClientOp::Subscribe { channel_id: id }).await;
                                    }
                                }
                                protocol::OP_HEARTBEAT_ACK => {}
                                protocol::OP_INVALID_SESSION => {
                                    session = None;
                                    last_seq = 0;
                                    reconnect = false;
                                    state.write().status = "reconnecting".into();
                                }
                                protocol::OP_DISPATCH => {
                                    last_seq = frame.seq;
                                    handle_dispatch(&frame.body, &my_id, channel, &mut state);
                                }
                                protocol::OP_ACK => {
                                    if let Some(ack) = decode_payload::<AckPayload>(&frame.body) {
                                        settle_ack(ack, &mut state);
                                    }
                                }
                                protocol::OP_ERROR => {
                                    if let Some(err) = decode_payload::<ErrorPayload>(&frame.body) {
                                        state.write().error = err.message;
                                    }
                                }
                                protocol::OP_RESYNC => {
                                    last_seq = 0;
                                    if let Some(id) = channel
                                        && let Ok(history) = api::messages(&id.to_string()).await
                                    {
                                        state.write().messages = history;
                                    }
                                }
                                _ => {}
                            }
                        }
                        Some(Ok(_)) => {}
                        Some(Err(err)) => {
                            state.write().error = format!("Gateway: {err}");
                            reconnect = false;
                        }
                        None => reconnect = false,
                    }
                }
                _ = tick => {
                    if online {
                        let _ = send_op(&mut sink, protocol::OP_REQUEST, 0, &ClientOp::Heartbeat { seq: last_seq }).await;
                    }
                }
                command = rx.next().fuse() => {
                    match command {
                        Some(Command::Subscribe(id)) => {
                            channel = Some(id);
                            if online {
                                let _ = send_op(&mut sink, protocol::OP_REQUEST, 0, &ClientOp::Subscribe { channel_id: id }).await;
                            }
                        }
                        Some(Command::SendMessage { nonce, channel_id, content }) => {
                            let _ = send_op(&mut sink, protocol::OP_REQUEST, 0, &ClientOp::MessageCreate {
                                nonce,
                                channel_id,
                                content,
                                reply_to: None,
                                attachment_ids: Vec::new(),
                            }).await;
                        }
                        Some(Command::Close) | None => {
                            closed = true;
                            reconnect = false;
                        }
                    }
                }
            }
        }
        state.write().status = "offline".into();
        if !closed {
            TimeoutFuture::new(700).await;
        }
    }
    state.write().status = "offline".into();
}

fn handle_dispatch(
    body: &[u8],
    my_id: &str,
    channel: Option<i64>,
    state: &mut Signal<SessionState>,
) {
    let Some(event) = protocol::parse_dispatch(body) else {
        return;
    };
    match event {
        Dispatched::Created(message) => {
            let channel_id = message.channel_id.clone();
            let mut guard = state.write();
            let list = &mut guard.messages;
            if list.iter().any(|m| m.id == message.id) {
                return;
            }
            if message.author.id == my_id
                && let Some(pos) = list
                    .iter()
                    .position(|m| m.id.starts_with("local-") && m.content == message.content)
            {
                list.remove(pos);
            }
            if channel.map(|id| id.to_string()).as_deref() == Some(channel_id.as_str()) {
                list.push(message);
            }
        }
        Dispatched::Updated(message) => {
            let mut guard = state.write();
            let list = &mut guard.messages;
            if let Some(existing) = list.iter_mut().find(|m| m.id == message.id) {
                *existing = message;
            }
        }
        Dispatched::Deleted { message_id, .. } => {
            let id = message_id.to_string();
            state.write().messages.retain(|m| m.id != id);
        }
        Dispatched::Other => {}
    }
}

fn settle_ack(ack: AckPayload, state: &mut Signal<SessionState>) {
    let Some(id) = ack.id else {
        return;
    };
    let local = format!("local-{}", ack.nonce);
    let mut guard = state.write();
    let list = &mut guard.messages;
    if let Some(message) = list.iter_mut().find(|m| m.id == local) {
        message.id = id.to_string();
        if let Some(created) = ack.created_at {
            message.created_at = created;
        }
    }
}
