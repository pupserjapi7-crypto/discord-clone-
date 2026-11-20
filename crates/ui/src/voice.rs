//! WebRTC-Client über `web-sys`. Der SFU leitet RTP nur weiter; jede Spur kommt
//! mit der Stream-ID `u{user_id}` an, darüber ordnen wir Ton und Bild Personen zu.
//!
//! Jitter-Buffer und Echo-/Rauschunterdrückung macht der Browser (NetEq,
//! `getUserMedia`-Constraints); hier wird bewusst nichts zusätzlich gepuffert.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use dioxus::prelude::*;
use gloo_timers::future::TimeoutFuture;
use js_sys::Reflect;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AnalyserNode, AudioContext, DisplayMediaStreamConstraints, Event, HtmlAudioElement,
    HtmlMediaElement, HtmlVideoElement, MediaDevices, MediaStream, MediaStreamConstraints,
    MediaStreamTrack, MessageEvent, RtcDataChannel, RtcIceConnectionState, RtcPeerConnection,
    RtcRtpTransceiver, RtcRtpTransceiverDirection, RtcRtpTransceiverInit, RtcSdpType,
    RtcSessionDescriptionInit, RtcTrackEvent, Window,
};

use crate::{
    api,
    store::Store,
    types::{CaptureProfile, Settings},
};

/// Pegel (RMS), ab dem jemand als sprechend gilt.
const SPEAKING_LEVEL: f32 = 0.018;
/// So lange bleibt der Ring nach dem letzten lauten Frame an (kein Flackern).
const SPEAKING_HOLD_MS: f64 = 350.0;

#[derive(Default)]
struct Remote {
    audio: Option<HtmlAudioElement>,
    /// Bildschirm (Stream-ID `u…`).
    video: Option<HtmlVideoElement>,
    /// Ton der Bildschirmfreigabe (Stream-ID `s…`), ohne Sprecher-Ring.
    screen_audio: Option<HtmlAudioElement>,
    /// Kamera (Stream-ID `c…`).
    camera: Option<HtmlVideoElement>,
    analyser: Option<AnalyserNode>,
    last_loud: f64,
    /// Mids der empfangenen Videospuren und die zuletzt gewünschte Schicht.
    video_mid: Option<String>,
    video_rid: Option<&'static str>,
    camera_mid: Option<String>,
    camera_rid: Option<&'static str>,
}

/// Eigene Bildschirmfreigabe: Bild (Simulcast) und optional Ton.
struct ScreenShare {
    stream: MediaStream,
    video: RtcRtpTransceiver,
    audio: Option<RtcRtpTransceiver>,
}

/// Eigene Kamera mit Vorschau in der eigenen Kachel.
struct CameraShare {
    stream: MediaStream,
    transceiver: RtcRtpTransceiver,
    preview: HtmlVideoElement,
}

struct Session {
    pc: RtcPeerConnection,
    dc: RtcDataChannel,
    local: MediaStream,
    audio: MediaStreamTrack,
    local_analyser: Option<AnalyserNode>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_track: Closure<dyn FnMut(RtcTrackEvent)>,
    _on_ice: Closure<dyn FnMut(Event)>,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static REMOTES: RefCell<HashMap<String, Remote>> = RefCell::new(HashMap::new());
    static SCREEN: RefCell<Option<ScreenShare>> = const { RefCell::new(None) };
    static CAMERA: RefCell<Option<CameraShare>> = const { RefCell::new(None) };
    static AUDIO_CTX: RefCell<Option<AudioContext>> = const { RefCell::new(None) };
    static DEAFENED: Cell<bool> = const { Cell::new(false) };
    static VOLUMES: RefCell<HashMap<String, f64>> = RefCell::new(HashMap::new());
    /// Erhöht sich bei jedem Beitritt/Verlassen; alte Schleifen beenden sich.
    static GENERATION: Cell<u32> = const { Cell::new(0) };
    static LOCAL_LOUD: Cell<f64> = const { Cell::new(0.0) };
}

fn js_error(value: JsValue, context: &str) -> String {
    let detail = value
        .as_string()
        .or_else(|| {
            Reflect::get(&value, &JsValue::from_str("message"))
                .ok()
                .and_then(|message| message.as_string())
        })
        .unwrap_or_else(|| format!("{value:?}"));
    format!("{context}: {detail}")
}

fn media_devices(window: &Window, method: &str) -> Result<MediaDevices, String> {
    let secure = Reflect::get(window.as_ref(), &JsValue::from_str("isSecureContext"))
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let value = Reflect::get(
        window.navigator().as_ref(),
        &JsValue::from_str("mediaDevices"),
    )
    .map_err(|err| js_error(err, "Medien-API nicht lesbar"))?;
    let function = Reflect::get(&value, &JsValue::from_str(method))
        .ok()
        .filter(|function| function.is_function());
    if value.is_undefined() || value.is_null() || function.is_none() {
        return Err(if secure {
            "Dieser Browser stellt keine Medien-API bereit. Bitte einen aktuellen Chrome, Edge oder Firefox verwenden.".to_string()
        } else {
            "Mikrofon und Bildschirmfreigabe brauchen HTTPS oder localhost.".to_string()
        });
    }
    Ok(value.unchecked_into())
}

