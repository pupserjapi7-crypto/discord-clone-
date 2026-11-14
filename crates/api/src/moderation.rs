//! Moderation: Kick, Bann, Timeout, Rollen, Kanal-Overrides, Audit-Log.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{Duration as ChronoDuration, Utc};
use nova_common::{permissions::Permissions, protocol::Event, validate};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    AppState,
    error::{ApiError, ApiResult},
    guard::{audit, ensure_can_moderate, ensure_role_below, reason, require_guild_perm},
    json_ids,
    state::Auth,
};

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ReasonBody {
    reason: Option<String>,
}

pub(crate) async fn kick(
    State(state): State<AppState>,
    auth: Auth,
    Path((guild_id, user_id)): Path<(i64, i64)>,
    body: Option<Json<ReasonBody>>,
) -> ApiResult<StatusCode> {
    let body = body.map(|Json(body)| body).unwrap_or_default();
    let reason = reason(body.reason.as_deref())?;
    let (guild, _) =
        require_guild_perm(&state, guild_id, auth.user_id, Permissions::KICK_MEMBERS).await?;
    ensure_can_moderate(&state, &guild, auth.user_id, user_id).await?;
    if state.db.remove_member(guild_id, user_id).await? == 0 {
        return Err(ApiError::NotFound);
    }
    state.sfu_kick(guild_id, user_id).await;
    state.gateway.member_removed(guild_id, user_id);
    state
        .gateway
        .publish_guild(guild_id, &Event::GuildMemberRemove { guild_id, user_id });
    audit(
        &state,
        guild_id,
        auth.user_id,
        "member_kick",
        Some(user_id),
        reason.as_deref(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn ban(
    State(state): State<AppState>,
    auth: Auth,
    Path((guild_id, user_id)): Path<(i64, i64)>,
    body: Option<Json<ReasonBody>>,
) -> ApiResult<StatusCode> {
    let body = body.map(|Json(body)| body).unwrap_or_default();
    let reason = reason(body.reason.as_deref())?;
    let (guild, _) =
        require_guild_perm(&state, guild_id, auth.user_id, Permissions::BAN_MEMBERS).await?;
    ensure_can_moderate(&state, &guild, auth.user_id, user_id).await?;
    if state.db.user_by_id(user_id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    state
        .db
        .ban(guild_id, user_id, auth.user_id, reason.as_deref())
        .await?;
    state.sfu_kick(guild_id, user_id).await;
    state.gateway.member_removed(guild_id, user_id);
    state
        .gateway
        .publish_guild(guild_id, &Event::GuildMemberRemove { guild_id, user_id });
    audit(
        &state,
        guild_id,
        auth.user_id,
        "member_ban",
        Some(user_id),
        reason.as_deref(),
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn unban(
    State(state): State<AppState>,
    auth: Auth,
    Path((guild_id, user_id)): Path<(i64, i64)>,
) -> ApiResult<StatusCode> {
    require_guild_perm(&state, guild_id, auth.user_id, Permissions::BAN_MEMBERS).await?;
    if state.db.unban(guild_id, user_id).await? == 0 {
        return Err(ApiError::NotFound);
    }
    audit(
        &state,
        guild_id,
        auth.user_id,
        "member_unban",
        Some(user_id),
        None,
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_bans(
    State(state): State<AppState>,
    auth: Auth,
    Path(guild_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    require_guild_perm(&state, guild_id, auth.user_id, Permissions::BAN_MEMBERS).await?;
    Ok(json_ids(&state.db.bans(guild_id).await?))
}

#[derive(Debug, Deserialize)]
pub(crate) struct TimeoutBody {
    /// 0 hebt den Timeout auf.
    seconds: i64,
    reason: Option<String>,
}

pub(crate) async fn timeout(
    State(state): State<AppState>,
    auth: Auth,
    Path((guild_id, user_id)): Path<(i64, i64)>,
    Json(body): Json<TimeoutBody>,
) -> ApiResult<Json<Value>> {
    if !(0..=validate::MAX_TIMEOUT_SECS).contains(&body.seconds) {
        return Err(ApiError::BadRequest(
            "seconds must be between 0 and 28 days".into(),
        ));
    }
    let reason = reason(body.reason.as_deref())?;
    let (guild, _) =
        require_guild_perm(&state, guild_id, auth.user_id, Permissions::TIMEOUT_MEMBERS).await?;
    ensure_can_moderate(&state, &guild, auth.user_id, user_id).await?;
    let until = (body.seconds > 0).then(|| Utc::now() + ChronoDuration::seconds(body.seconds));
    if state.db.set_timeout(guild_id, user_id, until).await? == 0 {
        return Err(ApiError::NotFound);
    }
    if until.is_some() {
        state.sfu_kick(guild_id, user_id).await;
    }
    state.gateway.invalidate_guild(guild_id);
    state.gateway.publish_guild(
        guild_id,
        &Event::GuildMemberUpdate {
            guild_id,
            user_id,
            timeout_until: until,
        },
    );
    let detail = until.map(|until| until.to_rfc3339());
    audit(
        &state,
        guild_id,
        auth.user_id,
        if until.is_some() {
            "member_timeout"
        } else {
            "member_timeout_clear"
        },
        Some(user_id),
        reason.as_deref(),
        detail.as_deref(),
    )
    .await;
    Ok(json_ids(&serde_json::json!({
        "user_id": user_id,
        "timeout_until": until,
    })))
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuditQuery {
    before: Option<i64>,
    limit: Option<i64>,
}

/// Audit-Log lesen dürfen alle mit `MANAGE_GUILD` oder irgendeinem
/// Moderationsrecht.
pub(crate) async fn audit_log(
    State(state): State<AppState>,
    auth: Auth,
    Path(guild_id): Path<i64>,
    Query(query): Query<AuditQuery>,
) -> ApiResult<Json<Value>> {
    let (_, perms) = crate::guard::member_guild(&state, guild_id, auth.user_id).await?;
    let allowed = Permissions::MANAGE_GUILD
        | Permissions::KICK_MEMBERS
        | Permissions::BAN_MEMBERS
        | Permissions::TIMEOUT_MEMBERS
        | Permissions::MANAGE_ROLES;
    if !perms.intersects(allowed) {
        return Err(ApiError::Forbidden);
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    Ok(json_ids(
        &state.db.audit_log(guild_id, query.before, limit).await?,
    ))
}

// ------------------------------------------------------------------- Rollen

#[derive(Debug, Deserialize)]
pub(crate) struct RoleBody {
    name: Option<String>,
    permissions: Option<u64>,
    color: Option<i32>,
    position: Option<i32>,
}

/// Nur Rechte, die man selbst besitzt, dürfen vergeben werden.
fn checked_permissions(actor: Permissions, requested: Option<u64>) -> ApiResult<Option<i64>> {
    let Some(bits) = requested else {
        return Ok(None);
    };
    let requested = Permissions::from_bits_truncate(bits);
    if !actor.contains(requested) {
        return Err(ApiError::Forbidden);
    }
    Ok(Some(requested.bits() as i64))
}

pub(crate) async fn create_role(
    State(state): State<AppState>,
    auth: Auth,
    Path(guild_id): Path<i64>,
    Json(body): Json<RoleBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let name = body
        .name
        .as_deref()
        .unwrap_or("Neue Rolle")
        .trim()
        .to_owned();
    validate::validate_role_name(&name).map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    let (guild, perms) =
        require_guild_perm(&state, guild_id, auth.user_id, Permissions::MANAGE_ROLES).await?;
    let permissions = checked_permissions(perms, body.permissions)?.unwrap_or(0);
    let position = body.position.unwrap_or(1).max(1);
    ensure_role_below(&state, &guild, auth.user_id, position).await?;
    let role = state
        .db
        .create_role(
            state.ids.next_id(),
            guild_id,
            &name,
            permissions,
            body.color.unwrap_or(0),
            position,
        )
        .await?;
    state.gateway.invalidate_guild(guild_id);
    state
        .gateway
        .publish_guild(guild_id, &Event::GuildUpdate { guild_id });
    audit(
        &state,
        guild_id,
        auth.user_id,
        "role_create",
        Some(role.id),
        None,
        Some(&role.name),
    )
    .await;
    Ok((StatusCode::CREATED, json_ids(&role)))
}

pub(crate) async fn update_role(
    State(state): State<AppState>,
    auth: Auth,
    Path((guild_id, role_id)): Path<(i64, i64)>,
    Json(body): Json<RoleBody>,
) -> ApiResult<Json<Value>> {
    let (guild, perms) =
        require_guild_perm(&state, guild_id, auth.user_id, Permissions::MANAGE_ROLES).await?;
    let Some(role) = state
        .db
        .role(role_id)
        .await?
        .filter(|r| r.guild_id == guild_id)
    else {
        return Err(ApiError::NotFound);
    };
    if let Some(name) = &body.name {
        if role.is_everyone {
            return Err(ApiError::BadRequest("@everyone cannot be renamed".into()));
        }
        validate::validate_role_name(name).map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    }
    if !role.is_everyone {
        ensure_role_below(&state, &guild, auth.user_id, role.position).await?;
    }
    if let Some(position) = body.position {
        if role.is_everyone {
            return Err(ApiError::BadRequest("@everyone stays at position 0".into()));
        }
        ensure_role_below(&state, &guild, auth.user_id, position.max(1)).await?;
    }
    let permissions = checked_permissions(perms, body.permissions)?;
    let role = state
        .db
        .update_role(
            role_id,
            body.name.as_deref().map(str::trim),
            permissions,
            body.color,
            body.position.map(|position| position.max(1)),
        )
        .await?;
    state.gateway.invalidate_guild(guild_id);
    state
        .gateway
        .publish_guild(guild_id, &Event::GuildUpdate { guild_id });
    audit(
        &state,
        guild_id,
        auth.user_id,
        "role_update",
        Some(role_id),
        None,
        Some(&role.name),
    )
    .await;
    Ok(json_ids(&role))
}

pub(crate) async fn delete_role(
    State(state): State<AppState>,
    auth: Auth,
    Path((guild_id, role_id)): Path<(i64, i64)>,
) -> ApiResult<StatusCode> {
    let (guild, _) =
        require_guild_perm(&state, guild_id, auth.user_id, Permissions::MANAGE_ROLES).await?;
    let Some(role) = state
        .db
        .role(role_id)
        .await?
        .filter(|r| r.guild_id == guild_id)
    else {
        return Err(ApiError::NotFound);
    };
    if role.is_everyone {
        return Err(ApiError::BadRequest("@everyone cannot be deleted".into()));
    }
    ensure_role_below(&state, &guild, auth.user_id, role.position).await?;
    state.db.delete_role(role_id).await?;
    state.gateway.invalidate_guild(guild_id);
    state
        .gateway
        .publish_guild(guild_id, &Event::GuildUpdate { guild_id });
    audit(
        &state,
        guild_id,
        auth.user_id,
        "role_delete",
        Some(role_id),
        None,
        Some(&role.name),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn change_member_role(
    state: &AppState,
    auth: &Auth,
    guild_id: i64,
    user_id: i64,
    role_id: i64,
    add: bool,
) -> ApiResult<StatusCode> {
    let (guild, _) =
        require_guild_perm(state, guild_id, auth.user_id, Permissions::MANAGE_ROLES).await?;
    let Some(role) = state
        .db
        .role(role_id)
        .await?
        .filter(|r| r.guild_id == guild_id)
    else {
        return Err(ApiError::NotFound);
    };
    if role.is_everyone {
        return Err(ApiError::BadRequest("@everyone is implicit".into()));
    }
    ensure_role_below(state, &guild, auth.user_id, role.position).await?;
    if !state.db.is_member(guild_id, user_id).await? {
        return Err(ApiError::NotFound);
    }
    if add {
        state.db.add_member_role(guild_id, user_id, role_id).await?;
    } else {
        state
            .db
            .remove_member_role(guild_id, user_id, role_id)
            .await?;
    }
    state.gateway.invalidate_guild(guild_id);
    state.gateway.publish_guild(
        guild_id,
        &Event::GuildMemberUpdate {
            guild_id,
            user_id,
            timeout_until: None,
        },
    );
    audit(
        state,
        guild_id,
        auth.user_id,
        if add {
            "member_role_add"
        } else {
            "member_role_remove"
        },
        Some(user_id),
        None,
        Some(&role.name),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn add_member_role(
    State(state): State<AppState>,
    auth: Auth,
    Path((guild_id, user_id, role_id)): Path<(i64, i64, i64)>,
) -> ApiResult<StatusCode> {
    change_member_role(&state, &auth, guild_id, user_id, role_id, true).await
}

pub(crate) async fn remove_member_role(
    State(state): State<AppState>,
    auth: Auth,
    Path((guild_id, user_id, role_id)): Path<(i64, i64, i64)>,
) -> ApiResult<StatusCode> {
    change_member_role(&state, &auth, guild_id, user_id, role_id, false).await
}

// -------------------------------------------------------- Kanal-Overrides

#[derive(Debug, Deserialize)]
pub(crate) struct OverrideBody {
    allow: u64,
    deny: u64,
}

fn override_kind(kind: &str) -> ApiResult<i16> {
    match kind {
        "role" => Ok(0),
        "member" => Ok(1),
        _ => Err(ApiError::BadRequest(
            "override target must be 'role' or 'member'".into(),
        )),
    }
}

pub(crate) async fn set_override(
    State(state): State<AppState>,
    auth: Auth,
    Path((channel_id, kind, target_id)): Path<(i64, String, i64)>,
    Json(body): Json<OverrideBody>,
) -> ApiResult<StatusCode> {
    let target_type = override_kind(&kind)?;
    let Some(guild_id) = state.db.channel_guild(channel_id).await? else {
        return Err(ApiError::NotFound);
    };
    let (_, perms) = require_guild_perm(
        &state,
        guild_id,
        auth.user_id,
        Permissions::MANAGE_ROLES | Permissions::MANAGE_CHANNELS,
    )
    .await?;
    let allow = Permissions::from_bits_truncate(body.allow);
    let deny = Permissions::from_bits_truncate(body.deny);
    if !perms.contains(allow | deny) {
        return Err(ApiError::Forbidden);
    }
    state
        .db
        .set_override(
            channel_id,
            target_type,
            target_id,
            allow.bits() as i64,
            deny.bits() as i64,
        )
        .await?;
    state.gateway.invalidate_guild(guild_id);
    state
        .gateway
        .publish_guild(guild_id, &Event::GuildUpdate { guild_id });
    audit(
        &state,
        guild_id,
        auth.user_id,
        "channel_override_set",
        Some(channel_id),
        None,
        Some(&format!(
            "{kind}:{target_id} allow={} deny={}",
            allow.bits(),
            deny.bits()
        )),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_override(
    State(state): State<AppState>,
    auth: Auth,
    Path((channel_id, kind, target_id)): Path<(i64, String, i64)>,
) -> ApiResult<StatusCode> {
    let target_type = override_kind(&kind)?;
    let Some(guild_id) = state.db.channel_guild(channel_id).await? else {
        return Err(ApiError::NotFound);
    };
    require_guild_perm(
        &state,
        guild_id,
        auth.user_id,
        Permissions::MANAGE_ROLES | Permissions::MANAGE_CHANNELS,
    )
    .await?;
    state
        .db
        .delete_override(channel_id, target_type, target_id)
        .await?;
    state.gateway.invalidate_guild(guild_id);
    state
        .gateway
        .publish_guild(guild_id, &Event::GuildUpdate { guild_id });
    audit(
        &state,
        guild_id,
        auth.user_id,
        "channel_override_delete",
        Some(channel_id),
        None,
        Some(&format!("{kind}:{target_id}")),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
