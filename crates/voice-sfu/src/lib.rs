//! Eigener SFU auf Basis von str0m.
//!
//! Ein UDP-Socket pro Sprachkanal. Alle Teilnehmer eines Kanals multiplexen
//! ihre ICE-/DTLS-/RTP-Pakete über diesen Socket; eingehende RTP-Pakete werden
//! unverändert an alle anderen Teilnehmer weitergeleitet (kein Mixing,
//! kein Transcoding). Die SDP-Aushandlung läuft über HTTP (initial) und über
//! einen DataChannel (spätere Offers, z. B. Bildschirmfreigabe).

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    ops::Deref,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use nova_common::{
    metrics::{Histogram, HistogramSnapshot},
    protocol::UserPublic,
};
use serde::Serialize;
use str0m::{
    Candidate, Event, IceConnectionState, Input, Output, Rtc,
    change::{SdpAnswer, SdpOffer, SdpPendingOffer},
    channel::{ChannelData, ChannelId},
    media::{Direction, KeyframeRequest, KeyframeRequestKind, MediaData, MediaKind, Mid, Rid},
    net::{Protocol, Receive},
};
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub enum VoiceError {
    Disabled,
    Unavailable,
    Full,
    InvalidOffer(String),
}

impl fmt::Display for VoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => write!(f, "voice is disabled"),
            Self::Unavailable => write!(f, "voice backend unavailable"),
            Self::Full => write!(f, "voice channel is full"),
            Self::InvalidOffer(reason) => write!(f, "invalid SDP offer: {reason}"),
        }
    }
}

impl std::error::Error for VoiceError {}

/// Höchstens so viele gleichzeitige Videospuren (Bildschirme und Kameras) pro
/// Raum. Mehr würde auf dem 2-vCore-VPS die Upload-Bandbreite sprengen.
pub const MAX_VIDEO_TRACKS: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct Participant {
    pub user_id: i64,
    pub handle: String,
    pub display_name: String,
    pub muted: bool,
    pub screen: bool,
    pub deafened: bool,
    /// Kamera läuft (ab v0.8).
    pub camera: bool,
}

/// Wofür eine eingehende Spur steht. Bestimmt die Stream-ID beim
/// Weiterleiten: `u{id}` für Mikrofon und Bildschirm (wie vor v0.8, damit
/// ältere Clients weiter funktionieren), `s{id}` für den Ton der
/// Bildschirmfreigabe, `c{id}` für die Kamera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackRole {
    Mic,
    Screen,
    ScreenAudio,
    Camera,
}

impl TrackRole {
    /// Name im DataChannel-Offer (`"roles": {"<mid>": "camera"}`).
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "mic" => Some(Self::Mic),
            "screen" => Some(Self::Screen),
            "screen-audio" => Some(Self::ScreenAudio),
            "camera" => Some(Self::Camera),
            _ => None,
        }
    }

    /// Ohne Angabe (Clients vor v0.8): Ton ist Mikrofon, Bild ist Bildschirm.
    fn default_for(kind: MediaKind) -> Self {
        match kind {
            MediaKind::Audio => Self::Mic,
            MediaKind::Video => Self::Screen,
        }
    }

    fn fits(self, kind: MediaKind) -> bool {
        match self {
            Self::Mic | Self::ScreenAudio => kind == MediaKind::Audio,
            Self::Screen | Self::Camera => kind == MediaKind::Video,
        }
    }

    pub fn stream_id(self, user_id: i64) -> String {
        match self {
            Self::Mic | Self::Screen => format!("u{user_id}"),
            Self::ScreenAudio => format!("s{user_id}"),
            Self::Camera => format!("c{user_id}"),
        }
    }
}

/// Simulcast-Schichten, die Browser senden: hoch, mittel, niedrig.
pub const LAYERS: [&str; 3] = ["h", "m", "l"];
/// Eine Schicht gilt als ausgefallen, wenn so lange nichts von ihr kam.
const LAYER_STALE: Duration = Duration::from_millis(1500);

/// Schichtwahl einer weitergeleiteten Videospur für **einen** Empfänger.
///
/// Gewechselt wird nur an einem Schlüsselbild der neuen Schicht (sonst sieht
/// der Empfänger Artefakte); bis dahin läuft die alte weiter. Sendet der
/// Absender die gewünschte Schicht nicht (Bandbreite, kleines Fenster), wird
/// die nächstbeste genommen.
#[derive(Debug, Clone)]
struct Layers {
    want: Rid,
    active: Option<Rid>,
    seen: Vec<(Rid, Instant)>,
    /// Auf ein Schlüsselbild dieser Schicht wird gewartet.
    waiting: Option<Rid>,
}

impl Default for Layers {
    fn default() -> Self {
        Self {
            want: "h".into(),
            active: None,
            seen: Vec::new(),
            waiting: None,
        }
    }
}

impl Layers {
    fn fresh(&self, rid: Rid, now: Instant) -> bool {
        self.seen
            .iter()
            .any(|(seen, at)| *seen == rid && now.duration_since(*at) < LAYER_STALE)
    }

    fn note(&mut self, rid: Rid, now: Instant) {
        match self.seen.iter_mut().find(|(seen, _)| *seen == rid) {
            Some(entry) => entry.1 = now,
            None => self.seen.push((rid, now)),
        }
    }

    /// Schicht, die gerade weitergeleitet werden soll.
    fn target(&self, now: Instant) -> Rid {
        if self.fresh(self.want, now) {
            return self.want;
        }
        let order: [&str; 3] = match self.want.as_ref() {
            "l" => ["l", "m", "h"],
            "m" => ["m", "l", "h"],
            _ => ["h", "m", "l"],
        };
        order
            .into_iter()
            .map(Rid::from)
            .find(|rid| self.fresh(*rid, now))
            .unwrap_or(self.want)
    }

    /// Paket dieser Schicht weiterleiten?
    fn accept(&mut self, rid: Rid, keyframe: bool, now: Instant) -> bool {
        self.note(rid, now);
        let target = self.target(now);
        if self.active == Some(rid) {
            if rid == target {
                self.waiting = None;
            }
            return true;
        }
        if rid != target {
            return false;
        }
        let active_stale = self.active.is_some_and(|active| !self.fresh(active, now));
        if self.active.is_none() || keyframe || active_stale {
            self.active = Some(rid);
            self.waiting = None;
            return true;
        }
        self.waiting = Some(rid);
        false
    }

