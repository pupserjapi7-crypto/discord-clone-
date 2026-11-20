---
name: nova-release-check
description: Abschluss-Checkliste für jedes Arbeitspaket in Project Nova – Formatierung, Clippy (Workspace und WASM), Tests, UI-Build, Messungen, STATUS.md/README, Version, Commit. Laden am Ende jedes Pakets und vor jedem Release-Tag.
---

# Release-Check

Reihenfolge einhalten; bei Rot zuerst beheben.

1. **Datenbank bereit**: Postgres läuft, `nova migrate` angewendet
   (sqlx prüft Queries zur Compile-Zeit).
2. **Prüfungen**: `cargo xtask check`
   - `cargo fmt --all --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo clippy -p nova-ui --target wasm32-unknown-unknown --features web -- -D warnings`
   - `cargo test --workspace` (Integrationstests in `crates/nova/tests` brauchen
     `DATABASE_URL`)
3. **UI-Build**: `cargo xtask ui` bei jeder UI-Änderung (siehe `nova-ui-build`).
4. **Messen, nicht schätzen** (Werte in `docs/03-roadmap.md`):
   - WASM-Größe roh und gzip (`crates/ui/dist/assets/*.wasm`)
   - Server-RAM im Leerlauf (`nova serve` lokal, RSS/Working Set nach 10 s)
   - Gateway-Dispatch p99 (`/metrics` oder `cargo run -p nova-loadtest`)
   - nativer Client: RAM und „erste Ansicht“ aus stderr
   - Voice: `nova seed --users 30`, Server mit `NOVA_VOICE_MAX_USERS=30`, dann
     `nova-loadtest --mode voice --users 30 --guild <id> --server-pid <pid>`
     (Verlust, Laufzeit durch den SFU, CPU/RAM; dazu `nova_sfu_forward_*`)
   Budgets: Server Leerlauf < 150 MB, WASM gzip < 2 MB, Dispatch p99 ≤ 5 ms,
   SFU-Weiterleitung p99 < 5 ms, Voice-Verlust 0 %, 30 Personen < 900 MB.
   Nur **Release-Builds** messen und **nicht während eines Builds** (belegte
   Kerne verfälschen Latenzen um ein Vielfaches). `/metrics` antwortet nur
   lokal. `--fetch-metrics` ist ein Schalter ohne Wert.
5. **`nova serve` startet** und die App ist benutzbar (Login, Kanal, Nachricht).
6. **Doku**: `STATUS.md` (was geht, was nicht, Messwerte), README-Statustabelle,
   neue Entscheidungen in `docs/DECISIONS.md`, Betriebsänderungen in
   `docs/06-vps-production.md`.
7. **Version**: `[workspace.package] version` in `Cargo.toml`,
   `desktop/src-tauri/Cargo.toml` und `tauri.conf.json` gleichziehen,
   `cargo update -w --offline`.
8. **Commit** mit aussagekräftiger Nachricht. Tag (`vX.Y.Z`) nur, wenn ein Release
   gewollt ist – der Tag startet den Release-Workflow (Server, Tauri, nativer
   Client mit `.sha256` für den In-App-Updater).
