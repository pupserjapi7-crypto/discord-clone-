use std::{env, net::SocketAddr, path::PathBuf, str::FromStr, time::Duration};

use anyhow::{Context, Result, anyhow};

/// Runtime configuration, read once at startup and then shared immutably.
///
/// Environment variables are accepted with a `NOVA_` prefix first and the plain
/// name second, so container-style overrides (`DATABASE_URL`) keep working.
#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: SocketAddr,
    pub public_origin: String,
    pub upload_dir: PathBuf,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
    pub max_message_len: usize,
    pub max_upload_bytes: usize,
    pub max_avatar_bytes: usize,
    pub argon2_m_cost: u32,
    pub argon2_t_cost: u32,
    pub argon2_p_cost: u32,
    pub heartbeat_timeout: Duration,
    pub resume_ttl: Duration,
    /// `listen(2)` backlog. The std default (128) drops SYNs under bursts.
    pub listen_backlog: u32,
    /// Maximum concurrent Argon2 hashes. One per core minus one leaves the
    /// gateway and HTTP handlers responsive on a 2-vCore box.
    pub hash_concurrency: usize,
    /// Per-session message token bucket (rate per second, burst).
    pub msg_rate: f64,
    pub msg_burst: f64,
    pub worker_id: u64,
    pub dev_mode: bool,
    pub trust_proxy: bool,
    /// Token bucket for auth endpoints: burst and refill rate per second.
    /// Raise both for load tests (`NOVA_AUTH_BURST`, `NOVA_AUTH_RATE`).
    pub auth_burst: f64,
    pub auth_rate: f64,
    pub log_filter: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url = env_any(&["NOVA_DATABASE_URL", "DATABASE_URL"])
            .ok_or_else(|| anyhow!("DATABASE_URL (or NOVA_DATABASE_URL) is required"))?;
        let bind_addr_raw =
            env_any(&["NOVA_BIND_ADDR"]).unwrap_or_else(|| "127.0.0.1:8080".to_owned());
        let bind_addr = bind_addr_raw
            .parse::<SocketAddr>()
            .with_context(|| format!("invalid NOVA_BIND_ADDR: {bind_addr_raw}"))?;

        let default_hash_concurrency = std::thread::available_parallelism()
            .map(|cores| cores.get().saturating_sub(1).max(1))
            .unwrap_or(1);

        Ok(Self {
            database_url,
            bind_addr,
            public_origin: env_any(&["NOVA_PUBLIC_ORIGIN"])
                .unwrap_or_else(|| "http://localhost:5173".to_owned()),
            upload_dir: PathBuf::from(
                env_any(&["NOVA_UPLOAD_DIR"]).unwrap_or_else(|| "./data/uploads".to_owned()),
            ),
            access_ttl: Duration::from_secs(parse_or("NOVA_ACCESS_TTL_SECS", 900)?),
            refresh_ttl: Duration::from_secs(parse_or::<u64>(
                "NOVA_REFRESH_TTL_SECS",
                30 * 86_400,
            )?),
            max_message_len: parse_or("NOVA_MAX_MESSAGE_LEN", 8_000)?,
            max_upload_bytes: parse_or("NOVA_MAX_UPLOAD_BYTES", 25 * 1024 * 1024)?,
            max_avatar_bytes: parse_or("NOVA_MAX_AVATAR_BYTES", 2 * 1024 * 1024)?,
            argon2_m_cost: parse_or("NOVA_ARGON2_M_COST", 19_456)?,
            argon2_t_cost: parse_or("NOVA_ARGON2_T_COST", 2)?,
            argon2_p_cost: parse_or("NOVA_ARGON2_P_COST", 1)?,
            heartbeat_timeout: Duration::from_secs(parse_or("NOVA_HEARTBEAT_TIMEOUT_SECS", 45)?),
            resume_ttl: Duration::from_secs(parse_or("NOVA_RESUME_TTL_SECS", 120)?),
            listen_backlog: parse_or("NOVA_LISTEN_BACKLOG", 4096)?,
            hash_concurrency: parse_or("NOVA_HASH_CONCURRENCY", default_hash_concurrency)?,
            msg_rate: parse_or("NOVA_MSG_RATE", 1.0)?,
            msg_burst: parse_or("NOVA_MSG_BURST", 10.0)?,
            worker_id: parse_or("NOVA_WORKER_ID", 1)?,
            dev_mode: parse_or("NOVA_DEV_MODE", false)?,
            trust_proxy: parse_or("NOVA_TRUST_PROXY", false)?,
            auth_burst: parse_or("NOVA_AUTH_BURST", 10.0)?,
            auth_rate: parse_or("NOVA_AUTH_RATE", 0.2)?,
            log_filter: env_any(&["NOVA_LOG"]).unwrap_or_else(|| "info,nova=debug".to_owned()),
        })
    }

    /// Cookies get `Secure` only when we are actually served over TLS.
    pub fn secure_cookies(&self) -> bool {
        self.public_origin.starts_with("https://")
    }

    pub fn is_dev(&self) -> bool {
        self.dev_mode
    }
}

fn env_any(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    })
}

fn parse_or<T>(name: &str, default: T) -> Result<T>
where
    T: FromStr + std::fmt::Debug,
    T::Err: std::fmt::Display,
{
    match env_any(&[name]) {
        None => Ok(default),
        Some(raw) => raw
            .parse::<T>()
            .map_err(|err| anyhow!("invalid {name}={raw}: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_or_uses_default_when_unset() {
        assert_eq!(parse_or::<u64>("NOVA_DEFINITELY_UNSET_VAR", 7).unwrap(), 7);
    }
}
