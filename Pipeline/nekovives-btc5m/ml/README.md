# ml/ — research plane

Label vs Chainlink → train a **calibrated** P(Up at close) → **cost-aware** OOS backtest.

| file | role |
|---|---|
| `record.py` | **generate REAL data** — live recorder (Binance + RTDS + CLOB) |
| `features.py` | canonical `FEATURE_ORDER` + transforms, incl. live `compute_features` (mirror of `features.rs`) |
| `fee.py` | Polymarket taker-fee model (crypto = highest category) |
| `label.py` | resolve Up/Down against **Chainlink** close vs price-to-beat |
| `gen_synthetic.py` | smoke-test fixture (NOT research data) |
| `train.py` | gradient boosting + isotonic calibration, walk-forward, Brier/reliability |
| `backtest.py` | OOS gate sim with slippage sweep + edge buckets |

## Generating REAL data (the important part)

The model must train on the distribution it will see live, so you **record it
yourself in paper mode** — there is no historical download for this. `record.py`
subscribes to the same three sources as the live Rust plane and writes the exact
schema `train.py`/`backtest.py` consume.

```bash
pip install -r requirements.txt
# verify the offline path first (no network):
python record.py --selftest

# then record live. Run on a low-latency VPS (London/EU) for 2+ weeks, ideally
# 24/7 so you span Asian/European/US sessions:
python record.py --out data --hours 0        # 0 = until Ctrl-C; flushes every 30s
```

It writes `data/samples.parquet` and `data/resolutions.parquet`. Then:

```bash
python train.py --samples data/samples.parquet --resolutions data/resolutions.parquet
python backtest.py --samples data/samples.parquet --resolutions data/resolutions.parquet --secs 12
```

### What it records
- one **sample** row every 5s during the last 60s of each window: the 10 features
  + `pm_up_ask/pm_down_ask` + spreads;
- one **resolution** row per window: the Chainlink close vs the captured price-to-beat.

### Verify once against a live market (formats can drift)
- Gamma discovery assumes slug `btc-updown-5m-{close_unix_s}` (300s-aligned, ts =
  window close). Open one live market page and confirm the slug/close mapping.
- Outcome→token mapping is by the `outcomes` strings ("Up"/"Down"), not index.
- RTDS Chainlink: `crypto_prices_chainlink`, filter `{"symbol":"btc/usd"}`, PING/5s.
- CLOB market: `assets_ids` + `custom_feature_enabled:true`, PING/10s.

## How to judge results
Optimize for **Brier skill score > 0** and a reliability table that tracks the
diagonal — NOT accuracy. The make-or-break test is `backtest.py`'s slippage sweep:
if the edge dies between slippage 0.00 and 0.02, it is not live-viable. The
synthetic ROI from `gen_synthetic.py` is an artifact of the fixture; never read it
as a forecast.
