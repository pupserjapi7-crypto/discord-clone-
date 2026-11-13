use std::cell::{Cell, RefCell};

use js_sys::Reflect;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    DisplayMediaStreamConstraints, HtmlElement, HtmlMediaElement, HtmlVideoElement, MediaDevices,
    MediaStream, MediaStreamConstraints, MediaStreamTrack, MessageEvent, RtcDataChannel,
    RtcPeerConnection, RtcSdpType, RtcSessionDescriptionInit, RtcTrackEvent, Window,
};

use crate::api;

thread_local! {
    static SESSION: RefCell<Option<VoiceSession>> = const { RefCell::new(None) };
    static REMOTE: RefCell<Vec<HtmlElement>> = const { RefCell::new(Vec::new()) };
    static SCREEN: RefCell<Option<MediaStream>> = const { RefCell::new(None) };
    static DEAFENED: Cell<bool> = const { Cell::new(false) };
}

struct VoiceSession {
    pc: RtcPeerConnection,
    dc: RtcDataChannel,
    local: MediaStream,
    audio: MediaStreamTrack,
    _on_message: wasm_bindgen::closure::Closure<dyn FnMut(MessageEvent)>,
    _on_track: wasm_bindgen::closure::Closure<dyn FnMut(RtcTrackEvent)>,
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

/// Prüft vor dem Zugriff, ob der Browser die Medien-API überhaupt bereitstellt.
/// Auf einer unsicheren HTTP-Seite ist `navigator.mediaDevices` `undefined`;
/// ein direkter Aufruf würde sonst mit einem nichtssagenden JS-Fehler abbrechen.
fn media_devices(window: &Window, method: &str) -> Result<MediaDevices, String> {
    let secure = Reflect::get(window.as_ref(), &JsValue::from_str("isSecureContext"))
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let navigator = window.navigator();
    let value = Reflect::get(navigator.as_ref(), &JsValue::from_str("mediaDevices"))
        .map_err(|err| js_error(err, "Medien-API nicht lesbar"))?;
    let function = Reflect::get(&value, &JsValue::from_str(method))
        .ok()
        .filter(|function| function.is_function());
    if value.is_undefined() || value.is_null() || function.is_none() {
        return Err(if secure {
            "Dieser Browser stellt keine Medien-API bereit. Bitte einen aktuellen Chrome, Edge oder Firefox verwenden."
                .to_string()
        } else {
            "Mikrofon und Bildschirmfreigabe brauchen einen sicheren Kontext (HTTPS oder localhost). Nova läuft gerade über HTTP; bitte die Seite über HTTPS öffnen."
                .to_string()
        });
    }
    Ok(value.unchecked_into())
}

fn media_constraints(audio: bool, video: bool) -> MediaStreamConstraints {
    let constraints = MediaStreamConstraints::new();
    if audio {
        let audio_settings = js_sys::Object::new();
        let _ = Reflect::set(&audio_settings, &"echoCancellation".into(), &JsValue::TRUE);
        let _ = Reflect::set(&audio_settings, &"noiseSuppression".into(), &JsValue::TRUE);
        let _ = Reflect::set(&audio_settings, &"autoGainControl".into(), &JsValue::TRUE);
        constraints.set_audio(&JsValue::from(audio_settings));
    } else {
        constraints.set_audio(&JsValue::FALSE);
    }
    if video {
        let width = js_sys::Object::new();
        let _ = Reflect::set(&width, &"ideal".into(), &JsValue::from_f64(1920.0));
        let height = js_sys::Object::new();
        let _ = Reflect::set(&height, &"ideal".into(), &JsValue::from_f64(1080.0));
        let frame_rate = js_sys::Object::new();
        let _ = Reflect::set(&frame_rate, &"ideal".into(), &JsValue::from_f64(60.0));
        let _ = Reflect::set(&frame_rate, &"max".into(), &JsValue::from_f64(60.0));
        let video_settings = js_sys::Object::new();
        let _ = Reflect::set(&video_settings, &"width".into(), &width);
        let _ = Reflect::set(&video_settings, &"height".into(), &height);
        let _ = Reflect::set(&video_settings, &"frameRate".into(), &frame_rate);
        constraints.set_video(&JsValue::from(video_settings));
    } else {
        constraints.set_video(&JsValue::FALSE);
    }
    constraints
}

async fn request_screen() -> Result<MediaStream, String> {
    let window = web_sys::window().ok_or("kein Browserfenster")?;
    let devices = media_devices(&window, "getDisplayMedia")?;
    let constraints = DisplayMediaStreamConstraints::new();
    constraints.set_audio(&JsValue::FALSE);
    let width = js_sys::Object::new();
    let _ = Reflect::set(&width, &"ideal".into(), &JsValue::from_f64(1920.0));
    let height = js_sys::Object::new();
    let _ = Reflect::set(&height, &"ideal".into(), &JsValue::from_f64(1080.0));
    let frame_rate = js_sys::Object::new();
    let _ = Reflect::set(&frame_rate, &"ideal".into(), &JsValue::from_f64(60.0));
    let _ = Reflect::set(&frame_rate, &"max".into(), &JsValue::from_f64(60.0));
    let video = js_sys::Object::new();
    let _ = Reflect::set(&video, &"width".into(), &width);
    let _ = Reflect::set(&video, &"height".into(), &height);
    let _ = Reflect::set(&video, &"frameRate".into(), &frame_rate);
    let _ = Reflect::set(
        &video,
        &"displaySurface".into(),
        &JsValue::from_str("monitor"),
    );
    constraints.set_video(&JsValue::from(video));
    let promise = devices
        .get_display_media_with_constraints(&constraints)
        .map_err(|err| js_error(err, "Bildschirmfreigabe verweigert"))?;
    let value = JsFuture::from(promise)
        .await
        .map_err(|err| js_error(err, "Bildschirmfreigabe fehlgeschlagen"))?;
    Ok(value.unchecked_into::<MediaStream>())
}

async fn request_media(audio: bool, video: bool) -> Result<MediaStream, String> {
    let window = web_sys::window().ok_or("kein Browserfenster")?;
    let devices = media_devices(&window, "getUserMedia")?;
    let promise = devices
        .get_user_media_with_constraints(&media_constraints(audio, video))
        .map_err(|err| js_error(err, "Mikrofon/Bildschirm verweigert"))?;
    let value = JsFuture::from(promise)
        .await
        .map_err(|err| js_error(err, "Medienfreigabe fehlgeschlagen"))?;
    Ok(value.unchecked_into::<MediaStream>())
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

fn send_offer(pc: &RtcPeerConnection, dc: &RtcDataChannel) -> Result<(), String> {
    let promise = pc.create_offer();
    let dc = dc.clone();
    let pc = pc.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let Ok(offer) = JsFuture::from(promise).await else {
            return;
        };
        let Ok(description) = description_from(&offer, RtcSdpType::Offer) else {
            return;
        };
        if JsFuture::from(pc.set_local_description(&description))
            .await
            .is_err()
        {
            return;
        }
        let Ok(text) = js_sys::JSON::stringify(&offer) else {
            return;
        };
        if let Some(text) = text.as_string() {
            let _ = dc.send_with_str(&text);
        }
    });
    Ok(())
}

