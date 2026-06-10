//! Midpoint-RTT clock-offset estimation (NTP / Cristian's algorithm).
//!
//! Ported from the reference projects' `perform_initial_time_sync`, with two
//! additions geared purely at accuracy: explicit warm-up probes (so every
//! measured probe rides a hot keep-alive connection) and per-endpoint summary
//! statistics that expose RTT jitter.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Client;
use serde_json::Value;
use tokio::time::sleep;

use crate::endpoints::Endpoint;

/// Local wall-clock epoch in nanoseconds. The offset we report is defined
/// relative to this clock, so using it for the RTT midpoint is correct.
fn epoch_ns_now() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch");
    (now.as_secs() as i64)
        .saturating_mul(1_000_000_000)
        .saturating_add(now.subsec_nanos() as i64)
}

/// One successful probe: round-trip time and the derived offset, both in ns.
#[derive(Clone, Copy, Debug)]
struct Sample {
    rtt_ns: i64,
    offset_ns: i64,
}

/// Outcome of probing a single endpoint.
#[derive(Clone, Debug)]
pub struct ProbeResult {
    pub exchange: &'static str,
    pub market: &'static str,
    pub url: String,
    pub shared: bool,
    pub samples_ok: usize,
    pub samples_total: usize,
    /// Set when at least one probe succeeded.
    pub stats: Option<OffsetStats>,
    /// First error encountered, when nothing succeeded.
    pub error: Option<String>,
}

impl ProbeResult {
    pub fn status(&self) -> &'static str {
        if self.samples_ok == 0 {
            "unreachable"
        } else if self.samples_ok < self.samples_total {
            "partial"
        } else {
            "ok"
        }
    }
}

/// Aggregated statistics across the successful probes for one endpoint.
#[derive(Clone, Copy, Debug)]
pub struct OffsetStats {
    /// Offset from the probe with the lowest RTT — the primary result.
    pub offset_best_ns: i64,
    pub offset_median_ns: i64,
    pub offset_p90_ns: i64,
    pub rtt_min_ns: i64,
    pub rtt_median_ns: i64,
}

/// Probe one endpoint: `warmup` throwaway requests, then `probes` measured ones.
pub async fn probe_endpoint(
    client: &Client,
    endpoint: &Endpoint,
    warmup: usize,
    probes: usize,
    gap: Duration,
) -> ProbeResult {
    // Warm-up: establish TCP+TLS, prime DNS, trigger session resumption. The
    // results are discarded so handshake cost never enters a measured RTT.
    for _ in 0..warmup {
        let _ = fetch_server_time_ns(client, endpoint).await;
        sleep(gap).await;
    }

    let mut samples: Vec<Sample> = Vec::with_capacity(probes);
    let mut first_error: Option<String> = None;

    for i in 0..probes {
        let t0 = epoch_ns_now();
        match fetch_server_time_ns(client, endpoint).await {
            Ok(server_ns) => {
                let t1 = epoch_ns_now();
                let rtt_ns = (t1 - t0).max(1);
                // Estimate the local epoch at the moment the server stamped the
                // time as the round-trip midpoint, assuming symmetric latency.
                let midpoint_local_ns = t0 + rtt_ns / 2;
                let offset_ns = server_ns - midpoint_local_ns;
                samples.push(Sample { rtt_ns, offset_ns });
            }
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e.to_string());
                }
            }
        }
        if i + 1 < probes {
            sleep(gap).await;
        }
    }

    let stats = summarize(&samples);
    ProbeResult {
        exchange: endpoint.exchange,
        market: endpoint.market.as_str(),
        url: endpoint.url.clone(),
        shared: endpoint.shared,
        samples_ok: samples.len(),
        samples_total: probes,
        stats,
        error: if samples.is_empty() { first_error } else { None },
    }
}

/// Single request: fetch and parse the server time into epoch ns.
async fn fetch_server_time_ns(client: &Client, endpoint: &Endpoint) -> anyhow::Result<i64> {
    let body: Value = client
        .get(&endpoint.url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    endpoint.parse_server_time_ns(&body)
}

fn summarize(samples: &[Sample]) -> Option<OffsetStats> {
    if samples.is_empty() {
        return None;
    }

    // Primary offset comes from the lowest-RTT probe (least path asymmetry).
    let best = samples
        .iter()
        .min_by_key(|s| s.rtt_ns)
        .expect("non-empty");

    let mut offsets: Vec<i64> = samples.iter().map(|s| s.offset_ns).collect();
    let mut rtts: Vec<i64> = samples.iter().map(|s| s.rtt_ns).collect();
    offsets.sort_unstable();
    rtts.sort_unstable();

    Some(OffsetStats {
        offset_best_ns: best.offset_ns,
        offset_median_ns: median(&offsets),
        offset_p90_ns: percentile(&offsets, 90.0),
        rtt_min_ns: rtts[0],
        rtt_median_ns: median(&rtts),
    })
}

/// Median of a pre-sorted slice.
fn median(sorted: &[i64]) -> i64 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2
    }
}

/// Nearest-rank percentile of a pre-sorted slice.
fn percentile(sorted: &[i64], p: f64) -> i64 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    let rank = (p / 100.0 * n as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted[idx]
}