fn set(object: &js_sys::Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(object, &key.into(), value);
}

fn audio_constraints(settings: &Settings) -> MediaStreamConstraints {
    let constraints = MediaStreamConstraints::new();
    let audio = js_sys::Object::new();
    set(&audio, "echoCancellation", &JsValue::TRUE);
    set(&audio, "autoGainControl", &JsValue::TRUE);
    set(
        &audio,
        "noiseSuppression",
        &JsValue::from_bool(settings.noise_suppression),
    );
    if let Some(device) = &settings.input_device {
        let exact = js_sys::Object::new();
        set(&exact, "exact", &JsValue::from_str(device));
        set(&audio, "deviceId", &exact);
    }
    constraints.set_audio(&audio);
    constraints.set_video(&JsValue::FALSE);
    constraints
}

fn audio_context() -> Option<AudioContext> {
    AUDIO_CTX.with(|ctx| {
        if ctx.borrow().is_none() {
            *ctx.borrow_mut() = AudioContext::new().ok();
        }
        ctx.borrow().clone()
    })
}

fn analyser_for(stream: &MediaStream) -> Option<AnalyserNode> {
    let ctx = audio_context()?;
    let source = ctx.create_media_stream_source(stream).ok()?;
    let analyser = ctx.create_analyser().ok()?;
    analyser.set_fft_size(512);
    source.connect_with_audio_node(&analyser).ok()?;
    Some(analyser)
}

fn level(analyser: &AnalyserNode) -> f32 {
    let mut buffer = vec![0f32; analyser.fft_size() as usize];
    analyser.get_float_time_domain_data(&mut buffer);
    let sum: f32 = buffer.iter().map(|sample| sample * sample).sum();
    (sum / buffer.len().max(1) as f32).sqrt()
}

fn description_from(
    value: &JsValue,
    kind: RtcSdpType,
) -> Result<RtcSessionDescriptionInit, String> {
    let sdp = Reflect::get(value, &JsValue::from_str("sdp"))
        .ok()
        .and_then(|sdp| sdp.as_string())
        .ok_or("SDP fehlt in der Antwort")?;
    let description = RtcSessionDescriptionInit::new(kind);
    description.set_sdp(&sdp);
    Ok(description)
}

/// Rollen der eigenen Spuren für den SFU (`mid` → Rolle). Ohne Angabe hält
/// er Ton für das Mikrofon und Bild für den Bildschirm.
fn track_roles() -> js_sys::Object {
    let roles = js_sys::Object::new();
    let add = |transceiver: &RtcRtpTransceiver, role: &str| {
        if let Some(mid) = transceiver.mid() {
            set(&roles, &mid, &JsValue::from_str(role));
        }
    };
    SCREEN.with(|screen| {
        if let Some(screen) = screen.borrow().as_ref() {
            add(&screen.video, "screen");
            if let Some(audio) = &screen.audio {
                add(audio, "screen-audio");
            }
        }
    });
    CAMERA.with(|camera| {
        if let Some(camera) = camera.borrow().as_ref() {
            add(&camera.transceiver, "camera");
        }
    });
    roles
}

/// Neuverhandlung über den DataChannel (neue/entfernte Spuren). Das Offer
/// trägt zusätzlich die Rollen der Spuren (`roles`).
fn renegotiate(pc: &RtcPeerConnection, dc: &RtcDataChannel) {
    let pc = pc.clone();
    let dc = dc.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let offer = match JsFuture::from(pc.create_offer()).await {
            Ok(offer) => offer,
            Err(err) => {
                web_sys::console::warn_1(&err);
                return;
            }
        };
        let Ok(description) = description_from(&offer, RtcSdpType::Offer) else {
            return;
        };
        if let Err(err) = JsFuture::from(pc.set_local_description(&description)).await {
            web_sys::console::warn_1(&err);
            return;
        }
        // Erst nach `setLocalDescription` haben neue Transceiver ihre Mid.
        let message = js_sys::Object::new();
        set(&message, "type", &JsValue::from_str("offer"));
        set(
            &message,
            "sdp",
            &Reflect::get(&offer, &"sdp".into()).unwrap_or(JsValue::NULL),
        );
        set(&message, "roles", &track_roles());
        if let Ok(text) = js_sys::JSON::stringify(&message)
            && let Some(text) = text.as_string()
            && dc.send_with_str(&text).is_err()
        {
            web_sys::console::warn_1(&"Neuverhandlung: DataChannel geschlossen".into());
        }
    });
}

