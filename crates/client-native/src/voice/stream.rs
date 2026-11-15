//! Streams (Bildschirm teilen) in der Sprachverbindung.
//!
//! - **Senden**: Aufnahme-Thread (GDI/X11, [`crate::capture`]) → Kodier-Thread
//!   (OpenH264, [`crate::video`]) → Netz-Thread schreibt RTP. Die Videospur
//!   wird per eigenem Offer über den DataChannel `sdp` nachverhandelt.
//! - **Empfangen**: Der Netz-Thread reicht zusammengesetzte H.264-Bilder an
//!   einen eigenen Dekodier-Thread weiter; fertige Bilder liegen als
//!   `image::Handle` pro Person in [`Pictures`].
//!
//! Überall gilt „aktuell vor vollständig“: Kommt ein Thread nicht hinterher,
//! wird ein Bild verworfen und danach ein Schlüsselbild angefordert, statt
//! Latenz anzuhäufen.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

use iced::widget::image;
use str0m::{
    change::{SdpAnswer, SdpOffer},
    format::Codec,
    media::{Direction, Frequency, KeyframeRequestKind, MediaKind, MediaTime, Mid},
};

use super::{Engine, VoiceStatus};
use crate::{capture, video};

/// Ein fertiges Bild eines fremden Streams für die Oberfläche.
#[derive(Debug, Clone)]
pub struct Picture {
    pub handle: image::Handle,
    pub width: u32,
    pub height: u32,
    pub at: Instant,
}

/// Letztes Bild pro Nutzer-ID.
pub type Pictures = Arc<Mutex<HashMap<i64, Picture>>>;

/// Nach so langer Pause gilt ein Stream als beendet.
const PICTURE_TTL: Duration = Duration::from_secs(3);

pub fn picture(pictures: &Pictures, user_id: i64) -> Option<Picture> {
    pictures
        .lock()
        .ok()?
        .get(&user_id)
        .filter(|p| p.at.elapsed() < PICTURE_TTL)
        .cloned()
}

pub fn streamers(pictures: &Pictures) -> Vec<i64> {
    let Ok(pictures) = pictures.lock() else {
        return Vec::new();
    };
    let mut list: Vec<i64> = pictures
        .iter()
        .filter(|(_, p)| p.at.elapsed() < PICTURE_TTL)
        .map(|(id, _)| *id)
        .collect();
    list.sort_unstable();
    list
}

/// Zustand des eigenen Streams.
pub struct Screen {
    pub state: ScreenState,
    packets: Receiver<Encoded>,
    keyframe: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    started: Instant,
    sent: u32,
    sent_since: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenState {
    /// Offer muss (noch einmal) gesendet werden.
    NeedOffer,
    Offered(Mid),
    Active(Mid),
}

/// Ein kodiertes Bild aus dem Kodier-Thread.
struct Encoded {
    data: Vec<u8>,
    captured: Instant,
    size: (usize, usize),
}

impl Engine {
    /// Offer des SFU (neue/entfernte fremde Spuren) beantworten.
    pub(super) fn accept_remote_offer(&mut self, offer: SdpOffer) {
        // Gleichzeitige Offers: das des SFU gewinnt, unseres wird danach neu
        // gestellt.
        if self.pending.take().is_some()
            && let Some(screen) = self.screen.as_mut()
            && matches!(screen.state, ScreenState::Offered(_))
        {
            screen.state = ScreenState::NeedOffer;
        }
        if let Ok(answer) = self.rtc.sdp_api().accept_offer(offer)
            && let Ok(json) = serde_json::to_string(&answer)
            && let Some(mut channel) = self.rtc.channel(self.channel)
        {
            let _ = channel.write(false, json.as_bytes());
        }
    }

