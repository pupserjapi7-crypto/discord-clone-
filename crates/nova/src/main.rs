//! `nova` — single binary for the API, the gateway and the SFU (M2).
//!
//! `serve` hosts API and gateway in one process for milestone 1. The role flag is
//! already accepted so deployment can split them later without a code change.

use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use axum::{
    Router,
    body::Body,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use clap::{Parser, Subcommand};
use nova_api::{AppState, state::RateLimiter};
use nova_common::{config::Config, ids::Snowflake, metrics::Metrics, tokens::TokenStore};
use nova_db::Db;
use nova_gateway::Gateway;
use nova_sfu::Sfu;
use rust_embed::RustEmbed;
use serde_json::json;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::sync::Semaphore;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod update;

#[cfg(not(target_os = "windows"))]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(RustEmbed)]
#[folder = "../../crates/ui/dist/"]
struct Assets;

#[derive(Parser)]
#[command(name = "nova", version, about = "Project Nova server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (API + gateway).
    Serve {
        #[arg(long, default_value = "all")]
        role: String,
    },
    /// Apply database migrations and exit.
    Migrate,
    /// Seed load-test users, a guild and a channel. Idempotent for users.
    Seed {
        #[arg(long, default_value_t = 120)]
        users: usize,
        #[arg(long, default_value = "loadtest")]
        guild: String,
        #[arg(long, default_value = "loadtest-password")]
        password: String,
    },
    /// Print configuration and connectivity diagnostics.
    Doctor,
    /// Update this binary from the latest GitHub release.
    Update {
        /// Only report whether a newer release exists.
        #[arg(long)]
        check: bool,
        /// Repository in `owner/name` form.
        #[arg(long)]
        repo: Option<String>,
        /// Do not restart the systemd unit after replacing the binary.
        #[arg(long)]
        no_restart: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve { role } => serve(&role).await,
        Command::Migrate => {
            let cfg = Config::from_env()?;
            init_tracing(&cfg.log_filter);
            let db = Db::connect(&cfg.database_url, 5).await?;
            db.migrate().await?;
            info!("migrations applied");
            Ok(())
        }
        Command::Seed {
            users,
            guild,
            password,
        } => seed(users, &guild, &password).await,
        Command::Doctor => doctor().await,
        Command::Update {
            check,
            repo,
            no_restart,
        } => update::run(check, repo, no_restart).await,
    }
}

fn init_tracing(filter: &str) {
    let filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

async fn serve(role: &str) -> Result<()> {
    let cfg = Arc::new(Config::from_env()?);
    init_tracing(&cfg.log_filter);
    if role != "all" {
        warn!(
            role,
            "milestone 1 only implements --role all; starting everything"
        );
    }

    let blobs = nova_api::blob::LocalFs::open(&cfg.upload_dir)
        .await
        .with_context(|| {
            format!(
                "open upload dir {:?} (NOVA_UPLOAD_DIR, must be writable)",
                cfg.upload_dir
            )
        })?;

    let db = Db::connect(&cfg.database_url, 10).await?;
    db.migrate().await?;

    let metrics = Arc::new(Metrics::default());
    let tokens = Arc::new(TokenStore::new(cfg.access_ttl));
    let ids = Arc::new(Snowflake::new(cfg.worker_id));
    let gateway = Gateway::new(
        db.clone(),
        Arc::clone(&cfg),
        Arc::clone(&tokens),
        Arc::clone(&metrics),
        Arc::clone(&ids),
    );
    gateway.spawn_janitor();

    let limiter = Arc::new(RateLimiter::new());
    let sfu = Arc::new(Sfu::from_env());
    let state = AppState {
        db,
        cfg: Arc::clone(&cfg),
        tokens,
        metrics,
        ids,
        gateway: Arc::clone(&gateway),
        sfu,
        started: Instant::now(),
        auth_limiter: Arc::clone(&limiter),
        hash_limit: Arc::new(Semaphore::new(cfg.hash_concurrency)),
        blobs: Arc::new(blobs),
        blob_gc: Arc::new(tokio::sync::RwLock::new(())),
        media_limit: Arc::new(Semaphore::new(2)),
    };
    nova_api::uploads::spawn_janitor(state.clone());
    nova_api::spawn_sfu_events(state.clone());

    // Janitor for auth rate-limit buckets.
    {
        let limiter = Arc::clone(&limiter);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(300));
            loop {
                tick.tick().await;
                limiter.sweep();
            }
        });
    }

    let page = Arc::new(PageHeaders::new(&cfg.public_origin));
    let app = nova_api::router(state).merge(gateway.router()).fallback(
        move |uri: Uri, headers: axum::http::HeaderMap| {
            let page = Arc::clone(&page);
            async move { static_asset(&uri, &headers, &page) }
        },
    );

    let listener = bind_listener(cfg.bind_addr, cfg.listen_backlog)?;
    info!(
        addr = %listener.local_addr()?,
        backlog = cfg.listen_backlog,
        hash_concurrency = cfg.hash_concurrency,
        cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "nova listening (api + gateway)"
    );

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("serve")?;
    info!("shutdown complete");
    Ok(())
}

