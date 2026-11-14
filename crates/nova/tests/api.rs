//! Integrationstests gegen einen echten Server (API + Gateway) auf einem
//! freien Port. Brauchen eine migrierte PostgreSQL-Datenbank in `DATABASE_URL`;
//! ohne die Variable werden sie übersprungen (CI setzt sie immer).

use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt};
use nova_api::{AppState, state::RateLimiter};
use nova_common::{
    config::Config,
    ids::Snowflake,
    metrics::Metrics,
    protocol::{self, ClientOp, Event, ServerOp},
    tokens::TokenStore,
};
use nova_db::Db;
use nova_gateway::Gateway;
use nova_sfu::{Sfu, SfuConfig};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio_tungstenite::tungstenite::Message as WsMessage;

static WORKER: AtomicU64 = AtomicU64::new(500);

fn database_url() -> Option<String> {
    std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("NOVA_DATABASE_URL"))
        .ok()
}

/// Startet API + Gateway wie `nova serve`, nur ohne statische Assets.
async fn spawn_server(database_url: &str) -> String {
    let cfg = Arc::new(Config {
        database_url: database_url.to_owned(),
        bind_addr: "127.0.0.1:0".parse().expect("addr"),
        public_origin: "http://127.0.0.1".to_owned(),
        upload_dir: PathBuf::from("target/test-uploads"),
        access_ttl: Duration::from_secs(900),
        refresh_ttl: Duration::from_secs(3600),
        max_message_len: 8000,
        max_upload_bytes: 1024 * 1024,
        max_avatar_bytes: 1024 * 1024,
        // Billige Argon2-Parameter: Tests prüfen Logik, nicht Härte.
        argon2_m_cost: 1024,
        argon2_t_cost: 1,
        argon2_p_cost: 1,
        heartbeat_timeout: Duration::from_secs(45),
        resume_ttl: Duration::from_secs(120),
        listen_backlog: 128,
        hash_concurrency: 2,
        msg_rate: 50.0,
        msg_burst: 50.0,
        worker_id: WORKER.fetch_add(1, Ordering::Relaxed),
        dev_mode: true,
        trust_proxy: false,
        auth_burst: 1000.0,
        auth_rate: 1000.0,
        log_filter: "warn".to_owned(),
    });
    let db = Db::connect(database_url, 5).await.expect("db");
    let metrics = Arc::new(Metrics::default());
    let tokens = Arc::new(TokenStore::new(cfg.access_ttl));
    let ids = Arc::new(Snowflake::new(cfg.worker_id));
    let gateway = Gateway::new(
        db.clone(),
        Arc::clone(&cfg),
        Arc::clone(&tokens),
        Arc::clone(&metrics),
        Arc::clone(&ids),
    );
    let state = AppState {
        db,
        cfg: Arc::clone(&cfg),
        tokens,
        metrics,
        ids,
        gateway: Arc::clone(&gateway),
        sfu: Arc::new(Sfu::new(SfuConfig {
            enabled: false,
            ..SfuConfig::from_env()
        })),
        started: Instant::now(),
        auth_limiter: Arc::new(RateLimiter::new()),
        hash_limit: Arc::new(Semaphore::new(2)),
    };
    let app = nova_api::router(state).merge(gateway.router());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .ok();
    });
    format!("http://{addr}")
}

fn unique(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{prefix}{}", nanos % 10_000_000_000)
}

struct Client {
    base: String,
    http: reqwest::Client,
    token: String,
    id: String,
}