    /// Empfänger wünscht eine andere Schicht. Liefert sie, wenn sich etwas ändert.
    fn request(&mut self, rid: Rid) -> Option<Rid> {
        if self.want == rid {
            return None;
        }
        self.want = rid;
        Some(rid)
    }
}

/// Zustandsänderung, die der SFU selbst bemerkt (Videospur kam an oder ist
/// weg, Verbindung abgerissen). Die App verteilt sie als `voice_state_update`.
#[derive(Debug, Clone)]
pub struct SfuEvent {
    pub channel_id: i64,
    pub user_id: i64,
    /// `None`: die Person ist nicht mehr im Raum.
    pub participant: Option<Participant>,
}

/// Meldet Änderungen eines Raums an die App.
#[derive(Debug, Clone)]
struct Notifier {
    channel_id: i64,
    tx: Sender<SfuEvent>,
    /// Weiterleitungszeit pro Paket (Empfang → an str0m übergeben), alle Räume.
    forward: Arc<Histogram>,
}

impl Notifier {
    fn send(&self, user_id: i64, participant: Option<Participant>) {
        let _ = self.tx.send(SfuEvent {
            channel_id: self.channel_id,
            user_id,
            participant,
        });
    }
}

#[derive(Debug, Clone)]
pub struct SfuConfig {
    pub enabled: bool,
    pub public_ip: IpAddr,
    pub port_start: u16,
    pub port_end: u16,
    pub max_users: usize,
}

impl SfuConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("NOVA_SFU_ENABLED")
            .map(|value| {
                !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "off" | "no"
                )
            })
            .unwrap_or(true);
        let public_ip = std::env::var("NOVA_SFU_PUBLIC_IP")
            .ok()
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let port_start = env_u16("NOVA_SFU_PORT_START", 40_000);
        let port_end = env_u16("NOVA_SFU_PORT_END", 40_100).max(port_start);
        let max_users = env_u16("NOVA_VOICE_MAX_USERS", 15).max(2) as usize;
        Self {
            enabled,
            public_ip,
            port_start,
            port_end,
            max_users,
        }
    }
}

