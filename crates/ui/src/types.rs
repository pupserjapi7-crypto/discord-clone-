//! Datentypen der REST-API, so wie der Server sie als JSON liefert. Alle IDs
//! sind Strings (64-Bit-Snowflakes passen nicht in JS-Zahlen).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct User {
    pub id: String,
    pub handle: String,
    pub display_name: String,
    #[serde(default)]
    pub status: i16,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub bot: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct GuildSummary {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Guild {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub owner_id: String,
}

pub const KIND_TEXT: i16 = 0;
pub const KIND_VOICE: i16 = 1;
pub const KIND_THREAD: i16 = 2;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Channel {
    pub id: String,
    #[serde(default)]
    pub guild_id: String,
    pub name: String,
    pub kind: i16,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub position: i32,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub slowmode_secs: i32,
    #[serde(default)]
    pub root_message_id: Option<String>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Member {
    pub user_id: String,
    pub handle: String,
    pub display_name: String,
    #[serde(default)]
    pub status: i16,
    #[serde(default)]
    pub role_ids: Vec<String>,
    #[serde(default)]
    pub timeout_until: Option<String>,
    #[serde(default)]
    pub bot: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Role {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: i32,
    #[serde(default)]
    pub permissions: i64,
    #[serde(default)]
    pub position: i32,
    #[serde(default)]
    pub is_everyone: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Unread {
    pub channel_id: String,
    #[serde(default)]
    pub last_message_id: Option<String>,
    #[serde(default)]
    pub last_read_id: String,
}

impl Unread {
    pub fn is_unread(&self) -> bool {
        let last = self
            .last_message_id
            .as_deref()
            .and_then(|id| id.parse::<i64>().ok())
            .unwrap_or(0);
        let read = self.last_read_id.parse::<i64>().unwrap_or(0);
        last > read
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct GuildDetail {
    pub guild: Guild,
    #[serde(default)]
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub roles: Vec<Role>,
    #[serde(default)]
    pub permissions: u64,
    #[serde(default)]
    pub unread: Vec<Unread>,
}

impl GuildDetail {
    pub fn can(&self, permission: u64) -> bool {
        self.permissions & perm::ADMINISTRATOR != 0 || self.permissions & permission == permission
    }

    pub fn channel(&self, id: &str) -> Option<&Channel> {
        self.channels.iter().find(|channel| channel.id == id)
    }

    pub fn member(&self, user_id: &str) -> Option<&Member> {
        self.members.iter().find(|member| member.user_id == user_id)
    }
}

/// Rechte-Bits, gespiegelt aus `nova_common::permissions`.
pub mod perm {
    pub const ADMINISTRATOR: u64 = 1 << 0;
    pub const MANAGE_GUILD: u64 = 1 << 1;
    pub const MANAGE_ROLES: u64 = 1 << 2;
    pub const MANAGE_CHANNELS: u64 = 1 << 3;
    pub const KICK_MEMBERS: u64 = 1 << 4;
    pub const BAN_MEMBERS: u64 = 1 << 5;
    pub const TIMEOUT_MEMBERS: u64 = 1 << 6;
    pub const SEND_MESSAGES: u64 = 1 << 8;
    pub const MANAGE_MESSAGES: u64 = 1 << 9;
    pub const ADD_REACTIONS: u64 = 1 << 10;
    pub const CONNECT: u64 = 1 << 13;
    pub const SPEAK: u64 = 1 << 14;
    pub const STREAM: u64 = 1 << 15;
    pub const CREATE_INVITE: u64 = 1 << 19;
    pub const CREATE_THREADS: u64 = 1 << 23;
    pub const MANAGE_THREADS: u64 = 1 << 24;

    /// Für den Rollen-Editor: Bit, Name, Erklärung.
    pub const EDITABLE: &[(u64, &str, &str)] = &[
        (
            ADMINISTRATOR,
            "Administrator",
            "Alle Rechte, umgeht Kanal-Overrides",
        ),
        (
            MANAGE_GUILD,
            "Server verwalten",
            "Bots hinzufügen, Audit-Log lesen",
        ),
        (
            MANAGE_ROLES,
            "Rollen verwalten",
            "Rollen unterhalb der eigenen bearbeiten",
        ),
        (
            MANAGE_CHANNELS,
            "Kanäle verwalten",
            "Anlegen, umbenennen, Slowmode",
        ),
        (KICK_MEMBERS, "Mitglieder entfernen", "Kick"),
        (BAN_MEMBERS, "Mitglieder bannen", "Bann und Entbannen"),
        (
            TIMEOUT_MEMBERS,
            "Timeout vergeben",
            "Schreiben und Sprechen sperren",
        ),
        (
            MANAGE_MESSAGES,
            "Nachrichten verwalten",
            "Löschen, anpinnen, Slowmode umgehen",
        ),
        (
            MANAGE_THREADS,
            "Threads verwalten",
            "Archivieren und löschen",
        ),
        (SEND_MESSAGES, "Nachrichten senden", ""),
        (CREATE_THREADS, "Threads starten", ""),
        (ADD_REACTIONS, "Reagieren", ""),
        (CONNECT, "Sprachkanälen beitreten", ""),
        (SPEAK, "Sprechen", ""),
        (STREAM, "Bildschirm teilen", ""),
        (CREATE_INVITE, "Einladungen erstellen", ""),
    ];
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    #[serde(default)]
    pub count: i64,
    #[serde(default)]
    pub me: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Message {
    pub id: String,
    pub channel_id: String,
    pub author: User,
    pub content: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub edited_at: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub pinned: bool,
    /// Nur lokal: Nonce einer noch nicht bestätigten eigenen Nachricht.
    #[serde(skip)]
    pub nonce: Option<u64>,
    /// Nur lokal: Server hat die Nachricht abgelehnt.
    #[serde(skip)]
    pub failed: bool,
}

impl Message {
    pub fn pending(&self) -> bool {
        self.nonce.is_some() && !self.failed
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct AuthResponse {
    pub user: User,
    pub access_token: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct InviteResponse {
    pub code: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct VoiceParticipant {
    pub user_id: String,
    pub handle: String,
    pub display_name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub screen: bool,
    #[serde(default)]
    pub deafened: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct VoiceJoinResponse {
    pub sdp: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub actor_name: Option<String>,
    pub action: String,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Ban {
    pub user_id: String,
    pub handle: String,
    pub display_name: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

/// Lokale Einstellungen, in `localStorage` unter `nova.settings`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    /// Schriftgröße in Prozent, 90–125.
    pub font_scale: u8,
    pub push_to_talk: bool,
    /// `KeyboardEvent.code`, z. B. `KeyV` oder `Space`.
    pub ptt_key: String,
    pub noise_suppression: bool,
    pub input_device: Option<String>,
    pub notifications: bool,
    pub show_members: bool,
    /// Lautstärke pro Nutzer-ID, 0.0–1.0.
    pub volumes: Vec<(String, f64)>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            font_scale: 100,
            push_to_talk: false,
            ptt_key: "KeyV".to_owned(),
            noise_suppression: true,
            input_device: None,
            notifications: false,
            show_members: true,
            volumes: Vec::new(),
        }
    }
}

impl Settings {
    pub fn volume(&self, user_id: &str) -> f64 {
        self.volumes
            .iter()
            .find(|(id, _)| id == user_id)
            .map(|(_, volume)| *volume)
            .unwrap_or(1.0)
    }

    pub fn set_volume(&mut self, user_id: &str, volume: f64) {
        let volume = volume.clamp(0.0, 1.0);
        match self.volumes.iter_mut().find(|(id, _)| id == user_id) {
            Some(entry) => entry.1 = volume,
            None => self.volumes.push((user_id.to_owned(), volume)),
        }
    }
}

pub fn now_string() -> String {
    js_sys::Date::new_0()
        .to_iso_string()
        .as_string()
        .unwrap_or_default()
}
