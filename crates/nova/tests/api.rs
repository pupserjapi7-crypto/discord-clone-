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
    spawn_server_with_state(database_url).await.0
}

/// Wie [`spawn_server`], liefert zusätzlich den Zustand (für Aufräum-Tests).
async fn spawn_server_with_state(database_url: &str) -> (String, AppState) {
    // Serverfehler landen in der Testausgabe (nur bei Fehlschlag sichtbar).
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn")
        .with_test_writer()
        .try_init();
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
        guild_upload_quota: 4 * 1024 * 1024,
        dm_upload_quota: 4 * 1024 * 1024,
        uploads_per_minute: 1000.0,
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
        blobs: Arc::new(
            nova_api::blob::LocalFs::open(std::env::temp_dir().join("nova-test-uploads"))
                .await
                .expect("blob store"),
        ),
        blob_gc: Arc::new(tokio::sync::RwLock::new(())),
        media_limit: Arc::new(Semaphore::new(2)),
    };
    let app = nova_api::router(state.clone()).merge(gateway.router());
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
    (format!("http://{addr}"), state)
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

    /// Rohe Bytes hochladen (`POST …/attachments?filename=`).
    async fn upload(&self, path: &str, filename: &str, bytes: Vec<u8>) -> (u16, Value) {
        let response = self
            .http
            .post(format!(
                "{}{path}?filename={}",
                self.base,
                encode_query(filename)
            ))
            .bearer_auth(&self.token)
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .expect("upload");
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// GET mit zusätzlichen Headern, liefert die ganze Antwort.
    async fn fetch(&self, path: &str, headers: &[(&str, &str)]) -> reqwest::Response {
        let mut request = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send().await.expect("fetch")
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

#[tokio::test]
async fn direct_messages_open_send_list_and_stay_private() {
    let url = db_or_skip!();
    let base = spawn_server(&url).await;
    let alice = Client::register(&base, "dma").await;
    let bob = Client::register(&base, "dmb").await;
    let eve = Client::register(&base, "dme").await;
    let (_, bob_me) = bob.get("/api/v1/users/@me").await;
    let bob_handle = bob_me["handle"].as_str().expect("handle").to_owned();

    // Bob hört über das Gateway mit.
    let ws_url = format!("{}/gateway", base.replacen("http", "ws", 1));
    let (socket, _) = tokio_tungstenite::connect_async(ws_url)
        .await
        .expect("connect");
    let (mut sink, mut stream) = socket.split();
    sink.send(WsMessage::Binary(
        protocol::encode(
            protocol::OP_HELLO,
            0,
            &ClientOp::Hello {
                token: bob.token.clone(),
                resume: None,
            },
        )
        .expect("encode"),
    ))
    .await
    .expect("hello");
    next_frame(&mut stream, protocol::OP_HELLO_OK).await;

    // Unbekannte Namen und man selbst sind kein Gegenüber.
    let (status, _) = alice
        .post(
            "/api/v1/users/@me/dms",
            json!({ "handle": "gibt-es-nicht-xyz" }),
        )
        .await;
    assert_eq!(status, 404);
    let (_, alice_me) = alice.get("/api/v1/users/@me").await;
    let (status, _) = alice
        .post(
            "/api/v1/users/@me/dms",
            json!({ "handle": alice_me["handle"] }),
        )
        .await;
    assert_eq!(status, 400);

    // Öffnen ist idempotent, auch mit "@" und anderer Schreibweise.
    let (status, dm) = alice
        .post("/api/v1/users/@me/dms", json!({ "handle": bob_handle }))
        .await;
    assert_eq!(status, 200, "{dm}");
    let dm_id = dm["id"].as_str().expect("dm id").to_owned();
    assert_eq!(dm["user"]["id"].as_str(), Some(bob.id.as_str()));
    let (_, again) = bob
        .post(
            "/api/v1/users/@me/dms",
            json!({ "handle": format!("@{}", alice_me["handle"].as_str().unwrap().to_uppercase()) }),
        )
        .await;
    assert_eq!(again["id"].as_str(), Some(dm_id.as_str()));

    let (status, sent) = alice
        .post(
            &format!("/api/v1/dms/{dm_id}/messages"),
            json!({ "content": "Hallo Bob" }),
        )
        .await;
    assert_eq!(status, 201, "{sent}");
    let event: Event = protocol::decode_body(&next_frame(&mut stream, protocol::OP_DISPATCH).await)
        .expect("dispatch");
    match event {
        Event::DmMessageCreate { message } => {
            assert_eq!(message.content, "Hallo Bob");
            assert_eq!(message.channel_id.to_string(), dm_id);
            assert_eq!(message.author.id.to_string(), alice.id);
        }
        other => panic!("unerwartetes Event: {other:?}"),
    }

    let (status, history) = bob.get(&format!("/api/v1/dms/{dm_id}/messages")).await;
    assert_eq!(status, 200);
    assert_eq!(history[0]["content"], "Hallo Bob");
    let (_, list) = bob.get("/api/v1/users/@me/dms").await;
    assert_eq!(list[0]["id"].as_str(), Some(dm_id.as_str()));
    assert_eq!(list[0]["user"]["id"].as_str(), Some(alice.id.as_str()));

    // Dritte sehen weder Verlauf noch dürfen sie schreiben.
    let (status, _) = eve.get(&format!("/api/v1/dms/{dm_id}/messages")).await;
    assert_eq!(status, 404);
    let (status, _) = eve
        .post(
            &format!("/api/v1/dms/{dm_id}/messages"),
            json!({ "content": "Einbruch" }),
        )
        .await;
    assert_eq!(status, 404);
    let (_, eve_list) = eve.get("/api/v1/users/@me/dms").await;
    assert_eq!(eve_list.as_array().map(Vec::len), Some(0));
}

/// Kleines JPEG mit EXIF-Drehung (6) und GPS-Daten, Kommentar und Anhängsel
/// hinter dem Bildende.
fn jpeg_with_gps(marker: &str) -> Vec<u8> {
    let mut data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    data.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
    let mut tiff = b"MM\x00\x2A\x00\x00\x00\x08".to_vec();
    tiff.extend_from_slice(&[0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
    tiff.extend_from_slice(&[0, 0, 0, 0]);
    tiff.extend_from_slice(format!("GPS 52.52N 13.40E {marker}").as_bytes());
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    data.extend_from_slice(&[0xFF, 0xE1]);
    data.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    data.extend_from_slice(&app1);
    data.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 30, 0, 40, 1, 1, 0x11, 0]);
    data.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 0x3F, 0]);
    data.extend_from_slice(&[0x12, 0x34, 0xFF, 0x00, 0x56]);
    data.extend_from_slice(&[0xFF, 0xD9]);
    data.extend_from_slice(b"<html>polyglot</html>");
    data
}