fn env_u16(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

pub struct Sfu {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: SfuConfig,
    rooms: Mutex<HashMap<i64, Arc<Room>>>,
    events_tx: Sender<SfuEvent>,
    events_rx: Mutex<Option<Receiver<SfuEvent>>>,
    /// 16 Ki Messwerte (64 KB): Perzentile über die letzten Sekunden Betrieb.
    forward: Arc<Histogram>,
}

impl Sfu {
    pub fn new(cfg: SfuConfig) -> Self {
        if cfg.enabled {
            info!(
                public_ip = %cfg.public_ip,
                ports = format!("{}-{}", cfg.port_start, cfg.port_end),
                max_users = cfg.max_users,
                "SFU configured"
            );
        } else {
            info!("SFU disabled");
        }
        let (events_tx, events_rx) = mpsc::channel();
        Self {
            inner: Arc::new(Inner {
                cfg,
                rooms: Mutex::new(HashMap::new()),
                events_tx,
                events_rx: Mutex::new(Some(events_rx)),
                forward: Arc::new(Histogram::new(16_384)),
            }),
        }
    }

    /// Weiterleitungszeit pro Paket: vom Empfang im Raum-Thread bis zur
    /// Übergabe an die Verbindung des Empfängers (Budget < 5 ms).
    pub fn forward_latency(&self) -> HistogramSnapshot {
        self.inner.forward.snapshot()
    }

    /// Zeilen für `/metrics` (Prometheus-Text).
    pub fn render_metrics(&self) -> String {
        let forward = self.forward_latency();
        let rooms = self
            .inner
            .rooms
            .lock()
            .map(|rooms| rooms.values().filter(|room| room.alive()).count())
            .unwrap_or(0);
        format!(
            "# TYPE nova_sfu_rooms gauge\nnova_sfu_rooms {rooms}\n\
             # TYPE nova_sfu_forward_seconds gauge\n\
             nova_sfu_forward_p50_seconds {}\nnova_sfu_forward_p99_seconds {}\n\
             nova_sfu_forward_max_seconds {}\nnova_sfu_forward_packets_total {}\n",
            forward.p50 as f64 / 1e6,
            forward.p99 as f64 / 1e6,
            forward.max as f64 / 1e6,
            forward.count
        )
    }

    /// Ereignisse, die der SFU selbst bemerkt. Nur einmal abholbar.
    pub fn take_events(&self) -> Option<Receiver<SfuEvent>> {
        self.inner.events_rx.lock().ok()?.take()
    }

    pub fn from_env() -> Self {
        Self::new(SfuConfig::from_env())
    }

    pub fn config(&self) -> &SfuConfig {
        &self.inner.cfg
    }

    pub fn join(
        &self,
        channel_id: i64,
        user: &UserPublic,
        offer_json: &str,
    ) -> Result<String, VoiceError> {
        if !self.inner.cfg.enabled {
            return Err(VoiceError::Disabled);
        }
        let offer: SdpOffer = serde_json::from_str(offer_json)
            .map_err(|err| VoiceError::InvalidOffer(err.to_string()))?;
        let room = self.room(channel_id)?;
        let participant = Participant {
            user_id: user.id,
            handle: user.handle.clone(),
            display_name: user.display_name.clone(),
            muted: false,
            screen: false,
            deafened: false,
            camera: false,
        };
        room.join(participant, offer, self.inner.cfg.public_ip)
    }

    /// Vom Client gemeldeter Zustand. `None`, wenn die Person nicht im Raum ist.
    pub fn set_state(
        &self,
        channel_id: i64,
        user_id: i64,
        muted: bool,
        deafened: bool,
        screen: bool,
        camera: bool,
    ) -> Option<Participant> {
        let room = self
            .inner
            .rooms
            .lock()
            .ok()
            .and_then(|rooms| rooms.get(&channel_id).cloned())?;
        let mut participants = room.participants.lock().ok()?;
        let state = participants.get_mut(&user_id)?;
        state.public.muted = muted;
        state.public.deafened = deafened;
        // `screen = true` setzt erst der SFU, wenn wirklich eine Videospur
        // ankommt; der Client darf es nur zurücknehmen.
        if !screen {
            state.public.screen = false;
        }
        if !camera {
            state.public.camera = false;
        }
        Some(state.public.clone())
    }

    pub fn leave(&self, channel_id: i64, user_id: i64) {
        let room = self
            .inner
            .rooms
            .lock()
            .ok()
            .and_then(|rooms| rooms.get(&channel_id).cloned());
        if let Some(room) = room {
            room.remove(user_id);
        }
    }

    pub fn participants(&self, channel_id: i64) -> Vec<Participant> {
        let room = self
            .inner
            .rooms
            .lock()
            .ok()
            .and_then(|rooms| rooms.get(&channel_id).cloned());
        match room {
            Some(room) => room.participants(),
            None => Vec::new(),
        }
    }

    fn room(&self, channel_id: i64) -> Result<Arc<Room>, VoiceError> {
        let mut rooms = self
            .inner
            .rooms
            .lock()
            .map_err(|_| VoiceError::Unavailable)?;
        if let Some(room) = rooms.get(&channel_id)
            && room.alive()
        {
            return Ok(Arc::clone(room));
        }
        let notifier = Notifier {
            channel_id,
            tx: self.inner.events_tx.clone(),
            forward: Arc::clone(&self.inner.forward),
        };
        let room = Room::new(
            channel_id,
            &self.inner.cfg,
            self.inner.cfg.max_users,
            notifier,
        )?;
        rooms.insert(channel_id, Arc::clone(&room));
        Ok(room)
    }
}

fn _sfu_is_send_sync() {
    fn check<T: Send + Sync>() {}
    check::<Sfu>();
}

struct Room {
    port: u16,
    tx: Sender<RoomCommand>,
    participants: Arc<Mutex<HashMap<i64, ParticipantState>>>,
    alive: Arc<AtomicBool>,
    max_users: usize,
}

#[derive(Debug, Clone)]
struct ParticipantState {
    public: Participant,
}

impl Room {
    fn new(
        channel_id: i64,
        cfg: &SfuConfig,
        max_users: usize,
        notifier: Notifier,
    ) -> Result<Arc<Self>, VoiceError> {
        let mut socket = None;
        for port in cfg.port_start..=cfg.port_end {
            let bind = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
            if let Ok(candidate) = UdpSocket::bind(bind) {
                socket = Some((candidate, port));
                break;
            }
        }
        let Some((socket, port)) = socket else {
            warn!(channel_id, "no free SFU UDP port");
            return Err(VoiceError::Unavailable);
        };
        let (tx, rx) = mpsc::channel();
        let participants = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let thread_participants = Arc::clone(&participants);
        let thread_alive = Arc::clone(&alive);
        let public_ip = cfg.public_ip;
        thread::Builder::new()
            .name(format!("sfu-{channel_id}"))
            .spawn(move || {
                run_room(
                    socket,
                    SocketAddr::new(public_ip, port),
                    rx,
                    thread_participants,
                    thread_alive,
                    &notifier,
                );
            })
            .map_err(|_| VoiceError::Unavailable)?;
        info!(channel_id, port, "SFU room listening");
        Ok(Arc::new(Self {
            port,
            tx,
            participants,
            alive,
            max_users,
        }))
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    fn join(
        &self,
        participant: Participant,
        offer: SdpOffer,
        public_ip: IpAddr,
    ) -> Result<String, VoiceError> {
        {
            let mut participants = self
                .participants
                .lock()
                .map_err(|_| VoiceError::Unavailable)?;
            if participants.get(&participant.user_id).is_none()
                && participants.len() >= self.max_users
            {
                return Err(VoiceError::Full);
            }
            participants.insert(
                participant.user_id,
                ParticipantState {
                    public: participant.clone(),
                },
            );
        }

        let mut rtc = rtc_config().set_ice_lite(true).build(Instant::now());
        let candidate_addr = SocketAddr::new(public_ip, self.port);
        let candidate = Candidate::host(candidate_addr, "udp")
            .map_err(|err| VoiceError::InvalidOffer(err.to_string()))?;
        if rtc.add_local_candidate(candidate).is_none() {
            return Err(VoiceError::InvalidOffer(
                "candidate rejected by str0m".into(),
            ));
        }
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .map_err(|err| VoiceError::InvalidOffer(err.to_string()))?;
        if self
            .tx
            .send(RoomCommand::Add {
                user_id: participant.user_id,
                rtc: Box::new(rtc),
            })
            .is_err()
        {
            return Err(VoiceError::Unavailable);
        }
        serde_json::to_string(&answer)
            .map_err(|_| VoiceError::Unavailable)
            .map_err(|_| VoiceError::Unavailable)
    }

    fn remove(&self, user_id: i64) {
        if let Ok(mut participants) = self.participants.lock() {
            participants.remove(&user_id);
        }
        let _ = self.tx.send(RoomCommand::Remove { user_id });
    }

    fn participants(&self) -> Vec<Participant> {
        let mut list: Vec<Participant> = self
            .participants
            .lock()
            .map(|participants| {
                participants
                    .values()
                    .map(|state| state.public.clone())
                    .collect()
            })
            .unwrap_or_default();
        list.sort_by_key(|participant| participant.user_id);
        list
    }
}

enum RoomCommand {
    Add { user_id: i64, rtc: Box<Rtc> },
    Remove { user_id: i64 },
}

fn run_room(
    socket: UdpSocket,
    local_addr: SocketAddr,
    rx: Receiver<RoomCommand>,
    participants: Arc<Mutex<HashMap<i64, ParticipantState>>>,
    alive: Arc<AtomicBool>,
    notifier: &Notifier,
) {
    let mut clients: Vec<Client> = Vec::new();
    let mut to_propagate: VecDeque<Propagated> = VecDeque::new();
    let mut buf = vec![0; 2000];
    let mut idle_since = Instant::now();

    loop {
        loop {
            match rx.try_recv() {
                Ok(RoomCommand::Add { user_id, rtc }) => {
                    let mut client =
                        Client::new(*rtc, user_id, Arc::clone(&participants), notifier.clone());
                    for track in clients.iter().flat_map(|client| client.tracks_in.iter()) {
                        client.handle_track_open(Arc::downgrade(&track.id));
                    }
                    clients.retain(|client| client.user_id != user_id);
                    clients.push(client);
                    info!(user_id, clients = clients.len(), "SFU client added to room");
                    idle_since = Instant::now();
                }
                Ok(RoomCommand::Remove { user_id }) => {
                    if let Some(client) =
                        clients.iter_mut().find(|client| client.user_id == user_id)
                    {
                        client.rtc.disconnect();
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    info!("SFU room channel closed");
                    alive.store(false, Ordering::Relaxed);
                    return;
                }
            }
        }

        // Nur aufräumen, wenn wirklich jemand weg ist (sonst kostete jeder
        // Durchlauf eine Allokation, ein Lock und N² Vergleiche).
        if clients.iter().any(|client| !client.rtc.is_alive()) {
            let connected_before: Vec<i64> = clients.iter().map(|client| client.user_id).collect();
            clients.retain(|client| client.rtc.is_alive());
            if let Ok(mut participants) = participants.lock() {
                for user_id in connected_before {
                    if !clients.iter().any(|client| client.user_id == user_id)
                        && participants.remove(&user_id).is_some()
                    {
                        debug!(user_id, "SFU participant removed after disconnect");
                        notifier.send(user_id, None);
                    }
                }
            }
        }
        if clients.is_empty() {
            if idle_since.elapsed() > Duration::from_secs(300) {
                info!("SFU room idle, closing");
                break;
            }
        } else {
            idle_since = Instant::now();
        }

        let mut timeout = Instant::now() + Duration::from_millis(100);
        for client in clients.iter_mut() {
            let next = poll_until_timeout(client, &mut to_propagate, &socket);
            timeout = timeout.min(next);
        }

        // Alles Angefallene in einem Zug weiterleiten (bis 0.8 kostete jedes
        // einzelne Ereignis einen kompletten Durchlauf über alle Verbindungen).
        if !to_propagate.is_empty() {
            while let Some(propagated) = to_propagate.pop_front() {
                propagate(&propagated, &mut clients);
            }
            for client in clients.iter_mut() {
                to_propagate.extend(client.take_keyframe_requests());
            }
            // str0m gibt frisch geschriebene Pakete erst nach einem Zeitschritt
            // frei; ohne ihn warteten sie bis zum nächsten eingehenden Paket.
            let now = Instant::now();
            for client in &mut clients {
                client.handle_input(Input::Timeout(now));
            }
            continue;
        }

        // Auf das erste Paket warten, höchstens bis zum nächsten str0m-Timeout
        // (ist schon etwas fällig, gar nicht), dann ohne Warten nachlesen, was
        // im Puffer liegt. Bis 0.8 war der Socket schon beim ersten Lesen nicht
        // blockierend: der Raum-Thread drehte dauerhaft mit 100 % CPU.
        let wait = timeout.saturating_duration_since(Instant::now());
        let mut read = 0;
        if !wait.is_zero() {
            socket
                .set_read_timeout(Some(wait.max(Duration::from_micros(100))))
                .ok();
            if let Some(input) = read_socket_input(&socket, local_addr, &mut buf) {
                route_input(&mut clients, input);
                read = 1;
            }
        }
        if read > 0 || wait.is_zero() {
            socket.set_nonblocking(true).ok();
            while read < READ_BATCH {
                let Some(input) = read_socket_input(&socket, local_addr, &mut buf) else {
                    break;
                };
                route_input(&mut clients, input);
                read += 1;
            }
            socket.set_nonblocking(false).ok();
        }

        let now = Instant::now();
        for client in &mut clients {
            client.handle_input(Input::Timeout(now));
        }
    }

    alive.store(false, Ordering::Relaxed);
    if let Ok(mut participants) = participants.lock() {
        participants.clear();
    }
}

/// Höchstens so viele Pakete am Stück lesen, bevor die Verbindungen wieder
/// abgefragt werden. Die DTLS-Schicht von str0m hält pro Verbindung nur 30
/// unverarbeitete Datensätze; große SDP-Neuverhandlungen über den
/// DataChannel kamen bei vielen Personen darüber (bis 0.8: 256 → Abbruch
/// mit „receive queue full“).
const READ_BATCH: usize = 24;

fn route_input(clients: &mut [Client], input: Input) {
    if let Some(client) = clients.iter_mut().find(|client| client.accepts(&input)) {
        client.handle_input(input);
    }
}

fn poll_until_timeout(
    client: &mut Client,
    queue: &mut VecDeque<Propagated>,
    socket: &UdpSocket,
) -> Instant {
    loop {
        if !client.rtc.is_alive() {
            return Instant::now();
        }
        match client.poll_output(socket) {
            Propagated::Timeout(timeout) => return timeout,
            // Gesendet o. Ä.: nichts weiterzuleiten. (Bis 0.8 landete auch das
            // in der Warteschlange und kostete je einen ganzen Durchlauf.)
            Propagated::Noop => {}
            propagated => queue.push_back(propagated),
        }
    }
}

fn propagate(propagated: &Propagated, clients: &mut [Client]) {
    let Some(client_id) = propagated.client_id() else {
        return;
    };
    for client in clients {
        if client.id == client_id {
            continue;
        }
        match propagated {
            Propagated::TrackOpen(_, track_in) => client.handle_track_open(track_in.clone()),
            Propagated::MediaData(_, data) => client.handle_media_data_out(client_id, data),
            Propagated::KeyframeRequest(_, request, origin, mid_in) => {
                if *origin == client.id {
                    client.handle_keyframe_request(*request, *mid_in);
                }
            }
            Propagated::Noop | Propagated::Timeout(_) => {}
        }
    }
}

fn read_socket_input<'a>(
    socket: &UdpSocket,
    local_addr: SocketAddr,
    buf: &'a mut Vec<u8>,
) -> Option<Input<'a>> {
    buf.resize(2000, 0);
    match socket.recv_from(buf) {
        Ok((size, source)) => {
            buf.truncate(size);
            let contents = buf.as_slice().try_into().ok()?;
            Some(Input::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: local_addr,
                    contents,
                },
            ))
        }
        Err(_) => None,
    }
}

