//! WebSocket load generator.
//!
//! Roles run as separate processes so the server is measured without the
//! generator competing for the same core:
//!
//! ```text
//! nova-loadtest --mode receivers --receivers 1000 --offset 20 --channel <id>
//! nova-loadtest --mode senders   --senders 20     --offset 0  --channel <id> --msg-rate 1
//! nova-loadtest --mode login     --users 200
//! nova-loadtest --mode slow      --channel <id> --duration 15
//! ```

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use clap::{Parser, ValueEnum};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use nova_common::protocol::{
    self, ClientOp, Event, OP_ACK, OP_DISPATCH, OP_ERROR, OP_HELLO, OP_HELLO_OK,
    OP_INVALID_SESSION, OP_REQUEST, OP_RESPONSE, OP_RESUMED, OP_RESYNC, ServerOp,
};
use tokio::net::TcpStream;
use tokio::time::{Instant as TokioInstant, MissedTickBehavior};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Mode {
    /// Send messages and measure ACK round-trip.
    Senders,
    /// Subscribe only; count events, sequence gaps and resyncs.
    Receivers,
    /// Only log in; measures the Argon2 burst.
    Login,
    /// One slow reader plus a fast sender: verifies backpressure + resume.
    Slow,
}

#[derive(Parser, Debug)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    http: String,
    #[arg(long, default_value = "ws://127.0.0.1:8080/gateway")]
    ws: String,
    #[arg(long, value_enum, default_value_t = Mode::Senders)]
    mode: Mode,
    /// Sender count (mode senders) or user count (mode login/slow).
    #[arg(long, default_value_t = 20)]
    users: usize,
    #[arg(long)]
    senders: Option<usize>,
    #[arg(long)]
    receivers: Option<usize>,
    /// First seeded user index (`load{offset:04}`).
    #[arg(long, default_value_t = 0)]
    offset: usize,
    #[arg(long, default_value_t = 30)]
    duration: u64,
    /// Messages per second per sender.
    #[arg(long, default_value_t = 0.05)]
    msg_rate: f64,
    #[arg(long, default_value = "loadtest-password")]
    password: String,
    #[arg(long, default_value = "load")]
    prefix: String,
    #[arg(long, default_value_t = 0)]
    channel: i64,
    #[arg(long, default_value_t = true)]
    fetch_metrics: bool,
    #[arg(long, default_value_t = 3)]
    connect_retries: usize,
}

#[derive(Default)]
struct PeerStats {
    connect_ms: u64,
    sent: u64,
    acked: u64,
    errors: u64,
    msgs: u64,
    seq_gaps: u64,
    resyncs: u64,
    reconnects: u64,
    latencies: Vec<u32>,
}

impl PeerStats {
    fn absorb(&mut self, other: PeerStats) {
        self.sent += other.sent;
        self.acked += other.acked;
        self.errors += other.errors;
        self.msgs += other.msgs;
        self.seq_gaps += other.seq_gaps;
        self.resyncs += other.resyncs;
        self.reconnects += other.reconnects;
        self.latencies.extend(other.latencies);
        self.connect_ms = self.connect_ms.max(other.connect_ms);
    }
}

struct Conn {
    sink: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    session_id: String,
    connect_ms: u64,
}

#[derive(serde::Deserialize)]
struct LoginResponse {
    access_token: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Arc::new(Args::parse());
    match args.mode {
        Mode::Login => login_burst(&args).await,
        Mode::Slow => slow_client(&args).await,
        Mode::Senders | Mode::Receivers => run_load(&args).await,
    }
}

fn handle(index: usize, args: &Args) -> String {
    format!("{}{index:04}", args.prefix)
}

async fn login(client: &reqwest::Client, args: &Args, index: usize) -> Result<(String, u64)> {
    let handle = handle(index, args);
    let started = Instant::now();
    let response = client
        .post(format!("{}/api/v1/auth/login", args.http))
        .json(&serde_json::json!({ "handle": handle, "password": args.password }))
        .send()
        .await
        .with_context(|| format!("login {handle}"))?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "login {handle} failed: {} {}",
            response.status(),
            response.text().await.unwrap_or_default()
        ));
    }
    let body: LoginResponse = response.json().await.context("decode login response")?;
    Ok((body.access_token, started.elapsed().as_millis() as u64))
}

async fn login_all(args: &Arc<Args>, count: usize) -> Result<Vec<(String, u64)>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let started = Instant::now();
    let mut out = Vec::with_capacity(count);
    for chunk_start in (0..count).step_by(32) {
        let chunk_end = (chunk_start + 32).min(count);
        let mut tasks = Vec::new();
        for index in (args.offset + chunk_start)..(args.offset + chunk_end) {
            let client = client.clone();
            let args = Arc::clone(args);
            tasks.push(tokio::spawn(
                async move { login(&client, &args, index).await },
            ));
        }
        for task in tasks {
            out.push(task.await.context("login task panicked")??);
        }
    }
    let secs = started.elapsed().as_secs_f64();
    println!(
        "logged in {} users in {:.1}s ({:.0}/s)",
        out.len(),
        secs,
        out.len() as f64 / secs.max(0.001)
    );
    Ok(out)
}

