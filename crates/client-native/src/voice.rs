//! Native Sprachverbindung: str0m (wie der SFU), cpal für Mikrofon und
//! Lautsprecher, Opus über `unsafe-libopus` (reines Rust).
//!
//! Aufbau:
//! - **Netz-Thread**: besitzt `Rtc` und den UDP-Socket, kodiert Mikrofon-Frames,
//!   dekodiert ankommendes Opus, beantwortet Neuverhandlungen des SFU über den
//!   DataChannel `sdp`.
//! - **cpal-Callbacks**: Mikrofon schiebt Samples in einen Kanal, die
//!   Wiedergabe mischt alle Personen aus ihren Jitter-Puffern.
//!
//! Gepuffert wird nur so viel wie nötig: Wiedergabe startet ab 40 ms pro
//! Person, mehr als 200 ms werden verworfen (Latenz vor Lückenlosigkeit).
//!
//! Streams (Bildschirm teilen) laufen über dieselbe Verbindung, siehe
//! `voice/stream.rs`: Aufnahme- und Kodier-Thread beim Senden, ein eigener
//! Dekodier-Thread beim Empfangen (Audio stockt so nicht).

mod stream;

use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use str0m::{
    Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig,
    change::{SdpAnswer, SdpOffer, SdpPendingOffer},
    channel::ChannelId,
    format::Codec,
    media::{Direction, Frequency, MediaKind, MediaTime, Mid},
    net::{Protocol, Receive},
};

use crate::api::Api;
pub use stream::Picture;
use stream::{Pictures, Screen};

/// Wie im SFU (`crates/voice-sfu`, `rtc_config`): Opus und H.264 Constrained
/// Baseline mit `packetization-mode=1`, Payload-Typ 108 (RTX 109).
fn rtc_config() -> RtcConfig {
    let mut config = RtcConfig::new().clear_codecs().enable_opus(true, false);
    config
        .codec_config()
        .add_h264(108.into(), Some(109.into()), true, 0x42e01f);
    config
}

/// Opus arbeitet hier mit 48 kHz mono und 20-ms-Frames.
const RATE: u32 = 48_000;
const FRAME: usize = 960;
/// Ab diesem Füllstand startet die Wiedergabe einer Person (40 ms).
const START_SAMPLES: usize = FRAME * 2;
/// Obergrenze pro Person (200 ms); darüber wird Altes verworfen.
const MAX_SAMPLES: usize = FRAME * 10;
const SPEAKING_RMS: f32 = 0.02;
const SPEAKING_HOLD: Duration = Duration::from_millis(350);

/// Was die Oberfläche anzeigen kann.
#[derive(Debug, Clone, Default)]
pub struct VoiceStatus {
    pub connected: bool,
    pub ended: bool,
    pub error: Option<String>,
    /// Nutzer-IDs, die gerade sprechen (inklusive der eigenen).
    pub speaking: Vec<i64>,
    pub has_mic: bool,
    pub packets_in: u64,
    pub packets_out: u64,
    /// Eigener Stream läuft (Videospur ausgehandelt, Bilder werden gesendet).
    pub sharing: bool,
    /// Größe des eigenen Streams.
    pub share_size: Option<(usize, usize)>,
    /// Gesendete Bilder pro Sekunde (eigener Stream).
    pub share_fps: u32,
    /// Fehler beim Teilen (Aufnahme, Wayland …); der Ton läuft weiter.
    pub share_error: Option<String>,
}

enum Command {
    Muted(bool),
    Deafened(bool),
    StartScreen,
    StopScreen,
    Leave,
}

/// Griff auf eine laufende Verbindung. Fallenlassen beendet sie.
pub struct VoiceHandle {
    pub channel_id: i64,
    commands: Sender<Command>,
    status: Arc<Mutex<VoiceStatus>>,
    pictures: Pictures,
}

impl std::fmt::Debug for VoiceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceHandle")
            .field("channel_id", &self.channel_id)
            .finish()
    }
}

impl VoiceHandle {
    pub fn status(&self) -> VoiceStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_muted(&self, muted: bool) {
        let _ = self.commands.send(Command::Muted(muted));
    }

    pub fn set_deafened(&self, deafened: bool) {
        let _ = self.commands.send(Command::Deafened(deafened));
    }

    pub fn share_screen(&self, on: bool) {
        let _ = self.commands.send(if on {
            Command::StartScreen
        } else {
            Command::StopScreen
        });
    }