#[derive(Debug)]
struct Client {
    id: ClientId,
    user_id: i64,
    rtc: Rtc,
    pending: Option<SdpPendingOffer>,
    cid: Option<ChannelId>,
    tracks_in: Vec<TrackInEntry>,
    tracks_out: Vec<TrackOut>,
    /// Rollen der eigenen Spuren, aus den Offers des Clients.
    roles: HashMap<Mid, TrackRole>,
    /// Schlüsselbilder, die beim Absender angefordert werden sollen
    /// (Absender, dessen Mid, Schicht, eigene Mid).
    keyframe_requests: Vec<(ClientId, Mid, Option<Rid>, Mid)>,
    participants: Arc<Mutex<HashMap<i64, ParticipantState>>>,
    notifier: Notifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClientId(u64);

impl Deref for ClientId {
    type Target = u64;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
struct TrackIn {
    origin: ClientId,
    /// Nutzer hinter der Spur; Stream-ID siehe [`TrackRole::stream_id`].
    user_id: i64,
    mid: Mid,
    kind: MediaKind,
    role: TrackRole,
}

/// Stream-ID ausgehender Spuren. Der Web-Client ordnet darüber Audio-/Video-
/// Elemente Personen zu (Lautstärke, Sprecher-Ring, Kacheln).
/// Payload-Typen und Profil für Video. Der native Client
/// (`crates/client-native/src/voice.rs`) nutzt dieselben Werte.
pub const H264_PT: u8 = 108;
pub const H264_RTX_PT: u8 = 109;
/// Constrained Baseline, Level 3.1 – das kann jeder Browser und OpenH264.
pub const H264_PROFILE_LEVEL: u32 = 0x42e01f;

/// Codecs des SFU: Opus für Sprache, für Video nur H.264 Constrained Baseline
/// mit `packetization-mode=1`. Der SFU leitet RTP unverändert weiter; nur
/// wenn alle Teilnehmer denselben Codec haben, kann jeder jeden Stream
/// abspielen (Browser wie nativer Client mit OpenH264).
fn rtc_config() -> str0m::RtcConfig {
    let mut config = str0m::RtcConfig::new()
        .clear_codecs()
        .enable_opus(true, false);
    config.codec_config().add_h264(
        H264_PT.into(),
        Some(H264_RTX_PT.into()),
        true,
        H264_PROFILE_LEVEL,
    );
    config
}

#[derive(Debug)]
struct TrackInEntry {
    id: Arc<TrackIn>,
    last_keyframe_request: Option<Instant>,
}

#[derive(Debug)]
struct TrackOut {
    track_in: Weak<TrackIn>,
    state: TrackOutState,
    layers: Layers,
    last_keyframe_request: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackOutState {
    ToOpen,
    Negotiating(Mid),
    Open(Mid),
    ToStop(Mid),
    NegotiatingStop(Mid),
}

impl TrackOut {
    fn mid(&self) -> Option<Mid> {
        match self.state {
            TrackOutState::ToOpen => None,
            TrackOutState::Negotiating(mid)
            | TrackOutState::Open(mid)
            | TrackOutState::ToStop(mid)
            | TrackOutState::NegotiatingStop(mid) => Some(mid),
        }
    }
}

impl Client {
    fn new(
        rtc: Rtc,
        user_id: i64,
        participants: Arc<Mutex<HashMap<i64, ParticipantState>>>,
        notifier: Notifier,
    ) -> Self {
        static ID_COUNTER: AtomicU64 = AtomicU64::new(0);
        Self {
            id: ClientId(ID_COUNTER.fetch_add(1, Ordering::SeqCst)),
            user_id,
            rtc,
            pending: None,
            cid: None,
            tracks_in: Vec::new(),
            tracks_out: Vec::new(),
            roles: HashMap::new(),
            keyframe_requests: Vec::new(),
            participants,
            notifier,
        }
    }