async fn login_burst(args: &Arc<Args>) -> Result<()> {
    let logins = login_all(args, args.users).await?;
    let mut samples: Vec<u32> = logins.iter().map(|(_, ms)| *ms as u32).collect();
    samples.sort_unstable();
    let pick = |q: f64| samples[((samples.len() as f64 * q) as usize).min(samples.len() - 1)];
    println!(
        "login_ms: p50={} p95={} p99={} max={}",
        pick(0.50),
        pick(0.95),
        pick(0.99),
        samples[samples.len() - 1]
    );
    Ok(())
}

async fn run_load(args: &Arc<Args>) -> Result<()> {
    let (senders, receivers) = match args.mode {
        Mode::Receivers => (0, args.receivers.unwrap_or(args.users)),
        Mode::Senders => (
            args.senders.unwrap_or(args.users),
            args.receivers.unwrap_or(0),
        ),
        Mode::Login | Mode::Slow => (args.senders.unwrap_or(args.users), 0),
    };
    let count = senders + receivers;
    println!(
        "loadtest: mode={:?} clients={} senders={} receivers={} step={}s channel={} rate={}/s",
        args.mode, count, senders, receivers, args.duration, args.channel, args.msg_rate
    );
    let logins = login_all(args, count).await?;
    let run_started = Instant::now();
    let mut tasks = Vec::with_capacity(logins.len());
    for (slot, (token, _)) in logins.into_iter().enumerate() {
        let args = Arc::clone(args);
        let index = args.offset + slot;
        let is_sender = slot < senders;
        tasks.push(tokio::spawn(async move {
            run_peer(args, token, index, is_sender).await
        }));
    }

    let mut stats = PeerStats::default();
    let mut failed = 0usize;
    for task in tasks {
        match task.await.context("peer task panicked")? {
            Ok(peer) => stats.absorb(peer),
            Err(err) => {
                failed += 1;
                if failed <= 5 {
                    eprintln!("client failed: {err:#}");
                }
            }
        }
    }
    let secs = run_started.elapsed().as_secs_f64();
    stats.latencies.sort_unstable();
    let pick = |q: f64| -> u64 {
        if stats.latencies.is_empty() {
            return 0;
        }
        let idx = ((stats.latencies.len() as f64 * q) as usize).min(stats.latencies.len() - 1);
        stats.latencies[idx] as u64
    };

    println!("--- results ---");
    println!(
        "clients={} failed={} duration={:.1}s sent={} acked={} errors={} msgs_seen={} seq_gaps={} resyncs={} reconnects={}",
        count,
        failed,
        secs,
        stats.sent,
        stats.acked,
        stats.errors,
        stats.msgs,
        stats.seq_gaps,
        stats.resyncs,
        stats.reconnects
    );
    println!(
        "throughput: {:.1} msg/s",
        stats.sent as f64 / secs.max(0.001)
    );
    println!(
        "connect_ms_max={} roundtrip_us: p50={} p95={} p99={} samples={}",
        stats.connect_ms,
        pick(0.50),
        pick(0.95),
        pick(0.99),
        stats.latencies.len()
    );
    if args.fetch_metrics {
        scrape(&args.http).await;
    }
    Ok(())
}

async fn scrape(http: &str) {
    let client = reqwest::Client::new();
    match client.get(format!("{http}/metrics")).send().await {
        Ok(response) => match response.text().await {
            Ok(text) => {
                println!("--- server metrics ---");
                for line in text.lines() {
                    if line.starts_with("nova_ws_")
                        || line.starts_with("nova_msg_latency")
                        || line.starts_with("nova_dispatch_latency")
                        || line.starts_with("nova_process_rss")
                    {
                        println!("{line}");
                    }
                }
            }
            Err(err) => eprintln!("metrics body failed: {err}"),
        },
        Err(err) => eprintln!("metrics request failed: {err}"),
    }
}

async fn connect_client(args: &Args, token: &str, resume: Option<(String, u32)>) -> Result<Conn> {
    let mut last_error = None;
    for attempt in 0..=args.connect_retries {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(100 * attempt as u64)).await;
        }
        let started = Instant::now();
        match connect_once(args, token, resume.clone()).await {
            Ok(mut conn) => {
                conn.connect_ms = started.elapsed().as_millis() as u64;
                return Ok(conn);
            }
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("connect failed")))
}