/// Stream-ID des SFU: `u{id}` Mikrofon/Bildschirm, `s{id}` Bildschirmton,
/// `c{id}` Kamera.
fn owner_of_stream(stream_id: &str) -> Option<(char, String)> {
    let mut chars = stream_id.chars();
    let kind = chars.next()?;
    let user = chars.as_str();
    (matches!(kind, 'u' | 's' | 'c')
        && !user.is_empty()
        && user.bytes().all(|b| b.is_ascii_digit()))
    .then(|| (kind, user.to_owned()))
}

fn new_video(document: &web_sys::Document, stream: &MediaStream) -> Option<HtmlVideoElement> {
    let element = document.create_element("video").ok()?;
    let video: HtmlVideoElement = element.unchecked_into();
    video.set_autoplay(true);
    video.set_muted(true);
    let _ = video.set_attribute("playsinline", "true");
    video.set_class_name("nv-tile__video");
    video.set_src_object(Some(stream));
    let _ = video.play();
    Some(video)
}

fn new_audio(
    document: &web_sys::Document,
    stream: &MediaStream,
    user_id: &str,
) -> Option<HtmlAudioElement> {
    let element = document.create_element("audio").ok()?;
    let audio: HtmlAudioElement = element.unchecked_into();
    let media: &HtmlMediaElement = audio.as_ref();
    media.set_autoplay(true);
    media.set_muted(DEAFENED.with(Cell::get));
    media.set_volume(VOLUMES.with(|v| v.borrow().get(user_id).copied().unwrap_or(1.0)));
    let _ = audio.set_attribute("aria-hidden", "true");
    let _ = audio.set_attribute("data-nova-voice", user_id);
    media.set_src_object(Some(stream));
    if let Some(body) = document.body() {
        let _ = body.append_child(&audio);
    }
    let _ = media.play();
    Some(audio)
}

fn attach_remote(event: RtcTrackEvent, mut store: Store) {
    let track = event.track();
    let mid = event.transceiver().mid();
    let stream = if event.streams().length() > 0 {
        event.streams().get(0).unchecked_into::<MediaStream>()
    } else {
        let Ok(stream) = MediaStream::new() else {
            return;
        };
        stream.add_track(&track);
        stream
    };
    let Some((kind, user_id)) = owner_of_stream(&stream.id()) else {
        return;
    };
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let video = track.kind() == "video";
    match (kind, video) {
        // Bildschirm
        ('u', true) => {
            let Some(element) = new_video(&document, &stream) else {
                return;
            };
            REMOTES.with(|remotes| {
                let mut remotes = remotes.borrow_mut();
                let entry = remotes.entry(user_id.clone()).or_default();
                if let Some(old) = entry.video.replace(element) {
                    old.remove();
                }
                entry.video_mid = mid;
                entry.video_rid = None;
            });
            let mut voice = store.voice.write();
            if !voice.video_users.contains(&user_id) {
                voice.video_users.push(user_id);
            }
        }
        // Kamera
        ('c', true) => {
            let Some(element) = new_video(&document, &stream) else {
                return;
            };
            REMOTES.with(|remotes| {
                let mut remotes = remotes.borrow_mut();
                let entry = remotes.entry(user_id.clone()).or_default();
                if let Some(old) = entry.camera.replace(element) {
                    old.remove();
                }
                entry.camera_mid = mid;
                entry.camera_rid = None;
            });
            let mut voice = store.voice.write();
            if !voice.camera_users.contains(&user_id) {
                voice.camera_users.push(user_id);
            }
        }
        // Ton der Bildschirmfreigabe: eigenes Element, kein Sprecher-Ring.
        ('s', false) => {
            let Some(audio) = new_audio(&document, &stream, &user_id) else {
                return;
            };
            REMOTES.with(|remotes| {
                let mut remotes = remotes.borrow_mut();
                let entry = remotes.entry(user_id).or_default();
                if let Some(old) = entry.screen_audio.replace(audio) {
                    old.remove();
                }
            });
        }
        // Mikrofon
        ('u', false) => {
            let Some(audio) = new_audio(&document, &stream, &user_id) else {
                return;
            };
            let analyser = analyser_for(&stream);
            REMOTES.with(|remotes| {
                let mut remotes = remotes.borrow_mut();
                let entry = remotes.entry(user_id).or_default();
                if let Some(old) = entry.audio.replace(audio) {
                    old.remove();
                }
                entry.analyser = analyser;
            });
        }
        _ => {}
    }
    apply_layers(store);
}

/// Schichtwunsch an den SFU: `{"type":"layer","mid":"…","rid":"h|m|l"}`.
fn request_layer(dc: &RtcDataChannel, mid: &str, rid: &str) {
    let message = js_sys::Object::new();
    set(&message, "type", &JsValue::from_str("layer"));
    set(&message, "mid", &JsValue::from_str(mid));
    set(&message, "rid", &JsValue::from_str(rid));
    if let Ok(text) = js_sys::JSON::stringify(&message)
        && let Some(text) = text.as_string()
    {
        let _ = dc.send_with_str(&text);
    }
}

