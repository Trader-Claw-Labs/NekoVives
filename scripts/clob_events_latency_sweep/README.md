# clob_events latency sweep — REAL data (btc_5m_ev)

Latency sweep of the event-driven `clob_events` engine (BACKTEST_ENGINE_PLAN Fase C)
running `src/tools/scripts/clob_events_latency_arb.rhai` over the **real** pmxt v2
event stream, with the 3-leg `edge_validator.py` as the final honesty gate.

## How this was produced

1. Stream generated with `to-events` from local archive parquets:
   ```
   python3 tools/orderbook_parser.py to-events --series-prefix btc-updown-5m \
     --slug btc_5m_ev --binance-symbol BTCUSDT \
     --in ~/.traderclaw/workspace/data/orderbook/ \
     --out ~/.traderclaw/workspace/data/events/
   ```
2. Sweep run by the `#[ignore]` test `clob_events_latency_sweep_export`
   (`src/tools/backtest.rs`, test module). It calls
   `run_clob_events_backtest_from_files(... fee_model="crypto_taker")` at order
   latencies `[0, 110, 220, 500, 1000] ms` (feed latency held at 0) and exports
   each run's trades to `clob_events_lat_<lat>.csv` (`entry_price,won`):
   ```
   cargo test --lib clob_events_latency_sweep_export -- --ignored --nocapture
   ```
3. Each CSV validated with the 3-leg test:
   ```
   python3 scripts/ml/edge_validator.py --source csv --csv clob_events_lat_<lat>.csv
   ```
   (raw output in `edge_validator_verdicts.txt`).

## Data coverage (HONEST)

- **3 days: 2026-05-16 → 2026-05-18** (the days complete on disk when the sweep ran).
- `btc-updown-5m`, 100% **official** Polymarket resolution (off/bin = 375/0 — every
  settled window used the on-chain oracle outcome, zero Binance-price fallback).
- The full 33-day archive run (2026-04-23 → 2026-05-25) is impractically slow on this
  machine: `to-events` uses a Python `df.iterrows()` write loop over the day's raw
  events (e.g. 2026-05-16 = **38.7M** raw book events before dedup → ~10–13 min/day),
  so 33 days ≈ many hours. A focused 10-day run (2026-05-16…25) was generated instead;
  the sweep was executed on the 3 days complete on disk at run time. n≈375 trades.
  (The earlier 2-day run, 05-16..17, gave the identical verdict at n≈200 — see git
  history of this file.)

## Results table

`crypto_taker` fee model (1.8%·p·(1−p)); feed_latency=0; order latency swept.
`ret%` is the **compounded** balance return of the demo script (sizes 5% of a growing
balance) — informative only and absurd by construction (a 76% WR at ~0.5 entry
compounds explosively), NOT an edge measure. The edge verdict is the edge_validator's
per-trade EV across the 3 legs.

| Order latency | trades* | WR    | EV/trade | bootstrap CI (Leg 1)  | Leg 2 (rand-side) | Leg 3 (shuffle) | off/bin res | **VERDICT** |
|---------------|---------|-------|----------|-----------------------|-------------------|-----------------|-------------|-------------|
| 0 ms          | 375     | 76.3% | +68.2%   | [+53.2%, +85.3%] PASS | p=0.000 PASS      | p=0.999 **FAIL** | 375/0       | **NO EDGE** |
| 110 ms        | 375     | 76.3% | +59.9%   | [+47.3%, +72.8%] PASS | p=0.000 PASS      | p=1.000 **FAIL** | 375/0       | **NO EDGE** |
| 220 ms        | 375     | 76.3% | +59.1%   | [+46.9%, +71.7%] PASS | p=0.000 PASS      | p=1.000 **FAIL** | 375/0       | **NO EDGE** |
| 500 ms        | 375     | 76.3% | +67.2%   | [+49.0%, +91.0%] PASS | p=0.000 PASS      | p=1.000 **FAIL** | 375/0       | **NO EDGE** |
| 1000 ms       | 373     | 76.1% | +62.6%   | [+45.8%, +81.4%] PASS | p=0.000 PASS      | p=1.000 **FAIL** | 373/0       | **NO EDGE** |

\* `trades` = engine-recorded trades (375 at ≤500ms, 373 at 1000ms — 2 orders missed
their fill once the future-book window had closed: the latency mechanic working as
designed). The edge_validator's loader keeps only `0.01 < entry < 0.99`, so its n is
slightly lower (375/368/365/367/359) after the latency shifts a few entries to the
extremes; `EV/trade` and the CIs above are reported on that filtered n.

## Verdict (honest)

**NO EDGE at any latency.** The strategy passes Leg 1 (bootstrap CI > 0) and Leg 2
(beats random side-selection) but **fails Leg 3** at every latency (shuffle-null
p ≈ 0.999–1.000). Leg 3 permutes the win/loss labels and finds the observed EV sits
deep inside that null — i.e. the apparent profit comes from *which* windows happened
to win in this 3-day sample, not from a repeatable signal. This is precisely the
data-snooping artifact the validator exists to catch; per the house rule, **do not
commit capital.**

Two secondary findings:
- **Latency-insensitive.** Trades and WR barely change from 0→1000 ms. The demo's
  entries are not in book-moving micro-windows at this resolution, so the future-book
  fill ≈ the signal-time fill. The latency cost only shows up as the occasional missed
  fill (375→373 at 1000 ms). The latency-arb thesis is *not* supported by this slice —
  there is no edge to erode, and what little there is does not erode with latency.
- **Sample is still short for a definitive call.** n≈375 over 3 days. A real edge
  decision needs the multi-week stream; this run validates the *mechanics* (engine +
  CSV export + edge_validator chain, 100% official resolution) and demonstrates the
  gate rejecting a non-edge cheaply — exactly what the engine is for. The verdict was
  identical and consistent at n≈200 (2 days) and n≈375 (3 days).