fn attach_remote(event: RtcTrackEvent) {
    let track = event.track();
    let stream = if event.streams().length() > 0 {
        event.streams().get(0).unchecked_into::<MediaStream>()
    } else {
        let stream = MediaStream::new().expect("stream");
        stream.add_track(&track);
        stream
    };
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let is_video = track.kind() == "video";
    let Ok(element) = document.create_element(if is_video { "video" } else { "audio" }) else {
        return;
    };
    let _ = element.set_attribute("data-nova-voice", "1");
    let media = element.clone().unchecked_into::<HtmlMediaElement>();
    media.set_autoplay(true);
    media.set_muted(DEAFENED.with(Cell::get));
    if is_video {
        let video = element.clone().unchecked_into::<HtmlVideoElement>();
        let _ = video.set_attribute("playsinline", "true");
        video.set_controls(false);
    } else {
        let _ = element.set_attribute("aria-hidden", "true");
        let _ = element.set_attribute("style", "display:none");
    }
    media.set_src_object(Some(&stream));
    let parent: web_sys::Element = if is_video {
        document
            .get_element_by_id("voice-videos")
            .unwrap_or_else(|| document.body().map(web_sys::Element::from).expect("body"))
    } else {
        document.body().map(web_sys::Element::from).expect("body")
    };
    if parent.append_child(&element).is_ok() {
        let html: HtmlElement = element.unchecked_into();
        let _ = media.play();
        REMOTE.with(|remote| remote.borrow_mut().push(html));
    }
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
    if kind == "offer" {
        wasm_bindgen_futures::spawn_local(async move {
            let Ok(description) = description_from(&value, RtcSdpType::Offer) else {
                return;
            };
            if JsFuture::from(pc.set_remote_description(&description))
                .await
                .is_err()
            {
                return;
            }
            let answer = JsFuture::from(pc.create_answer()).await;
            let Ok(answer) = answer else {
                return;
            };
            let Ok(description) = description_from(&answer, RtcSdpType::Answer) else {
                return;
            };
            if JsFuture::from(pc.set_local_description(&description))
                .await
                .is_err()
            {
                return;
            }
            if let Ok(text) = js_sys::JSON::stringify(&answer)
                && let Some(text) = text.as_string()
            {
                let _ = dc.send_with_str(&text);
            }
        });
    } else if kind == "answer" {
        wasm_bindgen_futures::spawn_local(async move {
            let Ok(description) = description_from(&value, RtcSdpType::Answer) else {
                return;
            };
            let _ = JsFuture::from(pc.set_remote_description(&description)).await;
        });
    }
}

