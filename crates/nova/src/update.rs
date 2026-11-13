//! Self-update from GitHub Releases.
//!
//! The binary downloads the raw musl asset, verifies its SHA-256 against the
//! checksum asset from the same release and atomically replaces itself.

// The installer path is Unix-only; the Windows build still compiles and tests
// version comparison, so unused helpers are expected there.
#![cfg_attr(not(unix), allow(dead_code))]

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const ASSET: &str = "nova-linux-x86_64";
const ASSET_SUM: &str = "nova-linux-x86_64.sha256";

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub async fn run(check_only: bool, repo: Option<String>, no_restart: bool) -> Result<()> {
    let repo = repo
        .or_else(|| std::env::var("NOVA_UPDATE_REPO").ok())
        .or_else(|| option_env!("NOVA_UPDATE_REPO").map(str::to_owned))
        .context(
            "no update repository configured: pass --repo owner/name or set NOVA_UPDATE_REPO",
        )?;
    if !repo.contains('/') {
        bail!("invalid repository '{repo}', expected owner/name");
    }

    let client = reqwest::Client::builder()
        .user_agent(concat!("nova/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()?;

    let release: Release = client
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .send()
        .await
        .with_context(|| format!("query latest release of {repo}"))?
        .error_for_status()
        .context("GitHub release request failed (is the repo public?)")?
        .json()
        .await
        .context("decode GitHub release")?;

    let current = env!("CARGO_PKG_VERSION");
    let latest = release.tag_name.trim_start_matches('v').to_owned();
    println!("current={current} latest={latest}");

    if !is_newer(&latest, current) {
        println!("already up to date");
        return Ok(());
    }
    if check_only {
        println!("update available: {latest}");
        return Ok(());
    }

    install(&client, &release, &latest, no_restart).await
}

#[cfg(unix)]
async fn install(
    client: &reqwest::Client,
    release: &Release,
    latest: &str,
    no_restart: bool,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let asset = find_asset(release, ASSET)?;
    let checksum = find_asset(release, ASSET_SUM)?;
    let binary = download(client, &asset.browser_download_url).await?;
    let checksum_text = String::from_utf8(download(client, &checksum.browser_download_url).await?)
        .context("checksum file is not UTF-8")?;
    verify_checksum(&binary, &checksum_text)?;

    let exe = std::env::current_exe().context("resolve current executable")?;
    let staged = exe.with_extension("new");
    tokio::fs::write(&staged, &binary)
        .await
        .with_context(|| format!("write {staged:?}"))?;
    tokio::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
        .await
        .context("chmod staged binary")?;
    tokio::fs::rename(&staged, &exe)
        .await
        .context("atomically replace binary")?;
    println!("updated to {latest}");

    if !no_restart {
        match tokio::process::Command::new("systemctl")
            .args(["restart", "nova"])
            .status()
            .await
        {
            Ok(status) if status.success() => println!("nova restarted"),
            _ => println!("restart manually: systemctl restart nova"),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
async fn install(
    _client: &reqwest::Client,
    _release: &Release,
    _latest: &str,
    _no_restart: bool,
) -> Result<()> {
    bail!("self-update is only supported on Linux; use install.sh on the server")
}

fn find_asset<'a>(release: &'a Release, name: &str) -> Result<&'a Asset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .with_context(|| format!("release {} has no asset {name}", release.tag_name))
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .with_context(|| format!("download {url}"))?
        .error_for_status()?;
    Ok(response.bytes().await?.to_vec())
}

fn verify_checksum(binary: &[u8], checksum_text: &str) -> Result<()> {
    let expected = checksum_text
        .split_whitespace()
        .next()
        .context("empty checksum file")?
        .to_ascii_lowercase();
    let actual = format!("{:x}", Sha256::digest(binary));
    if expected != actual {
        bail!("checksum mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(version: &str) -> Vec<u64> {
        version
            .split(['.', '-', '+'])
            .map_while(|part| part.parse::<u64>().ok())
            .collect()
    }
    let latest = parse(latest);
    let current = parse(current);
    let len = latest.len().max(current.len());
    for index in 0..len {
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
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn rejects_a_wrong_checksum() {
        assert!(verify_checksum(b"nova", "00  nova").is_err());
        let good = format!("{:x}  nova", Sha256::digest(b"nova"));
        assert!(verify_checksum(b"nova", &good).is_ok());
    }
}
