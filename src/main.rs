//! exchange-clock-offset — measure the clock offset between this machine and
//! major crypto exchange servers (spot & USDⓈ-M futures) using a midpoint-RTT
//! (NTP / Cristian) estimate over each exchange's REST `/time` endpoint.

mod endpoints;
mod report;
mod sync;

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use clap::Parser as ClapParser;
use reqwest::Client;

use endpoints::{all_endpoints, known_exchanges, Market};
use sync::{probe_endpoint, ProbeResult};

#[derive(ClapParser, Debug)]
#[command(
    name = "exchange-clock-offset",
    about = "Measure clock offset between local machine and exchange servers"
)]
struct Cli {
    /// Comma-separated exchanges to probe (default: all known).
    #[arg(long)]
    exchanges: Option<String>,

    /// Comma-separated markets to probe: spot,usdm (default: both).
    #[arg(long, default_value = "spot,usdm")]
    markets: String,

    /// Number of measured probes per endpoint (lowest-RTT one wins).
    #[arg(long, default_value_t = 7)]
    probes: usize,

    /// Number of throwaway warm-up requests per endpoint before measuring.
    #[arg(long, default_value_t = 2)]
    warmup: usize,

    /// Delay between consecutive requests to the same endpoint, milliseconds.
    #[arg(long, default_value_t = 150)]
    probe_gap_ms: u64,

    /// Per-request HTTP timeout, seconds.
    #[arg(long, default_value_t = 6)]
    timeout_secs: u64,

    /// Override an exchange's endpoint URL, e.g. `okx=https://aws.okx.com/api/v5/public/time`.
    /// Repeatable. Applies to every market of that exchange.
    #[arg(long = "host-override")]
    host_override: Vec<String>,

    /// Output format: table | csv | json.
    #[arg(long, default_value = "table")]
    format: String,

    /// Optional path to write the chosen format to (in addition to stdout summary).
    #[arg(long)]
    out: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let wanted_exchanges = parse_exchange_filter(cli.exchanges.as_deref())?;
    let wanted_markets = parse_market_filter(&cli.markets)?;
    let overrides = parse_overrides(&cli.host_override)?;

    // Select and customise endpoints.
    let mut targets = Vec::new();
    for mut e in all_endpoints() {
        if let Some(set) = &wanted_exchanges {
            if !set.contains(&e.exchange) {
                continue;
            }
        }
        if !wanted_markets.contains(&e.market) {
            continue;
        }
        if let Some(url) = overrides.get(e.exchange) {
            e.url = url.clone();
        }
        targets.push(e);
    }
    if targets.is_empty() {
        return Err(anyhow!("no endpoints match the given --exchanges/--markets"));
    }

    let gap = Duration::from_millis(cli.probe_gap_ms);
    let timeout = Duration::from_secs(cli.timeout_secs);

    // One reqwest Client per host so keep-alive / TLS session reuse is not
    // contended across exchanges. Built once and shared via the spawned tasks.
    let mut handles = Vec::with_capacity(targets.len());
    for e in targets {
        let client = build_client(timeout)?;
        let warmup = cli.warmup;
        let probes = cli.probes;
        handles.push(tokio::spawn(async move {
            probe_endpoint(&client, &e, warmup, probes, gap).await
        }));
    }

    let mut results: Vec<ProbeResult> = Vec::with_capacity(handles.len());
    for h in handles {
        results.push(h.await.context("probe task panicked")?);
    }

    // Stable ordering: registry exchange order, then spot before usdm.
    let order: HashMap<&str, usize> = known_exchanges()
        .into_iter()
        .enumerate()
        .map(|(i, n)| (n, i))
        .collect();
    results.sort_by(|a, b| {
        order
            .get(a.exchange)
            .cmp(&order.get(b.exchange))
            .then_with(|| a.market.cmp(b.market))
    });

    let measured_at = now_rfc3339_ish();

    // stdout always shows the table for a quick glance.
    print!("{}", report::render_table(&results));

    if let Some(path) = &cli.out {
        let content = match cli.format.as_str() {
            "csv" => report::render_csv(&results, &measured_at),
            "json" => report::render_json(&results, &measured_at),
            "table" => report::render_table(&results),
            other => return Err(anyhow!("unknown --format '{other}' (table|csv|json)")),
        };
        std::fs::write(path, content).with_context(|| format!("writing {path}"))?;
        eprintln!("wrote {} ({})", path, cli.format);
    } else if cli.format == "json" {
        // No file: still honour json/csv on stdout after the table separator.
        println!("\n{}", report::render_json(&results, &measured_at));
    } else if cli.format == "csv" {
        println!("\n{}", report::render_csv(&results, &measured_at));
    }

    Ok(())
}

fn build_client(timeout: Duration) -> Result<Client> {
    Client::builder()
        .user_agent("exchange-clock-offset/0.1")
        .timeout(timeout)
        .tcp_nodelay(true)
        // Keep the connection hot between warm-up and measured probes.
        .pool_max_idle_per_host(2)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
        .context("building HTTP client")
}

fn parse_exchange_filter(arg: Option<&str>) -> Result<Option<Vec<&'static str>>> {
    let Some(s) = arg else { return Ok(None) };
    let known = known_exchanges();
    let mut out = Vec::new();
    for raw in s.split(',') {
        let name = raw.trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        let matched = known
            .iter()
            .find(|k| **k == name)
            .ok_or_else(|| anyhow!("unknown exchange '{name}'; known: {}", known.join(",")))?;
        out.push(*matched);
    }
    if out.is_empty() {
        return Ok(None);
    }
    Ok(Some(out))
}

fn parse_market_filter(s: &str) -> Result<Vec<Market>> {
    let mut out = Vec::new();
    for raw in s.split(',') {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        let m = Market::parse(t)?;
        if !out.contains(&m) {
            out.push(m);
        }
    }
    if out.is_empty() {
        return Err(anyhow!("--markets resolved to empty set"));
    }
    Ok(out)
}

fn parse_overrides(items: &[String]) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for item in items {
        let (k, v) = item
            .split_once('=')
            .ok_or_else(|| anyhow!("--host-override must be EXCHANGE=URL, got '{item}'"))?;
        map.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }
    Ok(map)
}

/// Compact UTC timestamp (epoch-ms based) without pulling in a date crate.
fn now_rfc3339_ish() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("epoch_ms={ms}")
}
