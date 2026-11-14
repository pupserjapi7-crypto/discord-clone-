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
    RtcRtpSender, RtcSdpType, RtcSessionDescriptionInit, RtcTrackEvent, Window,
};

use crate::{api, store::Store, types::Settings};

/// Pegel (RMS), ab dem jemand als sprechend gilt.
const SPEAKING_LEVEL: f32 = 0.018;
/// So lange bleibt der Ring nach dem letzten lauten Frame an (kein Flackern).
const SPEAKING_HOLD_MS: f64 = 350.0;

struct Remote {
    audio: Option<HtmlAudioElement>,
    video: Option<HtmlVideoElement>,
    analyser: Option<AnalyserNode>,
    last_loud: f64,
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
    static SCREEN: RefCell<Option<(MediaStream, RtcRtpSender)>> = const { RefCell::new(None) };
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

/// Neuverhandlung über den DataChannel (neue/entfernte Spuren).
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
        if let Ok(text) = js_sys::JSON::stringify(&offer)
            && let Some(text) = text.as_string()
            && dc.send_with_str(&text).is_err()
        {
            web_sys::console::warn_1(&"Neuverhandlung: DataChannel geschlossen".into());
        }
    });
}

fn user_from_stream(stream_id: &str) -> Option<String> {
    stream_id.strip_prefix('u').map(str::to_owned)
}

fn attach_remote(event: RtcTrackEvent, mut store: Store) {
    let track = event.track();
    let stream = if event.streams().length() > 0 {
        event.streams().get(0).unchecked_into::<MediaStream>()
    } else {
        let Ok(stream) = MediaStream::new() else {
            return;
        };
        stream.add_track(&track);
        stream
    };
    let Some(user_id) = user_from_stream(&stream.id()) else {
        return;
    };
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if track.kind() == "video" {
        let Ok(element) = document.create_element("video") else {
            return;
        };
        let video: HtmlVideoElement = element.unchecked_into();
        video.set_autoplay(true);
        video.set_muted(true);
        let _ = video.set_attribute("playsinline", "true");
        video.set_class_name("nv-tile__video");
        video.set_src_object(Some(&stream));
        let _ = video.play();
        REMOTES.with(|remotes| {
            let mut remotes = remotes.borrow_mut();
            let entry = remotes.entry(user_id.clone()).or_insert(Remote {
                audio: None,
                video: None,
                analyser: None,
                last_loud: 0.0,
            });
            if let Some(old) = entry.video.replace(video) {
                old.remove();
            }
        });
        let mut voice = store.voice.write();
        if !voice.video_users.contains(&user_id) {
            voice.video_users.push(user_id);
        }
        return;
    }

    let Ok(element) = document.create_element("audio") else {
        return;
    };
    let audio: HtmlAudioElement = element.unchecked_into();
    let media: &HtmlMediaElement = audio.as_ref();
    media.set_autoplay(true);
    media.set_muted(DEAFENED.with(Cell::get));
    media.set_volume(VOLUMES.with(|v| v.borrow().get(&user_id).copied().unwrap_or(1.0)));
    let _ = audio.set_attribute("aria-hidden", "true");
    let _ = audio.set_attribute("data-nova-voice", &user_id);
    media.set_src_object(Some(&stream));
    if let Some(body) = document.body() {
        let _ = body.append_child(&audio);
    }
    let _ = media.play();
    let analyser = analyser_for(&stream);
    REMOTES.with(|remotes| {
        let mut remotes = remotes.borrow_mut();
        let entry = remotes.entry(user_id).or_insert(Remote {
            audio: None,
            video: None,
            analyser: None,
            last_loud: 0.0,
        });
        if let Some(old) = entry.audio.replace(audio) {
            old.remove();
        }
        entry.analyser = analyser;
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
        }
        TimeoutFuture::new(2000).await;
    }
}