    /// Letztes Bild des Streams einer Person (`None`: streamt nicht).
    pub fn picture(&self, user_id: i64) -> Option<Picture> {
        stream::picture(&self.pictures, user_id)
    }

    /// Wer gerade sichtbar streamt.
    pub fn streamers(&self) -> Vec<i64> {
        stream::streamers(&self.pictures)
    }
}

impl Drop for VoiceHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Leave);
    }
}

/// Lokale IP, über die der Server erreichbar ist (ohne ein Paket zu senden).
fn local_ip_towards(api: &Api) -> IpAddr {
    let host = api
        .base()
        .split("://")
        .nth(1)
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let target = if host.contains(':') {
        host
    } else {
        format!("{host}:443")
    };
    let probe = || -> Option<IpAddr> {
        let addr = target.to_socket_addrs().ok()?.find(SocketAddr::is_ipv4)?;
        let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect(addr).ok()?;
        Some(socket.local_addr().ok()?.ip())
    };
    probe().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

/// Verbindung aufbauen: Offer erzeugen, per REST tauschen, Threads starten.
pub async fn join(api: Api, channel_id: i64) -> Result<VoiceHandle, String> {
    let ip = local_ip_towards(&api);
    let socket = UdpSocket::bind(SocketAddr::new(ip, 0))
        .or_else(|_| UdpSocket::bind("0.0.0.0:0"))
        .map_err(|err| format!("UDP-Socket: {err}"))?;
    let local = socket.local_addr().map_err(|err| err.to_string())?;
    let local = SocketAddr::new(ip, local.port());

    let mut rtc = rtc_config().build(Instant::now());
    let candidate = Candidate::host(local, "udp").map_err(|err| format!("ICE-Kandidat: {err}"))?;
    rtc.add_local_candidate(candidate);

    let mut change = rtc.sdp_api();
    let mid = change.add_media(
        MediaKind::Audio,
        Direction::SendRecv,
        Some("nova".into()),
        None,
        None,
    );
    let channel = change.add_channel("sdp".into());
    let (offer, pending) = change.apply().ok_or("Kein Offer erzeugt")?;
    let offer_json = serde_json::to_string(&offer).map_err(|err| err.to_string())?;

    let answer_json = api.voice_join(channel_id, &offer_json).await?;
    let answer: SdpAnswer =
        serde_json::from_str(&answer_json).map_err(|err| format!("Antwort des SFU: {err}"))?;
    rtc.sdp_api()
        .accept_answer(pending, answer)
        .map_err(|err| format!("Answer abgelehnt: {err}"))?;

    let status = Arc::new(Mutex::new(VoiceStatus::default()));
    let pictures: Pictures = Arc::default();
    let (commands, rx) = mpsc::channel();
    let engine_status = Arc::clone(&status);
    let engine_pictures = Arc::clone(&pictures);
    thread::Builder::new()
        .name("nova-voice".into())
        .spawn(move || {
            let mut engine = Engine::new(rtc, socket, local, mid, channel, engine_status);
            engine.pictures = engine_pictures;
            engine.run(rx);
        })
        .map_err(|err| err.to_string())?;

    Ok(VoiceHandle {
        channel_id,
        commands,
        status,
        pictures,
    })
}

// ------------------------------------------------------------------- Audio

/// Gemeinsamer Zustand zwischen Netz-Thread und Wiedergabe-Callback.
#[derive(Default)]
struct Mixer {
    /// Jitter-Puffer pro eingehender Spur, 48 kHz mono.
    streams: HashMap<Mid, Stream>,
}

#[derive(Default)]
struct Stream {
    samples: VecDeque<f32>,
    playing: bool,
}

impl Mixer {
    fn push(&mut self, mid: Mid, pcm: &[f32]) {
        let stream = self.streams.entry(mid).or_default();
        stream.samples.extend(pcm.iter().copied());
        while stream.samples.len() > MAX_SAMPLES {
            stream.samples.pop_front();
        }
        if stream.samples.len() >= START_SAMPLES {
            stream.playing = true;
        }
    }

    /// Ein Sample (48 kHz) aus allen Personen gemischt.
    fn next(&mut self) -> f32 {
        let mut sum = 0.0;
        for stream in self.streams.values_mut() {
            if !stream.playing {
                continue;
            }
            match stream.samples.pop_front() {
                Some(sample) => sum += sample,
                // Puffer leer: neu anfüllen lassen statt zu knacksen.
                None => stream.playing = false,
            }
        }
        sum.clamp(-1.0, 1.0)
    }
}

/// Lineare Umrechnung zwischen Geräterate und 48 kHz – für Sprache ausreichend.
struct Resampler {
    step: f64,
    pos: f64,
    last: f32,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self {
            step: f64::from(from) / f64::from(to),
            pos: 0.0,
            last: 0.0,
        }
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for &sample in input {
            while self.pos < 1.0 {
                out.push(self.last + (sample - self.last) * self.pos as f32);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.last = sample;
        }
    }
}

struct Audio {
    _input: Option<cpal::Stream>,
    _output: Option<cpal::Stream>,
    mic: Option<Receiver<Vec<f32>>>,
}

/// Gewählte Geräte (Name); `None` = Systemstandard. Gilt ab dem nächsten Beitritt.
static DEVICES: Mutex<(Option<String>, Option<String>)> = Mutex::new((None, None));

pub fn set_devices(input: Option<String>, output: Option<String>) {
    if let Ok(mut devices) = DEVICES.lock() {
        *devices = (input, output);
    }
}

fn device_name(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_owned())
}

