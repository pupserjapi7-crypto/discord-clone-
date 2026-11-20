//! Nova Desktop — nativer Client ohne WebView.
//!
//! iced rendert direkt über wgpu (DirectX 12/Vulkan/Metal), Netzwerk läuft über
//! tokio. Kein Chromium, kein WebView2, kein JavaScript.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod attach;
mod capture;
mod gateway;
mod icon;
mod prefs;
mod style;
mod update;
mod video;
mod voice;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc::UnboundedSender;
use iced::keyboard;
use iced::widget::{
    button, column, container, hover, mouse_area, opaque, row, scrollable, space, stack, text,
    text_input, toggler, tooltip,
};
use iced::{Color, Element, Fill, Length, Subscription, Task, Theme};
use nova_common::protocol::{Event, MessageView};

use api::{Api, Channel, GuildDetail, User, perm};
use gateway::{Command, GatewayEvent, Status};
use icon::{Icon, icon, icon_hover};

const DEFAULT_SERVER: &str = "https://85-215-175-202.sslip.io";

static START: OnceLock<Instant> = OnceLock::new();
static FIRST_FRAME: OnceLock<()> = OnceLock::new();

fn main() -> iced::Result {
    START.get_or_init(Instant::now);
    update::cleanup();
    iced::application(App::boot, App::update, App::view)
        .font(
            include_bytes!("../../ui/assets/fonts/AtkinsonHyperlegibleNext-Regular.ttf").as_slice(),
        )
        .font(include_bytes!("../../ui/assets/fonts/AtkinsonHyperlegibleNext-Bold.ttf").as_slice())
        .default_font(style::FONT)
        .title("Nova")
        .theme(App::theme)
        .style(App::style)
        .subscription(App::subscription)
        .window_size((1280.0, 800.0))
        .antialiasing(true)
        .run()
}

// ---------------------------------------------------------------------------
// Zustand
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Booting,
    Login,
    Chat,
}

#[derive(Debug, Clone)]
enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(update::Available),
    Installing(String),
    Failed(String),
}

/// Schnellreaktionen unter jeder Nachricht.
const QUICK_REACTIONS: [&str; 5] = ["👍", "❤️", "😂", "🎉", "👀"];

/// Wie lange „… schreibt“ ohne neues Signal sichtbar bleibt.
const TYPING_TTL: Duration = Duration::from_secs(6);

#[derive(Debug, Clone)]
struct ChatMessage {
    id: Option<i64>,
    nonce: Option<u64>,
    author_id: i64,
    author: String,
    content: String,
    time: String,
    minute: i64,
    edited: bool,
    pinned: bool,
    reactions: Vec<api::Reaction>,
    attachments: Vec<api::Attachment>,
}

impl ChatMessage {
    fn from_rest(message: api::Message) -> Self {
        let parsed = chrono::DateTime::parse_from_rfc3339(&message.created_at)
            .map(|time| time.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());
        let mut built = Self::build(
            Some(message.id),
            message.author.id,
            message.author.display_name,
            message.content,
            parsed,
        );
        built.edited = message.edited_at.is_some();
        built.pinned = message.pinned;
        built.reactions = message.reactions;
        built.attachments = message.attachments;
        built
    }

    fn from_gateway(message: MessageView) -> Self {
        let mut built = Self::build(
            Some(message.id),
            message.author.id,
            message.author.display_name,
            message.content,
            message.created_at,
        );
        built.edited = message.edited_at.is_some();
        built.pinned = message.pinned;
        built.reactions = message
            .reactions
            .into_iter()
            .map(|r| api::Reaction {
                emoji: r.emoji,
                count: r.count,
                me: r.me,
            })
            .collect();
        built.attachments = message.attachments.into_iter().map(Into::into).collect();
        built
    }

    fn react(&mut self, emoji: &str, delta: i64, mine: bool) {
        match self.reactions.iter_mut().find(|r| r.emoji == emoji) {
            Some(reaction) => {
                reaction.count = (reaction.count + delta).max(0);
                if mine {
                    reaction.me = delta > 0;
                }
            }
            None if delta > 0 => self.reactions.push(api::Reaction {
                emoji: emoji.to_owned(),
                count: delta,
                me: mine,
            }),
            None => {}
        }
        self.reactions.retain(|r| r.count > 0);
    }

    fn build(
        id: Option<i64>,
        author_id: i64,
        author: String,
        content: String,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        let local = at.with_timezone(&chrono::Local);
        Self {
            id,
            nonce: None,
            author_id,
            author,
            content,
            time: local.format("%H:%M").to_string(),
            minute: at.timestamp() / 60,
            edited: false,
            pinned: false,
            reactions: Vec::new(),
            attachments: Vec::new(),
        }
    }
}

/// Offener Dialog über dem Chat.
#[derive(Debug, Clone)]
enum Modal {
    CreateGuild,
    JoinGuild,
    CreateChannel {
        voice: bool,
    },
    /// Einladung; `None`, solange der Code geladen wird.
    Invite(Option<String>),
    Thread {
        message_id: i64,
    },
    Member(i64),
    Pins(Option<Vec<api::Message>>),
    Settings,
}

/// Moderationsaktion, die erst nach Bestätigung ausgeführt wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModAction {
    Kick,
    Ban,
}

/// Ergebnis eines Dialogs, das die App danach umsetzt.
#[derive(Debug, Clone)]
enum Outcome {
    /// Serverliste neu laden und diesen Server öffnen.
    Guild(Option<i64>),
    /// Server neu laden und diesen Kanal öffnen.
    Channel(i64),
    Invite(String),
    Profile(User),
    /// Server neu laden, Dialog schließen.
    Reload,
}

struct App {
    api: Api,
    epoch: u64,
    screen: Screen,

    server: String,
    handle: String,
    password: String,
    register: bool,
    busy: bool,
    auth_error: Option<String>,

    me: Option<User>,
    guilds: Vec<api::Guild>,
    detail: Option<GuildDetail>,
    channel: Option<i64>,
    /// Verlauf je Kanal: Kanalwechsel zeigen sofort den Cache, der Server
    /// liefert im Hintergrund nach.
    history: HashMap<i64, Vec<ChatMessage>>,
    draft: String,
    status: Status,
    error: Option<String>,
    gateway: Option<UnboundedSender<Command>>,
    show_members: bool,
    next_nonce: u64,
    update: UpdateState,

    /// Laufende Sprachverbindung (Fallenlassen = Verlassen).
    voice: Option<Arc<voice::VoiceHandle>>,
    voice_joining: Option<i64>,
    voice_muted: bool,
    voice_deafened: bool,
    voice_status: voice::VoiceStatus,
    /// Eigener Stream angefordert (Knopf); der Status sagt, ob er läuft.
    sharing: bool,
    /// Wer im eigenen Sprachkanal gerade sichtbar streamt.
    streamers: Vec<i64>,
    /// Stream auf der Bühne (`None`: Kachelansicht).
    stage: Option<i64>,
    /// Bühne im ganzen Fenster.
    theater: bool,
    /// Wer in welchem Sprachkanal ist.
    voice_rooms: HashMap<i64, Vec<api::VoiceParticipant>>,
    /// Ungewollt getrennt: so oft wird automatisch neu verbunden.
    voice_retries: u8,

    modal: Option<Modal>,
    modal_input: String,
    modal_busy: bool,
    modal_error: Option<String>,
    confirm: Option<ModAction>,
    editing: Option<(i64, String)>,
    /// Kanäle mit ungelesenen Nachrichten.
    unread: HashSet<i64>,
    /// Wer gerade wo tippt (Kanal → Nutzer → Name, letztes Signal).
    typing: HashMap<i64, HashMap<i64, (String, Instant)>>,
    last_typing_sent: Option<Instant>,
    /// Kanäle, für die es keine älteren Nachrichten mehr gibt.
    history_complete: HashSet<i64>,

    /// Direktnachrichten statt eines Servers anzeigen.
    home: bool,
    /// Eigene DM-Kanäle, zuletzt aktive zuerst.
    dms: Vec<api::DmChannel>,
    /// Offener DM-Kanal.
    dm: Option<i64>,
    /// Eingabe „Benutzername“ zum Starten einer Unterhaltung.
    dm_input: String,
    dm_busy: bool,
    dm_error: Option<String>,
    dm_unread: HashSet<i64>,

    /// Wartende Anhänge je Ziel (Kanal oder DM).
    pending: HashMap<api::UploadTarget, Vec<attach::PendingFile>>,
    /// Vorschaubilder je Anhang-ID.
    previews: HashMap<i64, attach::Preview>,

    prefs: prefs::Prefs,
    devices: (Vec<String>, Vec<String>),
    ptt_held: bool,
    capturing_key: bool,
    /// Bildzähler für vollständiges Neuzeichnen, siehe [`App::style`].
    frame: std::cell::Cell<u32>,
}

#[derive(Debug, Clone)]
enum Message {
    ServerChanged(String),
    HandleChanged(String),
    PasswordChanged(String),
    ToggleRegister,
    Submit,
    Authenticated(Result<User, String>),
    Booted(Result<User, String>),

    GuildsLoaded(Result<Vec<api::Guild>, String>),
    OpenGuild(i64),
    GuildLoaded(Result<GuildDetail, String>),
    OpenChannel(i64),
    HistoryLoaded(i64, Result<Vec<api::Message>, String>),

    DraftChanged(String),
    Send,
    Gateway(GatewayEvent),
    ToggleMembers,
    DismissError,
    Logout,
    LoggedOut,

    CheckUpdate,
    UpdateChecked(Result<Option<update::Available>, String>),
    InstallUpdate,
    UpdateFailed(String),
    DismissUpdate,

    JoinVoice(i64),
    VoiceJoined(i64, Result<Arc<voice::VoiceHandle>, String>),
    LeaveVoice,
    ToggleMute,
    ToggleDeafen,
    ToggleShare,
    Watch(Option<i64>),
    ToggleTheater,
    VoiceTick,
    VoiceRoom(i64, Result<Vec<api::VoiceParticipant>, String>),
    RetryVoice(i64),

    OpenModal(Modal),
    CloseModal,
    ModalInput(String),
    ModalSubmit,
    ModalDone(Result<Outcome, String>),
    CopyInvite(String),
    PinsLoaded(Result<Vec<api::Message>, String>),

    StartEdit(i64, String),
    EditChanged(String),
    SaveEdit,
    CancelEdit,
    Delete(i64),
    React(i64, String),
    TogglePin(i64, bool),
    ArchiveThread(i64),
    LoadOlder,
    OlderLoaded(i64, Result<Vec<api::Message>, String>),

    OpenHome,
    SelectGuild(i64),
    DmsLoaded(Result<Vec<api::DmChannel>, String>),
    OpenDm(i64),
    DmInput(String),
    StartDm,
    /// Unterhaltung mit diesem Benutzernamen öffnen (Eingabe oder Profilkarte).
    DmWith(String),
    DmOpened(Result<api::DmChannel, String>),
    /// (DM-Kanal, Nonce des Platzhalters, Antwort)
    DmSent(i64, u64, Result<api::Message, String>),

    PickFiles,
    /// Gewählte Dateien und Meldungen zu übersprungenen.
    FilesPicked(api::UploadTarget, Vec<attach::Picked>, Vec<String>),
    Uploaded(api::UploadTarget, u64, Result<api::Attachment, String>),
    RemovePending(api::UploadTarget, u64),
    PreviewLoaded(i64, Result<attach::Decoded, String>),
    SaveAttachment(api::Attachment),
    Saved(Result<Option<String>, String>),

    AskConfirm(Option<ModAction>),
    Moderate(i64, ModAction),
    Timeout(i64, i64),
    ToggleRole(i64, i64, bool),
    ActionDone(Result<(), String>),

    SetInput(String),
    SetOutput(String),
    SetPushToTalk(bool),
    CaptureKey,
    DevicesLoaded((Vec<String>, Vec<String>)),
    Key(keyboard::Event),
    Tick,
    Ignore,
}

/// Auswahl „Systemstandard“ in den Gerätelisten.
const DEFAULT_DEVICE: &str = "Systemstandard";

