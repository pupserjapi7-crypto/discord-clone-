---
name: nova-voice
description: Arbeit am Voice- und Screenshare-Pfad von Project Nova – str0m-SFU, WebRTC im Browser über web-sys, Audio, Screenshare, Latenzmessung. Laden bei Änderungen an crates/voice-sfu, crates/ui/src/voice.rs oder den Voice-Endpunkten.
---

# Nova-Voice

## Aufbau

- **SFU** `crates/voice-sfu`: ein Raum = ein OS-Thread (`run_room`) + ein UDP-Port
  aus `NOVA_SFU_PORT_START..END` (Standard 40000–40100). str0m mit
  `set_ice_lite(true)`, ein Host-Kandidat mit `NOVA_SFU_PUBLIC_IP` (Pflicht hinter
  NAT, sonst scheitert ICE still).
- **Signalisierung**: Erstes Offer per REST `POST /channels/{id}/voice/join`
  (Body `{"sdp": "<JSON von RTCSessionDescription>"}`), Antwort `{"sdp": …}`.
  Neuverhandlungen (neue Spuren, Screenshare) laufen über den DataChannel `sdp`.
- **Weiterleitung**: reines RTP-Fanout, kein Mixing/Transcoding.
  `Propagated::MediaData` → `handle_media_data_out` für jeden anderen Client.
- **Stream-IDs**: ausgehende Spuren tragen als Stream-ID `u{user_id}`. Der Client
  ordnet darüber Audio-/Video-Elemente Personen zu (Lautstärke, Sprecher-Ring).
- **Limits**: `NOVA_VOICE_MAX_USERS` pro Raum (Standard 15); höchstens 8 gleichzeitige
  Videospuren (Screenshare) pro Raum (`MAX_VIDEO_TRACKS`).
- **Codecs**: `rtc_config()` im SFU erlaubt nur Opus und H.264 Constrained
  Baseline (PT 108, `42e01f`, `packetization-mode=1`). Der native Client nutzt
  dieselbe Konfiguration (`crates/client-native/src/voice.rs`) und OpenH264
  (`src/video.rs`). Neue Codecs nur, wenn wirklich jeder Client sie kann –
  der SFU transkodiert nicht.
- **Nativer Stream**: `voice/stream.rs` – Aufnahme- und Kodier-Thread,
  Dekodier-Thread, Videospur per eigenem Offer über den DataChannel. Testen
  ohne Klicks mit Debug-Build: `NOVA_DEV_VOICE=1` (+ `NOVA_DEV_SHARE=1` bzw.
  `NOVA_DEV_WATCH=1`).
- **Rollen**: Offers über den DataChannel tragen `"roles": {"<mid>": "screen" |
  "screen-audio" | "camera"}`. Stream-IDs nach Rolle: `u{id}` Mikrofon und
  Bildschirm, `s{id}` Bildschirm-Ton, `c{id}` Kamera (`TrackRole::stream_id`).
  Nie bestehende Rollen auf andere Präfixe legen – ältere Clients erkennen nur `u`.
- **Simulcast**: Browser senden `h`/`m`/`l` (`simulcast_encodings` in
  `crates/ui/src/voice.rs`). Der SFU hält pro Empfänger und Spur `Layers`
  (Wunsch, aktive Schicht, zuletzt gesehen); Wechsel am Schlüsselbild, Rückfall
  nach 1,5 s ohne Pakete. Empfänger wählen mit
  `{"type":"layer","mid":"…","rid":"h|m|l"}` (`apply_layers`). Ohne Wunsch: `h`.
- **Ereignisse**: Was der SFU selbst bemerkt (Videospur da/weg, Abriss), geht
  über `Sfu::take_events` → `nova_api::spawn_sfu_events` → `voice_state_update`.
  Neue Flags in `Participant` immer dort mitschicken.
