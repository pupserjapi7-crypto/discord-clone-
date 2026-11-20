-- Project Nova — Uploads (v0.8).
-- Nur neue Spalten mit Default und neue Indizes: ältere Server und Clients
-- lesen die Tabelle weiter wie bisher.
--
-- Anhänge in Direktnachrichten: `channel_id` verweist auf `channels`, DMs
-- liegen aber in `dm_channels`/`dm_messages`. Deshalb zwei eigene Spalten.

alter table attachments
    add column dm_id bigint default null references dm_channels(id) on delete cascade;
alter table attachments
    add column dm_message_id bigint default null references dm_messages(id) on delete set null;

create index attachments_dm_message_idx on attachments (dm_message_id)
    where dm_message_id is not null;

-- Speicherbereinigung: Blobs ohne Verweis finden, verwaiste Uploads löschen.
create index attachments_sha256_idx on attachments (sha256);
create index attachments_orphan_idx on attachments (created_at)
    where message_id is null and dm_message_id is null;

-- Nachrichten nur aus Anhängen haben leeren Text. Die Prüfung wird gelockert
-- (höchstens 8000 Zeichen statt 1–8000). Vorwärtskompatibel: ältere Server
-- schreiben weiter nur nicht-leere Texte, ältere Clients zeigen leeren Text
-- einfach leer an. Begründung in docs/DECISIONS.md.
alter table messages drop constraint if exists messages_content_check;
alter table messages add constraint messages_content_len
    check (char_length(content) <= 8000);
