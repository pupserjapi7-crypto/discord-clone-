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
/// Forum: nur Beiträge (Threads), keine Nachrichten direkt im Kanal.
pub const KIND_FORUM: i16 = 3;

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
    pub const VIEW_CHANNEL: u64 = 1 << 7;
    pub const SEND_MESSAGES: u64 = 1 << 8;
    pub const MANAGE_MESSAGES: u64 = 1 << 9;
    pub const ADD_REACTIONS: u64 = 1 << 10;
    pub const ATTACH_FILES: u64 = 1 << 11;
    pub const READ_MESSAGE_HISTORY: u64 = 1 << 12;
    pub const CONNECT: u64 = 1 << 13;
    pub const SPEAK: u64 = 1 << 14;
    pub const STREAM: u64 = 1 << 15;
    pub const CREATE_INVITE: u64 = 1 << 19;
    pub const MANAGE_WEBHOOKS: u64 = 1 << 20;
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
        (
            MANAGE_WEBHOOKS,
            "Webhooks verwalten",
            "Webhooks für Kanäle anlegen und löschen",
        ),
        (SEND_MESSAGES, "Nachrichten senden", ""),
        (
            ATTACH_FILES,
            "Dateien anhängen",
            "Bilder und Dateien hochladen",
        ),
        (CREATE_THREADS, "Threads starten", ""),
        (ADD_REACTIONS, "Reagieren", ""),
        (CONNECT, "Sprachkanälen beitreten", ""),
        (SPEAK, "Sprechen", ""),
        (STREAM, "Video senden", "Bildschirm teilen und Kamera"),
        (CREATE_INVITE, "Einladungen erstellen", ""),
    ];
}

/// Rechte, die sich pro Kanal überschreiben lassen: Bit, Name, Erklärung.
/// Text und Forum bzw. Sprache.
pub const TEXT_OVERRIDES: &[(u64, &str, &str)] = &[
    (
        perm::VIEW_CHANNEL,
        "Kanal sehen",
        "Ohne dieses Recht ist der Kanal unsichtbar",
    ),
    (
        perm::SEND_MESSAGES,
        "Schreiben",
        "Nachrichten bzw. Forum-Beiträge",
    ),
    (perm::READ_MESSAGE_HISTORY, "Verlauf lesen", ""),
    (perm::ADD_REACTIONS, "Reagieren", ""),
    (perm::ATTACH_FILES, "Dateien anhängen", ""),
    (perm::CREATE_THREADS, "Threads starten", ""),
    (
        perm::MANAGE_MESSAGES,
        "Nachrichten verwalten",
        "Löschen, anpinnen, Slowmode umgehen",
    ),
    (perm::MANAGE_THREADS, "Threads verwalten", ""),
];

pub const VOICE_OVERRIDES: &[(u64, &str, &str)] = &[
    (
        perm::VIEW_CHANNEL,
        "Kanal sehen",
        "Ohne dieses Recht ist der Kanal unsichtbar",
    ),
    (perm::CONNECT, "Beitreten", ""),
    (perm::SPEAK, "Sprechen", ""),
    (perm::STREAM, "Video senden", "Bildschirm teilen und Kamera"),
];

/// Kanal-Override für eine Rolle oder ein Mitglied.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct ChannelOverride {
    /// `role` oder `member`
    pub kind: String,
    pub target_id: String,
    #[serde(default)]
    pub allow: u64,
    #[serde(default)]
    pub deny: u64,
}

/// Zustand eines Rechts in einem Override.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverrideState {
    Deny,
    Inherit,
    Allow,
}

impl ChannelOverride {
    pub fn state(&self, bit: u64) -> OverrideState {
        if self.deny & bit != 0 {
            OverrideState::Deny
        } else if self.allow & bit != 0 {
            OverrideState::Allow
        } else {
            OverrideState::Inherit
        }
    }

    pub fn set(&mut self, bit: u64, state: OverrideState) {
        self.allow &= !bit;
        self.deny &= !bit;
        match state {
            OverrideState::Allow => self.allow |= bit,
            OverrideState::Deny => self.deny |= bit,
            OverrideState::Inherit => {}
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Webhook {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub created_at: String,
    /// Nur direkt nach dem Anlegen.
    #[serde(default)]
    pub url: Option<String>,
}

/// Beitrag in einem Forum.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct ForumPost {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub replies: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct PostCreated {
    pub thread: Channel,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    #[serde(default)]
    pub count: i64,
    #[serde(default)]
    pub me: bool,
}

/// Anhang einer Nachricht (ab Server v0.8).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub filename: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
}

impl Attachment {
    /// Relativ statt der absoluten Server-URL: funktioniert unabhängig von
    /// `NOVA_PUBLIC_ORIGIN` und schickt das Session-Cookie mit.
    pub fn url(&self) -> String {
        format!("/api/v1/attachments/{}", self.id)
    }

    pub fn download_url(&self) -> String {
        format!("/api/v1/attachments/{}?download=1", self.id)
    }

    /// Bilder mit bekannter Größe zeigt die UI inline.
    pub fn is_image(&self) -> bool {
        self.content_type.starts_with("image/") && self.width.is_some() && self.height.is_some()
    }

    pub fn is_video(&self) -> bool {
        matches!(self.content_type.as_str(), "video/mp4" | "video/webm")
    }

    pub fn is_audio(&self) -> bool {
        self.content_type.starts_with("audio/")
    }