    /// Answer des SFU auf unser Offer (Videospur hinzugefügt/entfernt).
    pub(super) fn accept_remote_answer(&mut self, answer: SdpAnswer) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let accepted = self.rtc.sdp_api().accept_answer(pending, answer).is_ok();
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        if let ScreenState::Offered(mid) = screen.state {
            if accepted {
                screen.state = ScreenState::Active(mid);
                screen.keyframe.store(true, Ordering::Relaxed);
                self.update_status(|s| s.sharing = true);
            } else {
                screen.state = ScreenState::NeedOffer;
            }
        }
    }

    /// Ein Zuschauer (über den SFU) braucht ein Schlüsselbild.
    pub(super) fn on_keyframe_request(&mut self, mid: Mid) {
        if let Some(screen) = &self.screen
            && screen.state == ScreenState::Active(mid)
        {
            screen.keyframe.store(true, Ordering::Relaxed);
        }
    }

    // ------------------------------------------------------- Empfangen

    /// Ein H.264-Bild eines fremden Streams an den Dekodier-Thread geben.
    pub(super) fn handle_video(&mut self, mid: Mid, contiguous: bool, data: &[u8]) {
        let Some(user) = self.user_for(mid) else {
            return;
        };
        if !contiguous {
            // Lücke im Strom: bis zum nächsten Schlüsselbild ist das Bild kaputt.
            self.ask_keyframe(mid);
        }
        if self.decode_tx.is_none() {
            self.start_decoder();
        }
        let Some(tx) = &self.decode_tx else {
            return;
        };
        match tx.try_send((mid, user, data.to_vec())) {
            Ok(()) => {}
            // Dekodierer kommt nicht hinterher: Bild verwerfen, danach
            // Schlüsselbild holen statt Fehlerbilder zu zeigen.
            Err(TrySendError::Full(_)) => self.ask_keyframe(mid),
            Err(TrySendError::Disconnected(_)) => self.decode_tx = None,
        }
    }

    fn start_decoder(&mut self) {
        // Puffer für 4 Bilder: genug gegen kurze Spitzen, ohne Latenz anzuhäufen.
        let (tx, rx) = mpsc::sync_channel::<(Mid, i64, Vec<u8>)>(4);
        let (key_tx, key_rx) = mpsc::channel::<Mid>();
        let pictures = Arc::clone(&self.pictures);
        let spawned = thread::Builder::new()
            .name("nova-video-dec".into())
            .spawn(move || decode_loop(&rx, &key_tx, &pictures));
        if spawned.is_ok() {
            self.decode_tx = Some(tx);
            self.keyframe_rx = Some(key_rx);
        }
    }

    /// Schlüsselbild anfordern, höchstens einmal pro Sekunde und Spur.
    fn ask_keyframe(&mut self, mid: Mid) {
        if self
            .keyframe_asked
            .get(&mid)
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        if let Some(mut writer) = self.rtc.writer(mid)
            && writer
                .request_keyframe(None, KeyframeRequestKind::Pli)
                .is_ok()
        {
            self.keyframe_asked.insert(mid, Instant::now());
        }
    }

    pub(super) fn pump_keyframe_requests(&mut self) {
        let mids: Vec<Mid> = match &self.keyframe_rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for mid in mids {
            self.ask_keyframe(mid);
        }
    }

    // ---------------------------------------------------------- Senden

    pub(super) fn start_screen(&mut self) {
        if self.screen.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let keyframe = Arc::new(AtomicBool::new(true));
        // Aufnahme → (1 Bild Puffer) → Kodierung → Netz-Thread.
        let (frame_tx, frame_rx) = mpsc::sync_channel::<(capture::Frame, Instant)>(1);
        let (packet_tx, packet_rx) = mpsc::channel::<Encoded>();
        let status = Arc::clone(&self.status);
        let capture_stop = Arc::clone(&stop);
        let capture_status = Arc::clone(&status);
        let capture = thread::Builder::new()
            .name("nova-capture".into())
            .spawn(move || capture_loop(&frame_tx, &capture_stop, &capture_status));
        let encode_stop = Arc::clone(&stop);
        let encode_key = Arc::clone(&keyframe);
        let encode = thread::Builder::new()
            .name("nova-video-enc".into())
            .spawn(move || encode_loop(&frame_rx, &packet_tx, &encode_key, &encode_stop, &status));
        if capture.is_err() || encode.is_err() {
            stop.store(true, Ordering::Relaxed);
            self.update_status(|s| s.share_error = Some("Threads für den Stream fehlen".into()));
            return;
        }
        self.update_status(|s| {
            s.share_error = None;
            s.share_fps = 0;
        });
        self.screen = Some(Screen {
            state: ScreenState::NeedOffer,
            packets: packet_rx,
            keyframe,
            stop,
            started: Instant::now(),
            sent: 0,
            sent_since: Instant::now(),
        });
    }

