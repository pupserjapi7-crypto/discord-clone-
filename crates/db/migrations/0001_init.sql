-- Project Nova — initial schema (milestone 1).
-- IDs are snowflakes (BIGINT). No FTS yet: search moves to milestone 3.

create table users (
    id            bigint primary key,
    handle        text not null,
    display_name  text not null,
    password_hash text not null,
    avatar_id     bigint,
    status        smallint not null default 0,
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now()
);
create unique index users_handle_lower_idx on users (lower(handle));

create table sessions (
    id           bigint primary key,
    user_id      bigint not null references users(id) on delete cascade,
    family_id    text not null,
    token_hash   bytea not null unique,
    created_at   timestamptz not null default now(),
    expires_at   timestamptz not null,
    revoked_at   timestamptz,
    last_used_at timestamptz not null default now(),
    user_agent   text
);
create index sessions_user_idx on sessions (user_id);
create index sessions_family_idx on sessions (family_id);

create table guilds (
    id         bigint primary key,
    name       text not null,
    owner_id   bigint not null references users(id) on delete cascade,
    icon_id    bigint,
    created_at timestamptz not null default now()
);

create table guild_members (
    guild_id  bigint not null references guilds(id) on delete cascade,
    user_id   bigint not null references users(id) on delete cascade,
    nickname  text,
    joined_at timestamptz not null default now(),
    primary key (guild_id, user_id)
);
create index guild_members_user_idx on guild_members (user_id);

create table roles (
    id          bigint primary key,
    guild_id    bigint not null references guilds(id) on delete cascade,
    name        text not null,
    color       integer not null default 0,
    permissions bigint not null default 0,
    position    integer not null default 0,
    is_everyone boolean not null default false,
    created_at  timestamptz not null default now()
);
create index roles_guild_idx on roles (guild_id);
create unique index roles_everyone_idx on roles (guild_id) where is_everyone;

create table member_roles (
    guild_id bigint not null,
    user_id  bigint not null,
    role_id  bigint not null references roles(id) on delete cascade,
    primary key (guild_id, user_id, role_id),
    foreign key (guild_id, user_id) references guild_members(guild_id, user_id) on delete cascade
);

create table channels (
    id         bigint primary key,
    guild_id   bigint not null references guilds(id) on delete cascade,
    name       text not null,
    kind       smallint not null default 0,
    topic      text,
    parent_id  bigint references channels(id) on delete cascade,
    position   integer not null default 0,
    created_at timestamptz not null default now()
);
create index channels_guild_idx on channels (guild_id, position);

create table channel_overrides (
    channel_id  bigint not null references channels(id) on delete cascade,
    target_type smallint not null,
    target_id   bigint not null,
    allow       bigint not null default 0,
    deny        bigint not null default 0,
    primary key (channel_id, target_type, target_id)
);

create table messages (
    id         bigint primary key,
    channel_id bigint not null references channels(id) on delete cascade,
    author_id  bigint not null references users(id) on delete cascade,
    content    text not null check (char_length(content) between 1 and 8000),
    reply_to   bigint references messages(id) on delete set null,
    created_at timestamptz not null default now(),
    edited_at  timestamptz
);
create index messages_channel_cursor_idx on messages (channel_id, id desc);
create index messages_author_idx on messages (author_id, id desc);

create table message_mentions (
    message_id bigint not null references messages(id) on delete cascade,
    user_id    bigint not null references users(id) on delete cascade,
    primary key (message_id, user_id)
);

create table reactions (
    message_id bigint not null references messages(id) on delete cascade,
    user_id    bigint not null references users(id) on delete cascade,
    emoji      text not null,
    created_at timestamptz not null default now(),
    primary key (message_id, user_id, emoji)
);

create table attachments (
    id           bigint primary key,
    channel_id   bigint references channels(id) on delete cascade,
    message_id   bigint references messages(id) on delete set null,
    uploader_id  bigint not null references users(id) on delete cascade,
    filename     text not null,
    content_type text not null,
    size         bigint not null,
    sha256       bytea not null,
    width        integer,
    height       integer,
    created_at   timestamptz not null default now()
);
create index attachments_message_idx on attachments (message_id);
create index attachments_channel_idx on attachments (channel_id);

create table read_states (
    user_id      bigint not null references users(id) on delete cascade,
    channel_id   bigint not null references channels(id) on delete cascade,
    last_read_id bigint not null default 0,
    updated_at   timestamptz not null default now(),
    primary key (user_id, channel_id)
);

create table invites (
    code       text primary key,
    guild_id   bigint not null references guilds(id) on delete cascade,
    creator_id bigint not null references users(id) on delete cascade,
    uses       integer not null default 0,
    max_uses   integer,
    expires_at timestamptz,
    created_at timestamptz not null default now()
);
create index invites_guild_idx on invites (guild_id);