/// Namen aller Ein- und Ausgabegeräte für die Einstellungen.
pub fn list_devices() -> (Vec<String>, Vec<String>) {
    let host = cpal::default_host();
    let names = |devices: Option<Vec<cpal::Device>>| -> Vec<String> {
        let mut list: Vec<String> = devices
            .unwrap_or_default()
            .iter()
            .filter_map(device_name)
            .collect();
        list.dedup();
        list
    };
    (
        names(host.input_devices().ok().map(Iterator::collect)),
        names(host.output_devices().ok().map(Iterator::collect)),
    )
}

fn pick(
    devices: Option<Vec<cpal::Device>>,
    wanted: Option<&str>,
    fallback: Option<cpal::Device>,
) -> Option<cpal::Device> {
    wanted
        .and_then(|name| {
            devices?
                .into_iter()
                .find(|d| device_name(d).as_deref() == Some(name))
        })
        .or(fallback)
}

fn start_audio(mixer: Arc<Mutex<Mixer>>, deafened: Arc<AtomicBool>) -> (Audio, Option<String>) {
    let host = cpal::default_host();
    let mut warning = None;
    let (want_in, want_out) = DEVICES.lock().map(|d| d.clone()).unwrap_or_default();
    let input_device = pick(
        host.input_devices().ok().map(Iterator::collect),
        want_in.as_deref(),
        host.default_input_device(),
    );
    let output_device = pick(
        host.output_devices().ok().map(Iterator::collect),
        want_out.as_deref(),
        host.default_output_device(),
    );

    // Mikrofon → mono 48 kHz → Kanal zum Netz-Thread.
    let (mic_tx, mic_rx) = mpsc::channel::<Vec<f32>>();
    let input = input_device.and_then(|device| {
        let config = device.default_input_config().ok()?;
        let channels = usize::from(config.channels());
        let mut resampler = Resampler::new(config.sample_rate(), RATE);
        let stream_config = config.config();
        let stream = device
            .build_input_stream::<f32, _, _>(
                stream_config,
                move |data: &[f32], _| {
                    let mono: Vec<f32> = data
                        .chunks(channels.max(1))
                        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
                        .collect();
                    let mut out = Vec::with_capacity(mono.len() + 8);
                    resampler.process(&mono, &mut out);
                    let _ = mic_tx.send(out);
                },
                |err| eprintln!("[nova-voice] Mikrofon: {err}"),
                None,
            )
            .ok()?;
        stream.play().ok()?;
        Some(stream)
    });
    if input.is_none() {
        warning = Some("Kein Mikrofon gefunden – du kannst nur zuhören.".to_owned());
    }

    // Wiedergabe: 48 kHz gemischt → Geräterate/Kanäle.
    let output = output_device.and_then(|device| {
        let config = device.default_output_config().ok()?;
        let channels = usize::from(config.channels());
        let step = f64::from(RATE) / f64::from(config.sample_rate());
        let mut pos = 0.0f64;
        let mut current = 0.0f32;
        let mut next = 0.0f32;
        let stream = device
            .build_output_stream::<f32, _, _>(
                config.config(),
                move |data: &mut [f32], _| {
                    let silent = deafened.load(Ordering::Relaxed);
                    let Ok(mut mixer) = mixer.lock() else {
                        data.fill(0.0);
                        return;
                    };
                    for frame in data.chunks_mut(channels.max(1)) {
                        while pos >= 1.0 {
                            current = next;
                            next = mixer.next();
                            pos -= 1.0;
                        }
                        let sample = current + (next - current) * pos as f32;
                        pos += step;
                        let value = if silent { 0.0 } else { sample };
                        frame.fill(value);
                    }
                },
                |err| eprintln!("[nova-voice] Lautsprecher: {err}"),
                None,
            )
            .ok()?;
        stream.play().ok()?;
        Some(stream)
    });
    if output.is_none() {
        warning = Some("Kein Ausgabegerät gefunden.".to_owned());
    }

    (
        Audio {
            mic: input.as_ref().map(|_| mic_rx),
            _input: input,
            _output: output,
        },
        warning,
    )
}

