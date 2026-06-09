"""Label windows by their ACTUAL resolution: Chainlink close vs price-to-beat.

Polymarket resolves "Up" if the Chainlink reference price at window close is
>= the price-to-beat (the Chainlink reference at open). Labeling against Binance
close instead is the most common silent bug — it trains the model on a target
that differs from the one it's paid on whenever spot and Chainlink diverge.
"""

from __future__ import annotations

import pandas as pd


def resolve_label(chainlink_close, price_to_beat):
    """1 if the window resolves Up, else 0. ">=" per Polymarket rules."""
    return int(chainlink_close >= price_to_beat)


def attach_labels(samples: pd.DataFrame, resolutions: pd.DataFrame) -> pd.DataFrame:
    """Join per-window resolution onto per-snapshot samples.

    `samples`     : rows of (window_id, t, features...).
    `resolutions` : one row per window_id with (chainlink_close, price_to_beat).
    """
    res = resolutions.copy()
    res["y"] = (res["chainlink_close"] >= res["price_to_beat"]).astype(int)
    out = samples.merge(res[["window_id", "y"]], on="window_id", how="inner")
    return out
