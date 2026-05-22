# Polymarket UP/DOWN 5m — Cross-Asset Analysis & Per-Asset Scripts

Companion notes for the `polymarket_<asset>_updown_5m_*.rhai` family. Empirical
results from the 2026-03-06 → 2026-05-15 window, 1m Binance candles overlaid
with Polymarket P3 (`min3_<asset>_5m.jsonl`) and P4 (`<asset>_5m.jsonl`) windows.

## 1. Why a single BTC-tuned script fails on alts

The original `polymarket_btc_updown_5m_drift_v2_combo.rhai` was calibrated to
BTC's micro-structure. Three structural differences break it on other assets:

| Driver | BTC | DOGE | Effect on BTC script |
|---|---|---|---|
| σ of 5m return | 0.129% | 0.200% (+55%) | `win_pct ±0.02` band saturates on DOGE |
| Autocorr lag-1 (1m) | +0.014 | −0.013 | DOGE mean-reverts; BTC drifts |
| P4 mid-band share | 26.5% | 15.6% | §1 setups fire 40% less often on alts |
| Accumulation share | 50.2% | 52.8% | DOGE in net-bid regime |
| US/Asia vol ratio | 1.28× | 0.93× | DOGE moves more in Asia hours |

## 2. Setup-by-setup edge across the 6 assets ($1/bet)

| Setup | Logic (P4 range / drift / candle) | BTC | ETH | SOL | XRP | DOGE | HYPE |
|---|---|---|---|---|---|---|---|
| 1 | 0.50-0.68, dft≤-0.05, buy YES | −$3.9 | −$2.2 | **−$17** | +$1.7 | −$8.1 | −$2.1 |
| 2 | 0.50-0.68, dft≥0.05, sell NO | −$14.8 | −$16.5 | **+$37.3** | −$3.7 | +$3.6 | **+$38.9** |
| 3 | 0.50-0.68, -0.05<dft≤-0.02, buy YES | +$6.3 | −$3.6 | −$2.8 | −$2.8 | +$1.5 | −$4.7 |
| 4 | 0.32-0.50, dft≥0.05, sell NO | −$13.4 | **+$9.0** | +$0.1 | **+$8.6** | **+$8.3** | **+$7.3** |
| 6 | 0.32-0.50, 0.02≤dft<0.05, sell NO | +$7.6 | +$3.0 | −$3.8 | −$8.3 | +$2.1 | +$0.6 |
| 7 | 0.18-0.32, stable dft, buy YES | **−$44.2** | +$6.8 | −$1.1 | −$2.4 | **−$23.9** | **+$32.7** |
| 8 | 0.68-0.82, stable dft, sell NO | **+$67.3** | **+$23.6** | **+$19.7** | **+$13.8** | **+$15.6** | **+$40.4** |
| 9 | 0.05-0.18, w%>-0.02 ∧ rsi<44, buy YES | **+$63.0** | **+$40.4** | −$2.6 | −$11.3 | +$12.5 | — |
| 10 | 0.82-0.95, w%<0.02 ∧ rsi>56, sell NO | **+$92.8** | +$2.1 | **+$111.3** | +$16.1 | +$2.0 | — |

Bold = highest contributor for that asset. HYPE has no candle data → §2 setups (9, 10) cannot fire.

**Universal observations**:
- **Setup 8** is positive on all 6 → always include.
- **Setup 4** is positive or breakeven on 5/6 (BTC the only loss).
- **Setup 7** loses on 5/6 (HYPE is the singular outlier).
- **Setup 10** is mostly carried by BTC + SOL.

## 3. Strategy selection matrix

| Script | Setups kept | Setups removed | Stake notes |
|---|---|---|---|
| `polymarket_eth_updown_5m_btcproxy.rhai`         | 2(0.5×), 4, 6, 8, 9 | 1, 3, 5, 7, 10 | half stake on setup 2 |
| `polymarket_sol_updown_5m_favorite_fade.rhai`    | 2, 4, 8, 10(1.5×) | 1, 3, 5, 6, 7, 9 | oversize setup 10 (60% WR) |
| `polymarket_xrp_updown_5m_lowvol_drift.rhai`     | 1(0.7×), 4, 8(0.7×), 10(0.7×) | 2, 3, 5, 6, 7, 9 | all at 0.7× (low edge / low σ) |
| `polymarket_doge_updown_5m_meanrev.rhai`         | 1, 4, 6, 8, 9, 11, 12 | 2, 3, 5, 7, 10 | adds spike-fade setups 11/12 with 2× thresholds |
| `polymarket_hype_updown_5m_thinmkt.rhai`         | 2, 4, 6, 7, 8 | 1, 3, 5, 9, 10 | drift-only, no candle filters |
| `polymarket_all_updown_5m_adaptive.rhai`         | 2 (low-vol), 4, 8, 9, 10 | 1, 3, 5, 6, 7 | ATR-scaled win_pct band |

