//! Forum-Kanäle und eingehende Webhooks.
//!
//! - `GET  /channels/{forum}/posts` – Beiträge, zuletzt aktive zuerst
//! - `POST /channels/{forum}/posts` `{title, content, attachment_ids?}` – neuer
//!   Beitrag (Thread mit erster Nachricht)
//! - `GET/POST /channels/{id}/webhooks` – Webhooks eines Textkanals
//!   (`MANAGE_WEBHOOKS`); das Token gibt es nur einmal beim Anlegen
//! - `DELETE /webhooks/{id}` – löschen
//! - `POST /webhooks/{id}/{token}` `{content}` – ausführen (ohne Anmeldung)

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use nova_common::{
    permissions::Permissions,
    protocol::{Event, MessageView, id_compat},
    validate,
};
use nova_db::models::{CHANNEL_FORUM, CHANNEL_TEXT};
use nova_db::views;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AppState,
    error::{ApiError, ApiResult},
    guard::{audit, visible_channel},
    json_ids, random_hex, sha256,
    state::Auth,
};

/// Höchstens so viele Beiträge liefert die Übersicht.
const MAX_POSTS: i64 = 100;
/// Höchstens so viele Webhooks pro Kanal.
const MAX_WEBHOOKS: usize = 10;

#[derive(Debug, Deserialize)]
pub(crate) struct CreatePost {
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default, deserialize_with = "id_compat::deserialize_vec")]
    attachment_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
struct PostCreated {
    thread: nova_db::models::Channel,
    message: MessageView,
}