/// Simulcast-Schichten wählen: geteilte Bildschirme groß (hoch), Kameras in
/// Kacheln klein. Schickt nur Änderungen.
pub fn apply_layers(store: Store) {
    let Some(dc) = SESSION.with(|session| session.borrow().as_ref().map(|s| s.dc.clone())) else {
        return;
    };
    let quality = store.settings.peek().receive_quality;
    let (screens, tiles) = {
        let voice = store.voice.peek();
        let tiles = voice
            .channel
            .as_ref()
            .and_then(|channel| store.voice_rooms.peek().get(&channel.id).map(Vec::len))
            .unwrap_or(0);
        (voice.video_users.len(), tiles)
    };
    let screen_rid = quality.screen_layer(screens);
    let camera_rid = quality.camera_layer(tiles);
    REMOTES.with(|remotes| {
        for remote in remotes.borrow_mut().values_mut() {
            if let Some(mid) = &remote.video_mid
                && remote.video_rid != Some(screen_rid)
            {
                request_layer(&dc, mid, screen_rid);
                remote.video_rid = Some(screen_rid);
            }
            if let Some(mid) = &remote.camera_mid
                && remote.camera_rid != Some(camera_rid)
            {
                request_layer(&dc, mid, camera_rid);
                remote.camera_rid = Some(camera_rid);
            }
        }
    });
}

fn handle_message(text: &str, pc: &RtcPeerConnection, dc: &RtcDataChannel) {
    let Ok(value) = js_sys::JSON::parse(text) else {
        return;
    };
    let kind = Reflect::get(&value, &JsValue::from_str("type"))
        .ok()
        .and_then(|kind| kind.as_string())
        .unwrap_or_default();
    let pc = pc.clone();
    let dc = dc.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result: Result<(), String> = async {
            if kind == "offer" {
                let description = description_from(&value, RtcSdpType::Offer)?;
                JsFuture::from(pc.set_remote_description(&description))
                    .await
                    .map_err(|err| js_error(err, "Remote-Offer"))?;
                let answer = JsFuture::from(pc.create_answer())
                    .await
                    .map_err(|err| js_error(err, "Answer"))?;
                let local = description_from(&answer, RtcSdpType::Answer)?;
                JsFuture::from(pc.set_local_description(&local))
                    .await
                    .map_err(|err| js_error(err, "Lokale Answer"))?;
                let text = js_sys::JSON::stringify(&answer)
                    .ok()
                    .and_then(|text| text.as_string())
                    .ok_or("Answer nicht serialisierbar")?;
                dc.send_with_str(&text)
                    .map_err(|err| js_error(err, "DataChannel"))?;
            } else if kind == "answer" {
                let description = description_from(&value, RtcSdpType::Answer)?;
                JsFuture::from(pc.set_remote_description(&description))
                    .await
                    .map_err(|err| js_error(err, "Remote-Answer"))?;
            }
            Ok(())
        }
        .await;
        if let Err(err) = result {
            web_sys::console::warn_1(&err.into());
        }
    });
}