#[tokio::test]
async fn uploads_strip_metadata_and_restrict_downloads() {
    let url = db_or_skip!();
    let base = spawn_server(&url).await;
    let owner = Client::register(&base, "upo").await;
    let member = Client::register(&base, "upm").await;
    let stranger = Client::register(&base, "ups").await;
    let (guild_id, channel_id) = owner.guild().await;
    owner.invite_and_join(&member, &guild_id).await;
    let upload_path = format!("/api/v1/channels/{channel_id}/attachments");

    // Fremde können nicht in den Kanal hochladen.
    let (status, _) = stranger
        .upload(&upload_path, "x.txt", b"hallo".to_vec())
        .await;
    assert_eq!(status, 404);

    let marker = unique("geheim");
    let (status, photo) = owner
        .upload(&upload_path, "../Urlaub 2026.jpg", jpeg_with_gps(&marker))
        .await;
    assert_eq!(status, 201, "{photo}");
    assert_eq!(photo["content_type"], "image/jpeg");
    assert_eq!(photo["filename"], "Urlaub 2026.jpg");
    // Drehung 6: Anzeige ist 30 breit, 40 hoch.
    assert_eq!(photo["width"], 30);
    assert_eq!(photo["height"], 40);
    let photo_id = photo["id"].as_str().expect("id").to_owned();
    let download = format!("/api/v1/attachments/{photo_id}");

    // Noch nicht gesendet: nur die hochladende Person sieht den Upload.
    assert_eq!(member.fetch(&download, &[]).await.status().as_u16(), 404);
    let response = owner.fetch(&download, &[]).await;
    assert_eq!(response.status().as_u16(), 200);
    let headers = response.headers().clone();
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["content-type"], "image/jpeg");
    assert!(
        headers["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("sandbox")
    );
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("inline;")
    );
    let etag = headers["etag"].to_str().unwrap().to_owned();
    let body = response.bytes().await.expect("body");
    let text = String::from_utf8_lossy(&body);
    assert!(!text.contains(&marker), "GPS-Daten müssen weg sein");
    assert!(!text.contains("polyglot"));
    assert_eq!(&body[..2], &[0xFF, 0xD8]);

    // Senden hängt den Upload an; danach sehen ihn alle Mitglieder.
    let (status, message) = owner
        .post(
            &format!("/api/v1/channels/{channel_id}/messages"),
            json!({ "content": "", "attachment_ids": [photo_id] }),
        )
        .await;
    assert_eq!(status, 201, "{message}");
    assert_eq!(message["attachments"][0]["id"], photo_id.as_str());
    let response = member.fetch(&download, &[]).await;
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.bytes().await.unwrap(), body);
    assert_eq!(stranger.fetch(&download, &[]).await.status().as_u16(), 404);

    // Ein Anhang gehört zu genau einer Nachricht.
    let (status, _) = owner
        .post(
            &format!("/api/v1/channels/{channel_id}/messages"),
            json!({ "content": "nochmal", "attachment_ids": [photo_id] }),
        )
        .await;
    assert_eq!(status, 400);
    // Fremde Uploads lassen sich nicht anhängen.
    let (status, note) = member
        .upload(&upload_path, "notiz.txt", b"nur meins".to_vec())
        .await;
    assert_eq!(status, 201);
    let (status, _) = owner
        .post(
            &format!("/api/v1/channels/{channel_id}/messages"),
            json!({ "content": "klau", "attachment_ids": [note["id"]] }),
        )
        .await;
    assert_eq!(status, 400);

    // Verlauf enthält den Anhang.
    let (_, history) = member
        .get(&format!("/api/v1/channels/{channel_id}/messages"))
        .await;
    let last = history.as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["attachments"][0]["filename"], "Urlaub 2026.jpg");

    // Teil-Download, Cache-Validierung
    let response = member.fetch(&download, &[("range", "bytes=0-1")]).await;
    assert_eq!(response.status().as_u16(), 206);
    assert_eq!(
        response.headers()["content-range"].to_str().unwrap(),
        format!("bytes 0-1/{}", body.len())
    );
    assert_eq!(response.bytes().await.unwrap().as_ref(), &[0xFF, 0xD8]);
    let response = member.fetch(&download, &[("if-none-match", &etag)]).await;
    assert_eq!(response.status().as_u16(), 304);

    // Textdateien und SVG sind Downloads, nie inline.
    let (status, svg) = owner
        .upload(
            &upload_path,
            "bild.svg",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>".to_vec(),
        )
        .await;
    assert_eq!(status, 201);
    assert_eq!(svg["content_type"], "text/plain; charset=utf-8");
    let response = owner
        .fetch(
            &format!("/api/v1/attachments/{}", svg["id"].as_str().unwrap()),
            &[],
        )
        .await;
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment;")
    );

    // Leer, zu groß
    let (status, _) = owner.upload(&upload_path, "leer.txt", Vec::new()).await;
    assert_eq!(status, 400);
    let (status, _) = owner
        .upload(&upload_path, "gross.bin", vec![7u8; 1024 * 1024 + 1])
        .await;
    assert_eq!(status, 413);

    // Verwerfen: nur eigene, nur ungesendete
    let (status, draft) = owner
        .upload(&upload_path, "entwurf.txt", b"weg damit".to_vec())
        .await;
    assert_eq!(status, 201);
    let draft_path = format!("/api/v1/attachments/{}", draft["id"].as_str().unwrap());
    assert_eq!(member.delete(&draft_path).await.0, 404);
    assert_eq!(owner.delete(&draft_path).await.0, 204);
    assert_eq!(owner.fetch(&draft_path, &[]).await.status().as_u16(), 404);
    assert_eq!(owner.delete(&download).await.0, 404, "gesendet bleibt");

    // Ohne ATTACH_FILES (Kanal-Override) kein Upload, aber Schreiben geht.
    let (status, _) = owner
        .put(
            &format!(
                "/api/v1/channels/{channel_id}/overrides/member/{}",
                member.id
            ),
            json!({ "allow": 0, "deny": 1u64 << 11 }),
        )
        .await;
    assert_eq!(status, 204);
    let (status, _) = member.upload(&upload_path, "x.txt", b"x".to_vec()).await;
    assert_eq!(status, 403);
    let (status, _) = member
        .post(
            &format!("/api/v1/channels/{channel_id}/messages"),
            json!({ "content": "Text geht noch" }),
        )
        .await;
    assert_eq!(status, 201);
}