impl App {
    fn boot() -> (Self, Task<Message>) {
        let server = std::env::var("NOVA_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.to_owned());
        let api = Api::new(&server);
        let saved = api.has_saved_session();
        let app = Self {
            api: api.clone(),
            epoch: 0,
            screen: if saved {
                Screen::Booting
            } else {
                Screen::Login
            },
            server,
            handle: String::new(),
            password: String::new(),
            register: false,
            busy: false,
            auth_error: None,
            me: None,
            guilds: Vec::new(),
            detail: None,
            channel: None,
            history: HashMap::new(),
            draft: String::new(),
            status: Status::Offline,
            error: None,
            gateway: None,
            show_members: true,
            next_nonce: 1,
            update: UpdateState::Checking,
            voice: None,
            voice_joining: None,
            voice_muted: false,
            voice_deafened: false,
            voice_status: voice::VoiceStatus::default(),
            sharing: false,
            streamers: Vec::new(),
            stage: None,
            theater: false,
            voice_rooms: HashMap::new(),
            voice_retries: 0,
            modal: None,
            modal_input: String::new(),
            modal_busy: false,
            modal_error: None,
            confirm: None,
            editing: None,
            unread: HashSet::new(),
            typing: HashMap::new(),
            last_typing_sent: None,
            history_complete: HashSet::new(),
            home: false,
            dms: Vec::new(),
            dm: None,
            dm_input: String::new(),
            dm_busy: false,
            dm_error: None,
            dm_unread: HashSet::new(),
            pending: HashMap::new(),
            previews: HashMap::new(),
            prefs: prefs::Prefs::load(),
            devices: (Vec::new(), Vec::new()),
            ptt_held: false,
            capturing_key: false,
            frame: std::cell::Cell::new(0),
        };
        voice::set_devices(app.prefs.input.clone(), app.prefs.output.clone());
        let session = if saved {
            Task::perform(async move { api.refresh().await }, Message::Booted)
        } else {
            Task::none()
        };
        // Beim Start still im Hintergrund nach einer neuen Version schauen.
        let check = Task::perform(update::check(), Message::UpdateChecked);
        (app, Task::batch([session, check]))
    }

    fn theme(&self) -> Theme {
        style::theme()
    }

    /// Workaround für die Teil-Neuzeichnung von iced/tiny-skia 0.14: Bei
    /// umgebauten Listen und Overlays (Aufklappmenüs, Dialoge) gelten Teile als
    /// „unverändert“ und bleiben als Reste stehen. Ein von Bild zu Bild minimal
    /// anderer Hintergrund (unsichtbar, die Container decken ihn ab) erzwingt
    /// ein volles Neuzeichnen. Gezeichnet wird ohnehin nur bei Ereignissen;
    /// ein volles Bild kostet auf der CPU wenige Millisekunden.
    fn style(&self, theme: &Theme) -> iced::theme::Style {
        let frame = self.frame.get().wrapping_add(1);
        self.frame.set(frame);
        let mut background = theme.palette().background;
        // Jeder Aufruf ein anderer Wert, egal wie oft pro Bild abgefragt wird.
        background.b = (background.b + (frame % 64) as f32 * 0.0001).min(1.0);
        iced::theme::Style {
            background_color: background,
            text_color: theme.palette().text,
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.screen != Screen::Chat {
            return Subscription::none();
        }
        let gateway = gateway::subscription(gateway::Params {
            epoch: self.epoch,
            api: self.api.clone(),
        })
        .map(Message::Gateway);
        let mut subs = vec![gateway, keyboard::listen().map(Message::Key)];
        if self.voice.is_some() {
            // Sprecher-Ring und Status fünfmal pro Sekunde; läuft ein Stream,
            // so oft wie Bilder kommen (30 fps).
            let every = if self.streamers.is_empty() {
                Duration::from_millis(200)
            } else {
                Duration::from_millis(33)
            };
            subs.push(iced::time::every(every).map(|_| Message::VoiceTick));
        }
        if self.typing.values().any(|people| !people.is_empty()) {
            // Nur solange jemand tippt: abgelaufene Hinweise entfernen.
            subs.push(iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick));
        }
        Subscription::batch(subs)
    }

    fn can(&self, bit: u64) -> bool {
        self.detail.as_ref().is_some_and(|d| d.can(bit))
    }

    fn me_id(&self) -> i64 {
        self.me.as_ref().map(|m| m.id).unwrap_or_default()
    }

    fn reload_guild(&self) -> Task<Message> {
        match self.detail.as_ref().map(|d| d.guild.id) {
            Some(id) => self.update_task(Message::OpenGuild(id)),
            None => Task::none(),
        }
    }

    fn update_task(&self, message: Message) -> Task<Message> {
        Task::done(message)
    }

    fn send_gateway(&self, command: Command) {
        if let Some(gateway) = &self.gateway {
            let _ = gateway.unbounded_send(command);
        }
    }

    /// Lesestand des offenen Kanals auf die neueste Nachricht setzen.
    fn mark_read(&mut self) {
        let Some(channel_id) = self.channel else {
            return;
        };
        self.unread.remove(&channel_id);
        if let Some(message_id) = self
            .history
            .get(&channel_id)
            .and_then(|list| list.iter().rev().find_map(|m| m.id))
        {
            self.send_gateway(Command::ReadAck {
                channel_id,
                message_id,
            });
        }
    }

    /// Wirksame Stummschaltung: Knopf oder Push-to-Talk-Taste nicht gedrückt.
    fn effective_mute(&self) -> bool {
        self.voice_muted || (self.prefs.push_to_talk && !self.ptt_held)
    }

    fn find_message(&mut self, channel_id: Option<i64>, id: i64) -> Option<&mut ChatMessage> {
        match channel_id {
            Some(channel) => self
                .history
                .get_mut(&channel)?
                .iter_mut()
                .find(|m| m.id == Some(id)),
            None => self
                .history
                .values_mut()
                .flat_map(|list| list.iter_mut())
                .find(|m| m.id == Some(id)),
        }
    }

    fn messages(&self) -> &[ChatMessage] {
        (if self.home { self.dm } else { self.channel })
            .and_then(|id| self.history.get(&id))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// DM-Kanal in der Liste nach oben schieben (neueste Aktivität zuerst).
    fn bump_dm(&mut self, id: i64) {
        if let Some(pos) = self.dms.iter().position(|d| d.id == id)
            && pos > 0
        {
            let dm = self.dms.remove(pos);
            self.dms.insert(0, dm);
        }
    }

    /// Direktnachricht per REST senden; bis zur Antwort steht ein Platzhalter da.
    fn send_dm(&mut self) -> Task<Message> {
        let content = self.draft.trim().to_owned();
        let (Some(dm), Some(me)) = (self.dm, self.me.clone()) else {
            return Task::none();
        };
        let Some(attachments) = self.take_ready(api::UploadTarget::Dm(dm)) else {
            return Task::none();
        };
        if content.is_empty() && attachments.is_empty() {
            return Task::none();
        }
        let nonce = self.next_nonce;
        self.next_nonce += 1;
        let mut local = ChatMessage::build(
            None,
            me.id,
            me.display_name,
            content.clone(),
            chrono::Utc::now(),
        );
        local.nonce = Some(nonce);
        let ids: Vec<i64> = attachments.iter().map(|a| a.id).collect();
        local.attachments = attachments;
        self.history.entry(dm).or_default().push(local);
        self.draft.clear();
        let api = self.api.clone();
        Task::perform(
            async move { api.send_dm(dm, &content, &ids).await },
            move |result| Message::DmSent(dm, nonce, result),
        )
    }

    /// Wohin ein Upload gerade ginge.
    fn upload_target(&self) -> Option<api::UploadTarget> {
        if self.home {
            self.dm.map(api::UploadTarget::Dm)
        } else {
            self.channel.map(api::UploadTarget::Channel)
        }
    }

    /// Fertige Uploads des Ziels für das Senden herausnehmen. `None`, solange
    /// noch etwas hochlädt (dann bleibt alles liegen).
    fn take_ready(&mut self, target: api::UploadTarget) -> Option<Vec<api::Attachment>> {
        let busy = self.pending.get(&target).is_some_and(|list| {
            list.iter()
                .any(|file| matches!(file.state, attach::PendingState::Uploading))
        });
        if busy {
            self.error = Some("Einen Moment – die Dateien werden noch hochgeladen.".into());
            return None;
        }
        Some(
            self.pending
                .remove(&target)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|file| match file.state {
                    attach::PendingState::Done(attachment) => Some(attachment),
                    _ => None,
                })
                .collect(),
        )
    }

    /// Fehlende Vorschaubilder der sichtbaren Nachrichten nachladen; alte
    /// Vorschauen fallen weg, wenn der Speicher voll wird.
    fn load_previews(&mut self) -> Task<Message> {
        let wanted: Vec<i64> = self
            .messages()
            .iter()
            .flat_map(|message| message.attachments.iter())
            .chain(
                self.upload_target()
                    .and_then(|target| self.pending.get(&target))
                    .into_iter()
                    .flatten()
                    .filter_map(|file| match &file.state {
                        attach::PendingState::Done(attachment) => Some(attachment),
                        _ => None,
                    }),
            )
            .filter(|attachment| attachment.is_image())
            .map(|attachment| attachment.id)
            .collect();
        if self.previews.len() > 96 {
            self.previews.retain(|id, _| wanted.contains(id));
        }
        let mut tasks = Vec::new();
        for id in wanted {
            if self.previews.contains_key(&id) {
                continue;
            }
            self.previews.insert(id, attach::Preview::Loading);
            let api = self.api.clone();
            tasks.push(Task::perform(
                attach::load_preview(api, id),
                move |result| Message::PreviewLoaded(id, result),
            ));
        }
        Task::batch(tasks)
    }

    fn current_channel(&self) -> Option<&Channel> {
        let id = self.channel?;
        self.detail.as_ref()?.channels.iter().find(|c| c.id == id)
    }

    fn enter_chat(&mut self, user: User) -> Task<Message> {
        self.me = Some(user);
        self.screen = Screen::Chat;
        self.epoch += 1;
        self.busy = false;
        self.password.clear();
        if let Some(start) = START.get() {
            eprintln!("[nova] angemeldet nach {} ms", start.elapsed().as_millis());
        }
        let api = self.api.clone();
        let guilds = Task::perform(async move { api.guilds().await }, Message::GuildsLoaded);
        // Nur Debug-Builds: `NOVA_DEV_DM=<benutzername>` öffnet die Unterhaltung.
        match std::env::var("NOVA_DEV_DM") {
            Ok(handle) if cfg!(debug_assertions) => {
                Task::batch([guilds, Task::done(Message::DmWith(handle))])
            }
            _ => guilds,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // Nach Nachrichten, die den sichtbaren Verlauf ändern können, fehlende
        // Vorschaubilder holen (billig: nur ein Blick über die Liste).
        let previews = !matches!(
            message,
            Message::Tick | Message::VoiceTick | Message::Key(_) | Message::DraftChanged(_)
        );
        let task = self.update_inner(message);
        if previews && self.screen == Screen::Chat {
            Task::batch([task, self.load_previews()])
        } else {
            task
        }
    }

    fn update_inner(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ServerChanged(value) => self.server = value,
            Message::HandleChanged(value) => self.handle = value,
            Message::PasswordChanged(value) => self.password = value,
            Message::ToggleRegister => {
                self.register = !self.register;
                self.auth_error = None;
            }
            Message::Submit => {
                let handle = self.handle.trim().to_owned();
                // Der Server verlangt bei neuen Konten mindestens 10 Zeichen
                // (`validate_password`); beim Anmelden reicht ein Passwort.
                let min_password = if self.register { 10 } else { 1 };
                if handle.len() < 3 || self.password.chars().count() < min_password {
                    self.auth_error = Some(if self.register {
                        "Der Benutzername braucht mindestens 3 Zeichen, das Passwort mindestens 10."
                            .into()
                    } else {
                        "Gib deinen Benutzernamen und dein Passwort ein.".into()
                    });
                    return Task::none();
                }
                if self.api.base() != self.server.trim().trim_end_matches('/') {
                    self.api = Api::new(self.server.trim());
                }
                self.busy = true;
                self.auth_error = None;
                let api = self.api.clone();
                let password = self.password.clone();
                let register = self.register;
                return Task::perform(
                    async move { api.login(&handle, &password, register).await },
                    Message::Authenticated,
                );
            }
            Message::Authenticated(Ok(user)) | Message::Booted(Ok(user)) => {
                return self.enter_chat(user);
            }
            Message::Authenticated(Err(err)) => {
                self.busy = false;
                self.auth_error = Some(err);
            }
            Message::Booted(Err(_)) => self.screen = Screen::Login,

            Message::GuildsLoaded(Ok(list)) => {
                let target = self
                    .detail
                    .as_ref()
                    .map(|d| d.guild.id)
                    .filter(|id| list.iter().any(|g| g.id == *id))
                    .or_else(|| list.first().map(|g| g.id));
                self.guilds = list;
                if let Some(id) = target {
                    return self.update(Message::OpenGuild(id));
                }
            }
            Message::OpenGuild(id) => {
                let api = self.api.clone();
                return Task::perform(async move { api.guild(id).await }, Message::GuildLoaded);
            }
            Message::GuildLoaded(Ok(detail)) => {
                let same_guild = self.detail.as_ref().map(|d| d.guild.id) == Some(detail.guild.id);
                let keep = self.channel.filter(|id| {
                    detail
                        .channels
                        .iter()
                        .any(|c| c.id == *id && !(c.kind == 2 && c.archived))
                });
                let target = keep
                    .or_else(|| detail.channels.iter().find(|c| c.kind == 0).map(|c| c.id))
                    .or_else(|| detail.channels.first().map(|c| c.id));
                if !same_guild {
                    self.unread.clear();
                }
                for entry in &detail.unread {
                    if entry.last_message_id.unwrap_or_default()
                        > entry.last_read_id.unwrap_or_default()
                    {
                        self.unread.insert(entry.channel_id);
                    }
                }
                self.detail = Some(detail);
                // Bei Neuladen desselben Servers (Rollen, Threads …) nicht den
                // Verlauf neu holen, wenn der Kanal bleibt.
                if same_guild && let Some(id) = keep {
                    self.unread.remove(&id);
                    return Task::none();
                }
                if let Some(id) = target {
                    let open = self.update(Message::OpenChannel(id));
                    return Task::batch([
                        open,
                        self.dev_modal(),
                        self.dev_voice(),
                        self.dev_attach(),
                    ]);
                }
                self.channel = None;
            }
            Message::OpenChannel(id) => {
                self.channel = Some(id);
                self.editing = None;
                self.unread.remove(&id);
                if let Some(gateway) = &self.gateway {
                    let _ = gateway.unbounded_send(Command::Subscribe(id));
                }
                let is_text = self.current_channel().map(|c| c.kind != 1).unwrap_or(false);
                if is_text {
                    let api = self.api.clone();
                    return Task::perform(async move { api.messages(id).await }, move |result| {
                        Message::HistoryLoaded(id, result)
                    });
                }
                return self.load_room(id);
            }
            Message::HistoryLoaded(id, Ok(list)) => {
                let mut fresh: Vec<ChatMessage> =
                    list.into_iter().map(ChatMessage::from_rest).collect();
                // Noch nicht bestätigte eigene Nachrichten behalten.
                if fresh.len() < 50 {
                    self.history_complete.insert(id);
                } else {
                    self.history_complete.remove(&id);
                }
                if let Some(old) = self.history.get(&id) {
                    fresh.extend(old.iter().filter(|m| m.id.is_none()).cloned());
                }
                self.history.insert(id, fresh);
                if self.channel == Some(id) {
                    self.mark_read();
                }
            }
            Message::GuildsLoaded(Err(err))
            | Message::GuildLoaded(Err(err))
            | Message::HistoryLoaded(_, Err(err))
            | Message::OlderLoaded(_, Err(err))
            | Message::ActionDone(Err(err)) => self.error = Some(err),

            Message::DraftChanged(value) => {
                self.draft = value;
                // Höchstens alle 4 s „tippt“ senden.
                if !self.home
                    && let Some(channel_id) = self.channel
                    && !self.draft.is_empty()
                    && self
                        .last_typing_sent
                        .is_none_or(|at| at.elapsed() > Duration::from_secs(4))
                {
                    self.last_typing_sent = Some(Instant::now());
                    self.send_gateway(Command::Typing(channel_id));
                }
            }
            Message::Send if self.home => return self.send_dm(),
            Message::Send => {
                let content = self.draft.trim().to_owned();
                let (Some(channel_id), Some(me)) = (self.channel, self.me.clone()) else {
                    return Task::none();
                };
                let target = api::UploadTarget::Channel(channel_id);
                let has_files = self
                    .pending
                    .get(&target)
                    .is_some_and(|list| !list.is_empty());
                if content.is_empty() && !has_files {
                    return Task::none();
                }
                let Some(gateway) = self
                    .gateway
                    .clone()
                    .filter(|_| self.status == Status::Online)
                else {
                    self.error = Some("Gateway ist nicht verbunden.".into());
                    return Task::none();
                };
                let Some(attachments) = self.take_ready(target) else {
                    return Task::none();
                };
                if content.is_empty() && attachments.is_empty() {
                    return Task::none();
                }
                let nonce = self.next_nonce;
                self.next_nonce += 1;
                let mut local = ChatMessage::build(
                    None,
                    me.id,
                    me.display_name,
                    content.clone(),
                    chrono::Utc::now(),
                );
                local.nonce = Some(nonce);
                let attachment_ids = attachments.iter().map(|a| a.id).collect();
                local.attachments = attachments;
                self.history.entry(channel_id).or_default().push(local);
                let _ = gateway.unbounded_send(Command::Send {
                    nonce,
                    channel_id,
                    content,
                    attachment_ids,
                });
                self.draft.clear();
                self.last_typing_sent = None;
            }
            Message::Gateway(event) => return self.on_gateway(event),
            Message::ToggleMembers => self.show_members = !self.show_members,
            Message::DismissError => self.error = None,
            Message::Logout => {
                let api = self.api.clone();
                return Task::perform(async move { api.logout().await }, |()| Message::LoggedOut);
            }
            Message::LoggedOut => {
                let _ = self.leave_voice();
                self.screen = Screen::Login;
                self.gateway = None;
                self.me = None;
                self.detail = None;
                self.guilds.clear();
                self.history.clear();
                self.channel = None;
                self.home = false;
                self.dms.clear();
                self.dm = None;
                self.dm_input.clear();
                self.dm_error = None;
                self.dm_unread.clear();
                self.modal = None;
                self.editing = None;
                self.unread.clear();
                self.typing.clear();
                self.history_complete.clear();
                self.pending.clear();
                self.previews.clear();
            }

            Message::CheckUpdate => {
                self.update = UpdateState::Checking;
                return Task::perform(update::check(), Message::UpdateChecked);
            }
            Message::UpdateChecked(result) => {
                self.update = match result {
                    Ok(Some(available)) => UpdateState::Available(available),
                    Ok(None) => UpdateState::UpToDate,
                    Err(err) => UpdateState::Failed(err),
                };
            }
            Message::InstallUpdate => {
                if let UpdateState::Available(available) = self.update.clone() {
                    self.update = UpdateState::Installing(available.version.clone());
                    // Bei Erfolg startet die neue Version und dieser Prozess endet.
                    return Task::perform(update::install(available), |result| {
                        Message::UpdateFailed(result.err().unwrap_or_default())
                    });
                }
            }
            Message::UpdateFailed(err) => self.update = UpdateState::Failed(err),
            Message::DismissUpdate => self.update = UpdateState::Idle,

            Message::JoinVoice(channel_id) => {
                if self.voice_joining.is_some() {
                    return Task::none();
                }
                // Ein Kanal zur Zeit: alte Verbindung zuerst beenden.
                let leave = self.leave_voice();
                self.voice_joining = Some(channel_id);
                self.voice_muted = false;
                self.voice_deafened = false;
                let api = self.api.clone();
                let join = Task::perform(
                    async move { voice::join(api, channel_id).await.map(Arc::new) },
                    move |result| Message::VoiceJoined(channel_id, result),
                );
                return Task::batch([leave, join]);
            }
            Message::VoiceJoined(channel_id, result) => {
                self.voice_joining = None;
                match result {
                    Ok(handle) => {
                        self.voice = Some(handle);
                        // Nur Debug-Builds: `NOVA_DEV_SHARE=1` teilt sofort.
                        if dev_flag("NOVA_DEV_SHARE") && !self.sharing {
                            return Task::batch([
                                self.load_room(channel_id),
                                Task::done(Message::ToggleShare),
                            ]);
                        }
                        // Push-to-Talk: stumm, bis die Taste gedrückt wird.
                        let state = self.push_voice_state();
                        return Task::batch([state, self.load_room(channel_id)]);
                    }
                    Err(err) => self.error = Some(format!("Sprachkanal: {err}")),
                }
            }
            Message::LeaveVoice => {
                self.voice_retries = 0;
                return self.leave_voice();
            }
            Message::ToggleMute => {
                self.voice_muted = !self.voice_muted;
                return self.push_voice_state();
            }
            Message::ToggleDeafen => {
                self.voice_deafened = !self.voice_deafened;
                // Wer nichts hört, spricht auch nicht.
                if self.voice_deafened {
                    self.voice_muted = true;
                }
                return self.push_voice_state();
            }
            Message::ToggleShare => {
                let Some(handle) = &self.voice else {
                    return Task::none();
                };
                self.sharing = !self.sharing;
                handle.share_screen(self.sharing);
                return self.push_voice_state();
            }
            Message::Watch(user) => {
                self.stage = user;
                if user.is_none() {
                    self.theater = false;
                }
            }
            Message::ToggleTheater => {
                self.theater = !self.theater && self.stage.is_some();
            }
            Message::VoiceTick => {
                if let Some(handle) = &self.voice {
                    let status = handle.status();
                    // Teilen ist gescheitert (Aufnahme, Wayland …): Knopf
                    // zurücksetzen und den Grund zeigen.
                    if self.sharing && !status.sharing && status.share_error.is_some() {
                        self.sharing = false;
                        self.error = status.share_error.clone();
                        handle.share_screen(false);
                        self.voice_status = status;
                        return self.push_voice_state();
                    }
                    self.streamers = handle.streamers();
                    // Nur Debug-Builds: `NOVA_DEV_WATCH=1` schaut den ersten Stream an.
                    if self.stage.is_none() && dev_flag("NOVA_DEV_WATCH") {
                        self.stage = self.streamers.first().copied();
                    }
                    if self.stage.is_some_and(|id| !self.streamers.contains(&id)) {
                        self.stage = None;
                        self.theater = false;
                    }
                    if status.ended {
                        let channel_id = handle.channel_id;
                        let leave = self.leave_voice();
                        // Verbindung weggebrochen: bis zu dreimal neu verbinden.
                        if self.voice_retries < 3 {
                            self.voice_retries += 1;
                            self.error = Some(format!(
                                "Sprachverbindung unterbrochen – neuer Versuch ({}/3) …",
                                self.voice_retries
                            ));
                            let retry = Task::perform(
                                tokio::time::sleep(Duration::from_secs(2)),
                                move |()| Message::RetryVoice(channel_id),
                            );
                            return Task::batch([leave, retry]);
                        }
                        self.voice_retries = 0;
                        self.error = Some(
                            status
                                .error
                                .clone()
                                .unwrap_or_else(|| "Sprachverbindung beendet.".into()),
                        );
                        return leave;
                    }
                    if status.connected {
                        self.voice_retries = 0;
                    }
                    self.voice_status = status;
                }
            }
            Message::RetryVoice(channel_id) => {
                if self.voice.is_none() && self.screen == Screen::Chat {
                    return self.update(Message::JoinVoice(channel_id));
                }
            }
            Message::VoiceRoom(channel_id, Ok(list)) => {
                self.voice_rooms.insert(channel_id, list);
            }
            Message::VoiceRoom(_, Err(_)) | Message::Ignore => {}

            // --- Dialoge -------------------------------------------------
            Message::OpenModal(modal) => {
                self.modal_input.clear();
                self.modal_error = None;
                self.modal_busy = false;
                self.confirm = None;
                let task = match &modal {
                    Modal::Invite(None) => match self.detail.as_ref().map(|d| d.guild.id) {
                        Some(guild) => {
                            let api = self.api.clone();
                            Task::perform(
                                async move { api.create_invite(guild).await.map(|i| i.code) },
                                |result| Message::ModalDone(result.map(Outcome::Invite)),
                            )
                        }
                        None => Task::none(),
                    },
                    Modal::Pins(None) => match self.channel {
                        Some(channel) => {
                            let api = self.api.clone();
                            Task::perform(
                                async move { api.pins(channel).await },
                                Message::PinsLoaded,
                            )
                        }
                        None => Task::none(),
                    },
                    Modal::Settings => {
                        self.modal_input = self
                            .me
                            .as_ref()
                            .map(|m| m.display_name.clone())
                            .unwrap_or_default();
                        // Geräteliste kann dauern (WASAPI) → nicht im UI-Thread.
                        Task::perform(
                            async { tokio::task::spawn_blocking(voice::list_devices).await },
                            |result| result.map_or(Message::Ignore, Message::DevicesLoaded),
                        )
                    }
                    _ => Task::none(),
                };
                self.modal = Some(modal);
                return task;
            }
            Message::CloseModal => {
                self.modal = None;
                self.confirm = None;
                self.capturing_key = false;
            }
            Message::ModalInput(value) => self.modal_input = value,
            Message::ModalSubmit => return self.submit_modal(),
            Message::ModalDone(Ok(outcome)) => {
                self.modal_busy = false;
                match outcome {
                    Outcome::Invite(code) => self.modal = Some(Modal::Invite(Some(code))),
                    Outcome::Guild(target) => {
                        self.modal = None;
                        if let Some(id) = target {
                            // Neuer Server: sofort öffnen, Liste nachladen.
                            self.detail = None;
                            self.channel = None;
                            let api = self.api.clone();
                            return Task::batch([
                                self.update(Message::OpenGuild(id)),
                                Task::perform(
                                    async move { api.guilds().await },
                                    Message::GuildsLoaded,
                                ),
                            ]);
                        }
                        let api = self.api.clone();
                        return Task::perform(
                            async move { api.guilds().await },
                            Message::GuildsLoaded,
                        );
                    }
                    Outcome::Channel(id) => {
                        self.modal = None;
                        self.channel = Some(id);
                        return self.reload_guild();
                    }
                    Outcome::Profile(user) => {
                        self.me = Some(user);
                        self.modal = None;
                        return self.reload_guild();
                    }
                    Outcome::Reload => {
                        if !matches!(self.modal, Some(Modal::Settings)) {
                            self.modal = None;
                        }
                        self.confirm = None;
                        return self.reload_guild();
                    }
                }
            }
            Message::ModalDone(Err(err)) => {
                self.modal_busy = false;
                self.modal_error = Some(err);
            }
            Message::CopyInvite(code) => {
                return iced::clipboard::write(code);
            }
            Message::PinsLoaded(result) => match result {
                Ok(list) => {
                    if matches!(self.modal, Some(Modal::Pins(_))) {
                        self.modal = Some(Modal::Pins(Some(list)));
                    }
                }
                Err(err) => self.modal_error = Some(err),
            },

            // --- Nachrichten ---------------------------------------------
            Message::StartEdit(id, content) => self.editing = Some((id, content)),
            Message::EditChanged(value) => {
                if let Some((_, draft)) = &mut self.editing {
                    *draft = value;
                }
            }
            Message::CancelEdit => self.editing = None,
            Message::SaveEdit => {
                let Some((message_id, content)) = self.editing.take() else {
                    return Task::none();
                };
                let content = content.trim().to_owned();
                if content.is_empty() {
                    return self.update(Message::Delete(message_id));
                }
                let nonce = self.next_nonce;
                self.next_nonce += 1;
                if let Some(message) = self.find_message(self.channel, message_id) {
                    message.content.clone_from(&content);
                    message.edited = true;
                }
                self.send_gateway(Command::Edit {
                    nonce,
                    message_id,
                    content,
                });
            }
            Message::Delete(message_id) => {
                let nonce = self.next_nonce;
                self.next_nonce += 1;
                self.send_gateway(Command::Delete { nonce, message_id });
            }
            Message::React(message_id, emoji) => {
                let channel = self.channel;
                let Some(message) = self.find_message(channel, message_id) else {
                    return Task::none();
                };
                let add = !message.reactions.iter().any(|r| r.emoji == emoji && r.me);
                // Sofort anzeigen; das Gateway-Echo eines anderen Nutzers zählt
                // separat, das eigene wird über `user_id == me` übersprungen.
                message.react(&emoji, if add { 1 } else { -1 }, true);
                self.send_gateway(Command::React {
                    message_id,
                    emoji,
                    add,
                });
            }
            Message::TogglePin(message_id, pinned) => {
                if let Some(channel) = self.channel {
                    let api = self.api.clone();
                    return Task::perform(
                        async move { api.set_pin(channel, message_id, pinned).await },
                        Message::ActionDone,
                    );
                }
            }
            Message::ArchiveThread(channel_id) => {
                let api = self.api.clone();
                let parent = self.current_channel().and_then(|c| c.parent_id);
                if let Some(parent) = parent {
                    self.channel = Some(parent);
                }
                return Task::perform(
                    async move {
                        api.update_channel(channel_id, serde_json::json!({ "archived": true }))
                            .await
                    },
                    |result| Message::ModalDone(result.map(|()| Outcome::Reload)),
                );
            }
            Message::LoadOlder => {
                let home = self.home;
                let Some(channel) = (if home { self.dm } else { self.channel }) else {
                    return Task::none();
                };
                let Some(oldest) = self
                    .history
                    .get(&channel)
                    .and_then(|list| list.iter().find_map(|m| m.id))
                else {
                    return Task::none();
                };
                let api = self.api.clone();
                return Task::perform(
                    async move {
                        if home {
                            api.dm_messages(channel, Some(oldest)).await
                        } else {
                            api.older_messages(channel, oldest).await
                        }
                    },
                    move |result| Message::OlderLoaded(channel, result),
                );
            }
            Message::OlderLoaded(channel, Ok(list)) => {
                if list.len() < 50 {
                    self.history_complete.insert(channel);
                }
                let mut older: Vec<ChatMessage> =
                    list.into_iter().map(ChatMessage::from_rest).collect();
                let current = self.history.entry(channel).or_default();
                older.retain(|m| !current.iter().any(|c| c.id == m.id));
                older.append(current);
                *current = older;
            }

            // --- Direktnachrichten ---------------------------------------
            Message::OpenHome => {
                self.home = true;
                self.editing = None;
                let api = self.api.clone();
                return Task::perform(async move { api.dms().await }, Message::DmsLoaded);
            }
            Message::SelectGuild(id) => {
                self.home = false;
                self.editing = None;
                return self.update(Message::OpenGuild(id));
            }
            Message::DmsLoaded(Ok(list)) => {
                self.dms = list;
                if self.home
                    && self.dm.is_none()
                    && let Some(first) = self.dms.first().map(|d| d.id)
                {
                    return self.update(Message::OpenDm(first));
                }
            }
            Message::DmsLoaded(Err(err)) => self.dm_error = Some(err),
            Message::OpenDm(id) => {
                self.home = true;
                self.dm = Some(id);
                self.editing = None;
                self.dm_unread.remove(&id);
                let api = self.api.clone();
                return Task::perform(
                    async move { api.dm_messages(id, None).await },
                    move |result| Message::HistoryLoaded(id, result),
                );
            }
            Message::DmInput(value) => {
                self.dm_input = value;
                self.dm_error = None;
            }
            Message::StartDm => {
                let handle = self.dm_input.trim().to_owned();
                if handle.is_empty() || self.dm_busy {
                    return Task::none();
                }
                return self.update(Message::DmWith(handle));
            }
            Message::DmWith(handle) => {
                self.dm_busy = true;
                self.dm_error = None;
                self.home = true;
                self.modal = None;
                let api = self.api.clone();
                return Task::perform(async move { api.open_dm(&handle).await }, Message::DmOpened);
            }
            Message::DmOpened(Ok(dm)) => {
                self.dm_busy = false;
                self.dm_input.clear();
                let id = dm.id;
                self.dms.retain(|d| d.id != id);
                self.dms.insert(0, dm);
                return self.update(Message::OpenDm(id));
            }
            Message::DmOpened(Err(err)) => {
                self.dm_busy = false;
                self.dm_error = Some(err);
            }
            Message::DmSent(dm, nonce, Ok(sent)) => {
                let list = self.history.entry(dm).or_default();
                // Kam die Nachricht schon über das Gateway, ist der Platzhalter
                // überflüssig; sonst bekommt er jetzt seine ID.
                let elsewhere = list
                    .iter()
                    .any(|m| m.id == Some(sent.id) && m.nonce != Some(nonce));
                if elsewhere {
                    list.retain(|m| m.nonce != Some(nonce));
                } else if let Some(local) = list.iter_mut().find(|m| m.nonce == Some(nonce)) {
                    local.id = Some(sent.id);
                }
                self.bump_dm(dm);
            }
            Message::DmSent(dm, nonce, Err(err)) => {
                if let Some(list) = self.history.get_mut(&dm) {
                    list.retain(|m| m.nonce != Some(nonce));
                }
                self.error = Some(err);
            }

            // --- Anhänge -------------------------------------------------
            Message::PickFiles => {
                let Some(target) = self.upload_target() else {
                    return Task::none();
                };
                return Task::perform(attach::pick_files(), move |(files, errors)| {
                    Message::FilesPicked(target, files, errors)
                });
            }
            Message::FilesPicked(target, files, errors) => {
                if !errors.is_empty() {
                    self.error = Some(errors.join(" "));
                }
                let mut tasks = Vec::new();
                for file in files {
                    let list = self.pending.entry(target).or_default();
                    if list.len() >= attach::MAX_ATTACHMENTS {
                        self.error = Some("Höchstens 10 Anhänge pro Nachricht.".into());
                        break;
                    }
                    let key = self.next_nonce;
                    self.next_nonce += 1;
                    list.push(attach::PendingFile {
                        key,
                        name: file.name.clone(),
                        size: file.bytes.len() as i64,
                        state: attach::PendingState::Uploading,
                    });
                    let api = self.api.clone();
                    tasks.push(Task::perform(
                        async move { api.upload(target, &file.name, file.bytes).await },
                        move |result| Message::Uploaded(target, key, result),
                    ));
                }
                return Task::batch(tasks);
            }
            Message::Uploaded(target, key, result) => {
                let item = self
                    .pending
                    .get_mut(&target)
                    .and_then(|list| list.iter_mut().find(|file| file.key == key));
                match (item, result) {
                    (Some(item), Ok(attachment)) => {
                        item.state = attach::PendingState::Done(attachment);
                    }
                    (Some(item), Err(err)) => {
                        self.error = Some(format!("{}: {err}", item.name));
                        item.state = attach::PendingState::Failed(err);
                    }
                    // Inzwischen entfernt: gleich wieder verwerfen.
                    (None, Ok(attachment)) => {
                        let api = self.api.clone();
                        return Task::perform(
                            async move { api.discard_attachment(attachment.id).await },
                            |_| Message::Ignore,
                        );
                    }
                    (None, Err(_)) => {}
                }
            }
            Message::RemovePending(target, key) => {
                let removed = self.pending.get_mut(&target).and_then(|list| {
                    let pos = list.iter().position(|file| file.key == key)?;
                    Some(list.remove(pos))
                });
                if let Some(attach::PendingFile {
                    state: attach::PendingState::Done(attachment),
                    ..
                }) = removed
                {
                    let api = self.api.clone();
                    return Task::perform(
                        async move { api.discard_attachment(attachment.id).await },
                        |_| Message::Ignore,
                    );
                }
            }
            Message::PreviewLoaded(id, result) => {
                let preview = match result {
                    Ok(decoded) => attach::Preview::Ready(decoded.handle()),
                    Err(_) => attach::Preview::Failed,
                };
                self.previews.insert(id, preview);
            }
            Message::SaveAttachment(attachment) => {
                let api = self.api.clone();
                return Task::perform(attach::save(api, attachment), Message::Saved);
            }
            Message::Saved(Ok(_)) => {}
            Message::Saved(Err(err)) => self.error = Some(err),

            // --- Moderation ----------------------------------------------
            Message::AskConfirm(action) => self.confirm = action,
            Message::Moderate(user, action) => {
                let Some(guild) = self.detail.as_ref().map(|d| d.guild.id) else {
                    return Task::none();
                };
                self.confirm = None;
                self.modal = None;
                let api = self.api.clone();
                return Task::perform(
                    async move {
                        match action {
                            ModAction::Kick => api.kick(guild, user).await,
                            ModAction::Ban => api.ban(guild, user).await,
                        }
                    },
                    |result| Message::ModalDone(result.map(|()| Outcome::Reload)),
                );
            }
            Message::Timeout(user, seconds) => {
                let Some(guild) = self.detail.as_ref().map(|d| d.guild.id) else {
                    return Task::none();
                };
                self.modal_busy = true;
                let api = self.api.clone();
                return Task::perform(
                    async move { api.timeout(guild, user, seconds).await },
                    |result| Message::ModalDone(result.map(|()| Outcome::Reload)),
                );
            }
            Message::ToggleRole(user, role, on) => {
                let Some(guild) = self.detail.as_ref().map(|d| d.guild.id) else {
                    return Task::none();
                };
                // Sofort umschalten, Server lädt danach neu.
                if let Some(member) = self
                    .detail
                    .as_mut()
                    .and_then(|d| d.members.iter_mut().find(|m| m.user_id == Some(user)))
                {
                    if on {
                        member.role_ids.push(role);
                    } else {
                        member.role_ids.retain(|r| *r != role);
                    }
                }
                let api = self.api.clone();
                return Task::perform(
                    async move { api.set_member_role(guild, user, role, on).await },
                    Message::ActionDone,
                );
            }
            Message::ActionDone(Ok(())) => {}

            // --- Einstellungen -------------------------------------------
            Message::SetInput(name) => {
                self.prefs.input = (name != DEFAULT_DEVICE).then_some(name);
                self.save_prefs();
            }
            Message::SetOutput(name) => {
                self.prefs.output = (name != DEFAULT_DEVICE).then_some(name);
                self.save_prefs();
            }
            Message::SetPushToTalk(on) => {
                self.prefs.push_to_talk = on;
                if on && self.prefs.ptt_key.is_none() {
                    self.capturing_key = true;
                }
                self.save_prefs();
                return self.push_voice_state();
            }
            Message::CaptureKey => self.capturing_key = true,
            Message::DevicesLoaded(devices) => self.devices = devices,
            Message::Key(event) => return self.on_key(event),
            Message::Tick => {
                for people in self.typing.values_mut() {
                    people.retain(|_, (_, at)| at.elapsed() < TYPING_TTL);
                }
                self.typing.retain(|_, people| !people.is_empty());
            }
        }
        Task::none()
    }

    fn load_room(&self, channel_id: i64) -> Task<Message> {
        let api = self.api.clone();
        Task::perform(
            async move { api.voice_participants(channel_id).await },
            move |result| Message::VoiceRoom(channel_id, result),
        )
    }

    /// Verbindung lösen (Threads enden beim Fallenlassen) und dem Server melden.
    fn leave_voice(&mut self) -> Task<Message> {
        let Some(handle) = self.voice.take() else {
            return Task::none();
        };
        let channel_id = handle.channel_id;
        drop(handle);
        self.voice_status = voice::VoiceStatus::default();
        self.sharing = false;
        self.streamers.clear();
        self.stage = None;
        self.theater = false;
        let api = self.api.clone();
        Task::perform(async move { api.voice_leave(channel_id).await }, |_| {
            Message::Ignore
        })
    }

    fn push_voice_state(&self) -> Task<Message> {
        let Some(handle) = &self.voice else {
            return Task::none();
        };
        handle.set_muted(self.effective_mute());
        handle.set_deafened(self.voice_deafened);
        let api = self.api.clone();
        let (channel_id, muted, deafened, screen) = (
            handle.channel_id,
            self.voice_muted,
            self.voice_deafened,
            self.sharing,
        );
        Task::perform(
            async move { api.voice_state(channel_id, muted, deafened, screen).await },
            |_| Message::Ignore,
        )
    }

    /// Nur Debug-Builds: `NOVA_DEV_MODAL=settings|invite|member|pins|guild|join|channel|thread`
    /// öffnet nach dem Laden direkt einen Dialog (für Screenshots ohne Klicks).
    fn dev_modal(&self) -> Task<Message> {
        if !cfg!(debug_assertions) {
            return Task::none();
        }
        let Ok(which) = std::env::var("NOVA_DEV_MODAL") else {
            return Task::none();
        };
        let other = self.detail.as_ref().and_then(|d| {
            d.members
                .iter()
                .filter_map(|m| m.user_id)
                .find(|id| *id != self.me_id())
        });
        let first_message = self
            .channel
            .and_then(|c| self.history.get(&c))
            .and_then(|list| list.iter().find_map(|m| m.id))
            .unwrap_or_default();
        let modal = match which.as_str() {
            "settings" => Modal::Settings,
            "invite" => Modal::Invite(None),
            "member" => Modal::Member(other.unwrap_or_else(|| self.me_id())),
            "pins" => Modal::Pins(None),
            "guild" => Modal::CreateGuild,
            "join" => Modal::JoinGuild,
            "channel" => Modal::CreateChannel { voice: false },
            "thread" => Modal::Thread {
                message_id: first_message,
            },
            _ => return Task::none(),
        };
        // Kurz warten, bis Verlauf und Rechte da sind.
        Task::perform(tokio::time::sleep(Duration::from_millis(600)), move |()| {
            Message::OpenModal(modal.clone())
        })
    }

    /// Nur Debug-Builds: `NOVA_DEV_ATTACH=<datei>[,<datei>…]` hängt die Dateien
    /// nach dem Laden an (Dateidialoge lassen sich nicht fernsteuern).
    fn dev_attach(&self) -> Task<Message> {
        if !cfg!(debug_assertions) {
            return Task::none();
        }
        let (Ok(paths), Some(channel)) = (std::env::var("NOVA_DEV_ATTACH"), self.channel) else {
            return Task::none();
        };
        let files: Vec<attach::Picked> = paths
            .split(',')
            .filter_map(|path| {
                let path = std::path::Path::new(path.trim());
                Some(attach::Picked {
                    name: path.file_name()?.to_string_lossy().into_owned(),
                    bytes: std::fs::read(path).ok()?,
                })
            })
            .collect();
        Task::done(Message::FilesPicked(
            api::UploadTarget::Channel(channel),
            files,
            Vec::new(),
        ))
    }

    /// Nur Debug-Builds: `NOVA_DEV_VOICE=1` öffnet den ersten Sprachkanal und
    /// tritt bei (für Stream-Tests ohne Klicks).
    fn dev_voice(&self) -> Task<Message> {
        if !dev_flag("NOVA_DEV_VOICE") || self.voice.is_some() {
            return Task::none();
        }
        let Some(id) = self
            .detail
            .as_ref()
            .and_then(|d| d.channels.iter().find(|c| c.kind == 1))
            .map(|c| c.id)
        else {
            return Task::none();
        };
        Task::batch([
            Task::done(Message::OpenChannel(id)),
            Task::done(Message::JoinVoice(id)),
        ])
    }

    fn save_prefs(&self) {
        self.prefs.save();
        voice::set_devices(self.prefs.input.clone(), self.prefs.output.clone());
    }

    /// Absenden des offenen Dialogs (Enter oder Knopf).
    fn submit_modal(&mut self) -> Task<Message> {
        let Some(modal) = self.modal.clone() else {
            return Task::none();
        };
        if self.modal_busy {
            return Task::none();
        }
        let value = self.modal_input.trim().to_owned();
        let api = self.api.clone();
        let guild = self.detail.as_ref().map(|d| d.guild.id);
        let channel = self.channel;
        let task = match modal {
            Modal::CreateGuild => {
                if value.is_empty() {
                    self.modal_error = Some("Gib dem Server einen Namen.".into());
                    return Task::none();
                }
                Task::perform(
                    async move { api.create_guild(&value).await.map(|g| Some(g.id)) },
                    |result| Message::ModalDone(result.map(Outcome::Guild)),
                )
            }
            Modal::JoinGuild => {
                if value.is_empty() {
                    self.modal_error = Some("Füge den Einladungscode oder -link ein.".into());
                    return Task::none();
                }
                Task::perform(async move { api.join_invite(&value).await }, |result| {
                    Message::ModalDone(result.map(|()| Outcome::Guild(None)))
                })
            }
            Modal::CreateChannel { voice } => {
                let name = channel_slug(&value, voice);
                let Some(guild) = guild.filter(|_| !name.is_empty()) else {
                    self.modal_error = Some("Gib dem Kanal einen Namen.".into());
                    return Task::none();
                };
                Task::perform(
                    async move {
                        api.create_channel(guild, &name, i16::from(voice))
                            .await
                            .map(|c| c.id)
                    },
                    |result| Message::ModalDone(result.map(Outcome::Channel)),
                )
            }
            Modal::Thread { message_id } => {
                let Some(channel) = channel else {
                    return Task::none();
                };
                let name = if value.is_empty() {
                    "Thread".to_owned()
                } else {
                    value
                };
                Task::perform(
                    async move {
                        api.create_thread(channel, message_id, &name)
                            .await
                            .map(|c| c.id)
                    },
                    |result| Message::ModalDone(result.map(Outcome::Channel)),
                )
            }
            Modal::Settings => {
                if value.is_empty() {
                    self.modal_error = Some("Der Anzeigename darf nicht leer sein.".into());
                    return Task::none();
                }
                Task::perform(async move { api.update_profile(&value).await }, |result| {
                    Message::ModalDone(result.map(Outcome::Profile))
                })
            }
            Modal::Invite(_) | Modal::Member(_) | Modal::Pins(_) => return Task::none(),
        };
        self.modal_busy = true;
        self.modal_error = None;
        task
    }

    fn on_key(&mut self, event: keyboard::Event) -> Task<Message> {
        match event {
            keyboard::Event::KeyPressed {
                key,
                physical_key,
                repeat,
                ..
            } => {
                let code = format!("{physical_key:?}");
                if self.capturing_key {
                    if key != keyboard::Key::Named(keyboard::key::Named::Escape) {
                        self.prefs.ptt_key = Some(code);
                        self.prefs.push_to_talk = true;
                        self.save_prefs();
                    }
                    self.capturing_key = false;
                    return self.push_voice_state();
                }
                if key == keyboard::Key::Named(keyboard::key::Named::Escape) {
                    if self.theater {
                        self.theater = false;
                        return Task::none();
                    }
                    if self.modal.is_some() {
                        return self.update(Message::CloseModal);
                    }
                    self.editing = None;
                    return Task::none();
                }
                if !repeat
                    && self.prefs.push_to_talk
                    && self.prefs.ptt_key.as_deref() == Some(code.as_str())
                    && !self.ptt_held
                {
                    self.ptt_held = true;
                    if let Some(handle) = &self.voice {
                        handle.set_muted(self.effective_mute());
                    }
                }
            }
            keyboard::Event::KeyReleased { physical_key, .. } => {
                let code = format!("{physical_key:?}");
                if self.ptt_held && self.prefs.ptt_key.as_deref() == Some(code.as_str()) {
                    self.ptt_held = false;
                    if let Some(handle) = &self.voice {
                        handle.set_muted(self.effective_mute());
                    }
                }
            }
            keyboard::Event::ModifiersChanged(_) => {}
        }
        Task::none()
    }

    fn on_gateway(&mut self, event: GatewayEvent) -> Task<Message> {
        match event {
            GatewayEvent::Ready(sender) => {
                if let Some(id) = self.channel {
                    let _ = sender.unbounded_send(Command::Subscribe(id));
                }
                self.gateway = Some(sender);
            }
            GatewayEvent::Status(status) => {
                if status == Status::Online
                    && self.status != Status::Online
                    && let Some(start) = START.get()
                {
                    eprintln!(
                        "[nova] Gateway online nach {} ms",
                        start.elapsed().as_millis()
                    );
                }
                self.status = status;
            }
            GatewayEvent::Hello { guilds, .. } => {
                // Neue oder entfernte Server seit dem letzten REST-Abruf übernehmen.
                if guilds.len() != self.guilds.len() {
                    let api = self.api.clone();
                    return Task::perform(async move { api.guilds().await }, Message::GuildsLoaded);
                }
            }
            GatewayEvent::Event(event) => match *event {
                Event::MessageCreate { message } => {
                    let channel_id = message.channel_id;
                    let incoming = ChatMessage::from_gateway(message);
                    let list = self.history.entry(channel_id).or_default();
                    // Bereits per Ack bestätigt → nichts tun.
                    if list.iter().any(|m| m.id == incoming.id) {
                        return Task::none();
                    }
                    let author = incoming.author_id;
                    list.push(incoming);
                    // Wer sendet, tippt nicht mehr.
                    if let Some(people) = self.typing.get_mut(&channel_id) {
                        people.remove(&author);
                    }
                    if self.channel == Some(channel_id) {
                        self.mark_read();
                    } else if author != self.me_id() {
                        self.unread.insert(channel_id);
                    }
                }
                Event::DmMessageCreate { message } => {
                    let dm = message.channel_id;
                    let incoming = ChatMessage::from_gateway(message);
                    let mine = incoming.author_id == self.me_id();
                    let list = self.history.entry(dm).or_default();
                    if list.iter().any(|m| m.id == incoming.id) {
                        return Task::none();
                    }
                    // Eigene Nachricht aus diesem Fenster: Platzhalter bestätigen.
                    if mine
                        && let Some(local) = list
                            .iter_mut()
                            .find(|m| m.id.is_none() && m.content == incoming.content)
                    {
                        local.id = incoming.id;
                    } else {
                        list.push(incoming);
                    }
                    if !mine && !(self.home && self.dm == Some(dm)) {
                        self.dm_unread.insert(dm);
                    }
                    if self.dms.iter().any(|d| d.id == dm) {
                        self.bump_dm(dm);
                    } else {
                        // Neue Unterhaltung: Liste samt Gegenüber neu holen.
                        let api = self.api.clone();
                        return Task::perform(async move { api.dms().await }, Message::DmsLoaded);
                    }
                }
                Event::MessageUpdate { message } => {
                    let channel_id = message.channel_id;
                    let updated = ChatMessage::from_gateway(message);
                    if let Some(list) = self.history.get_mut(&channel_id)
                        && let Some(existing) = list.iter_mut().find(|m| m.id == updated.id)
                    {
                        existing.content = updated.content;
                        existing.edited = updated.edited;
                        existing.pinned = updated.pinned;
                    }
                }
                // Eigene Reaktionen sind schon lokal gezählt.
                Event::ReactionAdd {
                    channel_id,
                    message_id,
                    user_id,
                    emoji,
                } if user_id != self.me_id() => {
                    if let Some(message) = self.find_message(Some(channel_id), message_id) {
                        message.react(&emoji, 1, false);
                    }
                }
                Event::ReactionRemove {
                    channel_id,
                    message_id,
                    user_id,
                    emoji,
                } if user_id != self.me_id() => {
                    if let Some(message) = self.find_message(Some(channel_id), message_id) {
                        message.react(&emoji, -1, false);
                    }
                }
                Event::TypingStart {
                    channel_id,
                    user_id,
                    display_name,
                } => {
                    if user_id != self.me_id() {
                        self.typing
                            .entry(channel_id)
                            .or_default()
                            .insert(user_id, (display_name, Instant::now()));
                    }
                }
                Event::MessagePin {
                    channel_id,
                    message_id,
                    pinned,
                } => {
                    if let Some(message) = self.find_message(Some(channel_id), message_id) {
                        message.pinned = pinned;
                    }
                }
                Event::PresenceUpdate {
                    guild_id,
                    user_id,
                    status,
                } => {
                    if let Some(member) = self
                        .detail
                        .as_mut()
                        .filter(|d| d.guild.id == guild_id)
                        .and_then(|d| d.members.iter_mut().find(|m| m.user_id == Some(user_id)))
                    {
                        member.status = status;
                    }
                }
                Event::GuildUpdate { guild_id } | Event::GuildMemberUpdate { guild_id, .. } => {
                    if self.detail.as_ref().map(|d| d.guild.id) == Some(guild_id) {
                        return self.reload_guild();
                    }
                }
                Event::GuildMemberRemove { guild_id, user_id } => {
                    if user_id == self.me_id() {
                        // Gekickt oder gebannt: Server verlassen.
                        if self.detail.as_ref().map(|d| d.guild.id) == Some(guild_id) {
                            self.detail = None;
                            self.channel = None;
                        }
                        let api = self.api.clone();
                        return Task::perform(
                            async move { api.guilds().await },
                            Message::GuildsLoaded,
                        );
                    }
                    if self.detail.as_ref().map(|d| d.guild.id) == Some(guild_id) {
                        return self.reload_guild();
                    }
                }
                Event::MessageDelete {
                    channel_id,
                    message_id,
                } => {
                    if let Some(list) = self.history.get_mut(&channel_id) {
                        list.retain(|m| m.id != Some(message_id));
                    }
                }
                Event::VoiceStateUpdate { channel_id, .. } => {
                    // Teilnehmerliste dieses Kanals neu holen statt zu pollen.
                    return self.load_room(channel_id);
                }
                _ => {}
            },
            GatewayEvent::Ack { nonce, id } => {
                // Zuordnung über die Nonce, nicht über den Text: zwei gleiche
                // Nachrichten hintereinander bleiben so zwei Nachrichten.
                for list in self.history.values_mut() {
                    let Some(pos) = list.iter().position(|m| m.nonce == Some(nonce)) else {
                        continue;
                    };
                    let echoed = id.is_some() && list.iter().any(|m| m.id == id);
                    if echoed || id.is_none() {
                        // Dispatch war schneller (Duplikat) oder Server hat abgelehnt.
                        list.remove(pos);
                    } else {
                        list[pos].id = id;
                        list[pos].nonce = None;
                    }
                    break;
                }
            }
            GatewayEvent::Error(err) => self.error = Some(err),
            GatewayEvent::Resync => {
                if let Some(id) = self.channel {
                    return self.update(Message::OpenChannel(id));
                }
            }
            GatewayEvent::SessionExpired => return self.update(Message::LoggedOut),
        }
        Task::none()
    }

    // -----------------------------------------------------------------------
    // Ansicht
    // -----------------------------------------------------------------------

    fn view(&self) -> Element<'_, Message> {
        if FIRST_FRAME.set(()).is_ok()
            && let Some(start) = START.get()
        {
            eprintln!(
                "[nova] erste Ansicht nach {} ms",
                start.elapsed().as_millis()
            );
        }
        let page = match self.screen {
            Screen::Booting => container(
                column![
                    icon(Icon::Logo, 40.0, style::TEXT),
                    text("Sitzung wird geprüft…").color(style::MUTED),
                ]
                .spacing(16)
                .align_x(iced::Center),
            )
            .center(Fill)
            .style(style::app_bg)
            .into(),
            Screen::Login => self.view_login(),
            Screen::Chat => self.view_chat(),
        };
        match self.view_update_banner() {
            Some(banner) => column![banner, page].into(),
            None => page,
        }
    }

