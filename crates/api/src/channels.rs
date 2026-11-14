//! Kanäle bearbeiten, Nachrichten per REST (für Bots und Moderation), Pins,
//! Threads und Lesestand.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use nova_common::{
    permissions::Permissions,
    protocol::{Event, MessageView},
    validate,
};
use nova_db::{models::CHANNEL_THREAD, views};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    AppState,
    error::{ApiError, ApiResult},
    guard::{audit, visible_channel},
    json_ids,
    state::Auth,
};

#[derive(Debug, Deserialize)]
pub(crate) struct ChannelPatch {
    name: Option<String>,
    topic: Option<String>,
    slowmode_secs: Option<i32>,
    archived: Option<bool>,
}

/// Kanal ändern. Name/Thema/Slowmode: `MANAGE_CHANNELS`. Threads archivieren:
/// `MANAGE_THREADS` oder wer den Thread gestartet hat.
pub(crate) async fn update_channel(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
    Json(body): Json<ChannelPatch>,
) -> ApiResult<Json<Value>> {
    let (channel, perms) = visible_channel(&state, channel_id, auth.user_id).await?;
    let is_thread = channel.kind == CHANNEL_THREAD;

    let structural = body.name.is_some() || body.topic.is_some() || body.slowmode_secs.is_some();
    let can_manage = perms.contains(Permissions::MANAGE_CHANNELS)
        || (is_thread && perms.contains(Permissions::MANAGE_THREADS));
    if structural && !can_manage {
        return Err(ApiError::Forbidden);
    }
    if body.archived.is_some() {
        if !is_thread {
            return Err(ApiError::BadRequest("only threads can be archived".into()));
        }
        let owner = channel.owner_id == Some(auth.user_id);
        if !owner && !perms.contains(Permissions::MANAGE_THREADS) {
            return Err(ApiError::Forbidden);
        }
    }
    if let Some(name) = &body.name {
        let check = if is_thread {
            validate::validate_thread_name(name)
        } else {
            validate::validate_channel_name(name)
        };
        check.map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    }
    if let Some(topic) = &body.topic {
        validate::validate_topic(topic).map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    }
    if let Some(secs) = body.slowmode_secs
        && !(0..=validate::MAX_SLOWMODE_SECS).contains(&secs)
    {
        return Err(ApiError::BadRequest(
            "slowmode must be between 0 and 21600 seconds".into(),
        ));
    }

    let updated = state
        .db
        .update_channel(
            channel_id,
            body.name.as_deref().map(str::trim),
            body.topic.as_deref().map(str::trim),
            body.slowmode_secs,
            body.archived,
        )
        .await?;
    state.gateway.invalidate_channel(channel_id);
    state.gateway.publish_guild(
        channel.guild_id,
        &Event::GuildUpdate {
            guild_id: channel.guild_id,
        },
    );
    let mut changes = Vec::new();
    if let Some(name) = &body.name {
        changes.push(format!("name={name}"));
    }
    if body.topic.is_some() {
        changes.push("topic".to_owned());
    }
    if let Some(secs) = body.slowmode_secs {
        changes.push(format!("slowmode={secs}s"));
    }
    if let Some(archived) = body.archived {
        changes.push(format!("archived={archived}"));
    }
    audit(
        &state,
        channel.guild_id,
        auth.user_id,
        "channel_update",
        Some(channel_id),
        None,
        Some(&changes.join(", ")),
    )
    .await;
    Ok(json_ids(&updated))
}