pub async fn start_screen(mut store: Store) -> Result<(), String> {
    let Some((pc, dc)) = SESSION.with(|session| {
        session
            .borrow()
            .as_ref()
            .map(|session| (session.pc.clone(), session.dc.clone()))
    }) else {
        return Err("Zuerst einem Sprachkanal beitreten.".into());
    };
    if SCREEN.with(|screen| screen.borrow().is_some()) {
        return Ok(());
    }
    let window = web_sys::window().ok_or("kein Browserfenster")?;
    let devices = media_devices(&window, "getDisplayMedia")?;
    let constraints = DisplayMediaStreamConstraints::new();
    constraints.set_audio(&JsValue::FALSE);
    let video = js_sys::Object::new();
    for (key, ideal) in [("width", 1920.0), ("height", 1080.0), ("frameRate", 60.0)] {
        let value = js_sys::Object::new();
        set(&value, "ideal", &JsValue::from_f64(ideal));
        if key == "frameRate" {
            set(&value, "max", &JsValue::from_f64(60.0));
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
    let _ = Reflect::set(&track, &"contentHint".into(), &"detail".into());
    // Beendet jemand die Freigabe über die Browserleiste, räumen wir auf.
    let on_ended = Closure::<dyn FnMut()>::new(move || {
        stop_screen();
        store.voice.write().screen = false;
        spawn(crate::actions::report_voice_state(store));
    });
    track.set_onended(Some(on_ended.as_ref().unchecked_ref()));
    on_ended.forget();
    let sender = pc.add_track(&track, &stream, &js_sys::Array::new());
    renegotiate(&pc, &dc);
    SCREEN.with(|screen| *screen.borrow_mut() = Some((stream, sender)));
    Ok(())
}

/// Spuren stoppen **und** aus der Verbindung nehmen, dann neu verhandeln –
/// sonst sehen die anderen ein eingefrorenes Bild.
pub fn stop_screen() {
    let Some((stream, sender)) = SCREEN.with(|screen| screen.borrow_mut().take()) else {
        return;
    };
    let tracks = stream.get_tracks();
    for index in 0..tracks.length() {
        if let Ok(track) = tracks.get(index).dyn_into::<MediaStreamTrack>() {
            track.stop();
        }
    }
    SESSION.with(|session| {
        if let Some(session) = session.borrow().as_ref() {
            session.pc.remove_track(&sender);
            renegotiate(&session.pc, &session.dc);
        }
    });
}

pub fn set_deafened(deafened: bool) {
    DEAFENED.with(|state| state.set(deafened));
    REMOTES.with(|remotes| {
        for remote in remotes.borrow().values() {
            if let Some(audio) = &remote.audio {
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
        if let Some(audio) = remotes.borrow().get(user_id).and_then(|r| r.audio.as_ref()) {
            let media: &HtmlMediaElement = audio.as_ref();
            media.set_volume(volume.clamp(0.0, 1.0));
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

pub fn drop_remote_video(user_id: &str) {
    REMOTES.with(|remotes| {
        if let Some(remote) = remotes.borrow_mut().get_mut(user_id)
            && let Some(video) = remote.video.take()
        {
            video.remove();
        }
    });
}

pub fn drop_remote_user(user_id: &str) {
    REMOTES.with(|remotes| {
        if let Some(remote) = remotes.borrow_mut().remove(user_id) {
            if let Some(audio) = remote.audio {
                audio.remove();
            }
            if let Some(video) = remote.video {
                video.remove();
            }
        }
    });
}

pub fn shutdown() {
    GENERATION.with(|g| g.set(g.get() + 1));
    stop_screen();
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
            if let Some(audio) = remote.audio {
                audio.remove();
            }
            if let Some(video) = remote.video {
                video.remove();
            }
        }
    });
}

/// Mikrofone für die Einstellungen: (deviceId, Bezeichnung).
pub async fn input_devices() -> Vec<(String, String)> {
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
        .filter(|info| info.kind() == web_sys::MediaDeviceKind::Audioinput)
        .enumerate()
        .map(|(index, info)| {
            let label = info.label();
            let label = if label.is_empty() {
                format!("Mikrofon {}", index + 1)
            } else {
                label
            };
            (info.device_id(), label)
        })
        .collect()
}