/// Bind with an explicit backlog. `std`'s default (128) resets SYNs during a
/// thundering herd of reconnects; 4096 held 1.000 simultaneous sockets locally.
fn bind_listener(addr: SocketAddr, backlog: u32) -> Result<tokio::net::TcpListener> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket =
        Socket::new(domain, Type::STREAM, Some(Protocol::TCP)).context("create listener socket")?;
    #[cfg(unix)]
    socket.set_reuse_address(true).context("SO_REUSEADDR")?;
    socket
        .bind(&addr.into())
        .with_context(|| format!("bind {addr}"))?;
    socket
        .listen(i32::try_from(backlog).unwrap_or(4096))
        .context("listen")?;
    socket.set_nonblocking(true).context("nonblocking")?;
    let listener: std::net::TcpListener = socket.into();
    tokio::net::TcpListener::from_std(listener).context("register listener")
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("shutdown signal received");
}

/// Sicherheits-Header für die Web-App (gilt auch in der Tauri-Hülle, die die
/// Server-URL lädt).
struct PageHeaders {
    csp: HeaderValue,
}

impl PageHeaders {
    fn new(public_origin: &str) -> Self {
        Self {
            csp: HeaderValue::from_str(&content_security_policy(public_origin))
                .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'")),
        }
    }
}

/// CSP der Web-App. WASM braucht `wasm-unsafe-eval` (nicht `unsafe-eval`);
/// Dioxus setzt Style-Attribute, daher `unsafe-inline` nur für Styles.
/// `connect-src` nennt Gateway-URL zusätzlich ausdrücklich, weil ältere
/// Browser `'self'` nicht auf `wss:` anwenden.
fn content_security_policy(public_origin: &str) -> String {
    let origin = public_origin.trim_end_matches('/');
    let socket = origin
        .strip_prefix("https://")
        .map(|host| format!(" wss://{host}"))
        .or_else(|| {
            origin
                .strip_prefix("http://")
                .map(|host| format!(" ws://{host}"))
        })
        .unwrap_or_default();
    format!(
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
         style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
         media-src 'self' blob:; font-src 'self'; connect-src 'self'{socket}; \
         object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"
    )
}

fn static_asset(uri: &Uri, request: &axum::http::HeaderMap, page: &PageHeaders) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    let file = Assets::get(path).map(|file| (path, file));
    // SPA fallback for client-side routes.
    let Some((path, file)) = file.or_else(|| Assets::get("index.html").map(|f| ("index.html", f)))
    else {
        return (
            StatusCode::NOT_FOUND,
            "frontend not built: run `cargo xtask ui`",
        )
            .into_response();
    };
    let etag = format!("\"{}\"", hex::encode(file.metadata.sha256_hash()));
    let fresh = request
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|tag| tag.trim() == etag));
    let mut response = if fresh {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Response::new(Body::from(file.data.into_owned()))
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(path)),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control(path)),
    );
    if let Ok(value) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, value);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if path.ends_with(".html") {
        headers.insert(header::CONTENT_SECURITY_POLICY, page.csp.clone());
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        );
        headers.insert(
            "permissions-policy",
            HeaderValue::from_static(
                "camera=(self), microphone=(self), display-capture=(self), geolocation=()",
            ),
        );
    }
    response
}