pub async fn join(store: Store, channel_id: &str, settings: &Settings) -> Result<(), String> {
    shutdown();
    let generation = GENERATION.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    VOLUMES.with(|volumes| {
        *volumes.borrow_mut() = settings.volumes.iter().cloned().collect();
    });
    let window = web_sys::window().ok_or("kein Browserfenster")?;
    let devices = media_devices(&window, "getUserMedia")?;
    let promise = devices
        .get_user_media_with_constraints(&audio_constraints(settings))
        .map_err(|err| js_error(err, "Mikrofon verweigert"))?;
    let local: MediaStream = JsFuture::from(promise)
        .await
        .map_err(|err| js_error(err, "Mikrofon nicht verfügbar"))?
        .unchecked_into();

    let pc = RtcPeerConnection::new_with_configuration(&web_sys::RtcConfiguration::new())
        .map_err(|err| js_error(err, "PeerConnection"))?;
    let dc = pc.create_data_channel("sdp");

    let on_message = {
        let pc = pc.clone();
        let dc = dc.clone();
        Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            if let Some(text) = event.data().as_string() {
                handle_message(&text, &pc, &dc);
            }
        })
    };
    dc.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    let on_track = Closure::<dyn FnMut(RtcTrackEvent)>::new(move |event: RtcTrackEvent| {
        attach_remote(event, store);
    });
    pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));

    // Netzwechsel: ICE fällt aus → automatisch neu beitreten.
    let on_ice = {
        let pc = pc.clone();
        Closure::<dyn FnMut(Event)>::new(move |_| {
            if pc.ice_connection_state() == RtcIceConnectionState::Failed
                && GENERATION.with(Cell::get) == generation
            {
                spawn(crate::actions::reconnect_voice(store));
            }
        })
    };
    pc.set_oniceconnectionstatechange(Some(on_ice.as_ref().unchecked_ref()));

    let audio = local
        .get_audio_tracks()
        .get(0)
        .dyn_into::<MediaStreamTrack>()
        .map_err(|_| "kein Mikrofontrack")?;
    pc.add_track(&audio, &local, &js_sys::Array::new());
    let local_analyser = analyser_for(&local);

    let offer = JsFuture::from(pc.create_offer())
        .await
        .map_err(|err| js_error(err, "Offer"))?;
    let description = description_from(&offer, RtcSdpType::Offer)?;
    JsFuture::from(pc.set_local_description(&description))
        .await
        .map_err(|err| js_error(err, "Lokale Beschreibung"))?;
    let local_description = pc.local_description().ok_or("Lokale SDP fehlt")?;
    let offer_json =
        serde_json::json!({ "type": "offer", "sdp": local_description.sdp() }).to_string();
    let response = api::voice_join(channel_id, &offer_json).await?;
    let answer = js_sys::JSON::parse(&response.sdp).map_err(|err| js_error(err, "Answer-JSON"))?;
    let answer = description_from(&answer, RtcSdpType::Answer)?;
    JsFuture::from(pc.set_remote_description(&answer))
        .await
        .map_err(|err| js_error(err, "Remote-Answer"))?;

    SESSION.with(|session| {
        *session.borrow_mut() = Some(Session {
            pc,
            dc,
            local,
            audio,
            local_analyser,
            _on_message: on_message,
            _on_track: on_track,
            _on_ice: on_ice,
        });
    });
    spawn(level_loop(store, generation));
    spawn(ping_loop(store, generation));
    Ok(())
}

/// Sprecher-Erkennung: alle 100 ms Pegel messen, mit Haltezeit.
async fn level_loop(mut store: Store, generation: u32) {
    while GENERATION.with(Cell::get) == generation {
        let now = js_sys::Date::now();
        let me = store.me_id();
        let mut speaking: Vec<String> = Vec::new();
        REMOTES.with(|remotes| {
            for (user, remote) in remotes.borrow_mut().iter_mut() {
                if let Some(analyser) = &remote.analyser
                    && level(analyser) > SPEAKING_LEVEL
                {
                    remote.last_loud = now;
                }
                if now - remote.last_loud < SPEAKING_HOLD_MS {
                    speaking.push(user.clone());
                }
            }
        });
        let local_level = SESSION.with(|session| {
            session.borrow().as_ref().and_then(|session| {
                session
                    .audio
                    .enabled()
                    .then(|| session.local_analyser.as_ref().map(level))
                    .flatten()
            })
        });
        if local_level.is_some_and(|value| value > SPEAKING_LEVEL) {
            LOCAL_LOUD.with(|loud| loud.set(now));
        }
        if now - LOCAL_LOUD.with(Cell::get) < SPEAKING_HOLD_MS
            && let Some(me) = me
        {
            speaking.push(me);
        }
        speaking.sort();
        if store.voice.read().speaking != speaking {
            store.voice.write().speaking = speaking;
        }
        TimeoutFuture::new(100).await;
    }
}

/// RTT des aktiven Kandidatenpaars alle 2 s.
async fn ping_loop(mut store: Store, generation: u32) {
    while GENERATION.with(Cell::get) == generation {
        let pc = SESSION.with(|session| session.borrow().as_ref().map(|s| s.pc.clone()));
        let Some(pc) = pc else { break };
        if let Ok(report) = JsFuture::from(pc.get_stats()).await {
            let mut rtt: Option<f64> = None;
            if let Ok(Some(entries)) = js_sys::try_iter(&report) {
                for entry in entries.flatten() {
                    let stat = Reflect::get(&entry, &JsValue::from(1)).unwrap_or(JsValue::NULL);
                    let get = |key: &str| Reflect::get(&stat, &key.into()).unwrap_or(JsValue::NULL);
                    if get("type").as_string().as_deref() == Some("candidate-pair")
                        && get("state").as_string().as_deref() == Some("succeeded")
                        && let Some(value) = get("currentRoundTripTime").as_f64()
                    {
                        rtt = Some(value);
                    }
                }
            }
            let ping = rtt.map(|seconds| (seconds * 1000.0).round() as u32);
            if store.voice.read().ping_ms != ping {
                store.voice.write().ping_ms = ping;
            }
            // Raumgröße und Einstellung ändern sich: Schichten nachziehen.
            apply_layers(store);
        }
        TimeoutFuture::new(2000).await;
    }
}

