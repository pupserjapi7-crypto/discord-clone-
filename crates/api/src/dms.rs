//! Direktnachrichten zwischen zwei Personen.
//!
//! - `GET  /users/@me/dms` – eigene DM-Kanäle, zuletzt aktive zuerst
//! - `POST /users/@me/dms` `{ "handle": "…" }` – Kanal mit einer Person öffnen
//! - `GET  /dms/{id}/messages?before=&limit=` – Verlauf
//! - `POST /dms/{id}/messages` `{ "content": "…", "attachment_ids": [] }` – senden
//! - `POST /dms/{id}/attachments?filename=` – Anhang hochladen (siehe `uploads`)
//!
//! Nur Beteiligte sehen einen Kanal; für alle anderen gibt es 404. Bots
//! können keine DMs öffnen oder senden (Spam-Schutz).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use nova_common::{
    protocol::{AttachmentView, DmChannelView, Event, MessageView, UserPublic, id_compat},
    validate,
};
use nova_db::{attachments::InsertOutcome, views};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    AppState,
    error::{ApiError, ApiResult},
    json_ids,
    state::Auth,
};

/// Höchstens so viele DM-Kanäle liefert die Liste.
const MAX_DMS: i64 = 200;

#[derive(Debug, Deserialize)]
pub(crate) struct OpenDm {
    handle: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DmMessageQuery {
    before: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateDmMessage {
    #[serde(default)]
    content: String,
    /// Ab v0.8: vorher über `POST /dms/{id}/attachments` hochgeladen.
    #[serde(default, deserialize_with = "id_compat::deserialize_vec")]
    attachment_ids: Vec<i64>,
}

fn no_bots(auth: &Auth) -> ApiResult<()> {
    if auth.is_bot {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

pub(crate) async fn list_dms(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    no_bots(&auth)?;
    let base = &state.cfg.public_origin;
    let views: Vec<DmChannelView> = state
        .db
        .list_dms(auth.user_id, MAX_DMS)
        .await?
        .into_iter()
        .map(|row| DmChannelView {
            id: row.id,
            user: views::user_public(base, &row.peer),
            last_message_at: row.last_message_at,
        })
        .collect();
    Ok(json_ids(&views))
}

pub(crate) async fn open_dm(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<OpenDm>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    no_bots(&auth)?;
    // Gleiches Limit wie Nachrichten: verhindert das Abklappern von Namen.
    if !state.auth_limiter.allow(
        format!("dm-open:{}", auth.user_id),
        state.cfg.msg_burst,
        state.cfg.msg_rate,
    ) {
        return Err(ApiError::RateLimited);
    }
    let handle = body.handle.trim().trim_start_matches('@');
    if handle.is_empty() || handle.len() > 64 {
        return Err(ApiError::BadRequest("invalid handle".into()));
    }
    // Unbekannt und Bot sehen gleich aus: niemand erfährt, welche Namen es gibt,
    // außer über die normale Suche nach einem Gegenüber.
    let Some(peer) = state.db.user_by_handle(handle).await? else {
        return Err(ApiError::NotFound);
    };
    if peer.is_bot {
        return Err(ApiError::NotFound);
    }
    if peer.id == auth.user_id {
        return Err(ApiError::BadRequest("you cannot message yourself".into()));
    }
    let (id, last_message_at) = state
        .db
        .open_dm(state.ids.next_id(), auth.user_id, peer.id)
        .await?;
    let view = DmChannelView {
        id,
        user: views::user_public(&state.cfg.public_origin, &peer),
        last_message_at,
    };
    Ok((StatusCode::OK, json_ids(&view)))
}

/// Prüft die Beteiligung und liefert beide Personen (ich, Gegenüber).
async fn participants(
    state: &AppState,
    dm_id: i64,
    user_id: i64,
) -> ApiResult<(UserPublic, UserPublic)> {
    let Some(peer_id) = state.db.dm_peer(dm_id, user_id).await? else {
        return Err(ApiError::NotFound);
    };
    let base = &state.cfg.public_origin;
    let me = state
        .db
        .user_by_id(user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let peer = state
        .db
        .user_by_id(peer_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok((
        views::user_public(base, &me),
        views::user_public(base, &peer),
    ))
}

fn dm_view(
    id: i64,
    dm_id: i64,
    author: UserPublic,
    content: String,
    created_at: chrono::DateTime<chrono::Utc>,
    attachments: Vec<AttachmentView>,
) -> MessageView {
    MessageView {
        id,
        channel_id: dm_id,
        author,
        content,
        created_at,
        edited_at: None,
        reply_to: None,
        reactions: Vec::new(),
        attachments,
        pinned: false,
    }
}

pub(crate) async fn list_dm_messages(
    State(state): State<AppState>,
    auth: Auth,
    Path(dm_id): Path<i64>,
    Query(query): Query<DmMessageQuery>,
) -> ApiResult<Json<Value>> {
    no_bots(&auth)?;
    let (me, peer) = participants(&state, dm_id, auth.user_id).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let mut rows = state.db.dm_messages(dm_id, query.before, limit).await?;
    // Neueste zuerst geholt, angezeigt wird von alt nach neu.
    rows.reverse();
    let ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
    let attachments = state.db.dm_attachments_for(&ids).await?;
    let base = &state.cfg.public_origin;
    let views: Vec<MessageView> = rows
        .into_iter()
        .map(|row| {
            let author = if row.author_id == me.id {
                me.clone()
            } else {
                peer.clone()
            };
            let own = attachments
                .iter()
                .filter(|attachment| attachment.dm_message_id == Some(row.id))
                .map(|attachment| views::attachment_view(base, attachment))
                .collect();
            dm_view(row.id, row.dm_id, author, row.content, row.created_at, own)
        })
        .collect();
    Ok(json_ids(&views))
}

pub(crate) async fn create_dm_message(
    State(state): State<AppState>,
    auth: Auth,
    Path(dm_id): Path<i64>,
    Json(body): Json<CreateDmMessage>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    no_bots(&auth)?;
    let (me, peer) = participants(&state, dm_id, auth.user_id).await?;
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
    let id = state.ids.next_id();
    let (created_at, attachments) = if body.attachment_ids.is_empty() {
        let created_at = state
            .db
            .insert_dm_message(id, dm_id, auth.user_id, &body.content)
            .await?;
        (created_at, Vec::new())
    } else {
        match state
            .db
            .insert_dm_message_with_attachments(
                id,
                dm_id,
                auth.user_id,
                &body.content,
                &body.attachment_ids,
            )
            .await?
        {
            InsertOutcome::Stored {
                created_at,
                attachments,
            } => (created_at, attachments),
            InsertOutcome::InvalidAttachments => {
                return Err(ApiError::BadRequest("unknown attachment".into()));
            }
        }
    };
    state.metrics.messages_created.inc();
    let base = &state.cfg.public_origin;
    let attachments = attachments
        .iter()
        .map(|attachment| views::attachment_view(base, attachment))
        .collect();
    let view = dm_view(id, dm_id, me, body.content, created_at, attachments);
    let event = Event::DmMessageCreate {
        message: view.clone(),
    };
    state.gateway.publish_user(auth.user_id, &event);
    state.gateway.publish_user(peer.id, &event);
    Ok((StatusCode::CREATED, json_ids(&view)))
}
