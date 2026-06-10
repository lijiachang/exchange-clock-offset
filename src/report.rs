//! Rendering of probe results to table (stdout), CSV, or JSON.

use crate::sync::ProbeResult;

/// Convert ns to milliseconds as a float for display/serialisation.
fn ns_to_ms(ns: i64) -> f64 {
    ns as f64 / 1_000_000.0
}

/// Human-readable table for stdout.
pub fn render_table(results: &[ProbeResult]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<9} {:<7} {:>10} {:>10} {:>9} {:>8} {:<12} {}\n",
        "exchange", "market", "offset_ms", "median_ms", "rtt_ms", "samples", "status", "endpoint"
    ));
    for r in results {
        let (offset, median, rtt) = match &r.stats {
            Some(s) => (
                format!("{:+.3}", ns_to_ms(s.offset_best_ns)),
                format!("{:+.3}", ns_to_ms(s.offset_median_ns)),
                format!("{:.3}", ns_to_ms(s.rtt_min_ns)),
            ),
            None => ("-".into(), "-".into(), "-".into()),
        };
        out.push_str(&format!(
            "{:<9} {:<7} {:>10} {:>10} {:>9} {:>8} {:<12} {}\n",
            r.exchange,
            r.market,
            offset,
            median,
            rtt,
            format!("{}/{}", r.samples_ok, r.samples_total),
            r.status(),
            r.url,
        ));
    }
    out
}

/// CSV with the full column set from the plan.
pub fn render_csv(results: &[ProbeResult], measured_at: &str) -> String {
    let mut out = String::new();
    out.push_str(
        "exchange,market,offset_ms,offset_median_ms,offset_p90_ms,rtt_min_ms,rtt_median_ms,samples_ok,samples_total,status,shared,endpoint,measured_at\n",
    );
    for r in results {
        let (o, om, op, rm, rmed) = match &r.stats {
            Some(s) => (
                fmt(ns_to_ms(s.offset_best_ns)),
                fmt(ns_to_ms(s.offset_median_ns)),
                fmt(ns_to_ms(s.offset_p90_ns)),
                fmt(ns_to_ms(s.rtt_min_ns)),
                fmt(ns_to_ms(s.rtt_median_ns)),
            ),
            None => ("".into(), "".into(), "".into(), "".into(), "".into()),
        };
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            r.exchange,
            r.market,
            o,
            om,
            op,
            rm,
            rmed,
            r.samples_ok,
            r.samples_total,
            r.status(),
            r.shared,
            r.url,
            measured_at,
        ));
    }
    out
}

fn fmt(v: f64) -> String {
    format!("{v:.4}")
}

/// JSON array of result objects (hand-built to avoid an extra serde derive layer).
pub fn render_json(results: &[ProbeResult], measured_at: &str) -> String {
    let items: Vec<String> = results
        .iter()
        .map(|r| {
            let stats = match &r.stats {
                Some(s) => format!(
                    "\"offset_ms\":{:.4},\"offset_median_ms\":{:.4},\"offset_p90_ms\":{:.4},\"rtt_min_ms\":{:.4},\"rtt_median_ms\":{:.4}",
                    ns_to_ms(s.offset_best_ns),
                    ns_to_ms(s.offset_median_ns),
                    ns_to_ms(s.offset_p90_ns),
                    ns_to_ms(s.rtt_min_ns),
                    ns_to_ms(s.rtt_median_ns),
                ),
                None => "\"offset_ms\":null,\"offset_median_ms\":null,\"offset_p90_ms\":null,\"rtt_min_ms\":null,\"rtt_median_ms\":null".into(),
            };
            let error = match &r.error {
                Some(e) => format!(",\"error\":{}", json_str(e)),
                None => String::new(),
            };
            format!(
                "  {{\"exchange\":{},\"market\":{},{},\"samples_ok\":{},\"samples_total\":{},\"status\":{},\"shared\":{},\"endpoint\":{},\"measured_at\":{}{}}}",
                json_str(r.exchange),
                json_str(r.market),
                stats,
                r.samples_ok,
                r.samples_total,
                json_str(r.status()),
                r.shared,
                json_str(&r.url),
                json_str(measured_at),
                error,
            )
        })
        .collect();
    format!("[\n{}\n]\n", items.join(",\n"))
}

/// Minimal JSON string escaping.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