pub async fn join(channel_id: &str) -> Result<(), String> {
    shutdown();
    let local = request_media(true, false).await?;
    let pc = RtcPeerConnection::new_with_configuration(&web_sys::RtcConfiguration::new())
        .map_err(|err| js_error(err, "PeerConnection"))?;
    let dc = pc.create_data_channel("sdp");

    let on_message = {
        let pc = pc.clone();
        let dc = dc.clone();
        wasm_bindgen::closure::Closure::<dyn FnMut(MessageEvent)>::new(
            move |event: MessageEvent| {
                if let Some(text) = event.data().as_string() {
                    handle_message(&text, &pc, &dc);
                }
            },
        )
    };
    dc.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    let on_track = wasm_bindgen::closure::Closure::<dyn FnMut(RtcTrackEvent)>::new(
        move |event: RtcTrackEvent| attach_remote(event),
    );
    pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));

    let tracks = local.get_tracks();
    let audio = tracks
        .get(0)
        .dyn_into::<MediaStreamTrack>()
        .map_err(|_| "kein Mikrofontrack")?;
    pc.add_track(&audio, &local, &js_sys::Array::new());

    let offer = JsFuture::from(pc.create_offer())
        .await
        .map_err(|err| js_error(err, "Offer"))?;
    let description = description_from(&offer, RtcSdpType::Offer)?;
    JsFuture::from(pc.set_local_description(&description))
        .await
        .map_err(|err| js_error(err, "Lokale Beschreibung"))?;
    let Some(local_description) = pc.local_description() else {
        return Err("Lokale SDP fehlt".into());
    };

    // str0m erwartet das serialisierte SDP-Objekt, nicht den nackten SDP-Text.
    let offer_json = serde_json::json!({
        "type": "offer",
        "sdp": local_description.sdp(),
    })
    .to_string();
    let response = api::voice_join(channel_id, &offer_json).await?;
    let answer = js_sys::JSON::parse(&response.sdp).map_err(|err| js_error(err, "Answer-JSON"))?;
    let answer_description = description_from(&answer, RtcSdpType::Answer)?;
    JsFuture::from(pc.set_remote_description(&answer_description))
        .await
        .map_err(|err| js_error(err, "Remote-Answer"))?;

    SESSION.with(|session| {
        *session.borrow_mut() = Some(VoiceSession {
            pc,
            dc,
            local,
            audio,
            _on_message: on_message,
            _on_track: on_track,
        });
    });
    Ok(())
}

pub async fn start_screen() -> Result<(), String> {
    let Some((pc, dc)) = SESSION.with(|session| {
        session
            .borrow()
            .as_ref()
            .map(|session| (session.pc.clone(), session.dc.clone()))
    }) else {
        return Err("Zuerst dem Sprachkanal beitreten.".into());
    };
    if SCREEN.with(|screen| screen.borrow().is_some()) {
        return Ok(());
    }
    let stream = request_screen().await?;
    let tracks = stream.get_tracks();
    let video = tracks
        .get(0)
        .dyn_into::<MediaStreamTrack>()
        .map_err(|_| "kein Videotrack")?;
    video.set_enabled(true);
    pc.add_track(&video, &stream, &js_sys::Array::new());
    send_offer(&pc, &dc)?;
    SCREEN.with(|screen| *screen.borrow_mut() = Some(stream));
    Ok(())
}

pub fn stop_screen() {
    SCREEN.with(|screen| {
        if let Some(stream) = screen.borrow_mut().take() {
            let tracks = stream.get_tracks();
            for index in 0..tracks.length() {
                if let Ok(track) = tracks.get(index).dyn_into::<MediaStreamTrack>() {
                    track.stop();
                }
            }
        }
    });
}

/// Schaltet alle bereits empfangenen Tonspuren stumm. Neue Tracks erben den
/// Zustand in `attach_remote`, ohne dass die PeerConnection neu verhandelt wird.
pub fn set_deafened(deafened: bool) {
    DEAFENED.with(|state| state.set(deafened));
    REMOTE.with(|remote| {
        for element in remote.borrow().iter() {
            let media = element.clone().unchecked_into::<HtmlMediaElement>();
            media.set_muted(deafened);
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

pub fn shutdown() {
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
    REMOTE.with(|remote| {
        for element in remote.borrow_mut().drain(..) {
            element.remove();
        }
    });
}
