//! Anhänge: Metadaten in PostgreSQL, die Bytes liegen im `BlobStore`
//! (content-addressed über `sha256`).
//!
//! Lebenslauf eines Anhangs:
//! 1. Upload legt eine Zeile ohne Nachricht an (`message_id`/`dm_message_id`
//!    leer) – nur die hochladende Person sieht sie.
//! 2. Beim Senden hängt eine Transaktion die Zeilen an die neue Nachricht.
//! 3. Unbenutzte Uploads und Anhänge gelöschter Nachrichten räumt
//!    [`Db::delete_orphan_attachments`] nach einer Frist ab.

use chrono::{DateTime, Utc};

use crate::{Db, models::Attachment};

/// Wo ein neuer Upload hingehört: Server-Kanal oder Direktnachricht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadTarget {
    Channel(i64),
    Dm(i64),
}

/// Felder eines neuen Anhangs.
#[derive(Debug, Clone)]
pub struct NewAttachment<'a> {
    pub id: i64,
    pub target: UploadTarget,
    pub uploader_id: i64,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub size: i64,
    pub sha256: &'a [u8],
    pub width: Option<i32>,
    pub height: Option<i32>,
}

/// Ergebnis beim Anhängen an eine neue Nachricht.
#[derive(Debug)]
pub enum InsertOutcome {
    Stored {
        created_at: DateTime<Utc>,
        attachments: Vec<Attachment>,
    },
    /// Mindestens eine ID gehört nicht der Person, liegt in einem anderen
    /// Kanal oder hängt schon an einer Nachricht. Nichts wurde gespeichert.
    InvalidAttachments,
}

impl Db {
    pub async fn create_attachment(
        &self,
        new: &NewAttachment<'_>,
    ) -> Result<Attachment, sqlx::Error> {
        let (channel_id, dm_id) = match new.target {
            UploadTarget::Channel(id) => (Some(id), None),
            UploadTarget::Dm(id) => (None, Some(id)),
        };
        sqlx::query_as!(
            Attachment,
            r#"insert into attachments
                 (id, channel_id, dm_id, uploader_id, filename, content_type, size, sha256,
                  width, height)
               values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               returning id, channel_id, message_id, dm_id, dm_message_id, uploader_id,
                         filename, content_type, size, sha256, width, height"#,
            new.id,
            channel_id,
            dm_id,
            new.uploader_id,
            new.filename,
            new.content_type,
            new.size,
            new.sha256,
            new.width,
            new.height
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn attachment(&self, id: i64) -> Result<Option<Attachment>, sqlx::Error> {
        sqlx::query_as!(
            Attachment,
            r#"select id, channel_id, message_id, dm_id, dm_message_id, uploader_id, filename,
                      content_type, size, sha256, width, height
               from attachments where id = $1"#,
            id
        )
        .fetch_optional(&self.pool)
        .await
    }

    /// Anhänge mehrerer Server-Nachrichten, sortiert nach ID (Upload-Reihenfolge).
    pub async fn attachments_for(
        &self,
        message_ids: &[i64],
    ) -> Result<Vec<Attachment>, sqlx::Error> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as!(
            Attachment,
            r#"select id, channel_id, message_id, dm_id, dm_message_id, uploader_id, filename,
                      content_type, size, sha256, width, height
               from attachments where message_id = any($1) order by id"#,
            message_ids
        )
        .fetch_all(&self.pool)
        .await
    }

    /// Anhänge mehrerer Direktnachrichten.
    pub async fn dm_attachments_for(
        &self,
        dm_message_ids: &[i64],
    ) -> Result<Vec<Attachment>, sqlx::Error> {
        if dm_message_ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as!(
            Attachment,
            r#"select id, channel_id, message_id, dm_id, dm_message_id, uploader_id, filename,
                      content_type, size, sha256, width, height
               from attachments where dm_message_id = any($1) order by id"#,
            dm_message_ids
        )
        .fetch_all(&self.pool)
        .await
    }

    /// Nachricht speichern und die Uploads in derselben Transaktion anhängen.
    /// Passt eine ID nicht, wird nichts gespeichert.
    pub async fn insert_message_with_attachments(
        &self,
        id: i64,
        channel_id: i64,
        author_id: i64,
        content: &str,
        reply_to: Option<i64>,
        attachment_ids: &[i64],
    ) -> Result<InsertOutcome, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query!(
            r#"insert into messages (id, channel_id, author_id, content, reply_to)
               values ($1, $2, $3, $4, $5) returning created_at"#,
            id,
            channel_id,
            author_id,
            content,
            reply_to
        )
        .fetch_one(&mut *tx)
        .await?;
        let attachments = sqlx::query_as!(
            Attachment,
            r#"update attachments set message_id = $1
               where id = any($2) and uploader_id = $3 and channel_id = $4
                 and message_id is null
               returning id, channel_id, message_id, dm_id, dm_message_id, uploader_id,
                         filename, content_type, size, sha256, width, height"#,
            id,
            attachment_ids,
            author_id,
            channel_id
        )
        .fetch_all(&mut *tx)
        .await?;
        if attachments.len() != unique_count(attachment_ids) {
            tx.rollback().await?;
            return Ok(InsertOutcome::InvalidAttachments);
        }
        tx.commit().await?;
        Ok(InsertOutcome::Stored {
            created_at: row.created_at,
            attachments: sorted(attachments),
        })
    }

