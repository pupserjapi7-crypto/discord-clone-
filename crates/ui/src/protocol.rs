use serde::{Deserialize, Serialize};

use crate::types::{Message, Reaction, User};

pub const HEADER_LEN: usize = 10;
pub const OP_HELLO: u8 = 1;
pub const OP_HELLO_OK: u8 = 2;
pub const OP_RESUMED: u8 = 4;
pub const OP_INVALID_SESSION: u8 = 5;
pub const OP_HEARTBEAT_ACK: u8 = 7;
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
}

pub fn encode(op: u8, seq: u32, value: &ClientOp) -> Result<Vec<u8>, String> {
    // Frame-Layout und MessagePack-Repräsentation müssen exakt zum Server passen.
    // Der Test am Ende dieser Datei prüft das gegen den echten Server-Typ.
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
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct DispatchEnvelope {
    pub t: String,
    #[serde(default)]
    pub d: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct MessagePayload {
    message: WireMessage,
}

#[derive(Debug, Deserialize)]
struct DeletePayload {
    message_id: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let op: nova_common::protocol::ClientOp =
            nova_common::protocol::decode_body(&decoded.body).expect("server decode");
        match op {
            nova_common::protocol::ClientOp::Hello { token, resume } => {
                assert_eq!(token, "token");
                assert!(resume.is_none());
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct WireUser {
    id: i64,
    handle: String,
    display_name: String,
    #[serde(default)]
    status: i16,
    avatar_url: Option<String>,
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
        },
        content: message.content,
        created_at: message.created_at,
        edited_at: message.edited_at,
        reply_to: message.reply_to.map(|id| id.to_string()),
        reactions: message.reactions,
    }
}

pub enum Dispatched {
    Created(Message),
    Updated(Message),
    Deleted { message_id: i64 },
    Other,
}

pub fn parse_dispatch(body: &[u8]) -> Option<Dispatched> {
    // Dispatch-Events werden als Envelope mit tolerantem Datenfeld gelesen,
    // damit neue Event-Typen den Client nicht aus dem Tritt bringen.
    let envelope: DispatchEnvelope = rmp_serde::from_slice(body).ok()?;
    let data = envelope.d?;
    match envelope.t.as_str() {
        "message_create" | "message_update" => {
            let payload: MessagePayload = serde_json::from_value(data).ok()?;
            let message = convert(payload.message);
            if envelope.t == "message_create" {
                Some(Dispatched::Created(message))
            } else {
                Some(Dispatched::Updated(message))
            }
        }
        "message_delete" => {
            let payload: DeletePayload = serde_json::from_value(data).ok()?;
            Some(Dispatched::Deleted {
                message_id: payload.message_id,
            })
        }
        _ => Some(Dispatched::Other),
    }
}