    /// Leiste oben, sobald es etwas zum Update zu sagen gibt.
    fn view_update_banner(&self) -> Option<Element<'_, Message>> {
        let (message, action, tone): (String, Option<(&str, Message)>, Color) = match &self.update {
            UpdateState::Available(available) => (
                format!(
                    "Version {} ist verfügbar (installiert: {}).",
                    available.version,
                    update::CURRENT
                ),
                Some(("Jetzt aktualisieren", Message::InstallUpdate)),
                style::MUTED,
            ),
            UpdateState::Installing(version) => (
                format!("Version {version} wird geladen und geprüft… Nova startet gleich neu."),
                None,
                style::MUTED,
            ),
            UpdateState::Failed(err) => (
                format!("Update fehlgeschlagen: {err}"),
                Some(("Erneut versuchen", Message::CheckUpdate)),
                style::DANGER,
            ),
            _ => return None,
        };
        let mut bar = row![text(message).size(13).color(style::TEXT).width(Fill)]
            .spacing(8)
            .align_y(iced::Center);
        if let Some((label, msg)) = action {
            bar = bar.push(
                button(text(label).size(13))
                    .padding([6, 12])
                    .on_press(msg)
                    .style(style::primary),
            );
        }
        if !matches!(self.update, UpdateState::Installing(_)) {
            bar = bar.push(
                button(text("Später").size(13))
                    .on_press(Message::DismissUpdate)
                    .style(style::ghost),
            );
        }
        Some(
            container(bar)
                .padding([8, 16])
                .width(Fill)
                .style(move |_: &Theme| style::banner(tone))
                .into(),
        )
    }

    /// Kleiner Knopf „Nach Updates suchen“ samt aktuellem Stand.
    fn view_update_button(&self) -> Element<'_, Message> {
        let label = match &self.update {
            UpdateState::Checking => "Suche Updates…".to_owned(),
            UpdateState::UpToDate => format!("Version {} ist aktuell", update::CURRENT),
            UpdateState::Available(a) => format!("Update auf v{}", a.version),
            UpdateState::Installing(_) => "Wird installiert…".to_owned(),
            UpdateState::Idle | UpdateState::Failed(_) => {
                format!("Version {} – nach Updates suchen", update::CURRENT)
            }
        };
        let message = match &self.update {
            UpdateState::Available(_) => Some(Message::InstallUpdate),
            UpdateState::Checking | UpdateState::Installing(_) => None,
            _ => Some(Message::CheckUpdate),
        };
        button(text(label).size(11))
            .padding([2, 6])
            .on_press_maybe(message)
            .style(style::link)
            .into()
    }

    /// Kompakte Variante für die Nutzerleiste: Icon, Stand im Tooltip.
    fn view_update_icon(&self) -> Element<'_, Message> {
        let (glyph, color, label, message) = match &self.update {
            UpdateState::Available(a) => (
                Icon::Download,
                style::TEXT,
                format!("Update auf v{} installieren", a.version),
                Some(Message::InstallUpdate),
            ),
            UpdateState::Checking => (
                Icon::Refresh,
                style::FAINT,
                "Suche Updates…".to_owned(),
                None,
            ),
            UpdateState::Installing(_) => (
                Icon::Download,
                style::FAINT,
                "Wird installiert…".to_owned(),
                None,
            ),
            UpdateState::UpToDate => (
                Icon::Refresh,
                style::MUTED,
                format!("Version {} ist aktuell. Erneut prüfen", update::CURRENT),
                Some(Message::CheckUpdate),
            ),
            UpdateState::Idle | UpdateState::Failed(_) => (
                Icon::Refresh,
                style::MUTED,
                format!("Nach Updates suchen (Version {})", update::CURRENT),
                Some(Message::CheckUpdate),
            ),
        };
        tip(
            button(icon_hover(glyph, 18.0, color, style::TEXT))
                .padding(7)
                .on_press_maybe(message)
                .style(|theme, status| style::icon_button(theme, status, false)),
            label,
            tooltip::Position::Top,
        )
    }

    fn view_login(&self) -> Element<'_, Message> {
        let submit_label = if self.busy {
            "Einen Moment…"
        } else if self.register {
            "Konto erstellen"
        } else {
            "Anmelden"
        };
        let subtitle = if self.register {
            "Leg dir in wenigen Sekunden ein Konto an."
        } else {
            "Melde dich an, um weiterzuschreiben."
        };
        let head = column![
            container(icon(Icon::Logo, 32.0, style::TEXT)).center(60),
            space().height(6),
            text("Nova").size(26).font(style::BOLD).color(style::TEXT),
            text(subtitle).size(14).color(style::MUTED),
        ]
        .spacing(4)
        .width(Fill)
        .align_x(iced::Center);

        let mut form = column![
            head,
            space().height(10),
            field(
                "Server",
                text_input("https://…", &self.server)
                    .on_input(Message::ServerChanged)
                    .padding(10)
                    .style(style::input),
            ),
            field(
                "Benutzername",
                text_input("dein_name", &self.handle)
                    .on_input(Message::HandleChanged)
                    .on_submit(Message::Submit)
                    .padding(10)
                    .style(style::input),
            ),
            field(
                "Passwort",
                text_input(
                    if self.register {
                        "Mindestens 10 Zeichen"
                    } else {
                        "Passwort"
                    },
                    &self.password,
                )
                .on_input(Message::PasswordChanged)
                .on_submit(Message::Submit)
                .secure(true)
                .padding(10)
                .style(style::input),
            ),
        ]
        .spacing(14);
        if let Some(err) = &self.auth_error {
            form = form.push(text(err).color(style::DANGER).size(13));
        }
        form = form
            .push(space().height(2))
            .push(
                button(text(submit_label).font(style::BOLD).center().width(Fill))
                    .on_press_maybe((!self.busy).then_some(Message::Submit))
                    .width(Fill)
                    .padding(12)
                    .style(style::primary),
            )
            .push(
                container(
                    button(
                        text(if self.register {
                            "Schon ein Konto? Anmelden"
                        } else {
                            "Neu hier? Konto erstellen"
                        })
                        .size(13),
                    )
                    .on_press(Message::ToggleRegister)
                    .style(style::link),
                )
                .center_x(Fill),
            );

        let footer = self.view_update_button();

        container(
            column![
                container(form.width(340)).padding(32).style(style::card),
                footer,
            ]
            .spacing(14)
            .align_x(iced::Center),
        )
        .center(Fill)
        .style(style::app_bg)
        .into()
    }

    fn view_chat(&self) -> Element<'_, Message> {
        if self.theater
            && let Some(stage) = self.view_stage(true)
        {
            return container(stage)
                .width(Fill)
                .height(Fill)
                .style(style::stage)
                .into();
        }
        let mut layout = row![self.view_rail(), self.view_sidebar(), self.view_main()];
        if self.show_members && self.detail.is_some() && !self.home {
            layout = layout.push(vline(Fill)).push(self.view_members());
        }
        let mut page = column![layout.height(Fill)];
        if let Some(err) = &self.error {
            page = page.push(
                container(
                    row![
                        text(err).size(13).color(style::TEXT).width(Fill),
                        button(icon(Icon::Close, 14.0, style::MUTED))
                            .padding(6)
                            .on_press(Message::DismissError)
                            .style(|theme, status| style::icon_button(theme, status, false)),
                    ]
                    .align_y(iced::Center),
                )
                .padding([4, 12])
                .style(style::toast),
            );
        }
        let base = container(page)
            .width(Fill)
            .height(Fill)
            .style(style::app_bg);
        match &self.modal {
            Some(modal) => {
                // Klick auf den Hintergrund schließt; die Karte selbst schluckt
                // alle Klicks (`opaque`), damit sie nicht durchfallen.
                let card = opaque(
                    container(
                        // Schmale Leiste mit eigenem Platz, damit sie nichts überdeckt.
                        scrollable(self.view_modal(modal))
                            .direction(scrollable::Direction::Vertical(
                                scrollable::Scrollbar::new()
                                    .width(4)
                                    .scroller_width(4)
                                    .spacing(8),
                            ))
                            .height(Length::Shrink),
                    )
                    .width(Length::Fixed(452.0))
                    .max_height(680)
                    .padding(iced::Padding {
                        top: 24.0,
                        right: 12.0,
                        bottom: 24.0,
                        left: 24.0,
                    })
                    .style(style::modal),
                );
                let layer = mouse_area(
                    container(card)
                        .center(Fill)
                        .padding(24)
                        .style(style::backdrop),
                )
                .on_press(Message::CloseModal);
                stack![base, opaque(layer)].into()
            }
            None => base.into(),
        }
    }

    fn view_modal<'a>(&'a self, modal: &'a Modal) -> Element<'a, Message> {
        let title = |label: &'a str| -> Element<'a, Message> {
            row![
                text(label)
                    .size(18)
                    .font(style::BOLD)
                    .color(style::TEXT)
                    .width(Fill),
                button(icon_hover(Icon::Close, 16.0, style::MUTED, style::TEXT))
                    .padding(6)
                    .on_press(Message::CloseModal)
                    .style(|theme, status| style::icon_button(theme, status, false)),
            ]
            .align_y(iced::Center)
            .into()
        };
        let input = |label: &'a str, placeholder: &'a str| -> Element<'a, Message> {
            field(
                label,
                text_input(placeholder, &self.modal_input)
                    .on_input(Message::ModalInput)
                    .on_submit(Message::ModalSubmit)
                    .padding(10)
                    .style(style::input),
            )
        };
        let actions = |confirm: &'a str| -> Element<'a, Message> {
            row![
                space().width(Fill),
                button(text("Abbrechen").size(14))
                    .padding([8, 14])
                    .on_press(Message::CloseModal)
                    .style(style::ghost),
                button(
                    text(if self.modal_busy {
                        "Einen Moment …"
                    } else {
                        confirm
                    })
                    .size(14)
                )
                .padding([8, 16])
                .on_press_maybe((!self.modal_busy).then_some(Message::ModalSubmit))
                .style(style::primary),
            ]
            .spacing(8)
            .align_y(iced::Center)
            .into()
        };
        let hint = |body: &'a str| text(body).size(13).color(style::MUTED);

        let mut content = column![].spacing(14);
        match modal {
            Modal::CreateGuild => {
                content = content
                    .push(title("Server erstellen"))
                    .push(hint(
                        "Dein Server ist ein Ort für dich und deine Leute. Kanäle und Einladungen kannst du gleich danach anlegen.",
                    ))
                    .push(input("Servername", "Zum Beispiel „Spieleabend“"))
                    .push(actions("Erstellen"));
            }
            Modal::JoinGuild => {
                content = content
                    .push(title("Server beitreten"))
                    .push(hint("Füge den Einladungscode ein, den du bekommen hast."))
                    .push(input("Einladung", "z. B. a1B2c3D4"))
                    .push(actions("Beitreten"));
            }
            Modal::CreateChannel { voice } => {
                let switch = |label: &'a str, kind_voice: bool| {
                    let active = *voice == kind_voice;
                    button(
                        row![
                            icon(
                                if kind_voice { Icon::Volume } else { Icon::Hash },
                                16.0,
                                if active { style::TEXT } else { style::FAINT }
                            ),
                            text(label).size(14)
                        ]
                        .spacing(8)
                        .align_y(iced::Center),
                    )
                    .width(Fill)
                    .padding([10, 12])
                    .on_press(Message::OpenModal(Modal::CreateChannel {
                        voice: kind_voice,
                    }))
                    .style(move |theme, status| style::channel_button(theme, status, active))
                };
                content = content
                    .push(title("Kanal anlegen"))
                    .push(row![switch("Text", false), switch("Sprache", true)].spacing(8))
                    .push(input(
                        "Kanalname",
                        if *voice { "Lounge" } else { "neuer-kanal" },
                    ))
                    .push(actions("Anlegen"));
            }
            Modal::Invite(code) => {
                content = content.push(title("Leute einladen"));
                match code {
                    None => content = content.push(hint("Einladung wird erstellt …")),
                    Some(code) => {
                        content = content
                            .push(hint(
                                "Schick den Code an Freunde. Sie lösen ihn über „Mit Einladung beitreten“ ein.",
                            ))
                            .push(
                                row![
                                    text_input("", code)
                                        .padding(10)
                                        .size(18)
                                        .font(iced::Font::MONOSPACE)
                                        .style(style::input),
                                    button(text("Code kopieren").size(14))
                                        .padding([10, 14])
                                        .on_press(Message::CopyInvite(code.clone()))
                                        .style(style::primary),
                                ]
                                .spacing(8)
                                .align_y(iced::Center),
                            );
                    }
                }
            }
            Modal::Thread { .. } => {
                content = content
                    .push(title("Thread starten"))
                    .push(hint("Ein Thread hält eine Nebenunterhaltung zusammen, ohne den Kanal zu füllen."))
                    .push(input("Name", "Thread"))
                    .push(actions("Starten"));
            }
            Modal::Member(user_id) => content = content.push(self.view_member_card(*user_id)),
            Modal::Pins(list) => {
                content = content.push(title("Angepinnt"));
                match list {
                    None => content = content.push(hint("Wird geladen …")),
                    Some(list) if list.is_empty() => {
                        content = content.push(hint("In diesem Kanal ist nichts angepinnt."));
                    }
                    Some(list) => {
                        let manage = self.can(perm::MANAGE_MESSAGES);
                        let items = list.iter().map(|message| {
                            let mut head = row![
                                text(message.author.display_name.clone())
                                    .font(style::BOLD)
                                    .size(14)
                                    .color(style::TEXT)
                                    .width(Fill),
                            ]
                            .align_y(iced::Center);
                            if manage {
                                head = head.push(
                                    button(text("Lösen").size(12))
                                        .on_press(Message::TogglePin(message.id, false))
                                        .style(style::link),
                                );
                            }
                            container(
                                column![
                                    head,
                                    text(message.content.clone()).size(14).color(style::MUTED)
                                ]
                                .spacing(4),
                            )
                            .padding(10)
                            .width(Fill)
                            .style(style::card)
                            .into()
                        });
                        content = content
                            .push(scrollable(column(items).spacing(8)).height(Length::Shrink));
                    }
                }
            }
            Modal::Settings => content = content.push(self.view_settings()),
        }
        if let Some(err) = &self.modal_error {
            content = content.push(text(err).size(13).color(style::DANGER));
        }
        content.into()
    }

    fn view_member_card(&self, user_id: i64) -> Element<'_, Message> {
        let Some(detail) = &self.detail else {
            return space().into();
        };
        let Some(member) = detail.members.iter().find(|m| m.user_id == Some(user_id)) else {
            return text("Nicht mehr auf diesem Server.")
                .color(style::MUTED)
                .into();
        };
        let mut card = column![
            row![
                avatar(&member.display_name, 56.0),
                column![
                    text(&member.display_name)
                        .size(18)
                        .font(style::BOLD)
                        .color(style::TEXT),
                    text(format!("@{}", member.handle))
                        .size(13)
                        .color(style::MUTED),
                ]
                .spacing(2)
                .width(Fill),
                button(icon_hover(Icon::Close, 16.0, style::MUTED, style::TEXT))
                    .padding(6)
                    .on_press(Message::CloseModal)
                    .style(|theme, status| style::icon_button(theme, status, false)),
            ]
            .spacing(14)
            .align_y(iced::Center)
        ]
        .spacing(14);

        if member.user_id != Some(self.me_id()) && !member.bot {
            card = card.push(
                button(
                    row![
                        icon(Icon::Message, 16.0, style::ON_ACCENT),
                        text("Nachricht senden").size(14).color(style::ON_ACCENT),
                    ]
                    .spacing(8)
                    .align_y(iced::Center),
                )
                .width(Fill)
                .padding([8, 12])
                .on_press(Message::DmWith(member.handle.clone()))
                .style(style::primary),
            );
        }

        if let Some(until) = &member.timeout_until {
            let local = chrono::DateTime::parse_from_rfc3339(until)
                .map(|t| {
                    t.with_timezone(&chrono::Local)
                        .format("%d.%m. %H:%M")
                        .to_string()
                })
                .unwrap_or_else(|_| until.clone());
            card = card.push(
                text(format!("Im Timeout bis {local}"))
                    .size(13)
                    .color(style::DANGER),
            );
        }

        // Rollen: anzeigen, mit MANAGE_ROLES umschaltbar.
        let roles: Vec<&api::Role> = detail.roles.iter().filter(|r| !r.is_everyone).collect();
        if !roles.is_empty() {
            let manage = detail.can(perm::MANAGE_ROLES);
            let pills = roles.into_iter().filter_map(|role| {
                let has = member.role_ids.contains(&role.id);
                if !has && !manage {
                    return None;
                }
                let role_id = role.id;
                // Vergeben: Akzent mit Haken; nicht vergeben: „+ Name“ gedämpft.
                let label = if has {
                    format!("✓ {}", role.name)
                } else {
                    format!("+ {}", role.name)
                };
                Some(
                    button(text(label).size(12).color(if has {
                        style::TEXT
                    } else {
                        style::FAINT
                    }))
                    .padding([4, 10])
                    .on_press_maybe(manage.then_some(Message::ToggleRole(user_id, role_id, !has)))
                    .style(move |theme, status| style::reaction(theme, status, has))
                    .into(),
                )
            });
            card = card
                .push(modal_label("Rollen"))
                .push(row(pills).spacing(6).wrap());
        }

        if user_id == self.me_id() {
            return card.into();
        }
        let mut moderation = column![].spacing(8);
        if detail.can(perm::TIMEOUT_MEMBERS) {
            let busy = self.modal_busy;
            let pick = |label: &'static str, seconds: i64| {
                button(text(label).size(13))
                    .padding([6, 10])
                    .on_press_maybe((!busy).then_some(Message::Timeout(user_id, seconds)))
                    .style(style::ghost)
            };
            let mut line = row![
                text("Timeout").size(13).color(style::MUTED).width(Fill),
                pick("10 min", 600),
                pick("1 h", 3600),
                pick("1 Tag", 86_400),
            ]
            .spacing(4)
            .align_y(iced::Center);
            if member.timeout_until.is_some() {
                line = line.push(pick("Aufheben", 0));
            }
            moderation = moderation.push(line);
        }
        let mut buttons = row![space().width(Fill)].spacing(8);
        if detail.can(perm::KICK_MEMBERS) {
            buttons = buttons.push(
                button(text("Kicken").size(13))
                    .padding([8, 14])
                    .on_press(Message::AskConfirm(Some(ModAction::Kick)))
                    .style(style::ghost),
            );
        }
        if detail.can(perm::BAN_MEMBERS) {
            buttons = buttons.push(
                button(text("Bannen").size(13).color(style::ON_ACCENT))
                    .padding([8, 14])
                    .on_press(Message::AskConfirm(Some(ModAction::Ban)))
                    .style(style::danger_button),
            );
        }
        moderation = moderation.push(buttons);
        if let Some(action) = self.confirm {
            let (question, label) = match action {
                ModAction::Kick => (
                    format!(
                        "{} vom Server entfernen? Mit einer neuen Einladung kann die Person zurückkommen.",
                        member.display_name
                    ),
                    "Ja, kicken",
                ),
                ModAction::Ban => (
                    format!(
                        "{} dauerhaft bannen? Die Person kann nicht wieder beitreten, bis der Bann aufgehoben wird.",
                        member.display_name
                    ),
                    "Ja, bannen",
                ),
            };
            moderation = moderation.push(
                container(
                    column![
                        text(question).size(13),
                        row![
                            space().width(Fill),
                            button(text("Abbrechen").size(13))
                                .on_press(Message::AskConfirm(None))
                                .style(style::ghost),
                            button(text(label).size(13).color(style::ON_ACCENT))
                                .padding([6, 12])
                                .on_press(Message::Moderate(user_id, action))
                                .style(style::danger_button),
                        ]
                        .spacing(8)
                        .align_y(iced::Center),
                    ]
                    .spacing(8),
                )
                .padding(12)
                .style(style::danger_note),
            );
        }
        if detail.can(perm::TIMEOUT_MEMBERS)
            || detail.can(perm::KICK_MEMBERS)
            || detail.can(perm::BAN_MEMBERS)
        {
            card = card.push(modal_label("Moderation")).push(moderation);
        }
        card.into()
    }

    fn view_settings(&self) -> Element<'_, Message> {
        // Auswahlliste statt Aufklappmenü: Overlays in Dialogen zeichnet
        // tiny-skia derzeit fehlerhaft.
        let choices = |names: &[String],
                       current: &Option<String>,
                       on_pick: fn(String) -> Message|
         -> Element<'_, Message> {
            let current = current.as_deref().unwrap_or(DEFAULT_DEVICE);
            let options = std::iter::once(DEFAULT_DEVICE.to_owned()).chain(names.iter().cloned());
            column(options.map(|name| {
                let active = name == current;
                button(
                    row![
                        container(space())
                            .width(8)
                            .height(8)
                            .style(move |_: &Theme| style::status_dot(
                                if active { style::ACCENT } else { style::RAISED },
                                style::SIDEBAR
                            )),
                        text(name.clone())
                            .size(13)
                            .color(if active { style::TEXT } else { style::MUTED })
                            .wrapping(text::Wrapping::None),
                    ]
                    .spacing(10)
                    .align_y(iced::Center),
                )
                .width(Fill)
                .padding([6, 10])
                .on_press(on_pick(name))
                .style(move |theme, status| style::channel_button(theme, status, active))
                .into()
            }))
            .spacing(2)
            .into()
        };
        let key = if self.capturing_key {
            "Drück jetzt eine Taste …".to_owned()
        } else {
            self.prefs
                .ptt_key
                .as_deref()
                .map(prefs::key_label)
                .unwrap_or_else(|| "Keine Taste".to_owned())
        };
        column![
            row![
                text("Einstellungen")
                    .size(18)
                    .font(style::BOLD)
                    .color(style::TEXT)
                    .width(Fill),
                button(icon_hover(Icon::Close, 16.0, style::MUTED, style::TEXT))
                    .padding(6)
                    .on_press(Message::CloseModal)
                    .style(|theme, status| style::icon_button(theme, status, false)),
            ]
            .align_y(iced::Center),
            modal_label("Profil"),
            field(
                "Anzeigename",
                row![
                    text_input("Dein Name", &self.modal_input)
                        .on_input(Message::ModalInput)
                        .on_submit(Message::ModalSubmit)
                        .padding(10)
                        .style(style::input),
                    button(text("Speichern").size(13))
                        .padding([10, 14])
                        .on_press_maybe((!self.modal_busy).then_some(Message::ModalSubmit))
                        .style(style::primary),
                ]
                .spacing(8)
                .align_y(iced::Center),
            ),
            modal_label("Sprache"),
            field(
                "Mikrofon",
                choices(&self.devices.0, &self.prefs.input, Message::SetInput),
            ),
            field(
                "Lautsprecher",
                choices(&self.devices.1, &self.prefs.output, Message::SetOutput),
            ),
            text("Gerätewechsel gilt ab dem nächsten Beitritt.")
                .size(12)
                .color(style::FAINT),
            toggler(self.prefs.push_to_talk)
                .label("Push-to-Talk")
                .on_toggle(Message::SetPushToTalk),
            row![
                text(format!("Taste: {key}"))
                    .size(13)
                    .color(style::MUTED)
                    .width(Fill),
                button(text("Taste festlegen").size(13))
                    .padding([6, 12])
                    .on_press(Message::CaptureKey)
                    .style(style::ghost),
            ]
            .align_y(iced::Center),
            text("Gilt, solange das Nova-Fenster im Vordergrund ist. Esc bricht ab.")
                .size(12)
                .color(style::FAINT),
            modal_label("APP"),
            row![
                container(self.view_update_button()).width(Fill),
                button(text("Abmelden").size(13).color(style::DANGER))
                    .on_press(Message::Logout)
                    .style(style::ghost),
            ]
            .align_y(iced::Center),
        ]
        .spacing(10)
        .into()
    }

    fn view_rail(&self) -> Element<'_, Message> {
        let current = self
            .detail
            .as_ref()
            .map(|d| d.guild.id)
            .filter(|_| !self.home);
        let guilds = self.guilds.iter().map(|guild| {
            let active = current == Some(guild.id);
            tip(
                button(
                    text(initials(&guild.name))
                        .size(15)
                        .font(style::BOLD)
                        .center()
                        .width(Fill)
                        .height(Fill)
                        .color(style::TEXT),
                )
                .width(44)
                .height(44)
                .padding(0)
                .on_press(Message::SelectGuild(guild.id))
                .style(move |theme, status| style::rail_button(theme, status, active)),
                guild.name.as_str(),
                tooltip::Position::Right,
            )
        });
        // Die Bildmarke ist bewusst keine Kachel: kein Hintergrund, kein Klick,
        // darunter ein Trenner — Server beginnen erst danach.
        let rail_action = |glyph: Icon, label: &'static str, modal: Modal| {
            tip(
                button(container(icon_hover(glyph, 20.0, style::MUTED, style::TEXT)).center(Fill))
                    .width(44)
                    .height(44)
                    .padding(0)
                    .on_press(Message::OpenModal(modal))
                    .style(|theme, status| style::rail_button(theme, status, false)),
                label,
                tooltip::Position::Right,
            )
        };
        container(
            column![
                container(icon(Icon::Logo, 26.0, style::TEXT)).center(44),
                self.view_dm_button(),
                container(space()).width(32).height(2).style(style::divider),
                scrollable(column(guilds).spacing(8)).height(Length::Shrink),
                rail_action(Icon::Plus, "Server erstellen", Modal::CreateGuild),
                rail_action(Icon::UserPlus, "Mit Einladung beitreten", Modal::JoinGuild),
            ]
            .spacing(10)
            .align_x(iced::Center),
        )
        .width(72)
        .height(Fill)
        .padding([12, 0])
        .style(style::rail)
        .into()
    }

    /// Eigene Leiste unten links (Name, Status, Einstellungen, Abmelden).
    fn view_me_bar(&self) -> Element<'_, Message> {
        let me = self.me.as_ref();
        let name = me.map(|m| m.display_name.as_str()).unwrap_or("?");
        let (online, label) = match self.status {
            Status::Online => (true, "Online"),
            Status::Connecting => (false, "Verbinde…"),
            Status::Offline => (false, "Offline"),
        };
        container(
            row![
                avatar_with_status(name, 32.0, online, style::RAIL),
                column![
                    text(name).size(14).font(style::BOLD).color(style::TEXT),
                    text(label).size(12).color(style::MUTED),
                ]
                .spacing(1)
                .width(Fill),
                self.view_update_icon(),
                tip(
                    button(icon_hover(Icon::Settings, 18.0, style::MUTED, style::TEXT))
                        .padding(7)
                        .on_press(Message::OpenModal(Modal::Settings))
                        .style(|theme, status| style::icon_button(theme, status, false)),
                    "Einstellungen",
                    tooltip::Position::Top,
                ),
                tip(
                    button(icon_hover(Icon::LogOut, 18.0, style::MUTED, style::DANGER))
                        .padding(7)
                        .on_press(Message::Logout)
                        .style(style::danger_icon_button),
                    "Abmelden",
                    tooltip::Position::Top,
                ),
            ]
            .spacing(4)
            .align_y(iced::Center),
        )
        .padding(iced::Padding {
            top: 8.0,
            right: 6.0,
            bottom: 8.0,
            left: 10.0,
        })
        .style(style::rail)
        .into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        if self.home {
            return self.view_dm_sidebar();
        }
        let title = self
            .detail
            .as_ref()
            .map(|d| d.guild.name.as_str())
            .unwrap_or("Nova");
        let mut channels = column![].spacing(2);
        if let Some(detail) = &self.detail {
            let manage = detail.can(perm::MANAGE_CHANNELS);
            for (label, kind) in [("Textkanäle", 0), ("Sprachkanäle", 1)] {
                let mut head =
                    row![container(section_label(label)).width(Fill)].align_y(iced::Center);
                if manage {
                    head = head.push(tip(
                        button(icon_hover(Icon::Plus, 14.0, style::FAINT, style::TEXT))
                            .padding([4, 6])
                            .on_press(Message::OpenModal(Modal::CreateChannel {
                                voice: kind == 1,
                            }))
                            .style(|theme, status| style::icon_button(theme, status, false)),
                        if kind == 1 {
                            "Sprachkanal anlegen"
                        } else {
                            "Textkanal anlegen"
                        },
                        tooltip::Position::Top,
                    ));
                }
                channels = channels.push(head);
                for channel in detail.channels.iter().filter(|c| c.kind == kind) {
                    channels = channels.push(self.channel_entry(channel, 0.0));
                    // Aktive Threads direkt unter ihrem Kanal.
                    for thread in detail
                        .channels
                        .iter()
                        .filter(|t| t.kind == 2 && !t.archived && t.parent_id == Some(channel.id))
                    {
                        channels = channels.push(self.channel_entry(thread, 18.0));
                    }
                    // Sprachkanal: Avatar-Reihe (wer da ist, wer redet), darunter die Namen.
                    if kind == 1 {
                        let people = self
                            .voice_rooms
                            .get(&channel.id)
                            .map(Vec::as_slice)
                            .unwrap_or_default();
                        let talking = self.talking_in(channel.id);
                        if !people.is_empty() {
                            // Max. 6 Gesichter, je 4 px überlappend, dann „+n“.
                            let mut faces = iced::widget::Stack::new();
                            for (index, person) in people.iter().take(6).enumerate() {
                                faces = faces.push(
                                    container(voice_face(
                                        &person.display_name,
                                        talking.contains(&person.user_id),
                                    ))
                                    .padding(iced::Padding {
                                        left: index as f32 * 20.0,
                                        ..iced::Padding::ZERO
                                    }),
                                );
                            }
                            let mut line = row![space().width(34), faces]
                                .spacing(6)
                                .padding([4, 8])
                                .align_y(iced::Center);
                            if people.len() > 6 {
                                line = line.push(
                                    text(format!("+{}", people.len() - 6))
                                        .size(12)
                                        .color(style::FAINT),
                                );
                            }
                            channels = channels.push(line);
                        }
                        for person in people {
                            let speaking = talking.contains(&person.user_id);
                            channels = channels.push(
                                row![
                                    space().width(34),
                                    text(person.display_name.clone())
                                        .size(13)
                                        .color(if speaking { style::TEXT } else { style::MUTED }),
                                    live_tag(person.screen),
                                ]
                                .spacing(8)
                                .padding([1, 8])
                                .align_y(iced::Center),
                            );
                        }
                    }
                }
            }
        } else {
            channels = channels.push(
                container(
                    column![
                        text("Noch kein Server")
                            .font(style::BOLD)
                            .color(style::TEXT),
                        text("Leg einen eigenen Server an oder tritt mit einer Einladung bei.")
                            .size(13)
                            .color(style::MUTED),
                        space().height(6),
                        button(text("Server erstellen").size(13).center().width(Fill))
                            .width(Fill)
                            .padding([8, 12])
                            .on_press(Message::OpenModal(Modal::CreateGuild))
                            .style(style::primary),
                        button(text("Einladung einlösen").size(13).center().width(Fill))
                            .width(Fill)
                            .padding([8, 12])
                            .on_press(Message::OpenModal(Modal::JoinGuild))
                            .style(style::ghost),
                    ]
                    .spacing(6),
                )
                .padding([16, 8]),
            );
        }

        let me_bar = self.view_me_bar();

        let mut head = row![
            text(title)
                .size(15)
                .font(style::BOLD)
                .color(style::TEXT)
                .width(Fill)
        ]
        .align_y(iced::Center);
        if self.can(perm::CREATE_INVITE) {
            head = head.push(tip(
                button(icon_hover(Icon::UserPlus, 18.0, style::MUTED, style::TEXT))
                    .padding(6)
                    .on_press(Message::OpenModal(Modal::Invite(None)))
                    .style(|theme, status| style::icon_button(theme, status, false)),
                "Leute einladen",
                tooltip::Position::Bottom,
            ));
        }
        container(column![
            header_bar(head),
            scrollable(channels.padding([0, 8])).height(Fill),
            hline(),
            me_bar,
        ])
        .width(240)
        .height(Fill)
        .style(style::sidebar)
        .into()
    }

    /// Ein Eintrag der Kanalliste; `indent` > 0 für Threads.
    fn channel_entry<'a>(&'a self, channel: &'a Channel, indent: f32) -> Element<'a, Message> {
        let active = self.channel == Some(channel.id);
        let unread = !active && self.unread.contains(&channel.id);
        let glyph = match channel.kind {
            1 => Icon::Volume,
            2 => Icon::Thread,
            _ => Icon::Hash,
        };
        let name_color = if active || unread {
            style::TEXT
        } else {
            style::MUTED
        };
        let mut name = text(&channel.name)
            .color(name_color)
            .width(Fill)
            .wrapping(text::Wrapping::None);
        if unread {
            name = name.font(style::BOLD);
        }
        let mut line = row![
            icon(
                glyph,
                if indent > 0.0 { 14.0 } else { 16.0 },
                if active { style::MUTED } else { style::FAINT }
            ),
            name,
        ]
        .spacing(8)
        .align_y(iced::Center);
        if unread {
            line = line.push(
                container(space())
                    .width(8)
                    .height(8)
                    .style(style::unread_dot),
            );
        }
        // Ausgewählt: Akzent-Fläche plus 2-px-Streifen links.
        let stripe: Element<'a, Message> = if active {
            container(
                container(space())
                    .width(2)
                    .height(Fill)
                    .style(style::accent_stripe),
            )
            .padding([6, 0])
            .height(Fill)
            .into()
        } else {
            space().width(2).into()
        };
        row![
            space().width(indent),
            stripe,
            button(line)
                .width(Fill)
                .padding([if indent > 0.0 { 5 } else { 7 }, 8])
                .on_press(Message::OpenChannel(channel.id))
                .style(move |theme, status| style::channel_button(theme, status, active)),
        ]
        .height(Length::Shrink)
        .into()
    }

    /// Wer in diesem Sprachkanal gerade spricht; bekannt nur, wenn man selbst drin ist.
    fn talking_in(&self, channel_id: i64) -> Vec<i64> {
        if self.voice.as_ref().map(|v| v.channel_id) != Some(channel_id) {
            return Vec::new();
        }
        let me = self.me_id();
        self.voice_status
            .speaking
            .iter()
            .map(|id| if *id == -1 { me } else { *id })
            .collect()
    }

    fn view_voice<'a>(&'a self, channel: &'a Channel) -> Element<'a, Message> {
        let here = self.voice.as_ref().map(|v| v.channel_id) == Some(channel.id);
        let joining = self.voice_joining == Some(channel.id);
        let me = self.me.as_ref().map(|m| m.id);
        let status = &self.voice_status;
        let speaking: Vec<i64> = status
            .speaking
            .iter()
            .map(|id| if *id == -1 { me.unwrap_or(-1) } else { *id })
            .collect();
        let people = self
            .voice_rooms
            .get(&channel.id)
            .cloned()
            .unwrap_or_default();

        let stage = if here { self.view_stage(false) } else { None };
        // Mit Bühne werden die Kacheln klein und stehen darunter.
        let (tile_w, tile_h, face) = if stage.is_some() {
            (150.0, 96.0, 40.0)
        } else {
            (180.0, 132.0, 64.0)
        };
        let me_id = self.me_id();
        let tiles = people.into_iter().map(|person| {
            let talking = here && speaking.contains(&person.user_id);
            let muted = person.muted || person.deafened;
            let mine = person.user_id == me_id;
            let picture = self
                .voice
                .as_ref()
                .filter(|_| here)
                .and_then(|v| v.picture(person.user_id));
            let live = picture.is_some() || person.screen || (mine && self.sharing);
            let mut name_row = row![
                text(person.display_name.clone())
                    .size(13)
                    .font(style::BOLD)
                    .color(if talking {
                        style::ON_ACCENT
                    } else {
                        style::TEXT
                    })
                    .wrapping(text::Wrapping::None)
            ]
            .spacing(6)
            .align_y(iced::Center);
            if live {
                name_row = name_row.push(live_tag(true));
            }
            if muted {
                name_row = name_row.push(icon(
                    if person.deafened {
                        Icon::HeadphonesOff
                    } else {
                        Icon::MicOff
                    },
                    14.0,
                    style::DANGER,
                ));
            }
            // Namensbalken füllt sich gelb, solange die Person spricht.
            let name_row = container(name_row)
                .padding([2, 6])
                .style(move |_: &Theme| style::name_bar(talking));
            // Wer streamt, zeigt sein Bild statt des Avatars.
            let face: Element<'a, Message> = match &picture {
                Some(picture) => iced::widget::image(picture.handle.clone())
                    .content_fit(iced::ContentFit::Cover)
                    .width(Fill)
                    .height(Fill)
                    .into(),
                None => avatar(&person.display_name, face),
            };
            let content: Element<'a, Message> = if picture.is_some() {
                column![container(face).height(Fill).width(Fill), name_row]
                    .spacing(6)
                    .padding(6)
                    .align_x(iced::Center)
                    .into()
            } else {
                column![face, name_row]
                    .spacing(10)
                    .align_x(iced::Center)
                    .into()
            };
            let tile = container(content)
                .center_x(tile_w)
                .center_y(tile_h)
                .style(move |_: &Theme| style::voice_tile(talking));
            if picture.is_some() {
                let user = person.user_id;
                tip(
                    button(tile)
                        .padding(0)
                        .on_press(Message::Watch(Some(user)))
                        .style(|theme, status| style::channel_button(theme, status, false)),
                    "Ansehen",
                    tooltip::Position::Top,
                )
            } else {
                tile.into()
            }
        });
        let tiles_row = iced::widget::row(tiles).spacing(12);
        // Unter der Bühne eine Reihe (waagrecht scrollbar), sonst umbrechen.
        let grid: Element<'a, Message> = if stage.is_some() {
            tiles_row.into()
        } else {
            tiles_row.wrap().into()
        };

        let sharing = self.sharing;
        let controls: Element<'_, Message> = if here {
            let state = if !status.connected {
                text("Verbinde …").color(style::MUTED)
            } else if sharing && status.sharing {
                let size = status
                    .share_size
                    .map(|(w, h)| format!("{w}×{h}, "))
                    .unwrap_or_default();
                text(format!(
                    "Du teilst deinen Bildschirm ({size}{} fps)",
                    status.share_fps
                ))
                .color(style::ONLINE)
            } else if sharing {
                text("Bildschirm wird vorbereitet …").color(style::MUTED)
            } else {
                text("Verbunden").color(style::ONLINE)
            };
            let mic_on = !self.voice_muted;
            let ears_on = !self.voice_deafened;
            row![
                state.size(13),
                space().width(Fill),
                tip(
                    button(icon(
                        if mic_on { Icon::Mic } else { Icon::MicOff },
                        20.0,
                        if mic_on { style::TEXT } else { style::DANGER }
                    ))
                    .padding(10)
                    .on_press(Message::ToggleMute)
                    .style(move |theme, status| style::icon_button(theme, status, !mic_on)),
                    if mic_on {
                        "Stummschalten"
                    } else {
                        "Mikrofon einschalten"
                    },
                    tooltip::Position::Top,
                ),
                tip(
                    button(icon(
                        if ears_on {
                            Icon::Headphones
                        } else {
                            Icon::HeadphonesOff
                        },
                        20.0,
                        if ears_on { style::TEXT } else { style::DANGER }
                    ))
                    .padding(10)
                    .on_press(Message::ToggleDeafen)
                    .style(move |theme, status| style::icon_button(theme, status, !ears_on)),
                    if ears_on {
                        "Taub schalten"
                    } else {
                        "Ton einschalten"
                    },
                    tooltip::Position::Top,
                ),
                tip(
                    button(icon(
                        Icon::Monitor,
                        20.0,
                        if self.sharing {
                            style::ACCENT
                        } else {
                            style::TEXT
                        }
                    ))
                    .padding(10)
                    .on_press(Message::ToggleShare)
                    .style(move |theme, status| style::icon_button(theme, status, sharing)),
                    if self.sharing {
                        "Teilen beenden"
                    } else {
                        "Bildschirm teilen"
                    },
                    tooltip::Position::Top,
                ),
                tip(
                    button(icon(Icon::PhoneOff, 20.0, style::ON_ACCENT))
                        .padding(10)
                        .on_press(Message::LeaveVoice)
                        .style(style::danger_button),
                    "Auflegen",
                    tooltip::Position::Top,
                ),
            ]
            .spacing(8)
            .align_y(iced::Center)
            .into()
        } else {
            row![
                text(if joining {
                    "Verbinde …"
                } else {
                    "Noch nicht verbunden"
                })
                .size(13)
                .color(style::MUTED),
                space().width(Fill),
                button(
                    row![
                        icon(Icon::Volume, 16.0, style::ON_ACCENT),
                        text("Beitreten")
                    ]
                    .spacing(8)
                    .align_y(iced::Center)
                )
                .padding([10, 16])
                .on_press_maybe((!joining).then_some(Message::JoinVoice(channel.id)))
                .style(style::primary),
            ]
            .align_y(iced::Center)
            .into()
        };

        let mut body = column![].spacing(16).padding(24);
        if self.voice_rooms.get(&channel.id).is_none_or(Vec::is_empty) {
            body = body.push(
                text("Noch niemand hier – tritt bei, die anderen sehen dich sofort.")
                    .color(style::MUTED),
            );
        }
        let someone_live = self
            .voice_rooms
            .get(&channel.id)
            .is_some_and(|people| people.iter().any(|p| p.screen));
        if !here && someone_live {
            body = body.push(
                text("Hier wird gestreamt – tritt bei, um zuzusehen.")
                    .size(13)
                    .color(style::MUTED),
            );
        }
        body = body.push(grid);
        if here && let Some(warning) = &status.error {
            body = body.push(text(warning.clone()).size(13).color(style::DANGER));
        }
        if here {
            body = body.push(
                text(format!(
                    "{} Pakete gesendet, {} empfangen",
                    status.packets_out, status.packets_in
                ))
                .size(11)
                .color(style::FAINT),
            );
        }
        let main: Element<'a, Message> = match stage {
            // Bühne oben, Kacheln klein darunter.
            Some(stage) => column![
                container(stage)
                    .width(Fill)
                    .height(Fill)
                    .padding(iced::Padding {
                        top: 16.0,
                        right: 16.0,
                        bottom: 0.0,
                        left: 16.0,
                    }),
                scrollable(body)
                    .direction(scrollable::Direction::Horizontal(
                        scrollable::Scrollbar::new().width(4).scroller_width(4),
                    ))
                    .height(Length::Shrink),
            ]
            .height(Fill)
            .into(),
            None => scrollable(body).height(Fill).into(),
        };
        column![
            main,
            container(controls)
                .padding([12, 24])
                .width(Fill)
                .style(style::voice_bar),
        ]
        .height(Fill)
        .into()
    }

    /// Angeschauter Stream mit Name, Auflösung und Knöpfen. `None`, solange
    /// kein Stream ausgewählt ist oder noch kein Bild da ist.
    fn view_stage(&self, theater: bool) -> Option<Element<'_, Message>> {
        let user = self.stage?;
        let picture = self.voice.as_ref()?.picture(user)?;
        let name = self
            .voice_rooms
            .values()
            .flatten()
            .find(|p| p.user_id == user)
            .map(|p| p.display_name.clone())
            .unwrap_or_default();
        let tool = |glyph: Icon, label: &'static str, msg: Message| -> Element<'_, Message> {
            tip(
                button(icon_hover(glyph, 18.0, style::MUTED, style::TEXT))
                    .padding(8)
                    .on_press(msg)
                    .style(|theme, status| style::icon_button(theme, status, false)),
                label,
                tooltip::Position::Bottom,
            )
        };
        let bar = container(
            row![
                live_tag(true),
                text(name).size(14).font(style::BOLD).color(style::TEXT),
                text(format!("{}×{}", picture.width, picture.height))
                    .size(12)
                    .color(style::MUTED),
                space().width(Fill),
                tool(
                    if theater {
                        Icon::Minimize
                    } else {
                        Icon::Maximize
                    },
                    if theater {
                        "Vollbild verlassen (Esc)"
                    } else {
                        "Vollbild"
                    },
                    Message::ToggleTheater,
                ),
                tool(Icon::Close, "Nicht mehr ansehen", Message::Watch(None)),
            ]
            .spacing(10)
            .align_y(iced::Center),
        )
        .padding([6, 10])
        .style(style::stage_bar);
        let video = container(
            iced::widget::image(picture.handle)
                .content_fit(iced::ContentFit::Contain)
                .width(Fill)
                .height(Fill),
        )
        .width(Fill)
        .height(Fill)
        .style(style::stage);
        Some(column![bar, video].into())
    }

    /// Knopf „Direktnachrichten“ ganz oben in der Serverleiste.
    fn view_dm_button(&self) -> Element<'_, Message> {
        let active = self.home;
        let unread = !self.dm_unread.is_empty();
        let glyph = icon_hover(
            Icon::Message,
            20.0,
            if active { style::TEXT } else { style::MUTED },
            style::TEXT,
        );
        let mut face = stack![container(glyph).center(44)];
        if unread && !active {
            face = face.push(
                container(
                    container(space())
                        .width(10)
                        .height(10)
                        .style(style::unread_dot),
                )
                .width(44)
                .height(44)
                .align_x(iced::alignment::Horizontal::Right)
                .padding(4),
            );
        }
        tip(
            button(face)
                .width(44)
                .height(44)
                .padding(0)
                .on_press(Message::OpenHome)
                .style(move |theme, status| style::rail_button(theme, status, active)),
            "Direktnachrichten",
            tooltip::Position::Right,
        )
    }

    /// Seitenleiste im DM-Modus: Benutzername eingeben, Unterhaltungen.
    fn view_dm_sidebar(&self) -> Element<'_, Message> {
        let can_start = !self.dm_busy && !self.dm_input.trim().is_empty();
        let start = row![
            text_input("Benutzername", &self.dm_input)
                .on_input(Message::DmInput)
                .on_submit(Message::StartDm)
                .size(13)
                .padding([8, 10])
                .style(style::input),
            tip(
                button(icon(
                    Icon::Send,
                    16.0,
                    if can_start {
                        style::ON_ACCENT
                    } else {
                        style::FAINT
                    }
                ))
                .padding(8)
                .on_press_maybe(can_start.then_some(Message::StartDm))
                .style(style::send_button),
                "Unterhaltung starten",
                tooltip::Position::Top,
            ),
        ]
        .spacing(6)
        .align_y(iced::Center);
        let mut top = column![
            text("Schreib jemandem über den Benutzernamen.")
                .size(12)
                .color(style::MUTED),
            start
        ]
        .spacing(6);
        if let Some(err) = &self.dm_error {
            top = top.push(text(err).size(12).color(style::DANGER));
        }

        let mut list = column![section_label("Unterhaltungen")].spacing(2);
        if self.dms.is_empty() {
            list = list.push(
                container(
                    text("Noch keine Unterhaltungen.")
                        .size(13)
                        .color(style::MUTED),
                )
                .padding([4, 8]),
            );
        }
        for dm in &self.dms {
            let active = self.dm == Some(dm.id);
            let unread = self.dm_unread.contains(&dm.id);
            let mut name = text(&dm.user.display_name)
                .size(14)
                .color(if active || unread {
                    style::TEXT
                } else {
                    style::MUTED
                })
                .wrapping(text::Wrapping::None);
            if unread {
                name = name.font(style::BOLD);
            }
            let mut line = row![
                avatar(&dm.user.display_name, 28.0),
                column![
                    name,
                    text(format!("@{}", dm.user.handle))
                        .size(11)
                        .color(style::FAINT)
                        .wrapping(text::Wrapping::None),
                ]
                .width(Fill),
            ]
            .spacing(10)
            .align_y(iced::Center);
            if unread {
                line = line.push(
                    container(space())
                        .width(8)
                        .height(8)
                        .style(style::unread_dot),
                );
            }
            list = list.push(
                button(line)
                    .width(Fill)
                    .padding([5, 8])
                    .on_press(Message::OpenDm(dm.id))
                    .style(move |theme, status| style::channel_button(theme, status, active)),
            );
        }

        container(column![
            header_bar(
                text("Direktnachrichten")
                    .size(15)
                    .font(style::BOLD)
                    .color(style::TEXT)
            ),
            container(top).padding([8, 12]),
            scrollable(list.padding([0, 8])).height(Fill),
            hline(),
            self.view_me_bar(),
        ])
        .width(240)
        .height(Fill)
        .style(style::sidebar)
        .into()
    }

    /// Hauptbereich im DM-Modus: Verlauf mit einer Person.
    fn view_dm_main(&self) -> Element<'_, Message> {
        let peer = self.dm.and_then(|id| self.dms.iter().find(|d| d.id == id));
        let Some(peer) = peer else {
            let header = header_bar(
                text("Direktnachrichten")
                    .size(15)
                    .font(style::BOLD)
                    .color(style::TEXT),
            );
            let body = empty_state(
                Icon::Message,
                "Privat schreiben".to_owned(),
                "Gib links einen Benutzernamen ein, um eine Unterhaltung zu starten.",
            );
            return column![header, body].into();
        };
        let header = header_bar(
            row![
                avatar(&peer.user.display_name, 24.0),
                text(&peer.user.display_name)
                    .size(15)
                    .font(style::BOLD)
                    .color(style::TEXT),
                text(format!("@{}", peer.user.handle))
                    .size(13)
                    .color(style::MUTED),
            ]
            .spacing(8)
            .align_y(iced::Center),
        );
        let list = self.view_history(
            peer.id,
            false,
            format!("Deine Unterhaltung mit {}", peer.user.display_name),
            "Nur ihr beide könnt diese Nachrichten sehen.",
        );
        let composer = self.view_composer(format!("Nachricht an @{}", peer.user.handle), true);
        column![header, list, space().height(18), composer]
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn view_main(&self) -> Element<'_, Message> {
        if self.home {
            return self.view_dm_main();
        }
        let channel = self.current_channel();
        let is_voice = channel.map(|c| c.kind == 1).unwrap_or(false);
        let is_thread = channel.map(|c| c.kind == 2).unwrap_or(false);

        let mut title = row![
            icon(
                if is_voice {
                    Icon::Volume
                } else if is_thread {
                    Icon::Thread
                } else {
                    Icon::Hash
                },
                20.0,
                style::FAINT
            ),
            text(channel.map(|c| c.name.as_str()).unwrap_or("Nova"))
                .size(15)
                .font(style::BOLD)
                .color(style::TEXT),
        ]
        .spacing(8)
        .align_y(iced::Center);
        if let Some(topic) = channel.and_then(|c| c.topic.as_deref())
            && !topic.is_empty()
        {
            title = title
                .push(space().width(4))
                .push(vline(20))
                .push(space().width(4))
                .push(text(topic).size(13).color(style::MUTED));
        }
        let show = self.show_members;
        let mut tools = row![].spacing(4).align_y(iced::Center);
        if let Some(channel) = channel.filter(|c| c.kind != 1) {
            let can_archive = channel.kind == 2
                && (channel.owner_id == Some(self.me_id()) || self.can(perm::MANAGE_THREADS));
            if can_archive {
                tools = tools.push(tip(
                    button(icon_hover(Icon::Archive, 18.0, style::MUTED, style::TEXT))
                        .padding(7)
                        .on_press(Message::ArchiveThread(channel.id))
                        .style(|theme, status| style::icon_button(theme, status, false)),
                    "Thread archivieren",
                    tooltip::Position::Bottom,
                ));
            }
            tools = tools.push(tip(
                button(icon_hover(Icon::Pin, 18.0, style::MUTED, style::TEXT))
                    .padding(7)
                    .on_press(Message::OpenModal(Modal::Pins(None)))
                    .style(|theme, status| style::icon_button(theme, status, false)),
                "Angepinnte Nachrichten",
                tooltip::Position::Bottom,
            ));
        }
        let header = header_bar(
            row![
                container(title).width(Fill),
                tools,
                tip(
                    button(icon(
                        Icon::Users,
                        18.0,
                        if show { style::TEXT } else { style::MUTED }
                    ))
                    .padding(7)
                    .on_press(Message::ToggleMembers)
                    .style(move |theme, status| style::icon_button(theme, status, show)),
                    if show {
                        "Mitglieder ausblenden"
                    } else {
                        "Mitglieder einblenden"
                    },
                    tooltip::Position::Bottom,
                ),
            ]
            .align_y(iced::Center),
        );

        let Some(channel) = channel else {
            let body = empty_state(
                Icon::Hash,
                "Kein Kanal ausgewählt".to_owned(),
                "Wähle links einen Kanal, um loszulegen.",
            );
            return column![header, body].into();
        };
        if is_voice {
            return column![header, self.view_voice(channel)].into();
        }

        let list = self.view_history(
            channel.id,
            is_thread,
            format!("Willkommen in #{}", channel.name),
            "Hier ist es noch still. Schreib die erste Nachricht.",
        );

        let online = self.status == Status::Online;
        let placeholder = if online && channel.slowmode_secs > 0 {
            format!(
                "Nachricht an #{} (Slowmode: {} s)",
                channel.name, channel.slowmode_secs
            )
        } else if online {
            format!("Nachricht an #{}", channel.name)
        } else {
            "Verbinde mit dem Gateway…".to_owned()
        };
        let composer = self.view_composer(placeholder, online);

        // „… schreibt“ – feste Höhe, damit der Composer nicht springt.
        let typing_names: Vec<&str> = self
            .typing
            .get(&channel.id)
            .map(|people| people.values().map(|(name, _)| name.as_str()).collect())
            .unwrap_or_default();
        let typing_text = match typing_names.as_slice() {
            [] => String::new(),
            [one] => format!("{one} schreibt …"),
            [a, b] => format!("{a} und {b} schreiben …"),
            _ => "Mehrere schreiben …".to_owned(),
        };
        let typing = container(text(typing_text).size(12).color(style::MUTED))
            .height(18)
            .padding([0, 20]);

        column![header, list, typing, composer]
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// Verlauf eines Kanals oder einer Unterhaltung (`key`), unten verankert.
    fn view_history<'a>(
        &'a self,
        key: i64,
        is_thread: bool,
        welcome: String,
        welcome_body: &'a str,
    ) -> Element<'a, Message> {
        let messages = self.messages();
        if messages.is_empty() {
            empty_state(Icon::Message, welcome, welcome_body)
        } else {
            let items = messages.iter().enumerate().map(|(index, message)| {
                let grouped = index > 0 && {
                    let prev = &messages[index - 1];
                    prev.author_id == message.author_id && message.minute - prev.minute < 5
                };
                self.view_message(message, grouped, is_thread)
            });
            let mut list = column![];
            if !self.history_complete.contains(&key) {
                list = list.push(
                    container(
                        button(text("Ältere Nachrichten laden").size(13))
                            .padding([6, 12])
                            .on_press(Message::LoadOlder)
                            .style(style::ghost),
                    )
                    .center_x(Fill)
                    .padding([8, 0]),
                );
            }
            // `anchor_bottom` hält die Liste unten, solange man unten ist — wer
            // hochgescrollt hat, wird nicht zurückgerissen.
            scrollable(list.extend(items).push(space().height(12)))
                .anchor_bottom()
                .height(Fill)
                .width(Fill)
                .into()
        }
    }

    /// Eingabezeile; `ready` = Senden ist gerade möglich.
    fn view_composer(&self, placeholder: String, ready: bool) -> Element<'_, Message> {
        let target = self.upload_target();
        let pending: &[attach::PendingFile] = target
            .and_then(|target| self.pending.get(&target))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let has_files = pending
            .iter()
            .any(|file| matches!(file.state, attach::PendingState::Done(_)));
        let can_send = ready && (!self.draft.trim().is_empty() || has_files);
        let can_attach = ready && target.is_some() && (self.home || self.can(perm::ATTACH_FILES));
        let attach_button = tip(
            button(icon(
                Icon::Plus,
                18.0,
                if can_attach {
                    style::MUTED
                } else {
                    style::FAINT
                },
            ))
            .padding(8)
            .on_press_maybe(can_attach.then_some(Message::PickFiles))
            .style(|theme, status| style::icon_button(theme, status, false)),
            "Datei anhängen",
            tooltip::Position::Top,
        );
        let mut stacked = column![].spacing(8);
        if let Some(target) = target.filter(|_| !pending.is_empty()) {
            let chips = pending.iter().map(|file| self.view_pending(target, file));
            stacked = stacked.push(row(chips).spacing(8).wrap());
        }
        container(
            stacked.push(
                container(
                    row![
                        attach_button,
                        text_input(&placeholder, &self.draft)
                            .on_input(Message::DraftChanged)
                            .on_submit(Message::Send)
                            .padding([10, 10])
                            .style(style::bare_input),
                        tip(
                            button(icon(
                                Icon::Send,
                                18.0,
                                if can_send {
                                    style::ON_ACCENT
                                } else {
                                    style::FAINT
                                }
                            ))
                            .padding(8)
                            .on_press_maybe(can_send.then_some(Message::Send))
                            .style(style::send_button),
                            "Senden (Enter)",
                            tooltip::Position::Top,
                        ),
                    ]
                    .spacing(4)
                    .align_y(iced::Center),
                )
                .padding([4, 6])
                .style(style::composer),
            ),
        )
        .padding(iced::Padding {
            top: 4.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        })
        .into()
    }

    /// Wartender Anhang über der Eingabezeile.
    fn view_pending<'a>(
        &'a self,
        target: api::UploadTarget,
        file: &'a attach::PendingFile,
    ) -> Element<'a, Message> {
        let failed = matches!(file.state, attach::PendingState::Failed(_));
        let (status, color) = match &file.state {
            attach::PendingState::Uploading => ("lädt hoch …".to_owned(), style::FAINT),
            attach::PendingState::Done(_) => (attach::size_label(file.size), style::FAINT),
            attach::PendingState::Failed(err) => (err.clone(), style::DANGER),
        };
        let preview = match &file.state {
            attach::PendingState::Done(attachment) => match self.previews.get(&attachment.id) {
                Some(attach::Preview::Ready(handle)) => Some(handle.clone()),
                _ => None,
            },
            _ => None,
        };
        let thumb: Element<'a, Message> = match preview {
            // `Contain` statt `Cover`: tiny-skia schneidet Bilder nicht ab.
            Some(handle) => iced::widget::image(handle)
                .width(40)
                .height(40)
                .content_fit(iced::ContentFit::Contain)
                .border_radius(4)
                .into(),
            None => icon(
                Icon::File,
                20.0,
                if failed { style::DANGER } else { style::MUTED },
            ),
        };
        container(
            row![
                container(thumb)
                    .width(40)
                    .height(40)
                    .center_x(40)
                    .center_y(40)
                    .style(style::image_placeholder),
                column![
                    text(short_name(&file.name, 24))
                        .size(13)
                        .font(style::BOLD)
                        .color(style::TEXT),
                    text(short_name(&status, 28)).size(12).color(color),
                ]
                .spacing(2)
                .width(Fill),
                tip(
                    button(icon_hover(Icon::Close, 14.0, style::MUTED, style::TEXT))
                        .padding(6)
                        .on_press(Message::RemovePending(target, file.key))
                        .style(|theme, status| style::icon_button(theme, status, false)),
                    "Entfernen",
                    tooltip::Position::Top,
                ),
            ]
            .spacing(8)
            .align_y(iced::Center),
        )
        .padding(8)
        .width(232)
        .style(if failed {
            style::attachment_failed
        } else {
            style::attachment
        })
        .into()
    }

    /// Anhang unter einer Nachricht: Bild (Klick speichert) oder Datei-Zeile.
    fn view_attachment<'a>(&'a self, attachment: &'a api::Attachment) -> Element<'a, Message> {
        if attachment.is_image() {
            let (width, height) = attach::display_size(attachment);
            match self.previews.get(&attachment.id) {
                Some(attach::Preview::Ready(handle)) => {
                    return tip(
                        button(
                            iced::widget::image(handle.clone())
                                .width(width)
                                .height(height)
                                .content_fit(iced::ContentFit::Contain)
                                .border_radius(8),
                        )
                        .padding(0)
                        .on_press(Message::SaveAttachment(attachment.clone()))
                        .style(style::image_button),
                        format!("{} speichern", attachment.filename),
                        tooltip::Position::Top,
                    );
                }
                Some(attach::Preview::Failed) => {}
                _ => {
                    return container(text("Bild lädt …").size(12).color(style::FAINT))
                        .width(width)
                        .height(height)
                        .center_x(width)
                        .center_y(height)
                        .style(style::image_placeholder)
                        .into();
                }
            }
        }
        container(
            row![
                icon(Icon::File, 20.0, style::MUTED),
                column![
                    text(short_name(&attachment.filename, 48))
                        .size(14)
                        .font(style::BOLD)
                        .color(style::ACCENT),
                    text(attach::size_label(attachment.size))
                        .size(12)
                        .color(style::FAINT),
                ]
                .spacing(2)
                .width(Fill),
                tip(
                    button(icon_hover(Icon::Download, 18.0, style::MUTED, style::TEXT))
                        .padding(6)
                        .on_press(Message::SaveAttachment(attachment.clone()))
                        .style(|theme, status| style::icon_button(theme, status, false)),
                    "Speichern",
                    tooltip::Position::Top,
                ),
            ]
            .spacing(12)
            .align_y(iced::Center),
        )
        .padding([8, 12])
        .max_width(420)
        .style(style::attachment)
        .into()
    }

    /// Eine Nachricht samt Reaktionen; Aktionen erscheinen beim Überfahren.
    fn view_message<'a>(
        &'a self,
        message: &'a ChatMessage,
        grouped: bool,
        in_thread: bool,
    ) -> Element<'a, Message> {
        let me = self.me_id();
        let own = message.author_id == me;
        let editing = message
            .id
            .and_then(|id| self.editing.as_ref().filter(|(e, _)| *e == id))
            .map(|(_, draft)| draft);

        let mut body = column![].spacing(4).width(Fill);
        if let Some(draft) = editing {
            body = body.push(
                column![
                    text_input("Nachricht bearbeiten", draft)
                        .on_input(Message::EditChanged)
                        .on_submit(Message::SaveEdit)
                        .padding(8)
                        .style(style::input),
                    row![
                        text("Enter speichert, Esc bricht ab")
                            .size(11)
                            .color(style::FAINT),
                        space().width(Fill),
                        button(text("Abbrechen").size(12))
                            .on_press(Message::CancelEdit)
                            .style(style::ghost),
                        button(text("Speichern").size(12))
                            .padding([4, 10])
                            .on_press(Message::SaveEdit)
                            .style(style::primary),
                    ]
                    .spacing(6)
                    .align_y(iced::Center),
                ]
                .spacing(4),
            );
        } else {
            let color = if message.id.is_none() {
                style::FAINT
            } else {
                style::TEXT
            };
            if !message.content.is_empty() || message.edited {
                let mut line = row![
                    text(&message.content)
                        .size(15)
                        .line_height(1.5)
                        .color(color)
                ]
                .spacing(6)
                .align_y(iced::Center);
                if message.edited {
                    line = line.push(text("(bearbeitet)").size(11).color(style::FAINT));
                }
                body = body.push(line.wrap());
            }
            for attachment in &message.attachments {
                body = body.push(self.view_attachment(attachment));
            }
        }

        if let Some(id) = message.id.filter(|_| !message.reactions.is_empty()) {
            let pills = message.reactions.iter().map(|reaction| {
                let mine = reaction.me;
                button(
                    row![
                        text(reaction.emoji.clone()).size(13),
                        text(reaction.count.to_string()).size(12).color(if mine {
                            style::TEXT
                        } else {
                            style::MUTED
                        }),
                    ]
                    .spacing(4)
                    .align_y(iced::Center),
                )
                .padding([2, 8])
                .on_press(Message::React(id, reaction.emoji.clone()))
                .style(move |theme, status| style::reaction(theme, status, mine))
                .into()
            });
            body = body.push(row(pills).spacing(4).wrap());
        }

        let content: Element<'a, Message> = if grouped {
            row![space().width(48), body].padding([1, 16]).into()
        } else {
            let mut head = row![
                text(&message.author)
                    .font(style::BOLD)
                    .color(style::TEXT)
                    .size(15),
                text(&message.time).color(style::FAINT).size(12),
            ]
            .spacing(8)
            .align_y(iced::Center);
            if message.pinned {
                head = head.push(icon(Icon::Pin, 12.0, style::FAINT));
            }
            row![
                avatar(&message.author, 36.0),
                column![head, body].spacing(2).width(Fill),
            ]
            .spacing(12)
            .padding(iced::Padding {
                top: 14.0,
                right: 16.0,
                bottom: 1.0,
                left: 16.0,
            })
            .into()
        };

        // Unbestätigte oder gerade bearbeitete Nachrichten: keine Werkzeuge.
        let Some(id) = message.id.filter(|_| editing.is_none() && !self.home) else {
            return content;
        };
        let tool =
            |glyph: Icon, label: String, msg: Message, danger: bool| -> Element<'a, Message> {
                tip(
                    button(icon_hover(
                        glyph,
                        16.0,
                        style::MUTED,
                        if danger { style::DANGER } else { style::TEXT },
                    ))
                    .padding(5)
                    .on_press(msg)
                    .style(|theme, status| style::icon_button(theme, status, false)),
                    label,
                    tooltip::Position::Top,
                )
            };
        let mut tools = row![].spacing(2).align_y(iced::Center);
        for emoji in QUICK_REACTIONS {
            tools = tools.push(
                button(text(emoji).size(14))
                    .padding([3, 5])
                    .on_press(Message::React(id, emoji.to_owned()))
                    .style(|theme, status| style::icon_button(theme, status, false)),
            );
        }
        tools = tools.push(vline(18));
        if own {
            tools = tools.push(tool(
                Icon::Pencil,
                "Bearbeiten".into(),
                Message::StartEdit(id, message.content.clone()),
                false,
            ));
        }
        if !in_thread && self.can(perm::CREATE_THREADS) {
            tools = tools.push(tool(
                Icon::Thread,
                "Thread starten".into(),
                Message::OpenModal(Modal::Thread { message_id: id }),
                false,
            ));
        }
        if self.can(perm::MANAGE_MESSAGES) {
            tools = tools.push(tool(
                Icon::Pin,
                if message.pinned {
                    "Lösen".into()
                } else {
                    "Anpinnen".into()
                },
                Message::TogglePin(id, !message.pinned),
                false,
            ));
        }
        if own || self.can(perm::MANAGE_MESSAGES) {
            tools = tools.push(tool(
                Icon::Trash,
                "Löschen".into(),
                Message::Delete(id),
                true,
            ));
        }
        let toolbar = container(container(tools).padding([2, 4]).style(style::message_tools))
            .width(Fill)
            .align_x(iced::alignment::Horizontal::Right)
            .padding(iced::Padding {
                top: if grouped { 0.0 } else { 6.0 },
                right: 16.0,
                bottom: 0.0,
                left: 0.0,
            });
        let overlay = container(toolbar)
            .width(Fill)
            .height(Fill)
            .style(style::message_hover);
        // Nur Debug-Builds: `NOVA_DEV_TOOLS=1` zeigt die Leiste dauerhaft.
        if cfg!(debug_assertions) && std::env::var_os("NOVA_DEV_TOOLS").is_some() {
            return stack![content, overlay].into();
        }
        hover(content, overlay)
    }

    fn view_members(&self) -> Element<'_, Message> {
        let Some(detail) = &self.detail else {
            return space().into();
        };
        let mut list = column![].spacing(2);
        for (label, online) in [("Online", true), ("Offline", false)] {
            let members: Vec<_> = detail
                .members
                .iter()
                .filter(|m| (m.status >= 0) == online)
                .collect();
            if members.is_empty() {
                continue;
            }
            list = list.push(section_count(label, members.len()));
            for member in members {
                let mut name = row![
                    text(&member.display_name)
                        .color(if online { style::TEXT } else { style::FAINT })
                        .size(14)
                ]
                .spacing(6)
                .align_y(iced::Center);
                if member.bot {
                    name = name.push(
                        container(text("Bot").size(10).font(style::BOLD).color(style::MUTED))
                            .padding([1, 4])
                            .style(|_: &Theme| style::tag(style::RAISED_HI)),
                    );
                }
                if member.timeout_until.is_some() {
                    name = name.push(icon(Icon::MicOff, 12.0, style::FAINT));
                }
                list = list.push(
                    button(
                        row![
                            avatar_with_status(&member.display_name, 32.0, online, style::SIDEBAR),
                            name,
                        ]
                        .spacing(10)
                        .align_y(iced::Center),
                    )
                    .width(Fill)
                    .padding([4, 8])
                    .on_press_maybe(
                        member
                            .user_id
                            .map(|id| Message::OpenModal(Modal::Member(id))),
                    )
                    .style(|theme, status| style::channel_button(theme, status, false)),
                );
            }
        }
        container(column![
            header_bar(
                row![
                    text("Mitglieder")
                        .size(13)
                        .font(style::BOLD)
                        .color(style::MUTED),
                    text(detail.members.len()).size(13).color(style::FAINT),
                ]
                .spacing(6)
            ),
            scrollable(list.padding([0, 8])).height(Fill),
        ])
        .width(Length::Fixed(232.0))
        .height(Fill)
        .style(style::sidebar)
        .into()
    }
}

