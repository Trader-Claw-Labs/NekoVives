"""Generate a synthetic dataset so the whole pipeline runs end-to-end.

This is NOT for research — it's a smoke-test fixture with a *deliberately* small,
realistic edge: the Polymarket "book" lags the true probability (the stale-order
-book effect), so a calibrated model can extract a thin edge that mostly survives
fees and shrinks under slippage. Replace with your recorded live data:
  - `samples.parquet`     : (window_id, t, <FEATURE_ORDER>, pm_up_ask, pm_down_ask,
                             pm_up_spread, pm_down_spread)
  - `resolutions.parquet` : (window_id, chainlink_close, price_to_beat)
"""

from __future__ import annotations

import argparse

import numpy as np
import pandas as pd
from scipy.stats import norm

from features import FEATURE_ORDER


def true_prob_up(dist_bps, secs_left, sigma_bps_per_sqrt_s):
    """P(close >= ptb | now) under driftless GBM. dist in bps of ptb."""
    tau = np.maximum(secs_left, 1e-6)
    vol = sigma_bps_per_sqrt_s * np.sqrt(tau)
    return norm.cdf(dist_bps / np.maximum(vol, 1e-6))


def gen(n_windows: int, seed: int):
    rng = np.random.default_rng(seed)
    sigma = 1.2  # bps per sqrt(second) of BTC at this scale (~tunable)

    sample_rows, res_rows = [], []
    base_ts = 1_750_000_000_000
    for w in range(n_windows):
        ptb = 100_000.0 * (1 + rng.normal(0, 0.01))
        # Simulate a 300s driftless path in bps space.
        steps = 300
        incr = rng.normal(0, sigma, steps)
        path_bps = np.cumsum(incr)  # cumulative bps move from open
        chainlink_close_bps = path_bps[-1]
        chainlink_close = ptb * (1 + chainlink_close_bps / 1e4)
        res_rows.append(
            dict(window_id=w, chainlink_close=chainlink_close, price_to_beat=ptb)
        )

        # Emit snapshots in the last 60s (every 5s).
        for secs_left in range(60, 4, -5):
            t_idx = steps - secs_left
            dist = path_bps[t_idx]
            # spot leads chainlink by a small noisy basis.
            basis = rng.normal(0, 1.5)
            dist_spot = dist + basis
            # crude flow/momentum/imbalance correlated with recent path slope.
            recent = path_bps[max(0, t_idx - 15)] if t_idx >= 1 else 0.0
            mom = dist - recent
            flow15 = np.tanh(mom / 8.0) + rng.normal(0, 0.2)
            flow5 = np.tanh(mom / 5.0) + rng.normal(0, 0.25)
            imb1 = np.clip(np.tanh(mom / 10.0) + rng.normal(0, 0.3), -1, 1)
            imb5 = np.clip(np.tanh(mom / 12.0) + rng.normal(0, 0.2), -1, 1)
            rv = abs(rng.normal(sigma * np.sqrt(60), 3))

            feats = dict(
                dist_bps=dist,
                dist_bps_spot=dist_spot,
                basis_bps=basis,
                book_imbalance=float(np.clip(flow15, -1, 1)),
                book_imbalance_l5=float(imb5),
                flow_15s=float(np.clip(flow15, -1, 1)),
                flow_5s=float(np.clip(flow5, -1, 1)),
                mom_15s_bps=float(mom),
                rv_60s_bps=float(rv),
                secs_left=float(secs_left),
            )

            # Market belief = true prob, but LAGGED by ~10s + noise (stale book).
            lag_idx = max(0, t_idx - 10)
            p_true_now = true_prob_up(dist, secs_left, sigma)
            p_market = true_prob_up(path_bps[lag_idx], secs_left + 10, sigma)
            p_market = float(np.clip(p_market + rng.normal(0, 0.02), 0.02, 0.98))

            spread = 0.02 + abs(rng.normal(0, 0.01))
            up_ask = float(np.clip(p_market + spread / 2, 0.01, 0.99))
            down_ask = float(np.clip((1 - p_market) + spread / 2, 0.01, 0.99))

            row = dict(window_id=w, t=base_ts + w * 300_000 + t_idx * 1000)
            row.update(feats)
            row.update(
                pm_up_ask=up_ask,
                pm_down_ask=down_ask,
                pm_up_spread=spread,
                pm_down_spread=spread,
                _p_true=float(p_true_now),  # for diagnostics only; not a feature
            )
            sample_rows.append(row)

    samples = pd.DataFrame(sample_rows)
    resolutions = pd.DataFrame(res_rows)
    # sanity: feature columns present
    assert all(c in samples.columns for c in FEATURE_ORDER)
    return samples, resolutions


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--windows", type=int, default=4000)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--out", default=".")
    a = ap.parse_args()
    s, r = gen(a.windows, a.seed)
    s.to_parquet(f"{a.out}/samples.parquet")
    r.to_parquet(f"{a.out}/resolutions.parquet")
    print(f"wrote {len(s)} samples across {len(r)} windows")
