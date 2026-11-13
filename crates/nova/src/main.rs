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
#[folder = "../../web/build/"]
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

    tokio::fs::create_dir_all(&cfg.upload_dir)
        .await
        .with_context(|| format!("create upload dir {:?}", cfg.upload_dir))?;

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
    let state = AppState {
        db,
        cfg: Arc::clone(&cfg),
        tokens,
        metrics,
        ids,
        gateway: Arc::clone(&gateway),
        started: Instant::now(),
        auth_limiter: Arc::clone(&limiter),
        hash_limit: Arc::new(Semaphore::new(cfg.hash_concurrency)),
    };

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

    let app = nova_api::router(state)
        .merge(gateway.router())
        .fallback(static_asset);

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

async fn static_asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = Assets::get(path) {
        return asset_response(path, file.data.into_owned());
    }
    // SPA fallback for client-side routes.
    match Assets::get("index.html") {
        Some(file) => asset_response("index.html", file.data.into_owned()),
        None => (
            StatusCode::NOT_FOUND,
            "frontend not built: run `npm run build` in web/",
        )
            .into_response(),
    }
}

fn asset_response(path: &str, data: Vec<u8>) -> Response {
    let cache = if path.starts_with("_app/immutable/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut response = Response::new(Body::from(data));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(path)),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
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