    fn accepts(&self, input: &Input) -> bool {
        self.rtc.accepts(input)
    }

    fn handle_input(&mut self, input: Input) {
        if !self.rtc.is_alive() {
            return;
        }
        if let Err(err) = self.rtc.handle_input(input) {
            warn!(user_id = self.user_id, ?err, "SFU client disconnected");
            self.rtc.disconnect();
        }
    }

    fn poll_output(&mut self, socket: &UdpSocket) -> Propagated {
        if !self.rtc.is_alive() {
            return Propagated::Noop;
        }
        if self.negotiate_if_needed() {
            return Propagated::Noop;
        }
        match self.rtc.poll_output() {
            Ok(output) => self.handle_output(output, socket),
            Err(err) => {
                warn!(user_id = self.user_id, ?err, "SFU poll_output failed");
                self.rtc.disconnect();
                Propagated::Noop
            }
        }
    }

    fn handle_output(&mut self, output: Output, socket: &UdpSocket) -> Propagated {
        match output {
            Output::Transmit(transmit) => {
                if let Err(err) = socket.send_to(&transmit.contents, transmit.destination) {
                    debug!(?err, "SFU UDP send failed");
                }
                Propagated::Noop
            }
            Output::Timeout(timeout) => Propagated::Timeout(timeout),
            Output::Event(event) => match event {
                Event::IceConnectionStateChange(state) => {
                    if matches!(
                        state,
                        IceConnectionState::Connected | IceConnectionState::Completed
                    ) {
                        info!(user_id = self.user_id, ?state, "SFU ICE connected");
                    }
                    if state == IceConnectionState::Disconnected {
                        warn!(user_id = self.user_id, "SFU ICE disconnected");
                        self.rtc.disconnect();
                    }
                    Propagated::Noop
                }
                Event::MediaAdded(added) => self.handle_media_added(added.mid, added.kind),
                Event::MediaChanged(changed) => {
                    self.handle_media_changed(changed.mid, changed.direction)
                }
                Event::MediaData(data) => self.handle_media_data_in(data),
                Event::KeyframeRequest(request) => self.handle_incoming_keyframe_request(request),
                Event::ChannelOpen(cid, _) => {
                    self.cid = Some(cid);
                    Propagated::Noop
                }
                Event::ChannelData(data) => self.handle_channel_data(data),
                _ => Propagated::Noop,
            },
        }
    }