    /// Anzeigegröße im Chat: höchstens 400 × 300 px, Seitenverhältnis bleibt.
    pub fn display_size(&self, max_w: f64, max_h: f64) -> (u32, u32) {
        let width = f64::from(self.width.unwrap_or(1).max(1));
        let height = f64::from(self.height.unwrap_or(1).max(1));
        let scale = (max_w / width).min(max_h / height).min(1.0);
        (
            (width * scale).round().max(1.0) as u32,
            (height * scale).round().max(1.0) as u32,
        )
    }
}

/// Dateigröße für Menschen: „820 B“, „14 KB“, „3,2 MB“.
pub fn size_label(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    if bytes < 1024.0 {
        return format!("{bytes} B");
    }
    if bytes < 1024.0 * 1024.0 {
        return format!("{} KB", (bytes / 1024.0).round());
    }
    let mb = bytes / (1024.0 * 1024.0);
    let text = if mb < 10.0 {
        format!("{mb:.1}")
    } else {
        format!("{}", mb.round())
    };
    format!("{} MB", text.replace('.', ","))
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
    #[serde(default)]
    pub attachments: Vec<Attachment>,
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
    /// Kamera läuft (ab Server v0.8).
    #[serde(default)]
    pub camera: bool,
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

/// Qualität beim Bildschirm teilen (Sender).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenQuality {
    /// 1080p, 30 fps – Spiele und Präsentationen.
    #[default]
    Balanced,
    /// 1440p, 15 fps – Code und kleine Schrift.
    Sharp,
    /// 720p, 60 fps – schnelle Bewegung.
    Smooth,
}

/// Aufnahme-Profil einer Qualitätsstufe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureProfile {
    pub width: f64,
    pub height: f64,
    pub fps: f64,
    /// Bitrate der hohen Simulcast-Schicht in bit/s.
    pub bitrate: f64,
    /// `MediaStreamTrack.contentHint`: Encoder bevorzugt Schärfe oder Bewegung.
    pub hint: &'static str,
}

impl ScreenQuality {
    pub const ALL: [Self; 3] = [Self::Balanced, Self::Sharp, Self::Smooth];

    pub fn label(self) -> &'static str {
        match self {
            Self::Balanced => "Ausgewogen (1080p, 30 fps)",
            Self::Sharp => "Scharf (1440p, 15 fps) – Text und Code",
            Self::Smooth => "Flüssig (720p, 60 fps) – Spiele",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Sharp => "sharp",
            Self::Smooth => "smooth",
        }
    }

    pub fn profile(self) -> CaptureProfile {
        match self {
            Self::Balanced => CaptureProfile {
                width: 1920.0,
                height: 1080.0,
                fps: 30.0,
                bitrate: 2_500_000.0,
                hint: "detail",
            },
            Self::Sharp => CaptureProfile {
                width: 2560.0,
                height: 1440.0,
                fps: 15.0,
                bitrate: 3_000_000.0,
                hint: "text",
            },
            Self::Smooth => CaptureProfile {
                width: 1280.0,
                height: 720.0,
                fps: 60.0,
                bitrate: 2_500_000.0,
                hint: "motion",
            },
        }
    }
}

/// Welche Simulcast-Schicht man von anderen empfängt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReceiveQuality {
    /// Groß angezeigte Bilder in hoher, Kacheln in niedriger Auflösung.
    #[default]
    Auto,
    /// Immer die höchste Schicht.
    High,
    /// Immer kleine Schichten (mobile Daten).
    Saver,
}

impl ReceiveQuality {
    pub const ALL: [Self; 3] = [Self::Auto, Self::High, Self::Saver];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatisch",
            Self::High => "Immer hoch",
            Self::Saver => "Datensparend",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::High => "high",
            Self::Saver => "saver",
        }
    }

    /// Schicht für einen Bildschirm (`screens` = wie viele gerade laufen).
    pub fn screen_layer(self, screens: usize) -> &'static str {
        match self {
            Self::High => "h",
            Self::Saver => "m",
            Self::Auto if screens <= 2 => "h",
            Self::Auto => "m",
        }
    }

    /// Schicht für eine Kamera (`tiles` = Personen im Raster).
    pub fn camera_layer(self, tiles: usize) -> &'static str {
        match self {
            Self::High => "h",
            Self::Saver => "l",
            Self::Auto if tiles <= 4 => "m",
            Self::Auto => "l",
        }
    }
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
    pub screen_quality: ScreenQuality,
    /// Ton beim Teilen mitsenden (Tab- bzw. Systemton, wo der Browser es kann).
    pub screen_audio: bool,
    pub receive_quality: ReceiveQuality,
    pub camera_device: Option<String>,
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
            screen_quality: ScreenQuality::Balanced,
            screen_audio: true,
            receive_quality: ReceiveQuality::Auto,
            camera_device: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_german() {
        assert_eq!(size_label(820), "820 B");
        assert_eq!(size_label(14 * 1024), "14 KB");
        assert_eq!(size_label(3_355_443), "3,2 MB");
        assert_eq!(size_label(25 * 1024 * 1024), "25 MB");
    }

    #[test]
    fn images_scale_down_but_never_up() {
        let photo = Attachment {
            width: Some(4000),
            height: Some(3000),
            content_type: "image/jpeg".into(),
            ..Default::default()
        };
        assert_eq!(photo.display_size(400.0, 300.0), (400, 300));
        let tall = Attachment {
            width: Some(600),
            height: Some(1200),
            ..photo.clone()
        };
        assert_eq!(tall.display_size(400.0, 300.0), (150, 300));
        let icon = Attachment {
            width: Some(32),
            height: Some(32),
            ..photo
        };
        assert_eq!(icon.display_size(400.0, 300.0), (32, 32));
    }
}
