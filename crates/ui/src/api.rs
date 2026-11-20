//! REST-Client der Web-UI. Antworten kommen als JSON mit String-IDs.

use std::sync::Mutex;

use gloo_net::http::{Request, RequestBuilder};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::types::*;

static TOKEN: Mutex<Option<String>> = Mutex::new(None);

pub fn set_token(value: Option<String>) {
    if let Ok(mut token) = TOKEN.lock() {
        *token = value;
    }
}

pub fn token() -> Option<String> {
    TOKEN.lock().ok().and_then(|token| token.clone())
}

fn auth_header(builder: RequestBuilder) -> RequestBuilder {
    match token() {
        Some(value) => builder.header("authorization", &format!("Bearer {value}")),
        None => builder,
    }
}

/// Fehlermeldungen des Servers in verständliches Deutsch übersetzen.
fn friendly(status: u16, code: &str, message: &str) -> String {
    match (status, code) {
        (413, "quota_exceeded") => "Der Speicherplatz dieses Servers ist voll.".into(),
        (413, _) => "Die Datei ist zu groß.".into(),
        (401, _) => "Anmeldung abgelaufen oder ungültig.".into(),
        (403, _) => "Dafür fehlen dir die Rechte.".into(),
        (404, _) => "Nicht gefunden – vielleicht wurde es gelöscht.".into(),
        (409, _) if message.contains("taken") => "Dieser Name ist schon vergeben.".into(),
        (409, _) if message.contains("archived") => "Der Thread ist archiviert.".into(),
        (429, _) => "Etwas langsamer bitte – Rate-Limit oder Slowmode.".into(),
        (400, _) if message.contains("password") => {
            "Das Passwort braucht mindestens 10 Zeichen.".into()
        }
        (400, _) if message.contains("handle") => {
            "Der Benutzername braucht 3–32 Zeichen: a–z, 0–9, Punkt und Unterstrich.".into()
        }
        (500.., _) => "Der Server hat einen Fehler gemeldet. Bitte später erneut versuchen.".into(),
        _ if !message.is_empty() => message.to_owned(),
        _ => format!("Anfrage fehlgeschlagen (HTTP {status})."),
    }
}

async fn send<T: DeserializeOwned>(
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<T, String> {
    let builder = match method {
        "GET" => Request::get(path),
        "POST" => Request::post(path),
        "PUT" => Request::put(path),
        "PATCH" => Request::patch(path),
        "DELETE" => Request::delete(path),
        _ => return Err(format!("unbekannte Methode {method}")),
    };
    let builder = auth_header(builder.credentials(web_sys::RequestCredentials::Include));
    let response = match body {
        Some(value) => {
            builder
                .header("content-type", "application/json")
                .body(value.to_string())
                .map_err(|err| err.to_string())?
                .send()
                .await
        }
        None => builder.send().await,
    }
    .map_err(|_| "Keine Verbindung zum Server.".to_string())?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        let parsed = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
        let code = parsed.get("code").and_then(Value::as_str).unwrap_or("");
        let message = parsed.get("message").and_then(Value::as_str).unwrap_or("");
        return Err(friendly(status, code, message));
    }
    let text = if text.is_empty() { "null" } else { &text };
    serde_json::from_str(text).map_err(|err| format!("Antwort unlesbar: {err}"))
}

async fn send_empty(method: &str, path: &str, body: Option<Value>) -> Result<(), String> {
    let _: Value = send(method, path, body).await?;
    Ok(())
}

// ------------------------------------------------------------------- Auth

pub async fn refresh() -> Result<User, String> {
    let response: AuthResponse = send("POST", "/api/v1/auth/refresh", None).await?;
    set_token(Some(response.access_token));
    Ok(response.user)
}

