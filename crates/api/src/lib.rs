//! HTTP API: auth, users, guilds, channels, message history, health.

pub mod blob;
mod bots;
mod channels;
mod dms;
pub mod error;
mod guard;
mod media;
mod moderation;
pub mod state;
pub mod uploads;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    routing::{delete, get, post, put},
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use nova_common::{
    config::Config,
    permissions::Permissions,
    protocol::{GuildSummary, MessageView, UserPublic},
    validate,
};
use nova_db::{
    Db,
    models::{Channel, Guild, MemberRow, Role},
    views,
};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use error::{ApiError, ApiResult, unique_conflict};
pub use state::AppState;
use state::{
    ACCESS_COOKIE, ACCESS_PATH, Auth, REFRESH_COOKIE, REFRESH_PATH, clear_cookie, client_ip, cookie,
};

pub fn router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(origin_header(&state.cfg.public_origin))
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    Router::new()
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/users/@me", get(me).patch(update_me))
        .route(
            "/api/v1/users/@me/dms",
            get(dms::list_dms).post(dms::open_dm),
        )
        .route(
            "/api/v1/dms/{dm_id}/messages",
            get(dms::list_dm_messages).post(dms::create_dm_message),
        )
        .route(
            "/api/v1/dms/{dm_id}/attachments",
            post(uploads::upload_to_dm),
        )
        .route("/api/v1/guilds", get(list_guilds).post(create_guild))
        .route("/api/v1/guilds/{guild_id}", get(get_guild))
        .route("/api/v1/guilds/{guild_id}/channels", post(create_channel))
        .route("/api/v1/guilds/{guild_id}/invites", post(create_invite))
        .route("/api/v1/invites/{code}", post(join_invite))
        // Moderation, Rollen, Audit-Log
        .route(
            "/api/v1/guilds/{guild_id}/members/{user_id}/kick",
            post(moderation::kick),
        )
        .route(
            "/api/v1/guilds/{guild_id}/members/{user_id}/timeout",
            put(moderation::timeout),
        )
        .route(
            "/api/v1/guilds/{guild_id}/members/{user_id}/roles/{role_id}",
            put(moderation::add_member_role).delete(moderation::remove_member_role),
        )
        .route("/api/v1/guilds/{guild_id}/bans", get(moderation::list_bans))
        .route(
            "/api/v1/guilds/{guild_id}/bans/{user_id}",
            put(moderation::ban).delete(moderation::unban),
        )
        .route(
            "/api/v1/guilds/{guild_id}/audit-log",
            get(moderation::audit_log),
        )
        .route(
            "/api/v1/guilds/{guild_id}/roles",
            post(moderation::create_role),
        )
        .route(
            "/api/v1/guilds/{guild_id}/roles/{role_id}",
            axum::routing::patch(moderation::update_role).delete(moderation::delete_role),
        )
        .route(
            "/api/v1/guilds/{guild_id}/bots/{bot_id}",
            put(bots::add_to_guild),
        )
        // Kanäle, Nachrichten, Pins, Threads
        .route(
            "/api/v1/channels/{channel_id}",
            axum::routing::patch(channels::update_channel).delete(channels::delete_channel),
        )
        .route(
            "/api/v1/channels/{channel_id}/overrides/{kind}/{target_id}",
            put(moderation::set_override).delete(moderation::delete_override),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages",
            get(list_messages).post(channels::create_message),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}",
            delete(channels::delete_message),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/threads",
            post(channels::create_thread),
        )
        .route(
            "/api/v1/channels/{channel_id}/pins",
            get(channels::list_pins),
        )
        .route(
            "/api/v1/channels/{channel_id}/pins/{message_id}",
            put(channels::pin).delete(channels::unpin),
        )
        .route("/api/v1/channels/{channel_id}/ack", post(channels::ack))
        // Anhänge
        .route(
            "/api/v1/channels/{channel_id}/attachments",
            post(uploads::upload_to_channel),
        )
        .route(
            "/api/v1/attachments/{attachment_id}",
            get(uploads::download).delete(uploads::discard),
        )
        // Bots
        .route("/api/v1/bots", get(bots::list_bots).post(bots::create_bot))
        .route("/api/v1/bots/{bot_id}/token", post(bots::reset_token))
        // Voice
        .route("/api/v1/channels/{channel_id}/voice/join", post(voice_join))
        .route(
            "/api/v1/channels/{channel_id}/voice/state",
            post(voice_state),
        )
        .route(
            "/api/v1/channels/{channel_id}/voice/leave",
            post(voice_leave),
        )
        .route(
            "/api/v1/channels/{channel_id}/voice/participants",
            get(voice_participants),
        )
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// JavaScript numbers lose precision above 2^53; every `id` / `*_id` in JSON
/// responses is therefore a string. The MessagePack gateway keeps i64 IDs.
fn stringify_ids(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let is_id = key == "id" || key.ends_with("_id");
                    // Listen von IDs (`role_ids`, …) ebenfalls als Strings.
                    let is_id_list = key.ends_with("_ids");
                    let value = match value {
                        Value::Number(number) if is_id => Value::String(number.to_string()),
                        Value::Array(items) if is_id_list => Value::Array(
                            items
                                .into_iter()
                                .map(|item| match item {
                                    Value::Number(number) => Value::String(number.to_string()),
                                    other => other,
                                })
                                .collect(),
                        ),
                        other if is_id => other,
                        other => stringify_ids(other),
                    };
                    (key, value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(stringify_ids).collect()),
        other => other,
    }
}

