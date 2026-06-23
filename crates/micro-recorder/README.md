# micro-recorder

High-frequency BTC **market-microstructure recorder**. Captures, in real time and
to disk, everything needed to study how BTC moves at the ms/minute scale around
Polymarket's BTC Up/Down 5-minute markets — for **offline analysis**, not trading.

## What it records

Four independent WebSocket feeds funnel into one recorder task (mpsc, no shared
locks on the hot path):

| Feed | Source | Used for |
|------|--------|----------|
| Binance **SPOT** aggTrade + depth20@100ms | `stream.binance.com` | OBI, OFI, CVD, VAMP |
| Binance **PERP** aggTrade + depth20@100ms + **forceOrder** + **markPrice@1s** | `fstream.binance.com` | perp OBI/OFI/CVD, **liquidations**, funding/basis |
| **Chainlink RTDS** `crypto_prices_chainlink` | `ws-live-data.polymarket.com` | the price Polymarket actually resolves against |
| **Polymarket CLOB** market-channel | `ws-subscriptions-clob.polymarket.com/ws/market` | live two-sided Up/Down book + trades |

The Polymarket window (token ids) is re-discovered every 5 min via the Gamma API
(`btc-updown-5m-<window_open_unix>`) and the WS re-subscribes seamlessly; it holds
current+next window tokens so there is no gap at the boundary.

## Output

Two daily-rotating gzip JSONL planes per slug:

```
<out>/<slug>/<YYYY-MM-DD>/events.jsonl.gz    # every raw event, full fidelity
<out>/<slug>/<YYYY-MM-DD>/metrics.jsonl.gz   # OBI/OFI/CVD/liq/VAMP/basis @ N Hz
```

- **events** lets you recompute ANY metric offline with a different window/definition
  (raw kinds: `cex_trade`, `cex_book` (top-10 levels/side), `liquidation`, `mark`,
  `oracle`, `pm_book`, `pm_price_change`, `pm_trade`, `window`).
- **metrics** is the ready-to-use derived time series (see `MetricSnapshot` in
  `src/metrics.rs`). Note: in-progress (today's) file has no gzip trailer until
  rotation/shutdown — read tolerantly, or read the rotated prior-day file strictly.

### Metrics computed live
OBI (L1 & L5), OFI (Cont/Kukanov/Stoikov best-level flow, 5s window), CVD (per
venue: perp + spot), VAMP (volume-adjusted mid), signed liquidation notional (60s),
mark/index/funding, **basis_bps = (perp_mid − chainlink)/chainlink × 1e4**, and the
Polymarket Up/Down top-of-book + window seconds-left.

## Usage

```bash
cargo build -p micro-recorder --release
./target/release/micro-recorder \
  --slug btc_5m \
  --out ~/.traderclaw/workspace/data/micro \
  --metrics-hz 5            # snapshots/sec; 0 disables the metrics plane
# flags: --no-spot --no-perp --no-poly --no-chainlink
```

Graceful on SIGINT/SIGTERM (flushes + finishes both gzip streams). Suitable for a
systemd unit on a VPS.

## ⚠️ Known network limitation (validated 2026-06-22)

`fstream.binance.com` (Binance **Futures** push-WS) is **geo/network-restricted from
some regions**: it delivers the futures `depth` book but silently drops `aggTrade`,
`markPrice` and `forceOrder` on the same combined connection. Confirmed independently
with a raw Python WS client — it is NOT a parser bug (spot, sharing the same
code path, delivers everything). Consequences when restricted:

- `liquidation` / `mark` raw events and `funding_rate` / `index_price` metrics = 0/empty.
- `perp` CVD stays flat → **`cvd_total_spot` is the fallback** (spot tape always works).
- `basis_bps` and `perp_obi/ofi` still work (the futures **depth** book is delivered).

To capture liquidations/funding, run from an unrestricted region or via a VPS whose
egress Binance does not block (e.g. most non-US cloud regions).
