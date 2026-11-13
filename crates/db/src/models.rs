use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct User {
    pub id: i64,
    pub handle: String,
    pub display_name: String,
    pub avatar_id: Option<i64>,
    pub status: i16,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Guild {
    pub id: i64,
    pub name: String,
    pub owner_id: i64,
    pub icon_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Role {
    pub id: i64,
    pub guild_id: i64,
    pub name: String,
    pub color: i32,
    pub permissions: i64,
    pub position: i32,
    pub is_everyone: bool,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Channel {
    pub id: i64,
    pub guild_id: i64,
    pub name: String,
    pub kind: i16,
    pub topic: Option<String>,
    pub parent_id: Option<i64>,
    pub position: i32,
}

#[derive(Debug, Clone, FromRow)]
pub struct MessageRow {
    pub id: i64,
    pub channel_id: i64,
    pub author_id: i64,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub reply_to: Option<i64>,
    pub author_handle: String,
    pub author_display_name: String,
    pub author_avatar_id: Option<i64>,
    pub author_status: i16,
}

#[derive(Debug, Clone, FromRow)]
pub struct ReactionRow {
    pub message_id: i64,
    pub emoji: String,
    pub count: i64,
    pub me: bool,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Attachment {
    pub id: i64,
    pub channel_id: Option<i64>,
    pub message_id: Option<i64>,
    pub uploader_id: i64,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub sha256: Vec<u8>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

#[derive(Debug, Clone, FromRow)]
pub struct SessionRow {
    pub id: i64,
    pub user_id: i64,
    pub family_id: String,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct MemberRow {
    pub user_id: i64,
    pub handle: String,
    pub display_name: String,
    pub avatar_id: Option<i64>,
    pub status: i16,
    pub nickname: Option<String>,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct InviteRow {
    pub code: String,
    pub guild_id: i64,
    pub uses: i32,
    pub max_uses: Option<i32>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct OverrideRow {
    pub target_type: i16,
    pub target_id: i64,
    pub allow: i64,
    pub deny: i64,
}
