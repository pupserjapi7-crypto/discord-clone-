//! Zentraler UI-Zustand als einzelne Signale. Komponenten lesen nur, was sie
//! brauchen; eine neue Nachricht zeichnet so nicht die Kanal- oder
//! Mitgliederliste neu.

use std::collections::HashMap;

use dioxus::prelude::*;

use crate::{gateway::Gateway, types::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnStatus {
    Offline,
    Connecting,
    Online,
}

/// Nachrichtenverlauf eines Kanals.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    pub messages: Vec<Message>,
    pub loading: bool,
    pub error: Option<String>,
    /// Ältere Nachrichten gibt es nicht mehr.
    pub complete: bool,
    /// Lesestand beim Öffnen – für den „Neu“-Trenner.
    pub read_marker: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Typing {
    pub channel_id: String,
    pub user_id: String,
    pub name: String,
    pub until_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub text: String,
    pub error: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoiceUi {
    /// Kanal, in dem man verbunden ist (oder sich gerade verbindet).
    pub channel: Option<Channel>,
    pub connecting: bool,
    pub joined: bool,
    pub muted: bool,
    pub deafened: bool,
    pub screen: bool,
    pub ping_ms: Option<u32>,
    /// Push-to-Talk-Taste gerade gedrückt.
    pub ptt_down: bool,
    /// Nutzer-IDs, die gerade sprechen.
    pub speaking: Vec<String>,
    /// Nutzer-IDs mit aktivem Videostream (Kacheln).
    pub video_users: Vec<String>,
}

/// Wer in welchem Sprachkanal der offenen Guild ist (aus Events).
pub type VoiceRooms = HashMap<String, Vec<VoiceParticipant>>;

#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    None,
    CreateGuild,
    JoinGuild,
    Invite(Option<String>),
    CreateChannel(i16),
    NewThread {
        channel_id: String,
        message_id: String,
    },
    Settings(SettingsTab),
    ServerSettings(ServerTab),
    Palette,
    Member(String),
    Confirm(ConfirmAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    Profile,
    Voice,
    Appearance,
    Shortcuts,
    Notifications,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerTab {
    Overview,
    Roles,
    Bans,
    Audit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConfirmAction {
    Kick {
        user_id: String,
        name: String,
    },
    Ban {
        user_id: String,
        name: String,
    },
    DeleteMessage {
        channel_id: String,
        message_id: String,
    },
}

#[derive(Clone, Copy)]
pub struct Store {
    pub user: Signal<Option<User>>,
    pub booting: Signal<bool>,
    pub status: Signal<ConnStatus>,
    pub guilds: Signal<Vec<GuildSummary>>,
    pub guilds_loaded: Signal<bool>,
    pub detail: Signal<Option<GuildDetail>>,
    pub guild_loading: Signal<bool>,
    pub channel_id: Signal<Option<String>>,
    pub thread_id: Signal<Option<String>>,
    pub history: Signal<HashMap<String, History>>,
    pub typing: Signal<Vec<Typing>>,
    pub toast: Signal<Option<Toast>>,
    pub voice: Signal<VoiceUi>,
    pub voice_rooms: Signal<VoiceRooms>,
    pub overlay: Signal<Overlay>,
    pub pins: Signal<Option<Vec<Message>>>,
    pub settings: Signal<Settings>,
    pub gateway: Signal<Option<Gateway>>,
    pub nav_open: Signal<bool>,
    pub members_open: Signal<bool>,
}

impl Store {
    /// Muss in einer Komponente aufgerufen werden (Signale brauchen einen Owner).
    pub fn create() -> Self {
        let settings = crate::settings::load();
        // Unter 1100 px liegt die Mitgliederliste über dem Chat: dort startet
        // sie geschlossen und öffnet sich nur auf Wunsch.
        let wide = web_sys::window()
            .and_then(|window| window.inner_width().ok())
            .and_then(|width| width.as_f64())
            .is_none_or(|width| width >= 1100.0);
        let members_open = settings.show_members && wide;
        Self {
            user: Signal::new(None),
            booting: Signal::new(true),
            status: Signal::new(ConnStatus::Offline),
            guilds: Signal::new(Vec::new()),
            guilds_loaded: Signal::new(false),
            detail: Signal::new(None),
            guild_loading: Signal::new(false),
            channel_id: Signal::new(None),
            thread_id: Signal::new(None),
            history: Signal::new(HashMap::new()),
            typing: Signal::new(Vec::new()),
            toast: Signal::new(None),
            voice: Signal::new(VoiceUi::default()),
            voice_rooms: Signal::new(HashMap::new()),
            overlay: Signal::new(Overlay::None),
            pins: Signal::new(None),
            settings: Signal::new(settings),
            gateway: Signal::new(None),
            nav_open: Signal::new(false),
            members_open: Signal::new(members_open),
        }
    }

    pub fn me_id(&self) -> Option<String> {
        self.user.read().as_ref().map(|user| user.id.clone())
    }

    pub fn error(mut self, text: impl Into<String>) {
        self.toast.set(Some(Toast {
            text: text.into(),
            error: true,
        }));
    }

    pub fn notice(mut self, text: impl Into<String>) {
        self.toast.set(Some(Toast {
            text: text.into(),
            error: false,
        }));
    }

    pub fn close_overlay(mut self) {
        self.overlay.set(Overlay::None);
    }

    pub fn current_channel(&self) -> Option<Channel> {
        let id = self.channel_id.read().clone()?;
        self.detail.read().as_ref()?.channel(&id).cloned()
    }

    pub fn can(&self, permission: u64) -> bool {
        self.detail
            .read()
            .as_ref()
            .map(|detail| detail.can(permission))
            .unwrap_or(false)
    }
}

pub fn use_store() -> Store {
    use_context::<Store>()
}