/// Kopfzeile mit fester Höhe und Trennlinie, damit Sidebar, Chat und
/// Mitgliederliste auf einer Linie abschließen.
fn header_bar<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![
        container(content)
            .height(48)
            .width(Fill)
            .padding([0, 16])
            .align_y(iced::Center),
        hline(),
    ]
    .into()
}

fn hline<'a>() -> Element<'a, Message> {
    container(space())
        .width(Fill)
        .height(1)
        .style(style::divider)
        .into()
}

fn vline<'a>(height: impl Into<Length>) -> Element<'a, Message> {
    container(space())
        .width(1)
        .height(height)
        .style(style::divider)
        .into()
}

/// Abschnittsüberschrift in Dialogen, bündig mit den Feldern.
fn modal_label<'a>(label: &'a str) -> Element<'a, Message> {
    container(text(label).size(12).font(style::BOLD).color(style::FAINT))
        .padding(iced::Padding {
            top: 8.0,
            ..iced::Padding::ZERO
        })
        .into()
}

/// Abschnittsüberschrift in normaler Schreibweise (keine Versalien-Labels).
fn section_label<'a>(label: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    container(text(label).size(12).font(style::BOLD).color(style::FAINT))
        .padding(iced::Padding {
            top: 18.0,
            right: 8.0,
            bottom: 6.0,
            left: 8.0,
        })
        .into()
}

