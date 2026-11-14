//! Nova Desktop — nativer Client ohne WebView.
//!
//! iced rendert direkt über wgpu (DirectX 12/Vulkan/Metal), Netzwerk läuft über
//! tokio. Kein Chromium, kein WebView2, kein JavaScript.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod gateway;
mod icon;
mod style;
mod update;

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;

use iced::futures::channel::mpsc::UnboundedSender;
use iced::widget::{
    button, column, container, row, scrollable, space, stack, text, text_input, tooltip,
};
use iced::{Color, Element, Fill, Length, Subscription, Task, Theme};
use nova_common::protocol::{Event, MessageView};

use api::{Api, Channel, GuildDetail, User};
use gateway::{Command, GatewayEvent, Status};
use icon::{Icon, icon, icon_hover};

const DEFAULT_SERVER: &str = "https://85-215-175-202.sslip.io";

static START: OnceLock<Instant> = OnceLock::new();
static FIRST_FRAME: OnceLock<()> = OnceLock::new();

fn main() -> iced::Result {
    START.get_or_init(Instant::now);
    update::cleanup();
    iced::application(App::boot, App::update, App::view)
        .title("Nova")
        .theme(App::theme)
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

#[derive(Debug, Clone)]
struct ChatMessage {
    id: Option<i64>,
    nonce: Option<u64>,
    author_id: i64,
    author: String,
    content: String,
    time: String,
    minute: i64,
}

impl ChatMessage {
    fn from_rest(message: api::Message) -> Self {
        let parsed = chrono::DateTime::parse_from_rfc3339(&message.created_at)
            .map(|time| time.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());
        Self::build(
            Some(message.id),
            message.author.id,
            message.author.display_name,
            message.content,
            parsed,
        )
    }

    fn from_gateway(message: MessageView) -> Self {
        Self::build(
            Some(message.id),
            message.author.id,
            message.author.display_name,
            message.content,
            message.created_at,
        )
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
        }
    }
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
}

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
        };
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

    fn subscription(&self) -> Subscription<Message> {
        if self.screen != Screen::Chat {
            return Subscription::none();
        }
        gateway::subscription(gateway::Params {
            epoch: self.epoch,
            api: self.api.clone(),
        })
        .map(Message::Gateway)
    }

    fn messages(&self) -> &[ChatMessage] {
        self.channel
            .and_then(|id| self.history.get(&id))
            .map(Vec::as_slice)
            .unwrap_or(&[])
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
        Task::perform(async move { api.guilds().await }, Message::GuildsLoaded)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
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
                if handle.len() < 3 || self.password.len() < 8 {
                    self.auth_error = Some("Handle (min. 3) und Passwort (min. 8) prüfen.".into());
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
                let keep = self
                    .channel
                    .filter(|id| detail.channels.iter().any(|c| c.id == *id));
                let target = keep
                    .or_else(|| detail.channels.iter().find(|c| c.kind == 0).map(|c| c.id))
                    .or_else(|| detail.channels.first().map(|c| c.id));
                self.detail = Some(detail);
                if let Some(id) = target {
                    return self.update(Message::OpenChannel(id));
                }
                self.channel = None;
            }
            Message::OpenChannel(id) => {
                self.channel = Some(id);
                if let Some(gateway) = &self.gateway {
                    let _ = gateway.unbounded_send(Command::Subscribe(id));
                }
                let is_text = self.current_channel().map(|c| c.kind == 0).unwrap_or(false);
                if is_text {
                    let api = self.api.clone();
                    return Task::perform(async move { api.messages(id).await }, move |result| {
                        Message::HistoryLoaded(id, result)
                    });
                }
            }
            Message::HistoryLoaded(id, Ok(list)) => {
                let mut fresh: Vec<ChatMessage> =
                    list.into_iter().map(ChatMessage::from_rest).collect();
                // Noch nicht bestätigte eigene Nachrichten behalten.
                if let Some(old) = self.history.get(&id) {
                    fresh.extend(old.iter().filter(|m| m.id.is_none()).cloned());
                }
                self.history.insert(id, fresh);
            }
            Message::GuildsLoaded(Err(err))
            | Message::GuildLoaded(Err(err))
            | Message::HistoryLoaded(_, Err(err)) => self.error = Some(err),

            Message::DraftChanged(value) => self.draft = value,
            Message::Send => {
                let content = self.draft.trim().to_owned();
                let (Some(channel_id), Some(me)) = (self.channel, self.me.clone()) else {
                    return Task::none();
                };
                if content.is_empty() {
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
                self.history.entry(channel_id).or_default().push(local);
                let _ = gateway.unbounded_send(Command::Send {
                    nonce,
                    channel_id,
                    content,
                });
                self.draft.clear();
            }
            Message::Gateway(event) => return self.on_gateway(event),
            Message::ToggleMembers => self.show_members = !self.show_members,
            Message::DismissError => self.error = None,
            Message::Logout => {
                let api = self.api.clone();
                return Task::perform(async move { api.logout().await }, |()| Message::LoggedOut);
            }
            Message::LoggedOut => {
                self.screen = Screen::Login;
                self.gateway = None;
                self.me = None;
                self.detail = None;
                self.guilds.clear();
                self.history.clear();
                self.channel = None;
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
                    list.push(incoming);
                }
                Event::MessageUpdate { message } => {
                    let channel_id = message.channel_id;
                    let updated = ChatMessage::from_gateway(message);
                    if let Some(list) = self.history.get_mut(&channel_id)
                        && let Some(existing) = list.iter_mut().find(|m| m.id == updated.id)
                    {
                        existing.content = updated.content;
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
                    icon(Icon::Logo, 40.0, style::ACCENT),
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
                style::ACCENT,
            ),
            UpdateState::Installing(version) => (
                format!("Version {version} wird geladen und geprüft… Nova startet gleich neu."),
                None,
                style::ACCENT,
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
            UpdateState::UpToDate => format!("v{} · aktuell", update::CURRENT),
            UpdateState::Available(a) => format!("Update auf v{}", a.version),
            UpdateState::Installing(_) => "Wird installiert…".to_owned(),
            UpdateState::Idle | UpdateState::Failed(_) => {
                format!("v{} · Nach Updates suchen", update::CURRENT)
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
                style::ACCENT,
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
                format!("v{} ist aktuell · erneut prüfen", update::CURRENT),
                Some(Message::CheckUpdate),
            ),
            UpdateState::Idle | UpdateState::Failed(_) => (
                Icon::Refresh,
                style::MUTED,
                format!("v{} · nach Updates suchen", update::CURRENT),
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
            container(icon(Icon::Logo, 32.0, style::ACCENT))
                .center(60)
                .style(style::brand_soft),
            space().height(6),
            text("Nova")
                .size(26)
                .font(style::SEMIBOLD)
                .color(style::TEXT),
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
                "Handle",
                text_input("dein_name", &self.handle)
                    .on_input(Message::HandleChanged)
                    .on_submit(Message::Submit)
                    .padding(10)
                    .style(style::input),
            ),
            field(
                "Passwort",
                text_input("Mindestens 8 Zeichen", &self.password)
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
                button(
                    text(submit_label)
                        .font(style::SEMIBOLD)
                        .center()
                        .width(Fill),
                )
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

        let footer = row![
            text("Nativ · Rust").size(11).color(style::FAINT),
            text("·").size(11).color(style::FAINT),
            self.view_update_button(),
        ]
        .spacing(2)
        .align_y(iced::Center);

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
        let mut layout = row![self.view_rail(), self.view_sidebar(), self.view_main()];
        if self.show_members && self.detail.is_some() {
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
        container(page)
            .width(Fill)
            .height(Fill)
            .style(style::app_bg)
            .into()
    }

    fn view_rail(&self) -> Element<'_, Message> {
        let current = self.detail.as_ref().map(|d| d.guild.id);
        let guilds = self.guilds.iter().map(|guild| {
            let active = current == Some(guild.id);
            tip(
                button(
                    text(initials(&guild.name))
                        .size(15)
                        .font(style::SEMIBOLD)
                        .center()
                        .width(Fill)
                        .height(Fill)
                        .color(style::TEXT),
                )
                .width(48)
                .height(48)
                .padding(0)
                .on_press(Message::OpenGuild(guild.id))
                .style(move |theme, status| style::rail_button(theme, status, active)),
                guild.name.as_str(),
                tooltip::Position::Right,
            )
        });
        // Die Bildmarke ist bewusst keine Kachel: kein Hintergrund, kein Klick,
        // darunter ein Trenner — Server beginnen erst danach.
        container(
            column![
                container(icon(Icon::Logo, 26.0, style::ACCENT)).center(48),
                container(space()).width(32).height(2).style(style::divider),
                column(guilds).spacing(8),
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

    fn view_sidebar(&self) -> Element<'_, Message> {
        let title = self
            .detail
            .as_ref()
            .map(|d| d.guild.name.as_str())
            .unwrap_or("Nova");
        let mut channels = column![].spacing(2);
        if let Some(detail) = &self.detail {
            for (label, kind_text) in [("TEXTKANÄLE", true), ("SPRACHKANÄLE", false)] {
                channels = channels.push(section_label(label));
                for channel in detail
                    .channels
                    .iter()
                    .filter(|c| (c.kind == 0) == kind_text)
                {
                    let active = self.channel == Some(channel.id);
                    let glyph = if channel.kind == 0 {
                        Icon::Hash
                    } else {
                        Icon::Volume
                    };
                    channels = channels.push(
                        button(
                            row![
                                icon(
                                    glyph,
                                    16.0,
                                    if active { style::MUTED } else { style::FAINT }
                                ),
                                text(&channel.name).color(if active {
                                    style::TEXT
                                } else {
                                    style::MUTED
                                }),
                            ]
                            .spacing(8)
                            .align_y(iced::Center),
                        )
                        .width(Fill)
                        .padding([7, 8])
                        .on_press(Message::OpenChannel(channel.id))
                        .style(move |theme, status| style::channel_button(theme, status, active)),
                    );
                }
            }
        } else {
            channels = channels.push(
                container(
                    column![
                        text("Noch kein Server")
                            .font(style::SEMIBOLD)
                            .color(style::TEXT),
                        text("Lass dich einladen oder leg in der Web-App einen Server an.")
                            .size(13)
                            .color(style::MUTED),
                    ]
                    .spacing(4),
                )
                .padding([16, 8]),
            );
        }

        let me = self.me.as_ref();
        let name = me.map(|m| m.display_name.as_str()).unwrap_or("?");
        let (dot, label) = match self.status {
            Status::Online => (style::ONLINE, "Online"),
            Status::Connecting => (style::WARN, "Verbinde…"),
            Status::Offline => (style::FAINT, "Offline"),
        };
        let me_bar = container(
            row![
                avatar_with_status(name, 32.0, dot, style::RAIL),
                column![
                    text(name).size(14).font(style::SEMIBOLD).color(style::TEXT),
                    text(label).size(12).color(style::MUTED),
                ]
                .spacing(1)
                .width(Fill),
                self.view_update_icon(),
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
        .style(style::rail);

        container(column![
            header_bar(
                text(title)
                    .size(15)
                    .font(style::SEMIBOLD)
                    .color(style::TEXT)
            ),
            scrollable(channels.padding([0, 8])).height(Fill),
            hline(),
            me_bar,
        ])
        .width(240)
        .height(Fill)
        .style(style::sidebar)
        .into()
    }

    fn view_main(&self) -> Element<'_, Message> {
        let channel = self.current_channel();
        let is_voice = channel.map(|c| c.kind != 0).unwrap_or(false);

        let mut title = row![
            icon(
                if is_voice { Icon::Volume } else { Icon::Hash },
                20.0,
                style::FAINT
            ),
            text(channel.map(|c| c.name.as_str()).unwrap_or("Nova"))
                .size(15)
                .font(style::SEMIBOLD)
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
        let header = header_bar(
            row![
                container(title).width(Fill),
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
            let body = empty_state(
                Icon::Volume,
                channel.name.clone(),
                "Sprache kommt im nächsten Schritt nativ (str0m + cpal + Opus).",
            );
            return column![header, body].into();
        }

        let messages = self.messages();
        let list: Element<'_, Message> = if messages.is_empty() {
            empty_state(
                Icon::Message,
                format!("Willkommen in #{}", channel.name),
                "Hier ist es noch still. Schreib die erste Nachricht.",
            )
        } else {
            let items = messages.iter().enumerate().map(|(index, message)| {
                let grouped = index > 0 && {
                    let prev = &messages[index - 1];
                    prev.author_id == message.author_id && message.minute - prev.minute < 5
                };
                let body_color = if message.id.is_none() {
                    style::FAINT
                } else {
                    style::TEXT
                };
                let content = text(&message.content).color(body_color).width(Fill);
                if grouped {
                    row![space().width(48), content].padding([1, 16]).into()
                } else {
                    row![
                        avatar(&message.author, 36.0),
                        column![
                            row![
                                text(&message.author)
                                    .font(style::SEMIBOLD)
                                    .color(style::TEXT)
                                    .size(15),
                                text(&message.time).color(style::FAINT).size(12),
                            ]
                            .spacing(8)
                            .align_y(iced::Center),
                            content,
                        ]
                        .spacing(2)
                        .width(Fill),
                    ]
                    .spacing(12)
                    .padding(iced::Padding {
                        top: 14.0,
                        right: 16.0,
                        bottom: 1.0,
                        left: 16.0,
                    })
                    .into()
                }
            });
            // `anchor_bottom` hält die Liste unten, solange man unten ist — wer
            // hochgescrollt hat, wird nicht zurückgerissen.
            scrollable(column(items).push(space().height(12)))
                .anchor_bottom()
                .height(Fill)
                .width(Fill)
                .into()
        };

        let online = self.status == Status::Online;
        let placeholder = if online {
            format!("Nachricht an #{}", channel.name)
        } else {
            "Verbinde mit dem Gateway…".to_owned()
        };
        let can_send = online && !self.draft.trim().is_empty();
        let composer = container(
            container(
                row![
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
        )
        .padding(iced::Padding {
            top: 4.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        });

        column![header, list, composer]
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn view_members(&self) -> Element<'_, Message> {
        let Some(detail) = &self.detail else {
            return space().into();
        };
        let mut list = column![].spacing(2);
        for (label, online) in [("ONLINE", true), ("OFFLINE", false)] {
            let members: Vec<_> = detail
                .members
                .iter()
                .filter(|m| (m.status >= 0) == online)
                .collect();
            if members.is_empty() {
                continue;
            }
            list = list.push(section_label(format!("{label} — {}", members.len())));
            for member in members {
                let dot = if online { style::ONLINE } else { style::FAINT };
                list = list.push(
                    container(
                        row![
                            avatar_with_status(&member.display_name, 32.0, dot, style::SIDEBAR),
                            text(&member.display_name)
                                .color(if online { style::TEXT } else { style::FAINT })
                                .size(14),
                        ]
                        .spacing(10)
                        .align_y(iced::Center),
                    )
                    .padding([4, 8]),
                );
            }
        }
        container(column![
            header_bar(
                text(format!("Mitglieder · {}", detail.members.len()))
                    .size(13)
                    .font(style::SEMIBOLD)
                    .color(style::MUTED)
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

fn section_label<'a>(label: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    container(
        text(label)
            .size(11)
            .font(style::SEMIBOLD)
            .color(style::FAINT),
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
            container(icon(glyph, 28.0, style::ACCENT))
                .center(64)
                .style(style::badge),
            space().height(4),
            text(title)
                .size(18)
                .font(style::SEMIBOLD)
                .color(style::TEXT),
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
            .font(style::SEMIBOLD)
            .color(style::TEXT),
    )
    .center(size)
    .style(move |_: &Theme| style::avatar(tint))
    .into()
}

/// Avatar mit Statuspunkt unten rechts; `ring` ist die Farbe der Fläche dahinter.
fn avatar_with_status<'a>(name: &str, size: f32, dot: Color, ring: Color) -> Element<'a, Message> {
    let diameter = (size * 0.36).round().max(10.0);
    stack![
        avatar(name, size),
        container(
            container(space())
                .width(diameter)
                .height(diameter)
                .style(move |_: &Theme| style::status_dot(dot, ring)),
        )
        .width(Fill)
        .height(Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .align_y(iced::alignment::Vertical::Bottom),
    ]
    .into()
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}