/// `dx` hängt einen Inhalts-Hash an (`nova-ui-dxh…`): solche Dateien ändern
/// sich nie und dürfen ein Jahr im Cache bleiben. Alles andere wird per ETag
/// revalidiert (304 statt 1,3 MB WASM bei jedem Laden).
fn cache_control(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    if path.starts_with("assets/") && name.contains("-dxh") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

async fn seed(users: usize, guild_name: &str, password: &str) -> Result<()> {
    let cfg = Config::from_env()?;
    init_tracing(&cfg.log_filter);
    let db = Db::connect(&cfg.database_url, 5).await?;
    db.migrate().await?;

    let params = Params::new(
        cfg.argon2_m_cost,
        cfg.argon2_t_cost,
        cfg.argon2_p_cost,
        Some(32),
    )
    .map_err(|err| anyhow::anyhow!("argon2 params: {err}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| anyhow::anyhow!("hash: {err}"))?
        .to_string();

    let ids = Snowflake::new(cfg.worker_id);
    let mut user_ids = Vec::with_capacity(users);
    for index in 0..users {
        let handle = format!("load{index:04}");
        if let Some((user, _)) = db.user_with_hash(&handle).await? {
            user_ids.push(user.id);
            continue;
        }
        let user = db
            .create_user(ids.next_id(), &handle, &format!("Load {index:04}"), &hash)
            .await?;
        user_ids.push(user.id);
    }

    let guild = db
        .create_guild(
            ids.next_id(),
            guild_name,
            *user_ids.first().context("seed needs at least one user")?,
            ids.next_id(),
            ids.next_id(),
            ids.next_id(),
        )
        .await?;
    db.add_members(guild.id, &user_ids).await?;

    let channels = db.channels(guild.id).await?;
    let channel = channels.first().context("default channel missing")?;
    let code = format!("load{}", guild.id);
    db.create_invite(&code, guild.id, user_ids[0]).await.ok();

    println!(
        "{}",
        json!({
            "users": users,
            "handle_prefix": "load",
            "password": password,
            "guild_id": guild.id,
            "channel_id": channel.id,
            "invite": code,
        })
    );
    Ok(())
}

async fn doctor() -> Result<()> {
    let cfg = Config::from_env()?;
    init_tracing(&cfg.log_filter);
    let db = Db::connect(&cfg.database_url, 2).await?;
    let started = Instant::now();
    db.ping().await?;
    let ping_ms = started.elapsed().as_millis();

    let users: i64 = sqlx::query_scalar("select count(*) from users")
        .fetch_one(&db.pool)
        .await?;
    let guilds: i64 = sqlx::query_scalar("select count(*) from guilds")
        .fetch_one(&db.pool)
        .await?;
    let messages: i64 = sqlx::query_scalar("select count(*) from messages")
        .fetch_one(&db.pool)
        .await?;
    let (attachments, attachment_bytes): (i64, i64) =
        sqlx::query_as("select count(*), coalesce(sum(size), 0)::bigint from attachments")
            .fetch_one(&db.pool)
            .await
            .unwrap_or((0, 0));
    // Schreibprobe im Upload-Verzeichnis (dieselbe Prüfung wie beim Start).
    let upload_dir_writable = {
        let probe = cfg.upload_dir.join("tmp").join(".doctor-probe");
        let ok = tokio::fs::create_dir_all(cfg.upload_dir.join("tmp"))
            .await
            .is_ok()
            && tokio::fs::write(&probe, b"ok").await.is_ok();
        let _ = tokio::fs::remove_file(&probe).await;
        ok
    };

    println!(
        "{}",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "bind_addr": cfg.bind_addr.to_string(),
            "public_origin": cfg.public_origin,
            "database_ping_ms": ping_ms,
            "users": users,
            "guilds": guilds,
            "messages": messages,
            "upload_dir": cfg.upload_dir,
            "upload_dir_writable": upload_dir_writable,
            "attachments": attachments,
            "attachment_bytes": attachment_bytes,
            "max_upload_bytes": cfg.max_upload_bytes,
            "guild_upload_quota_bytes": cfg.guild_upload_quota,
            "argon2_m_cost_kib": cfg.argon2_m_cost,
        })
    );
    Ok(())
}

/// Keep `Router` referenced so `serve` stays explicit about its type.
#[allow(dead_code)]
fn _router_type(state: AppState) -> Router {
    nova_api::router(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csp_allows_wasm_and_own_gateway_only() {
        let csp = content_security_policy("https://nova.example.com/");
        assert!(csp.contains("script-src 'self' 'wasm-unsafe-eval';"));
        assert!(!csp.contains("'unsafe-eval'"));
        assert!(csp.contains("connect-src 'self' wss://nova.example.com;"));
        assert!(csp.contains("frame-ancestors 'none'"));
        let local = content_security_policy("http://127.0.0.1:8080");
        assert!(local.contains("connect-src 'self' ws://127.0.0.1:8080;"));
    }

    #[test]
    fn hashed_assets_are_immutable() {
        assert_eq!(
            cache_control("assets/nova-ui_bg-dxh49d97b95977e67a0.wasm"),
            "public, max-age=31536000, immutable"
        );
        assert_eq!(cache_control("assets/app.css"), "no-cache");
        assert_eq!(cache_control("index.html"), "no-cache");
    }
}
