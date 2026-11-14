//! Echo-Bot für Project Nova.
//!
//! ```text
//! NOVA_SERVER=https://nova.example NOVA_BOT_TOKEN=nb_… NOVA_CHANNELS=123,456 \
//!     cargo run -p echo-bot
//! ```
//!
//! - meldet sich mit dem Bot-Token am Gateway an,
//! - abonniert die Kanäle aus `NOVA_CHANNELS`,
//! - antwortet per REST auf `!ping` mit `pong` und auf `!echo <text>` mit dem Text,
//! - hält die Verbindung mit Heartbeats und verbindet sich bei Abbruch neu.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use nova_common::protocol::{self, ClientOp, Event, ServerOp};
use tokio_tungstenite::tungstenite::Message as WsMessage;

struct Bot {
    server: String,
    token: String,
    channels: Vec<i64>,
    http: reqwest::Client,
}

#[tokio::main]
async fn main() -> Result<()> {
    let server = std::env::var("NOVA_SERVER").context("NOVA_SERVER fehlt")?;
    let token = std::env::var("NOVA_BOT_TOKEN").context("NOVA_BOT_TOKEN fehlt")?;
    if !nova_common::tokens::is_bot_token(&token) {
        bail!("NOVA_BOT_TOKEN ist kein Bot-Token (nb_…)");
    }
    let channels = std::env::var("NOVA_CHANNELS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    let bot = Bot {
        server: server.trim_end_matches('/').to_owned(),
        token,
        channels,
        http: reqwest::Client::new(),
    };

    let mut delay = Duration::from_secs(1);
    loop {
        match bot.run().await {
            Ok(()) => delay = Duration::from_secs(1),
            Err(err) => eprintln!("Verbindung verloren: {err:#}"),
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

impl Bot {
    fn gateway_url(&self) -> String {
        let url = self
            .server
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        format!("{url}/gateway")
    }

    async fn run(&self) -> Result<()> {
        let (socket, _) = tokio_tungstenite::connect_async(self.gateway_url()).await?;
        let (mut sink, mut stream) = socket.split();
        let send = |op: u8, value: &ClientOp| -> Result<WsMessage> {
            Ok(WsMessage::Binary(protocol::encode(op, 0, value)?))
        };

        sink.send(send(
            protocol::OP_HELLO,
            &ClientOp::Hello {
                token: self.token.clone(),
                resume: None,
            },
        )?)
        .await?;

        let mut heartbeat = tokio::time::interval(Duration::from_secs(25));
        let mut last_seq = 0u32;
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    sink.send(send(protocol::OP_REQUEST, &ClientOp::Heartbeat { seq: last_seq })?).await?;
                }
                incoming = stream.next() => {
                    let Some(message) = incoming else { return Ok(()) };
                    let WsMessage::Binary(bytes) = message? else { continue };
                    let frame = protocol::decode(&bytes)?;
                    match frame.op {
                        protocol::OP_HELLO_OK => {
                            if let Ok(ServerOp::HelloOk { user, .. }) = protocol::decode_body(frame.body) {
                                println!("angemeldet als {} ({})", user.display_name, user.handle);
                            }
                            for channel_id in &self.channels {
                                sink.send(send(protocol::OP_REQUEST, &ClientOp::Subscribe { channel_id: *channel_id })?).await?;
                            }
                        }
                        protocol::OP_INVALID_SESSION => bail!("Token abgelehnt"),
                        protocol::OP_ERROR => {
                            if let Ok(ServerOp::Error { message, .. }) = protocol::decode_body(frame.body) {
                                eprintln!("Server meldet: {message}");
                            }
                        }
                        protocol::OP_DISPATCH => {
                            last_seq = frame.seq;
                            if let Ok(Event::MessageCreate { message }) = protocol::decode_body(frame.body)
                                && !message.author.bot
                                && let Some(reply) = answer(&message.content)
                            {
                                self.reply(message.channel_id, &reply).await?;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    async fn reply(&self, channel_id: i64, content: &str) -> Result<()> {
        let response = self
            .http
            .post(format!(
                "{}/api/v1/channels/{channel_id}/messages",
                self.server
            ))
            .header("authorization", format!("Bot {}", self.token))
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await?;
        if !response.status().is_success() {
            eprintln!("Antwort abgelehnt: HTTP {}", response.status());
        }
        Ok(())
    }
}

/// Die eigentliche Bot-Logik, getrennt und testbar.
fn answer(content: &str) -> Option<String> {
    let content = content.trim();
    if content == "!ping" {
        return Some("pong".to_owned());
    }
    content
        .strip_prefix("!echo ")
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::answer;

    #[test]
    fn answers_commands_only() {
        assert_eq!(answer("!ping").as_deref(), Some("pong"));
        assert_eq!(answer("!echo Hallo Welt").as_deref(), Some("Hallo Welt"));
        assert_eq!(answer("!echo   "), None);
        assert_eq!(answer("hallo"), None);
    }
}