    /// Wie [`Self::insert_message_with_attachments`], für Direktnachrichten.
    pub async fn insert_dm_message_with_attachments(
        &self,
        id: i64,
        dm_id: i64,
        author_id: i64,
        content: &str,
        attachment_ids: &[i64],
    ) -> Result<InsertOutcome, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query!(
            r#"insert into dm_messages (id, dm_id, author_id, content)
               values ($1, $2, $3, $4) returning created_at"#,
            id,
            dm_id,
            author_id,
            content
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query!(
            "update dm_channels set last_message_at = $2 where id = $1",
            dm_id,
            row.created_at
        )
        .execute(&mut *tx)
        .await?;
        let attachments = sqlx::query_as!(
            Attachment,
            r#"update attachments set dm_message_id = $1
               where id = any($2) and uploader_id = $3 and dm_id = $4
                 and dm_message_id is null
               returning id, channel_id, message_id, dm_id, dm_message_id, uploader_id,
                         filename, content_type, size, sha256, width, height"#,
            id,
            attachment_ids,
            author_id,
            dm_id
        )
        .fetch_all(&mut *tx)
        .await?;
        if attachments.len() != unique_count(attachment_ids) {
            tx.rollback().await?;
            return Ok(InsertOutcome::InvalidAttachments);
        }
        tx.commit().await?;
        Ok(InsertOutcome::Stored {
            created_at: row.created_at,
            attachments: sorted(attachments),
        })
    }

    /// Belegter Speicher einer Guild in Bytes (für die Quota).
    pub async fn guild_upload_bytes(&self, guild_id: i64) -> Result<i64, sqlx::Error> {
        let row = sqlx::query!(
            r#"select coalesce(sum(a.size), 0)::bigint as "total!"
               from attachments a join channels c on c.id = a.channel_id
               where c.guild_id = $1"#,
            guild_id
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.total)
    }

    /// Belegter Speicher einer Person in Direktnachrichten.
    pub async fn dm_upload_bytes(&self, uploader_id: i64) -> Result<i64, sqlx::Error> {
        let row = sqlx::query!(
            r#"select coalesce(sum(size), 0)::bigint as "total!"
               from attachments where uploader_id = $1 and dm_id is not null"#,
            uploader_id
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.total)
    }

    /// Eigenen, noch nicht gesendeten Upload verwerfen.
    pub async fn delete_pending_attachment(
        &self,
        id: i64,
        uploader_id: i64,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            r#"delete from attachments
               where id = $1 and uploader_id = $2
                 and message_id is null and dm_message_id is null"#,
            id,
            uploader_id
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Ist der Anhang ein Avatar oder Server-Icon? Die sieht jede angemeldete
    /// Person.
    pub async fn attachment_is_public(&self, id: i64) -> Result<bool, sqlx::Error> {
        let row = sqlx::query!(
            r#"select (exists (select 1 from users where avatar_id = $1)
                    or exists (select 1 from guilds where icon_id = $1)) as "public!""#,
            id
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.public)
    }

    /// Uploads ohne Nachricht, die älter als `before` sind, löschen – nie
    /// gesendete Entwürfe und Anhänge gelöschter Nachrichten. Avatare und
    /// Server-Icons bleiben.
    pub async fn delete_orphan_attachments(
        &self,
        before: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            r#"delete from attachments a
               where a.message_id is null and a.dm_message_id is null
                 and a.created_at < $1
                 and not exists (select 1 from users u where u.avatar_id = a.id)
                 and not exists (select 1 from guilds g where g.icon_id = a.id)"#,
            before
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Welche der Hashes noch von mindestens einem Anhang benutzt werden.
    pub async fn referenced_blobs(&self, hashes: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, sqlx::Error> {
        if hashes.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query!(
            r#"select distinct sha256 from attachments where sha256 = any($1)"#,
            hashes
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|row| row.sha256).collect())
    }
}

fn unique_count(ids: &[i64]) -> usize {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    ids.len()
}

fn sorted(mut attachments: Vec<Attachment>) -> Vec<Attachment> {
    attachments.sort_by_key(|attachment| attachment.id);
    attachments
}