    pub(super) fn stop_screen(&mut self) {
        let Some(screen) = self.screen.take() else {
            return;
        };
        screen.stop.store(true, Ordering::Relaxed);
        let mid = match screen.state {
            ScreenState::Active(mid) | ScreenState::Offered(mid) => Some(mid),
            ScreenState::NeedOffer => None,
        };
        self.pending = None;
        // Spur beim SFU abmelden, damit die anderen das Bild entfernen.
        if let Some(mid) = mid {
            let mut change = self.rtc.sdp_api();
            change.stop_media(mid);
            if let Some((offer, pending)) = change.apply()
                && self.send_offer(&offer)
            {
                self.pending = Some(pending);
            }
        }
        self.update_status(|s| {
            s.sharing = false;
            s.share_size = None;
            s.share_fps = 0;
        });
    }

    fn send_offer(&mut self, offer: &SdpOffer) -> bool {
        let Ok(json) = serde_json::to_string(offer) else {
            return false;
        };
        self.rtc
            .channel(self.channel)
            .is_some_and(|mut channel| channel.write(false, json.as_bytes()).is_ok())
    }

    /// Videospur aushandeln und kodierte Bilder als RTP verschicken.
    pub(super) fn pump_screen(&mut self) {
        let Some(state) = self.screen.as_ref().map(|s| s.state) else {
            return;
        };
        if self
            .screen
            .as_ref()
            .is_some_and(|s| s.stop.load(Ordering::Relaxed))
        {
            // Aufnahme oder Kodierer ist ausgefallen (Fehler steht im Status).
            self.stop_screen();
            return;
        }
        if state == ScreenState::NeedOffer && self.pending.is_none() {
            let mut change = self.rtc.sdp_api();
            let mid = change.add_media(
                MediaKind::Video,
                Direction::SendOnly,
                Some("screen".into()),
                None,
                None,
            );
            if let Some((offer, pending)) = change.apply()
                && self.send_offer(&offer)
            {
                self.pending = Some(pending);
                if let Some(screen) = self.screen.as_mut() {
                    screen.state = ScreenState::Offered(mid);
                }
            }
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        let ScreenState::Active(mid) = screen.state else {
            // Noch nicht ausgehandelt: Bilder verwerfen; nach der Answer
            // beginnt der Strom mit einem Schlüsselbild.
            while screen.packets.try_recv().is_ok() {}
            return;
        };
        let packets: Vec<Encoded> = screen.packets.try_iter().collect();
        let started = screen.started;
        let mut sent = 0;
        let mut size = None;
        for packet in packets {
            let Some(writer) = self.rtc.writer(mid) else {
                break;
            };
            let Some(pt) = writer
                .payload_params()
                .find(|p| p.spec().codec == Codec::H264)
                .map(|p| p.pt())
            else {
                break;
            };
            // RTP-Zeit in 90 kHz ab Streambeginn.
            let micros = packet
                .captured
                .saturating_duration_since(started)
                .as_micros();
            let ticks = u64::try_from(micros * 9 / 100).unwrap_or(u64::MAX);
            let time = MediaTime::new(ticks, Frequency::NINETY_KHZ);
            if writer.write(pt, packet.captured, time, packet.data).is_ok() {
                sent += 1;
                size = Some(packet.size);
            }
        }
        let Some(screen) = self.screen.as_mut() else {
            return;
        };
        screen.sent += sent;
        let fps = (screen.sent_since.elapsed() >= Duration::from_secs(1)).then(|| {
            let fps = screen.sent;
            screen.sent = 0;
            screen.sent_since = Instant::now();
            fps
        });
        if fps.is_some() || size.is_some() {
            self.update_status(|s| {
                if let Some(fps) = fps {
                    s.share_fps = fps;
                }
                if size.is_some() {
                    s.share_size = size;
                }
            });
        }
    }
}

// --------------------------------------------------------------- Threads

/// Bildschirm aufnehmen, höchstens [`video::FPS`] Bilder pro Sekunde. Ist der
/// Kodierer noch beschäftigt, wird das Bild verworfen.
fn capture_loop(
    tx: &SyncSender<(capture::Frame, Instant)>,
    stop: &AtomicBool,
    status: &Mutex<VoiceStatus>,
) {
    let fail = |message: String| {
        if let Ok(mut s) = status.lock() {
            s.share_error = Some(message);
        }
        stop.store(true, Ordering::Relaxed);
    };
    let mut capturer = match capture::open() {
        Ok(capturer) => capturer,
        Err(err) => return fail(err),
    };
    let interval = Duration::from_secs(1) / video::FPS;
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now < next {
            thread::sleep(next - now);
        }
        next += interval;
        if next < Instant::now() {
            // Hinterher: nicht nachholen, sondern im Takt weitermachen.
            next = Instant::now() + interval;
        }
        let captured = Instant::now();
        let frame = match capturer.grab() {
            Ok(frame) => frame,
            Err(err) => return fail(err),
        };
        match tx.try_send((frame, captured)) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => return,
        }
    }
}