/// Abschnitt mit Anzahl, z. B. „Online 2“: die Zahl leiser statt mit Gedankenstrich.
fn section_count(label: &str, count: usize) -> Element<'_, Message> {
    container(
        row![
            text(label).size(12).font(style::BOLD).color(style::FAINT),
            text(count).size(12).color(style::FAINT),
        ]
        .spacing(6),
    )
    .padding(iced::Padding {
        top: 18.0,
        right: 8.0,
        bottom: 6.0,
        left: 8.0,
    })
    .into()
}

fn field<'a>(label: &'a str, input: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![text(label).size(13).color(style::MUTED), input.into()]
        .spacing(6)
        .into()
}

fn tip<'a>(
    content: impl Into<Element<'a, Message>>,
    label: impl text::IntoFragment<'a>,
    position: tooltip::Position,
) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(label).size(12).color(style::TEXT))
            .padding([5, 9])
            .style(style::tooltip),
        position,
    )
    .gap(6)
    .into()
}

fn empty_state<'a>(glyph: Icon, title: String, body: &'a str) -> Element<'a, Message> {
    container(
        column![
            container(icon(glyph, 28.0, style::MUTED))
                .center(64)
                .style(style::badge),
            space().height(4),
            text(title).size(18).font(style::BOLD).color(style::TEXT),
            text(body).color(style::MUTED),
        ]
        .spacing(6)
        .align_x(iced::Center),
    )
    .center(Fill)
    .into()
}

