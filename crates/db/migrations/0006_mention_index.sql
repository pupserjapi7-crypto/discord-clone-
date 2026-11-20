-- Project Nova — ungelesene Erwähnungen pro Person zählen (v0.8).
-- Nur ein neuer Index. `GET /users/@me/unread` sucht die Erwähnungen einer
-- Person nach ihrem Lesestand; ohne Index müsste Postgres dafür alle
-- Nachrichten seit dem Lesestand durchgehen. Bis v0.7 blieb die Tabelle leer,
-- der Index entsteht also auch auf bestehenden Servern sofort.
create index message_mentions_user_idx on message_mentions (user_id, message_id);
