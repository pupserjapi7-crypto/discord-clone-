//! Gateway-Protokoll für die Web-UI. Frame-Layout und MessagePack müssen exakt
//! zum Server passen (`nova_common::protocol`); der Test am Ende prüft das gegen
//! den echten Server-Typ. Dispatches werden tolerant gelesen: unbekannte Events
//! werden ignoriert, neue Felder schaden nicht.

use serde::{Deserialize, Serialize};

use crate::types::{Attachment, Message, Reaction, User};

pub const HEADER_LEN: usize = 10;
pub const OP_HELLO: u8 = 1;
pub const OP_HELLO_OK: u8 = 2;
pub const OP_RESUMED: u8 = 4;
pub const OP_INVALID_SESSION: u8 = 5;
pub const OP_DISPATCH: u8 = 8;
pub const OP_REQUEST: u8 = 9;
pub const OP_ACK: u8 = 12;
pub const OP_ERROR: u8 = 13;
pub const OP_RESYNC: u8 = 15;

#[derive(Debug, Clone, Serialize)]
pub struct ResumeRequest {
    pub session_id: String,
    pub last_seq: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t", content = "d", rename_all = "snake_case")]
pub enum ClientOp {
    Hello {
        token: String,
        resume: Option<ResumeRequest>,
    },
    Heartbeat {
        seq: u32,
    },
    Subscribe {
        channel_id: i64,
    },
    MessageCreate {
        nonce: u64,
        channel_id: i64,
        content: String,
        reply_to: Option<i64>,
        attachment_ids: Vec<i64>,
    },
    MessageEdit {
        nonce: u64,
        message_id: i64,
        content: String,
    },
    ReactionAdd {
        message_id: i64,
        emoji: String,
    },
    ReactionRemove {
        message_id: i64,
        emoji: String,
    },
    Typing {
        channel_id: i64,
    },
}

pub fn encode(op: u8, seq: u32, value: &ClientOp) -> Result<Vec<u8>, String> {
    let body = rmp_serde::to_vec_named(value).map_err(|err| err.to_string())?;
    let mut frame = Vec::with_capacity(HEADER_LEN + body.len());
    frame.push(op);
    frame.push(0);
    frame.extend_from_slice(&seq.to_be_bytes());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub struct Frame {
    pub op: u8,
    pub seq: u32,
    pub body: Vec<u8>,
}

pub fn decode(bytes: &[u8]) -> Result<Frame, String> {
    if bytes.len() < HEADER_LEN {
        return Err("frame too short".into());
    }
    let len = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    if bytes.len() < HEADER_LEN + len {
        return Err("frame truncated".into());
    }
    Ok(Frame {
        op: bytes[0],
        seq: u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
        body: bytes[HEADER_LEN..HEADER_LEN + len].to_vec(),
    })
}

#[derive(Debug, Deserialize)]
pub struct HelloOk {
    pub session_id: String,
    pub seq: u32,
    #[serde(default)]
    pub heartbeat_secs: u32,
}

#[derive(Debug, Deserialize)]
pub struct AckPayload {
    pub nonce: u64,
    pub id: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ErrorPayload {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub nonce: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DispatchEnvelope {
    t: String,
    #[serde(default)]
    d: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct WireUser {
    id: i64,
    handle: String,
    display_name: String,
    #[serde(default)]
    status: i16,
    avatar_url: Option<String>,
    #[serde(default)]
    bot: bool,
}

#[derive(Debug, Deserialize)]
struct WireAttachment {
    id: i64,
    filename: String,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    size: i64,
    #[serde(default)]
    width: Option<i32>,
    #[serde(default)]
    height: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct WireMessage {
    id: i64,
    channel_id: i64,
    author: WireUser,
    content: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    edited_at: Option<String>,
    #[serde(default)]
    reply_to: Option<i64>,
    #[serde(default)]
    reactions: Vec<Reaction>,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    attachments: Vec<WireAttachment>,
}

fn convert(message: WireMessage) -> Message {
    Message {
        id: message.id.to_string(),
        channel_id: message.channel_id.to_string(),
        author: User {
            id: message.author.id.to_string(),
            handle: message.author.handle,
            display_name: message.author.display_name,
            status: message.author.status,
            avatar_url: message.author.avatar_url,
            bot: message.author.bot,
        },
        content: message.content,
        created_at: message.created_at,
        edited_at: message.edited_at,
        reply_to: message.reply_to.map(|id| id.to_string()),
        reactions: message.reactions,
        pinned: message.pinned,
        attachments: message
            .attachments
            .into_iter()
            .map(|attachment| Attachment {
                id: attachment.id.to_string(),
                filename: attachment.filename,
                content_type: attachment.content_type,
                size: attachment.size,
                width: attachment.width,
                height: attachment.height,
            })
            .collect(),
        nonce: None,
        failed: false,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceState {
    pub guild_id: String,
    pub channel_id: String,
    pub user_id: String,
    pub connected: bool,
    pub muted: bool,
    pub deafened: bool,
    pub screen: bool,
}

pub enum Dispatched {
    Created(Message),
    Updated(Message),
    Deleted {
        channel_id: String,
        message_id: String,
    },
    Typing {
        channel_id: String,
        user_id: String,
        display_name: String,
    },
    Reaction {
        channel_id: String,
        message_id: String,
        user_id: String,
        emoji: String,
        added: bool,
    },
    Presence {
        user_id: String,
        status: i16,
    },
    GuildUpdate {
        guild_id: String,
    },
    MemberRemove {
        guild_id: String,
        user_id: String,
    },
    MemberUpdate {
        guild_id: String,
    },
    Pin {
        channel_id: String,
        message_id: String,
        pinned: bool,
    },
    Voice(VoiceState),
    Other,
}

fn id(value: &serde_json::Value, key: &str) -> String {
    match value.get(key) {
        Some(serde_json::Value::Number(number)) => number.to_string(),
        Some(serde_json::Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

fn flag(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub fn parse_dispatch(body: &[u8]) -> Option<Dispatched> {
    let envelope: DispatchEnvelope = rmp_serde::from_slice(body).ok()?;
    let data = envelope.d.unwrap_or(serde_json::Value::Null);
    let event = match envelope.t.as_str() {
        "message_create" | "message_update" => {
            #[derive(Deserialize)]
            struct Payload {
                message: WireMessage,
            }
            let payload: Payload = serde_json::from_value(data).ok()?;
            let message = convert(payload.message);
            if envelope.t == "message_create" {
                Dispatched::Created(message)
            } else {
                Dispatched::Updated(message)
            }
        }
        "message_delete" => Dispatched::Deleted {
            channel_id: id(&data, "channel_id"),
            message_id: id(&data, "message_id"),
        },
        "typing_start" => Dispatched::Typing {
            channel_id: id(&data, "channel_id"),
            user_id: id(&data, "user_id"),
            display_name: data
                .get("display_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        },
        "reaction_add" | "reaction_remove" => Dispatched::Reaction {
            channel_id: id(&data, "channel_id"),
            message_id: id(&data, "message_id"),
            user_id: id(&data, "user_id"),
            emoji: data
                .get("emoji")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            added: envelope.t == "reaction_add",
        },
        "presence_update" => Dispatched::Presence {
            user_id: id(&data, "user_id"),
            status: data
                .get("status")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0) as i16,
        },
        "guild_update" => Dispatched::GuildUpdate {
            guild_id: id(&data, "guild_id"),
        },
        "guild_member_remove" => Dispatched::MemberRemove {
            guild_id: id(&data, "guild_id"),
            user_id: id(&data, "user_id"),
        },
        "guild_member_update" => Dispatched::MemberUpdate {
            guild_id: id(&data, "guild_id"),
        },
        "message_pin" => Dispatched::Pin {
            channel_id: id(&data, "channel_id"),
            message_id: id(&data, "message_id"),
            pinned: flag(&data, "pinned"),
        },
        "voice_state_update" => Dispatched::Voice(VoiceState {
            guild_id: id(&data, "guild_id"),
            channel_id: id(&data, "channel_id"),
            user_id: id(&data, "user_id"),
            connected: flag(&data, "connected"),
            muted: flag(&data, "muted"),
            deafened: flag(&data, "deafened"),
            screen: flag(&data, "screen"),
        }),
        _ => Dispatched::Other,
    };
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_common::protocol as server;

    #[test]
    fn hello_frame_decodes_as_server_client_op() {
        let frame = encode(
            OP_HELLO,
            0,
            &ClientOp::Hello {
                token: "token".into(),
                resume: None,
            },
        )
        .expect("encode");
        let decoded = decode(&frame).expect("frame");
        assert_eq!(decoded.op, OP_HELLO);
        let op: server::ClientOp = server::decode_body(&decoded.body).expect("server decode");
        assert!(matches!(op, server::ClientOp::Hello { token, resume: None } if token == "token"));
    }

    #[test]
    fn typing_and_edit_ops_match_the_server() {
        for op in [
            ClientOp::Typing { channel_id: 5 },
            ClientOp::MessageEdit {
                nonce: 1,
                message_id: 9,
                content: "neu".into(),
            },
            ClientOp::ReactionAdd {
                message_id: 9,
                emoji: "👍".into(),
            },
        ] {
            let frame = encode(OP_REQUEST, 0, &op).expect("encode");
            let decoded = decode(&frame).expect("frame");
            let _: server::ClientOp = server::decode_body(&decoded.body).expect("server decode");
        }
    }

    #[test]
    fn message_with_attachments_matches_the_server() {
        let op = ClientOp::MessageCreate {
            nonce: 3,
            channel_id: 5,
            content: String::new(),
            reply_to: None,
            attachment_ids: vec![9_007_199_254_740_993],
        };
        let frame = encode(OP_REQUEST, 0, &op).expect("encode");
        let decoded = decode(&frame).expect("frame");
        match server::decode_body::<server::ClientOp>(&decoded.body).expect("server decode") {
            server::ClientOp::MessageCreate { attachment_ids, .. } => {
                assert_eq!(attachment_ids, vec![9_007_199_254_740_993]);
            }
            other => panic!("unexpected {other:?}"),
        }

        let event = server::Event::MessageCreate {
            message: server::MessageView {
                id: 1,
                channel_id: 5,
                author: server::UserPublic {
                    id: 2,
                    handle: "mara".into(),
                    display_name: "Mara".into(),
                    avatar_url: None,
                    status: 0,
                    bot: false,
                },
                content: String::new(),
                created_at: chrono::Utc::now(),
                edited_at: None,
                reply_to: None,
                reactions: vec![],
                attachments: vec![server::AttachmentView {
                    id: 9_007_199_254_740_993,
                    filename: "karte.png".into(),
                    content_type: "image/png".into(),
                    size: 2048,
                    width: Some(640),
                    height: Some(480),
                    url: "https://nova.example/api/v1/attachments/9007199254740993".into(),
                }],
                pinned: false,
            },
        };
        let body = rmp_serde::to_vec_named(&event).expect("encode");
        let Some(Dispatched::Created(message)) = parse_dispatch(&body) else {
            panic!("kein MessageCreate");
        };
        assert_eq!(message.attachments.len(), 1);
        let attachment = &message.attachments[0];
        assert_eq!(attachment.id, "9007199254740993");
        assert!(attachment.is_image());
        assert_eq!(attachment.url(), "/api/v1/attachments/9007199254740993");
    }

    #[test]
    fn server_events_parse_including_new_ones() {
        let events = [
            server::Event::GuildUpdate { guild_id: 1 },
            server::Event::MessagePin {
                channel_id: 2,
                message_id: 3,
                pinned: true,
            },
            server::Event::VoiceStateUpdate {
                guild_id: 1,
                channel_id: 4,
                user_id: 5,
                connected: true,
                muted: true,
                deafened: false,
                screen: false,
            },
        ];
        for event in events {
            let body = rmp_serde::to_vec_named(&event).expect("encode");
            let parsed = parse_dispatch(&body).expect("parse");
            match (event, parsed) {
                (server::Event::GuildUpdate { .. }, Dispatched::GuildUpdate { guild_id }) => {
                    assert_eq!(guild_id, "1");
                }
                (server::Event::MessagePin { .. }, Dispatched::Pin { pinned, .. }) => {
                    assert!(pinned)
                }
                (server::Event::VoiceStateUpdate { .. }, Dispatched::Voice(state)) => {
                    assert_eq!(state.user_id, "5");
                    assert!(state.muted && state.connected);
                }
                _ => panic!("falsch zugeordnet"),
            }
        }
    }
}