fn encode_loop(
    rx: &Receiver<(capture::Frame, Instant)>,
    tx: &Sender<Encoded>,
    keyframe: &AtomicBool,
    stop: &AtomicBool,
    status: &Mutex<VoiceStatus>,
) {
    let mut encoder = match video::H264Encoder::new() {
        Ok(encoder) => encoder,
        Err(err) => {
            if let Ok(mut s) = status.lock() {
                s.share_error = Some(err);
            }
            stop.store(true, Ordering::Relaxed);
            return;
        }
    };
    while let Ok((frame, captured)) = rx.recv() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if keyframe.swap(false, Ordering::Relaxed) {
            encoder.force_keyframe();
        }
        let size = (frame.width, frame.height);
        match encoder.encode(&frame) {
            Ok(Some(data)) => {
                let packet = Encoded {
                    data,
                    captured,
                    size,
                };
                if tx.send(packet).is_err() {
                    return;
                }
            }
            Ok(None) => {}
            Err(err) => {
                if let Ok(mut s) = status.lock() {
                    s.share_error = Some(err);
                }
            }
        }
    }
}

/// Fremde Streams dekodieren, ein Dekodierer pro Spur. Bei Fehlern wird der
/// Dekodierer neu angelegt und über `keyframe` ein Schlüsselbild angefordert.
fn decode_loop(rx: &Receiver<(Mid, i64, Vec<u8>)>, keyframe: &Sender<Mid>, pictures: &Pictures) {
    let mut decoders: HashMap<Mid, video::H264Decoder> = HashMap::new();
    while let Ok((mid, user, data)) = rx.recv() {
        let decoder = match decoders.entry(mid) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => match video::H264Decoder::new() {
                Ok(decoder) => entry.insert(decoder),
                Err(_) => continue,
            },
        };
        match decoder.decode(&data) {
            Ok(Some(frame)) => {
                let picture = Picture {
                    handle: image::Handle::from_rgba(frame.width, frame.height, frame.rgba),
                    width: frame.width,
                    height: frame.height,
                    at: Instant::now(),
                };
                if let Ok(mut pictures) = pictures.lock() {
                    pictures.insert(user, picture);
                }
            }
            Ok(None) => {}
            Err(_) => {
                decoders.remove(&mid);
                if keyframe.send(mid).is_err() {
                    return;
                }
            }
        }
    }
}
