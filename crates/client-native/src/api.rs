//! REST-Client für `/api/v1`.
//!
//! Der Server schreibt jede `id`/`*_id` in JSON als String (wegen JavaScript);
//! hier wird das wieder zu `i64`. Der Refresh-Token kommt als `HttpOnly`-Cookie
//! `nova_rt`, den wir selbst lesen, rotieren und lokal ablegen, damit die App
//! beim nächsten Start ohne Login auskommt.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::Deserialize;
use serde::de::{DeserializeOwned, Deserializer};

const REFRESH_COOKIE: &str = "nova_rt";

#[derive(Clone)]
pub struct Api {
    base: Arc<str>,
    http: reqwest::Client,
    access: Arc<RwLock<Option<String>>>,
    refresh: Arc<RwLock<Option<String>>>,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api").field("base", &self.base).finish()
    }
}

// ---------------------------------------------------------------------------
// Typen
// ---------------------------------------------------------------------------

fn id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Int(i64),
        Str(String),
    }
    match Raw::deserialize(deserializer)? {
        Raw::Int(value) => Ok(value),
        Raw::Str(value) => value.parse().map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    #[serde(deserialize_with = "id")]
    pub id: i64,
    pub display_name: String,
}

fn opt_id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Int(i64),
        Str(String),
    }
    match Option::<Raw>::deserialize(deserializer)? {
        None => Ok(None),
        Some(Raw::Int(value)) => Ok(Some(value)),
        Some(Raw::Str(value)) => value.parse().map(Some).map_err(serde::de::Error::custom),
    }
}

fn ids<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<i64>, D::Error> {
    let raw = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .filter_map(|value| match value {
            serde_json::Value::Number(n) => n.as_i64(),
            serde_json::Value::String(s) => s.parse().ok(),
            _ => None,
        })
        .collect())
}

/// Rechte-Bits wie in `nova_common::permissions`.
pub mod perm {
    pub const ADMINISTRATOR: u64 = 1 << 0;
    pub const MANAGE_ROLES: u64 = 1 << 2;
    pub const MANAGE_CHANNELS: u64 = 1 << 3;
    pub const KICK_MEMBERS: u64 = 1 << 4;
    pub const BAN_MEMBERS: u64 = 1 << 5;
    pub const TIMEOUT_MEMBERS: u64 = 1 << 6;
    pub const MANAGE_MESSAGES: u64 = 1 << 9;
    pub const CREATE_INVITE: u64 = 1 << 19;
    pub const CREATE_THREADS: u64 = 1 << 23;
    pub const MANAGE_THREADS: u64 = 1 << 24;
}

#[derive(Debug, Clone, Deserialize)]
pub struct Guild {
    #[serde(deserialize_with = "id")]
    pub id: i64,
    pub name: String,
}