pub async fn login(handle: &str, password: &str, register: bool) -> Result<User, String> {
    let path = if register {
        "/api/v1/auth/register"
    } else {
        "/api/v1/auth/login"
    };
    let response: AuthResponse = send(
        "POST",
        path,
        Some(json!({ "handle": handle, "password": password })),
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

pub async fn update_profile(display_name: &str) -> Result<User, String> {
    send(
        "PATCH",
        "/api/v1/users/@me",
        Some(json!({ "display_name": display_name })),
    )
    .await
}

// ---------------------------------------------------------- Guilds/Kanäle

pub async fn guilds() -> Result<Vec<GuildSummary>, String> {
    send("GET", "/api/v1/guilds", None).await
}

pub async fn guild(id: &str) -> Result<GuildDetail, String> {
    send("GET", &format!("/api/v1/guilds/{id}"), None).await
}

pub async fn create_guild(name: &str) -> Result<Guild, String> {
    send("POST", "/api/v1/guilds", Some(json!({ "name": name }))).await
}

pub async fn create_channel(guild: &str, name: &str, kind: i16) -> Result<Channel, String> {
    send(
        "POST",
        &format!("/api/v1/guilds/{guild}/channels"),
        Some(json!({ "name": name, "kind": kind })),
    )
    .await
}

pub async fn update_channel(channel: &str, patch: Value) -> Result<Channel, String> {
    send("PATCH", &format!("/api/v1/channels/{channel}"), Some(patch)).await
}

pub async fn create_invite(guild: &str) -> Result<InviteResponse, String> {
    send(
        "POST",
        &format!("/api/v1/guilds/{guild}/invites"),
        Some(json!({})),
    )
    .await
}

pub async fn join_invite(code: &str) -> Result<(), String> {
    send_empty("POST", &format!("/api/v1/invites/{code}"), Some(json!({}))).await
}

// ------------------------------------------------------------- Nachrichten

pub async fn messages(channel: &str, before: Option<&str>) -> Result<Vec<Message>, String> {
    let path = match before {
        Some(before) => format!("/api/v1/channels/{channel}/messages?limit=50&before={before}"),
        None => format!("/api/v1/channels/{channel}/messages?limit=50"),
    };
    send("GET", &path, None).await
}

pub async fn delete_message(channel: &str, message: &str) -> Result<(), String> {
    send_empty(
        "DELETE",
        &format!("/api/v1/channels/{channel}/messages/{message}"),
        None,
    )
    .await
}

pub async fn ack(channel: &str, message: &str) -> Result<(), String> {
    send_empty(
        "POST",
        &format!("/api/v1/channels/{channel}/ack"),
        Some(json!({ "message_id": message })),
    )
    .await
}

pub async fn pins(channel: &str) -> Result<Vec<Message>, String> {
    send("GET", &format!("/api/v1/channels/{channel}/pins"), None).await
}

pub async fn set_pin(channel: &str, message: &str, pinned: bool) -> Result<(), String> {
    let path = format!("/api/v1/channels/{channel}/pins/{message}");
    if pinned {
        send_empty("PUT", &path, Some(json!({}))).await
    } else {
        send_empty("DELETE", &path, None).await
    }
}

pub async fn create_thread(channel: &str, message: &str, name: &str) -> Result<Channel, String> {
    send(
        "POST",
        &format!("/api/v1/channels/{channel}/messages/{message}/threads"),
        Some(json!({ "name": name })),
    )
    .await
}

// ----------------------------------------------------------------- Anhänge

/// Datei roh hochladen (XHR statt fetch, weil nur XHR den Upload-Fortschritt
/// meldet). `progress` bekommt Werte von 0.0 bis 1.0.
pub async fn upload(
    channel: &str,
    file: &web_sys::File,
    progress: impl FnMut(f64) + 'static,
) -> Result<Attachment, String> {
    use wasm_bindgen::{JsCast, closure::Closure};

    let path = format!(
        "/api/v1/channels/{channel}/attachments?filename={}",
        js_sys::encode_uri_component(&file.name())
    );
    let xhr = web_sys::XmlHttpRequest::new().map_err(|_| "Upload nicht möglich.".to_string())?;
    xhr.open_with_async("POST", &path, true)
        .map_err(|_| "Upload nicht möglich.".to_string())?;
    xhr.set_with_credentials(true);
    if let Some(value) = token() {
        let _ = xhr.set_request_header("authorization", &format!("Bearer {value}"));
    }
    let _ = xhr.set_request_header("content-type", "application/octet-stream");

    let mut progress = progress;
    let on_progress =
        Closure::<dyn FnMut(web_sys::ProgressEvent)>::new(move |event: web_sys::ProgressEvent| {
            if event.length_computable() && event.total() > 0.0 {
                progress((event.loaded() / event.total()).clamp(0.0, 1.0));
            }
        });
    let upload_target = xhr.upload().ok();
    if let Some(target) = &upload_target {
        target.set_onprogress(Some(on_progress.as_ref().unchecked_ref()));
    }

    let (done_tx, done_rx) = futures_channel::oneshot::channel::<bool>();
    let done_tx = std::rc::Rc::new(std::cell::RefCell::new(Some(done_tx)));
    let finish = |ok: bool| {
        let done_tx = std::rc::Rc::clone(&done_tx);
        Closure::<dyn FnMut()>::new(move || {
            if let Some(sender) = done_tx.borrow_mut().take() {
                let _ = sender.send(ok);
            }
        })
    };
    let on_load = finish(true);
    let on_error = finish(false);
    xhr.set_onload(Some(on_load.as_ref().unchecked_ref()));
    xhr.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    xhr.set_onabort(Some(on_error.as_ref().unchecked_ref()));

    xhr.send_with_opt_blob(Some(file))
        .map_err(|_| "Upload nicht möglich.".to_string())?;
    let loaded = done_rx.await.unwrap_or(false);

    // Handler lösen, bevor die Closures fallen.
    if let Some(target) = &upload_target {
        target.set_onprogress(None);
    }
    xhr.set_onload(None);
    xhr.set_onerror(None);
    xhr.set_onabort(None);
    drop((on_progress, on_load, on_error));

    if !loaded {
        return Err("Upload abgebrochen – keine Verbindung zum Server.".into());
    }
    let status = xhr.status().unwrap_or(0);
    let text = xhr.response_text().ok().flatten().unwrap_or_default();
    if !(200..300).contains(&status) {
        let parsed = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
        let code = parsed.get("code").and_then(Value::as_str).unwrap_or("");
        let message = parsed.get("message").and_then(Value::as_str).unwrap_or("");
        return Err(friendly(status, code, message));
    }
    serde_json::from_str(&text).map_err(|err| format!("Antwort unlesbar: {err}"))
}

/// Noch nicht gesendeten Upload verwerfen.
pub async fn discard_attachment(id: &str) -> Result<(), String> {
    send_empty("DELETE", &format!("/api/v1/attachments/{id}"), None).await
}

// -------------------------------------------------------------- Moderation

pub async fn kick(guild: &str, user: &str, reason: &str) -> Result<(), String> {
    send_empty(
        "POST",
        &format!("/api/v1/guilds/{guild}/members/{user}/kick"),
        Some(json!({ "reason": reason })),
    )
    .await
}

pub async fn ban(guild: &str, user: &str, reason: &str) -> Result<(), String> {
    send_empty(
        "PUT",
        &format!("/api/v1/guilds/{guild}/bans/{user}"),
        Some(json!({ "reason": reason })),
    )
    .await
}

pub async fn unban(guild: &str, user: &str) -> Result<(), String> {
    send_empty(
        "DELETE",
        &format!("/api/v1/guilds/{guild}/bans/{user}"),
        None,
    )
    .await
}

pub async fn bans(guild: &str) -> Result<Vec<Ban>, String> {
    send("GET", &format!("/api/v1/guilds/{guild}/bans"), None).await
}

pub async fn timeout(guild: &str, user: &str, seconds: i64) -> Result<(), String> {
    send_empty(
        "PUT",
        &format!("/api/v1/guilds/{guild}/members/{user}/timeout"),
        Some(json!({ "seconds": seconds })),
    )
    .await
}

pub async fn audit_log(guild: &str) -> Result<Vec<AuditEntry>, String> {
    send("GET", &format!("/api/v1/guilds/{guild}/audit-log"), None).await
}

pub async fn create_role(guild: &str, name: &str) -> Result<Role, String> {
    send(
        "POST",
        &format!("/api/v1/guilds/{guild}/roles"),
        Some(json!({ "name": name, "permissions": 0, "position": 1 })),
    )
    .await
}

pub async fn update_role(guild: &str, role: &str, patch: Value) -> Result<Role, String> {
    send(
        "PATCH",
        &format!("/api/v1/guilds/{guild}/roles/{role}"),
        Some(patch),
    )
    .await
}

pub async fn delete_role(guild: &str, role: &str) -> Result<(), String> {
    send_empty(
        "DELETE",
        &format!("/api/v1/guilds/{guild}/roles/{role}"),
        None,
    )
    .await
}

pub async fn set_member_role(guild: &str, user: &str, role: &str, on: bool) -> Result<(), String> {
    let path = format!("/api/v1/guilds/{guild}/members/{user}/roles/{role}");
    if on {
        send_empty("PUT", &path, Some(json!({}))).await
    } else {
        send_empty("DELETE", &path, None).await
    }
}

// ------------------------------------------------------------------- Voice

pub async fn voice_join(channel: &str, sdp: &str) -> Result<VoiceJoinResponse, String> {
    send(
        "POST",
        &format!("/api/v1/channels/{channel}/voice/join"),
        Some(json!({ "sdp": sdp })),
    )
    .await
}

pub async fn voice_leave(channel: &str) -> Result<(), String> {
    send_empty(
        "POST",
        &format!("/api/v1/channels/{channel}/voice/leave"),
        Some(json!({})),
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

pub async fn voice_state(
    channel: &str,
    muted: bool,
    deafened: bool,
    screen: bool,
    camera: bool,
) -> Result<(), String> {
    send_empty(
        "POST",
        &format!("/api/v1/channels/{channel}/voice/state"),
        Some(json!({ "muted": muted, "deafened": deafened, "screen": screen, "camera": camera })),
    )
    .await
}
