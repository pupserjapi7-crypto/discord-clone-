-- Project Nova — Moderation, Threads, Pins, Slowmode, Bots (v0.4).
-- Nur additive Änderungen: bestehende Clients und Daten bleiben gültig.

-- Moderation -----------------------------------------------------------------

alter table guild_members add column timeout_until timestamptz;

create table guild_bans (
    guild_id     bigint not null references guilds(id) on delete cascade,
    user_id      bigint not null references users(id) on delete cascade,
    moderator_id bigint references users(id) on delete set null,
    reason       text,
    created_at   timestamptz not null default now(),
    primary key (guild_id, user_id)
);

create table audit_log (
    id         bigint primary key,
    guild_id   bigint not null references guilds(id) on delete cascade,
    actor_id   bigint references users(id) on delete set null,
    action     text not null,
    target_id  bigint,
    reason     text,
    detail     text,
    created_at timestamptz not null default now()
);
create index audit_log_guild_idx on audit_log (guild_id, id desc);

-- Kanäle: Slowmode, Threads ---------------------------------------------------

alter table channels add column slowmode_secs integer not null default 0
    check (slowmode_secs between 0 and 21600);
-- kind 2 = Thread; parent_id zeigt auf den Textkanal, root_message_id auf die
-- Startnachricht.
alter table channels add column root_message_id bigint references messages(id) on delete set null;
alter table channels add column owner_id bigint references users(id) on delete set null;
alter table channels add column archived boolean not null default false;
create index channels_parent_idx on channels (parent_id) where parent_id is not null;

-- Nachrichten: Pins -----------------------------------------------------------

alter table messages add column pinned_at timestamptz;
create index messages_pinned_idx on messages (channel_id, pinned_at desc) where pinned_at is not null;

-- Bots ------------------------------------------------------------------------

alter table users add column is_bot boolean not null default false;
alter table users add column bot_owner_id bigint references users(id) on delete cascade;

create table bot_tokens (
    token_hash bytea primary key,
    bot_id     bigint not null references users(id) on delete cascade,
    created_at timestamptz not null default now(),
    revoked_at timestamptz
);
create index bot_tokens_bot_idx on bot_tokens (bot_id);
