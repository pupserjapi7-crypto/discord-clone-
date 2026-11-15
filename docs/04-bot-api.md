# Bot-API

Bots sind eigene Konten mit `bot: true`. Sie sprechen dieselbe REST-API und
dasselbe Gateway wie die App, authentifizieren sich aber mit einem Token statt
mit Passwort. Ein lauffähiges Beispiel liegt in
[`examples/echo-bot`](../examples/echo-bot/src/main.rs).

## 1. Bot anlegen

Mit einem normalen Nutzerkonto (Bearer-Token aus dem Login):

```http
POST /api/v1/bots
Authorization: Bearer <access-token>
Content-Type: application/json

{ "handle": "echo_bot", "display_name": "Echo" }
```

Antwort (`201`):

```json
{ "id": "247…", "handle": "echo_bot", "display_name": "Echo", "token": "nb_9f2c…" }
```

Das Token erscheint **nur in dieser Antwort**. Gespeichert wird nur der
SHA-256-Hash. Höchstens 5 Bots pro Konto.

| Aktion | Endpunkt |
|---|---|
| Eigene Bots auflisten | `GET /api/v1/bots` |
| Token erneuern (altes sofort ungültig) | `POST /api/v1/bots/{bot_id}/token` |
| Bot einer Guild hinzufügen (`MANAGE_GUILD`) | `PUT /api/v1/guilds/{guild_id}/bots/{bot_id}` |

## 2. Authentifizierung

```http
Authorization: Bot nb_<64 Hex-Zeichen>
```

Im Gateway geht das Bot-Token im `hello` an die Stelle des Access-Tokens.
Bots können keine Bots anlegen (`403`) und haben kein Passwort.

## 3. REST-Endpunkte für Bots

Alle IDs sind in JSON Strings (64-Bit-Snowflakes). Rechte prüft der Server wie
bei Menschen: Rollen, Kanal-Overrides, Timeouts.

| Methode | Pfad | Recht |
|---|---|---|
| `GET` | `/api/v1/guilds` | – |
| `GET` | `/api/v1/guilds/{id}` | Mitglied |
| `GET` | `/api/v1/channels/{id}/messages?before=&limit=` | `READ_MESSAGE_HISTORY` |
| `POST` | `/api/v1/channels/{id}/messages` `{content, reply_to?}` | `SEND_MESSAGES` |
| `DELETE` | `/api/v1/channels/{id}/messages/{msg}` | eigene oder `MANAGE_MESSAGES` |
| `PUT`/`DELETE` | `/api/v1/channels/{id}/pins/{msg}` | `MANAGE_MESSAGES` |
| `POST` | `/api/v1/channels/{id}/messages/{msg}/threads` `{name}` | `CREATE_THREADS` |
| `POST` | `/api/v1/guilds/{id}/members/{user}/kick` | `KICK_MEMBERS` + Hierarchie |
| `PUT` | `/api/v1/guilds/{id}/members/{user}/timeout` `{seconds}` | `TIMEOUT_MEMBERS` + Hierarchie |
| `PUT`/`DELETE` | `/api/v1/guilds/{id}/bans/{user}` | `BAN_MEMBERS` + Hierarchie |

## 4. Rate-Limits

- Nachrichten (REST und Gateway): Token-Bucket pro Konto, Standard 1/s mit
  Burst 10 (`NOVA_MSG_RATE`, `NOVA_MSG_BURST`). Überschreitung: `429`
  (REST) bzw. Fehler `rate_limited` (Gateway).
- Slowmode eines Kanals gilt auch für Bots (`429` bzw. `slowmode`), außer sie
  haben `MANAGE_MESSAGES`.

## 5. Gateway

WebSocket `wss://<server>/gateway`, nur Binär-Frames:

```text
op:u8 | flags:u8 | seq:u32 BE | len:u32 BE | MessagePack-Body (benannte Felder)
```

Ablauf:

1. `op 1 HELLO` mit `{"t":"hello","d":{"token":"nb_…","resume":null}}`
2. Server antwortet `op 2 HELLO_OK` (`session_id`, `user`, `guilds`,
   `heartbeat_secs`).
3. `op 9 REQUEST` mit `subscribe` für jeden Kanal, dessen Nachrichten der Bot
   bekommen soll.
4. Alle `heartbeat_secs` (≤ 30 s) `op 9 REQUEST` mit `heartbeat`.
5. Events kommen als `op 8 DISPATCH` mit fortlaufender `seq`.

Am einfachsten nutzt ein Rust-Bot direkt `nova_common::protocol` (wie das
Beispiel). Die Typen `ClientOp`, `ServerOp` und `Event` sind die echten
Server-Typen.

### Events

| `t` | Inhalt | ab |
|---|---|---|
| `message_create` | `message` | 0.1 |
| `message_update` | `message` | 0.1 |
| `message_delete` | `channel_id`, `message_id` | 0.1 |
| `typing_start` | `channel_id`, `user_id`, `display_name` | 0.1 |
| `reaction_add` / `reaction_remove` | `channel_id`, `message_id`, `user_id`, `emoji` | 0.1 |
| `presence_update` | `guild_id`, `user_id`, `status` | 0.1 |
| `guild_update` | `guild_id` – Kanäle/Rollen/Rechte neu laden | 0.4 |
| `guild_member_remove` | `guild_id`, `user_id` | 0.4 |
| `guild_member_update` | `guild_id`, `user_id`, `timeout_until` | 0.4 |
| `message_pin` | `channel_id`, `message_id`, `pinned` | 0.4 |
| `voice_state_update` | `guild_id`, `channel_id`, `user_id`, `connected`, `muted`, `deafened`, `screen` | 0.4 |
| `dm_message_create` | `message` (`channel_id` = DM-Kanal) – nur an beide Beteiligten; Bots haben keine DMs | 0.6 |

Unbekannte Events müssen ignoriert werden – der Server fügt neue hinzu, ohne
die Protokollversion zu ändern.

## 6. Beispiel ausführen

```bash
NOVA_SERVER=https://85-215-175-202.sslip.io \
NOVA_BOT_TOKEN=nb_… \
NOVA_CHANNELS=<kanal-id> \
cargo run -p echo-bot
```

Im Kanal antwortet der Bot auf `!ping` mit `pong` und auf `!echo Hallo` mit
`Hallo`. Nachrichten anderer Bots ignoriert er, damit sich zwei Bots nicht
endlos antworten.
