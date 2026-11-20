//! Voice-Last: N simulierte Teilnehmer in einem Sprachkanal.
//!
//! Jeder Teilnehmer ist eine echte str0m-Verbindung zum SFU – wie der native
//! Client: Offer per `POST /channels/{id}/voice/join`, ICE, DTLS, SRTP,
//! Neuverhandlung über den DataChannel `sdp`. Alle 20 ms geht ein Paket in
//! Opus-Größe hinaus; darin stehen Absender, Folgenummer und Sendezeit. Weil
//! alle Teilnehmer in diesem Prozess laufen, misst jeder Empfänger die
//! Laufzeit durch den SFU ohne Uhrenabgleich. Der SFU dekodiert nichts, der
//! Inhalt muss also kein gültiges Opus sein.
//!
//! Gemessen wird erst, wenn alle verbunden sind und jeder alle anderen hört
//! (plus zwei Sekunden Anlauf). Nicht enthalten: Mikrofon, Opus-Kodierung,
//! Jitter-Puffer und Lautsprecher – das braucht echte Geräte.

use std::{
    collections::HashMap,
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use str0m::{
    Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig,
    change::{SdpAnswer, SdpOffer},
    channel::ChannelId,
    format::Codec,
    media::{Direction, Frequency, MediaKind, MediaTime, Mid},
    net::{Protocol, Receive},
};

use crate::{Args, login_all};

/// Kennung am Paketanfang, damit Fremdes nicht mitgezählt wird.
const MAGIC: [u8; 2] = *b"NV";
/// Kopf: Kennung, Flags, Absender, Folgenummer, Sendezeit in µs.
const HEADER: usize = 2 + 1 + 2 + 4 + 8;
const FRAME: Duration = Duration::from_millis(20);
/// Gesetzt, wenn das Paket im Messfenster gesendet wurde.
const FLAG_MEASURED: u8 = 1;

/// Was der Hauptthread mit allen Teilnehmern teilt.
struct Shared {
    /// Gemeinsamer Nullpunkt für die Sendezeiten.
    epoch: Instant,
    /// Messfenster offen.
    measuring: AtomicBool,
    /// Alle Teilnehmer beenden sich.
    stop: AtomicBool,
    connected: AtomicUsize,
    /// Teilnehmer, die von allen anderen schon Pakete bekommen.
    hearing_all: AtomicUsize,
}

#[derive(Default)]
struct PeerReport {
    connected_after: Option<Duration>,
    sent_measured: u64,
    received_measured: u64,
    /// Laufzeiten in µs (nur Pakete aus dem Messfenster): Senden bis zum
    /// Ereignis beim Empfänger …
    latencies: Vec<u32>,
    /// … davon Senden bis zum Eintreffen am Socket (Netz + SFU).
    transport: Vec<u32>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct Created {
    id: String,
}

#[derive(Deserialize)]
struct JoinAnswer {
    sdp: String,
}

/// Wie im SFU (`rtc_config`) und im nativen Client: Opus plus H.264, damit die
/// Aushandlung genauso verläuft wie bei echten Teilnehmern.
fn rtc_config() -> RtcConfig {
    let mut config = RtcConfig::new().clear_codecs().enable_opus(true, false);
    config
        .codec_config()
        .add_h264(108.into(), Some(109.into()), true, 0x42e01f);
    config
}

pub async fn run(args: &Arc<Args>) -> Result<()> {
    let users = args.users.max(2);
    let guild = args
        .guild
        .ok_or_else(|| anyhow!("--guild fehlt (Ausgabe von `nova seed`)"))?;
    let logins = login_all(args, users).await?;
    let tokens: Vec<String> = logins.into_iter().map(|(token, _)| token).collect();
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    // Sprachkanal anlegen (Nutzer 0 ist bei `nova seed` Besitzer der Guild).
    let channel: Created = http
        .post(format!("{}/api/v1/guilds/{guild}/channels", args.http))
        .bearer_auth(&tokens[0])
        .json(&serde_json::json!({ "name": "lasttest", "kind": 1 }))
        .send()
        .await?
        .error_for_status()
        .context("Sprachkanal anlegen")?
        .json()
        .await?;
    let channel_id: i64 = channel.id.parse().context("Kanal-ID")?;
    let bind_ip = match args.bind_ip {
        Some(ip) => ip,
        None => local_ip_towards(&args.http),
    };
    println!(
        "Voice-Last: {users} Teilnehmer in Kanal {channel_id}, {} Byte alle 20 ms, \
         Messfenster {} s, lokal {bind_ip}",
        args.payload, args.duration
    );

    let shared = Arc::new(Shared {
        epoch: Instant::now(),
        measuring: AtomicBool::new(false),
        stop: AtomicBool::new(false),
        connected: AtomicUsize::new(0),
        hearing_all: AtomicUsize::new(0),
    });
    let reports: Arc<Mutex<Vec<PeerReport>>> = Arc::default();
    let mut threads = Vec::with_capacity(users);
    let join_started = Instant::now();
    for (index, token) in tokens.iter().enumerate() {
        let peer = Peer::join(
            &http,
            &args.http,
            token,
            channel_id,
            bind_ip,
            index,
            users,
            args.payload,
            Arc::clone(&shared),
        )
        .await
        .with_context(|| format!("Teilnehmer {index} tritt bei"))?;
        let reports = Arc::clone(&reports);
        threads.push(
            thread::Builder::new()
                .name(format!("voice-peer-{index}"))
                .spawn(move || {
                    let report = peer.run();
                    if let Ok(mut all) = reports.lock() {
                        all.push(report);
                    }
                })?,
        );
    }

    // Warten, bis alle verbunden sind und jeder alle anderen hört.
    let ready_deadline = Instant::now() + Duration::from_secs(30);
    while shared.hearing_all.load(Ordering::Relaxed) < users {
        if Instant::now() > ready_deadline {
            println!(
                "Nicht alle bereit nach 30 s: verbunden {}, hören alle {} von {users}",
                shared.connected.load(Ordering::Relaxed),
                shared.hearing_all.load(Ordering::Relaxed)
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let ready_after = join_started.elapsed();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let before = ProcStats::read(args.server_pid);
    let window_started = Instant::now();
    shared.measuring.store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(args.duration)).await;
    shared.measuring.store(false, Ordering::Relaxed);
    let window = window_started.elapsed();
    let after = ProcStats::read(args.server_pid);
    // Pakete aus dem Fenster dürfen noch ankommen.
    tokio::time::sleep(Duration::from_millis(500)).await;
    shared.stop.store(true, Ordering::Relaxed);
    for handle in threads {
        let _ = handle.join();
    }
    for token in &tokens {
        let _ = http
            .post(format!(
                "{}/api/v1/channels/{channel_id}/voice/leave",
                args.http
            ))
            .bearer_auth(token)
            .send()
            .await;
    }

    let reports = std::mem::take(&mut *reports.lock().map_err(|_| anyhow!("lock"))?);
    report(users, ready_after, window, &reports, before, after);
    if args.fetch_metrics {
        crate::scrape(&args.http).await;
    }
    Ok(())
}

fn report(
    users: usize,
    ready_after: Duration,
    window: Duration,
    reports: &[PeerReport],
    before: Option<ProcStats>,
    after: Option<ProcStats>,
) {
    let connected = reports
        .iter()
        .filter(|r| r.connected_after.is_some())
        .count();
    let ice_max = reports
        .iter()
        .filter_map(|r| r.connected_after)
        .max()
        .unwrap_or_default();
    for error in reports.iter().filter_map(|r| r.error.as_deref()) {
        println!("Fehler: {error}");
    }
    let sent: u64 = reports.iter().map(|r| r.sent_measured).sum();
    let received: u64 = reports.iter().map(|r| r.received_measured).sum();
    let expected = sent * (users as u64 - 1);
    let loss = if expected == 0 {
        0.0
    } else {
        100.0 * (expected.saturating_sub(received)) as f64 / expected as f64
    };
    let sorted = |pick: fn(&PeerReport) -> &Vec<u32>| -> Vec<u32> {
        let mut all: Vec<u32> = reports
            .iter()
            .flat_map(|r| pick(r).iter().copied())
            .collect();
        all.sort_unstable();
        all
    };
    let latencies = sorted(|r| &r.latencies);
    let transport = sorted(|r| &r.transport);
    let quantile = |values: &[u32], q: f64| -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let idx = ((values.len() as f64 * q) as usize).min(values.len() - 1);
        f64::from(values[idx]) / 1000.0
    };
    let pick = |q: f64| quantile(&latencies, q);
    println!(
        "verbunden: {connected}/{users} (ICE spätestens nach {} ms), alle hören alle nach {:.1} s",
        ice_max.as_millis(),
        ready_after.as_secs_f64()
    );
    println!(
        "Messfenster {:.1} s: gesendet {sent}, empfangen {received} von {expected} erwartet \
         (Verlust {loss:.3} %)",
        window.as_secs_f64()
    );
    println!(
        "Laufzeit durch den SFU: p50 {:.2} ms · p95 {:.2} ms · p99 {:.2} ms · max {:.2} ms \
         ({} Pakete)",
        pick(0.50),
        pick(0.95),
        pick(0.99),
        latencies.last().map_or(0.0, |v| f64::from(*v) / 1000.0),
        latencies.len()
    );
    println!(
        "  davon Netz + SFU (bis zum Socket des Empfängers): p50 {:.2} ms · p99 {:.2} ms · max {:.2} ms",
        quantile(&transport, 0.50),
        quantile(&transport, 0.99),
        transport.last().map_or(0.0, |v| f64::from(*v) / 1000.0)
    );
    if let (Some(before), Some(after)) = (before, after) {
        // USER_HZ ist auf Linux 100.
        let ticks = after.cpu_ticks.saturating_sub(before.cpu_ticks);
        let cpu = ticks as f64 / window.as_secs_f64();
        println!(
            "Server: CPU {cpu:.0} % eines Kerns, RSS {:.1} MB (Spitze {:.1} MB)",
            after.rss_kb as f64 / 1024.0,
            after.peak_kb as f64 / 1024.0
        );
    }
}

/// CPU-Zeit und Speicher des Servers aus `/proc` (nur Linux, nur lokal).
#[derive(Clone, Copy)]
struct ProcStats {
    cpu_ticks: u64,
    rss_kb: u64,
    peak_kb: u64,
}

impl ProcStats {
    fn read(pid: Option<u32>) -> Option<Self> {
        let pid = pid?;
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // Nach dem Prozessnamen in Klammern: Feld 14/15 = utime/stime.
        let rest = stat.rsplit_once(')')?.1;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let utime: u64 = fields.get(11)?.parse().ok()?;
        let stime: u64 = fields.get(12)?.parse().ok()?;
        let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let kb = |key: &str| -> u64 {
            status
                .lines()
                .find(|line| line.starts_with(key))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse().ok())
                .unwrap_or(0)
        };
        Some(Self {
            cpu_ticks: utime + stime,
            rss_kb: kb("VmRSS:"),
            peak_kb: kb("VmHWM:"),
        })
    }
}

/// Lokale IP, über die der Server erreichbar ist (ohne ein Paket zu senden).
fn local_ip_towards(http: &str) -> IpAddr {
    let host = http
        .split("://")
        .nth(1)
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default();
    let target = if host.contains(':') {
        host.to_owned()
    } else {
        format!("{host}:443")
    };
    let probe = || -> Option<IpAddr> {
        use std::net::ToSocketAddrs;
        let addr = target.to_socket_addrs().ok()?.find(SocketAddr::is_ipv4)?;
        let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect(addr).ok()?;
        Some(socket.local_addr().ok()?.ip())
    };
    probe().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

struct Peer {
    index: usize,
    users: usize,
    payload: usize,
    rtc: Rtc,
    socket: UdpSocket,
    local: SocketAddr,
    mid: Mid,
    channel: ChannelId,
    shared: Arc<Shared>,
    joined: Instant,
}

impl Peer {
    #[allow(clippy::too_many_arguments)]
    async fn join(
        http: &reqwest::Client,
        base: &str,
        token: &str,
        channel_id: i64,
        bind_ip: IpAddr,
        index: usize,
        users: usize,
        payload: usize,
        shared: Arc<Shared>,
    ) -> Result<Self> {
        let socket = UdpSocket::bind(SocketAddr::new(bind_ip, 0)).context("UDP-Socket")?;
        let local = socket.local_addr()?;
        let mut rtc = rtc_config().build(Instant::now());
        rtc.add_local_candidate(Candidate::host(local, "udp").context("ICE-Kandidat")?);
        let mut change = rtc.sdp_api();
        let mid = change.add_media(
            MediaKind::Audio,
            Direction::SendRecv,
            Some("nova".into()),
            None,
            None,
        );
        let channel = change.add_channel("sdp".into());
        let (offer, pending) = change.apply().ok_or_else(|| anyhow!("kein Offer"))?;
        let answer: JoinAnswer = http
            .post(format!("{base}/api/v1/channels/{channel_id}/voice/join"))
            .bearer_auth(token)
            .json(&serde_json::json!({ "sdp": serde_json::to_string(&offer)? }))
            .send()
            .await?
            .error_for_status()
            .context("voice/join")?
            .json()
            .await?;
        let answer: SdpAnswer = serde_json::from_str(&answer.sdp).context("Answer")?;
        rtc.sdp_api()
            .accept_answer(pending, answer)
            .map_err(|err| anyhow!("Answer abgelehnt: {err}"))?;
        Ok(Self {
            index,
            users,
            payload: payload.max(HEADER),
            rtc,
            socket,
            local,
            mid,
            channel,
            shared,
            joined: Instant::now(),
        })
    }

    fn run(mut self) -> PeerReport {
        let mut report = PeerReport::default();
        let mut buf = vec![0u8; 2000];
        // Sendezeitpunkte über die 20 ms verteilen wie bei echten Clients.
        let offset = FRAME.mul_f64(self.index as f64 / self.users as f64);
        let mut next_send = Instant::now() + offset;
        let mut seq: u32 = 0;
        let mut rtp_time: u64 = 0;
        let mut heard: HashMap<u16, u32> = HashMap::new();
        let mut hearing_all = false;
        let mut connected = false;

        while !self.shared.stop.load(Ordering::Relaxed) {
            let now = Instant::now();
            if connected && now >= next_send {
                let measured = self.shared.measuring.load(Ordering::Relaxed);
                if self.send(seq, rtp_time, measured) && measured {
                    report.sent_measured += 1;
                }
                seq = seq.wrapping_add(1);
                rtp_time += 960;
                next_send += FRAME;
                // Hinterher (z. B. nach einer Pause des Threads): nicht nachholen.
                if next_send < now {
                    next_send = now + FRAME;
                }
            }

            // Ausgaben von str0m abarbeiten.
            let timeout = loop {
                if !self.rtc.is_alive() {
                    report.error = Some(format!("Teilnehmer {}: Verbindung beendet", self.index));
                    return report;
                }
                match self.rtc.poll_output() {
                    Ok(Output::Timeout(at)) => break at,
                    Ok(Output::Transmit(transmit)) => {
                        let _ = self
                            .socket
                            .send_to(&transmit.contents, transmit.destination);
                    }
                    Ok(Output::Event(event)) => match event {
                        Event::IceConnectionStateChange(
                            IceConnectionState::Connected | IceConnectionState::Completed,
                        ) if !connected => {
                            connected = true;
                            report.connected_after = Some(self.joined.elapsed());
                            self.shared.connected.fetch_add(1, Ordering::Relaxed);
                        }
                        Event::ChannelData(data) => {
                            if let Ok(offer) = serde_json::from_slice::<SdpOffer>(&data.data) {
                                self.answer(offer);
                            }
                        }
                        Event::MediaData(data) if data.mid != self.mid => {
                            let Some((sender, measured, sent_us)) = parse(&data.data) else {
                                continue;
                            };
                            *heard.entry(sender).or_default() += 1;
                            if !hearing_all && heard.len() + 1 >= self.users {
                                hearing_all = true;
                                self.shared.hearing_all.fetch_add(1, Ordering::Relaxed);
                            }
                            if measured {
                                report.received_measured += 1;
                                let now_us = self.shared.epoch.elapsed().as_micros() as u64;
                                let latency = now_us.saturating_sub(sent_us).max(1);
                                report
                                    .latencies
                                    .push(u32::try_from(latency).unwrap_or(u32::MAX));
                                let arrived_us =
                                    data.network_time
                                        .saturating_duration_since(self.shared.epoch)
                                        .as_micros() as u64;
                                let transport = arrived_us.saturating_sub(sent_us).max(1);
                                report
                                    .transport
                                    .push(u32::try_from(transport).unwrap_or(u32::MAX));
                            }
                        }
                        _ => {}
                    },
                    Err(err) => {
                        report.error = Some(format!("Teilnehmer {}: {err}", self.index));
                        return report;
                    }
                }
            };

            // Bis zum nächsten Ereignis oder Sendezeitpunkt auf Pakete warten.
            let until = if connected {
                timeout.min(next_send)
            } else {
                timeout
            };
            let wait = until
                .saturating_duration_since(Instant::now())
                .clamp(Duration::from_micros(200), Duration::from_millis(20));
            self.socket.set_read_timeout(Some(wait)).ok();
            if let Ok((len, source)) = self.socket.recv_from(&mut buf)
                && let Ok(contents) = buf[..len].try_into()
            {
                let input = Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Udp,
                        source,
                        destination: self.local,
                        contents,
                    },
                );
                if self.rtc.accepts(&input) && self.rtc.handle_input(input).is_err() {
                    report.error = Some(format!("Teilnehmer {}: Eingabe abgelehnt", self.index));
                    return report;
                }
            }
            let _ = self.rtc.handle_input(Input::Timeout(Instant::now()));
        }
        self.rtc.disconnect();
        report
    }

    /// Neue Spuren der anderen: Offer des SFU beantworten.
    fn answer(&mut self, offer: SdpOffer) {
        if let Ok(answer) = self.rtc.sdp_api().accept_offer(offer)
            && let Ok(json) = serde_json::to_string(&answer)
            && let Some(mut channel) = self.rtc.channel(self.channel)
        {
            let _ = channel.write(false, json.as_bytes());
        }
    }

    fn send(&mut self, seq: u32, rtp_time: u64, measured: bool) -> bool {
        let Some(writer) = self.rtc.writer(self.mid) else {
            return false;
        };
        let Some(pt) = writer
            .payload_params()
            .find(|params| params.spec().codec == Codec::Opus)
            .map(|params| params.pt())
        else {
            return false;
        };
        let mut packet = vec![0u8; self.payload];
        packet[0..2].copy_from_slice(&MAGIC);
        packet[2] = if measured { FLAG_MEASURED } else { 0 };
        packet[3..5].copy_from_slice(&(self.index as u16).to_be_bytes());
        packet[5..9].copy_from_slice(&seq.to_be_bytes());
        let sent_us = self.shared.epoch.elapsed().as_micros() as u64;
        packet[9..17].copy_from_slice(&sent_us.to_be_bytes());
        let time = MediaTime::new(rtp_time, Frequency::FORTY_EIGHT_KHZ);
        let ok = writer.write(pt, Instant::now(), time, packet).is_ok();
        // str0m gibt das Paket erst nach einem Zeitschritt frei (wie der SFU).
        let _ = self.rtc.handle_input(Input::Timeout(Instant::now()));
        ok
    }
}

/// (Absender, im Messfenster gesendet, Sendezeit in µs)
fn parse(data: &[u8]) -> Option<(u16, bool, u64)> {
    if data.len() < HEADER || data[0..2] != MAGIC {
        return None;
    }
    let sender = u16::from_be_bytes([data[3], data[4]]);
    let sent_us = u64::from_be_bytes(data[9..17].try_into().ok()?);
    Some((sender, data[2] & FLAG_MEASURED != 0, sent_us))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_fits_and_parses() {
        let mut packet = vec![0u8; 80];
        packet[0..2].copy_from_slice(&MAGIC);
        packet[2] = FLAG_MEASURED;
        packet[3..5].copy_from_slice(&7u16.to_be_bytes());
        packet[9..17].copy_from_slice(&123_456u64.to_be_bytes());
        assert_eq!(parse(&packet), Some((7, true, 123_456)));
        assert_eq!(parse(&packet[..10]), None);
        assert_eq!(parse(b"xx-----------------"), None);
    }
}