impl Client {
    async fn register(base: &str, prefix: &str) -> Self {
        let http = reqwest::Client::new();
        let handle = unique(prefix);
        let response: Value = http
            .post(format!("{base}/api/v1/auth/register"))
            .json(&json!({ "handle": handle, "password": "korrekt-pferd-batterie" }))
            .send()
            .await
            .expect("register")
            .error_for_status()
            .expect("register status")
            .json()
            .await
            .expect("register json");
        Self {
            base: base.to_owned(),
            http,
            token: response["access_token"].as_str().expect("token").to_owned(),
            id: response["user"]["id"].as_str().expect("id").to_owned(),
        }
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.expect("request");
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        self.call(reqwest::Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }

    async fn put(&self, path: &str, body: Value) -> (u16, Value) {
        self.call(reqwest::Method::PUT, path, Some(body)).await
    }

    async fn delete(&self, path: &str) -> (u16, Value) {
        self.call(reqwest::Method::DELETE, path, None).await
    }

    /// Guild anlegen, liefert (guild_id, channel_id des Standardkanals).
    async fn guild(&self) -> (String, String) {
        let (status, guild) = self
            .post("/api/v1/guilds", json!({ "name": "Testserver" }))
            .await;
        assert_eq!(status, 201, "{guild}");
        let guild_id = guild["id"].as_str().expect("guild id").to_owned();
        let (_, detail) = self.get(&format!("/api/v1/guilds/{guild_id}")).await;
        let channel = detail["channels"][0]["id"]
            .as_str()
            .expect("channel")
            .to_owned();
        (guild_id, channel)
    }

    async fn invite_and_join(&self, other: &Client, guild_id: &str) {
        let (status, invite) = self
            .post(&format!("/api/v1/guilds/{guild_id}/invites"), json!({}))
            .await;
        assert_eq!(status, 201);
        let code = invite["code"].as_str().expect("code");
        let (status, _) = other
            .post(&format!("/api/v1/invites/{code}"), json!({}))
            .await;
        assert_eq!(status, 200);
    }
}

macro_rules! db_or_skip {
    () => {
        match database_url() {
            Some(url) => url,
            None => {
                eprintln!("DATABASE_URL fehlt – Integrationstest übersprungen");
                return;
            }
        }
    };
}

#[tokio::test]
async fn auth_register_login_refresh_and_rejects_bad_credentials() {
    let url = db_or_skip!();
    let base = spawn_server(&url).await;
    let http = reqwest::Client::new();
    let handle = unique("auth");

    let register = http
        .post(format!("{base}/api/v1/auth/register"))
        .json(&json!({ "handle": handle, "password": "korrekt-pferd-batterie" }))
        .send()
        .await
        .expect("register");
    assert_eq!(register.status().as_u16(), 200);
    let cookie = register
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("nova_rt="))
        .map(|value| value.split(';').next().unwrap_or_default().to_owned())
        .expect("refresh cookie");

    let duplicate = http
        .post(format!("{base}/api/v1/auth/register"))
        .json(&json!({ "handle": handle, "password": "korrekt-pferd-batterie" }))
        .send()
        .await
        .expect("duplicate");
    assert_eq!(duplicate.status().as_u16(), 409);

