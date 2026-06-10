//! Static registry of exchange server-time endpoints and per-exchange parsers.
//!
//! Every endpoint here was probed live (curl) on 2026-06-10 and the response
//! shape / field encoding recorded. See `parse_server_time_ns` for the exact
//! field each exchange uses.

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

/// Market type. `Shared` means the exchange exposes a single server-time
/// endpoint for both spot and futures, but we still report it under both rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Market {
    Spot,
    Usdm,
}

impl Market {
    pub fn as_str(self) -> &'static str {
        match self {
            Market::Spot => "spot",
            Market::Usdm => "usdm",
        }
    }

    pub fn parse(s: &str) -> Result<Market> {
        match s.to_ascii_lowercase().as_str() {
            "spot" => Ok(Market::Spot),
            "usdm" | "futures" | "swap" | "perp" => Ok(Market::Usdm),
            other => Err(anyhow!("unknown market '{other}' (expected spot|usdm)")),
        }
    }
}

/// One concrete probe target: an exchange + market pointing at a URL, plus the
/// parser id used to decode the server time from the JSON body.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub exchange: &'static str,
    pub market: Market,
    pub url: String,
    /// True when spot and usdm share the same underlying URL (single clock).
    pub shared: bool,
    parser: Parser,
}

#[derive(Clone, Copy, Debug)]
enum Parser {
    /// `{"serverTime": <ms>}` — Binance spot & usdm, MEXC spot.
    BinanceMs,
    /// `{"data":[{"ts":"<ms>"}]}` — OKX (string ms).
    OkxData,
    /// `{"result":{"timeNano":"<ns>"}}` — Bybit (string ns, best resolution).
    BybitNano,
    /// `{"data":{"serverTime":"<ms>"}}` — Bitget (string ms).
    BitgetData,
    /// `{"server_time": <ms>}` — Gate.
    GateMs,
    /// `{"data": <ms>}` — KuCoin spot & futures, MEXC contract ping.
    DataMs,
}

impl Endpoint {
    /// Extract the server time from a parsed JSON body, normalised to epoch ns.
    pub fn parse_server_time_ns(&self, body: &Value) -> Result<i64> {
        match self.parser {
            Parser::BinanceMs => num_field_ms(body, "serverTime"),
            Parser::GateMs => num_field_ms(body, "server_time"),
            Parser::DataMs => any_to_ms(&body["data"]).map(ms_to_ns),
            Parser::OkxData => {
                let ts = body["data"]
                    .get(0)
                    .and_then(|v| v.get("ts"))
                    .ok_or_else(|| anyhow!("missing data[0].ts"))?;
                any_to_ms(ts).map(ms_to_ns)
            }
            Parser::BybitNano => {
                let nano = &body["result"]["timeNano"];
                any_to_i64(nano).context("missing result.timeNano")
            }
            Parser::BitgetData => any_to_ms(&body["data"]["serverTime"]).map(ms_to_ns),
        }
    }
}

fn ms_to_ns(ms: i64) -> i64 {
    ms.saturating_mul(1_000_000)
}

fn num_field_ms(body: &Value, field: &str) -> Result<i64> {
    let v = body
        .get(field)
        .ok_or_else(|| anyhow!("missing field '{field}'"))?;
    any_to_ms(v).map(ms_to_ns)
}

/// Accept either a JSON number or a numeric string, returning the integer value.
fn any_to_i64(v: &Value) -> Result<i64> {
    if let Some(n) = v.as_i64() {
        return Ok(n);
    }
    if let Some(s) = v.as_str() {
        return s
            .trim()
            .parse::<i64>()
            .with_context(|| format!("cannot parse '{s}' as integer"));
    }
    Err(anyhow!("value is neither integer nor numeric string: {v}"))
}

/// Same as `any_to_i64` but semantically a milliseconds value.
fn any_to_ms(v: &Value) -> Result<i64> {
    any_to_i64(v)
}

fn ep(
    exchange: &'static str,
    market: Market,
    url: &str,
    shared: bool,
    parser: Parser,
) -> Endpoint {
    Endpoint {
        exchange,
        market,
        url: url.to_string(),
        shared,
        parser,
    }
}

/// Build the full endpoint table. Filtering by exchange/market happens in main.
pub fn all_endpoints() -> Vec<Endpoint> {
    use Market::*;
    use Parser::*;
    vec![
        // Binance: separate hosts for spot vs USDⓈ-M futures.
        ep("binance", Spot, "https://api.binance.com/api/v3/time", false, BinanceMs),
        ep("binance", Usdm, "https://fapi.binance.com/fapi/v1/time", false, BinanceMs),
        // OKX: one unified public time endpoint (string ms).
        ep("okx", Spot, "https://www.okx.com/api/v5/public/time", true, OkxData),
        ep("okx", Usdm, "https://www.okx.com/api/v5/public/time", true, OkxData),
        // Bybit: unified v5 market time, nanosecond resolution.
        ep("bybit", Spot, "https://api.bybit.com/v5/market/time", true, BybitNano),
        ep("bybit", Usdm, "https://api.bybit.com/v5/market/time", true, BybitNano),
        // Bitget: unified public time (string ms under data.serverTime).
        ep("bitget", Spot, "https://api.bitget.com/api/v2/public/time", true, BitgetData),
        ep("bitget", Usdm, "https://api.bitget.com/api/v2/public/time", true, BitgetData),
        // Gate: only spot/time documented; futures host has no time endpoint.
        ep("gate", Spot, "https://api.gateio.ws/api/v4/spot/time", true, GateMs),
        ep("gate", Usdm, "https://api.gateio.ws/api/v4/spot/time", true, GateMs),
        // KuCoin: distinct spot vs futures hosts, both `data` ms.
        ep("kucoin", Spot, "https://api.kucoin.com/api/v1/timestamp", false, DataMs),
        ep("kucoin", Usdm, "https://api-futures.kucoin.com/api/v1/timestamp", false, DataMs),
        // MEXC: spot serverTime, contract ping returns `data` ms.
        ep("mexc", Spot, "https://api.mexc.com/api/v3/time", false, BinanceMs),
        ep("mexc", Usdm, "https://contract.mexc.com/api/v1/contract/ping", false, DataMs),
    ]
}

/// All distinct exchange ids, in registry order.
pub fn known_exchanges() -> Vec<&'static str> {
    let mut seen = Vec::new();
    for e in all_endpoints() {
        if !seen.contains(&e.exchange) {
            seen.push(e.exchange);
        }
    }
    seen
}