#[tokio::test]
async fn uploads_reach_gateway_dms_and_get_cleaned_up() {
    let url = db_or_skip!();
    let (base, state) = spawn_server_with_state(&url).await;
    let alice = Client::register(&base, "upa").await;
    let bob = Client::register(&base, "upb").await;
    let eve = Client::register(&base, "upe").await;
    let (_, channel_id) = alice.guild().await;
    let channel: i64 = channel_id.parse().expect("channel id");

    // Gateway: Nachricht nur aus einem Anhang
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
            token: alice.token.clone(),
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
    next_frame(&mut stream, protocol::OP_RESPONSE).await;

    let content = unique("protokoll").into_bytes();
    let (status, file) = alice
        .upload(
            &format!("/api/v1/channels/{channel_id}/attachments"),
            "protokoll.txt",
            content.clone(),
        )
        .await;
    assert_eq!(status, 201, "{file}");
    let file_id: i64 = file["id"].as_str().unwrap().parse().unwrap();
    sink.send(send(
        protocol::OP_REQUEST,
        ClientOp::MessageCreate {
            nonce: 7,
            channel_id: channel,
            content: String::new(),
            reply_to: None,
            attachment_ids: vec![file_id],
        },
    ))
    .await
    .expect("send");
    next_frame(&mut stream, protocol::OP_ACK).await;
    let event: Event = protocol::decode_body(&next_frame(&mut stream, protocol::OP_DISPATCH).await)
        .expect("dispatch");
    match event {
        Event::MessageCreate { message } => {
            assert_eq!(message.attachments.len(), 1);
            assert_eq!(message.attachments[0].id, file_id);
            assert_eq!(message.attachments[0].filename, "protokoll.txt");
            assert!(
                message.attachments[0]
                    .url
                    .ends_with(&format!("/attachments/{file_id}"))
            );
        }
        other => panic!("unerwartetes Event: {other:?}"),
    }

    // Direktnachricht mit Anhang
    let (_, bob_me) = bob.get("/api/v1/users/@me").await;
    let (_, dm) = alice
        .post(
            "/api/v1/users/@me/dms",
            json!({ "handle": bob_me["handle"] }),
        )
        .await;
    let dm_id = dm["id"].as_str().unwrap().to_owned();
    let (status, _) = eve
        .upload(
            &format!("/api/v1/dms/{dm_id}/attachments"),
            "x.txt",
            b"x".to_vec(),
        )
        .await;
    assert_eq!(status, 404);
    let (status, secret) = alice
        .upload(
            &format!("/api/v1/dms/{dm_id}/attachments"),
            "geheim.txt",
            b"nur fuer bob".to_vec(),
        )
        .await;
    assert_eq!(status, 201, "{secret}");
    let secret_path = format!("/api/v1/attachments/{}", secret["id"].as_str().unwrap());
    assert_eq!(bob.fetch(&secret_path, &[]).await.status().as_u16(), 404);
    let (status, sent) = alice
        .post(
            &format!("/api/v1/dms/{dm_id}/messages"),
            json!({ "content": "", "attachment_ids": [secret["id"]] }),
        )
        .await;
    assert_eq!(status, 201, "{sent}");
    let response = bob.fetch(&secret_path, &[]).await;
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.text().await.unwrap(), "nur fuer bob");
    assert_eq!(eve.fetch(&secret_path, &[]).await.status().as_u16(), 404);
    let (_, history) = bob.get(&format!("/api/v1/dms/{dm_id}/messages")).await;
    assert_eq!(history[0]["attachments"][0]["filename"], "geheim.txt");

    // Aufräumen: ein nie gesendeter, alter Upload verschwindet samt Blob.
    let orphan_bytes = unique("verwaist").into_bytes();
    let (status, orphan) = alice
        .upload(
            &format!("/api/v1/channels/{channel_id}/attachments"),
            "verwaist.txt",
            orphan_bytes.clone(),
        )
        .await;
    assert_eq!(status, 201);
    let orphan_id: i64 = orphan["id"].as_str().unwrap().parse().unwrap();
    sqlx::query("update attachments set created_at = now() - interval '2 days' where id = $1")
        .bind(orphan_id)
        .execute(&state.db.pool)
        .await
        .expect("age row");
    let hash = hex_sha256(&orphan_bytes);
    let blob = std::env::temp_dir()
        .join("nova-test-uploads")
        .join(&hash[..2])
        .join(&hash);
    assert!(blob.exists(), "Blob liegt unter dem Hash");
    std::fs::File::options()
        .append(true)
        .open(&blob)
        .and_then(|file| {
            file.set_modified(std::time::SystemTime::now() - Duration::from_secs(7200))
        })
        .expect("age blob");
    let (rows, _) = nova_api::uploads::sweep(&state).await.expect("sweep");
    assert!(rows >= 1);
    assert!(!blob.exists(), "Blob ohne Verweis ist gelöscht");
    assert_eq!(
        alice
            .fetch(&format!("/api/v1/attachments/{orphan_id}"), &[])
            .await
            .status()
            .as_u16(),
        404
    );
    // Gesendete Anhänge bleiben.
    let response = alice
        .fetch(&format!("/api/v1/attachments/{file_id}"), &[])
        .await;
    assert_eq!(response.bytes().await.unwrap().as_ref(), content.as_slice());
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

fn encode_query(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}
