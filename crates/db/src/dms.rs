//! Direktnachrichten: Kanäle zwischen genau zwei Personen und ihre Nachrichten.
//!
//! Rechteprüfung ist hier eingebaut: jede Abfrage mit `user_id` liefert nur
//! Kanäle, an denen diese Person beteiligt ist.

use chrono::{DateTime, Utc};

use crate::{Db, models::User};

/// Ein DM-Kanal aus Sicht einer Person: das Gegenüber und die letzte Aktivität.
#[derive(Debug, Clone)]
pub struct DmRow {
    pub id: i64,
    pub peer: User,
    pub last_message_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DmMessageRow {
    pub id: i64,
    pub dm_id: i64,
    pub author_id: i64,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

impl Db {
    /// Kanal zwischen zwei Personen holen oder anlegen. `new_id` wird nur beim
    /// Anlegen benutzt. Die beiden IDs müssen verschieden sein.
    pub async fn open_dm(
        &self,
        new_id: i64,
        a: i64,
        b: i64,
    ) -> Result<(i64, DateTime<Utc>), sqlx::Error> {
        let (low, high) = if a < b { (a, b) } else { (b, a) };
        // `do update` statt `do nothing`, damit `returning` auch beim
        // vorhandenen Kanal eine Zeile liefert.
        let row = sqlx::query!(
            r#"insert into dm_channels (id, user_low, user_high) values ($1, $2, $3)
               on conflict (user_low, user_high) do update set user_low = excluded.user_low
               returning id, last_message_at"#,
            new_id,
            low,
            high
        )
        .fetch_one(&self.pool)
        .await?;
        Ok((row.id, row.last_message_at))
    }

    /// Das Gegenüber in einem DM-Kanal, sofern `user_id` beteiligt ist.
    pub async fn dm_peer(&self, dm_id: i64, user_id: i64) -> Result<Option<i64>, sqlx::Error> {
        let row = sqlx::query!(
            r#"select case when user_low = $2 then user_high else user_low end as "peer!"
               from dm_channels where id = $1 and (user_low = $2 or user_high = $2)"#,
            dm_id,
            user_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| row.peer))
    }

    /// Alle DM-Kanäle einer Person, zuletzt aktive zuerst.
    pub async fn list_dms(&self, user_id: i64, limit: i64) -> Result<Vec<DmRow>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"select d.id, d.last_message_at,
                      u.id as peer_id, u.handle, u.display_name, u.avatar_id, u.status,
                      u.created_at, u.is_bot
               from dm_channels d
               join users u on u.id = case when d.user_low = $1 then d.user_high else d.user_low end
               where d.user_low = $1 or d.user_high = $1
               order by d.last_message_at desc
               limit $2"#,
            user_id,
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| DmRow {
                id: row.id,
                last_message_at: row.last_message_at,
                peer: User {
                    id: row.peer_id,
                    handle: row.handle,
                    display_name: row.display_name,
                    avatar_id: row.avatar_id,
                    status: row.status,
                    created_at: row.created_at,
                    is_bot: row.is_bot,
                },
            })
            .collect())
    }

    /// Nachricht speichern und den Kanal als zuletzt aktiv markieren.
    pub async fn insert_dm_message(
        &self,
        id: i64,
        dm_id: i64,
        author_id: i64,
        content: &str,
    ) -> Result<DateTime<Utc>, sqlx::Error> {
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
        tx.commit().await?;
        Ok(row.created_at)
    }

    /// Nachrichten eines DM-Kanals, neueste zuerst (für die Seitenweise-Abfrage).
    pub async fn dm_messages(
        &self,
        dm_id: i64,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<DmMessageRow>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"select id, dm_id, author_id, content, created_at from dm_messages
               where dm_id = $1 and ($2::bigint is null or id < $2)
               order by id desc
               limit $3"#,
            dm_id,
            before,
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| DmMessageRow {
                id: row.id,
                dm_id: row.dm_id,
                author_id: row.author_id,
                content: row.content,
                created_at: row.created_at,
            })
            .collect())
    }

    /// Person über ihren Benutzernamen finden (ohne Passwort-Hash).
    pub async fn user_by_handle(&self, handle: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as!(
            User,
            r#"select id, handle, display_name, avatar_id, status, created_at, is_bot
               from users where lower(handle) = lower($1)"#,
            handle
        )
        .fetch_optional(&self.pool)
        .await
    }
}