    fn handle_media_added(&mut self, mid: Mid, kind: MediaKind) -> Propagated {
        let role = self
            .roles
            .get(&mid)
            .copied()
            .filter(|role| role.fits(kind))
            .unwrap_or_else(|| TrackRole::default_for(kind));
        if kind == MediaKind::Video {
            let Ok(mut participants) = self.participants.lock() else {
                return Propagated::Noop;
            };
            let video_tracks: usize = participants
                .iter()
                .filter(|(user_id, _)| **user_id != self.user_id)
                .map(|(_, state)| {
                    usize::from(state.public.screen) + usize::from(state.public.camera)
                })
                .sum();
            if video_tracks >= MAX_VIDEO_TRACKS {
                warn!(
                    user_id = self.user_id,
                    "SFU: Video-Limit erreicht, Spur wird nicht weitergeleitet"
                );
                return Propagated::Noop;
            }
            if let Some(participant) = participants.get_mut(&self.user_id) {
                match role {
                    TrackRole::Camera => participant.public.camera = true,
                    _ => participant.public.screen = true,
                }
                self.notifier
                    .send(self.user_id, Some(participant.public.clone()));
            }
        }
        let entry = TrackInEntry {
            id: Arc::new(TrackIn {
                origin: self.id,
                user_id: self.user_id,
                mid,
                kind,
                role,
            }),
            last_keyframe_request: None,
        };
        let weak = Arc::downgrade(&entry.id);
        self.tracks_in.retain(|track| track.id.mid != mid);
        self.tracks_in.push(entry);
        Propagated::TrackOpen(self.id, weak)
    }

    /// Richtung geändert: sendet der Client auf dieser Mid nicht mehr (Freigabe
    /// oder Kamera beendet), fällt die Spur weg und die Empfänger verhandeln sie
    /// ab; sendet er wieder (Transceiver wiederverwendet), entsteht sie neu.
    fn handle_media_changed(&mut self, mid: Mid, direction: Direction) -> Propagated {
        let receiving = matches!(direction, Direction::RecvOnly | Direction::SendRecv);
        let existing = self.tracks_in.iter().position(|track| track.id.mid == mid);
        match (receiving, existing) {
            (false, Some(index)) => {
                let track = self.tracks_in.remove(index);
                if track.id.kind == MediaKind::Video
                    && let Ok(mut participants) = self.participants.lock()
                    && let Some(participant) = participants.get_mut(&self.user_id)
                {
                    match track.id.role {
                        TrackRole::Camera => participant.public.camera = false,
                        _ => participant.public.screen = false,
                    }
                    self.notifier
                        .send(self.user_id, Some(participant.public.clone()));
                }
                Propagated::Noop
            }
            (true, None) => {
                let kind = self
                    .rtc
                    .media(mid)
                    .map(|media| media.kind())
                    .unwrap_or(MediaKind::Audio);
                self.handle_media_added(mid, kind)
            }
            _ => Propagated::Noop,
        }
    }

    fn handle_media_data_in(&mut self, data: MediaData) -> Propagated {
        if !data.contiguous {
            self.request_keyframe_throttled(data.mid, data.rid, KeyframeRequestKind::Fir);
        }
        Propagated::MediaData(self.id, Box::new(data))
    }

    fn request_keyframe_throttled(
        &mut self,
        mid: Mid,
        rid: Option<Rid>,
        kind: KeyframeRequestKind,
    ) {
        let Some(mut writer) = self.rtc.writer(mid) else {
            return;
        };
        let Some(entry) = self.tracks_in.iter_mut().find(|entry| entry.id.mid == mid) else {
            return;
        };
        if entry
            .last_keyframe_request
            .map(|time| time.elapsed() < Duration::from_secs(1))
            .unwrap_or(false)
        {
            return;
        }
        let _ = writer.request_keyframe(rid, kind);
        entry.last_keyframe_request = Some(Instant::now());
    }

    fn handle_incoming_keyframe_request(&self, mut request: KeyframeRequest) -> Propagated {
        let Some(track_out) = self
            .tracks_out
            .iter()
            .find(|track| track.mid() == Some(request.mid))
        else {
            return Propagated::Noop;
        };
        let Some(track_in) = track_out.track_in.upgrade() else {
            return Propagated::Noop;
        };
        request.rid = track_out.layers.active;
        Propagated::KeyframeRequest(self.id, request, track_in.origin, track_in.mid)
    }

    fn negotiate_if_needed(&mut self) -> bool {
        if self.cid.is_none() || self.pending.is_some() {
            return false;
        }
        for track in &mut self.tracks_out {
            if let TrackOutState::Open(mid) = track.state
                && track.track_in.upgrade().is_none()
            {
                track.state = TrackOutState::ToStop(mid);
            }
        }

        let mut change = self.rtc.sdp_api();
        for track in &mut self.tracks_out {
            match track.state {
                TrackOutState::ToOpen => {
                    if let Some(track_in) = track.track_in.upgrade() {
                        let mid = change.add_media(
                            track_in.kind,
                            Direction::SendOnly,
                            Some(track_in.role.stream_id(track_in.user_id)),
                            None,
                            None,
                        );
                        track.state = TrackOutState::Negotiating(mid);
                    }
                }
                TrackOutState::ToStop(mid) => {
                    change.stop_media(mid);
                    track.state = TrackOutState::NegotiatingStop(mid);
                }
                _ => {}
            }
        }
        if !change.has_changes() {
            return false;
        }
        let Some((offer, pending)) = change.apply() else {
            return false;
        };
        let Some(mut channel) = self.cid.and_then(|id| self.rtc.channel(id)) else {
            return false;
        };
        let Ok(json) = serde_json::to_string(&offer) else {
            return false;
        };
        if channel.write(false, json.as_bytes()).is_err() {
            return false;
        }
        self.pending = Some(pending);
        true
    }

