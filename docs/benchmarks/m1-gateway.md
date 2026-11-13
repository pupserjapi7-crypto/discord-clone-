# M1 — Messprotokoll Gateway (echte Messung)

**Datum:** 2026-11-13
**Umgebung:** Windows-Entwicklungsrechner (16 logische Kerne), **nicht der VPS**; Server und Lastgenerator laufen auf derselben Maschine. Release-Build, PostgreSQL 17.6 lokal.
**Binary:** `nova` (Windows, Release, eingebettetes Frontend), Loadtest `nova-loadtest`.

> Alle Zahlen sind lokal gemessen. Der VPS-Lauf mit getrennten Maschinen folgt in M1-Abnahme (Schritt 7).

## 1. Verbindungsaufbau: 1.000/1.000

| Lauf | Backlog | Ergebnis |
|---|---|---|
| vorher | std-Default (128) | 931/1000, 69 × `WSAECONNABORTED` beim Connect |
| nachher | `NOVA_LISTEN_BACKLOG=4096` (socket2) | **1000/1000, 0 Fehler** |

**Ursache:** `std::net::TcpListener` übergibt einen Backlog von 128 an `listen(2)`. 1.000 gleichzeitige SYNs nach der Login-Phase überlaufen die Accept-Queue; Windows beantwortet den Rest mit RST. Der Fix bindet den Listener explizit mit `socket2` und Backlog 4096; der Server loggt `backlog=4096`. Kein Client-Retry nötig — die Server-Seite war die Ursache.

## 2. Realistischer Lasttest: 20 Sender × 1.000 Empfänger

Getrennte Prozesse (`--mode senders` / `--mode receivers`), 60 s, 1 Nachricht/s pro Sender, ein Kanal:

| Metrik | Wert |
|---|---|
| Verbindungen | 1.020 (1.000 Empfänger + 20 Sender) |
| Nachrichten | 1.200 gesendet, 1.200 bestätigt |
| Empfangene Events | **1.200.000** (exakt 1.200 × 1.000) |
| Sequenz-Gaps / Resyncs / Reconnects | **0 / 0 / 0** |
| Backpressure-Trennungen (steady state) | **0** |
| End-to-End ACK (Sender) | p50 10,0 ms · p95 20,6 ms · **p99 181 ms** |
| Server-Dispatch (Enqueue an alle) | p50 0,6 ms · p95 3,1 ms · **p99 16,9 ms** |
| Server-Handle inkl. DB-Insert | p50 7,0 ms · p95 19,7 ms · p99 158 ms |
| Connect | max 44 ms |

**Einordnung:** p50/p95 liegen deutlich unter dem 50-ms-Ziel; **p99 181 ms verfehlt das 150-ms-Ziel** in diesem Ein-Box-Aufbau. Ursache ist der Ausreißer im DB-Insert/Persist-Pfad (Server-p99 158 ms bei gleichem Zeitfenster), nicht der Fan-out (Dispatch-p99 17 ms). Auf dem VPS mit getrennter Lastmaschine und lokaler PG-Verbindung ist das erneut zu messen; erst dann gilt das Gate als bestanden.

## 3. 1.000er Burst (alle senden, alle empfangen)

`--mode senders --senders 1000`, 10 s: 1000/1000 verbunden, 1.000 Nachrichten, 1,95 Mio. Frames. Erwartungsgemäß liefen 742 Sessions in Backpressure (siehe §4) und wurden getrennt; E2E-p50 222 ms. Dieser Test ist ein absichtlicher Overload, keine Abnahme.

## 4. Backpressure: kein stilles Droppen

- Bei vollem Outbox (256 Frames) wird die Session markiert, der Writer sendet **Close 1013 (try again later)**, der Client verbindet sich mit `session_id` + `last_seq` neu.
- Der Resume-Ring (128 Frames/32 KB) replayt die Lücke. Reicht er nicht, sendet der Server **`OP_RESYNC`** mit `oldest_seq`; der Client lädt Historie neu. Es gibt keinen Pfad mehr, der Frames still verwirft (`nova_ws_frames_dropped_total` wurde entfernt).
- Sequenzvergabe und Einreihung sind pro Session serialisiert (`dispatch_order`); vorher traten durch parallele Dispatcher 1.162 Umordnungen/„Gaps" auf, jetzt 0.
- Tests: `crates/gateway` — `full_outbox_lags_session_and_keeps_frame_for_resume`, `resume_inside_window_replays_everything_after_last_seq`, `resume_before_ring_start_reports_the_gap`, `reconnect_never_returns_a_silent_gap`, `evicted_ring_forces_explicit_resync`.
- **Grenze des E2E-Slow-Tests:** Ein künstlich lesender Client ließ sich lokal nicht zuverlässig in den Überlauf zwingen, weil Windows-Socketpuffer mehrere Hundert KB schluckten; der Test ist deshalb nicht Teil der CI. Der Mechanismus ist über den Unit-Test mit voller Queue deterministisch abgedeckt.

## 5. Argon2

Konfigurierbar über `NOVA_HASH_CONCURRENCY` (Default: Kerne − 1, min. 1); jede Hash-/Verify-Operation hält ein `Semaphore`-Permit und läuft in `spawn_blocking`.

| Lauf | Ergebnis |
|---|---|
| 1.000 Logins, Concurrency 15 (16 Kerne) | 3,3 s (304/s), p50 80 ms · p95 103 ms · p99 107 ms |
| 300 Logins, **Concurrency 1** (2-vCore-Emulation) | 3,6 s (83/s), p50 192 ms · p99 393 ms |
| `/healthz` **während** des 1-Slot-Bursts | p50/p95/p99 **< 1 ms**, 250 Requests |

**Parameter:** Argon2id, m = 19.456 KiB (19 MiB), t = 2, p = 1 — OWASP-Empfehlung. Auf 2 vCores/2 GB bewusst **ein** Permit: 19 MiB pro Hash, 2 parallele Hashes würden beide Kerne binden und den Gateway-Jitter treiben. Der Login-Burst ist die einzige Lastspitze; 83 Logins/s auf einem Slot sind für einen Community-Server ausreichend, und der Gateway blieb nachweislich reaktionsfähig.

## 6. IDs und JSON

Snowflake-IDs (~2,5 × 10¹⁷) überschreiten `Number.MAX_SAFE_INTEGER`; JSON rundete sie und die UI fragte falsche Guild-/Channel-IDs ab (404). Die JSON-API serialisiert `id`/`*_id` jetzt als String; MessagePack bleibt i64, und Client-Operationen akzeptieren IDs als Zahl oder String. Der Playwright-Smoke deckt den Pfad ab.

## 7. Frontend-Smoke (Playwright)

`web/e2e/smoke.spec.ts`: Registrieren → Logout → Login (`load0000`) → Guild/Kanal → Nachricht senden → zweiter Browser-Kontext (`load0001`) sieht sie → Netzwerk trennen/verbinden → Antwort kommt nach Resume an. **1 Test, grün in 2,4 s** (Chromium, Vite-Dev-Server gegen lokalen Backend).

## Offen

- VPS-Lauf mit getrennter Lastmaschine (p99-Ziel, RSS/CPU unter Linux).
- Musl-Build und CI sind vorbereitet, aber ohne GitHub-Remote nicht ausgeführt.
