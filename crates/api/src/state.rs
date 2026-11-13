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

pub const REFRESH_COOKIE: &str = "nova_rt";
pub const ACCESS_COOKIE: &str = "nova_at";
pub const REFRESH_PATH: &str = "/api/v1/auth";
pub const ACCESS_PATH: &str = "/api/v1";

pub struct Auth {
    pub user_id: i64,
    pub session_id: String,
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let bearer = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
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
