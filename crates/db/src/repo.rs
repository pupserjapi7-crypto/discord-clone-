use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use nova_common::permissions::Permissions;

use crate::{
    Db,
    models::{
        Attachment, AuditRow, BanRow, BotRow, CHANNEL_THREAD, Channel, Guild, InviteRow, MemberRow,
        MessageRow, OverrideRow, ReactionRow, Role, SessionRow, UnreadRow, User,
    },
};

impl Db {
    // ----------------------------------------------------------------- users

    pub async fn create_user(
        &self,
        id: i64,
        handle: &str,
        display_name: &str,
        password_hash: &str,
    ) -> Result<User, sqlx::Error> {
        sqlx::query_as!(
            User,
            r#"insert into users (id, handle, display_name, password_hash)
               values ($1, $2, $3, $4)
               returning id, handle, display_name, avatar_id, status, created_at, is_bot"#,
            id,
            handle,
            display_name,
            password_hash
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn user_with_hash(
        &self,
        handle: &str,
    ) -> Result<Option<(User, String)>, sqlx::Error> {
        let row = sqlx::query!(
            r#"select id, handle, display_name, avatar_id, status, created_at, is_bot, password_hash
               from users where lower(handle) = lower($1)"#,
            handle
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| {
            (
                User {
                    id: r.id,
                    handle: r.handle,
                    display_name: r.display_name,
                    avatar_id: r.avatar_id,
                    status: r.status,
                    created_at: r.created_at,
                    is_bot: r.is_bot,
                },
                r.password_hash,
            )
        }))
    }

    pub async fn user_by_id(&self, id: i64) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as!(
            User,
            r#"select id, handle, display_name, avatar_id, status, created_at, is_bot
               from users where id = $1"#,
            id
        )
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn update_profile(
        &self,
        id: i64,
        display_name: Option<&str>,
        status: Option<i16>,
    ) -> Result<User, sqlx::Error> {
        sqlx::query_as!(
            User,
            r#"update users
               set display_name = coalesce($2, display_name),
                   status = coalesce($3, status),
                   updated_at = now()
               where id = $1
               returning id, handle, display_name, avatar_id, status, created_at, is_bot"#,
            id,
            display_name,
            status
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn set_avatar(&self, id: i64, avatar_id: Option<i64>) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "update users set avatar_id = $2, updated_at = now() where id = $1",
            id,
            avatar_id
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    // -------------------------------------------------------------- sessions

    pub async fn create_session(
        &self,
        id: i64,
        user_id: i64,
        family_id: &str,
        token_hash: &[u8],
        expires_at: DateTime<Utc>,
        user_agent: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"insert into sessions (id, user_id, family_id, token_hash, expires_at, user_agent)
               values ($1, $2, $3, $4, $5, $6)"#,
            id,
            user_id,
            family_id,
            token_hash,
            expires_at,
            user_agent
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn session_by_hash(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<SessionRow>, sqlx::Error> {
        sqlx::query_as!(
            SessionRow,
            r#"select id, user_id, family_id, expires_at, revoked_at
               from sessions where token_hash = $1"#,
            token_hash
        )
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn revoke_session(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "update sessions set revoked_at = now() where id = $1 and revoked_at is null",
            id
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn revoke_family(&self, family_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "update sessions set revoked_at = now() where family_id = $1 and revoked_at is null",
            family_id
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn touch_session(&self, id: i64) -> Result<(), sqlx::Error> {
        sqlx::query!("update sessions set last_used_at = now() where id = $1", id)
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    /// Revoke the old refresh token and insert its successor in one transaction.
    pub async fn rotate_session(
        &self,
        old_id: i64,
        new_id: i64,
        new_hash: &[u8],
        expires_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "update sessions set revoked_at = now() where id = $1",
            old_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            r#"insert into sessions (id, user_id, family_id, token_hash, expires_at, user_agent)
               select $1, user_id, family_id, $2, $3, user_agent from sessions where id = $4"#,
            new_id,
            new_hash,
            expires_at,
            old_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    // ---------------------------------------------------------------- guilds

    #[allow(clippy::too_many_arguments)]
    pub async fn create_guild(
        &self,
        guild_id: i64,
        name: &str,
        owner_id: i64,
        everyone_role_id: i64,
        owner_role_id: i64,
        channel_id: i64,
    ) -> Result<Guild, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let guild = sqlx::query_as!(
            Guild,
            r#"insert into guilds (id, name, owner_id) values ($1, $2, $3)
               returning id, name, owner_id, icon_id, created_at"#,
            guild_id,
            name,
            owner_id
        )
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query!(
            r#"insert into roles (id, guild_id, name, permissions, position, is_everyone)
               values ($1, $2, '@everyone', $3, 0, true)"#,
            everyone_role_id,
            guild_id,
            Permissions::DEFAULT_EVERYONE.bits() as i64
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            r#"insert into roles (id, guild_id, name, permissions, position, is_everyone)
               values ($1, $2, 'Owner', $3, 100, false)"#,
            owner_role_id,
            guild_id,
            Permissions::all().bits() as i64
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            "insert into guild_members (guild_id, user_id) values ($1, $2)",
            guild_id,
            owner_id
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            "insert into member_roles (guild_id, user_id, role_id) values ($1, $2, $3)",
            guild_id,
            owner_id,
            owner_role_id
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query!(
            r#"insert into channels (id, guild_id, name, kind, position)
               values ($1, $2, 'allgemein', 0, 0)"#,
            channel_id,
            guild_id
        )
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(guild)
    }

    pub async fn guilds_for_user(&self, user_id: i64) -> Result<Vec<Guild>, sqlx::Error> {
        sqlx::query_as!(
            Guild,
            r#"select g.id, g.name, g.owner_id, g.icon_id, g.created_at
               from guilds g
               join guild_members m on m.guild_id = g.id
               where m.user_id = $1
               order by g.id"#,
            user_id
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn guild(&self, guild_id: i64) -> Result<Option<Guild>, sqlx::Error> {
        sqlx::query_as!(
            Guild,
            "select id, name, owner_id, icon_id, created_at from guilds where id = $1",
            guild_id
        )
        .fetch_optional(&self.pool)
        .await
    }

    /// Bulk-add seeded users to a guild (load testing).
    pub async fn add_members(&self, guild_id: i64, user_ids: &[i64]) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"insert into guild_members (guild_id, user_id)
               select $1, unnest($2::bigint[]) on conflict do nothing"#,
            guild_id,
            user_ids
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    /// Fügt ein Mitglied hinzu, sofern es nicht gebannt ist. `false` bei Bann.
    pub async fn add_member(&self, guild_id: i64, user_id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            r#"insert into guild_members (guild_id, user_id)
               select $1, $2
               where not exists (select 1 from guild_bans where guild_id = $1 and user_id = $2)
               on conflict do nothing"#,
            guild_id,
            user_id
        )
        .execute(&self.pool)
        .await?;
        if result.rows_affected() > 0 {
            return Ok(true);
        }
        Ok(!self.is_banned(guild_id, user_id).await?)
    }

    pub async fn is_member(&self, guild_id: i64, user_id: i64) -> Result<bool, sqlx::Error> {
        let row = sqlx::query!(
            "select exists(select 1 from guild_members where guild_id = $1 and user_id = $2) as \"exists!\"",
            guild_id,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.exists)
    }

    pub async fn members(&self, guild_id: i64) -> Result<Vec<MemberRow>, sqlx::Error> {
        sqlx::query_as!(
            MemberRow,
            r#"select m.user_id, u.handle, u.display_name, u.avatar_id, u.status,
                      m.nickname, m.joined_at, m.timeout_until, u.is_bot,
                      coalesce(array_agg(mr.role_id) filter (where mr.role_id is not null),
                               '{}') as "role_ids!"
               from guild_members m
               join users u on u.id = m.user_id
               left join member_roles mr on mr.guild_id = m.guild_id and mr.user_id = m.user_id
               where m.guild_id = $1
               group by m.user_id, u.handle, u.display_name, u.avatar_id, u.status,
                        m.nickname, m.joined_at, m.timeout_until, u.is_bot
               order by u.display_name, m.user_id"#,
            guild_id
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn remove_member(&self, guild_id: i64, user_id: i64) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "delete from guild_members where guild_id = $1 and user_id = $2",
            guild_id,
            user_id
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn set_timeout(
        &self,
        guild_id: i64,
        user_id: i64,
        until: Option<DateTime<Utc>>,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "update guild_members set timeout_until = $3 where guild_id = $1 and user_id = $2",
            guild_id,
            user_id,
            until
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Höchste Rollenposition eines Mitglieds (für die Moderations-Hierarchie).
    pub async fn top_role_position(&self, guild_id: i64, user_id: i64) -> Result<i32, sqlx::Error> {
        let row = sqlx::query!(
            r#"select coalesce(max(r.position), 0) as "position!"
               from member_roles mr join roles r on r.id = mr.role_id
               where mr.guild_id = $1 and mr.user_id = $2"#,
            guild_id,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.position)
    }

    // ------------------------------------------------------------------ bans

    pub async fn is_banned(&self, guild_id: i64, user_id: i64) -> Result<bool, sqlx::Error> {
        let row = sqlx::query!(
            r#"select exists(select 1 from guild_bans where guild_id = $1 and user_id = $2) as "exists!""#,
            guild_id,
            user_id
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.exists)
    }

    /// Bannt und entfernt das Mitglied in einer Transaktion.
    pub async fn ban(
        &self,
        guild_id: i64,
        user_id: i64,
        moderator_id: i64,
        reason: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            r#"insert into guild_bans (guild_id, user_id, moderator_id, reason)
               values ($1, $2, $3, $4)
               on conflict (guild_id, user_id) do update
                 set moderator_id = excluded.moderator_id, reason = excluded.reason"#,
            guild_id,
            user_id,
            moderator_id,
            reason
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "delete from guild_members where guild_id = $1 and user_id = $2",
            guild_id,
            user_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }

    pub async fn unban(&self, guild_id: i64, user_id: i64) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "delete from guild_bans where guild_id = $1 and user_id = $2",
            guild_id,
            user_id
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn bans(&self, guild_id: i64) -> Result<Vec<BanRow>, sqlx::Error> {
        sqlx::query_as!(
            BanRow,
            r#"select b.user_id, u.handle, u.display_name, b.moderator_id, b.reason, b.created_at
               from guild_bans b join users u on u.id = b.user_id
               where b.guild_id = $1 order by b.created_at desc"#,
            guild_id
        )
        .fetch_all(&self.pool)
        .await
    }

    // ------------------------------------------------------------- audit log

    #[allow(clippy::too_many_arguments)]
    pub async fn audit(
        &self,
        id: i64,
        guild_id: i64,
        actor_id: i64,
        action: &str,
        target_id: Option<i64>,
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"insert into audit_log (id, guild_id, actor_id, action, target_id, reason, detail)
               values ($1, $2, $3, $4, $5, $6, $7)"#,
            id,
            guild_id,
            actor_id,
            action,
            target_id,
            reason,
            detail
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn audit_log(
        &self,
        guild_id: i64,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<AuditRow>, sqlx::Error> {
        sqlx::query_as!(
            AuditRow,
            r#"select a.id, a.actor_id, u.display_name as "actor_name?", a.action, a.target_id,
                      a.reason, a.detail, a.created_at
               from audit_log a left join users u on u.id = a.actor_id
               where a.guild_id = $1 and ($2::bigint is null or a.id < $2)
               order by a.id desc limit $3"#,
            guild_id,
            before,
            limit
        )
        .fetch_all(&self.pool)
        .await
    }

    // ----------------------------------------------------------------- roles

    pub async fn roles(&self, guild_id: i64) -> Result<Vec<Role>, sqlx::Error> {
        sqlx::query_as!(
            Role,
            r#"select id, guild_id, name, color, permissions, position, is_everyone
               from roles where guild_id = $1 order by position desc, id"#,
            guild_id
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn role(&self, role_id: i64) -> Result<Option<Role>, sqlx::Error> {
        sqlx::query_as!(
            Role,
            r#"select id, guild_id, name, color, permissions, position, is_everyone
               from roles where id = $1"#,
            role_id
        )
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn create_role(
        &self,
        id: i64,
        guild_id: i64,
        name: &str,
        permissions: i64,
        color: i32,
        position: i32,
    ) -> Result<Role, sqlx::Error> {
        sqlx::query_as!(
            Role,
            r#"insert into roles (id, guild_id, name, permissions, color, position, is_everyone)
               values ($1, $2, $3, $4, $5, $6, false)
               returning id, guild_id, name, color, permissions, position, is_everyone"#,
            id,
            guild_id,
            name,
            permissions,
            color,
            position
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn update_role(
        &self,
        id: i64,
        name: Option<&str>,
        permissions: Option<i64>,
        color: Option<i32>,
        position: Option<i32>,
    ) -> Result<Role, sqlx::Error> {
        sqlx::query_as!(
            Role,
            r#"update roles set name = coalesce($2, name),
                                permissions = coalesce($3, permissions),
                                color = coalesce($4, color),
                                position = coalesce($5, position)
               where id = $1
               returning id, guild_id, name, color, permissions, position, is_everyone"#,
            id,
            name,
            permissions,
            color,
            position
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn delete_role(&self, id: i64) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("delete from roles where id = $1 and not is_everyone", id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn add_member_role(
        &self,
        guild_id: i64,
        user_id: i64,
        role_id: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"insert into member_roles (guild_id, user_id, role_id)
               values ($1, $2, $3) on conflict do nothing"#,
            guild_id,
            user_id,
            role_id
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn remove_member_role(
        &self,
        guild_id: i64,
        user_id: i64,
        role_id: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "delete from member_roles where guild_id = $1 and user_id = $2 and role_id = $3",
            guild_id,
            user_id,
            role_id
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    // -------------------------------------------------------------- channels

    pub async fn create_channel(
        &self,
        id: i64,
        guild_id: i64,
        name: &str,
        kind: i16,
        parent_id: Option<i64>,
    ) -> Result<Channel, sqlx::Error> {
        sqlx::query_as!(
            Channel,
            r#"insert into channels (id, guild_id, name, kind, parent_id, position)
               values ($1, $2, $3, $4, $5,
                       coalesce((select max(position) + 1 from channels where guild_id = $2), 0))
               returning id, guild_id, name, kind, topic, parent_id, position,
                         slowmode_secs, root_message_id, owner_id, archived"#,
            id,
            guild_id,
            name,
            kind,
            parent_id
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn create_thread(
        &self,
        id: i64,
        guild_id: i64,
        parent_id: i64,
        name: &str,
        root_message_id: i64,
        owner_id: i64,
    ) -> Result<Channel, sqlx::Error> {
        sqlx::query_as!(
            Channel,
            r#"insert into channels (id, guild_id, name, kind, parent_id, position,
                                     root_message_id, owner_id)
               values ($1, $2, $3, $4, $5, 0, $6, $7)
               returning id, guild_id, name, kind, topic, parent_id, position,
                         slowmode_secs, root_message_id, owner_id, archived"#,
            id,
            guild_id,
            name,
            CHANNEL_THREAD,
            parent_id,
            root_message_id,
            owner_id
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn thread_for_message(&self, message_id: i64) -> Result<Option<i64>, sqlx::Error> {
        let row = sqlx::query!(
            "select id from channels where root_message_id = $1 and kind = 2 limit 1",
            message_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.id))
    }

    pub async fn update_channel(
        &self,
        id: i64,
        name: Option<&str>,
        topic: Option<&str>,
        slowmode_secs: Option<i32>,
        archived: Option<bool>,
    ) -> Result<Channel, sqlx::Error> {
        sqlx::query_as!(
            Channel,
            r#"update channels set name = coalesce($2, name),
                                   topic = coalesce($3, topic),
                                   slowmode_secs = coalesce($4, slowmode_secs),
                                   archived = coalesce($5, archived)
               where id = $1
               returning id, guild_id, name, kind, topic, parent_id, position,
                         slowmode_secs, root_message_id, owner_id, archived"#,
            id,
            name,
            topic,
            slowmode_secs,
            archived
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn delete_channel(&self, id: i64) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("delete from channels where id = $1", id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn channels(&self, guild_id: i64) -> Result<Vec<Channel>, sqlx::Error> {
        sqlx::query_as!(
            Channel,
            r#"select id, guild_id, name, kind, topic, parent_id, position,
                      slowmode_secs, root_message_id, owner_id, archived
               from channels where guild_id = $1 order by position, id"#,
            guild_id
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn channel(&self, channel_id: i64) -> Result<Option<Channel>, sqlx::Error> {
        sqlx::query_as!(
            Channel,
            r#"select id, guild_id, name, kind, topic, parent_id, position,
                      slowmode_secs, root_message_id, owner_id, archived
               from channels where id = $1"#,
            channel_id
        )
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn channel_guild(&self, channel_id: i64) -> Result<Option<i64>, sqlx::Error> {
        let row = sqlx::query!("select guild_id from channels where id = $1", channel_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.guild_id))
    }

    /// Letzte Nachricht und Lesestand je Kanal. Ein Index-Lookup pro Kanal
    /// statt eines Aggregats über alle Nachrichten.
    pub async fn unread_state(
        &self,
        user_id: i64,
        channel_ids: &[i64],
    ) -> Result<Vec<UnreadRow>, sqlx::Error> {
        if channel_ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as!(
            UnreadRow,
            r#"select c.id as "channel_id!",
                      (select m.id from messages m where m.channel_id = c.id
                       order by m.id desc limit 1) as last_message_id,
                      coalesce(rs.last_read_id, 0) as "last_read_id!"
               from unnest($2::bigint[]) as c(id)
               left join read_states rs on rs.channel_id = c.id and rs.user_id = $1"#,
            user_id,
            channel_ids
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn set_override(
        &self,
        channel_id: i64,
        target_type: i16,
        target_id: i64,
        allow: i64,
        deny: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"insert into channel_overrides (channel_id, target_type, target_id, allow, deny)
               values ($1, $2, $3, $4, $5)
               on conflict (channel_id, target_type, target_id) do update
                 set allow = excluded.allow, deny = excluded.deny"#,
            channel_id,
            target_type,
            target_id,
            allow,
            deny
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn delete_override(
        &self,
        channel_id: i64,
        target_type: i16,
        target_id: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "delete from channel_overrides where channel_id = $1 and target_type = $2 and target_id = $3",
            channel_id,
            target_type,
            target_id
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn overrides(&self, channel_id: i64) -> Result<Vec<OverrideRow>, sqlx::Error> {
        sqlx::query_as!(
            OverrideRow,
            "select target_type, target_id, allow, deny from channel_overrides where channel_id = $1",
            channel_id
        )
        .fetch_all(&self.pool)
        .await
    }

    // --------------------------------------------------------------- invites

    pub async fn create_invite(
        &self,
        code: &str,
        guild_id: i64,
        creator_id: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "insert into invites (code, guild_id, creator_id) values ($1, $2, $3)",
            code,
            guild_id,
            creator_id
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn invite(&self, code: &str) -> Result<Option<InviteRow>, sqlx::Error> {
        sqlx::query_as!(
            InviteRow,
            "select code, guild_id, uses, max_uses, expires_at from invites where code = $1",
            code
        )
        .fetch_optional(&self.pool)
        .await
    }

    /// Returns the guild id on success, `None` for unknown/expired/exhausted
    /// invites and for users banned from the guild.
    pub async fn accept_invite(
        &self,
        code: &str,
        user_id: i64,
    ) -> Result<Option<i64>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query!(
            r#"select guild_id, uses, max_uses, expires_at from invites
               where code = $1 for update"#,
            code
        )
        .fetch_optional(&mut *tx)
        .await?;

        let Some(invite) = row else {
            tx.rollback().await?;
            return Ok(None);
        };
        let expired = invite
            .expires_at
            .map(|ts| ts <= Utc::now())
            .unwrap_or(false);
        let exhausted = invite
            .max_uses
            .map(|max| invite.uses >= max)
            .unwrap_or(false);
        let banned = sqlx::query!(
            r#"select exists(select 1 from guild_bans where guild_id = $1 and user_id = $2) as "exists!""#,
            invite.guild_id,
            user_id
        )
        .fetch_one(&mut *tx)
        .await?
        .exists;
        if expired || exhausted || banned {
            tx.rollback().await?;
            return Ok(None);
        }

        sqlx::query!(
            "insert into guild_members (guild_id, user_id) values ($1, $2) on conflict do nothing",
            invite.guild_id,
            user_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("update invites set uses = uses + 1 where code = $1", code)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(invite.guild_id))
    }

    // -------------------------------------------------------------- messages

    pub async fn insert_message(
        &self,
        id: i64,
        channel_id: i64,
        author_id: i64,
        content: &str,
        reply_to: Option<i64>,
    ) -> Result<DateTime<Utc>, sqlx::Error> {
        let row = sqlx::query!(
            r#"insert into messages (id, channel_id, author_id, content, reply_to)
               values ($1, $2, $3, $4, $5) returning created_at"#,
            id,
            channel_id,
            author_id,
            content,
            reply_to
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.created_at)
    }

    pub async fn message(&self, message_id: i64) -> Result<Option<MessageRow>, sqlx::Error> {
        sqlx::query_as!(
            MessageRow,
            r#"select m.id, m.channel_id, m.author_id, m.content, m.created_at, m.edited_at,
                      m.reply_to, m.pinned_at,
                      u.handle as author_handle, u.display_name as author_display_name,
                      u.avatar_id as author_avatar_id, u.status as author_status,
                      u.is_bot as author_is_bot
               from messages m join users u on u.id = m.author_id
               where m.id = $1"#,
            message_id
        )
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn list_messages(
        &self,
        channel_id: i64,
        before: Option<i64>,
        limit: i64,
    ) -> Result<Vec<MessageRow>, sqlx::Error> {
        sqlx::query_as!(
            MessageRow,
            r#"select m.id, m.channel_id, m.author_id, m.content, m.created_at, m.edited_at,
                      m.reply_to, m.pinned_at,
                      u.handle as author_handle, u.display_name as author_display_name,
                      u.avatar_id as author_avatar_id, u.status as author_status,
                      u.is_bot as author_is_bot
               from messages m join users u on u.id = m.author_id
               where m.channel_id = $1 and ($2::bigint is null or m.id < $2)
               order by m.id desc
               limit $3"#,
            channel_id,
            before,
            limit
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn pinned_messages(&self, channel_id: i64) -> Result<Vec<MessageRow>, sqlx::Error> {
        sqlx::query_as!(
            MessageRow,
            r#"select m.id, m.channel_id, m.author_id, m.content, m.created_at, m.edited_at,
                      m.reply_to, m.pinned_at,
                      u.handle as author_handle, u.display_name as author_display_name,
                      u.avatar_id as author_avatar_id, u.status as author_status,
                      u.is_bot as author_is_bot
               from messages m join users u on u.id = m.author_id
               where m.channel_id = $1 and m.pinned_at is not null
               order by m.pinned_at desc
               limit 50"#,
            channel_id
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn set_pinned(&self, message_id: i64, pinned: bool) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            r#"update messages set pinned_at = case when $2 then coalesce(pinned_at, now()) end
               where id = $1"#,
            message_id,
            pinned
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn update_message(&self, message_id: i64, content: &str) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "update messages set content = $2, edited_at = now() where id = $1",
            message_id,
            content
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn delete_message(&self, message_id: i64) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!("delete from messages where id = $1", message_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn reactions_for(
        &self,
        message_ids: &[i64],
        viewer_id: i64,
    ) -> Result<Vec<ReactionRow>, sqlx::Error> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as!(
            ReactionRow,
            r#"select message_id, emoji, count(*) as "count!", bool_or(user_id = $2) as "me!"
               from reactions
               where message_id = any($1)
               group by message_id, emoji
               order by min(created_at)"#,
            message_ids,
            viewer_id
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn add_reaction(
        &self,
        message_id: i64,
        user_id: i64,
        emoji: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"insert into reactions (message_id, user_id, emoji)
               values ($1, $2, $3) on conflict do nothing"#,
            message_id,
            user_id,
            emoji
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    pub async fn remove_reaction(
        &self,
        message_id: i64,
        user_id: i64,
        emoji: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "delete from reactions where message_id = $1 and user_id = $2 and emoji = $3",
            message_id,
            user_id,
            emoji
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    // ----------------------------------------------------------- attachments

    #[allow(clippy::too_many_arguments)]
    pub async fn create_attachment(
        &self,
        id: i64,
        channel_id: Option<i64>,
        uploader_id: i64,
        filename: &str,
        content_type: &str,
        size: i64,
        sha256: &[u8],
        width: Option<i32>,
        height: Option<i32>,
    ) -> Result<Attachment, sqlx::Error> {
        sqlx::query_as!(
            Attachment,
            r#"insert into attachments
                 (id, channel_id, uploader_id, filename, content_type, size, sha256, width, height)
               values ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               returning id, channel_id, message_id, uploader_id, filename, content_type, size,
                         sha256, width, height"#,
            id,
            channel_id,
            uploader_id,
            filename,
            content_type,
            size,
            sha256,
            width,
            height
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn attachment(&self, id: i64) -> Result<Option<Attachment>, sqlx::Error> {
        sqlx::query_as!(
            Attachment,
            r#"select id, channel_id, message_id, uploader_id, filename, content_type, size,
                      sha256, width, height
               from attachments where id = $1"#,
            id
        )
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn attachments_for(
        &self,
        message_ids: &[i64],
    ) -> Result<Vec<Attachment>, sqlx::Error> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as!(
            Attachment,
            r#"select id, channel_id, message_id, uploader_id, filename, content_type, size,
                      sha256, width, height
               from attachments where message_id = any($1) order by id"#,
            message_ids
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn attach_to_message(
        &self,
        message_id: i64,
        attachment_ids: &[i64],
        uploader_id: i64,
        channel_id: i64,
    ) -> Result<u64, sqlx::Error> {
        if attachment_ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query!(
            r#"update attachments set message_id = $1
               where id = any($2) and uploader_id = $3 and channel_id = $4 and message_id is null"#,
            message_id,
            attachment_ids,
            uploader_id,
            channel_id
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn upsert_read_state(
        &self,
        user_id: i64,
        channel_id: i64,
        message_id: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"insert into read_states (user_id, channel_id, last_read_id)
               values ($1, $2, $3)
               on conflict (user_id, channel_id) do update
                 set last_read_id = greatest(read_states.last_read_id, excluded.last_read_id),
                     updated_at = now()"#,
            user_id,
            channel_id,
            message_id
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
    }

    // ------------------------------------------------------------------ bots

    /// Legt ein Bot-Konto an. Bots haben kein nutzbares Passwort (`!` ist kein
    /// gültiger Argon2-Hash), sie melden sich nur per Token an.
    pub async fn create_bot(
        &self,
        id: i64,
        handle: &str,
        display_name: &str,
        owner_id: i64,
    ) -> Result<User, sqlx::Error> {
        sqlx::query_as!(
            User,
            r#"insert into users (id, handle, display_name, password_hash, is_bot, bot_owner_id)
               values ($1, $2, $3, '!', true, $4)
               returning id, handle, display_name, avatar_id, status, created_at, is_bot"#,
            id,
            handle,
            display_name,
            owner_id
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn bots_for_owner(&self, owner_id: i64) -> Result<Vec<BotRow>, sqlx::Error> {
        sqlx::query_as!(
            BotRow,
            r#"select id, handle, display_name, created_at from users
               where is_bot and bot_owner_id = $1 order by id"#,
            owner_id
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn bot_owner(&self, bot_id: i64) -> Result<Option<i64>, sqlx::Error> {
        let row = sqlx::query!(
            "select bot_owner_id from users where id = $1 and is_bot",
            bot_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|r| r.bot_owner_id))
    }

    /// Widerruft alle bisherigen Tokens und legt ein neues an.
    pub async fn replace_bot_token(
        &self,
        bot_id: i64,
        token_hash: &[u8],
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query!(
            "update bot_tokens set revoked_at = now() where bot_id = $1 and revoked_at is null",
            bot_id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "insert into bot_tokens (token_hash, bot_id) values ($1, $2)",
            token_hash,
            bot_id
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }

    pub async fn bot_by_token(&self, token_hash: &[u8]) -> Result<Option<i64>, sqlx::Error> {
        let row = sqlx::query!(
            "select bot_id from bot_tokens where token_hash = $1 and revoked_at is null",
            token_hash
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.bot_id))
    }

    // ----------------------------------------------------------- permissions

    /// Effektive Rechte. Nicht-Mitglieder (und damit auch Gebannte) bekommen
    /// keine Rechte; ein laufender Timeout entzieht `TIMEOUT_REVOKED`.
    pub async fn effective_permissions(
        &self,
        guild_id: i64,
        user_id: i64,
        channel_id: Option<i64>,
    ) -> Result<Permissions, sqlx::Error> {
        let Some(guild) = sqlx::query!("select owner_id from guilds where id = $1", guild_id)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(Permissions::empty());
        };
        if guild.owner_id == user_id {
            return Ok(Permissions::all());
        }

        let Some(member) = sqlx::query!(
            "select timeout_until from guild_members where guild_id = $1 and user_id = $2",
            guild_id,
            user_id
        )
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(Permissions::empty());
        };
        let timed_out = member.timeout_until.is_some_and(|until| until > Utc::now());

        let roles = sqlx::query_as!(
            Role,
            r#"select r.id, r.guild_id, r.name, r.color, r.permissions, r.position, r.is_everyone
               from roles r
               left join member_roles mr
                 on mr.role_id = r.id and mr.user_id = $2 and mr.guild_id = $1
               where r.guild_id = $1 and (r.is_everyone or mr.user_id is not null)"#,
            guild_id,
            user_id
        )
        .fetch_all(&self.pool)
        .await?;

        let mut perms = Permissions::empty();
        for role in &roles {
            perms |= Permissions::from_bits_retain(role.permissions as u64);
        }
        if perms.contains(Permissions::ADMINISTRATOR) {
            return Ok(Permissions::all());
        }
        let finish = |mut perms: Permissions| {
            if timed_out {
                perms.remove(Permissions::TIMEOUT_REVOKED);
            }
            perms
        };

        let Some(channel_id) = channel_id else {
            return Ok(finish(perms));
        };
        let Some(channel) = sqlx::query!(
            "select guild_id, parent_id, kind from channels where id = $1",
            channel_id
        )
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(Permissions::empty());
        };
        if channel.guild_id != guild_id {
            return Ok(Permissions::empty());
        }
        // Threads erben die Overrides ihres Elternkanals.
        let override_channel = if channel.kind == CHANNEL_THREAD {
            channel.parent_id.unwrap_or(channel_id)
        } else {
            channel_id
        };

        let overrides = self.overrides(override_channel).await?;
        let everyone_id = roles.iter().find(|r| r.is_everyone).map(|r| r.id);
        let find = |target_type: i16, target_id: i64| {
            overrides
                .iter()
                .find(|o| o.target_type == target_type && o.target_id == target_id)
                .map(|o| (o.allow as u64, o.deny as u64))
        };

        if let Some((allow, deny)) = everyone_id.and_then(|id| find(0, id)) {
            perms.apply_override(
                Permissions::from_bits_retain(allow),
                Permissions::from_bits_retain(deny),
            );
        }
        for role in &roles {
            if Some(role.id) == everyone_id {
                continue;
            }
            if let Some((allow, deny)) = find(0, role.id) {
                perms.apply_override(
                    Permissions::from_bits_retain(allow),
                    Permissions::from_bits_retain(deny),
                );
            }
        }
        if let Some((allow, deny)) = find(1, user_id) {
            perms.apply_override(
                Permissions::from_bits_retain(allow),
                Permissions::from_bits_retain(deny),
            );
        }

        Ok(finish(perms))
    }
}

/// Short-lived cache for permission lookups on the message hot path.
#[derive(Debug)]
pub struct PermCache {
    inner: DashMap<(i64, i64, i64), (Permissions, Instant)>,
    ttl: Duration,
}

impl PermCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: DashMap::new(),
            ttl,
        }
    }

    pub async fn get(
        &self,
        db: &Db,
        guild_id: i64,
        user_id: i64,
        channel_id: Option<i64>,
    ) -> Result<Permissions, sqlx::Error> {
        let key = (guild_id, user_id, channel_id.unwrap_or(0));
        if let Some(entry) = self.inner.get(&key)
            && entry.1.elapsed() < self.ttl
        {
            return Ok(entry.0);
        }
        let perms = db
            .effective_permissions(guild_id, user_id, channel_id)
            .await?;
        self.inner.insert(key, (perms, Instant::now()));
        Ok(perms)
    }

    pub fn invalidate_idle(&self, max_age: Duration) {
        self.inner.retain(|_, (_, at)| at.elapsed() < max_age);
    }

    pub fn invalidate_guild(&self, guild_id: i64) {
        self.inner.retain(|key, _| key.0 != guild_id);
    }
}