- **Kamera**: Web-Client, 720p30 mit Simulcast, eigene Vorschau gespiegelt in
  `#cslot-{id}`. Der native Client zeigt Kameras (noch) nicht an.

## Testen in der Cloud-Sandbox

- ICE klappt nur, wenn der SFU die Container-IP ankündigt
  (`NOVA_SFU_PUBLIC_IP=192.0.2.2` o. ä., siehe `/etc/hosts`); `127.0.0.1`
  scheitert still (Chrome sendet von der Container-IP).
- Chromium mit `--use-fake-ui-for-media-stream --use-fake-device-for-media-stream`
  liefert Fake-Mikrofon und -Kamera; mit
  `--auto-select-desktop-capture-source=Entire screen` auch `getDisplayMedia`.
- Empfangene Auflösung prüfen: `getStats()` → `inbound-rtp` (`frameWidth`),
  gesendete Schichten: `outbound-rtp` (`rid`).

## Client (Web, `crates/ui/src/voice.rs`)

- Mikrofon: `getUserMedia` mit `echoCancellation`, `noiseSuppression`,
  `autoGainControl` (Rauschunterdrückung über Browser-Constraints).
- Jitter-Buffer: macht der Browser (NetEq, adaptiv). Kein eigener Buffer im SFU –
  der SFU puffert nicht, damit keine Latenz entsteht.
- Ping: `RTCPeerConnection.getStats()` → `candidate-pair` mit
  `state == "succeeded"` → `currentRoundTripTime` × 1000. Anzeige grün < 60 ms,
  gelb < 120 ms, sonst rot.
- Sprecher-Erkennung: `AnalyserNode` pro Remote-Stream, RMS alle 100 ms,
  Schwelle mit Haltezeit (kein Flackern).
- Lautstärke pro Person: `HTMLMediaElement.volume` des Audio-Elements der
  Stream-ID `u{user_id}`.
- Push-to-Talk: Taste aus den Einstellungen; Mikrofonspur `enabled` nur solange
  gedrückt.
- Reconnect: `iceconnectionstatechange` → `failed`/`disconnected` > 2 s →
  `voice::join` erneut mit demselben Kanal.

## Latenz messen

1. SFU-Weiterleitung: Metrik `nova_sfu_forward_*` in `/metrics` (nur lokal:
   `curl http://127.0.0.1:8080/metrics`), gemessen von `network_time` des
   Pakets bis `writer.write`. Budget < 5 ms pro Paket. Last dazu:
   `nova seed --users 30` und `nova-loadtest --mode voice --users 30 --guild
   <id> --server-pid <pid>` (echte str0m-Verbindungen, Laufzeit durch den SFU,
   Verlust, CPU/RAM). **Nur Release-Builds messen** – im Debug-Build schafft
   der SFU keine 30 Personen. Für mehr als 15 Personen pro Raum
   `NOVA_VOICE_MAX_USERS` setzen.
2. Der Raum-Thread muss im Leerlauf schlafen: `top -H -p <pid>` zeigt
   `sfu-<kanal>` bei ~0 %. (Bis 0.8 lief er wegen eines nicht blockierenden
   Sockets dauerhaft auf 100 %.)
3. Ende-zu-Ende: zwei Clients, Click-Track im Sender, Aufnahme beim Empfänger,
   Kreuzkorrelation. Budget < 100 ms. Ergebnis mit Datum in
   `docs/03-roadmap.md` eintragen – nie schätzen.
4. Was in der Automation nicht geht (echtes Mikrofon, `getDisplayMedia`), in
   `STATUS.md` als „nicht gemessen“ führen.

## Checkliste

- [ ] Keine zusätzliche Pufferung im SFU-Pfad.
- [ ] Neue Spuren lösen Neuverhandlung über den DataChannel aus.
- [ ] Verlassen räumt Elemente, Analyser und Spuren auf.
- [ ] Messwerte in `docs/03-roadmap.md` aktualisiert.
