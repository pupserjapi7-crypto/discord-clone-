---
name: nova-security
description: Sicherheitsregeln für Auth, Rechte, Uploads, Moderation und Bot-API in Project Nova. Laden bei jeder Änderung an Authentifizierung, Rechteprüfung, Rate-Limits, Uploads, Moderationsaktionen oder Bot-Tokens.
---

# Nova-Sicherheit

## Authentifizierung

- Passwörter: Argon2id (`hash_password_limited` in `crates/api`), Parameter aus
  Config (`NOVA_ARGON2_*`). Hashing läuft hinter einem Semaphor
  (`hash_limit`), damit Gateway und SFU nicht verhungern.
- Access-Token: kurzlebig, im Speicher (`TokenStore`), als Bearer oder Cookie
  `nova_at`. Refresh-Token: zufällig, nur SHA-256 in `sessions.token_hash`,
  Cookie `nova_rt` (HttpOnly, SameSite=Lax, Secure bei https). Rotation bei
  jedem Refresh; Wiederverwendung eines rotierten Tokens widerruft die ganze
  Familie.
- Bot-Tokens: Präfix `nb_`, nur SHA-256 in `bot_tokens`, Header
  `Authorization: Bot <token>`. Klartext wird genau einmal beim Anlegen
  zurückgegeben.

## Rechte – immer serverseitig

- Reihenfolge in jedem Handler: Ressource laden → **Mitgliedschaft prüfen** →
  `effective_permissions(guild, user, channel)` → Bit prüfen.
- `effective_permissions` liefert für Nicht-Mitglieder und gebannte Nutzer
  `Permissions::empty()`. Owner und `ADMINISTRATOR` bekommen alles.
- Kanal-Overrides: `@everyone` → Rollen → Nutzer; Deny vor Allow pro Stufe.
- Moderation gegen andere Mitglieder zusätzlich mit Hierarchie:
  `can_moderate(actor, target)` – niemand moderiert den Owner, sich selbst oder
  jemanden mit gleich hoher oder höherer Rolle.
- Timeout (`guild_members.timeout_until`): blockiert Senden, Reaktionen,
  Tippen und Voice-Beitritt; Lesen bleibt erlaubt.
- Nach jeder Rechte-/Rollenänderung `PermCache::invalidate_guild` aufrufen
  (Gateway cacht 30 s).
- Jede Moderationsaktion schreibt einen Eintrag in `audit_log`.

## Eingaben und Limits

- Validierung zentral in `crates/common/src/validate.rs` (Handle, Anzeigename,
  Kanal-/Servername, Nachrichtenlänge, Emoji).
- Rate-Limits: Auth über `RateLimiter` pro IP/Handle; Nachrichten pro Session
  über `Bucket` (`NOVA_MSG_RATE`/`NOVA_MSG_BURST`); REST-Nachrichten und Bots
  über `RateLimiter` mit Schlüssel `msg:{user_id}`; Slowmode pro Kanal und
  Nutzer im Gateway (Bypass mit `MANAGE_MESSAGES`).
- Uploads: Größenlimit aus Config, Content-Addressing per SHA-256 hinter
  `BlobStore`, Download nur für Mitglieder des Kanals.
- SQL nur über sqlx-Makros (Prepared Statements).

## Web

- CORS nur für `NOVA_PUBLIC_ORIGIN`, Credentials an.
- CSP der Tauri-Hülle nicht auf `null` lassen, wenn lokale Assets geladen werden.
- Keine Secrets in Logs (Tokens, Passwörter, Refresh-Cookies).

## Checkliste vor dem Commit

- [ ] Jeder neue Handler prüft Mitgliedschaft **und** Recht.
- [ ] Moderation prüft Hierarchie und schreibt Audit-Log.
- [ ] Neue Eingaben validiert, Längen begrenzt.
- [ ] Integrationstest für den verbotenen Fall (403/404) vorhanden.
