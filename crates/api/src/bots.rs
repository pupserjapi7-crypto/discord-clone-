//! Bot-Konten: anlegen, Token erneuern, einer Guild hinzufügen.
//!
//! Bots sind normale Nutzerzeilen mit `is_bot = true`. Sie authentifizieren
//! sich mit `Authorization: Bot nb_…` bei REST und Gateway. Der Klartext des
//! Tokens wird nur einmal zurückgegeben; gespeichert ist der SHA-256-Hash.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use nova_common::{
    permissions::Permissions,
    protocol::Event,
    tokens::{hash_bot_token, new_bot_token},
    validate,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AppState,
    error::{ApiError, ApiResult, unique_conflict},
    guard::{audit, require_guild_perm},
    json_ids,
    state::Auth,
};

/// Wie viele Bots ein Konto besitzen darf.
const MAX_BOTS_PER_OWNER: usize = 5;

#[derive(Debug, Deserialize)]
pub(crate) struct CreateBot {
    handle: String,
    display_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct BotCreated {
    id: i64,
    handle: String,
    display_name: String,
    /// Nur in dieser Antwort sichtbar.
    token: String,
}

fn humans_only(auth: &Auth) -> ApiResult<()> {
    if auth.is_bot {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

pub(crate) async fn create_bot(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<CreateBot>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    humans_only(&auth)?;
    let handle = validate::normalize_handle(&body.handle);
    validate::validate_handle(&handle).map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    let display_name = body
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&handle)
        .to_owned();
    validate::validate_display_name(&display_name)
        .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    if state.db.bots_for_owner(auth.user_id).await?.len() >= MAX_BOTS_PER_OWNER {
        return Err(ApiError::Conflict(format!(
            "at most {MAX_BOTS_PER_OWNER} bots per account"
        )));
    }

    let bot = state
        .db
        .create_bot(state.ids.next_id(), &handle, &display_name, auth.user_id)
        .await
        .map_err(|err| unique_conflict(err, "handle already taken"))?;
    let token = new_bot_token();
    state
        .db
        .replace_bot_token(bot.id, &hash_bot_token(&token))
        .await?;
    Ok((
        StatusCode::CREATED,
        json_ids(&BotCreated {
            id: bot.id,
            handle: bot.handle,
            display_name: bot.display_name,
            token,
        }),
    ))
}

pub(crate) async fn list_bots(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    humans_only(&auth)?;
    Ok(json_ids(&state.db.bots_for_owner(auth.user_id).await?))
}

async fn owned_bot(state: &AppState, auth: &Auth, bot_id: i64) -> ApiResult<()> {
    humans_only(auth)?;
    match state.db.bot_owner(bot_id).await? {
        Some(owner) if owner == auth.user_id => Ok(()),
        _ => Err(ApiError::NotFound),
    }
}

/// Neues Token; alle alten werden sofort ungültig.
pub(crate) async fn reset_token(
    State(state): State<AppState>,
    auth: Auth,
    Path(bot_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    owned_bot(&state, &auth, bot_id).await?;
    let token = new_bot_token();
    state
        .db
        .replace_bot_token(bot_id, &hash_bot_token(&token))
        .await?;
    Ok(json_ids(
        &serde_json::json!({ "id": bot_id, "token": token }),
    ))
}

/// Eigenen Bot einer Guild hinzufügen (`MANAGE_GUILD` nötig).
pub(crate) async fn add_to_guild(
    State(state): State<AppState>,
    auth: Auth,
    Path((guild_id, bot_id)): Path<(i64, i64)>,
) -> ApiResult<StatusCode> {
    owned_bot(&state, &auth, bot_id).await?;
    require_guild_perm(&state, guild_id, auth.user_id, Permissions::MANAGE_GUILD).await?;
    if !state.db.add_member(guild_id, bot_id).await? {
        return Err(ApiError::Forbidden);
    }
    state.gateway.member_added(guild_id, bot_id);
    state
        .gateway
        .publish_guild(guild_id, &Event::GuildUpdate { guild_id });
    audit(
        &state,
        guild_id,
        auth.user_id,
        "bot_add",
        Some(bot_id),
        None,
        None,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
