use std::{
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, HeaderValue, header, request::Parts},
};
use dashmap::DashMap;
use nova_common::{config::Config, ids::Snowflake, metrics::Metrics, tokens::TokenStore};
use nova_db::Db;
use nova_gateway::Gateway;
use nova_sfu::Sfu;
use tokio::sync::Semaphore;

use crate::error::ApiError;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub cfg: Arc<Config>,
    pub tokens: Arc<TokenStore>,
    pub metrics: Arc<Metrics>,
    pub ids: Arc<Snowflake>,
    pub gateway: Arc<Gateway>,
    pub sfu: Arc<Sfu>,
    pub started: Instant,
    pub auth_limiter: Arc<RateLimiter>,
    /// Bounds concurrent Argon2 work: hashing is memory-hard and must never
    /// starve the gateway/SFU runtimes.
    pub hash_limit: Arc<Semaphore>,
}

impl AppState {
    /// Entfernt jemanden aus allen Sprachkanälen einer Guild (Kick, Bann,
    /// Timeout) und meldet das den übrigen Mitgliedern.
    pub async fn sfu_kick(&self, guild_id: i64, user_id: i64) {
        let Ok(channels) = self.db.channels(guild_id).await else {
            return;
        };
        for channel in channels.iter().filter(|channel| channel.kind == 1) {
            let present = self
                .sfu
                .participants(channel.id)
                .iter()
                .any(|participant| participant.user_id == user_id);
            if present {
                self.sfu.leave(channel.id, user_id);
                self.gateway.publish_guild(
                    guild_id,
                    &nova_common::protocol::Event::VoiceStateUpdate {
                        guild_id,
                        channel_id: channel.id,
                        user_id,
                        connected: false,
                        muted: false,
                        deafened: false,
                        screen: false,
                    },
                );
            }
        }
    }
}

pub const REFRESH_COOKIE: &str = "nova_rt";
pub const ACCESS_COOKIE: &str = "nova_at";
pub const REFRESH_PATH: &str = "/api/v1/auth";
pub const ACCESS_PATH: &str = "/api/v1";

pub struct Auth {
    pub user_id: i64,
    pub session_id: String,
    /// Anfrage kommt von einem Bot (`Authorization: Bot nb_…`).
    pub is_bot: bool,
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let authorization = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());

        // Bots: Token-Hash in der Datenbank. Kein Cookie-Fallback.
        if let Some(token) = authorization.and_then(|value| value.strip_prefix("Bot ")) {
            let token = token.trim();
            if !nova_common::tokens::is_bot_token(token) {
                return Err(ApiError::Unauthorized);
            }
            let hash = nova_common::tokens::hash_bot_token(token);
            let Some(bot_id) = state.db.bot_by_token(&hash).await? else {
                return Err(ApiError::Unauthorized);
            };
            return Ok(Self {
                user_id: bot_id,
                session_id: format!("bot:{bot_id}"),
                is_bot: true,
            });
        }

        let bearer = authorization
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|token| !token.is_empty());

        // Cookie auth is a fallback so plain `<img>`/download requests work.
        let token = bearer
            .map(str::to_owned)
            .or_else(|| cookie_value(&parts.headers, ACCESS_COOKIE));

        let Some(token) = token else {
            return Err(ApiError::Unauthorized);
        };
        let Some(entry) = state.tokens.validate(&token) else {
            return Err(ApiError::Unauthorized);
        };
        Ok(Self {
            user_id: entry.user_id,
            session_id: entry.session_id,
            is_bot: false,
        })
    }
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_owned())
    })
}

pub fn cookie(name: &str, value: &str, path: &str, max_age: Duration, secure: bool) -> HeaderValue {
    let mut cookie = format!(
        "{name}={value}; Path={path}; HttpOnly; SameSite=Lax; Max-Age={}",
        max_age.as_secs()
    );
    if secure {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).unwrap_or_else(|_| HeaderValue::from_static("nova=invalid"))
}

pub fn clear_cookie(name: &str, path: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{name}=; Path={path}; HttpOnly; SameSite=Lax; Max-Age=0"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("nova=invalid"))
}

/// Accepts `X-Forwarded-For` only when Caddy is explicitly trusted.
pub fn client_ip(headers: &HeaderMap, peer: IpAddr, trust_proxy: bool) -> IpAddr {
    if !trust_proxy {
        return peer;
    }
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(peer)
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

/// Simple token bucket keyed by an arbitrary string (IP, handle, user id).
#[derive(Debug, Default)]
pub struct RateLimiter {
    buckets: DashMap<String, Bucket>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow(&self, key: impl Into<String>, capacity: f64, per_second: f64) -> bool {
        let key = key.into();
        let now = Instant::now();
        let mut entry = self.buckets.entry(key).or_insert(Bucket {
            tokens: capacity,
            last: now,
        });
        let elapsed = now.duration_since(entry.last).as_secs_f64();
        entry.tokens = (entry.tokens + elapsed * per_second).min(capacity);
        entry.last = now;
        if entry.tokens >= 1.0 {
            entry.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Drop idle buckets; called from the janitor task.
    pub fn sweep(&self) {
        let now = Instant::now();
        self.buckets
            .retain(|_, bucket| now.duration_since(bucket.last) < Duration::from_secs(600));
    }
}