pub(crate) fn json_ids<T: Serialize>(value: &T) -> Json<Value> {
    Json(stringify_ids(
        serde_json::to_value(value).unwrap_or(Value::Null),
    ))
}

fn origin_header(origin: &str) -> HeaderValue {
    origin
        .parse::<HeaderValue>()
        .unwrap_or_else(|_| HeaderValue::from_static("http://localhost:5173"))
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    handle: String,
    display_name: Option<String>,
    password: String,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    handle: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct AuthResponse {
    user: UserPublic,
    access_token: String,
    expires_in: u64,
}

impl AuthResponse {
    fn json(self) -> Json<Value> {
        json_ids(&self)
    }
}

struct SessionTokens {
    access: String,
    refresh: String,
}

/// Run Argon2 only through this gate: one permit per configured slot.
async fn hash_password_limited(state: &AppState, password: String) -> ApiResult<String> {
    let permit = Arc::clone(&state.hash_limit)
        .acquire_owned()
        .await
        .map_err(ApiError::internal)?;
    let cfg = Arc::clone(&state.cfg);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        hash_password(&password, &cfg)
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::from)
}

async fn verify_password_limited(
    state: &AppState,
    password: String,
    hash: String,
) -> ApiResult<bool> {
    let permit = Arc::clone(&state.hash_limit)
        .acquire_owned()
        .await
        .map_err(ApiError::internal)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        verify_password(&password, &hash)
    })
    .await
    .map_err(ApiError::internal)
}