async fn connect_once(args: &Args, token: &str, resume: Option<(String, u32)>) -> Result<Conn> {
    let (ws, _) = connect_async(&args.ws).await.context("websocket connect")?;
    let (mut sink, mut stream) = ws.split();
    let hello = protocol::encode(
        OP_HELLO,
        0,
        &ClientOp::Hello {
            token: token.to_owned(),
            resume: resume
                .clone()
                .map(|(session_id, last_seq)| protocol::ResumeRequest {
                    session_id,
                    last_seq,
                }),
        },
    )?;
    sink.send(Message::Binary(hello)).await?;

    // On resume the replay frames arrive before RESUMED; hand the stream back
    // immediately so the caller counts them instead of discarding them here.
    if let Some((session_id, _)) = resume {
        return Ok(Conn {
            sink,
            stream,
            session_id,
            connect_ms: 0,
        });
    }

    let deadline = TokioInstant::now() + Duration::from_secs(15);
    let mut session_id = String::new();
    while TokioInstant::now() < deadline {
        let Some(message) = tokio::time::timeout_at(deadline, stream.next())
            .await
            .ok()
            .flatten()
        else {
            break;
        };
        let Message::Binary(bytes) = message? else {
            continue;
        };
        let frame = protocol::decode(&bytes)?;
        match frame.op {
            OP_HELLO_OK => {
                if let ServerOp::HelloOk { session_id: id, .. } =
                    protocol::decode_body::<ServerOp>(frame.body)?
                {
                    session_id = id;
                    break;
                }
            }
            OP_INVALID_SESSION => return Err(anyhow!("gateway rejected the session")),
            _ => {}
        }
    }
    if session_id.is_empty() {
        return Err(anyhow!("handshake timed out"));
    }
    {
        let subscribe = protocol::encode(
            OP_REQUEST,
            0,
            &ClientOp::Subscribe {
                channel_id: args.channel,
            },
        )?;
        sink.send(Message::Binary(subscribe)).await?;
        let sub_deadline = TokioInstant::now() + Duration::from_secs(10);
        while TokioInstant::now() < sub_deadline {
            let Some(message) = tokio::time::timeout_at(sub_deadline, stream.next())
                .await
                .ok()
                .flatten()
            else {
                break;
            };
            let Message::Binary(bytes) = message? else {
                continue;
            };
            let frame = protocol::decode(&bytes)?;
            if frame.op == OP_RESPONSE
                && let ServerOp::Subscribed { .. } = protocol::decode_body::<ServerOp>(frame.body)?
            {
                break;
            }
        }
    }
    Ok(Conn {
        sink,
        stream,
        session_id,
        connect_ms: 0,
    })
}

