//! Selbst-Update aus GitHub Releases.
//!
//! Ablauf: `releases/latest` abfragen → passende Datei plus `.sha256` laden →
//! Prüfsumme vergleichen → eigene Exe austauschen → neu starten.
//!
//! Windows sperrt eine laufende Exe gegen Überschreiben, erlaubt aber das
//! Umbenennen. Deshalb: `nova-desktop.exe` → `nova-desktop.exe.old`, neue Datei
//! an den alten Namen, neuen Prozess starten, alten beenden. Die `.old`-Datei
//! räumt der nächste Start weg.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Wird im Release-Workflow über `NOVA_UPDATE_REPO` gesetzt.
const DEFAULT_REPO: &str = "pupserjapi7-crypto/discord-clone-";

#[cfg(windows)]
const ASSET: &str = "nova-desktop-windows-x86_64.exe";
#[cfg(not(windows))]
const ASSET: &str = "nova-desktop-linux-x86_64";

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct Available {
    pub version: String,
    binary_url: String,
    checksum_url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn repo() -> String {
    std::env::var("NOVA_UPDATE_REPO")
        .ok()
        .or_else(|| option_env!("NOVA_UPDATE_REPO").map(str::to_owned))
        .unwrap_or_else(|| DEFAULT_REPO.to_owned())
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("nova-desktop/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|err| err.to_string())
}

/// `Ok(None)` heißt: aktuell, oder das neueste Release hat noch keinen
/// nativen Client für diese Plattform.
pub async fn check() -> Result<Option<Available>, String> {
    let release: Release = client()?
        .get(format!(
            "https://api.github.com/repos/{}/releases/latest",
            repo()
        ))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|err| format!("GitHub nicht erreichbar: {err}"))?
        .error_for_status()
        .map_err(|err| format!("GitHub-Antwort: {err}"))?
        .json()
        .await
        .map_err(|err| format!("Release unlesbar: {err}"))?;

    let version = release.tag_name.trim_start_matches('v').to_owned();
    if !is_newer(&version, CURRENT) {
        return Ok(None);
    }
    let url = |name: &str| {
        release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.clone())
    };
    let (Some(binary_url), Some(checksum_url)) = (url(ASSET), url(&format!("{ASSET}.sha256")))
    else {
        return Ok(None);
    };
    Ok(Some(Available {
        version,
        binary_url,
        checksum_url,
    }))
}

/// Lädt, prüft und installiert. Bei Erfolg startet die neue Version und dieser
/// Prozess beendet sich — die Funktion kehrt dann nicht zurück.
pub async fn install(update: Available) -> Result<(), String> {
    let http = client()?;
    let download = async |url: &str| -> Result<Vec<u8>, String> {
        let response = http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/octet-stream")
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|err| format!("Download fehlgeschlagen: {err}"))?;
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|err| format!("Download abgebrochen: {err}"))
    };
    let binary = download(&update.binary_url).await?;
    let checksum = String::from_utf8(download(&update.checksum_url).await?)
        .map_err(|_| "Prüfsummen-Datei ist kein Text".to_owned())?;
    verify(&binary, &checksum)?;

    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    replace(&exe, &binary).map_err(|err| format!("Austausch fehlgeschlagen: {err}"))?;

    std::process::Command::new(&exe)
        .spawn()
        .map_err(|err| format!("Neustart fehlgeschlagen: {err}"))?;
    std::process::exit(0);
}

/// Reste eines früheren Updates entfernen.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
    }
}

fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

fn replace(exe: &Path, binary: &[u8]) -> std::io::Result<()> {
    let mut staged = exe.as_os_str().to_owned();
    staged.push(".new");
    let staged = PathBuf::from(staged);
    std::fs::write(&staged, binary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    let old = old_path(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old)?;
    if let Err(err) = std::fs::rename(&staged, exe) {
        // Zurückrollen, damit die App startfähig bleibt.
        let _ = std::fs::rename(&old, exe);
        return Err(err);
    }
    Ok(())
}

fn verify(binary: &[u8], checksum_text: &str) -> Result<(), String> {
    let expected = checksum_text
        .split_whitespace()
        .next()
        .ok_or("Prüfsummen-Datei ist leer")?
        .to_ascii_lowercase();
    let actual = format!("{:x}", Sha256::digest(binary));
    if expected == actual {
        Ok(())
    } else {
        Err("Prüfsumme stimmt nicht — Update verworfen.".to_owned())
    }
}

fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(version: &str) -> Vec<u64> {
        version
            .split(['.', '-', '+'])
            .map_while(|part| part.parse().ok())
            .collect()
    }
    let (latest, current) = (parse(latest), parse(current));
    for index in 0..latest.len().max(current.len()) {
        let left = latest.get(index).copied().unwrap_or(0);
        let right = current.get(index).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("0.3.0", "0.2.2"));
        assert!(!is_newer("0.2.2", "0.2.2"));
        assert!(!is_newer("0.2.1", "0.2.2"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(verify(b"nova", "00  nova").is_err());
        let good = format!("{:x}  nova", Sha256::digest(b"nova"));
        assert!(verify(b"nova", &good).is_ok());
    }

    #[test]
    fn replace_swaps_file_and_keeps_old() {
        let dir = std::env::temp_dir().join(format!("nova-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("app.bin");
        std::fs::write(&exe, b"alt").unwrap();
        replace(&exe, b"neu").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"neu");
        assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"alt");
        let _ = std::fs::remove_dir_all(dir);
    }
}