fn hash_password(password: &str, cfg: &Config) -> anyhow::Result<String> {
    let params = Params::new(
        cfg.argon2_m_cost,
        cfg.argon2_t_cost,
        cfg.argon2_p_cost,
        Some(32),
    )
    .map_err(|err| anyhow::anyhow!("invalid argon2 params: {err}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = SaltString::generate(&mut OsRng);
    argon
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| anyhow::anyhow!("hash password: {err}"))
}

fn verify_password(password: &str, encoded: &str) -> bool {
    match PasswordHash::new(encoded) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

async fn start_session(
    state: &AppState,
    user_id: i64,
    user_agent: Option<&str>,
) -> ApiResult<SessionTokens> {
    let session_id = state.ids.next_id();
    let refresh = random_hex(32);
    let hash = sha256(refresh.as_bytes());
    let expires = Utc::now()
        + ChronoDuration::from_std(state.cfg.refresh_ttl).unwrap_or(ChronoDuration::days(30));
    state
        .db
        .create_session(
            session_id,
            user_id,
            &random_hex(8),
            &hash,
            expires,
            user_agent,
        )
        .await?;
    let access = state.tokens.issue(user_id, &session_id.to_string());
    Ok(SessionTokens { access, refresh })
}

fn session_headers(state: &AppState, access: &str, refresh: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let secure = state.cfg.secure_cookies();
    headers.append(
        header::SET_COOKIE,
        cookie(
            ACCESS_COOKIE,
            access,
            ACCESS_PATH,
            state.cfg.access_ttl,
            secure,
        ),
    );
    headers.append(
        header::SET_COOKIE,
        cookie(
            REFRESH_COOKIE,
            refresh,
            REFRESH_PATH,
            state.cfg.refresh_ttl,
            secure,
        ),
    );
    headers
}

async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<(HeaderMap, Json<Value>)> {
    let ip = client_ip(&headers, peer.ip(), state.cfg.trust_proxy);
    if !state.auth_limiter.allow(
        format!("register:{ip}"),
        state.cfg.auth_burst,
        state.cfg.auth_rate,
    ) {
        return Err(ApiError::RateLimited);
    }

    let handle = validate::normalize_handle(&req.handle);
    validate::validate_handle(&handle).map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    validate::validate_password(&req.password)
        .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    let display_name = req
        .display_name
        .unwrap_or_else(|| handle.clone())
        .trim()
        .to_owned();
    validate::validate_display_name(&display_name)
        .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;

    let hash = hash_password_limited(&state, req.password).await?;

    let user_id = state.ids.next_id();
    let user = state
        .db
        .create_user(user_id, &handle, &display_name, &hash)
        .await
        .map_err(|err| unique_conflict(err, "handle is already taken"))?;

    let tokens = start_session(&state, user_id, user_agent(&headers)).await?;
    let body = AuthResponse {
        user: views::user_public(&state.cfg.public_origin, &user),
        access_token: tokens.access.clone(),
        expires_in: state.cfg.access_ttl.as_secs(),
    };
    Ok((
        session_headers(&state, &tokens.access, &tokens.refresh),
        body.json(),
    ))
}

async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> ApiResult<(HeaderMap, Json<Value>)> {
    let ip = client_ip(&headers, peer.ip(), state.cfg.trust_proxy);
    if !state.auth_limiter.allow(
        format!("login:{ip}"),
        state.cfg.auth_burst,
        state.cfg.auth_rate,
    ) {
        return Err(ApiError::RateLimited);
    }

    let handle = validate::normalize_handle(&req.handle);
    let Some((user, hash)) = state.db.user_with_hash(&handle).await? else {
        state.metrics.auth_failures.inc();
        // Same response as a wrong password: no account enumeration.
        return Err(ApiError::Unauthorized);
    };

    let valid = verify_password_limited(&state, req.password, hash).await?;
    if !valid {
        state.metrics.auth_failures.inc();
        return Err(ApiError::Unauthorized);
    }

    let tokens = start_session(&state, user.id, user_agent(&headers)).await?;
    let body = AuthResponse {
        user: views::user_public(&state.cfg.public_origin, &user),
        access_token: tokens.access.clone(),
        expires_in: state.cfg.access_ttl.as_secs(),
    };
    Ok((
        session_headers(&state, &tokens.access, &tokens.refresh),
        body.json(),
    ))
}

async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<(HeaderMap, Json<Value>)> {
    let raw = state::cookie_value(&headers, REFRESH_COOKIE).ok_or(ApiError::Unauthorized)?;
    let hash = sha256(raw.as_bytes());
    let Some(session) = state.db.session_by_hash(&hash).await? else {
        return Err(ApiError::Unauthorized);
    };

    // Reuse of a rotated token means the cookie leaked: drop the whole family.
    if session.revoked_at.is_some() {
        state.db.revoke_family(&session.family_id).await?;
        return Err(ApiError::Unauthorized);
    }
    if session.expires_at <= Utc::now() {
        return Err(ApiError::Unauthorized);
    }

    let new_id = state.ids.next_id();
    let new_refresh = random_hex(32);
    let new_hash = sha256(new_refresh.as_bytes());
    let expires = Utc::now()
        + ChronoDuration::from_std(state.cfg.refresh_ttl).unwrap_or(ChronoDuration::days(30));
    state
        .db
        .rotate_session(session.id, new_id, &new_hash, expires)
        .await?;

    let user = state
        .db
        .user_by_id(session.user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let access = state.tokens.issue(user.id, &new_id.to_string());
    let body = AuthResponse {
        user: views::user_public(&state.cfg.public_origin, &user),
        access_token: access.clone(),
        expires_in: state.cfg.access_ttl.as_secs(),
    };
    Ok((session_headers(&state, &access, &new_refresh), body.json()))
}

async fn logout(State(state): State<AppState>, auth: Auth) -> ApiResult<HeaderMap> {
    if let Ok(id) = auth.session_id.parse::<i64>() {
        state.db.revoke_session(id).await?;
    }
    state.tokens.revoke_session(&auth.session_id);
    let mut headers = HeaderMap::new();
    headers.append(header::SET_COOKIE, clear_cookie(ACCESS_COOKIE, ACCESS_PATH));
    headers.append(
        header::SET_COOKIE,
        clear_cookie(REFRESH_COOKIE, REFRESH_PATH),
    );
    Ok(headers)
}

fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

async fn me(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let user = state
        .db
        .user_by_id(auth.user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    Ok(json_ids(&views::user_public(
        &state.cfg.public_origin,
        &user,
    )))
}

#[derive(Debug, Deserialize)]
struct UpdateProfileRequest {
    display_name: Option<String>,
    status: Option<i16>,
}

async fn update_me(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<UpdateProfileRequest>,
) -> ApiResult<Json<Value>> {
    if let Some(name) = &req.display_name {
        validate::validate_display_name(name)
            .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    }
    if let Some(status) = req.status
        && !(0..=3).contains(&status)
    {
        return Err(ApiError::BadRequest("invalid status".into()));
    }
    let user = state
        .db
        .update_profile(auth.user_id, req.display_name.as_deref(), req.status)
        .await?;
    state.gateway.broadcast_presence(user.id, user.status);
    Ok(json_ids(&views::user_public(
        &state.cfg.public_origin,
        &user,
    )))
}

// ---------------------------------------------------------------------------
// Guilds and channels
// ---------------------------------------------------------------------------

async fn list_guilds(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let guilds = state.db.guilds_for_user(auth.user_id).await?;
    let summaries: Vec<GuildSummary> = guilds
        .into_iter()
        .map(|guild| GuildSummary {
            id: guild.id,
            name: guild.name,
        })
        .collect();
    Ok(json_ids(&summaries))
}

#[derive(Debug, Deserialize)]
struct CreateGuildRequest {
    name: String,
}

async fn create_guild(
    State(state): State<AppState>,
    auth: Auth,
    Json(req): Json<CreateGuildRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate::validate_guild_name(&req.name).map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    let guild = state
        .db
        .create_guild(
            state.ids.next_id(),
            req.name.trim(),
            auth.user_id,
            state.ids.next_id(),
            state.ids.next_id(),
            state.ids.next_id(),
        )
        .await?;
    Ok((StatusCode::CREATED, json_ids(&guild)))
}

#[derive(Debug, Serialize)]
struct MemberView {
    user_id: i64,
    handle: String,
    display_name: String,
    status: i16,
    avatar_url: Option<String>,
    nickname: Option<String>,
    /// Ab v0.4: Rollen-IDs (ohne @everyone), Timeout, Bot-Kennzeichen.
    role_ids: Vec<i64>,
    timeout_until: Option<DateTime<Utc>>,
    bot: bool,
}

impl MemberView {
    fn from_row(base_url: &str, row: MemberRow) -> Self {
        let timeout_until = row.timeout_until.filter(|until| *until > Utc::now());
        Self {
            user_id: row.user_id,
            handle: row.handle,
            display_name: row.nickname.unwrap_or(row.display_name),
            status: row.status,
            avatar_url: row.avatar_id.map(|id| views::attachment_url(base_url, id)),
            nickname: None,
            role_ids: row.role_ids,
            timeout_until,
            bot: row.is_bot,
        }
    }
}

#[derive(Debug, Serialize)]
struct GuildDetail {
    guild: Guild,
    channels: Vec<Channel>,
    roles: Vec<Role>,
    members: Vec<MemberView>,
    permissions: u64,
    /// Ab v0.4: letzte Nachricht und Lesestand je sichtbarem Kanal.
    unread: Vec<nova_db::models::UnreadRow>,
}

async fn get_guild(
    State(state): State<AppState>,
    auth: Auth,
    Path(guild_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let Some(guild) = state.db.guild(guild_id).await? else {
        return Err(ApiError::NotFound);
    };
    if !state.db.is_member(guild_id, auth.user_id).await? {
        return Err(ApiError::NotFound);
    }

    let base_perms = state
        .db
        .effective_permissions(guild_id, auth.user_id, None)
        .await?;
    if !base_perms.contains(Permissions::VIEW_CHANNEL) {
        return Err(ApiError::Forbidden);
    }

    let mut visible = Vec::new();
    for channel in state.db.channels(guild_id).await? {
        let perms = state
            .db
            .effective_permissions(guild_id, auth.user_id, Some(channel.id))
            .await?;
        if perms.contains(Permissions::VIEW_CHANNEL) {
            visible.push(channel);
        }
    }

    let roles = state.db.roles(guild_id).await?;
    let members = state
        .db
        .members(guild_id)
        .await?
        .into_iter()
        .map(|row| MemberView::from_row(&state.cfg.public_origin, row))
        .collect();
    let text_ids: Vec<i64> = visible
        .iter()
        .filter(|channel| channel.kind != 1)
        .map(|channel| channel.id)
        .collect();
    let unread = state.db.unread_state(auth.user_id, &text_ids).await?;

    let detail = GuildDetail {
        guild,
        channels: visible,
        roles,
        members,
        permissions: base_perms.bits(),
        unread,
    };
    Ok(json_ids(&detail))
}

#[derive(Debug, Deserialize)]
struct CreateChannelRequest {
    name: String,
    #[serde(default)]
    kind: i16,
}

async fn create_channel(
    State(state): State<AppState>,
    auth: Auth,
    Path(guild_id): Path<i64>,
    Json(req): Json<CreateChannelRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate::validate_channel_name(&req.name)
        .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    if req.kind != 0 && req.kind != 1 {
        return Err(ApiError::BadRequest(
            "kind must be 0 (text) or 1 (voice)".into(),
        ));
    }
    if !state.db.is_member(guild_id, auth.user_id).await? {
        return Err(ApiError::NotFound);
    }
    let perms = state
        .db
        .effective_permissions(guild_id, auth.user_id, None)
        .await?;
    if !perms.contains(Permissions::MANAGE_CHANNELS) {
        return Err(ApiError::Forbidden);
    }

    let channel = state
        .db
        .create_channel(
            state.ids.next_id(),
            guild_id,
            req.name.trim(),
            req.kind,
            None,
        )
        .await?;
    state.gateway.publish_guild(
        guild_id,
        &nova_common::protocol::Event::GuildUpdate { guild_id },
    );
    guard::audit(
        &state,
        guild_id,
        auth.user_id,
        "channel_create",
        Some(channel.id),
        None,
        Some(&channel.name),
    )
    .await;
    Ok((StatusCode::CREATED, json_ids(&channel)))
}

#[derive(Debug, Serialize)]
struct InviteResponse {
    code: String,
}

async fn create_invite(
    State(state): State<AppState>,
    auth: Auth,
    Path(guild_id): Path<i64>,
) -> ApiResult<(StatusCode, Json<InviteResponse>)> {
    if !state.db.is_member(guild_id, auth.user_id).await? {
        return Err(ApiError::NotFound);
    }
    let perms = state
        .db
        .effective_permissions(guild_id, auth.user_id, None)
        .await?;
    if !perms.contains(Permissions::CREATE_INVITE) {
        return Err(ApiError::Forbidden);
    }
    let code = random_hex(5);
    state
        .db
        .create_invite(&code, guild_id, auth.user_id)
        .await?;
    Ok((StatusCode::CREATED, Json(InviteResponse { code })))
}

#[derive(Debug, Serialize)]
struct JoinResponse {
    guild_id: i64,
}

async fn join_invite(
    State(state): State<AppState>,
    auth: Auth,
    Path(code): Path<String>,
) -> ApiResult<Json<Value>> {
    match state.db.accept_invite(&code, auth.user_id).await? {
        Some(guild_id) => {
            state.gateway.member_added(guild_id, auth.user_id);
            state.gateway.publish_guild(
                guild_id,
                &nova_common::protocol::Event::GuildUpdate { guild_id },
            );
            Ok(json_ids(&JoinResponse { guild_id }))
        }
        None => Err(ApiError::NotFound),
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct MessageQuery {
    before: Option<i64>,
    limit: Option<i64>,
}

async fn list_messages(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
    Query(query): Query<MessageQuery>,
) -> ApiResult<Json<Value>> {
    let Some(guild_id) = state.db.channel_guild(channel_id).await? else {
        return Err(ApiError::NotFound);
    };
    if !state.db.is_member(guild_id, auth.user_id).await? {
        return Err(ApiError::NotFound);
    }
    let perms = state
        .db
        .effective_permissions(guild_id, auth.user_id, Some(channel_id))
        .await?;
    if !perms.contains(Permissions::VIEW_CHANNEL) {
        return Err(ApiError::NotFound);
    }
    if !perms.contains(Permissions::READ_MESSAGE_HISTORY) {
        return Err(ApiError::Forbidden);
    }

    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let mut rows = state
        .db
        .list_messages(channel_id, query.before, limit)
        .await?;
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let reactions = state.db.reactions_for(&ids, auth.user_id).await?;
    let attachments = state.db.attachments_for(&ids).await?;

    // Page is fetched newest-first for the index; the UI needs oldest-first.
    rows.reverse();
    let mut reactions = reactions;
    let mut attachments = attachments;
    let base = &state.cfg.public_origin;
    let views: Vec<MessageView> = rows
        .into_iter()
        .map(|row| {
            let msg_reactions: Vec<_> = reactions
                .iter()
                .filter(|r| r.message_id == row.id)
                .cloned()
                .collect();
            let msg_attachments: Vec<_> = attachments
                .iter()
                .filter(|a| a.message_id == Some(row.id))
                .cloned()
                .collect();
            reactions.retain(|r| r.message_id != row.id);
            attachments.retain(|a| a.message_id != Some(row.id));
            views::message_view(base, row, msg_reactions, msg_attachments)
        })
        .collect();

    Ok(json_ids(&views))
}

// ---------------------------------------------------------------------------
// Voice signaling
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct VoiceJoinRequest {
    sdp: String,
}

#[derive(Debug, Serialize)]
struct VoiceParticipantView {
    user_id: i64,
    handle: String,
    display_name: String,
    muted: bool,
    screen: bool,
    deafened: bool,
    /// Ab v0.8.
    camera: bool,
}

async fn voice_channel(
    state: &AppState,
    user_id: i64,
    channel_id: i64,
) -> ApiResult<(nova_db::models::Channel, Permissions)> {
    guard::visible_channel(state, channel_id, user_id).await
}

fn voice_event(
    channel: &nova_db::models::Channel,
    user_id: i64,
    participant: Option<&nova_sfu::Participant>,
) -> nova_common::protocol::Event {
    nova_common::protocol::Event::VoiceStateUpdate {
        guild_id: channel.guild_id,
        channel_id: channel.id,
        user_id,
        connected: participant.is_some(),
        muted: participant.is_some_and(|p| p.muted),
        deafened: participant.is_some_and(|p| p.deafened),
        screen: participant.is_some_and(|p| p.screen),
        camera: participant.is_some_and(|p| p.camera),
    }
}

async fn voice_join(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
    Json(req): Json<VoiceJoinRequest>,
) -> ApiResult<Json<Value>> {
    let (channel, perms) = voice_channel(&state, auth.user_id, channel_id).await?;
    if channel.kind != 1 {
        return Err(ApiError::BadRequest(
            "channel is not a voice channel".into(),
        ));
    }
    // CONNECT fehlt auch während eines Timeouts.
    if !perms.contains(Permissions::CONNECT) {
        return Err(ApiError::Forbidden);
    }
    if req.sdp.len() > 64 * 1024 {
        return Err(ApiError::TooLarge);
    }
    let user = state
        .db
        .user_by_id(auth.user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let public = views::user_public(&state.cfg.public_origin, &user);
    let answer = state
        .sfu
        .join(channel_id, &public, &req.sdp)
        .map_err(|err| match err {
            nova_sfu::VoiceError::Full => ApiError::Conflict("voice channel is full".into()),
            nova_sfu::VoiceError::Disabled => {
                ApiError::BadRequest("voice is disabled on this server".into())
            }
            nova_sfu::VoiceError::InvalidOffer(reason) => {
                ApiError::BadRequest(format!("invalid SDP: {reason}"))
            }
            nova_sfu::VoiceError::Unavailable => {
                ApiError::Internal(anyhow::anyhow!("SFU unavailable"))
            }
        })?;
    let participant = state
        .sfu
        .participants(channel_id)
        .into_iter()
        .find(|p| p.user_id == auth.user_id);
    state.gateway.publish_guild(
        channel.guild_id,
        &voice_event(&channel, auth.user_id, participant.as_ref()),
    );
    Ok(Json(json!({ "sdp": answer })))
}

async fn voice_leave(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
) -> ApiResult<StatusCode> {
    let (channel, _) = voice_channel(&state, auth.user_id, channel_id).await?;
    state.sfu.leave(channel_id, auth.user_id);
    state
        .gateway
        .publish_guild(channel.guild_id, &voice_event(&channel, auth.user_id, None));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct VoiceStateRequest {
    muted: bool,
    #[serde(default)]
    deafened: bool,
    #[serde(default)]
    screen: bool,
    /// Ab v0.8; ältere Clients senden es nicht (dann bleibt die Kamera aus).
    #[serde(default)]
    camera: bool,
}

/// Der Client meldet Stumm/Taub/Bildschirm; alle Mitglieder sehen es sofort.
async fn voice_state(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
    Json(req): Json<VoiceStateRequest>,
) -> ApiResult<StatusCode> {
    let (channel, _) = voice_channel(&state, auth.user_id, channel_id).await?;
    let Some(participant) = state.sfu.set_state(
        channel_id,
        auth.user_id,
        req.muted,
        req.deafened,
        req.screen,
        req.camera,
    ) else {
        return Err(ApiError::NotFound);
    };
    state.gateway.publish_guild(
        channel.guild_id,
        &voice_event(&channel, auth.user_id, Some(&participant)),
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn voice_participants(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    voice_channel(&state, auth.user_id, channel_id).await?;
    let participants: Vec<VoiceParticipantView> = state
        .sfu
        .participants(channel_id)
        .into_iter()
        .map(|participant| VoiceParticipantView {
            user_id: participant.user_id,
            handle: participant.handle,
            display_name: participant.display_name,
            muted: participant.muted,
            screen: participant.screen,
            deafened: participant.deafened,
            camera: participant.camera,
        })
        .collect();
    Ok(json_ids(&participants))
}

/// Was der SFU selbst bemerkt (Videospur kam an oder ist weg, Verbindung
/// abgerissen), geht als `voice_state_update` an die Guild. Ohne das sahen
/// die anderen eine Bildschirmfreigabe erst nach dem nächsten Statusbericht
/// und abgerissene Verbindungen gar nicht.
pub fn spawn_sfu_events(state: AppState) {
    let Some(events) = state.sfu.take_events() else {
        return;
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    // Der SFU meldet über einen std-Kanal (seine Räume sind OS-Threads).
    let spawned = std::thread::Builder::new()
        .name("sfu-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                if tx.send(event).is_err() {
                    break;
                }
            }
        });
    if spawned.is_err() {
        tracing::warn!("SFU events thread could not start");
        return;
    }
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let Ok(Some(channel)) = state.db.channel(event.channel_id).await else {
                continue;
            };
            state.gateway.publish_guild(
                channel.guild_id,
                &voice_event(&channel, event.user_id, event.participant.as_ref()),
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Ops
// ---------------------------------------------------------------------------

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let started = std::time::Instant::now();
    state.db.ping().await?;
    Ok(Json(json!({
        "status": "ready",
        "db_ms": started.elapsed().as_millis() as u64,
    })))
}

async fn metrics(State(state): State<AppState>) -> String {
    state.metrics.render(
        nova_common::metrics::process_rss_bytes(),
        state.started.elapsed().as_secs(),
    )
}

#[allow(dead_code)]
fn _assert_state_is_send_sync() {
    fn check<T: Send + Sync>() {}
    check::<AppState>();
    check::<Db>();
    check::<Arc<Config>>();
    check::<DateTime<Utc>>();
    check::<Duration>();
}