/// `kind`: 0 Text, 1 Sprache, 2 Thread.
#[derive(Debug, Clone, Deserialize)]
pub struct Channel {
    #[serde(deserialize_with = "id")]
    pub id: i64,
    pub name: String,
    pub kind: i16,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default, deserialize_with = "opt_id")]
    pub parent_id: Option<i64>,
    #[serde(default, deserialize_with = "opt_id")]
    pub owner_id: Option<i64>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub slowmode_secs: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Member {
    #[serde(default, deserialize_with = "opt_id")]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub handle: String,
    pub display_name: String,
    #[serde(default)]
    pub status: i16,
    #[serde(default, deserialize_with = "ids")]
    pub role_ids: Vec<i64>,
    #[serde(default)]
    pub timeout_until: Option<String>,
    #[serde(default)]
    pub bot: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Role {
    #[serde(deserialize_with = "id")]
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub is_everyone: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Unread {
    #[serde(deserialize_with = "id")]
    pub channel_id: i64,
    #[serde(default, deserialize_with = "opt_id")]
    pub last_message_id: Option<i64>,
    #[serde(default, deserialize_with = "opt_id")]
    pub last_read_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GuildDetail {
    pub guild: Guild,
    #[serde(default)]
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub roles: Vec<Role>,
    #[serde(default)]
    pub permissions: u64,
    #[serde(default)]
    pub unread: Vec<Unread>,
}

impl GuildDetail {
    pub fn can(&self, bit: u64) -> bool {
        self.permissions & perm::ADMINISTRATOR != 0 || self.permissions & bit == bit
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    #[serde(default)]
    pub count: i64,
    #[serde(default)]
    pub me: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    #[serde(deserialize_with = "id")]
    pub id: i64,
    pub author: User,
    pub content: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub edited_at: Option<String>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub pinned: bool,
}

/// Direktnachrichten-Kanal und das Gegenüber.
#[derive(Debug, Clone, Deserialize)]
pub struct DmChannel {
    #[serde(deserialize_with = "id")]
    pub id: i64,
    pub user: DmPeer,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DmPeer {
    pub handle: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Invite {
    pub code: String,
}

#[derive(Deserialize)]
struct AuthResponse {
    user: User,
    access_token: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

impl Api {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("nova-desktop/", env!("CARGO_PKG_VERSION")))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .tcp_nodelay(true)
            .build()
            .expect("HTTP-Client");
        Self {
            base: base.trim_end_matches('/').into(),
            http,
            access: Arc::default(),
            refresh: Arc::new(RwLock::new(load_refresh(base))),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn has_saved_session(&self) -> bool {
        self.refresh.read().unwrap().is_some()
    }

    pub fn access_token(&self) -> Option<String> {
        self.access.read().unwrap().clone()
    }

    pub fn gateway_url(&self) -> String {
        let rest = self
            .base
            .strip_prefix("https://")
            .map(|host| format!("wss://{host}"))
            .or_else(|| {
                self.base
                    .strip_prefix("http://")
                    .map(|h| format!("ws://{h}"))
            })
            .unwrap_or_else(|| self.base.to_string());
        format!("{rest}/gateway")
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, String> {
        let mut builder = self.http.request(method, format!("{}{path}", self.base));
        if let Some(token) = self.access_token() {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = body {
            builder = builder.json(&body);
        }
        let response = builder.send().await.map_err(|err| err.to_string())?;
        self.handle(response).await
    }

    async fn handle<T: DeserializeOwned>(&self, response: reqwest::Response) -> Result<T, String> {
        let status = response.status();
        self.capture_refresh(response.headers());
        let bytes = response.bytes().await.map_err(|err| err.to_string())?;
        if !status.is_success() {
            return Err(serde_json::from_slice::<ErrorBody>(&bytes)
                .map(|body| body.message)
                .unwrap_or_else(|_| format!("HTTP {status}")));
        }
        let bytes: &[u8] = if bytes.is_empty() { b"null" } else { &bytes };
        serde_json::from_slice(bytes).map_err(|err| format!("Antwort unlesbar: {err}"))
    }

    fn capture_refresh(&self, headers: &reqwest::header::HeaderMap) {
        for value in headers.get_all(reqwest::header::SET_COOKIE) {
            let Ok(text) = value.to_str() else { continue };
            let Some(pair) = text.split(';').next() else {
                continue;
            };
            if let Some(token) = pair.trim().strip_prefix(&format!("{REFRESH_COOKIE}=")) {
                let token = (!token.is_empty()).then(|| token.to_owned());
                save_refresh(&self.base, token.as_deref());
                *self.refresh.write().unwrap() = token;
            }
        }
    }

    fn finish_auth(&self, response: AuthResponse) -> User {
        *self.access.write().unwrap() = Some(response.access_token);
        response.user
    }

    pub async fn login(
        &self,
        handle: &str,
        password: &str,
        register: bool,
    ) -> Result<User, String> {
        let path = if register {
            "/api/v1/auth/register"
        } else {
            "/api/v1/auth/login"
        };
        let body = serde_json::json!({ "handle": handle, "password": password });
        let response: AuthResponse = self
            .request(reqwest::Method::POST, path, Some(body))
            .await?;
        Ok(self.finish_auth(response))
    }

    /// Holt mit dem gespeicherten Refresh-Cookie ein neues Access-Token.
    /// Der Server rotiert das Cookie dabei; `capture_refresh` speichert das neue.
    pub async fn refresh(&self) -> Result<User, String> {
        let token = self
            .refresh
            .read()
            .unwrap()
            .clone()
            .ok_or("keine Sitzung")?;
        let response = self
            .http
            .post(format!("{}/api/v1/auth/refresh", self.base))
            .header(reqwest::header::COOKIE, format!("{REFRESH_COOKIE}={token}"))
            .send()
            .await
            .map_err(|err| err.to_string())?;
        let unauthorized = response.status() == reqwest::StatusCode::UNAUTHORIZED;
        let result: Result<AuthResponse, String> = self.handle(response).await;
        if unauthorized {
            save_refresh(&self.base, None);
            *self.refresh.write().unwrap() = None;
        }
        Ok(self.finish_auth(result?))
    }

    pub async fn logout(&self) {
        let token = self.refresh.read().unwrap().clone();
        if let Some(token) = token {
            let _ = self
                .http
                .post(format!("{}/api/v1/auth/logout", self.base))
                .header(reqwest::header::COOKIE, format!("{REFRESH_COOKIE}={token}"))
                .send()
                .await;
        }
        save_refresh(&self.base, None);
        *self.refresh.write().unwrap() = None;
        *self.access.write().unwrap() = None;
    }

    pub async fn guilds(&self) -> Result<Vec<Guild>, String> {
        self.request(reqwest::Method::GET, "/api/v1/guilds", None)
            .await
    }

    pub async fn guild(&self, id: i64) -> Result<GuildDetail, String> {
        self.request(reqwest::Method::GET, &format!("/api/v1/guilds/{id}"), None)
            .await
    }

    pub async fn messages(&self, channel: i64) -> Result<Vec<Message>, String> {
        self.request(
            reqwest::Method::GET,
            &format!("/api/v1/channels/{channel}/messages?limit=50"),
            None,
        )
        .await
    }

    /// Aufruf ohne verwertbare Antwort (204 oder beliebiges JSON).
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(), String> {
        let _: serde_json::Value = self.request(method, path, body).await?;
        Ok(())
    }

    pub async fn older_messages(&self, channel: i64, before: i64) -> Result<Vec<Message>, String> {
        self.request(
            reqwest::Method::GET,
            &format!("/api/v1/channels/{channel}/messages?limit=50&before={before}"),
            None,
        )
        .await
    }

    pub async fn update_profile(&self, display_name: &str) -> Result<User, String> {
        self.request(
            reqwest::Method::PATCH,
            "/api/v1/users/@me",
            Some(serde_json::json!({ "display_name": display_name })),
        )
        .await
    }

    pub async fn create_guild(&self, name: &str) -> Result<Guild, String> {
        self.request(
            reqwest::Method::POST,
            "/api/v1/guilds",
            Some(serde_json::json!({ "name": name })),
        )
        .await
    }

    pub async fn create_channel(
        &self,
        guild: i64,
        name: &str,
        kind: i16,
    ) -> Result<Channel, String> {
        self.request(
            reqwest::Method::POST,
            &format!("/api/v1/guilds/{guild}/channels"),
            Some(serde_json::json!({ "name": name, "kind": kind })),
        )
        .await
    }

    pub async fn update_channel(
        &self,
        channel: i64,
        patch: serde_json::Value,
    ) -> Result<(), String> {
        self.call(
            reqwest::Method::PATCH,
            &format!("/api/v1/channels/{channel}"),
            Some(patch),
        )
        .await
    }

    pub async fn create_invite(&self, guild: i64) -> Result<Invite, String> {
        self.request(
            reqwest::Method::POST,
            &format!("/api/v1/guilds/{guild}/invites"),
            Some(serde_json::json!({})),
        )
        .await
    }

    pub async fn join_invite(&self, code: &str) -> Result<(), String> {
        // Nur Buchstaben und Ziffern in den Pfad lassen.
        let code: String = code
            .trim()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        if code.is_empty() {
            return Err("Einladungscode fehlt.".into());
        }
        self.call(
            reqwest::Method::POST,
            &format!("/api/v1/invites/{code}"),
            Some(serde_json::json!({})),
        )
        .await
    }

    // ------------------------------------------------------ Direktnachrichten

    pub async fn dms(&self) -> Result<Vec<DmChannel>, String> {
        self.request(reqwest::Method::GET, "/api/v1/users/@me/dms", None)
            .await
    }

    /// DM mit jemandem über den Benutzernamen öffnen (legt ihn bei Bedarf an).
    pub async fn open_dm(&self, handle: &str) -> Result<DmChannel, String> {
        let handle = handle.trim().trim_start_matches('@');
        if handle.is_empty() {
            return Err("Bitte einen Benutzernamen eingeben.".into());
        }
        self.request(
            reqwest::Method::POST,
            "/api/v1/users/@me/dms",
            Some(serde_json::json!({ "handle": handle })),
        )
        .await
        .map_err(|err| match err.as_str() {
            "not found" => format!("Niemanden mit dem Namen „{handle}“ gefunden."),
            "you cannot message yourself" => "Dir selbst kannst du nicht schreiben.".into(),
            _ => err,
        })
    }

    pub async fn dm_messages(&self, dm: i64, before: Option<i64>) -> Result<Vec<Message>, String> {
        let before = before.map(|id| format!("&before={id}")).unwrap_or_default();
        self.request(
            reqwest::Method::GET,
            &format!("/api/v1/dms/{dm}/messages?limit=50{before}"),
            None,
        )
        .await
    }

    pub async fn send_dm(&self, dm: i64, content: &str) -> Result<Message, String> {
        self.request(
            reqwest::Method::POST,
            &format!("/api/v1/dms/{dm}/messages"),
            Some(serde_json::json!({ "content": content })),
        )
        .await
    }

    pub async fn pins(&self, channel: i64) -> Result<Vec<Message>, String> {
        self.request(
            reqwest::Method::GET,
            &format!("/api/v1/channels/{channel}/pins"),
            None,
        )
        .await
    }

    pub async fn set_pin(&self, channel: i64, message: i64, pinned: bool) -> Result<(), String> {
        let path = format!("/api/v1/channels/{channel}/pins/{message}");
        if pinned {
            self.call(reqwest::Method::PUT, &path, Some(serde_json::json!({})))
                .await
        } else {
            self.call(reqwest::Method::DELETE, &path, None).await
        }
    }

    pub async fn create_thread(
        &self,
        channel: i64,
        message: i64,
        name: &str,
    ) -> Result<Channel, String> {
        self.request(
            reqwest::Method::POST,
            &format!("/api/v1/channels/{channel}/messages/{message}/threads"),
            Some(serde_json::json!({ "name": name })),
        )
        .await
    }

    pub async fn kick(&self, guild: i64, user: i64) -> Result<(), String> {
        self.call(
            reqwest::Method::POST,
            &format!("/api/v1/guilds/{guild}/members/{user}/kick"),
            Some(serde_json::json!({ "reason": "" })),
        )
        .await
    }

    pub async fn ban(&self, guild: i64, user: i64) -> Result<(), String> {
        self.call(
            reqwest::Method::PUT,
            &format!("/api/v1/guilds/{guild}/bans/{user}"),
            Some(serde_json::json!({ "reason": "" })),
        )
        .await
    }

    /// `seconds = 0` hebt einen Timeout auf.
    pub async fn timeout(&self, guild: i64, user: i64, seconds: i64) -> Result<(), String> {
        self.call(
            reqwest::Method::PUT,
            &format!("/api/v1/guilds/{guild}/members/{user}/timeout"),
            Some(serde_json::json!({ "seconds": seconds })),
        )
        .await
    }

    pub async fn set_member_role(
        &self,
        guild: i64,
        user: i64,
        role: i64,
        on: bool,
    ) -> Result<(), String> {
        let path = format!("/api/v1/guilds/{guild}/members/{user}/roles/{role}");
        if on {
            self.call(reqwest::Method::PUT, &path, Some(serde_json::json!({})))
                .await
        } else {
            self.call(reqwest::Method::DELETE, &path, None).await
        }
    }

    /// SDP-Offer (JSON von str0m) senden, Answer (JSON) zurück.
    pub async fn voice_join(&self, channel: i64, offer_json: &str) -> Result<String, String> {
        #[derive(Deserialize)]
        struct Answer {
            sdp: String,
        }
        let answer: Answer = self
            .request(
                reqwest::Method::POST,
                &format!("/api/v1/channels/{channel}/voice/join"),
                Some(serde_json::json!({ "sdp": offer_json })),
            )
            .await?;
        Ok(answer.sdp)
    }

    pub async fn voice_leave(&self, channel: i64) -> Result<(), String> {
        let _: serde_json::Value = self
            .request(
                reqwest::Method::POST,
                &format!("/api/v1/channels/{channel}/voice/leave"),
                Some(serde_json::json!({})),
            )
            .await?;
        Ok(())
    }

    pub async fn voice_state(
        &self,
        channel: i64,
        muted: bool,
        deafened: bool,
        screen: bool,
    ) -> Result<(), String> {
        let _: serde_json::Value = self
            .request(
                reqwest::Method::POST,
                &format!("/api/v1/channels/{channel}/voice/state"),
                Some(serde_json::json!({ "muted": muted, "deafened": deafened, "screen": screen })),
            )
            .await?;
        Ok(())
    }

    pub async fn voice_participants(&self, channel: i64) -> Result<Vec<VoiceParticipant>, String> {
        self.request(
            reqwest::Method::GET,
            &format!("/api/v1/channels/{channel}/voice/participants"),
            None,
        )
        .await
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoiceParticipant {
    #[serde(deserialize_with = "id")]
    pub user_id: i64,
    pub display_name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub deafened: bool,
    /// Teilt gerade den Bildschirm.
    #[serde(default)]
    pub screen: bool,
}

// ---------------------------------------------------------------------------
// Sitzung auf der Platte
// ---------------------------------------------------------------------------

fn session_file(base: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let host: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    Some(dir.join("nova").join(format!("session-{host}")))
}

fn load_refresh(base: &str) -> Option<String> {
    let text = std::fs::read_to_string(session_file(base)?).ok()?;
    let token = text.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn save_refresh(base: &str, token: Option<&str>) {
    let Some(path) = session_file(base) else {
        return;
    };
    match token {
        Some(token) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, token);
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}