    /// Nachrichten über den DataChannel: Offers (mit `roles`), Answers und
    /// Schichtwünsche `{"type":"layer","mid":"…","rid":"h|m|l"}`.
    fn handle_channel_data(&mut self, data: ChannelData) -> Propagated {
        let Ok(message) = serde_json::from_slice::<serde_json::Value>(&data.data) else {
            return Propagated::Noop;
        };
        match message.get("type").and_then(serde_json::Value::as_str) {
            Some("offer") => {
                if let Some(roles) = message.get("roles").and_then(serde_json::Value::as_object) {
                    for (mid, role) in roles {
                        if let Some(role) = role.as_str().and_then(TrackRole::parse) {
                            self.roles.insert(Mid::from(mid.as_str()), role);
                        }
                    }
                }
                if let Ok(offer) = serde_json::from_value::<SdpOffer>(message) {
                    self.handle_offer(offer);
                }
                Propagated::Noop
            }
            Some("answer") => {
                if let Ok(answer) = serde_json::from_value::<SdpAnswer>(message) {
                    self.handle_answer(answer);
                }
                Propagated::Noop
            }
            Some("layer") => {
                let mid = message.get("mid").and_then(serde_json::Value::as_str);
                let rid = message
                    .get("rid")
                    .and_then(serde_json::Value::as_str)
                    .filter(|rid| LAYERS.contains(rid));
                match (mid, rid) {
                    (Some(mid), Some(rid)) => self.handle_layer_request(Mid::from(mid), rid),
                    _ => Propagated::Noop,
                }
            }
            _ => Propagated::Noop,
        }
    }

    /// Empfänger wählt die Schicht einer weitergeleiteten Spur (z. B. „l“ für
    /// eine kleine Kachel). Das Schlüsselbild der neuen Schicht wird sofort
    /// beim Absender angefordert.
    fn handle_layer_request(&mut self, mid: Mid, rid: &str) -> Propagated {
        let Some(track_out) = self
            .tracks_out
            .iter_mut()
            .find(|track| track.mid() == Some(mid))
        else {
            return Propagated::Noop;
        };
        let Some(changed) = track_out.layers.request(Rid::from(rid)) else {
            return Propagated::Noop;
        };
        let Some(track_in) = track_out.track_in.upgrade() else {
            return Propagated::Noop;
        };
        track_out.last_keyframe_request = Some(Instant::now());
        Propagated::KeyframeRequest(
            self.id,
            KeyframeRequest {
                mid,
                rid: Some(changed),
                kind: KeyframeRequestKind::Pli,
            },
            track_in.origin,
            track_in.mid,
        )
    }

    fn handle_offer(&mut self, offer: SdpOffer) {
        let Ok(answer) = self.rtc.sdp_api().accept_offer(offer) else {
            warn!(user_id = self.user_id, "SFU rejected renegotiation offer");
            return;
        };
        for track in &mut self.tracks_out {
            match track.state {
                TrackOutState::Negotiating(_) => track.state = TrackOutState::ToOpen,
                TrackOutState::NegotiatingStop(mid) => track.state = TrackOutState::ToStop(mid),
                _ => {}
            }
        }
        let Some(mut channel) = self.cid.and_then(|id| self.rtc.channel(id)) else {
            return;
        };
        if let Ok(json) = serde_json::to_string(&answer) {
            let _ = channel.write(false, json.as_bytes());
        }
    }

    fn handle_answer(&mut self, answer: SdpAnswer) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if self.rtc.sdp_api().accept_answer(pending, answer).is_err() {
            return;
        }
        for track in &mut self.tracks_out {
            if let TrackOutState::Negotiating(mid) = track.state {
                track.state = TrackOutState::Open(mid);
            }
        }
        self.tracks_out
            .retain(|track| !matches!(track.state, TrackOutState::NegotiatingStop(_)));
    }

    fn handle_track_open(&mut self, track_in: Weak<TrackIn>) {
        self.tracks_out.push(TrackOut {
            track_in,
            state: TrackOutState::ToOpen,
            layers: Layers::default(),
            last_keyframe_request: None,
        });
    }

    fn handle_media_data_out(&mut self, origin: ClientId, data: &MediaData) {
        let Some(track_out) = self.tracks_out.iter_mut().find(|out| {
            out.track_in
                .upgrade()
                .filter(|track| track.origin == origin && track.mid == data.mid)
                .is_some()
        }) else {
            return;
        };
        let Some(mid) = track_out.mid() else {
            return;
        };
        if let Some(rid) = data.rid {
            let now = Instant::now();
            let before = track_out.layers.active;
            let accepted = track_out.layers.accept(rid, data.is_keyframe(), now);
            if track_out.layers.active != before {
                debug!(
                    user_id = self.user_id,
                    from = ?before,
                    to = ?track_out.layers.active,
                    keyframe = data.is_keyframe(),
                    "SFU simulcast layer switch"
                );
            }
            if !accepted {
                // Auf ein Schlüsselbild der gewünschten Schicht warten; höchstens
                // eins pro Sekunde anfordern.
                if let Some(waiting) = track_out.layers.waiting
                    && track_out
                        .last_keyframe_request
                        .is_none_or(|at| now.duration_since(at) > Duration::from_secs(1))
                {
                    track_out.last_keyframe_request = Some(now);
                    debug!(user_id = self.user_id, rid = ?waiting, "SFU waits for keyframe");
                    self.keyframe_requests
                        .push((origin, data.mid, Some(waiting), mid));
                }
                return;
            }
        }
        let Some(writer) = self.rtc.writer(mid) else {
            return;
        };
        let Some(pt) = writer.match_params(data.params) else {
            return;
        };
        if writer
            .write(pt, data.network_time, data.time, data.data.clone())
            .is_err()
        {
            self.rtc.disconnect();
            return;
        }
        // Mindestens 1 µs: 0 heißt im Histogramm „leerer Platz“.
        let micros = data.network_time.elapsed().as_micros().max(1);
        self.notifier
            .forward
            .record_us(u64::try_from(micros).unwrap_or(u64::MAX));
    }