    let wrong = http
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({ "handle": handle, "password": "falsches-passwort" }))
        .send()
        .await
        .expect("wrong login");
    assert_eq!(wrong.status().as_u16(), 401);

    let login = http
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({ "handle": handle, "password": "korrekt-pferd-batterie" }))
        .send()
        .await
        .expect("login");
    assert_eq!(login.status().as_u16(), 200);

    let refresh = http
        .post(format!("{base}/api/v1/auth/refresh"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("refresh");
    assert_eq!(refresh.status().as_u16(), 200);
    // Rotation: dasselbe Cookie ein zweites Mal ist ungültig.
    let reuse = http
        .post(format!("{base}/api/v1/auth/refresh"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("reuse");
    assert_eq!(reuse.status().as_u16(), 401);

    let anonymous = http
        .get(format!("{base}/api/v1/guilds"))
        .send()
        .await
        .expect("anon");
    assert_eq!(anonymous.status().as_u16(), 401);
}

#[tokio::test]
async fn permissions_hide_foreign_guilds_and_enforce_roles() {
    let url = db_or_skip!();
    let base = spawn_server(&url).await;
    let owner = Client::register(&base, "own").await;
    let stranger = Client::register(&base, "str").await;
    let (guild_id, channel_id) = owner.guild().await;

    // Nicht-Mitglieder sehen weder Guild noch Kanal.
    assert_eq!(
        stranger.get(&format!("/api/v1/guilds/{guild_id}")).await.0,
        404
    );
    assert_eq!(
        stranger
            .get(&format!("/api/v1/channels/{channel_id}/messages"))
            .await
            .0,
        404
    );
    assert_eq!(
        stranger
            .post(
                &format!("/api/v1/channels/{channel_id}/messages"),
                json!({ "content": "rein da" })
            )
            .await
            .0,
        404
    );

    // Mitglied ohne Rechte darf keine Kanäle anlegen oder moderieren.
    owner.invite_and_join(&stranger, &guild_id).await;
    assert_eq!(
        stranger
            .post(
                &format!("/api/v1/guilds/{guild_id}/channels"),
                json!({ "name": "geheim" })
            )
            .await
            .0,
        403
    );
    assert_eq!(
        stranger
            .post(
                &format!("/api/v1/guilds/{guild_id}/members/{}/kick", owner.id),
                json!({})
            )
            .await
            .0,
        403
    );

    // Rolle mit MANAGE_CHANNELS (1 << 3) vergeben → jetzt erlaubt.
    let (status, role) = owner
        .post(
            &format!("/api/v1/guilds/{guild_id}/roles"),
            json!({ "name": "Kanalpflege", "permissions": 1u64 << 3, "position": 5 }),
        )
        .await;
    assert_eq!(status, 201, "{role}");
    let role_id = role["id"].as_str().expect("role id");
    let (status, _) = owner
        .put(
            &format!(
                "/api/v1/guilds/{guild_id}/members/{}/roles/{role_id}",
                stranger.id
            ),
            json!({}),
        )
        .await;
    assert_eq!(status, 204);
    let (status, _) = stranger
        .post(
            &format!("/api/v1/guilds/{guild_id}/channels"),
            json!({ "name": "neu" }),
        )
        .await;
    assert_eq!(status, 201);

    // Niemand vergibt Rechte, die er selbst nicht hat.
    let (status, _) = stranger
        .post(
            &format!("/api/v1/guilds/{guild_id}/roles"),
            json!({ "name": "Admin", "permissions": 1u64 }),
        )
        .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn messages_create_list_pin_delete_and_threads() {
    let url = db_or_skip!();
    let base = spawn_server(&url).await;
    let owner = Client::register(&base, "msg").await;
    let member = Client::register(&base, "mem").await;
    let (guild_id, channel_id) = owner.guild().await;
    owner.invite_and_join(&member, &guild_id).await;

    let (status, message) = member
        .post(
            &format!("/api/v1/channels/{channel_id}/messages"),
            json!({ "content": "Hallo Nova" }),
        )
        .await;
    assert_eq!(status, 201, "{message}");
    let message_id = message["id"].as_str().expect("id").to_owned();

    let (status, list) = owner
        .get(&format!("/api/v1/channels/{channel_id}/messages"))
        .await;
    assert_eq!(status, 200);
    assert!(
        list.as_array()
            .expect("list")
            .iter()
            .any(|m| m["id"] == message_id.as_str())
    );

    // Pinnen braucht MANAGE_MESSAGES.
    assert_eq!(
        member
            .put(
                &format!("/api/v1/channels/{channel_id}/pins/{message_id}"),
                json!({})
            )
            .await
            .0,
        403
    );
    assert_eq!(
        owner
            .put(
                &format!("/api/v1/channels/{channel_id}/pins/{message_id}"),
                json!({})
            )
            .await
            .0,
        204
    );
    let (_, pins) = owner
        .get(&format!("/api/v1/channels/{channel_id}/pins"))
        .await;
    assert_eq!(pins.as_array().map(Vec::len), Some(1));

    // Thread aus der Nachricht, darin schreiben, archivieren.
    let (status, thread) = member
        .post(
            &format!("/api/v1/channels/{channel_id}/messages/{message_id}/threads"),
            json!({ "name": "Diskussion zur Begrüßung" }),
        )
        .await;
    assert_eq!(status, 201, "{thread}");
    let thread_id = thread["id"].as_str().expect("thread").to_owned();
    assert_eq!(thread["kind"], 2);
    let (status, _) = owner
        .post(
            &format!("/api/v1/channels/{thread_id}/messages"),
            json!({ "content": "im Thread" }),
        )
        .await;
    assert_eq!(status, 201);
    let (status, _) = member
        .call(
            reqwest::Method::PATCH,
            &format!("/api/v1/channels/{thread_id}"),
            Some(json!({ "archived": true })),
        )
        .await;
    assert_eq!(status, 200, "Thread-Ersteller darf archivieren");
    let (status, _) = owner
        .post(
            &format!("/api/v1/channels/{thread_id}/messages"),
            json!({ "content": "zu spät" }),
        )
        .await;
    assert_eq!(status, 409, "archivierte Threads nehmen nichts mehr an");

    // Ungelesen-Stand taucht in den Guild-Details auf.
    let (_, detail) = owner.get(&format!("/api/v1/guilds/{guild_id}")).await;
    assert!(
        detail["unread"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    // Alle IDs – auch in Listen wie `role_ids` – sind Strings (JS-sicher).
    let owner_row = detail["members"]
        .as_array()
        .and_then(|members| members.iter().find(|m| m["user_id"] == owner.id.as_str()))
        .expect("owner in members");
    assert!(
        owner_row["role_ids"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty() && ids.iter().all(Value::is_string)),
        "{owner_row}"
    );

    // Fremde Nachricht löschen: nur mit MANAGE_MESSAGES (Owner ja).
    let (status, own) = owner
        .post(
            &format!("/api/v1/channels/{channel_id}/messages"),
            json!({ "content": "vom Owner" }),
        )
        .await;
    assert_eq!(status, 201);
    let own_id = own["id"].as_str().expect("id");
    assert_eq!(
        member
            .delete(&format!("/api/v1/channels/{channel_id}/messages/{own_id}"))
            .await
            .0,
        403
    );
    assert_eq!(
        owner
            .delete(&format!(
                "/api/v1/channels/{channel_id}/messages/{message_id}"
            ))
            .await
            .0,
        204
    );
}

#[tokio::test]
async fn moderation_timeout_slowmode_kick_ban_and_audit_log() {
    let url = db_or_skip!();
    let base = spawn_server(&url).await;
    let owner = Client::register(&base, "mod").await;
    let member = Client::register(&base, "tgt").await;
    let (guild_id, channel_id) = owner.guild().await;
    owner.invite_and_join(&member, &guild_id).await;

    // Timeout: Schreiben verboten, Lesen erlaubt.
    let (status, _) = owner
        .put(
            &format!("/api/v1/guilds/{guild_id}/members/{}/timeout", member.id),
            json!({ "seconds": 600, "reason": "Spam" }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        member
            .post(
                &format!("/api/v1/channels/{channel_id}/messages"),
                json!({ "content": "darf nicht" })
            )
            .await
            .0,
        403
    );
    assert_eq!(
        member
            .get(&format!("/api/v1/channels/{channel_id}/messages"))
            .await
            .0,
        200
    );
    owner
        .put(
            &format!("/api/v1/guilds/{guild_id}/members/{}/timeout", member.id),
            json!({ "seconds": 0 }),
        )
        .await;

    // Slowmode 60 s: zweite Nachricht wird gedrosselt, der Owner nicht.
    let (status, _) = owner
        .call(
            reqwest::Method::PATCH,
            &format!("/api/v1/channels/{channel_id}"),
            Some(json!({ "slowmode_secs": 60 })),
        )
        .await;
    assert_eq!(status, 200);
    let path = format!("/api/v1/channels/{channel_id}/messages");
    assert_eq!(
        member.post(&path, json!({ "content": "eins" })).await.0,
        201
    );
    assert_eq!(
        member.post(&path, json!({ "content": "zwei" })).await.0,
        429
    );
    assert_eq!(owner.post(&path, json!({ "content": "eins" })).await.0, 201);
    assert_eq!(owner.post(&path, json!({ "content": "zwei" })).await.0, 201);

    // Niemand moderiert den Owner, auch nicht sich selbst.
    assert_eq!(
        owner
            .post(
                &format!("/api/v1/guilds/{guild_id}/members/{}/kick", owner.id),
                json!({})
            )
            .await
            .0,
        400
    );

    // Kick → raus; Bann → kein Wiederbeitritt per Einladung.
    assert_eq!(
        owner
            .post(
                &format!("/api/v1/guilds/{guild_id}/members/{}/kick", member.id),
                json!({ "reason": "Test" })
            )
            .await
            .0,
        204
    );
    assert_eq!(
        member.get(&format!("/api/v1/guilds/{guild_id}")).await.0,
        404
    );
    owner.invite_and_join(&member, &guild_id).await;
    assert_eq!(
        owner
            .put(
                &format!("/api/v1/guilds/{guild_id}/bans/{}", member.id),
                json!({ "reason": "wiederholt" })
            )
            .await
            .0,
        204
    );
    let (_, invite) = owner
        .post(&format!("/api/v1/guilds/{guild_id}/invites"), json!({}))
        .await;
    let code = invite["code"].as_str().expect("code");
    assert_eq!(
        member
            .post(&format!("/api/v1/invites/{code}"), json!({}))
            .await
            .0,
        404
    );

    let (status, log) = owner
        .get(&format!("/api/v1/guilds/{guild_id}/audit-log"))
        .await;
    assert_eq!(status, 200);
    let actions: Vec<&str> = log
        .as_array()
        .expect("log")
        .iter()
        .filter_map(|entry| entry["action"].as_str())
        .collect();
    for expected in [
        "member_timeout",
        "member_timeout_clear",
        "channel_update",
        "member_kick",
        "member_ban",
    ] {
        assert!(
            actions.contains(&expected),
            "{expected} fehlt in {actions:?}"
        );
    }
    assert_eq!(
        member
            .get(&format!("/api/v1/guilds/{guild_id}/audit-log"))
            .await
            .0,
        404
    );
}

#[tokio::test]
async fn bots_authenticate_with_token_and_post_messages() {
    let url = db_or_skip!();
    let base = spawn_server(&url).await;
    let owner = Client::register(&base, "bto").await;
    let (guild_id, channel_id) = owner.guild().await;

    let (status, created) = owner
        .post(
            "/api/v1/bots",
            json!({ "handle": unique("bot"), "display_name": "Echo" }),
        )
        .await;
    assert_eq!(status, 201, "{created}");
    let bot_id = created["id"].as_str().expect("bot id").to_owned();
    let token = created["token"].as_str().expect("token").to_owned();
    assert!(token.starts_with("nb_"));

    let http = reqwest::Client::new();
    let as_bot = |method: reqwest::Method, path: String| {
        http.request(method, format!("{base}{path}"))
            .header("authorization", format!("Bot {token}"))
    };

    // Vor dem Hinzufügen sieht der Bot die Guild nicht.
    let status = as_bot(reqwest::Method::GET, format!("/api/v1/guilds/{guild_id}"))
        .send()
        .await
        .expect("bot get")
        .status();
    assert_eq!(status.as_u16(), 404);

    let (status, _) = owner
        .put(
            &format!("/api/v1/guilds/{guild_id}/bots/{bot_id}"),
            json!({}),
        )
        .await;
    assert_eq!(status, 204);

    let response = as_bot(
        reqwest::Method::POST,
        format!("/api/v1/channels/{channel_id}/messages"),
    )
    .json(&json!({ "content": "Hallo, ich bin ein Bot" }))
    .send()
    .await
    .expect("bot post");
    assert_eq!(response.status().as_u16(), 201);
    let message: Value = response.json().await.expect("json");
    assert_eq!(message["author"]["bot"], true);

    // Bots dürfen keine Bots anlegen; falsches Token ist 401.
    let status = as_bot(reqwest::Method::GET, "/api/v1/bots".to_owned())
        .send()
        .await
        .expect("bot list")
        .status();
    assert_eq!(status.as_u16(), 403);
    let wrong = http
        .get(format!("{base}/api/v1/guilds"))
        .header("authorization", format!("Bot nb_{}", "0".repeat(64)))
        .send()
        .await
        .expect("wrong");
    assert_eq!(wrong.status().as_u16(), 401);

    // Token erneuern: das alte gilt sofort nicht mehr.
    let (status, _) = owner
        .post(&format!("/api/v1/bots/{bot_id}/token"), json!({}))
        .await;
    assert_eq!(status, 200);
    let status = as_bot(reqwest::Method::GET, "/api/v1/guilds".to_owned())
        .send()
        .await
        .expect("old token")
        .status();
    assert_eq!(status.as_u16(), 401);
}

async fn next_frame(
    stream: &mut (
             impl StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin
         ),
    op: u8,
) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let message = tokio::time::timeout(remaining, stream.next())
            .await
            .expect("Zeitüberschreitung beim Warten auf einen Frame")
            .expect("Socket geschlossen")
            .expect("WebSocket-Fehler");
        if let WsMessage::Binary(bytes) = message {
            let frame = protocol::decode(&bytes).expect("frame");
            if frame.op == op {
                return frame.body.to_vec();
            }
        }
    }
}

#[tokio::test]
async fn gateway_roundtrip_ack_dispatch_and_rest_fanout() {
    let url = db_or_skip!();
    let base = spawn_server(&url).await;
    let owner = Client::register(&base, "gw").await;
    let (_, channel_id) = owner.guild().await;
    let channel: i64 = channel_id.parse().expect("channel id");

    let ws_url = format!("{}/gateway", base.replacen("http", "ws", 1));
    let (socket, _) = tokio_tungstenite::connect_async(ws_url)
        .await
        .expect("connect");
    let (mut sink, mut stream) = socket.split();

    let send = |op: u8, value: ClientOp| {
        WsMessage::Binary(protocol::encode(op, 0, &value).expect("encode"))
    };
    sink.send(send(
        protocol::OP_HELLO,
        ClientOp::Hello {
            token: owner.token.clone(),
            resume: None,
        },
    ))
    .await
    .expect("hello");
    let hello: ServerOp =
        protocol::decode_body(&next_frame(&mut stream, protocol::OP_HELLO_OK).await).expect("ok");
    assert!(matches!(hello, ServerOp::HelloOk { .. }));

    sink.send(send(
        protocol::OP_REQUEST,
        ClientOp::Subscribe {
            channel_id: channel,
        },
    ))
    .await
    .expect("subscribe");
    next_frame(&mut stream, protocol::OP_RESPONSE).await;

    sink.send(send(
        protocol::OP_REQUEST,
        ClientOp::MessageCreate {
            nonce: 42,
            channel_id: channel,
            content: "über das Gateway".into(),
            reply_to: None,
            attachment_ids: vec![],
        },
    ))
    .await
    .expect("send");
    let ack: ServerOp =
        protocol::decode_body(&next_frame(&mut stream, protocol::OP_ACK).await).expect("ack");
    let ServerOp::Ack { nonce, id, .. } = ack else {
        panic!("kein Ack: {ack:?}");
    };
    assert_eq!(nonce, 42);
    let event: Event = protocol::decode_body(&next_frame(&mut stream, protocol::OP_DISPATCH).await)
        .expect("dispatch");
    match event {
        Event::MessageCreate { message } => {
            assert_eq!(Some(message.id), id);
            assert_eq!(message.content, "über das Gateway");
        }
        other => panic!("unerwartetes Event: {other:?}"),
    }

    // Eine REST-Nachricht kommt ebenfalls über das Gateway an.
    let (status, _) = owner
        .post(
            &format!("/api/v1/channels/{channel_id}/messages"),
            json!({ "content": "per REST" }),
        )
        .await;
    assert_eq!(status, 201);
    let event: Event = protocol::decode_body(&next_frame(&mut stream, protocol::OP_DISPATCH).await)
        .expect("dispatch");
    assert!(matches!(event, Event::MessageCreate { message } if message.content == "per REST"));
}

/// Regression: vor v0.4 bekam jeder angemeldete Nutzer über das Gateway die
/// `@everyone`-Rechte fremder Guilds, wenn er eine Kanal-ID kannte.
#[tokio::test]
async fn gateway_rejects_non_members() {
    let url = db_or_skip!();
    let base = spawn_server(&url).await;
    let owner = Client::register(&base, "gwo").await;
    let stranger = Client::register(&base, "gws").await;
    let (_, channel_id) = owner.guild().await;
    let channel: i64 = channel_id.parse().expect("channel id");

    let ws_url = format!("{}/gateway", base.replacen("http", "ws", 1));
    let (socket, _) = tokio_tungstenite::connect_async(ws_url)
        .await
        .expect("connect");
    let (mut sink, mut stream) = socket.split();
    let send = |op: u8, value: ClientOp| {
        WsMessage::Binary(protocol::encode(op, 0, &value).expect("encode"))
    };
    sink.send(send(
        protocol::OP_HELLO,
        ClientOp::Hello {
            token: stranger.token.clone(),
            resume: None,
        },
    ))
    .await
    .expect("hello");
    next_frame(&mut stream, protocol::OP_HELLO_OK).await;

    sink.send(send(
        protocol::OP_REQUEST,
        ClientOp::Subscribe {
            channel_id: channel,
        },
    ))
    .await
    .expect("subscribe");
    let error: ServerOp =
        protocol::decode_body(&next_frame(&mut stream, protocol::OP_ERROR).await).expect("error");
    assert!(matches!(error, ServerOp::Error { ref code, .. } if code == "forbidden"));

    sink.send(send(
        protocol::OP_REQUEST,
        ClientOp::MessageCreate {
            nonce: 7,
            channel_id: channel,
            content: "Einbruch".into(),
            reply_to: None,
            attachment_ids: vec![],
        },
    ))
    .await
    .expect("send");
    let error: ServerOp =
        protocol::decode_body(&next_frame(&mut stream, protocol::OP_ERROR).await).expect("error");
    assert!(
        matches!(error, ServerOp::Error { nonce: Some(7), ref code, .. } if code == "forbidden")
    );
}
