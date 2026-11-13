//! Minimal, dependency-free metrics with a Prometheus text exposition.
//!
//! No TSDB, no exporter sidecar: a single `/metrics` endpoint is enough for one VPS.

use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    pub fn inc(&self) {
        self.add(1);
    }

    pub fn add(&self, value: u64) {
        self.0.fetch_add(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Default)]
pub struct Gauge(AtomicI64);

impl Gauge {
    pub fn inc(&self) {
        self.add(1);
    }

    pub fn dec(&self) {
        self.add(-1);
    }

    pub fn add(&self, value: i64) {
        self.0.fetch_add(value, Ordering::Relaxed);
    }

    pub fn set(&self, value: i64) {
        self.0.store(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Fixed-capacity ring of recent samples. Percentiles are computed over the
/// retained window, which is exact for the last `capacity` observations.
#[derive(Debug)]
pub struct Histogram {
    slots: Box<[AtomicU32]>,
    cursor: AtomicU64,
    count: AtomicU64,
    sum: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HistogramSnapshot {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
    pub count: u64,
    pub avg: u64,
}

impl Histogram {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(16);
        Self {
            slots: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            cursor: AtomicU64::new(0),
            count: AtomicU64::new(0),
            sum: AtomicU64::new(0),
        }
    }

    pub fn record_us(&self, micros: u64) {
        let idx = (self.cursor.fetch_add(1, Ordering::Relaxed) as usize) % self.slots.len();
        let clamped = micros.min(u32::MAX as u64) as u32;
        self.slots[idx].store(clamped, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(micros, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> HistogramSnapshot {
        let count = self.count.load(Ordering::Relaxed);
        if count == 0 {
            return HistogramSnapshot::default();
        }
        let mut samples: Vec<u32> = self
            .slots
            .iter()
            .map(|slot| slot.load(Ordering::Relaxed))
            .filter(|value| *value > 0)
            .collect();
        samples.sort_unstable();
        let pick = |quantile: f64| -> u64 {
            if samples.is_empty() {
                return 0;
            }
            let idx = ((samples.len() as f64 * quantile) as usize).min(samples.len() - 1);
            samples[idx] as u64
        };
        let sum = self.sum.load(Ordering::Relaxed);
        HistogramSnapshot {
            p50: pick(0.50),
            p95: pick(0.95),
            p99: pick(0.99),
            max: samples.last().copied().unwrap_or(0) as u64,
            count,
            avg: sum / count.max(1),
        }
    }
}

/// Process-wide metrics. Cheap to clone behind an `Arc`.
#[derive(Debug)]
pub struct Metrics {
    pub ws_connections: Gauge,
    pub ws_frames_in: Counter,
    pub ws_frames_out: Counter,
    /// Sessions disconnected because their outbox was full (resume instead of
    /// silently dropping frames).
    pub ws_lagged: Counter,
    /// Resume requests whose ring no longer covered `last_seq`; the client is
    /// told to refetch instead of receiving a silent gap.
    pub ws_resync: Counter,
    pub ws_reconnects: Counter,
    pub messages_created: Counter,
    pub message_latency: Histogram,
    pub dispatch_latency: Histogram,
    pub http_requests: Counter,
    pub http_errors: Counter,
    pub auth_failures: Counter,
    pub uploads: Counter,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            ws_connections: Gauge::default(),
            ws_frames_in: Counter::default(),
            ws_frames_out: Counter::default(),
            ws_lagged: Counter::default(),
            ws_resync: Counter::default(),
            ws_reconnects: Counter::default(),
            messages_created: Counter::default(),
            // 64 KiB samples ≈ 256 KB of RAM: exact percentiles over a long window.
            message_latency: Histogram::new(65_536),
            dispatch_latency: Histogram::new(65_536),
            http_requests: Counter::default(),
            http_errors: Counter::default(),
            auth_failures: Counter::default(),
            uploads: Counter::default(),
        }
    }
}

impl Metrics {
    pub fn render(&self, rss_bytes: u64, uptime_secs: u64) -> String {
        let mut out = String::with_capacity(2048);
        let msg = self.message_latency.snapshot();
        let disp = self.dispatch_latency.snapshot();

        out.push_str("# TYPE nova_ws_connections gauge\n");
        out.push_str(&format!(
            "nova_ws_connections {}\n",
            self.ws_connections.get()
        ));
        out.push_str("# TYPE nova_ws_frames_total counter\n");
        out.push_str(&format!(
            "nova_ws_frames_in_total {}\nnova_ws_frames_out_total {}\n",
            self.ws_frames_in.get(),
            self.ws_frames_out.get()
        ));
        out.push_str(&format!(
            "nova_ws_lagged_total {}\nnova_ws_resync_total {}\n",
            self.ws_lagged.get(),
            self.ws_resync.get()
        ));
        out.push_str(&format!(
            "nova_ws_reconnects_total {}\nnova_messages_created_total {}\n",
            self.ws_reconnects.get(),
            self.messages_created.get()
        ));
        out.push_str("# TYPE nova_msg_latency_seconds gauge\n");
        out.push_str(&format!(
            "nova_msg_latency_p50_seconds {}\nnova_msg_latency_p95_seconds {}\nnova_msg_latency_p99_seconds {}\nnova_msg_latency_max_seconds {}\nnova_msg_latency_samples {}\nnova_msg_latency_avg_seconds {}\n",
            msg.p50 as f64 / 1e6,
            msg.p95 as f64 / 1e6,
            msg.p99 as f64 / 1e6,
            msg.max as f64 / 1e6,
            msg.count,
            msg.avg as f64 / 1e6
        ));
        out.push_str("# TYPE nova_dispatch_latency_seconds gauge\n");
        out.push_str(&format!(
            "nova_dispatch_latency_p50_seconds {}\nnova_dispatch_latency_p95_seconds {}\nnova_dispatch_latency_p99_seconds {}\nnova_dispatch_latency_max_seconds {}\n",
            disp.p50 as f64 / 1e6,
            disp.p95 as f64 / 1e6,
            disp.p99 as f64 / 1e6,
            disp.max as f64 / 1e6
        ));
        out.push_str(&format!(
            "nova_http_requests_total {}\nnova_http_errors_total {}\nnova_auth_failures_total {}\nnova_uploads_total {}\n",
            self.http_requests.get(),
            self.http_errors.get(),
            self.auth_failures.get(),
            self.uploads.get()
        ));
        out.push_str(&format!(
            "nova_process_rss_bytes {rss_bytes}\nnova_process_uptime_seconds {uptime_secs}\n"
        ));
        out
    }
}

/// Resident set size of this process. Linux only: the VPS runs Linux, and the
/// local Windows measurement uses PowerShell instead of parsing kernel data.
pub fn process_rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm")
            && let Some(pages) = statm.split_whitespace().nth(1)
            && let Ok(pages) = pages.parse::<u64>()
        {
            return pages.saturating_mul(4096);
        }
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_reports_percentiles() {
        let hist = Histogram::new(1000);
        for i in 1..=1000u64 {
            hist.record_us(i);
        }
        let snap = hist.snapshot();
        assert_eq!(snap.count, 1000);
        assert!((snap.p50 as i64 - 500).abs() <= 3);
        assert!(snap.p99 >= 985);
        assert_eq!(snap.max, 1000);
    }

    #[test]
    fn empty_histogram_is_zero() {
        let hist = Histogram::new(16);
        let snap = hist.snapshot();
        assert_eq!(snap.count, 0);
        assert_eq!(snap.p99, 0);
    }
}