pub(crate) async fn delete_channel(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
) -> ApiResult<StatusCode> {
    let (channel, perms) = visible_channel(&state, channel_id, auth.user_id).await?;
    let allowed = perms.contains(Permissions::MANAGE_CHANNELS)
        || (channel.kind == CHANNEL_THREAD && perms.contains(Permissions::MANAGE_THREADS));
    if !allowed {
        return Err(ApiError::Forbidden);
    }
    state.db.delete_channel(channel_id).await?;
    state.gateway.invalidate_channel(channel_id);
    state.gateway.publish_guild(
        channel.guild_id,
        &Event::GuildUpdate {
            guild_id: channel.guild_id,
        },
    );
    audit(
        &state,
        channel.guild_id,
        auth.user_id,
        "channel_delete",
        Some(channel_id),
        None,
        Some(&channel.name),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// --------------------------------------------------------------- Nachrichten

#[derive(Debug, Deserialize)]
pub(crate) struct CreateMessage {
    content: String,
    reply_to: Option<i64>,
}

/// Nachricht per REST senden – gleiche Regeln wie über das Gateway
/// (Rechte, Timeout, Slowmode, archivierte Threads) plus Rate-Limit pro Konto.
pub(crate) async fn create_message(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
    Json(body): Json<CreateMessage>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let (channel, perms) = visible_channel(&state, channel_id, auth.user_id).await?;
    if channel.kind == 1 {
        return Err(ApiError::BadRequest(
            "voice channels have no messages".into(),
        ));
    }
    if !perms.contains(Permissions::SEND_MESSAGES) {
        return Err(ApiError::Forbidden);
    }
    validate::validate_message_content(&body.content, state.cfg.max_message_len)
        .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    if !state.auth_limiter.allow(
        format!("msg:{}", auth.user_id),
        state.cfg.msg_burst,
        state.cfg.msg_rate,
    ) {
        return Err(ApiError::RateLimited);
    }
    if let Err((code, message)) = state
        .gateway
        .check_send(channel_id, auth.user_id, perms)
        .await
    {
        return Err(match code {
            "slowmode" => ApiError::RateLimited,
            _ => ApiError::Conflict(message),
        });
    }
    let author = state
        .db
        .user_by_id(auth.user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let id = state.ids.next_id();
    let created_at = state
        .db
        .insert_message(id, channel_id, auth.user_id, &body.content, body.reply_to)
        .await?;
    state.metrics.messages_created.inc();
    let view = MessageView {
        id,
        channel_id,
        author: views::user_public(&state.cfg.public_origin, &author),
        content: body.content,
        created_at,
        edited_at: None,
        reply_to: body.reply_to,
        reactions: Vec::new(),
        attachments: Vec::new(),
        pinned: false,
    };
    state.gateway.publish_channel(
        channel_id,
        &Event::MessageCreate {
            message: view.clone(),
        },
    );
    Ok((StatusCode::CREATED, json_ids(&view)))
}

pub(crate) async fn delete_message(
    State(state): State<AppState>,
    auth: Auth,
    Path((channel_id, message_id)): Path<(i64, i64)>,
) -> ApiResult<StatusCode> {
    let (channel, perms) = visible_channel(&state, channel_id, auth.user_id).await?;
    let Some(message) = state
        .db
        .message(message_id)
        .await?
        .filter(|message| message.channel_id == channel_id)
    else {
        return Err(ApiError::NotFound);
    };
    let own = message.author_id == auth.user_id;
    if !own && !perms.contains(Permissions::MANAGE_MESSAGES) {
        return Err(ApiError::Forbidden);
    }
    state.db.delete_message(message_id).await?;
    state.gateway.publish_channel(
        channel_id,
        &Event::MessageDelete {
            channel_id,
            message_id,
        },
    );
    if !own {
        audit(
            &state,
            channel.guild_id,
            auth.user_id,
            "message_delete",
            Some(message.author_id),
            None,
            Some(&format!("channel={channel_id}")),
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

// --------------------------------------------------------------------- Pins

async fn set_pin(
    state: &AppState,
    auth: &Auth,
    channel_id: i64,
    message_id: i64,
    pinned: bool,
) -> ApiResult<StatusCode> {
    let (channel, perms) = visible_channel(state, channel_id, auth.user_id).await?;
    if !perms.contains(Permissions::MANAGE_MESSAGES) {
        return Err(ApiError::Forbidden);
    }
    let Some(message) = state
        .db
        .message(message_id)
        .await?
        .filter(|message| message.channel_id == channel_id)
    else {
        return Err(ApiError::NotFound);
    };
    state.db.set_pinned(message_id, pinned).await?;
    state.gateway.publish_channel(
        channel_id,
        &Event::MessagePin {
            channel_id,
            message_id,
            pinned,
        },
    );
    audit(
        state,
        channel.guild_id,
        auth.user_id,
        if pinned {
            "message_pin"
        } else {
            "message_unpin"
        },
        Some(message.author_id),
        None,
        Some(&format!("channel={channel_id} message={message_id}")),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn pin(
    State(state): State<AppState>,
    auth: Auth,
    Path((channel_id, message_id)): Path<(i64, i64)>,
) -> ApiResult<StatusCode> {
    set_pin(&state, &auth, channel_id, message_id, true).await
}

pub(crate) async fn unpin(
    State(state): State<AppState>,
    auth: Auth,
    Path((channel_id, message_id)): Path<(i64, i64)>,
) -> ApiResult<StatusCode> {
    set_pin(&state, &auth, channel_id, message_id, false).await
}

pub(crate) async fn list_pins(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let (_, perms) = visible_channel(&state, channel_id, auth.user_id).await?;
    if !perms.contains(Permissions::READ_MESSAGE_HISTORY) {
        return Err(ApiError::Forbidden);
    }
    let rows = state.db.pinned_messages(channel_id).await?;
    let base = &state.cfg.public_origin;
    let views: Vec<MessageView> = rows
        .into_iter()
        .map(|row| views::message_view(base, row, Vec::new(), Vec::new()))
        .collect();
    Ok(json_ids(&views))
}

// ------------------------------------------------------------------ Threads

#[derive(Debug, Deserialize)]
pub(crate) struct CreateThread {
    name: String,
}

/// Thread aus einer Nachricht starten. Pro Nachricht gibt es höchstens einen
/// Thread; ein zweiter Versuch liefert den vorhandenen.
pub(crate) async fn create_thread(
    State(state): State<AppState>,
    auth: Auth,
    Path((channel_id, message_id)): Path<(i64, i64)>,
    Json(body): Json<CreateThread>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate::validate_thread_name(&body.name)
        .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    let (channel, perms) = visible_channel(&state, channel_id, auth.user_id).await?;
    if channel.kind != 0 {
        return Err(ApiError::BadRequest(
            "threads start from messages in text channels".into(),
        ));
    }
    if !perms.contains(Permissions::CREATE_THREADS) {
        return Err(ApiError::Forbidden);
    }
    if state
        .db
        .message(message_id)
        .await?
        .filter(|message| message.channel_id == channel_id)
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    if let Some(existing) = state.db.thread_for_message(message_id).await?
        && let Some(thread) = state.db.channel(existing).await?
    {
        return Ok((StatusCode::OK, json_ids(&thread)));
    }
    let thread = state
        .db
        .create_thread(
            state.ids.next_id(),
            channel.guild_id,
            channel_id,
            body.name.trim(),
            message_id,
            auth.user_id,
        )
        .await?;
    state.gateway.publish_guild(
        channel.guild_id,
        &Event::GuildUpdate {
            guild_id: channel.guild_id,
        },
    );
    Ok((StatusCode::CREATED, json_ids(&thread)))
}

// --------------------------------------------------------------- Lesestand

#[derive(Debug, Deserialize)]
pub(crate) struct AckBody {
    #[serde(deserialize_with = "id_or_string")]
    message_id: i64,
}

fn id_or_string<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
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

pub(crate) async fn ack(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
    Json(body): Json<AckBody>,
) -> ApiResult<StatusCode> {
    visible_channel(&state, channel_id, auth.user_id).await?;
    state
        .db
        .upsert_read_state(auth.user_id, channel_id, body.message_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
