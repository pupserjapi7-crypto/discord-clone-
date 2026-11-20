-- Project Nova — Forum-Kanäle und Webhooks (v0.8).
-- Nur eine neue Tabelle und ein Index; Forum-Kanäle sind `channels.kind = 3`
-- (die Spalte gibt es schon). Ältere Server ignorieren beides.

-- Eingehende Webhooks: externe Dienste posten mit einem geheimen Token in
-- einen Kanal. Jeder Webhook hat ein eigenes Bot-Konto (`user_id`), damit
-- seine Nachrichten wie gewohnt einen Absender haben. Vom Token wird nur der
-- SHA-256 gespeichert.
create table webhooks (
    id         bigint primary key,
    guild_id   bigint not null references guilds(id) on delete cascade,
    channel_id bigint not null references channels(id) on delete cascade,
    user_id    bigint not null references users(id) on delete cascade,
    name       text not null,
    token_hash bytea not null,
    creator_id bigint references users(id) on delete set null,
    created_at timestamptz not null default now()
);
create index webhooks_channel_idx on webhooks (channel_id);