/// Simulcast-Schichten für `addTransceiver`: hoch (volle Auflösung), mittel
/// (½), niedrig (¼). Browser, die kein Simulcast können, senden nur „h“; der
/// SFU leitet dann einfach diese Schicht weiter.
fn simulcast_encodings(profile: &CaptureProfile) -> js_sys::Array {
    let layers = [
        ("h", 1.0, profile.bitrate, profile.fps),
        ("m", 2.0, profile.bitrate / 3.0, profile.fps),
        ("l", 4.0, profile.bitrate / 10.0, profile.fps.min(15.0)),
    ];
    let list = js_sys::Array::new();
    for (rid, scale, bitrate, fps) in layers {
        let encoding = js_sys::Object::new();
        set(&encoding, "rid", &JsValue::from_str(rid));
        set(
            &encoding,
            "scaleResolutionDownBy",
            &JsValue::from_f64(scale),
        );
        set(&encoding, "maxBitrate", &JsValue::from_f64(bitrate.round()));
        set(&encoding, "maxFramerate", &JsValue::from_f64(fps));
        list.push(&encoding);
    }
    list
}

/// Sendende Spur als eigener Transceiver (Simulcast nur für Video).
fn add_sender(
    pc: &RtcPeerConnection,
    track: &MediaStreamTrack,
    stream: &MediaStream,
    encodings: Option<js_sys::Array>,
) -> RtcRtpTransceiver {
    let init = RtcRtpTransceiverInit::new();
    init.set_direction(RtcRtpTransceiverDirection::Sendonly);
    let streams = js_sys::Array::new();
    streams.push(stream);
    init.set_streams(&streams);
    if let Some(encodings) = encodings {
        init.set_send_encodings(&encodings);
    }
    pc.add_transceiver_with_media_stream_track_and_init(track, &init)
}

fn session_channel() -> Option<(RtcPeerConnection, RtcDataChannel)> {
    SESSION.with(|session| {
        session
            .borrow()
            .as_ref()
            .map(|session| (session.pc.clone(), session.dc.clone()))
    })
}

fn stop_tracks(stream: &MediaStream) {
    let tracks = stream.get_tracks();
    for index in 0..tracks.length() {
        if let Ok(track) = tracks.get(index).dyn_into::<MediaStreamTrack>() {
            track.stop();
        }
    }
}

pub async fn start_screen(mut store: Store, settings: &Settings) -> Result<(), String> {
    let Some((pc, dc)) = session_channel() else {
        return Err("Zuerst einem Sprachkanal beitreten.".into());
    };
    if SCREEN.with(|screen| screen.borrow().is_some()) {
        return Ok(());
    }
    let profile = settings.screen_quality.profile();
    let window = web_sys::window().ok_or("kein Browserfenster")?;
    let devices = media_devices(&window, "getDisplayMedia")?;
    let constraints = DisplayMediaStreamConstraints::new();
    if settings.screen_audio {
        // Tab-Ton überall, Systemton dort, wo der Browser es anbietet (Chrome
        // unter Windows/ChromeOS). Die eigene Wiedergabe bleibt hörbar.
        let audio = js_sys::Object::new();
        set(&audio, "echoCancellation", &JsValue::FALSE);
        set(&audio, "noiseSuppression", &JsValue::FALSE);
        set(&audio, "autoGainControl", &JsValue::FALSE);
        set(&audio, "suppressLocalAudioPlayback", &JsValue::FALSE);
        constraints.set_audio(&audio);
        set(
            constraints.as_ref(),
            "systemAudio",
            &JsValue::from_str("include"),
        );
    } else {
        constraints.set_audio(&JsValue::FALSE);
    }
    let video = js_sys::Object::new();
    for (key, ideal) in [
        ("width", profile.width),
        ("height", profile.height),
        ("frameRate", profile.fps),
    ] {
        let value = js_sys::Object::new();
        set(&value, "ideal", &JsValue::from_f64(ideal));
        if key == "frameRate" {
            set(&value, "max", &JsValue::from_f64(profile.fps));
        }
        set(&video, key, &value);
    }
    constraints.set_video(&video);
    let stream: MediaStream = JsFuture::from(
        devices
            .get_display_media_with_constraints(&constraints)
            .map_err(|err| js_error(err, "Bildschirmfreigabe verweigert"))?,
    )
    .await
    .map_err(|err| js_error(err, "Bildschirmfreigabe abgebrochen"))?
    .unchecked_into();
    let track = stream
        .get_video_tracks()
        .get(0)
        .dyn_into::<MediaStreamTrack>()
        .map_err(|_| "kein Videotrack")?;
    let _ = Reflect::set(&track, &"contentHint".into(), &profile.hint.into());
    // Beendet jemand die Freigabe über die Browserleiste, räumen wir auf.
    let on_ended = Closure::<dyn FnMut()>::new(move || {
        stop_screen();
        store.voice.write().screen = false;
        spawn(crate::actions::report_voice_state(store));
    });
    track.set_onended(Some(on_ended.as_ref().unchecked_ref()));
    on_ended.forget();
    let video = add_sender(&pc, &track, &stream, Some(simulcast_encodings(&profile)));
    let audio = stream
        .get_audio_tracks()
        .get(0)
        .dyn_into::<MediaStreamTrack>()
        .ok()
        .map(|audio| add_sender(&pc, &audio, &stream, None));
    SCREEN.with(|screen| {
        *screen.borrow_mut() = Some(ScreenShare {
            stream,
            video,
            audio,
        });
    });
    renegotiate(&pc, &dc);
    Ok(())
}

