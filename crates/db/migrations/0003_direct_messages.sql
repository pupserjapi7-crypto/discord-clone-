-- Project Nova — Direktnachrichten (v0.6).
-- Nur neue Tabellen: bestehende Clients und Daten bleiben gültig.
--
-- Ein DM-Kanal gehört genau zwei Personen. Das Paar wird sortiert gespeichert
-- (`user_low < user_high`), damit es pro Paar nur einen Kanal gibt.

create table dm_channels (
    id              bigint primary key,
    user_low        bigint not null references users(id) on delete cascade,
    user_high       bigint not null references users(id) on delete cascade,
    created_at      timestamptz not null default now(),
    last_message_at timestamptz not null default now(),
    check (user_low < user_high),
    unique (user_low, user_high)
);
create index dm_channels_high_idx on dm_channels (user_high);

create table dm_messages (
    id         bigint primary key,
    dm_id      bigint not null references dm_channels(id) on delete cascade,
    author_id  bigint not null references users(id) on delete cascade,
    content    text not null,
    created_at timestamptz not null default now()
);
create index dm_messages_dm_idx on dm_messages (dm_id, id desc);