pub(crate) async fn list_posts(
    State(state): State<AppState>,
    auth: Auth,
    Path(forum_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let (channel, perms) = visible_channel(&state, forum_id, auth.user_id).await?;
    if channel.kind != CHANNEL_FORUM {
        return Err(ApiError::BadRequest("channel is not a forum".into()));
    }
    if !perms.contains(Permissions::READ_MESSAGE_HISTORY) {
        return Err(ApiError::Forbidden);
    }
    let posts = state.db.forum_posts(forum_id, MAX_POSTS).await?;
    Ok(json_ids(&posts))
}

pub(crate) async fn create_post(
    State(state): State<AppState>,
    auth: Auth,
    Path(forum_id): Path<i64>,
    Json(body): Json<CreatePost>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let (forum, perms) = visible_channel(&state, forum_id, auth.user_id).await?;
    if forum.kind != CHANNEL_FORUM {
        return Err(ApiError::BadRequest("channel is not a forum".into()));
    }
    if !perms.contains(Permissions::SEND_MESSAGES) {
        return Err(ApiError::Forbidden);
    }
    if !body.attachment_ids.is_empty() && !perms.contains(Permissions::ATTACH_FILES) {
        return Err(ApiError::Forbidden);
    }
    validate::validate_thread_name(&body.title)
        .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    validate::validate_message_body(
        &body.content,
        state.cfg.max_message_len,
        body.attachment_ids.len(),
    )
    .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    if !state.auth_limiter.allow(
        format!("msg:{}", auth.user_id),
        state.cfg.msg_burst,
        state.cfg.msg_rate,
    ) {
        return Err(ApiError::RateLimited);
    }
    let author = state
        .db
        .user_by_id(auth.user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let Some(post) = state
        .db
        .create_forum_post(
            state.ids.next_id(),
            state.ids.next_id(),
            forum.guild_id,
            forum_id,
            body.title.trim(),
            auth.user_id,
            &body.content,
            &body.attachment_ids,
        )
        .await?
    else {
        return Err(ApiError::BadRequest("unknown attachment".into()));
    };
    state.metrics.messages_created.inc();
    let base = &state.cfg.public_origin;
    let message = MessageView {
        id: post.message_id,
        channel_id: post.thread.id,
        author: views::user_public(base, &author),
        content: body.content,
        created_at: post.created_at,
        edited_at: None,
        reply_to: None,
        reactions: Vec::new(),
        attachments: post
            .attachments
            .iter()
            .map(|attachment| views::attachment_view(base, attachment))
            .collect(),
        pinned: false,
    };
    state.gateway.publish_guild(
        forum.guild_id,
        &Event::GuildUpdate {
            guild_id: forum.guild_id,
        },
    );
    Ok((
        StatusCode::CREATED,
        json_ids(&PostCreated {
            thread: post.thread,
            message,
        }),
    ))
}

// ------------------------------------------------------------- Webhooks

#[derive(Debug, Deserialize)]
pub(crate) struct CreateWebhook {
    name: String,
}

#[derive(Debug, Serialize)]
struct WebhookCreated {
    #[serde(flatten)]
    hook: nova_db::forums::WebhookRow,
    /// Nur jetzt sichtbar; gespeichert ist nur der Hash.
    token: String,
    url: String,
}

fn webhook_url(base: &str, id: i64, token: &str) -> String {
    format!(
        "{}/api/v1/webhooks/{id}/{token}",
        base.trim_end_matches('/')
    )
}

/// Kanal laden und `MANAGE_WEBHOOKS` verlangen.
async fn webhook_channel(
    state: &AppState,
    channel_id: i64,
    user_id: i64,
) -> ApiResult<nova_db::models::Channel> {
    let (channel, perms) = visible_channel(state, channel_id, user_id).await?;
    if !perms.contains(Permissions::MANAGE_WEBHOOKS) {
        return Err(ApiError::Forbidden);
    }
    Ok(channel)
}

pub(crate) async fn list_webhooks(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    webhook_channel(&state, channel_id, auth.user_id).await?;
    Ok(json_ids(&state.db.webhooks(channel_id).await?))
}

pub(crate) async fn create_webhook(
    State(state): State<AppState>,
    auth: Auth,
    Path(channel_id): Path<i64>,
    Json(body): Json<CreateWebhook>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if auth.is_bot {
        return Err(ApiError::Forbidden);
    }
    let channel = webhook_channel(&state, channel_id, auth.user_id).await?;
    if channel.kind != CHANNEL_TEXT {
        return Err(ApiError::BadRequest(
            "webhooks post into text channels only".into(),
        ));
    }
    let name = body.name.trim();
    validate::validate_display_name(name).map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    if state.db.webhooks(channel_id).await?.len() >= MAX_WEBHOOKS {
        return Err(ApiError::Conflict(
            "too many webhooks in this channel".into(),
        ));
    }
    let token = random_hex(32);
    let id = state.ids.next_id();
    let hook = state
        .db
        .create_webhook(
            id,
            state.ids.next_id(),
            channel.guild_id,
            channel_id,
            name,
            &sha256(token.as_bytes()),
            auth.user_id,
        )
        .await?;
    audit(
        &state,
        channel.guild_id,
        auth.user_id,
        "webhook_create",
        Some(channel_id),
        None,
        Some(name),
    )
    .await;
    let url = webhook_url(&state.cfg.public_origin, id, &token);
    Ok((
        StatusCode::CREATED,
        json_ids(&WebhookCreated { hook, token, url }),
    ))
}

pub(crate) async fn delete_webhook(
    State(state): State<AppState>,
    auth: Auth,
    Path(webhook_id): Path<i64>,
) -> ApiResult<StatusCode> {
    let Some(hook) = state.db.webhook(webhook_id).await? else {
        return Err(ApiError::NotFound);
    };
    webhook_channel(&state, hook.channel_id, auth.user_id).await?;
    state.db.delete_webhook(webhook_id).await?;
    audit(
        &state,
        hook.guild_id,
        auth.user_id,
        "webhook_delete",
        Some(hook.channel_id),
        None,
        Some(&hook.name),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub(crate) struct ExecuteWebhook {
    content: String,
}

/// Webhook ausführen: Nachricht als Bot-Konto des Webhooks. Falsche ID oder
/// falsches Token sehen gleich aus (404). 30 Nachrichten Burst, dann eine
/// alle zwei Sekunden.
pub(crate) async fn execute_webhook(
    State(state): State<AppState>,
    Path((webhook_id, token)): Path<(i64, String)>,
    Json(body): Json<ExecuteWebhook>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::NotFound);
    }
    let Some((hook, user)) = state
        .db
        .webhook_by_token(webhook_id, &sha256(token.as_bytes()))
        .await?
    else {
        return Err(ApiError::NotFound);
    };
    validate::validate_message_content(&body.content, state.cfg.max_message_len)
        .map_err(|msg| ApiError::BadRequest(msg.to_owned()))?;
    if !state
        .auth_limiter
        .allow(format!("webhook:{webhook_id}"), 30.0, 0.5)
    {
        return Err(ApiError::RateLimited);
    }
    let id = state.ids.next_id();
    let created_at = state
        .db
        .insert_message(id, hook.channel_id, user.id, &body.content, None)
        .await?;
    state.metrics.messages_created.inc();
    let view = MessageView {
        id,
        channel_id: hook.channel_id,
        author: views::user_public(&state.cfg.public_origin, &user),
        content: body.content,
        created_at,
        edited_at: None,
        reply_to: None,
        reactions: Vec::new(),
        attachments: Vec::new(),
        pinned: false,
    };
    state.gateway.publish_channel(
        hook.channel_id,
        &Event::MessageCreate {
            message: view.clone(),
        },
    );
    Ok((StatusCode::CREATED, json_ids(&view)))
}
