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

use nova_common::protocol::UserPublic;
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

#[derive(Debug, Clone, Serialize)]
pub struct Participant {
    pub user_id: i64,
    pub handle: String,
    pub display_name: String,
    pub muted: bool,
    pub screen: bool,
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
        Self {
            inner: Arc::new(Inner {
                cfg,
                rooms: Mutex::new(HashMap::new()),
            }),
        }
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
        };
        room.join(participant, offer, self.inner.cfg.public_ip)
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
        let room = Room::new(channel_id, &self.inner.cfg, self.inner.cfg.max_users)?;
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
    fn new(channel_id: i64, cfg: &SfuConfig, max_users: usize) -> Result<Arc<Self>, VoiceError> {
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
                    max_users,
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

        let mut rtc = Rtc::builder().set_ice_lite(true).build(Instant::now());
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
    _max_users: usize,
) {
    let mut clients: Vec<Client> = Vec::new();
    let mut to_propagate: VecDeque<Propagated> = VecDeque::new();
    let mut buf = vec![0; 2000];
    let mut idle_since = Instant::now();

    loop {
        loop {
            match rx.try_recv() {
                Ok(RoomCommand::Add { user_id, rtc }) => {
                    let mut client = Client::new(*rtc, user_id, Arc::clone(&participants));
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

        let connected_before: Vec<i64> = clients.iter().map(|client| client.user_id).collect();
        clients.retain(|client| client.rtc.is_alive());
        if let Ok(mut participants) = participants.lock() {
            for user_id in connected_before {
                if !clients.iter().any(|client| client.user_id == user_id)
                    && participants.remove(&user_id).is_some()
                {
                    debug!(user_id, "SFU participant removed after disconnect");
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

        if let Some(propagated) = to_propagate.pop_front() {
            propagate(&propagated, &mut clients);
            continue;
        }

        let duration = (timeout - Instant::now()).max(Duration::from_millis(1));
        socket.set_read_timeout(Some(duration)).ok();

        socket.set_nonblocking(true).ok();
        for _ in 0..256 {
            let Some(input) = read_socket_input(&socket, local_addr, &mut buf) else {
                break;
            };
            if let Some(client) = clients.iter_mut().find(|client| client.accepts(&input)) {
                client.handle_input(input);
            }
        }
        socket.set_nonblocking(false).ok();

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
    chosen_rid: Option<Rid>,
    participants: Arc<Mutex<HashMap<i64, ParticipantState>>>,
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
    mid: Mid,
    kind: MediaKind,
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
            chosen_rid: None,
            participants,
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
        if kind == MediaKind::Video
            && let Ok(mut participants) = self.participants.lock()
            && let Some(participant) = participants.get_mut(&self.user_id)
        {
            participant.public.screen = true;
        }
        let entry = TrackInEntry {
            id: Arc::new(TrackIn {
                origin: self.id,
                mid,
                kind,
            }),
            last_keyframe_request: None,
        };
        let weak = Arc::downgrade(&entry.id);
        self.tracks_in.push(entry);
        Propagated::TrackOpen(self.id, weak)
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
        request.rid = self.chosen_rid;
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
                        let stream_id = track_in.origin.to_string();
                        let mid = change.add_media(
                            track_in.kind,
                            Direction::SendOnly,
                            Some(stream_id),
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

    fn handle_channel_data(&mut self, data: ChannelData) -> Propagated {
        if let Ok(offer) = serde_json::from_slice::<SdpOffer>(&data.data) {
            self.handle_offer(offer);
        } else if let Ok(answer) = serde_json::from_slice::<SdpAnswer>(&data.data) {
            self.handle_answer(answer);
        }
        Propagated::Noop
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
        });
    }

    fn handle_media_data_out(&mut self, origin: ClientId, data: &MediaData) {
        let Some(mid) = self
            .tracks_out
            .iter()
            .find(|out| {
                out.track_in
                    .upgrade()
                    .filter(|track| track.origin == origin && track.mid == data.mid)
                    .is_some()
            })
            .and_then(TrackOut::mid)
        else {
            return;
        };
        if data.rid.is_some() && data.rid != Some("h".into()) {
            // Simulcast: aktuell wird nur die hohe Schicht weitergeleitet.
            return;
        }
        if self.chosen_rid != data.rid {
            self.chosen_rid = data.rid;
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
        }
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

    fn user() -> UserPublic {
        UserPublic {
            id: 1,
            handle: "tester".into(),
            display_name: "Tester".into(),
            avatar_url: None,
            status: 0,
        }
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