/// Spuren stoppen **und** aus der Verbindung nehmen, dann neu verhandeln –
/// sonst sehen die anderen ein eingefrorenes Bild.
pub fn stop_screen() {
    let Some(share) = SCREEN.with(|screen| screen.borrow_mut().take()) else {
        return;
    };
    stop_tracks(&share.stream);
    if let Some((pc, dc)) = session_channel() {
        pc.remove_track(&share.video.sender());
        if let Some(audio) = &share.audio {
            pc.remove_track(&audio.sender());
        }
        renegotiate(&pc, &dc);
    }
}

/// Kamera einschalten: 720p30 mit drei Simulcast-Schichten.
pub async fn start_camera(store: Store, settings: &Settings) -> Result<(), String> {
    let Some((pc, dc)) = session_channel() else {
        return Err("Zuerst einem Sprachkanal beitreten.".into());
    };
    if CAMERA.with(|camera| camera.borrow().is_some()) {
        return Ok(());
    }
    let window = web_sys::window().ok_or("kein Browserfenster")?;
    let devices = media_devices(&window, "getUserMedia")?;
    let video = js_sys::Object::new();
    for (key, ideal) in [("width", 1280.0), ("height", 720.0), ("frameRate", 30.0)] {
        let value = js_sys::Object::new();
        set(&value, "ideal", &JsValue::from_f64(ideal));
        set(&video, key, &value);
    }
    if let Some(device) = &settings.camera_device {
        let exact = js_sys::Object::new();
        set(&exact, "exact", &JsValue::from_str(device));
        set(&video, "deviceId", &exact);
    }
    let constraints = MediaStreamConstraints::new();
    constraints.set_video(&video);
    constraints.set_audio(&JsValue::FALSE);
    let stream: MediaStream = JsFuture::from(
        devices
            .get_user_media_with_constraints(&constraints)
            .map_err(|err| js_error(err, "Kamera verweigert"))?,
    )
    .await
    .map_err(|err| js_error(err, "Kamera nicht verfügbar"))?
    .unchecked_into();
    let track = stream
        .get_video_tracks()
        .get(0)
        .dyn_into::<MediaStreamTrack>()
        .map_err(|_| "keine Kameraspur")?;
    let _ = Reflect::set(&track, &"contentHint".into(), &"motion".into());
    let profile = CaptureProfile {
        width: 1280.0,
        height: 720.0,
        fps: 30.0,
        bitrate: 1_200_000.0,
        hint: "motion",
    };
    let transceiver = add_sender(&pc, &track, &stream, Some(simulcast_encodings(&profile)));
    let document = window.document().ok_or("kein Dokument")?;
    let preview = new_video(&document, &stream).ok_or("Vorschau nicht möglich")?;
    preview.set_class_name("nv-tile__video nv-tile__video--mirror");
    CAMERA.with(|camera| {
        *camera.borrow_mut() = Some(CameraShare {
            stream,
            transceiver,
            preview,
        });
    });
    renegotiate(&pc, &dc);
    if let Some(me) = store.me_id() {
        mount_camera(&me);
    }
    Ok(())
}

pub fn stop_camera() {
    let Some(camera) = CAMERA.with(|camera| camera.borrow_mut().take()) else {
        return;
    };
    stop_tracks(&camera.stream);
    camera.preview.remove();
    if let Some((pc, dc)) = session_channel() {
        pc.remove_track(&camera.transceiver.sender());
        renegotiate(&pc, &dc);
    }
}

pub fn set_deafened(deafened: bool) {
    DEAFENED.with(|state| state.set(deafened));
    REMOTES.with(|remotes| {
        for remote in remotes.borrow().values() {
            for audio in [&remote.audio, &remote.screen_audio].into_iter().flatten() {
                let media: &HtmlMediaElement = audio.as_ref();
                media.set_muted(deafened);
            }
        }
    });
}

pub fn set_muted(muted: bool) {
    SESSION.with(|session| {
        if let Some(session) = session.borrow().as_ref() {
            session.audio.set_enabled(!muted);
        }
    });
}

pub fn set_volume(user_id: &str, volume: f64) {
    VOLUMES.with(|volumes| {
        volumes.borrow_mut().insert(user_id.to_owned(), volume);
    });
    REMOTES.with(|remotes| {
        if let Some(remote) = remotes.borrow().get(user_id) {
            for audio in [&remote.audio, &remote.screen_audio].into_iter().flatten() {
                let media: &HtmlMediaElement = audio.as_ref();
                media.set_volume(volume.clamp(0.0, 1.0));
            }
        }
    });
}

