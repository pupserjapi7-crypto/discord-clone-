# Nova — Status

**Stand:** 14.11.2026 · **Version:** 0.4.0 (in Arbeit, Master-Prompt v0.2.2 → v1.0)
**Live:** <https://85-215-175-202.sslip.io> (läuft noch mit v0.2.2)

Fortsetzen: „mach weiter laut STATUS.md“ – der Abschnitt *Nächste Schritte*
ist die Arbeitsliste.

---

## Bestandsaufnahme (Abschnitt 0 des Master-Prompts)

Lauf am 14.11.2026 auf Windows: `dx build`, fmt, clippy (Workspace + WASM),
Tests und `cargo build --release -p nova` waren **alle grün**. WASM 839 KB
unkomprimiert.

### Wirklich geprüft

- Text-Chat, Auth (Argon2id, Refresh-Rotation mit Reuse-Erkennung), Gateway mit
  Resume – jetzt durch Integrationstests gegen einen echten Server abgedeckt
  (`crates/nova/tests/api.rs`).
- Nativer Desktop-Client 0.3.0: Login, Kanäle, Nachrichten, Update-Prüfung gegen
  GitHub; lokal gemessen ~22–30 MB RAM, erste Ansicht ~10 ms.
- Release-Workflow v0.3.0: alle 6 Jobs grün, Assets für Server, Tauri und
  nativen Client inkl. `.sha256`.

### Nur behauptet (nicht nachgewiesen)

| Behauptung | Wirklichkeit |
|---|---|
| Voice funktioniert | Nur ICE/DTLS im Zwei-Client-Test verbunden. Dass **Audio beim Gegenüber ankommt**, ist nicht gemessen; Mouth-to-Ear unbekannt. |
| Screenshare 1080p60 | Nur synthetischer Video-Track; echte Display-Aufnahme nie getestet. |
| „Uploads hinter `BlobStore`“ (README) | **Nicht implementiert.** Es gibt Tabelle und URL-Schema, aber keinen Upload-Endpunkt, keinen `BlobStore`, kein Ausliefern. |
| Installer funktionieren | Gebaut, aber nie installiert; unsigniert. |

### Gefundene Bugs

| Bug | Status |
|---|---|
| **Sicherheit:** `effective_permissions` prüfte keine Mitgliedschaft. Über das Gateway konnte jeder angemeldete Nutzer mit bekannter Kanal-ID fremde Kanäle abonnieren und hineinschreiben. | **behoben** (v0.4), Regressionstest `gateway_rejects_non_members` |
| CI wandte nur `0001_init.sql` an | behoben: alle Migrationen |
| `crates/ui/dist` sammelte alte WASM-Dateien an (dx räumt nicht auf) | behoben in `cargo xtask ui` |
| Web-UI prüft Passwort ≥ 8, Server verlangt ≥ 10 | offen → P5 |
| Web-UI klont den ganzen State bei jedem Render, scrollt bei jedem Render nach unten, ordnet eigene Nachrichten über den Text statt die Nonce zu | offen → P4 |
| Web-UI fragt Voice-Teilnehmer alle 4 s per HTTP ab | Server sendet jetzt `voice_state_update`; Client-Umbau offen → P2 |
| Screenshare-Stopp nimmt die Spur nicht aus der PeerConnection, fremde Video-Elemente bleiben nach Verlassen stehen | offen → P2 |
| Tauri-Hülle: `csp: null`, lädt die Live-URL fest verdrahtet | offen → P5 |

---

## Erledigt in diesem Durchlauf

### P1 – Stabilisieren
- Rechte-Lücke geschlossen (s. o.); Nicht-Mitglieder und Gebannte bekommen
  keine Rechte.
- `cargo xtask ui|gallery|build|check|all` ersetzt die Shell-Kette
  (`xtask/`, Alias in `.cargo/config.toml`).
- 7 Integrationstests: Auth inkl. Refresh-Rotation, Rechte, Nachrichten-CRUD,
  Pins, Threads, Moderation, Bots, Gateway-Roundtrip, Gateway-Abweisung von
  Nicht-Mitgliedern. Dazu Protokoll-Tests für alle neuen Events und für
  Abwärtskompatibilität alter `MessageView`s.
- `CLAUDE.md` und 7 Projekt-Skills unter `.claude/skills/`.

### P3 – Server-Seite
- Migration `0002`: Timeouts, Banns, Audit-Log, Slowmode, Threads
  (`kind = 2`), Pins, Bot-Konten und Bot-Tokens. Nur additiv.
- Moderation: Kick, Bann/Entbannen, Timeout (entzieht Schreiben/Sprechen),
  Rollen anlegen/ändern/löschen/zuweisen, Kanal-Overrides, Slowmode,
  Nachrichten löschen und anpinnen, Audit-Log – alles mit Hierarchie-Prüfung
  und Audit-Eintrag.
- Threads aus Nachrichten, Archivieren, Ungelesen-Stand pro Kanal in
  `GET /guilds/{id}`.
- Bot-API mit `nb_`-Tokens für REST und Gateway, Doku `docs/04-bot-api.md`,
  Beispiel `examples/echo-bot`.
- Neue Gateway-Events (`guild_update`, `guild_member_remove`,
  `guild_member_update`, `message_pin`, `voice_state_update`) – ältere Clients
  ignorieren sie.

### P2 – Server-Seite
- SFU vergibt Stream-IDs `u{user_id}` (Zuordnung von Ton/Bild zu Personen).
- Höchstens 8 gleichzeitige Videospuren pro Raum.
- `POST /channels/{id}/voice/state` (stumm/taub/Bildschirm) und
  `voice_state_update`-Events statt Polling; Beitritt verlangt `CONNECT`.

---

## Nächste Schritte

1. **Web-UI (P4 + P2-Client + P3-UI + P5):** Token-Migration auf `--nv-*`,
   Komponenten aufteilen, Galerie `/dev/gallery`, Moderations-UI,
   Thread-Seitenleiste, Ungelesen-Badges, Voice-Grid mit Sprecher-Ring,
   Lautstärke pro Person, Push-to-Talk, Ping, Einstellungen, Onboarding.
2. Screenshots nach `docs/screenshots/` (Dark/Light).
3. Uploads mit `BlobStore` nachrüsten (README-Versprechen).
4. Tauri: CSP, Tray, Benachrichtigungen.
5. Messwerte erheben und in `docs/03-roadmap.md` eintragen; Version, Commit.

---

## Starten

```bash
cargo run -p nova -- migrate
cargo run -p nova -- serve
cargo xtask check
```