fn avatar<'a>(name: &str, size: f32) -> Element<'a, Message> {
    let tint = style::avatar_tint(name);
    container(
        text(initials(name))
            .size(size * 0.38)
            .font(style::BOLD)
            .color(style::TEXT),
    )
    .center(size)
    .style(move |_: &Theme| style::avatar(tint))
    .into()
}

/// Avatar mit Statuspunkt unten rechts; `ring` ist die Farbe der Fläche dahinter.
/// Status über Form: online gefüllter Punkt, sonst hohler Ring.
fn avatar_with_status<'a>(
    name: &str,
    size: f32,
    online: bool,
    ring: Color,
) -> Element<'a, Message> {
    stack![
        avatar(name, size),
        container(
            container(
                container(space())
                    .width(10)
                    .height(10)
                    .style(move |_: &Theme| style::presence(online)),
            )
            .padding(2)
            .style(move |_: &Theme| style::avatar(ring)),
        )
        .width(Fill)
        .height(Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .align_y(iced::alignment::Vertical::Bottom),
    ]
    .into()
}

/// Kleiner Avatar in der Reihe unter einem Sprachkanal: Ring in Flächenfarbe
/// trennt die Überlappung, wer spricht, bekommt Gelb und rückt 2 px hoch.
fn voice_face<'a>(name: &str, speaking: bool) -> Element<'a, Message> {
    let tint = style::avatar_tint(name);
    let ring = if speaking {
        style::SPEAKING
    } else {
        style::SIDEBAR
    };
    let face = container(
        text(initials(name))
            .size(8)
            .font(style::BOLD)
            .color(style::TEXT),
    )
    .center(24)
    .style(move |_: &Theme| style::avatar_ring(tint, ring));
    let lift = iced::Padding {
        top: if speaking { 0.0 } else { 2.0 },
        ..iced::Padding::ZERO
    };
    container(face).padding(lift).height(26).into()
}

