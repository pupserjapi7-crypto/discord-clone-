---
name: nova-protocol
description: Gateway-Events, REST-Endpunkte und gemeinsame Typen in Project Nova ändern, ohne ausgelieferte Clients zu brechen. Laden bei jeder Änderung an crates/common/src/protocol.rs, crates/api/src/lib.rs oder crates/gateway.
---

# Nova-Protokoll

## Wo was liegt

- Wire-Format und alle Typen: `crates/common/src/protocol.rs`. Ein Frame ist
  `op:u8 | flags:u8 | seq:u32 BE | len:u32 BE | body`, Body ist MessagePack mit
  benannten Feldern (`rmp_serde::to_vec_named`). `MAX_FRAME_BYTES` = 32 KiB.
- `ClientOp`, `ServerOp`, `Event` sind `#[serde(tag = "t", content = "d")]`,
  Varianten `snake_case`.
- REST: `crates/api/src/lib.rs`, Router in `router()`. JSON-Antworten laufen
  durch `json_ids()`, das jedes `id`/`*_id` als String ausgibt (JavaScript kann
  keine 64-Bit-Ints). Requests akzeptieren IDs als Zahl **oder** String
  (`id_compat` im Protokoll, `deserialize_with` im Client).
- Gateway-Verteilung: `Gateway::dispatch_channel` (Abonnenten eines Kanals),
  `dispatch_guild` (alle Sessions einer Guild). Von der API aus über die
  öffentlichen `Gateway::publish_*`-Methoden.

## Kompatibilität (drei Clients im Feld)

1. Web-UI (`crates/ui`) liest Dispatches tolerant: unbekanntes `t` → ignoriert.
2. Nativer Client (`crates/client-native`) dekodiert `Event` direkt; unbekannte
   Varianten scheitern beim Dekodieren und werden mit `if let Ok` übergangen.
3. Bots (`examples/echo-bot`) wie der native Client.

Daraus folgt:
- **Neue** Event-Varianten und **neue** optionale Felder (`#[serde(default)]`)
  sind immer erlaubt.
- Bestehende Felder nie umbenennen, Typ nie ändern, Varianten nie entfernen.
- Neue `ClientOp`-Varianten nur anhängen; der Server muss alte Clients weiter
  bedienen.
- Opcodes (`OP_*`) sind fix.

## Neues Event anlegen

1. Variante in `Event` ergänzen und in `Event::kind()` eintragen.
2. Serverseitig an der Stelle versenden, wo die Änderung in der DB festgeschrieben
   ist (erst DB, dann Dispatch).
3. Roundtrip-Test in `protocol.rs` (`encode` → `decode` → `decode_body`).
4. Web-UI: in `crates/ui/src/protocol.rs` `parse_dispatch` erweitern.
5. Doku: `docs/04-bot-api.md` (Event-Tabelle).

## Neuer REST-Endpunkt

1. Route in `router()`, Handler mit `auth: Auth` (Nutzer oder Bot).
2. Mitgliedschaft prüfen, dann `effective_permissions` – siehe `nova-security`.
3. Antwort über `json_ids`.
4. Integrationstest in `crates/nova/tests/`.