## 4. Volatility / accumulation / time-of-day fingerprints

```
asset   ret5m_σ%  ann_vol%  ATR14%  pct_up  ac1     ac5      Hurst   accum_up%  wknd/wkd  US/Asia
btc        0.129      42.0   0.057   50.0%  +0.014  -0.008    0.496    50.2%      0.61     1.28
eth        0.120      38.9   0.052   49.2%  +0.019  -0.005    0.504    49.8%      0.57     1.12
sol        0.115      37.3   0.058   46.7%  +0.025  -0.015    0.503    48.1%      0.64     1.05
xrp        0.105      34.1   0.049   48.2%  +0.019  -0.008    0.499    48.7%      0.66     1.06
doge       0.200      64.7   0.101   48.4%  -0.013  +0.004    0.483    52.8%      0.59     0.93
```

**Read this as**:
- **DOGE**: 60% more vol, mean-rev tilt, accumulation regime, Asia-stronger.
- **SOL**: lowest pct_up (46.7%) → asymmetric bear bias → favor the NO side.
- **XRP**: lowest vol + highest weekend activity → narrow trading window during weekday US hours.
- **ETH**: BTC clone with slightly weaker drift signal.
- **HYPE**: no candles, but its Polymarket bettors mis-price both extremes and mid-band — pure drift-only strategy beats all alts in WR (53.4%).

## 5. How to deploy

```bash
# Backtest a new script on its asset
trader-claw backtest \
  --script polymarket_sol_updown_5m_favorite_fade.rhai \
  --market polymarket_binary \
  --symbol SOLUSDT \
  --interval 5m \
  --from 2026-04-01 --to 2026-05-15

# Promote to live (workspace lookup)
cp src/tools/scripts/polymarket_sol_updown_5m_favorite_fade.rhai \
   ~/.traderclaw/workspace/scripts/
```

All six scripts are bundled in the binary (see `DEFAULT_SCRIPTS` in
`src/tools/backtest.rs`) and will appear automatically in the dashboard
strategy picker after `cargo build`.

## 6. Implemented improvements (2026-05-21)

### Idea B — Disabled YES side on ALL adaptive
`polymarket_all_updown_5m_adaptive.rhai` setup 9 removed.
Dry-run data showed 628 bets at 10.8% WR across SOL/ETH/DOGE (−$1,500+).
The script is now NO-token-only. Setup 9 code remains commented out so it
can be re-enabled after the 1Hz tick data produces a recalibrated threshold.

### Idea 1 — 1Hz Tick Recorder (`src/tick_recorder.rs`)
Records every second: Polymarket YES/NO bid/ask + mid-price, Binance spot
price (miniTicker WebSocket), optional Chainlink oracle price, oracle lag,
window_ts, window_secs_left. Data written to JSONL (daily rotation, 7-day
retention) under `<workspace>/data/ticks/<slug>/<YYYY-MM-DD>.jsonl`.

Tool: `tick_recorder` with actions `start | stop | status | read`.

Example:
```
tick_recorder action=start slug=btc_5m condition_id=0x... binance_symbol=BTCUSDT
tick_recorder action=read  slug=btc_5m last_n=60
tick_recorder action=stop  slug=btc_5m
```

### Idea 3 — Binance WebSocket + Chainlink oracle in ctx (`src/live_feed.rs`)
Already implemented feeds, now exposed in the Rhai decision context:
- `ctx.binance_mark`     — live Binance spot price at decision time
- `ctx.chainlink_mark`   — Chainlink oracle price (0 if not configured)
- `ctx.oracle_lag_secs`  — seconds since Chainlink last updated

Configure in `live_strategies.json`:
```json
"chainlink_endpoint_url": "https://your-chainlink-rest-url",
"chainlink_api_key": "optional-bearer-token",
"chainlink_interval_secs": 1
```

### Idea A — Dynamic asset selector (`src/tools/asset_selector.rs`)
Rolling 30-day win-rate tracker per (script × symbol). Auto-records every
resolved trade. Weights are proportional to rolling WR.

Tool: `asset_selector` with actions `record | weights | summary | clear`.

Example:
```
asset_selector action=summary
asset_selector action=weights min_trades=10
```

### Idea 2 — `ctx.minute_offset` (early-fire support)
`ctx.minute_offset` is now injected in every live signal call:
- `0` = evaluated at the standard decision candle (minute 4 for 5m window)
- `1` = evaluated 1 minute early (minute 3)
- `N` = evaluated N minutes before window close

Scripts can use this to apply tighter conditions when firing early:
```rhai
// Only allow strong signal when firing early
let min_drift = if ctx.minute_offset > 0 { 0.08 } else { 0.05 };
if dft >= min_drift { ctx.sell(1.0); }
```

Configure early fire in the runner config:
```json
"early_fire_secs": 30
```
