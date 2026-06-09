"""Feature definitions, mirrored 1:1 with crates/btc5m-feed/src/features.rs.

The live Rust path and this training path MUST agree on names, order and
formulas, otherwise the model trains on a different distribution than it sees
live. This module is the single source of truth for the order; the Rust
`Features::to_array` follows the same sequence.
"""

from __future__ import annotations

import numpy as np

# Order matters: model inputs are built in exactly this sequence.
FEATURE_ORDER = [
    "dist_bps",          # (chainlink - price_to_beat)/ptb * 1e4   (resolving series)
    "dist_bps_spot",     # (spot - price_to_beat)/ptb * 1e4
    "basis_bps",         # (spot - chainlink)/chainlink * 1e4
    "book_imbalance",    # L1 (bid-ask)/(bid+ask) in [-1,1]
    "book_imbalance_l5", # L5 depth-weighted
    "flow_15s",          # signed taker vol / total vol, 15s
    "flow_5s",           # signed taker vol / total vol, 5s
    "mom_15s_bps",       # spot return over 15s, bps
    "rv_60s_bps",        # realized vol over 60s, bps
    "secs_left",         # seconds remaining in window
]


def bps(numer, denom):
    denom = np.where(np.abs(denom) < 1e-12, np.nan, denom)
    return np.nan_to_num(numer / denom * 1e4)


def imbalance(bid_qty, ask_qty):
    tot = bid_qty + ask_qty
    return np.where(tot < 1e-12, 0.0, (bid_qty - ask_qty) / tot)


def to_matrix(df):
    """Return an (n, 10) float matrix in FEATURE_ORDER from a DataFrame."""
    missing = [c for c in FEATURE_ORDER if c not in df.columns]
    if missing:
        raise ValueError(f"missing feature columns: {missing}")
    return df[FEATURE_ORDER].to_numpy(dtype=float)


# --- Live feature computation (mirror of crates/btc5m-feed/src/features.rs) ---
# Used by record.py so the recorded distribution == the live Rust distribution.

def _bps_scalar(numer, denom):
    return 0.0 if abs(denom) < 1e-12 else float(numer / denom * 1e4)


def _imbalance(bids, asks, levels):
    """bids/asks: lists of (price, qty). Returns depth-weighted OFI in [-1,1]."""
    b = sum(q for _, q in bids[:levels])
    a = sum(q for _, q in asks[:levels])
    tot = b + a
    return 0.0 if tot < 1e-12 else float((b - a) / tot)


def _signed_flow(trades, now_ms, window_ms):
    """trades: list of dict(ts_ms, price, qty, buyer_is_maker). Ratio in [-1,1]."""
    cutoff = now_ms - window_ms
    signed = total = 0.0
    for t in trades:
        if t["ts_ms"] < cutoff:
            continue
        q = t["qty"]
        signed += (-q if t["buyer_is_maker"] else q)
        total += q
    return 0.0 if total < 1e-12 else float(signed / total)


def _realized_vol_bps(trades, now_ms, window_ms):
    cutoff = now_ms - window_ms
    buckets = []  # (sec, last_price)
    for t in trades:
        if t["ts_ms"] < cutoff:
            continue
        sec = t["ts_ms"] // 1000
        if buckets and buckets[-1][0] == sec:
            buckets[-1] = (sec, t["price"])
        else:
            buckets.append((sec, t["price"]))
    if len(buckets) < 3:
        return 0.0
    p = np.array([b[1] for b in buckets], dtype=float)
    rets = np.diff(np.log(p))
    return float(rets.std() * np.sqrt(len(rets)) * 1e4)


def compute_features(*, now_ms, spot, chainlink, price_to_beat, secs_left,
                     bids, asks, trades):
    """Return a dict keyed by FEATURE_ORDER. `trades` newest-last."""
    mom_start = spot
    for t in trades:
        if t["ts_ms"] >= now_ms - 15_000:
            mom_start = t["price"]
            break
    return {
        "dist_bps": _bps_scalar(chainlink - price_to_beat, price_to_beat),
        "dist_bps_spot": _bps_scalar(spot - price_to_beat, price_to_beat),
        "basis_bps": _bps_scalar(spot - chainlink, chainlink),
        "book_imbalance": _imbalance(bids, asks, 1),
        "book_imbalance_l5": _imbalance(bids, asks, 5),
        "flow_15s": _signed_flow(trades, now_ms, 15_000),
        "flow_5s": _signed_flow(trades, now_ms, 5_000),
        "mom_15s_bps": _bps_scalar(spot - mom_start, mom_start),
        "rv_60s_bps": _realized_vol_bps(trades, now_ms, 60_000),
        "secs_left": float(secs_left),
    }