// -------------------------------------------------------------------- Opus

struct Encoder(*mut unsafe_libopus::OpusEncoder);
struct Decoder(*mut unsafe_libopus::OpusDecoder);

// Beide werden nur im Netz-Thread benutzt.
unsafe impl Send for Encoder {}
unsafe impl Send for Decoder {}

impl Encoder {
    fn new() -> Option<Self> {
        let mut error = 0;
        // SAFETY: Parameter sind gültig (48 kHz, mono, VOIP); der Zeiger wird
        // auf null geprüft und nur in `Drop` freigegeben.
        let ptr = unsafe {
            unsafe_libopus::opus_encoder_create(
                RATE as i32,
                1,
                unsafe_libopus::OPUS_APPLICATION_VOIP,
                &mut error,
            )
        };
        (!ptr.is_null() && error == 0).then_some(Self(ptr))
    }

    fn encode(&mut self, pcm: &[f32]) -> Option<Vec<u8>> {
        let samples: Vec<i16> = pcm
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)
            .collect();
        let mut out = vec![0u8; 1275];
        // SAFETY: `samples` hat genau FRAME Werte, `out` 1275 Bytes (Opus-Maximum).
        let len = unsafe {
            unsafe_libopus::opus_encode(
                self.0,
                samples.as_ptr(),
                FRAME as i32,
                out.as_mut_ptr(),
                out.len() as i32,
            )
        };
        (len > 0).then(|| {
            out.truncate(len as usize);
            out
        })
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: Zeiger stammt aus `opus_encoder_create`, wird genau einmal freigegeben.
        unsafe { unsafe_libopus::opus_encoder_destroy(self.0) }
    }
}

impl Decoder {
    fn new() -> Option<Self> {
        let mut error = 0;
        // SAFETY: gültige Parameter, Zeiger wird geprüft.
        let ptr = unsafe { unsafe_libopus::opus_decoder_create(RATE as i32, 1, &mut error) };
        (!ptr.is_null() && error == 0).then_some(Self(ptr))
    }

    fn decode(&mut self, packet: &[u8]) -> Vec<f32> {
        let mut pcm = vec![0i16; FRAME * 6];
        // SAFETY: Puffer fasst 120 ms (Opus-Maximum pro Paket).
        let samples = unsafe {
            unsafe_libopus::opus_decode(
                self.0,
                packet.as_ptr(),
                packet.len() as i32,
                pcm.as_mut_ptr(),
                pcm.len() as i32,
                0,
            )
        };
        if samples <= 0 {
            return Vec::new();
        }
        pcm.truncate(samples as usize);
        pcm.iter()
            .map(|s| f32::from(*s) / f32::from(i16::MAX))
            .collect()
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: Zeiger stammt aus `opus_decoder_create`, wird genau einmal freigegeben.
        unsafe { unsafe_libopus::opus_decoder_destroy(self.0) }
    }
}

fn rms(pcm: &[f32]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32).sqrt()
}

// ------------------------------------------------------------------ Engine

struct Engine {
    rtc: Rtc,
    socket: UdpSocket,
    local: SocketAddr,
    mid: Mid,
    channel: ChannelId,
    status: Arc<Mutex<VoiceStatus>>,
    mixer: Arc<Mutex<Mixer>>,
    deafened: Arc<AtomicBool>,
    muted: bool,
    encoder: Option<Encoder>,
    decoders: HashMap<Mid, Decoder>,
    /// Letzter lauter Frame pro Person (für den Sprecher-Ring).
    loud: HashMap<i64, Instant>,
    my_loud: Option<Instant>,
    mic_buffer: Vec<f32>,
    rtp_time: u64,

