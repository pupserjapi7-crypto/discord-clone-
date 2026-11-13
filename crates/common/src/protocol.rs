//! Gateway wire protocol.
//!
//! Every WebSocket binary message is one frame:
//!
//! ```text
//! +--------+--------+------------------+------------------+----------------+
//! | op: u8 | flags  | seq: u32 (BE)    | len: u32 (BE)    | body: len B    |
//! +--------+--------+------------------+------------------+----------------+
//! ```
//!
//! `body` is MessagePack with named fields. `seq` is a per-session monotonic
//! counter for `DISPATCH` frames and is used for resume/replay.

use bytes::{BufMut, Bytes, BytesMut};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::ProtocolError;

pub const HEADER_LEN: usize = 10;
pub const MAX_FRAME_BYTES: usize = 32 * 1024;

pub const OP_HELLO: u8 = 1;
pub const OP_HELLO_OK: u8 = 2;
pub const OP_RESUMED: u8 = 4;
pub const OP_INVALID_SESSION: u8 = 5;
pub const OP_HEARTBEAT: u8 = 6;
pub const OP_HEARTBEAT_ACK: u8 = 7;
pub const OP_DISPATCH: u8 = 8;
pub const OP_REQUEST: u8 = 9;
pub const OP_RESPONSE: u8 = 10;
pub const OP_ACK: u8 = 12;
pub const OP_ERROR: u8 = 13;
pub const OP_CLOSE: u8 = 14;
/// Server tells the client its resume window is incomplete; refetch history.
pub const OP_RESYNC: u8 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    pub op: u8,
    pub flags: u8,
    pub seq: u32,
    pub body: &'a [u8],
}

/// Serialize a payload and prefix the frame header.
pub fn encode<T: Serialize>(op: u8, seq: u32, payload: &T) -> Result<Bytes, ProtocolError> {
    let body = rmp_serde::to_vec_named(payload)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::TooLarge {
            size: body.len(),
            max: MAX_FRAME_BYTES,
        });
    }
    Ok(encode_body(op, seq, &body))
}

/// Prefix an already-serialized body. Used for fan-out: serialize once, send many.
pub fn encode_body(op: u8, seq: u32, body: &[u8]) -> Bytes {
    let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
    let mut buf = BytesMut::with_capacity(HEADER_LEN + body.len());
    buf.put_u8(op);
    buf.put_u8(0);
    buf.put_u32(seq);
    buf.put_u32(len);
    buf.put_slice(body);
    buf.freeze()
}

pub fn decode(buf: &[u8]) -> Result<Frame<'_>, ProtocolError> {
    if buf.len() < HEADER_LEN {
        return Err(ProtocolError::TooShort {
            expected: HEADER_LEN,
        });
    }
    let op = buf[0];
    let flags = buf[1];
    let seq = u32::from_be_bytes([buf[2], buf[3], buf[4], buf[5]]);
    let declared = u32::from_be_bytes([buf[6], buf[7], buf[8], buf[9]]) as usize;
    if declared > MAX_FRAME_BYTES {
        return Err(ProtocolError::TooLarge {
            size: declared,
            max: MAX_FRAME_BYTES,
        });
    }
    let body = &buf[HEADER_LEN..];
    if body.len() != declared {
        return Err(ProtocolError::LengthMismatch {
            declared,
            actual: body.len(),
        });
    }
    Ok(Frame {
        op,
        flags,
        seq,
        body,
    })
}

