//! Realtime gateway.
//!
//! One process holds all sessions in memory. Message mutations arrive over the
//! socket, are written to PostgreSQL once, and the serialized event is fanned out
//! to every subscriber with a per-session sequence number.

use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Router,
    extract::{
        State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use bytes::Bytes;
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use nova_common::{
    config::Config,
    ids::Snowflake,
    metrics::Metrics,
    permissions::Permissions,
    protocol::{
        self, ClientOp, Event, MAX_FRAME_BYTES, MessageView, OP_ACK, OP_DISPATCH, OP_ERROR,
        OP_HELLO, OP_HELLO_OK, OP_INVALID_SESSION, OP_RESUMED, OP_RESYNC, ServerOp, UserPublic,
    },
    tokens::TokenStore,
    validate,
};
use nova_db::{Db, PermCache, models::MessageRow, views};
use tokio::sync::{Notify, mpsc};
use tracing::{debug, warn};

const RING_FRAMES: usize = 128;
const RING_BYTES: usize = 32 * 1024;
const OUTBOX: usize = 256;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
    rate: f64,
    burst: f64,
}

impl Bucket {
    fn new(rate: f64, burst: f64) -> Self {
        Self {
            tokens: burst,
            last: Instant::now(),
            rate,
            burst,
        }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

enum Outgoing {
    Frame(Bytes),
    Close,
}

struct Session {
    id: String,
    user: UserPublic,
    guild_ids: Vec<i64>,
    seq: AtomicU32,
    tx: Mutex<Option<mpsc::Sender<Outgoing>>>,
    ring: Mutex<VecDeque<Bytes>>,
    ring_bytes: AtomicUsize,
    connected: AtomicBool,
    last_seen_ms: AtomicI64,
    msg_bucket: Mutex<Bucket>,
    typing_bucket: Mutex<Bucket>,
    subscriptions: Mutex<HashSet<i64>>,
    /// Set when the outbox was full: the socket is closed and the client must
    /// resume from `last_seq` instead of the server dropping frames silently.
    lagged: AtomicBool,
    /// Wakes the writer task to close the socket on backpressure.
    overflow: Mutex<Option<Arc<Notify>>>,
    /// Serializes seq assignment and enqueue so the client sees them in order.
    dispatch_order: Mutex<()>,
}

impl Session {
    fn new(
        id: String,
        user: UserPublic,
        guild_ids: Vec<i64>,
        msg_rate: f64,
        msg_burst: f64,
    ) -> Self {
        Self {
            id,
            user,
            guild_ids,
            seq: AtomicU32::new(0),
            tx: Mutex::new(None),
            ring: Mutex::new(VecDeque::with_capacity(RING_FRAMES)),
            ring_bytes: AtomicUsize::new(0),
            connected: AtomicBool::new(true),
            last_seen_ms: AtomicI64::new(now_ms()),
            msg_bucket: Mutex::new(Bucket::new(msg_rate, msg_burst)),
            // 1 typing event / 3s, burst 2.
            typing_bucket: Mutex::new(Bucket::new(1.0 / 3.0, 2.0)),
            subscriptions: Mutex::new(HashSet::new()),
            lagged: AtomicBool::new(false),
            overflow: Mutex::new(None),
            dispatch_order: Mutex::new(()),
        }
    }
}

/// Shared gateway state. Cloned as `Arc` into every connection task.
pub struct Gateway {
    db: Db,
    cfg: Arc<Config>,
    tokens: Arc<TokenStore>,
    metrics: Arc<Metrics>,
    ids: Arc<Snowflake>,
    perms: PermCache,
    sessions: DashMap<String, Arc<Session>>,
    channels: DashMap<i64, HashSet<String>>,
    guilds: DashMap<i64, HashSet<String>>,
    channel_guilds: DashMap<i64, i64>,
}

impl Gateway {
    pub fn new(
        db: Db,
        cfg: Arc<Config>,
        tokens: Arc<TokenStore>,
        metrics: Arc<Metrics>,
        ids: Arc<Snowflake>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            cfg,
            tokens,
            metrics,
            ids,
            perms: PermCache::new(Duration::from_secs(30)),
            sessions: DashMap::new(),
            channels: DashMap::new(),
            guilds: DashMap::new(),
            channel_guilds: DashMap::new(),
        })
    }

    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/gateway", get(ws_upgrade))
            .with_state(self)
    }

    /// Remove sessions whose resume window expired, and sweep stale caches.
    pub fn spawn_janitor(self: &Arc<Self>) {
        let gateway = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            loop {
                tick.tick().await;
                gateway.tokens.sweep();
                gateway.perms.invalidate_idle(Duration::from_secs(120));
                let ttl_ms = gateway.cfg.resume_ttl.as_millis() as i64;
                let now = now_ms();
                let expired: Vec<String> = gateway
                    .sessions
                    .iter()
                    .filter(|entry| {
                        !entry.connected.load(Ordering::Relaxed)
                            && now - entry.last_seen_ms.load(Ordering::Relaxed) > ttl_ms
                    })
                    .map(|entry| entry.id.clone())
                    .collect();
                for id in expired {
                    if let Some((_, session)) = gateway.sessions.remove(&id) {
                        gateway.metrics.ws_connections.dec();
                        gateway.drop_subscriptions(&session);
                    }
                }
            }
        });
    }

    pub fn broadcast_presence(&self, user_id: i64, status: i16) {
        let guilds: Vec<i64> = self
            .sessions
            .iter()
            .filter(|entry| entry.user.id == user_id)
            .flat_map(|entry| entry.guild_ids.clone())
            .collect();
        for guild_id in guilds {
            self.dispatch_guild(
                guild_id,
                &Event::PresenceUpdate {
                    guild_id,
                    user_id,
                    status,
                },
                None,
            );
        }
    }

    async fn guild_for_channel(&self, channel_id: i64) -> Option<i64> {
        if let Some(cached) = self.channel_guilds.get(&channel_id) {
            return Some(*cached);
        }
        let guild_id = self.db.channel_guild(channel_id).await.ok().flatten()?;
        self.channel_guilds.insert(channel_id, guild_id);
        Some(guild_id)
    }

    async fn channel_permissions(
        &self,
        channel_id: i64,
        user_id: i64,
    ) -> Option<(i64, Permissions)> {
        let guild_id = self.guild_for_channel(channel_id).await?;
        let perms = self
            .perms
            .get(&self.db, guild_id, user_id, Some(channel_id))
            .await
            .ok()?;
        Some((guild_id, perms))
    }

    async fn handle_socket(self: Arc<Self>, socket: WebSocket) {
        let (mut sink, mut stream) = socket.split();
        let (tx, mut rx) = mpsc::channel::<Outgoing>(OUTBOX);
        let overflow = Arc::new(Notify::new());
        let writer_overflow = Arc::clone(&overflow);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = writer_overflow.notified() => {
                        // 1013 = try again later: the client reconnects with its
                        // last sequence number and the ring replays the gap.
                        let close = CloseFrame {
                            code: 1013,
                            reason: "backpressure".into(),
                        };
                        let _ = sink.send(Message::Close(Some(close))).await;
                        break;
                    }
                    out = rx.recv() => match out {
                        Some(Outgoing::Frame(bytes)) => {
                            if sink.send(Message::Binary(bytes)).await.is_err() {
                                break;
                            }
                        }
                        Some(Outgoing::Close) | None => {
                            let _ = sink.send(Message::Close(None)).await;
                            break;
                        }
                    },
                }
            }
            let _ = sink.close().await;
        });

        let first = match tokio::time::timeout(Duration::from_secs(10), stream.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => bytes,
            _ => return,
        };
        let Ok(frame) = protocol::decode(&first) else {
            return;
        };
        if frame.op != OP_HELLO {
            return;
        }
        let Ok(ClientOp::Hello { token, resume }) = protocol::decode_body::<ClientOp>(frame.body)
        else {
            return;
        };
        let Some(access) = self.tokens.validate(&token) else {
            self.send_to(
                &tx,
                OP_INVALID_SESSION,
                0,
                &ServerOp::InvalidSession { resumable: false },
            );
            return;
        };

        let session = match resume {
            Some(request) => match self.sessions.get(&request.session_id) {
                Some(entry) if entry.user.id == access.user_id => {
                    let session = Arc::clone(&entry);
                    drop(entry);
                    *lock(&session.tx) = Some(tx.clone());
                    session.connected.store(true, Ordering::Relaxed);
                    session.last_seen_ms.store(now_ms(), Ordering::Relaxed);
                    self.metrics.ws_reconnects.inc();
                    // Replay everything the client missed, in order. If the ring
                    // no longer covers `last_seq`, say so explicitly: never a
                    // silent gap.
                    let plan = plan_replay(&lock(&session.ring), request.last_seq);
                    if let Some(oldest_seq) = plan.gap_oldest {
                        self.metrics.ws_resync.inc();
                        self.send_to(&tx, OP_RESYNC, 0, &ServerOp::Resync { oldest_seq });
                    }
                    for frame in plan.frames {
                        if tx.try_send(Outgoing::Frame(frame)).is_err() {
                            break;
                        }
                    }
                    let seq = session.seq.load(Ordering::Relaxed);
                    self.send_to(&tx, OP_RESUMED, seq, &ServerOp::Resumed { seq });
                    self.metrics.ws_connections.set(self.sessions.len() as i64);
                    Some(session)
                }
                _ => {
                    self.send_to(
                        &tx,
                        OP_INVALID_SESSION,
                        0,
                        &ServerOp::InvalidSession { resumable: false },
                    );
                    None
                }
            },
            None => {
                let Some(user) = self.db.user_by_id(access.user_id).await.ok().flatten() else {
                    return;
                };
                let guilds = self.db.guilds_for_user(user.id).await.unwrap_or_default();
                let guild_ids: Vec<i64> = guilds.iter().map(|guild| guild.id).collect();
                let user_public = views::user_public(&self.cfg.public_origin, &user);
                let id = self.ids.next_id().to_string();
                let session = Arc::new(Session::new(
                    id.clone(),
                    user_public.clone(),
                    guild_ids.clone(),
                    self.cfg.msg_rate,
                    self.cfg.msg_burst,
                ));
                *lock(&session.tx) = Some(tx.clone());
                for guild_id in &guild_ids {
                    self.guilds.entry(*guild_id).or_default().insert(id.clone());
                }
                self.sessions.insert(id.clone(), Arc::clone(&session));
                self.metrics.ws_connections.inc();
                self.metrics.ws_connections.set(self.sessions.len() as i64);

                let hello = ServerOp::HelloOk {
                    session_id: id.clone(),
                    user: user_public,
                    guilds: guilds
                        .into_iter()
                        .map(|guild| protocol::GuildSummary {
                            id: guild.id,
                            name: guild.name,
                        })
                        .collect(),
                    seq: 0,
                    heartbeat_secs: 30,
                };
                self.send_to(&tx, OP_HELLO_OK, 0, &hello);
                for guild_id in guild_ids {
                    self.dispatch_guild(
                        guild_id,
                        &Event::PresenceUpdate {
                            guild_id,
                            user_id: user.id,
                            status: user.status,
                        },
                        Some(&session.id),
                    );
                }
                Some(session)
            }
        };

        let Some(session) = session else {
            let _ = tx.try_send(Outgoing::Close);
            return;
        };

        *lock(&session.overflow) = Some(Arc::clone(&overflow));
        session.lagged.store(false, Ordering::Relaxed);
        self.read_loop(&mut stream, &session).await;

        // Disconnect: keep the session around for the resume window.
        session.connected.store(false, Ordering::Relaxed);
        session.last_seen_ms.store(now_ms(), Ordering::Relaxed);
        *lock(&session.tx) = None;
        *lock(&session.overflow) = None;
        self.drop_subscriptions(&session);
        for guild_id in &session.guild_ids {
            self.dispatch_guild(
                *guild_id,
                &Event::PresenceUpdate {
                    guild_id: *guild_id,
                    user_id: session.user.id,
                    status: 0,
                },
                None,
            );
        }
        debug!(session = %session.id, "gateway session disconnected");
    }

    async fn read_loop(
        &self,
        stream: &mut futures_util::stream::SplitStream<WebSocket>,
        session: &Arc<Session>,
    ) {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    let idle = now_ms() - session.last_seen_ms.load(Ordering::Relaxed);
                    if idle > self.cfg.heartbeat_timeout.as_millis() as i64 {
                        warn!(session = %session.id, "heartbeat timeout");
                        break;
                    }
                }
                incoming = stream.next() => {
                    let Some(Ok(message)) = incoming else { break };
                    session.last_seen_ms.store(now_ms(), Ordering::Relaxed);
                    let bytes = match message {
                        Message::Binary(bytes) => bytes,
                        Message::Close(_) => break,
                        Message::Ping(_) | Message::Pong(_) => continue,
                        Message::Text(_) => continue,
                    };
                    if bytes.len() > MAX_FRAME_BYTES + protocol::HEADER_LEN {
                        break;
                    }
                    self.metrics.ws_frames_in.inc();
                    let Ok(frame) = protocol::decode(&bytes) else { continue };
                    let Ok(op) = protocol::decode_body::<ClientOp>(frame.body) else {
                        continue;
                    };
                    self.handle_op(session, op).await;
                }
            }
        }
    }

    async fn handle_op(&self, session: &Arc<Session>, op: ClientOp) {
        let user_id = session.user.id;
        match op {
            ClientOp::Hello { .. } => {
                self.send_error(
                    session,
                    "already_identified",
                    "session already identified",
                    None,
                );
            }
            ClientOp::Heartbeat { seq } => {
                self.send_op(
                    session,
                    protocol::OP_HEARTBEAT_ACK,
                    &ServerOp::HeartbeatAck { seq },
                );
            }
            ClientOp::Subscribe { channel_id } => {
                let Some((_guild_id, perms)) = self.channel_permissions(channel_id, user_id).await
                else {
                    self.send_error(session, "not_found", "channel not found", None);
                    return;
                };
                if !perms.contains(Permissions::VIEW_CHANNEL) {
                    self.send_error(session, "forbidden", "cannot view channel", None);
                    return;
                }
                self.channels
                    .entry(channel_id)
                    .or_default()
                    .insert(session.id.clone());
                lock(&session.subscriptions).insert(channel_id);
                self.send_op(
                    session,
                    protocol::OP_RESPONSE,
                    &ServerOp::Subscribed { channel_id },
                );
            }
            ClientOp::Unsubscribe { channel_id } => {
                if let Some(mut set) = self.channels.get_mut(&channel_id) {
                    set.remove(&session.id);
                }
                lock(&session.subscriptions).remove(&channel_id);
            }
            ClientOp::MessageCreate {
                nonce,
                channel_id,
                content,
                reply_to,
                attachment_ids: _,
            } => {
                let started = Instant::now();
                if !lock(&session.msg_bucket).allow() {
                    self.send_error(session, "rate_limited", "slow down", Some(nonce));
                    return;
                }
                let Some((_guild_id, perms)) = self.channel_permissions(channel_id, user_id).await
                else {
                    self.send_error(session, "not_found", "channel not found", Some(nonce));
                    return;
                };
                if !perms.contains(Permissions::SEND_MESSAGES) {
                    self.send_error(session, "forbidden", "cannot send messages", Some(nonce));
                    return;
                }
                if let Err(msg) =
                    validate::validate_message_content(&content, self.cfg.max_message_len)
                {
                    self.send_error(session, "bad_request", msg, Some(nonce));
                    return;
                }
                let id = self.ids.next_id();
                let created_at = match self
                    .db
                    .insert_message(id, channel_id, user_id, &content, reply_to)
                    .await
                {
                    Ok(ts) => ts,
                    Err(err) => {
                        warn!(error = %err, "insert message failed");
                        self.send_error(
                            session,
                            "internal",
                            "could not store message",
                            Some(nonce),
                        );
                        return;
                    }
                };
                self.db
                    .upsert_read_state(user_id, channel_id, id)
                    .await
                    .ok();
                self.metrics.messages_created.inc();
                self.send_op(
                    session,
                    OP_ACK,
                    &ServerOp::Ack {
                        nonce,
                        id: Some(id),
                        created_at: Some(created_at),
                    },
                );
                let view = MessageView {
                    id,
                    channel_id,
                    author: session.user.clone(),
                    content,
                    created_at,
                    edited_at: None,
                    reply_to,
                    reactions: Vec::new(),
                    attachments: Vec::new(),
                };
                self.dispatch_channel(channel_id, &Event::MessageCreate { message: view });
                self.metrics
                    .message_latency
                    .record_us(started.elapsed().as_micros() as u64);
            }
            ClientOp::MessageEdit {
                nonce,
                message_id,
                content,
            } => {
                if let Err(msg) =
                    validate::validate_message_content(&content, self.cfg.max_message_len)
                {
                    self.send_error(session, "bad_request", msg, Some(nonce));
                    return;
                }
                let Some(row) = self.db.message(message_id).await.ok().flatten() else {
                    self.send_error(session, "not_found", "message not found", Some(nonce));
                    return;
                };
                let Some((_guild_id, perms)) =
                    self.channel_permissions(row.channel_id, user_id).await
                else {
                    self.send_error(session, "not_found", "channel not found", Some(nonce));
                    return;
                };
                let allowed =
                    row.author_id == user_id || perms.contains(Permissions::MANAGE_MESSAGES);
                if !allowed {
                    self.send_error(
                        session,
                        "forbidden",
                        "cannot edit this message",
                        Some(nonce),
                    );
                    return;
                }
                if let Err(err) = self.db.update_message(message_id, &content).await {
                    warn!(error = %err, "update message failed");
                    self.send_error(session, "internal", "could not edit message", Some(nonce));
                    return;
                }
                self.send_op(
                    session,
                    OP_ACK,
                    &ServerOp::Ack {
                        nonce,
                        id: Some(message_id),
                        created_at: None,
                    },
                );
                if let Some(view) = self.message_view(row, user_id).await {
                    self.dispatch_channel(view.channel_id, &Event::MessageUpdate { message: view });
                }
            }
            ClientOp::MessageDelete { nonce, message_id } => {
                let Some(row) = self.db.message(message_id).await.ok().flatten() else {
                    self.send_error(session, "not_found", "message not found", Some(nonce));
                    return;
                };
                let Some((_guild_id, perms)) =
                    self.channel_permissions(row.channel_id, user_id).await
                else {
                    self.send_error(session, "not_found", "channel not found", Some(nonce));
                    return;
                };
                if row.author_id != user_id && !perms.contains(Permissions::MANAGE_MESSAGES) {
                    self.send_error(
                        session,
                        "forbidden",
                        "cannot delete this message",
                        Some(nonce),
                    );
                    return;
                }
                if let Err(err) = self.db.delete_message(message_id).await {
                    warn!(error = %err, "delete message failed");
                    self.send_error(session, "internal", "could not delete message", Some(nonce));
                    return;
                }
                self.send_op(
                    session,
                    OP_ACK,
                    &ServerOp::Ack {
                        nonce,
                        id: Some(message_id),
                        created_at: None,
                    },
                );
                self.dispatch_channel(
                    row.channel_id,
                    &Event::MessageDelete {
                        channel_id: row.channel_id,
                        message_id,
                    },
                );
            }
            ClientOp::ReactionAdd { message_id, emoji } => {
                if let Err(msg) = validate::validate_emoji(&emoji) {
                    self.send_error(session, "bad_request", msg, None);
                    return;
                }
                let Some(row) = self.db.message(message_id).await.ok().flatten() else {
                    self.send_error(session, "not_found", "message not found", None);
                    return;
                };
                let Some((_guild_id, perms)) =
                    self.channel_permissions(row.channel_id, user_id).await
                else {
                    return;
                };
                if !perms.contains(Permissions::ADD_REACTIONS) {
                    self.send_error(session, "forbidden", "cannot react", None);
                    return;
                }
                if self
                    .db
                    .add_reaction(message_id, user_id, &emoji)
                    .await
                    .is_ok()
                {
                    self.dispatch_channel(
                        row.channel_id,
                        &Event::ReactionAdd {
                            channel_id: row.channel_id,
                            message_id,
                            user_id,
                            emoji,
                        },
                    );
                }
            }
            ClientOp::ReactionRemove { message_id, emoji } => {
                let Some(row) = self.db.message(message_id).await.ok().flatten() else {
                    return;
                };
                if self
                    .db
                    .remove_reaction(message_id, user_id, &emoji)
                    .await
                    .is_ok()
                {
                    self.dispatch_channel(
                        row.channel_id,
                        &Event::ReactionRemove {
                            channel_id: row.channel_id,
                            message_id,
                            user_id,
                            emoji,
                        },
                    );
                }
            }
            ClientOp::Typing { channel_id } => {
                if !lock(&session.typing_bucket).allow() {
                    return;
                }
                let Some((_guild_id, perms)) = self.channel_permissions(channel_id, user_id).await
                else {
                    return;
                };
                if !perms.contains(Permissions::SEND_MESSAGES) {
                    return;
                }
                self.dispatch_channel_except(
                    channel_id,
                    &Event::TypingStart {
                        channel_id,
                        user_id,
                        display_name: session.user.display_name.clone(),
                    },
                    &session.id,
                );
            }
            ClientOp::ReadAck {
                channel_id,
                message_id,
            } => {
                self.db
                    .upsert_read_state(user_id, channel_id, message_id)
                    .await
                    .ok();
            }
        }
    }

    async fn message_view(&self, row: MessageRow, viewer_id: i64) -> Option<MessageView> {
        let reactions = self.db.reactions_for(&[row.id], viewer_id).await.ok()?;
        let attachments = self.db.attachments_for(&[row.id]).await.ok()?;
        Some(views::message_view(
            &self.cfg.public_origin,
            row,
            reactions,
            attachments,
        ))
    }

    fn dispatch_channel(&self, channel_id: i64, event: &Event) {
        self.dispatch_channel_except(channel_id, event, "");
    }

    fn dispatch_channel_except(&self, channel_id: i64, event: &Event, skip: &str) {
        let Ok(body) = rmp_serde::to_vec_named(event) else {
            return;
        };
        let body = Bytes::from(body);
        let subscribers: Vec<String> = match self.channels.get(&channel_id) {
            Some(set) => set
                .iter()
                .filter(|id| id.as_str() != skip)
                .cloned()
                .collect(),
            None => return,
        };
        let started = Instant::now();
        for id in subscribers {
            if let Some(session) = self.sessions.get(&id) {
                self.push_dispatch(&session, &body);
            }
        }
        self.metrics
            .dispatch_latency
            .record_us(started.elapsed().as_micros() as u64);
    }

    fn dispatch_guild(&self, guild_id: i64, event: &Event, skip: Option<&str>) {
        let Ok(body) = rmp_serde::to_vec_named(event) else {
            return;
        };
        let body = Bytes::from(body);
        let subscribers: Vec<String> = match self.guilds.get(&guild_id) {
            Some(set) => set
                .iter()
                .filter(|id| skip.map(|skip| id.as_str() != skip).unwrap_or(true))
                .cloned()
                .collect(),
            None => return,
        };
        for id in subscribers {
            if let Some(session) = self.sessions.get(&id) {
                self.push_dispatch(&session, &body);
            }
        }
    }

    fn push_dispatch(&self, session: &Session, body: &Bytes) {
        // Holding this across seq + enqueue guarantees a single order for the
        // client and the resume ring, even with concurrent dispatchers.
        let _order = lock(&session.dispatch_order);
        let seq = session.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let frame = protocol::encode_body(OP_DISPATCH, seq, body);
        self.push_frame(session, frame);
    }

    fn push_frame(&self, session: &Session, frame: Bytes) {
        deliver_frame(&self.metrics, session, frame);
    }

    fn send_op<T: serde::Serialize>(&self, session: &Session, op: u8, payload: &T) {
        let seq = session.seq.load(Ordering::Relaxed);
        match protocol::encode(op, seq, payload) {
            Ok(frame) => self.push_frame(session, frame),
            Err(err) => warn!(error = %err, "encode server op failed"),
        }
    }

    fn send_to<T: serde::Serialize>(
        &self,
        tx: &mpsc::Sender<Outgoing>,
        op: u8,
        seq: u32,
        payload: &T,
    ) {
        match protocol::encode(op, seq, payload) {
            Ok(frame) => {
                let _ = tx.try_send(Outgoing::Frame(frame));
            }
            Err(err) => warn!(error = %err, "encode server op failed"),
        }
    }

    fn send_error(&self, session: &Session, code: &str, message: &str, nonce: Option<u64>) {
        self.send_op(
            session,
            OP_ERROR,
            &ServerOp::Error {
                code: code.to_owned(),
                message: message.to_owned(),
                nonce,
            },
        );
    }

    fn drop_subscriptions(&self, session: &Session) {
        let channels: Vec<i64> = lock(&session.subscriptions).drain().collect();
        for channel_id in channels {
            if let Some(mut set) = self.channels.get_mut(&channel_id) {
                set.remove(&session.id);
            }
        }
        for guild_id in &session.guild_ids {
            if let Some(mut set) = self.guilds.get_mut(guild_id) {
                set.remove(&session.id);
            }
        }
    }
}