/// Video-Element in die Kachel `#vslot-{user}` hängen (nach dem Rendern).
pub fn mount_video(user_id: &str) {
    let Some(slot) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(&format!("vslot-{user_id}")))
    else {
        return;
    };
    REMOTES.with(|remotes| {
        if let Some(video) = remotes.borrow().get(user_id).and_then(|r| r.video.as_ref())
            && video.parent_element().as_ref() != Some(&slot)
        {
            let _ = slot.append_child(video);
            let _ = video.play();
        }
    });
}

/// Kamerabild in die Kachel `#cslot-{user}` hängen – fremde Kameras und die
/// eigene Vorschau.
pub fn mount_camera(user_id: &str) {
    let Some(slot) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(&format!("cslot-{user_id}")))
    else {
        return;
    };
    let mounted = CAMERA.with(|camera| {
        let camera = camera.borrow();
        let Some(camera) = camera.as_ref() else {
            return false;
        };
        let me = slot.get_attribute("data-me").as_deref() == Some("true");
        if me && camera.preview.parent_element().as_ref() != Some(&slot) {
            let _ = slot.append_child(&camera.preview);
            let _ = camera.preview.play();
        }
        me
    });
    if mounted {
        return;
    }
    REMOTES.with(|remotes| {
        if let Some(video) = remotes
            .borrow()
            .get(user_id)
            .and_then(|r| r.camera.as_ref())
            && video.parent_element().as_ref() != Some(&slot)
        {
            let _ = slot.append_child(video);
            let _ = video.play();
        }
    });
}

pub fn drop_remote_camera(user_id: &str) {
    REMOTES.with(|remotes| {
        if let Some(remote) = remotes.borrow_mut().get_mut(user_id) {
            if let Some(video) = remote.camera.take() {
                video.remove();
            }
            remote.camera_mid = None;
            remote.camera_rid = None;
        }
    });
}

pub fn drop_remote_video(user_id: &str) {
    REMOTES.with(|remotes| {
        if let Some(remote) = remotes.borrow_mut().get_mut(user_id) {
            if let Some(video) = remote.video.take() {
                video.remove();
            }
            if let Some(audio) = remote.screen_audio.take() {
                audio.remove();
            }
            remote.video_mid = None;
            remote.video_rid = None;
        }
    });
}

fn remove_elements(remote: Remote) {
    for audio in [remote.audio, remote.screen_audio].into_iter().flatten() {
        audio.remove();
    }
    for video in [remote.video, remote.camera].into_iter().flatten() {
        video.remove();
    }
}

pub fn drop_remote_user(user_id: &str) {
    REMOTES.with(|remotes| {
        if let Some(remote) = remotes.borrow_mut().remove(user_id) {
            remove_elements(remote);
        }
    });
}

pub fn shutdown() {
    GENERATION.with(|g| g.set(g.get() + 1));
    stop_screen();
    stop_camera();
    set_deafened(false);
    SESSION.with(|session| {
        if let Some(session) = session.borrow_mut().take() {
            let tracks = session.local.get_tracks();
            for index in 0..tracks.length() {
                if let Ok(track) = tracks.get(index).dyn_into::<MediaStreamTrack>() {
                    track.stop();
                }
            }
            session.pc.close();
        }
    });
    REMOTES.with(|remotes| {
        for (_, remote) in remotes.borrow_mut().drain() {
            remove_elements(remote);
        }
    });
}

/// Kameras für die Einstellungen: (deviceId, Bezeichnung).
pub async fn camera_devices() -> Vec<(String, String)> {
    devices(web_sys::MediaDeviceKind::Videoinput, "Kamera").await
}

/// Mikrofone für die Einstellungen: (deviceId, Bezeichnung).
pub async fn input_devices() -> Vec<(String, String)> {
    devices(web_sys::MediaDeviceKind::Audioinput, "Mikrofon").await
}

async fn devices(kind: web_sys::MediaDeviceKind, fallback: &str) -> Vec<(String, String)> {
    let Some(window) = web_sys::window() else {
        return Vec::new();
    };
    let Ok(devices) = media_devices(&window, "enumerateDevices") else {
        return Vec::new();
    };
    let Ok(promise) = devices.enumerate_devices() else {
        return Vec::new();
    };
    let Ok(list) = JsFuture::from(promise).await else {
        return Vec::new();
    };
    let list: js_sys::Array = list.unchecked_into();
    list.iter()
        .filter_map(|item| item.dyn_into::<web_sys::MediaDeviceInfo>().ok())
        .filter(|info| info.kind() == kind)
        .enumerate()
        .map(|(index, info)| {
            let label = info.label();
            let label = if label.is_empty() {
                format!("{fallback} {}", index + 1)
            } else {
                label
            };
            (info.device_id(), label)
        })
        .collect()
}
