# exchange-clock-offset

[English](README.md) | 简体中文

测量**本机时钟与各主流加密货币交易所服务器时钟的偏移（offset）**，同时覆盖
**现货（spot）**和 **U 本位合约（USDⓈ-M futures）**。这是一个一次性快照工具：
探测每个交易所的 REST `/time` 端点，用 midpoint-RTT（NTP / Cristian）算法估算偏移，
打印表格，并可选地导出 CSV / JSON。

`offset_ms > 0` 表示交易所时钟**比本机快**。

## 安装（预编译二进制，无需 Rust）

静态 musl 二进制——下载、解压、运行即可，适用于任意 Ubuntu/Debian/Alpine/CentOS。

```bash
# x86_64（绝大多数 Intel/AMD 云服务器）：
curl -L https://github.com/lijiachang/exchange-clock-offset/releases/latest/download/exchange-clock-offset-x86_64-unknown-linux-musl.tar.gz | tar xz
./exchange-clock-offset

# ARM 服务器（AWS Graviton 等）：
curl -L https://github.com/lijiachang/exchange-clock-offset/releases/latest/download/exchange-clock-offset-aarch64-unknown-linux-musl.tar.gz | tar xz
./exchange-clock-offset
```

> 用 `releases/latest/download/...` 始终拉最新版本；要固定版本就把 `latest/download` 换成 `download/v0.1.0`。

## 工作原理

对每个端点，先发送 `--warmup` 次"丢弃结果"的请求（用于建立 TCP+TLS、预热 keep-alive
连接），再发送 `--probes` 次正式测量请求：

```
t0 = 本地 epoch（请求发出前）
serverTime = 交易所返回的时间（统一换算到纳秒）
t1 = 本地 epoch（收到响应后）
rtt = t1 - t0
offset = serverTime - (t0 + rtt/2)     # 假设来回延迟对称，用中点估计本地时刻
```

最终报告的 `offset_ms` 取自 **RTT 最小**的那次探测（路径最对称 → 最准确）。同时输出
`median`/`p90` 与 `rtt` 列，便于观察 RTT 抖动——抖动越大，偏移估计越不可信。

## 编译

```
cargo build --release
```

## 运行

```
# 全部交易所，现货 + 合约，表格输出到 stdout：
./target/release/exchange-clock-offset

# 指定子集，导出 CSV：
./target/release/exchange-clock-offset --exchanges binance,bybit --format csv --out result.csv

# 导出 JSON：
./target/release/exchange-clock-offset --format json --out result.json
```

### 参数

| 参数 | 默认值 | 含义 |
|---|---|---|
| `--exchanges` | 全部 | 逗号分隔：`binance,okx,bybit,bitget,gate,kucoin,mexc` |
| `--markets` | `spot,usdm` | 要探测的市场 |
| `--probes` | 7 | 每个端点的正式测量次数 |
| `--warmup` | 2 | 每个端点的预热（丢弃）请求次数 |
| `--probe-gap-ms` | 150 | 同一端点相邻请求的间隔 |
| `--timeout-secs` | 6 | 单次请求的 HTTP 超时 |
| `--host-override` | — | `交易所=URL`，可重复（如给 OKX 用，见下文） |
| `--format` | `table` | `table` \| `csv` \| `json` |
| `--out` | — | 将所选格式写入文件 |

## 端点列表

| 交易所 | 现货 | 合约 | 备注 |
|---|---|---|---|
| binance | `api.binance.com/api/v3/time` | `fapi.binance.com/fapi/v1/time` | 现货/合约不同域名 |
| okx | `www.okx.com/api/v5/public/time` | （共用） | 统一端点 |
| bybit | `api.bybit.com/v5/market/time` | （共用） | 纳秒精度 |
| bitget | `api.bitget.com/api/v2/public/time` | （共用） | 统一端点 |
| gate | `api.gateio.ws/api/v4/spot/time` | （共用） | 合约主机无 time 端点 |
| kucoin | `api.kucoin.com/api/v1/timestamp` | `api-futures.kucoin.com/api/v1/timestamp` | 现货/合约不同域名 |
| mexc | `api.mexc.com/api/v3/time` | `contract.mexc.com/api/v1/contract/ping` | 现货/合约不同域名 |

## 说明 / 限制

- REST `/time` 的精度受限于毫秒量化 + RTT 不完全对称，预期精度约 **±1–5 ms**。
  Bybit 返回纳秒，精度更高。
- 探测失败的端点（DNS 拦截、地域限制、超时）会标记为 `unreachable`，**不会中断整个运行**。
  某些网络无法访问 OKX（`www.okx.com`），可用
  `--host-override okx=https://aws.okx.com/api/v5/public/time` 或换到未被限制的网络/colo 运行。
- **限频**会导致 `partial` 状态（例如 OKX 在连续快速探测几次后常返回
  `{"code":"50011","msg":"Requests too frequent"}`）。放慢节奏即可：
  调大间隔 `--probe-gap-ms 500`（默认 150），必要时再减少次数 `--warmup 1 --probes 5`。
- 这是一次性快照——不做周期刷新，因此不涉及长跑时钟漂移问题。
- RTT 偏高（如 100ms+）通常说明本机离交易所服务器较远；放到交易服务器/colo 上运行时
  RTT 会降到个位数毫秒，偏移精度更高。