/// Kanalnamen wie der Server sie annimmt: ohne Leerzeichen und `#`, max. 48
/// Zeichen; Textkanäle klein geschrieben.
fn channel_slug(raw: &str, voice: bool) -> String {
    let joined = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .replace('#', "");
    let cleaned: String = joined
        .chars()
        .filter(|c| !c.is_control())
        .take(48)
        .collect();
    if voice {
        cleaned
    } else {
        cleaned.to_lowercase()
    }
}

/// Debug-Schalter für Tests ohne Klicks; in Release-Builds immer aus.
fn dev_flag(name: &str) -> bool {
    cfg!(debug_assertions) && std::env::var_os(name).is_some()
}

/// „Live“ in Nova-Gelb für Streamende; leer, wenn nicht.
fn live_tag<'a>(live: bool) -> Element<'a, Message> {
    if !live {
        return space().width(0).into();
    }
    container(
        text("Live")
            .size(10)
            .font(style::BOLD)
            .color(style::ON_ACCENT),
    )
    .padding([1, 4])
    .style(|_: &Theme| style::tag(style::ACCENT))
    .into()
}

/// Lange Namen kürzen, damit Chips und Zeilen nicht umbrechen.
fn short_name(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_owned();
    }
    let head: String = name.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_slug_matches_server_rules() {
        assert_eq!(channel_slug("  Neue Ideen #1 ", false), "neue-ideen-1");
        assert_eq!(channel_slug("Lounge Ecke", true), "Lounge-Ecke");
        assert_eq!(channel_slug(&"x".repeat(80), false).len(), 48);
        assert!(
            nova_common::validate::validate_channel_name(&channel_slug("a b#c", false)).is_ok()
        );
    }

    #[test]
    fn reactions_count_and_vanish() {
        let mut message =
            ChatMessage::build(Some(1), 1, "A".into(), "hi".into(), chrono::Utc::now());
        message.react("👍", 1, true);
        message.react("👍", 1, false);
        assert_eq!(message.reactions[0].count, 2);
        assert!(message.reactions[0].me);
        message.react("👍", -1, true);
        assert!(!message.reactions[0].me);
        message.react("👍", -1, false);
        assert!(message.reactions.is_empty());
    }
}
