use serde::Deserialize;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct User {
    pub id: String,
    pub handle: String,
    pub display_name: String,
    pub status: i16,
    pub avatar_url: Option<String>,
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

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Channel {
    pub id: String,
    #[serde(default)]
    pub guild_id: String,
    pub name: String,
    pub kind: i16,
    pub topic: Option<String>,
    #[serde(default)]
    pub position: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Member {
    pub user_id: String,
    pub handle: String,
    pub display_name: String,
    #[serde(default)]
    pub status: i16,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct GuildDetail {
    pub guild: Guild,
    #[serde(default)]
    pub channels: Vec<Channel>,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default)]
    pub permissions: u64,
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
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct VoiceJoinResponse {
    pub sdp: String,
}

#[derive(Clone, Default)]
pub struct SessionState {
    pub user: Option<User>,
    pub guilds: Vec<GuildSummary>,
    pub detail: Option<GuildDetail>,
    pub channel: Option<Channel>,
    pub messages: Vec<Message>,
    pub status: String,
    pub error: String,
    pub booting: bool,
    pub voice_channel: Option<Channel>,
    pub voice_participants: Vec<VoiceParticipant>,
    pub voice_joined: bool,
    pub muted: bool,
    pub deafened: bool,
    pub screen: bool,
}

pub fn now_string() -> String {
    js_sys::Date::new_0()
        .to_iso_string()
        .as_string()
        .unwrap_or_default()
}
