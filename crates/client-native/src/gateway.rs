//! Natives Gateway: `tokio-tungstenite` + das Protokoll aus `nova-common`.
//!
//! Keine eigene Kopie des Wire-Formats mehr: Frame-Header, `ClientOp`,
//! `ServerOp` und `Event` kommen direkt aus dem Server-Crate.

use std::hash::{Hash, Hasher};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iced::Subscription;
use iced::futures::channel::mpsc;
use nova_common::protocol::{self, ClientOp, Event, GuildSummary, ResumeRequest, ServerOp};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::api::Api;

#[derive(Debug, Clone)]
pub enum Command {
    Subscribe(i64),
    Send {
        nonce: u64,
        channel_id: i64,
        content: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Online,
    Offline,
}

#[derive(Debug, Clone)]
pub enum GatewayEvent {
    Ready(mpsc::UnboundedSender<Command>),
    Status(Status),
    Hello { guilds: Vec<GuildSummary> },
    Event(Box<Event>),
    Ack { nonce: u64, id: Option<i64> },
    Error(String),
    Resync,
    SessionExpired,
}

/// Parameter der Subscription. Nur `epoch` bestimmt die Identität: ein neuer
/// Login erhöht sie, iced beendet dann die alte Verbindung und startet neu.
#[derive(Clone)]
pub struct Params {
    pub epoch: u64,
    pub api: Api,
}

impl Hash for Params {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.epoch.hash(state);
        self.api.base().hash(state);
    }
}

pub fn subscription(params: Params) -> Subscription<GatewayEvent> {
    Subscription::run_with(params, |params| run(params.clone()))
}

fn run(params: Params) -> impl iced::futures::Stream<Item = GatewayEvent> {
    iced::stream::channel(256, async move |mut out: mpsc::Sender<GatewayEvent>| {
        let api = params.api;
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded::<Command>();
        let _ = out.send(GatewayEvent::Ready(cmd_tx)).await;

        let mut session: Option<String> = None;
        let mut last_seq = 0u32;
        let mut channel: Option<i64> = None;
        let mut attempt = 0u32;

        loop {
            let _ = out.send(GatewayEvent::Status(Status::Connecting)).await;

            // Nach dem ersten Versuch ist das Access-Token womöglich abgelaufen.
            if (attempt > 0 || api.access_token().is_none())
                && let Err(err) = api.refresh().await
            {
                if !api.has_saved_session() {
                    let _ = out.send(GatewayEvent::SessionExpired).await;
                    std::future::pending::<()>().await;
                }
                let _ = out.send(GatewayEvent::Error(err)).await;
            }
            let Some(token) = api.access_token() else {
                backoff(&mut attempt).await;
                continue;
            };

            let ws = match tokio_tungstenite::connect_async(api.gateway_url()).await {
                Ok((ws, _)) => ws,
                Err(err) => {
                    let _ = out.send(GatewayEvent::Status(Status::Offline)).await;
                    let _ = out
                        .send(GatewayEvent::Error(format!("Gateway: {err}")))
                        .await;
                    backoff(&mut attempt).await;
                    continue;
                }
            };
            let (mut sink, mut stream) = ws.split();

            let hello = ClientOp::Hello {
                token,
                resume: session.clone().map(|session_id| ResumeRequest {
                    session_id,
                    last_seq,
                }),
            };
            if send(&mut sink, protocol::OP_HELLO, &hello).await.is_err() {
                backoff(&mut attempt).await;
                continue;
            }

            let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
            heartbeat.tick().await;
            let mut online = false;

            loop {
                tokio::select! {
                    incoming = stream.next() => {
                        let bytes = match incoming {
                            Some(Ok(WsMessage::Binary(bytes))) => bytes,
                            Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                            Some(Ok(_)) => continue,
                        };
                        let Ok(frame) = protocol::decode(&bytes) else { continue };
                        match frame.op {
                            protocol::OP_HELLO_OK | protocol::OP_RESUMED => {
                                match protocol::decode_body::<ServerOp>(frame.body) {
                                    Ok(ServerOp::HelloOk { session_id, guilds, seq, heartbeat_secs, .. }) => {
                                        session = Some(session_id);
                                        last_seq = seq;
                                        let secs = u64::from(heartbeat_secs.clamp(5, 60));
                                        heartbeat = tokio::time::interval(Duration::from_secs(secs));
                                        heartbeat.tick().await;
                                        let _ = out.send(GatewayEvent::Hello { guilds }).await;
                                    }
                                    Ok(ServerOp::Resumed { .. }) => {}
                                    _ => continue,
                                }
                                online = true;
                                attempt = 0;
                                let _ = out.send(GatewayEvent::Status(Status::Online)).await;
                                if let Some(id) = channel {
                                    let _ = send(&mut sink, protocol::OP_REQUEST, &ClientOp::Subscribe { channel_id: id }).await;
                                }
                            }
                            protocol::OP_INVALID_SESSION => {
                                session = None;
                                last_seq = 0;
                                break;
                            }
                            protocol::OP_DISPATCH => {
                                last_seq = frame.seq;
                                if let Ok(event) = protocol::decode_body::<Event>(frame.body) {
                                    let _ = out.send(GatewayEvent::Event(Box::new(event))).await;
                                }
                            }
                            protocol::OP_ACK => {
                                if let Ok(ServerOp::Ack { nonce, id, .. }) = protocol::decode_body(frame.body) {
                                    let _ = out.send(GatewayEvent::Ack { nonce, id }).await;
                                }
                            }
                            protocol::OP_ERROR => {
                                if let Ok(ServerOp::Error { message, .. }) = protocol::decode_body(frame.body) {
                                    let _ = out.send(GatewayEvent::Error(message)).await;
                                }
                            }
                            protocol::OP_RESYNC => {
                                last_seq = 0;
                                let _ = out.send(GatewayEvent::Resync).await;
                            }
                            _ => {}
                        }
                    }
                    command = cmd_rx.next() => {
                        let Some(command) = command else { return };
                        let op = match command {
                            Command::Subscribe(id) => {
                                channel = Some(id);
                                ClientOp::Subscribe { channel_id: id }
                            }
                            Command::Send { nonce, channel_id, content } => ClientOp::MessageCreate {
                                nonce,
                                channel_id,
                                content,
                                reply_to: None,
                                attachment_ids: Vec::new(),
                            },
                        };
                        if online && send(&mut sink, protocol::OP_REQUEST, &op).await.is_err() {
                            break;
                        }
                    }
                    _ = heartbeat.tick() => {
                        if online && send(&mut sink, protocol::OP_REQUEST, &ClientOp::Heartbeat { seq: last_seq }).await.is_err() {
                            break;
                        }
                    }
                }
            }

            let _ = out.send(GatewayEvent::Status(Status::Offline)).await;
            backoff(&mut attempt).await;
        }
    })
}

async fn send<S>(sink: &mut S, op: u8, value: &ClientOp) -> Result<(), ()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
{
    let frame = protocol::encode(op, 0, value).map_err(|_| ())?;
    sink.send(WsMessage::Binary(frame)).await.map_err(|_| ())
}

/// Exponentiell bis 15 s, plus Zufallsanteil, damit nach einem Server-Neustart
/// nicht alle Clients in derselben Millisekunde wieder anklopfen.
async fn backoff(attempt: &mut u32) {
    let base = 500u64.saturating_mul(1 << (*attempt).min(5)).min(15_000);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()))
        .unwrap_or(0);
    let jitter = nanos % (base / 2 + 1);
    *attempt = attempt.saturating_add(1);
    tokio::time::sleep(Duration::from_millis(base / 2 + jitter)).await;
}
