# late_certainty — latency sweep at low-VPS latencies (jun-2026)

`clob_events_late_certainty.rhai` (on_event port of `clob_1hz_late_certainty`) over the
real btc_5m_ev event stream, contiguous range **2026-05-16 → 2026-05-25** (10 days),
fee model `crypto_taker`, official Polymarket resolution. Motivated by a faster (~50ms)
VPS: does the lower order latency rescue the strategy?

## Result — NO. The verdict is identical at every latency.

| Order latency | trades | WR | EV/trade | Leg1 CI | Leg2 random | Leg3 shuffle | Verdict |
|---|---|---|---|---|---|---|---|
| 0 ms   | 1093 | 70.5% | +90% | ✅ [+73,+109] | ✅ p=0.002 | ❌ p=1.000 | **NO EDGE** |
| 30 ms  | 1081 | 70.5% | +88% | ✅ | ✅ p=0.001 | ❌ p=1.000 | **NO EDGE** |
| 50 ms  | 1081 | 70.5% | +88% | ✅ | ✅ p=0.002 | ❌ p=1.000 | **NO EDGE** |
| 80 ms  | 1081 | 70.5% | +88% | ✅ | ✅ p=0.002 | ❌ p=1.000 | **NO EDGE** |
| 110 ms | 1081 | 70.5% | +88% | ✅ | ✅ p=0.002 | ❌ p=1.000 | **NO EDGE** |

**Latency-insensitive**: trade count barely moves 0→110ms (1093→1081), so the fast VPS
captures the fills fine. WR 70.5% and EV +88% are high, and the side-selection beats
99.8% of skill-less strategies (Leg 2). But **Leg 3 (shuffle-null) fails at p≈1.0 across
the board**: shuffling which trades won reproduces the same edge → the gain comes from
*which windows won in this 10-day period*, not repeatable skill. A faster VPS improves
the EXECUTION of a signal with no predictive power. Do NOT commit capital.

Consistent with the full `validate-all` batch (0 EDGE / 59 strategies) and the
105k-window market-efficiency finding: the Polymarket 5m market is calibrated.

Reproduce:
```
NV_SWEEP_SCRIPT=clob_events_late_certainty.rhai NV_SWEEP_LATS=0,30,50,80,110 \
  cargo test --lib clob_events_latency_sweep_export -- --ignored --nocapture
python3 scripts/ml/edge_validator.py --source csv --csv /tmp/sweep_clob_events_late_certainty_lat_50.csv
```

## ⚠ Known engine issue (worked around here, fix pending)
The `clob_events` engine produced **0 trades** when the event stream had a large DATE GAP
inside [from_date, to_date] — here a stray `2026-04-23` day (generated WITHOUT
`--binance-symbol`, so no `binance_price`) sat 23 days before the contiguous block, and
`list_event_slugs` set from_date=04-23. The 23-day jump corrupted window tracking. The
sweep above was run on the contiguous 05-16→25 block. Fix TODO: the engine should reset
window state on a gap > a few windows, and `to-events` days without binance_price should
not anchor the run. Regenerate 04-23 with `--binance-symbol BTCUSDT` to include it.