    /// Gesammelte Schlüsselbild-Wünsche als Weiterleitungen an die Absender.
    fn take_keyframe_requests(&mut self) -> Vec<Propagated> {
        self.keyframe_requests
            .drain(..)
            .map(|(origin, mid_in, rid, mid_out)| {
                Propagated::KeyframeRequest(
                    self.id,
                    KeyframeRequest {
                        mid: mid_out,
                        rid,
                        kind: KeyframeRequestKind::Pli,
                    },
                    origin,
                    mid_in,
                )
            })
            .collect()
    }

    fn handle_keyframe_request(&mut self, request: KeyframeRequest, mid_in: Mid) {
        if !self.tracks_in.iter().any(|track| track.id.mid == mid_in) {
            return;
        }
        let Some(mut writer) = self.rtc.writer(mid_in) else {
            return;
        };
        if let Err(err) = writer.request_keyframe(request.rid, request.kind) {
            debug!(?err, "SFU keyframe request failed");
        }
    }
}

#[derive(Debug)]
enum Propagated {
    Noop,
    Timeout(Instant),
    TrackOpen(ClientId, Weak<TrackIn>),
    MediaData(ClientId, Box<MediaData>),
    KeyframeRequest(ClientId, KeyframeRequest, ClientId, Mid),
}

impl Propagated {
    fn client_id(&self) -> Option<ClientId> {
        match self {
            Self::TrackOpen(client, _)
            | Self::MediaData(client, _)
            | Self::KeyframeRequest(client, _, _, _) => Some(*client),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(name: &str) -> Rid {
        Rid::from(name)
    }

    #[test]
    fn roles_choose_stream_ids_compatible_with_old_clients() {
        assert_eq!(TrackRole::Mic.stream_id(7), "u7");
        assert_eq!(TrackRole::Screen.stream_id(7), "u7");
        assert_eq!(TrackRole::ScreenAudio.stream_id(7), "s7");
        assert_eq!(TrackRole::Camera.stream_id(7), "c7");
        assert_eq!(TrackRole::parse("camera"), Some(TrackRole::Camera));
        assert_eq!(TrackRole::parse("bogus"), None);
        assert_eq!(TrackRole::default_for(MediaKind::Video), TrackRole::Screen);
        assert!(!TrackRole::Camera.fits(MediaKind::Audio));
    }

    #[test]
    fn layers_start_on_wanted_layer_and_switch_on_keyframe() {
        let now = Instant::now();
        let mut layers = Layers::default();
        // Startet mit „h“, ohne auf ein Schlüsselbild zu warten.
        assert!(layers.accept(rid("h"), false, now));
        assert!(!layers.accept(rid("l"), false, now));
        // Wunsch „l“: „h“ läuft weiter, bis „l“ ein Schlüsselbild liefert.
        assert_eq!(layers.request(rid("l")), Some(rid("l")));
        assert_eq!(layers.request(rid("l")), None);
        assert!(!layers.accept(rid("l"), false, now));
        assert_eq!(layers.waiting, Some(rid("l")));
        assert!(layers.accept(rid("h"), false, now));
        assert!(layers.accept(rid("l"), true, now));
        assert_eq!(layers.active, Some(rid("l")));
        assert!(!layers.accept(rid("h"), true, now));
        assert!(layers.accept(rid("l"), false, now));
    }

    #[test]
    fn layers_fall_back_when_the_wanted_one_stops() {
        let start = Instant::now();
        let mut layers = Layers::default();
        assert!(layers.accept(rid("h"), false, start));
        assert!(!layers.accept(rid("m"), false, start));
        // „h“ bleibt aus, „m“ kommt weiter: nach der Frist wird umgeschaltet,
        // auch ohne Schlüsselbild (sonst stünde das Bild).
        let later = start + LAYER_STALE + Duration::from_millis(10);
        assert!(layers.accept(rid("m"), false, later));
        assert_eq!(layers.active, Some(rid("m")));
        // „h“ kommt zurück: am nächsten Schlüsselbild wieder hoch.
        assert!(layers.accept(rid("m"), false, later));
        assert!(!layers.accept(rid("h"), false, later));
        assert!(layers.accept(rid("h"), true, later));
        assert_eq!(layers.active, Some(rid("h")));
    }

    fn user() -> UserPublic {
        UserPublic {
            id: 1,
            handle: "tester".into(),
            display_name: "Tester".into(),
            avatar_url: None,
            status: 0,
            bot: false,
        }
    }

    #[test]
    fn only_opus_and_constrained_baseline_h264() {
        let mut config = rtc_config();
        let codecs: Vec<_> = config
            .codec_config()
            .params()
            .iter()
            .map(|p| p.spec().codec)
            .collect();
        assert!(codecs.contains(&str0m::format::Codec::Opus));
        assert!(codecs.contains(&str0m::format::Codec::H264));
        assert!(!codecs.contains(&str0m::format::Codec::Vp8));
        assert!(!codecs.contains(&str0m::format::Codec::Vp9));
    }

    #[test]
    fn stream_ids_name_the_user() {
        assert_eq!(
            TrackRole::Mic.stream_id(247126938131369984),
            "u247126938131369984"
        );
    }

    #[test]
    fn set_state_needs_a_participant() {
        assert!(sfu().set_state(42, 1, true, false, false, false).is_none());
    }

    fn sfu() -> Sfu {
        Sfu::new(SfuConfig {
            enabled: true,
            public_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port_start: 40_100,
            port_end: 40_100,
            max_users: 15,
        })
    }

    #[test]
    fn invalid_offer_is_rejected_before_binding_a_port() {
        let result = sfu().join(7, &user(), "not an offer");
        assert!(matches!(result, Err(VoiceError::InvalidOffer(_))));
    }

    #[test]
    fn disabled_sfu_rejects_join() {
        let sfu = Sfu::new(SfuConfig {
            enabled: false,
            ..SfuConfig::from_env()
        });
        assert!(matches!(
            sfu.join(7, &user(), "{}"),
            Err(VoiceError::Disabled)
        ));
    }

    #[test]
    fn empty_room_has_no_participants() {
        assert!(sfu().participants(42).is_empty());
    }
}