    /// Fremde Streams: H.264-Bilder gehen an den Dekodier-Thread.
    pictures: Pictures,
    decode_tx: Option<mpsc::SyncSender<(Mid, i64, Vec<u8>)>>,
    keyframe_rx: Option<Receiver<Mid>>,
    keyframe_asked: HashMap<Mid, Instant>,
    /// Eigener Stream.
    screen: Option<Screen>,
    /// Eigenes, noch unbeantwortetes Offer (Videospur hinzufügen/entfernen).
    pending: Option<SdpPendingOffer>,
}

impl Engine {
    fn new(
        rtc: Rtc,
        socket: UdpSocket,
        local: SocketAddr,
        mid: Mid,
        channel: ChannelId,
        status: Arc<Mutex<VoiceStatus>>,
    ) -> Self {
        Self {
            rtc,
            socket,
            local,
            mid,
            channel,
            status,
            mixer: Arc::new(Mutex::new(Mixer::default())),
            deafened: Arc::new(AtomicBool::new(false)),
            muted: false,
            encoder: Encoder::new(),
            decoders: HashMap::new(),
            loud: HashMap::new(),
            my_loud: None,
            mic_buffer: Vec::with_capacity(FRAME * 4),
            rtp_time: 0,
            pictures: Arc::default(),
            decode_tx: None,
            keyframe_rx: None,
            keyframe_asked: HashMap::new(),
            screen: None,
            pending: None,
        }
    }

    fn update_status(&self, f: impl FnOnce(&mut VoiceStatus)) {
        if let Ok(mut status) = self.status.lock() {
            f(&mut status);
        }
    }

    fn run(&mut self, commands: Receiver<Command>) {
        let (audio, warning) = start_audio(Arc::clone(&self.mixer), Arc::clone(&self.deafened));
        let has_mic = audio.mic.is_some();
        self.update_status(|s| {
            s.has_mic = has_mic;
            s.error = warning;
        });
        let mut buf = vec![0u8; 2000];
        let mut last_status = Instant::now();

        loop {
            loop {
                match commands.try_recv() {
                    Ok(Command::Muted(muted)) => self.muted = muted,
                    Ok(Command::Deafened(deafened)) => {
                        self.deafened.store(deafened, Ordering::Relaxed);
                    }
                    Ok(Command::StartScreen) => self.start_screen(),
                    Ok(Command::StopScreen) => self.stop_screen(),
                    Ok(Command::Leave) | Err(TryRecvError::Disconnected) => {
                        self.stop_screen();
                        self.rtc.disconnect();
                        self.update_status(|s| s.ended = true);
                        return;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }

            // Mikrofon → 20-ms-Opus-Frames → RTP.
            if let Some(mic) = &audio.mic {
                while let Ok(chunk) = mic.try_recv() {
                    self.mic_buffer.extend(chunk);
                }
                while self.mic_buffer.len() >= FRAME {
                    let frame: Vec<f32> = self.mic_buffer.drain(..FRAME).collect();
                    self.send_frame(&frame);
                }
            }

            self.pump_screen();
            self.pump_keyframe_requests();
            // str0m gibt frisch geschriebene Pakete erst nach einem Zeitschritt
            // frei; ohne ihn warteten sie bis zum nächsten Durchlauf (≤ 5 ms).
            let _ = self.rtc.handle_input(Input::Timeout(Instant::now()));

            let timeout = match self.drive() {
                Some(timeout) => timeout,
                None => {
                    self.update_status(|s| {
                        s.ended = true;
                        s.connected = false;
                    });
                    return;
                }
            };

            // Bis zum nächsten Ereignis warten, aber spätestens nach 5 ms wieder
            // das Mikrofon abholen.
            let wait = timeout
                .saturating_duration_since(Instant::now())
                .clamp(Duration::from_millis(1), Duration::from_millis(5));
            self.socket.set_read_timeout(Some(wait)).ok();
            if let Ok((len, source)) = self.socket.recv_from(&mut buf)
                && let Ok(contents) = buf[..len].try_into()
            {
                let input = Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Udp,
                        source,
                        destination: self.local,
                        contents,
                    },
                );
                if self.rtc.accepts(&input) && self.rtc.handle_input(input).is_err() {
                    self.update_status(|s| s.error = Some("Verbindungsfehler".into()));
                }
            }
            let _ = self.rtc.handle_input(Input::Timeout(Instant::now()));

            if last_status.elapsed() > Duration::from_millis(100) {
                last_status = Instant::now();
                self.publish_speaking();
            }
        }
    }

