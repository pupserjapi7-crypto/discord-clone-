//! Gemeinsame Prüfungen für alle Handler: erst Mitgliedschaft, dann Rechte,
//! bei Moderation zusätzlich die Rollen-Hierarchie. Siehe Skill `nova-security`.

use nova_common::permissions::Permissions;
use nova_db::models::{Channel, Guild};
use tracing::warn;

use crate::{
    AppState,
    error::{ApiError, ApiResult},
};

/// Guild laden und sicherstellen, dass der Aufrufer Mitglied ist. Für
/// Nicht-Mitglieder sieht eine fremde Guild aus wie eine nicht existierende.
pub(crate) async fn member_guild(
    state: &AppState,
    guild_id: i64,
    user_id: i64,
) -> ApiResult<(Guild, Permissions)> {
    let Some(guild) = state.db.guild(guild_id).await? else {
        return Err(ApiError::NotFound);
    };
    if !state.db.is_member(guild_id, user_id).await? {
        return Err(ApiError::NotFound);
    }
    let perms = state
        .db
        .effective_permissions(guild_id, user_id, None)
        .await?;
    Ok((guild, perms))
}

/// Wie [`member_guild`], verlangt aber ein bestimmtes Recht.
pub(crate) async fn require_guild_perm(
    state: &AppState,
    guild_id: i64,
    user_id: i64,
    needed: Permissions,
) -> ApiResult<(Guild, Permissions)> {
    let (guild, perms) = member_guild(state, guild_id, user_id).await?;
    if !perms.contains(needed) {
        return Err(ApiError::Forbidden);
    }
    Ok((guild, perms))
}

/// Kanal laden, Mitgliedschaft und `VIEW_CHANNEL` prüfen. Unsichtbare Kanäle
/// liefern 404, nicht 403.
pub(crate) async fn visible_channel(
    state: &AppState,
    channel_id: i64,
    user_id: i64,
) -> ApiResult<(Channel, Permissions)> {
    let Some(channel) = state.db.channel(channel_id).await? else {
        return Err(ApiError::NotFound);
    };
    if !state.db.is_member(channel.guild_id, user_id).await? {
        return Err(ApiError::NotFound);
    }
    let perms = state
        .db
        .effective_permissions(channel.guild_id, user_id, Some(channel_id))
        .await?;
    if !perms.contains(Permissions::VIEW_CHANNEL) {
        return Err(ApiError::NotFound);
    }
    Ok((channel, perms))
}

/// Hierarchie: niemand moderiert den Owner, sich selbst oder jemanden mit
/// gleich hoher oder höherer Rolle. Der Owner darf alle anderen moderieren.
pub(crate) async fn ensure_can_moderate(
    state: &AppState,
    guild: &Guild,
    actor_id: i64,
    target_id: i64,
) -> ApiResult<()> {
    if actor_id == target_id {
        return Err(ApiError::BadRequest("you cannot moderate yourself".into()));
    }
    if target_id == guild.owner_id {
        return Err(ApiError::Forbidden);
    }
    if actor_id == guild.owner_id {
        return Ok(());
    }
    let actor = state.db.top_role_position(guild.id, actor_id).await?;
    let target = state.db.top_role_position(guild.id, target_id).await?;
    if actor <= target {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

/// Rollen dürfen nur unterhalb der eigenen höchsten Rolle verwaltet werden.
pub(crate) async fn ensure_role_below(
    state: &AppState,
    guild: &Guild,
    actor_id: i64,
    role_position: i32,
) -> ApiResult<()> {
    if actor_id == guild.owner_id {
        return Ok(());
    }
    let actor = state.db.top_role_position(guild.id, actor_id).await?;
    if role_position >= actor {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

/// Audit-Eintrag schreiben. Ein Fehler hier darf die eigentliche Aktion nicht
/// rückgängig machen, wird aber geloggt.
pub(crate) async fn audit(
    state: &AppState,
    guild_id: i64,
    actor_id: i64,
    action: &str,
    target_id: Option<i64>,
    reason: Option<&str>,
    detail: Option<&str>,
) {
    if let Err(err) = state
        .db
        .audit(
            state.ids.next_id(),
            guild_id,
            actor_id,
            action,
            target_id,
            reason,
            detail,
        )
        .await
    {
        warn!(error = %err, action, "audit log write failed");
    }
}

/// Optionaler Grund: leer → `None`, sonst validiert.
pub(crate) fn reason(raw: Option<&str>) -> ApiResult<Option<String>> {
    let Some(text) = raw.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    nova_common::validate::validate_reason(text)
        .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    Ok(Some(text.to_owned()))
}
