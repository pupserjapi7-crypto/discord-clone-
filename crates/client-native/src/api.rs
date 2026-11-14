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

#[derive(Debug, Clone, Deserialize)]
pub struct Guild {
    #[serde(deserialize_with = "id")]
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Channel {
    #[serde(deserialize_with = "id")]
    pub id: i64,
    pub name: String,
    pub kind: i16,
    #[serde(default)]
    pub topic: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Member {
    pub display_name: String,
    #[serde(default)]
    pub status: i16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GuildDetail {
    pub guild: Guild,
    #[serde(default)]
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    #[serde(deserialize_with = "id")]
    pub id: i64,
    pub author: User,
    pub content: String,
    #[serde(default)]
    pub created_at: String,
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
