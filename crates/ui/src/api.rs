use std::sync::Mutex;

use gloo_net::http::{Request, RequestBuilder};
use serde::de::DeserializeOwned;

use crate::types::*;

static TOKEN: Mutex<Option<String>> = Mutex::new(None);

pub fn set_token(value: Option<String>) {
    *TOKEN.lock().unwrap() = value;
}

pub fn token() -> Option<String> {
    TOKEN.lock().unwrap().clone()
}

fn auth_header(builder: RequestBuilder) -> RequestBuilder {
    match token() {
        Some(value) => builder.header("authorization", &format!("Bearer {value}")),
        None => builder,
    }
}

async fn send<T: DeserializeOwned>(
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<T, String> {
    let builder = match method {
        "GET" => Request::get(path),
        "POST" => Request::post(path),
        "PATCH" => Request::patch(path),
        "DELETE" => Request::delete(path),
        _ => return Err(format!("unsupported method {method}")),
    };
    let builder = auth_header(builder.credentials(web_sys::RequestCredentials::Include));
    let response = match body {
        Some(value) => {
            let payload = serde_json::to_string(&value).map_err(|err| err.to_string())?;
            builder
                .header("content-type", "application/json")
                .body(payload)
                .map_err(|err| err.to_string())?
                .send()
                .await
                .map_err(|err| err.to_string())?
        }
        None => builder.send().await.map_err(|err| err.to_string())?,
    };
    let status = response.status();
    let text = response.text().await.map_err(|err| err.to_string())?;
    if !(200..300).contains(&status) {
        let message = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(message);
    }
    if text.is_empty() {
        return serde_json::from_str("null").map_err(|err| err.to_string());
    }
    serde_json::from_str(&text).map_err(|err| format!("decode: {err}"))
}

async fn send_empty(
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<(), String> {
    let _: serde_json::Value = send(method, path, body).await?;
    Ok(())
}

pub async fn refresh() -> Result<User, String> {
    let response: AuthResponse = send("POST", "/api/v1/auth/refresh", None).await?;
    set_token(Some(response.access_token));
    Ok(response.user)
}

pub async fn login(handle: &str, password: &str) -> Result<User, String> {
    let response: AuthResponse = send(
        "POST",
        "/api/v1/auth/login",
        Some(serde_json::json!({ "handle": handle, "password": password })),
    )
    .await?;
    set_token(Some(response.access_token));
    Ok(response.user)
}

pub async fn register(handle: &str, password: &str) -> Result<User, String> {
    let response: AuthResponse = send(
        "POST",
        "/api/v1/auth/register",
        Some(serde_json::json!({ "handle": handle, "password": password })),
    )
    .await?;
    set_token(Some(response.access_token));
    Ok(response.user)
}

pub async fn logout() -> Result<(), String> {
    let result = send_empty("POST", "/api/v1/auth/logout", None).await;
    set_token(None);
    result
}

pub async fn guilds() -> Result<Vec<GuildSummary>, String> {
    send("GET", "/api/v1/guilds", None).await
}

pub async fn guild(id: &str) -> Result<GuildDetail, String> {
    send("GET", &format!("/api/v1/guilds/{id}"), None).await
}

pub async fn messages(channel: &str) -> Result<Vec<Message>, String> {
    send(
        "GET",
        &format!("/api/v1/channels/{channel}/messages?limit=50"),
        None,
    )
    .await
}

pub async fn create_guild(name: &str) -> Result<Guild, String> {
    send(
        "POST",
        "/api/v1/guilds",
        Some(serde_json::json!({ "name": name })),
    )
    .await
}

pub async fn create_channel(guild: &str, name: &str, kind: i16) -> Result<Channel, String> {
    send(
        "POST",
        &format!("/api/v1/guilds/{guild}/channels"),
        Some(serde_json::json!({ "name": name, "kind": kind })),
    )
    .await
}

pub async fn create_invite(guild: &str) -> Result<InviteResponse, String> {
    send(
        "POST",
        &format!("/api/v1/guilds/{guild}/invites"),
        Some(serde_json::json!({})),
    )
    .await
}

pub async fn join_invite(code: &str) -> Result<(), String> {
    let _: serde_json::Value = send(
        "POST",
        &format!("/api/v1/invites/{code}"),
        Some(serde_json::json!({})),
    )
    .await?;
    Ok(())
}

pub async fn voice_join(channel: &str, sdp: &str) -> Result<VoiceJoinResponse, String> {
    send(
        "POST",
        &format!("/api/v1/channels/{channel}/voice/join"),
        Some(serde_json::json!({ "sdp": sdp })),
    )
    .await
}

pub async fn voice_leave(channel: &str) -> Result<(), String> {
    send_empty(
        "POST",
        &format!("/api/v1/channels/{channel}/voice/leave"),
        Some(serde_json::json!({})),
    )
    .await
}

pub async fn voice_participants(channel: &str) -> Result<Vec<VoiceParticipant>, String> {
    send(
        "GET",
        &format!("/api/v1/channels/{channel}/voice/participants"),
        None,
    )
    .await
}
