//! Lokale Einstellungen: `localStorage` lesen/schreiben und Theme sowie
//! Schriftgröße auf `<html>` anwenden.

use crate::types::{Settings, Theme};

const KEY: &str = "nova.settings";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub fn load() -> Settings {
    let settings = storage()
        .and_then(|storage| storage.get_item(KEY).ok().flatten())
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
        .unwrap_or_default();
    apply(&settings);
    settings
}

pub fn save(settings: &Settings) {
    if let (Some(storage), Ok(text)) = (storage(), serde_json::to_string(settings)) {
        let _ = storage.set_item(KEY, &text);
    }
    apply(settings);
}

/// `data-theme` steuert das Theme (ohne Attribut folgt es dem System),
/// `--nv-scale` die Schriftgröße.
pub fn apply(settings: &Settings) {
    let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    else {
        return;
    };
    match settings.theme {
        Theme::System => {
            let _ = root.remove_attribute("data-theme");
        }
        Theme::Dark => {
            let _ = root.set_attribute("data-theme", "dark");
        }
        Theme::Light => {
            let _ = root.set_attribute("data-theme", "light");
        }
    }
    let scale = settings.font_scale.clamp(90, 125);
    let _ = root.set_attribute(
        "style",
        &format!("--nv-scale: {}", f64::from(scale) / 100.0),
    );
}

/// Desktop-Benachrichtigung, wenn erlaubt und das Fenster nicht im Vordergrund ist.
pub fn notify(title: &str, body: &str) {
    let hidden = web_sys::window()
        .and_then(|window| window.document())
        .map(|document| document.hidden())
        .unwrap_or(false);
    if !hidden || web_sys::Notification::permission() != web_sys::NotificationPermission::Granted {
        return;
    }
    let options = web_sys::NotificationOptions::new();
    options.set_body(body);
    let _ = web_sys::Notification::new_with_options(title, &options);
}

pub async fn request_notification_permission() -> bool {
    let Ok(promise) = web_sys::Notification::request_permission() else {
        return false;
    };
    match wasm_bindgen_futures::JsFuture::from(promise).await {
        Ok(value) => value.as_string().as_deref() == Some("granted"),
        Err(_) => false,
    }
}