pub fn decode_body<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, ProtocolError> {
    Ok(rmp_serde::from_slice(body)?)
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserPublic {
    pub id: i64,
    pub handle: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub status: i16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuildSummary {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttachmentView {
    pub id: i64,
    pub filename: String,
    pub content_type: String,
    pub size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReactionSummary {
    pub emoji: String,
    pub count: i64,
    pub me: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageView {
    pub id: i64,
    pub channel_id: i64,
    pub author: UserPublic,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub reply_to: Option<i64>,
    pub reactions: Vec<ReactionSummary>,
    pub attachments: Vec<AttachmentView>,
}

// ---------------------------------------------------------------------------
// Client -> gateway
// ---------------------------------------------------------------------------

/// JSON can only carry 53-bit integers safely, so browser clients send IDs as
/// decimal strings. Accept both on the wire; serialize as i64 as before.
mod id_compat {
    use serde::{Deserialize, Deserializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Int(i64),
        Str(String),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
        match Raw::deserialize(deserializer)? {
            Raw::Int(value) => Ok(value),
            Raw::Str(value) => value.parse::<i64>().map_err(serde::de::Error::custom),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeRequest {
    pub session_id: String,
    pub last_seq: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        #[serde(deserialize_with = "id_compat::deserialize")]
        channel_id: i64,
    },
    Unsubscribe {
        #[serde(deserialize_with = "id_compat::deserialize")]
        channel_id: i64,
    },
    MessageCreate {
        nonce: u64,
        #[serde(deserialize_with = "id_compat::deserialize")]
        channel_id: i64,
        content: String,
        reply_to: Option<i64>,
        #[serde(default)]
        attachment_ids: Vec<i64>,
    },
    MessageEdit {
        nonce: u64,
        #[serde(deserialize_with = "id_compat::deserialize")]
        message_id: i64,
        content: String,
    },
    MessageDelete {
        nonce: u64,
        #[serde(deserialize_with = "id_compat::deserialize")]
        message_id: i64,
    },
    ReactionAdd {
        #[serde(deserialize_with = "id_compat::deserialize")]
        message_id: i64,
        emoji: String,
    },
    ReactionRemove {
        #[serde(deserialize_with = "id_compat::deserialize")]
        message_id: i64,
        emoji: String,
    },
    Typing {
        #[serde(deserialize_with = "id_compat::deserialize")]
        channel_id: i64,
    },
    ReadAck {
        #[serde(deserialize_with = "id_compat::deserialize")]
        channel_id: i64,
        #[serde(deserialize_with = "id_compat::deserialize")]
        message_id: i64,
    },
}

// ---------------------------------------------------------------------------
// Gateway -> client
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", content = "d", rename_all = "snake_case")]
pub enum ServerOp {
    HelloOk {
        session_id: String,
        user: UserPublic,
        guilds: Vec<GuildSummary>,
        seq: u32,
        heartbeat_secs: u32,
    },
    Resumed {
        seq: u32,
    },
    InvalidSession {
        resumable: bool,
    },
    HeartbeatAck {
        seq: u32,
    },
    Subscribed {
        channel_id: i64,
    },
    Ack {
        nonce: u64,
        id: Option<i64>,
        created_at: Option<DateTime<Utc>>,
    },
    Error {
        code: String,
        message: String,
        nonce: Option<u64>,
    },
    /// The ring no longer holds every event after `last_seq`; the client must
    /// refetch history, then continue from `oldest_seq - 1`.
    Resync {
        oldest_seq: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", content = "d", rename_all = "snake_case")]
pub enum Event {
    MessageCreate {
        message: MessageView,
    },
    MessageUpdate {
        message: MessageView,
    },
    MessageDelete {
        channel_id: i64,
        message_id: i64,
    },
    TypingStart {
        channel_id: i64,
        user_id: i64,
        display_name: String,
    },
    ReactionAdd {
        channel_id: i64,
        message_id: i64,
        user_id: i64,
        emoji: String,
    },
    ReactionRemove {
        channel_id: i64,
        message_id: i64,
        user_id: i64,
        emoji: String,
    },
    PresenceUpdate {
        guild_id: i64,
        user_id: i64,
        status: i16,
    },
}

impl Event {
    pub fn kind(&self) -> &'static str {
        match self {
            Event::MessageCreate { .. } => "message_create",
            Event::MessageUpdate { .. } => "message_update",
            Event::MessageDelete { .. } => "message_delete",
            Event::TypingStart { .. } => "typing_start",
            Event::ReactionAdd { .. } => "reaction_add",
            Event::ReactionRemove { .. } => "reaction_remove",
            Event::PresenceUpdate { .. } => "presence_update",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let payload = ServerOp::HeartbeatAck { seq: 9 };
        let bytes = encode(OP_HEARTBEAT_ACK, 9, &payload).expect("encode");
        let frame = decode(&bytes).expect("decode");
        assert_eq!(frame.op, OP_HEARTBEAT_ACK);
        assert_eq!(frame.seq, 9);
        let decoded: ServerOp = decode_body(frame.body).expect("decode body");
        match decoded {
            ServerOp::HeartbeatAck { seq } => assert_eq!(seq, 9),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn client_op_roundtrip() {
        let op = ClientOp::MessageCreate {
            nonce: 7,
            channel_id: 42,
            content: "hallo".into(),
            reply_to: None,
            attachment_ids: vec![],
        };
        let bytes = encode(OP_HELLO, 0, &op).expect("encode");
        let frame = decode(&bytes).expect("decode");
        let decoded: ClientOp = decode_body(frame.body).expect("decode body");
        match decoded {
            ClientOp::MessageCreate { nonce, content, .. } => {
                assert_eq!(nonce, 7);
                assert_eq!(content, "hallo");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn rejects_truncated_and_mismatched_frames() {
        assert!(matches!(
            decode(&[1, 0, 0, 0]),
            Err(ProtocolError::TooShort { .. })
        ));
        let mut bytes = encode_body(OP_PING_PLACEHOLDER, 0, b"abc").to_vec();
        bytes[9] = 9; // declared length does not match
        assert!(matches!(
            decode(&bytes),
            Err(ProtocolError::LengthMismatch { .. })
        ));
    }

    const OP_PING_PLACEHOLDER: u8 = 99;

    #[test]
    fn oversized_frames_are_rejected() {
        let body = vec![0u8; MAX_FRAME_BYTES + 1];
        let mut bytes = BytesMut::new();
        bytes.put_u8(OP_DISPATCH);
        bytes.put_u8(0);
        bytes.put_u32(0);
        bytes.put_u32((MAX_FRAME_BYTES + 1) as u32);
        bytes.put_slice(&body);
        assert!(matches!(
            decode(&bytes),
            Err(ProtocolError::TooLarge { .. })
        ));
    }
}