    fn send_frame(&mut self, frame: &[f32]) {
        let level = rms(frame);
        if !self.muted && level > SPEAKING_RMS {
            self.my_loud = Some(Instant::now());
        }
        let pcm: Vec<f32> = if self.muted {
            vec![0.0; FRAME]
        } else {
            frame.to_vec()
        };
        self.rtp_time += FRAME as u64;
        let Some(encoder) = self.encoder.as_mut() else {
            return;
        };
        let Some(packet) = encoder.encode(&pcm) else {
            return;
        };
        let Some(writer) = self.rtc.writer(self.mid) else {
            return;
        };
        let Some(pt) = writer
            .payload_params()
            .find(|p| p.spec().codec == Codec::Opus)
            .map(|p| p.pt())
        else {
            return;
        };
        let time = MediaTime::new(self.rtp_time, Frequency::FORTY_EIGHT_KHZ);
        if writer.write(pt, Instant::now(), time, packet).is_ok() {
            self.update_status(|s| s.packets_out += 1);
        }
    }

    /// Ausgaben von str0m abarbeiten. `None` = Verbindung beendet.
    fn drive(&mut self) -> Option<Instant> {
        loop {
            if !self.rtc.is_alive() {
                return None;
            }
            let output = self.rtc.poll_output().ok()?;
            match output {
                Output::Timeout(at) => return Some(at),
                Output::Transmit(transmit) => {
                    let _ = self
                        .socket
                        .send_to(&transmit.contents, transmit.destination);
                }
                Output::Event(event) => self.handle_event(event),
            }
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::IceConnectionStateChange(state) => match state {
                IceConnectionState::Connected | IceConnectionState::Completed => {
                    self.update_status(|s| s.connected = true);
                }
                IceConnectionState::Disconnected => {
                    self.update_status(|s| {
                        s.connected = false;
                        s.error = Some("Sprachverbindung getrennt".into());
                    });
                    self.rtc.disconnect();
                }
                _ => {}
            },
            Event::ChannelData(data) => {
                // Der SFU verhandelt neue/entfernte Spuren über den DataChannel;
                // auf unser eigenes Offer (Stream) kommt dort die Answer.
                if let Ok(offer) = serde_json::from_slice::<SdpOffer>(&data.data) {
                    self.accept_remote_offer(offer);
                } else if let Ok(answer) = serde_json::from_slice::<SdpAnswer>(&data.data) {
                    self.accept_remote_answer(answer);
                }
            }
            Event::KeyframeRequest(request) => self.on_keyframe_request(request.mid),
            Event::MediaData(data) => {
                if data.mid == self.mid {
                    return;
                }
                if data.params.spec().codec == Codec::H264 {
                    self.handle_video(data.mid, data.contiguous, &data.data);
                    return;
                }
                self.update_status(|s| s.packets_in += 1);
                let decoder = match self.decoders.entry(data.mid) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => match Decoder::new() {
                        Some(decoder) => entry.insert(decoder),
                        None => return,
                    },
                };
                let pcm = decoder.decode(&data.data);
                if rms(&pcm) > SPEAKING_RMS
                    && let Some(user) = self.user_for(data.mid)
                {
                    self.loud.insert(user, Instant::now());
                }
                if let Ok(mut mixer) = self.mixer.lock() {
                    mixer.push(data.mid, &pcm);
                }
            }
            _ => {}
        }
    }

    /// Stream-ID der Spur ist `u{user_id}` (vom SFU vergeben).
    fn user_for(&self, mid: Mid) -> Option<i64> {
        self.rtc
            .media(mid)?
            .stream_id()
            .strip_prefix('u')?
            .parse()
            .ok()
    }

    fn publish_speaking(&mut self) {
        let now = Instant::now();
        self.loud
            .retain(|_, at| now.duration_since(*at) < SPEAKING_HOLD);
        let mut speaking: Vec<i64> = self.loud.keys().copied().collect();
        let me_speaking = self
            .my_loud
            .is_some_and(|at| now.duration_since(at) < SPEAKING_HOLD);
        speaking.sort_unstable();
        self.update_status(|s| {
            s.speaking = speaking;
            if me_speaking {
                // Eigene ID setzt die Oberfläche ein (kennt sie besser).
                s.speaking.push(-1);
            }
        });
    }
}
