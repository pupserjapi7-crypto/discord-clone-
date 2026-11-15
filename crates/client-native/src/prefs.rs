//! Lokale Einstellungen des nativen Clients (Audio-Geräte, Push-to-Talk).
//!
//! Liegen als JSON neben der Sitzung unter `%APPDATA%\nova\native.json`
//! (Linux: `$XDG_CONFIG_HOME/nova` bzw. `~/.config/nova`). Fehlt die Datei
//! oder ist sie kaputt, gelten die Standardwerte.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Mikrofon nach Gerätename; `None` = Systemstandard.
    pub input: Option<String>,
    /// Lautsprecher nach Gerätename; `None` = Systemstandard.
    pub output: Option<String>,
    /// Push-to-Talk statt offenem Mikrofon.
    pub push_to_talk: bool,
    /// Physische Taste (Debug-Text von `keyboard::key::Physical`).
    pub ptt_key: Option<String>,
}

fn file() -> Option<PathBuf> {
    let dir = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(dir.join("nova").join("native.json"))
}

impl Prefs {
    pub fn load() -> Self {
        file()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = file() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, bytes);
        }
    }
}

/// Lesbarer Name einer Taste aus `Physical`-Debug-Text, z. B. `Code(F13)` → `F13`.
pub fn key_label(raw: &str) -> String {
    raw.strip_prefix("Code(")
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(raw)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_label_strips_wrapper() {
        assert_eq!(key_label("Code(F13)"), "F13");
        assert_eq!(key_label("Unidentified"), "Unidentified");
    }

    #[test]
    fn prefs_roundtrip() {
        let prefs = Prefs {
            input: Some("Mikro".into()),
            output: None,
            push_to_talk: true,
            ptt_key: Some("Code(KeyV)".into()),
        };
        let json = serde_json::to_vec(&prefs).expect("json");
        let back: Prefs = serde_json::from_slice(&json).expect("parse");
        assert_eq!(prefs, back);
        // Ältere/leere Dateien laden mit Standardwerten.
        let empty: Prefs = serde_json::from_slice(b"{}").expect("leer");
        assert_eq!(empty, Prefs::default());
    }
}
