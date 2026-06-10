# exchange-clock-offset

English | [简体中文](README.zh-CN.md)

Measure the **clock offset** between this machine and major crypto exchange
servers, for both **spot** and **USDⓈ-M futures**. A one-shot snapshot tool:
it probes each exchange's REST `/time` endpoint, estimates the offset with a
midpoint-RTT (NTP / Cristian) method, and prints a table (optionally writing
CSV/JSON).

`offset_ms > 0` means the exchange clock is **ahead** of the local clock.

## How it works

For each endpoint it sends `--warmup` throwaway requests (to establish
TCP+TLS and warm the keep-alive connection), then `--probes` measured requests:

```
t0 = local epoch (before request)
serverTime = exchange-reported time (normalised to ns)
t1 = local epoch (after response)
rtt = t1 - t0
offset = serverTime - (t0 + rtt/2)     # assume symmetric latency
```

The reported `offset_ms` is taken from the probe with the **lowest RTT**
(least path asymmetry → most accurate). `median`/`p90` and `rtt` columns are
also reported so RTT jitter — which degrades offset accuracy — is visible.

## Build

```
cargo build --release
```

## Run

```
# All exchanges, spot + usdm, table to stdout:
./target/release/exchange-clock-offset

# Subset, write CSV:
./target/release/exchange-clock-offset --exchanges binance,bybit --format csv --out result.csv

# JSON:
./target/release/exchange-clock-offset --format json --out result.json
```

### Options

| Flag | Default | Meaning |
|---|---|---|
| `--exchanges` | all | comma list: `binance,okx,bybit,bitget,gate,kucoin,mexc` |
| `--markets` | `spot,usdm` | which markets to probe |
| `--probes` | 7 | measured probes per endpoint |
| `--warmup` | 2 | throwaway warm-up requests per endpoint |
| `--probe-gap-ms` | 150 | delay between requests to one endpoint |
| `--timeout-secs` | 6 | per-request HTTP timeout |
| `--host-override` | — | `EXCHANGE=URL`, repeatable (e.g. for OKX, see below) |
| `--format` | `table` | `table` \| `csv` \| `json` |
| `--out` | — | write the chosen format to a file |

## Endpoints

| Exchange | spot | usdm | notes |
|---|---|---|---|
| binance | `api.binance.com/api/v3/time` | `fapi.binance.com/fapi/v1/time` | separate hosts |
| okx | `www.okx.com/api/v5/public/time` | (shared) | unified |
| bybit | `api.bybit.com/v5/market/time` | (shared) | nanosecond resolution |
| bitget | `api.bitget.com/api/v2/public/time` | (shared) | unified |
| gate | `api.gateio.ws/api/v4/spot/time` | (shared) | futures host has no time endpoint |
| kucoin | `api.kucoin.com/api/v1/timestamp` | `api-futures.kucoin.com/api/v1/timestamp` | separate hosts |
| mexc | `api.mexc.com/api/v3/time` | `contract.mexc.com/api/v1/contract/ping` | separate hosts |

## Notes / limitations

- REST `/time` precision is bounded by ms quantisation + RTT asymmetry, so
  expect ~±1–5 ms accuracy. Bybit reports ns and is finer.
- Endpoints that fail (DNS block, geo-restriction, timeout) are marked
  `unreachable` and do not abort the run. Some networks cannot reach OKX
  (`www.okx.com`); use `--host-override okx=https://aws.okx.com/api/v5/public/time`
  or run from an unblocked network/colo.
- This is a snapshot — no periodic refresh, so no long-run drift handling.