/// Append to the resume ring, then try the live outbox. A full outbox never
/// drops silently: the frame stays in the ring and the session is flagged so
/// the writer closes the socket and the client resumes.
fn deliver_frame(metrics: &Metrics, session: &Session, frame: Bytes) {
    {
        let mut ring = lock(&session.ring);
        let mut bytes = session.ring_bytes.load(Ordering::Relaxed) + frame.len();
        ring.push_back(frame.clone());
        while ring.len() > RING_FRAMES || bytes > RING_BYTES {
            match ring.pop_front() {
                Some(old) => bytes = bytes.saturating_sub(old.len()),
                None => break,
            }
        }
        session.ring_bytes.store(bytes, Ordering::Relaxed);
    }
    if session.lagged.load(Ordering::Relaxed) {
        return;
    }
    let tx = lock(&session.tx).clone();
    if let Some(tx) = tx {
        match tx.try_send(Outgoing::Frame(frame)) {
            Ok(()) => metrics.ws_frames_out.inc(),
            Err(mpsc::error::TrySendError::Full(_)) => {
                if !session.lagged.swap(true, Ordering::Relaxed) {
                    metrics.ws_lagged.inc();
                    if let Some(notify) = lock(&session.overflow).clone() {
                        notify.notify_one();
                    }
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }
}

struct ReplayPlan {
    frames: Vec<Bytes>,
    /// Present when the ring does not contain every event after `last_seq`.
    gap_oldest: Option<u32>,
}

/// Decide what a resuming client can be sent without hiding a gap.
fn plan_replay(ring: &VecDeque<Bytes>, last_seq: u32) -> ReplayPlan {
    let oldest_available = ring
        .front()
        .and_then(|frame| protocol::decode(frame).ok())
        .map(|header| header.seq);
    let gap_oldest = oldest_available.filter(|oldest| *oldest > last_seq.saturating_add(1));
    let frames = ring
        .iter()
        .filter(|frame| {
            protocol::decode(frame)
                .map(|header| header.seq > last_seq)
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    ReplayPlan { frames, gap_oldest }
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(gateway): State<Arc<Gateway>>,
) -> impl IntoResponse {
    ws.max_message_size(MAX_FRAME_BYTES + protocol::HEADER_LEN)
        .on_upgrade(move |socket| gateway.handle_socket(socket))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch(seq: u32) -> Bytes {
        protocol::encode_body(OP_DISPATCH, seq, b"event")
    }

    fn ring(range: std::ops::RangeInclusive<u32>) -> VecDeque<Bytes> {
        range.map(dispatch).collect()
    }

    fn seqs(plan: &ReplayPlan) -> Vec<u32> {
        plan.frames
            .iter()
            .map(|frame| protocol::decode(frame).expect("frame").seq)
            .collect()
    }

    #[test]
    fn resume_inside_window_replays_everything_after_last_seq() {
        let plan = plan_replay(&ring(1..=100), 40);
        assert_eq!(plan.gap_oldest, None);
        assert_eq!(seqs(&plan), (41..=100).collect::<Vec<_>>());
    }

    #[test]
    fn resume_before_ring_start_reports_the_gap() {
        let plan = plan_replay(&ring(73..=200), 10);
        assert_eq!(plan.gap_oldest, Some(73));
        assert_eq!(seqs(&plan).first(), Some(&73));
    }

    #[test]
    fn reconnect_never_returns_a_silent_gap() {
        let ring = ring(50..=60);
        for last_seq in 0..=60 {
            let plan = plan_replay(&ring, last_seq);
            match plan.gap_oldest {
                None => {
                    let expected: Vec<u32> = ((last_seq + 1)..=60).collect();
                    assert_eq!(seqs(&plan), expected, "last_seq={last_seq}");
                }
                Some(oldest) => {
                    assert!(oldest > last_seq + 1);
                    assert_eq!(seqs(&plan), (oldest..=60).collect::<Vec<_>>());
                }
            }
        }
    }

    fn session() -> Session {
        Session::new(
            "session-1".to_owned(),
            UserPublic {
                id: 1,
                handle: "load0000".to_owned(),
                display_name: "Load 0000".to_owned(),
                avatar_url: None,
                status: 1,
            },
            vec![1],
            1.0,
            10.0,
        )
    }

    /// A full outbox must disconnect and keep the frame for resume — never a
    /// silent `try_send` drop.
    #[tokio::test]
    async fn full_outbox_lags_session_and_keeps_frame_for_resume() {
        let metrics = Metrics::default();
        let session = session();
        let (tx, _rx) = mpsc::channel::<Outgoing>(1);
        *lock(&session.tx) = Some(tx.clone());
        let notify = Arc::new(Notify::new());
        *lock(&session.overflow) = Some(Arc::clone(&notify));
        tx.try_send(Outgoing::Frame(dispatch(1))).expect("fill");

        deliver_frame(&metrics, &session, dispatch(2));

        assert!(session.lagged.load(Ordering::Relaxed));
        assert_eq!(metrics.ws_lagged.get(), 1);
        tokio::time::timeout(Duration::from_millis(100), notify.notified())
            .await
            .expect("writer must be woken to close the socket");
        // The evicted frame is still replayable: no event disappears.
        let plan = plan_replay(&lock(&session.ring), 1);
        assert_eq!(plan.gap_oldest, None);
        assert_eq!(seqs(&plan), vec![2]);
    }

    #[test]
    fn evicted_ring_forces_explicit_resync() {
        let mut ring = ring(1..=RING_FRAMES as u32 + 10);
        while ring.len() > RING_FRAMES {
            ring.pop_front();
        }
        let plan = plan_replay(&ring, 1);
        assert_eq!(plan.gap_oldest, Some(11));
        let plan = plan_replay(&ring, 10);
        assert_eq!(plan.gap_oldest, None);
        assert_eq!(seqs(&plan).first(), Some(&11));
    }
}
