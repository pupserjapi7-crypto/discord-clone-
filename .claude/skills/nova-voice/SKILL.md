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
- **Limits**: `NOVA_SFU_MAX_USERS` pro Raum; höchstens 8 gleichzeitige
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
- **Simulcast**: nur die hohe Schicht (`rid = "h"`) wird weitergeleitet, siehe
  `docs/DECISIONS.md`.

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

1. SFU-Weiterleitung: Zeit zwischen `Input::Receive` und `writer.write` loggen
   (`debug`) bzw. Metrik `sfu_forward_us`. Budget < 5 ms pro Paket.
2. Ende-zu-Ende: zwei Clients, Click-Track im Sender, Aufnahme beim Empfänger,
   Kreuzkorrelation. Budget < 100 ms. Ergebnis mit Datum in
   `docs/03-roadmap.md` eintragen – nie schätzen.
3. Was in der Automation nicht geht (echtes Mikrofon, `getDisplayMedia`), in
   `STATUS.md` als „nicht gemessen“ führen.

## Checkliste

- [ ] Keine zusätzliche Pufferung im SFU-Pfad.
- [ ] Neue Spuren lösen Neuverhandlung über den DataChannel aus.
- [ ] Verlassen räumt Elemente, Analyser und Spuren auf.
- [ ] Messwerte in `docs/03-roadmap.md` aktualisiert.