async fn run_peer(args: Arc<Args>, token: String, index: usize, sender: bool) -> Result<PeerStats> {
    let deadline = TokioInstant::now() + Duration::from_secs(args.duration);
    let mut conn = connect_client(&args, &token, None).await?;
    let mut stats = PeerStats {
        connect_ms: conn.connect_ms,
        ..Default::default()
    };
    let mut last_seq = 0u32;
    let mut pending: HashMap<u64, Instant> = HashMap::new();
    let nonce = AtomicU64::new(0);

    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut send_tick =
        tokio::time::interval(Duration::from_secs_f64(1.0 / args.msg_rate.max(0.0001)));
    send_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            _ = heartbeat.tick() => {
                let frame = protocol::encode(OP_REQUEST, 0, &ClientOp::Heartbeat { seq: last_seq })?;
                if conn.sink.send(Message::Binary(frame)).await.is_err() {
                    break;
                }
            }
            _ = send_tick.tick(), if sender => {
                let n = nonce.fetch_add(1, Ordering::Relaxed) + 1;
                pending.insert(n, Instant::now());
                stats.sent += 1;
                let frame = protocol::encode(
                    OP_REQUEST,
                    0,
                    &ClientOp::MessageCreate {
                        nonce: n,
                        channel_id: args.channel,
                        content: format!("loadtest {index} {n}"),
                        reply_to: None,
                        attachment_ids: Vec::new(),
                    },
                )?;
                if conn.sink.send(Message::Binary(frame)).await.is_err() {
                    break;
                }
            }
            incoming = conn.stream.next() => {
                match incoming {
                    Some(Ok(Message::Binary(bytes))) => {
                        if let Ok(frame) = protocol::decode(&bytes) {
                            match frame.op {
                                OP_DISPATCH => {
                                    if last_seq != 0 && frame.seq != last_seq + 1 {
                                        stats.seq_gaps += 1;
                                    }
                                    last_seq = frame.seq;
                                    if let Ok(Event::MessageCreate { .. }) =
                                        protocol::decode_body::<Event>(frame.body)
                                    {
                                        stats.msgs += 1;
                                    }
                                }
                                OP_ACK => {
                                    if let Ok(ServerOp::Ack { nonce, .. }) =
                                        protocol::decode_body::<ServerOp>(frame.body)
                                    {
                                        stats.acked += 1;
                                        if let Some(started) = pending.remove(&nonce) {
                                            stats.latencies.push(
                                                started.elapsed().as_micros().min(u32::MAX as u128) as u32,
                                            );
                                        }
                                    }
                                }
                                OP_ERROR => stats.errors += 1,
                                OP_RESYNC => {
                                    stats.resyncs += 1;
                                    if let Ok(ServerOp::Resync { oldest_seq }) =
                                        protocol::decode_body::<ServerOp>(frame.body)
                                    {
                                        last_seq = oldest_seq.saturating_sub(1);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => {
                        stats.reconnects += 1;
                        let resume = conn.session_id.clone();
                        conn = match connect_client(&args, &token, Some((resume, last_seq))).await {
                            Ok(conn) => conn,
                            Err(_) => {
                                last_seq = 0;
                                connect_client(&args, &token, None).await?
                            }
                        };
                    }
                }
            }
        }
    }
    let _ = conn.sink.close().await;
    Ok(stats)
}

/// One client stops reading while another sends; the server must close it for
/// backpressure, and the resume must either replay everything or announce a gap.
async fn slow_client(args: &Arc<Args>) -> Result<()> {
    let logins = login_all(args, 2).await?;
    let slow_token = logins[0].0.clone();
    let sender_token = logins[1].0.clone();

    let mut slow = connect_client(args, &slow_token, None).await?;
    let sender_args = Arc::clone(args);
    let sender = tokio::spawn(async move {
        let mut conn = connect_client(&sender_args, &sender_token, None).await?;
        let total = 12_000u64;
        let mut tick =
            tokio::time::interval(Duration::from_secs_f64(1.0 / sender_args.msg_rate.max(1.0)));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        for n in 1..=total {
            tick.tick().await;
            let frame = protocol::encode(
                OP_REQUEST,
                0,
                &ClientOp::MessageCreate {
                    nonce: n,
                    channel_id: sender_args.channel,
                    content: format!("slow-test {n} {}", "x".repeat(180)),
                    reply_to: None,
                    attachment_ids: Vec::new(),
                },
            )?;
            conn.sink.send(Message::Binary(frame)).await?;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        let _ = conn.sink.close().await;
        Ok::<u64, anyhow::Error>(total)
    });

    // Stop reading long enough to fill socket buffers and the server outbox.
    tokio::time::sleep(Duration::from_secs(6)).await;

    let mut live = 0u64;
    let mut last_seq = 0u32;
    let mut closed = false;
    let drain_deadline = TokioInstant::now() + Duration::from_secs(20);
    while TokioInstant::now() < drain_deadline {
        match tokio::time::timeout_at(drain_deadline, slow.stream.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => {
                if let Ok(frame) = protocol::decode(&bytes) {
                    if frame.op == OP_DISPATCH {
                        live += 1;
                        last_seq = last_seq.max(frame.seq);
                    } else if frame.op == OP_RESYNC {
                        break;
                    }
                }
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(err))) => {
                eprintln!("slow client closed: {err}");
                closed = true;
                break;
            }
            Ok(None) => {
                closed = true;
                break;
            }
            Err(_) => break,
        }
    }

    let sent = sender.await.context("sender task")??;
    let mut resumed_msgs = 0u64;
    let mut resync = false;
    if closed {
        let mut resumed =
            connect_client(args, &slow_token, Some((slow.session_id.clone(), last_seq))).await?;
        let replay_deadline = TokioInstant::now() + Duration::from_secs(5);
        while TokioInstant::now() < replay_deadline {
            match tokio::time::timeout_at(replay_deadline, resumed.stream.next()).await {
                Ok(Some(Ok(Message::Binary(bytes)))) => {
                    if let Ok(frame) = protocol::decode(&bytes) {
                        match frame.op {
                            OP_DISPATCH => {
                                resumed_msgs += 1;
                                last_seq = last_seq.max(frame.seq);
                            }
                            OP_RESYNC => resync = true,
                            OP_RESUMED => break,
                            _ => {}
                        }
                    }
                }
                Ok(Some(Ok(_))) => {}
                _ => break,
            }
        }
    }

    println!("--- slow client ---");
    println!(
        "sent={sent} live_before_close={live} resumed_msgs={resumed_msgs} resync={resync} closed_for_backpressure={closed} last_seq={last_seq}"
    );
    if !closed {
        eprintln!("FAIL: server never closed the slow client (no backpressure)");
        std::process::exit(2);
    }
    println!("PASS: slow client was disconnected and resumed (gap announced={resync})");
    Ok(())
}
