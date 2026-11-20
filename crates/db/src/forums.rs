//! Forum-Beiträge und Webhooks.
//!
//! Ein Forum-Beitrag ist ein Thread (`kind = 2`) unter einem Forum-Kanal
//! (`kind = 3`); seine erste Nachricht liegt im Thread selbst und ist zugleich
//! `root_message_id`.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    Db,
    models::{Attachment, CHANNEL_THREAD, Channel, User},
};

/// Beitrag in der Forum-Übersicht.
#[derive(Debug, Clone, Serialize)]
pub struct ForumPostRow {
    pub id: i64,
    pub title: String,
    pub archived: bool,
    pub author_id: Option<i64>,
    pub author_name: Option<String>,
    /// Anfang der ersten Nachricht.
    pub preview: Option<String>,
    pub created_at: DateTime<Utc>,
    /// Antworten (ohne die erste Nachricht).
    pub replies: i64,
    pub last_message_id: Option<i64>,
}

/// Neuer Beitrag samt erster Nachricht.
#[derive(Debug)]
pub struct NewPost {
    pub thread: Channel,
    pub message_id: i64,
    pub created_at: DateTime<Utc>,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebhookRow {
    pub id: i64,
    pub guild_id: i64,
    pub channel_id: i64,
    pub user_id: i64,
    pub name: String,
    pub creator_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl Db {
    /// Beitrag anlegen: Thread, erste Nachricht und – in derselben
    /// Transaktion – Anhänge, die zuvor in das Forum hochgeladen wurden.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_forum_post(
        &self,
        thread_id: i64,
        message_id: i64,
        guild_id: i64,
        forum_id: i64,
        title: &str,
        author_id: i64,
        content: &str,
        attachment_ids: &[i64],
    ) -> Result<Option<NewPost>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let thread = sqlx::query_as!(
            Channel,
            r#"insert into channels (id, guild_id, name, kind, parent_id, position, owner_id)
               values ($1, $2, $3, $4, $5, 0, $6)
               returning id, guild_id, name, kind, topic, parent_id, position,
                         slowmode_secs, root_message_id, owner_id, archived"#,
            thread_id,
            guild_id,
            title,
            CHANNEL_THREAD,
            forum_id,
            author_id
        )
        .fetch_one(&mut *tx)
        .await?;
        let row = sqlx::query!(
            r#"insert into messages (id, channel_id, author_id, content)
               values ($1, $2, $3, $4) returning created_at"#,
            message_id,
            thread_id,
            author_id,
            content
        )
        .fetch_one(&mut *tx)
        .await?;
        let attachments = if attachment_ids.is_empty() {
            Vec::new()
        } else {
            let moved = sqlx::query_as!(
                Attachment,
                r#"update attachments set channel_id = $1, message_id = $2
                   where id = any($3) and uploader_id = $4 and channel_id = $5
                     and message_id is null
                   returning id, channel_id, message_id, dm_id, dm_message_id, uploader_id,
                             filename, content_type, size, sha256, width, height"#,
                thread_id,
                message_id,
                attachment_ids,
                author_id,
                forum_id
            )
            .fetch_all(&mut *tx)
            .await?;
            let mut unique = attachment_ids.to_vec();
            unique.sort_unstable();
            unique.dedup();
            if moved.len() != unique.len() {
                tx.rollback().await?;
                return Ok(None);
            }
            moved
        };
        let thread = sqlx::query_as!(
            Channel,
            r#"update channels set root_message_id = $2 where id = $1
               returning id, guild_id, name, kind, topic, parent_id, position,
                         slowmode_secs, root_message_id, owner_id, archived"#,
            thread.id,
            message_id
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut attachments = attachments;
        attachments.sort_by_key(|attachment| attachment.id);
        Ok(Some(NewPost {
            thread,
            message_id,
            created_at: row.created_at,
            attachments,
        }))
    }

    /// Beiträge eines Forums, zuletzt aktive zuerst.
    pub async fn forum_posts(
        &self,
        forum_id: i64,
        limit: i64,
    ) -> Result<Vec<ForumPostRow>, sqlx::Error> {
        sqlx::query_as!(
            ForumPostRow,
            r#"select c.id, c.name as title, c.archived,
                      m.author_id as "author_id?", u.display_name as "author_name?",
                      left(m.content, 200) as preview,
                      coalesce(m.created_at, now()) as "created_at!",
                      greatest((select count(*) from messages x where x.channel_id = c.id) - 1, 0)
                          as "replies!",
                      (select max(x.id) from messages x where x.channel_id = c.id) as last_message_id
               from channels c
               left join messages m on m.id = c.root_message_id
               left join users u on u.id = m.author_id
               where c.parent_id = $1 and c.kind = $2
               order by coalesce((select max(x.id) from messages x where x.channel_id = c.id), c.id) desc
               limit $3"#,
            forum_id,
            CHANNEL_THREAD,
            limit
        )
        .fetch_all(&self.pool)
        .await
    }

    // ------------------------------------------------------------- Webhooks

    /// Webhook samt Bot-Konto anlegen. `token_hash` ist der SHA-256 des Tokens.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_webhook(
        &self,
        id: i64,
        user_id: i64,
        guild_id: i64,
        channel_id: i64,
        name: &str,
        token_hash: &[u8],
        creator_id: i64,
    ) -> Result<WebhookRow, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        // Kein nutzbares Passwort (`!` ist kein Argon2-Hash), wie bei Bots.
        sqlx::query!(
            r#"insert into users (id, handle, display_name, password_hash, is_bot, bot_owner_id)
               values ($1, $2, $3, '!', true, $4)"#,
            user_id,
            format!("hook.{id}"),
            name,
            creator_id
        )
        .execute(&mut *tx)
        .await?;
        let row = sqlx::query_as!(
            WebhookRow,
            r#"insert into webhooks (id, guild_id, channel_id, user_id, name, token_hash, creator_id)
               values ($1, $2, $3, $4, $5, $6, $7)
               returning id, guild_id, channel_id, user_id, name, creator_id, created_at"#,
            id,
            guild_id,
            channel_id,
            user_id,
            name,
            token_hash,
            creator_id
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn webhooks(&self, channel_id: i64) -> Result<Vec<WebhookRow>, sqlx::Error> {
        sqlx::query_as!(
            WebhookRow,
            r#"select id, guild_id, channel_id, user_id, name, creator_id, created_at
               from webhooks where channel_id = $1 order by id"#,
            channel_id
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn webhook(&self, id: i64) -> Result<Option<WebhookRow>, sqlx::Error> {
        sqlx::query_as!(
            WebhookRow,
            r#"select id, guild_id, channel_id, user_id, name, creator_id, created_at
               from webhooks where id = $1"#,
            id
        )
        .fetch_optional(&self.pool)
        .await
    }

    /// Webhook zum Token, falls ID und Token passen.
    pub async fn webhook_by_token(
        &self,
        id: i64,
        token_hash: &[u8],
    ) -> Result<Option<(WebhookRow, User)>, sqlx::Error> {
        let Some(hook) = sqlx::query_as!(
            WebhookRow,
            r#"select id, guild_id, channel_id, user_id, name, creator_id, created_at
               from webhooks where id = $1 and token_hash = $2"#,
            id,
            token_hash
        )
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let Some(user) = self.user_by_id(hook.user_id).await? else {
            return Ok(None);
        };
        Ok(Some((hook, user)))
    }

    /// Webhook löschen. Sein Bot-Konto bleibt, damit alte Nachrichten ihren
    /// Absender behalten.
    pub async fn delete_webhook(&self, id: i64) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("delete from webhooks where id = $1", id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
