"""Polymarket taker-fee model (Global), mirrored from prob_engine.rs.

fee_per_share(p) = FEE_RATE * p * (1 - p), peaking at p = 0.5.
Crypto is the most expensive category; peak ~ $0.018/share => FEE_RATE ~ 0.072.
Makers pay 0 (and may earn a rebate). These rates change — verify against
https://docs.polymarket.com before trusting a live backtest.
"""

from __future__ import annotations

import numpy as np

# Per-category peak fee per 100 shares (USDC), as of mid-2026. Convert to a rate
# via peak = RATE * 0.25  =>  RATE = peak_per_share / 0.25.
PEAK_PER_100 = {
    "crypto": 1.80,
    "economics": 1.50,
    "politics": 1.00,
    "sports": 0.75,
    "geopolitics": 0.00,
}

CRYPTO_FEE_RATE = (PEAK_PER_100["crypto"] / 100.0) / 0.25  # = 0.072


def taker_fee_per_share(p, fee_rate: float = CRYPTO_FEE_RATE):
    """Vectorized taker fee per share at probability/price ``p``."""
    p = np.clip(np.asarray(p, dtype=float), 0.0, 1.0)
    return fee_rate * p * (1.0 - p)
