"""Cost-aware, out-of-sample backtest of the gate.

Trades ONLY the test windows (same chronological split as train.py) so results
are out of sample. For each window it picks one entry near the configured
`secs_left`, runs the same fee-aware gate as the Rust `prob_engine`, and books
realistic PnL: fills at ask + slippage, pays the crypto taker fee, settles
against the Chainlink-resolved label.

The headline question this answers: does the edge survive realistic costs? If it
evaporates when you bump `--slippage` from 0 to 0.02, it is not a live strategy.
"""

from __future__ import annotations

import argparse

import joblib
import numpy as np
import pandas as pd

from fee import taker_fee_per_share
from label import attach_labels
from train import walk_forward


def pick_entries(test: pd.DataFrame, target_secs: float) -> pd.DataFrame:
    """One row per window: the snapshot with secs_left closest to target (>= min)."""
    t = test.copy()
    t["_d"] = (t["secs_left"] - target_secs).abs()
    return t.sort_values("_d").groupby("window_id", as_index=False).first()


def run_gate(p_up, up_ask, down_ask, up_spread, down_spread, slippage,
             min_edge=0.04, max_spread=0.04):
    """Return (side, fill_ask) or (None, None). side in {'up','down'}."""
    best = (None, None, -1e9)
    for side, p, ask, spread in (
        ("up", p_up, up_ask, up_spread),
        ("down", 1 - p_up, down_ask, down_spread),
    ):
        if spread > max_spread or ask <= 0.01 or ask >= 0.99:
            continue
        fee = taker_fee_per_share(ask)
        edge = p - ask - fee - slippage
        if edge > min_edge and edge > best[2]:
            best = (side, ask, edge)
    return best[0], best[1]


def backtest(entries, labels, slippage, stake=25.0, **gate_kw):
    pnl, staked, wins, trades = [], 0.0, 0, 0
    for _, r in entries.iterrows():
        side, ask = run_gate(
            r["p_up"], r["pm_up_ask"], r["pm_down_ask"],
            r["pm_up_spread"], r["pm_down_spread"], slippage, **gate_kw,
        )
        if side is None:
            pnl.append(0.0)
            continue
        fill = min(0.99, ask + slippage)
        shares = stake / fill
        win = labels[r["window_id"]] == (1 if side == "up" else 0)
        payout = shares * (1.0 if win else 0.0)
        fee_usdc = shares * taker_fee_per_share(fill)
        net = payout - stake - fee_usdc
        pnl.append(net)
        staked += stake
        wins += int(win)
        trades += 1
    pnl = np.array(pnl)
    equity = np.cumsum(pnl)
    peak = np.maximum.accumulate(equity)
    max_dd = float((peak - equity).max()) if len(equity) else 0.0
    gains = pnl[pnl > 0].sum()
    losses = -pnl[pnl < 0].sum()
    return dict(
        trades=trades,
        hit_rate=(wins / trades) if trades else float("nan"),
        total_pnl=float(pnl.sum()),
        staked=staked,
        roi=(float(pnl.sum()) / staked) if staked else float("nan"),
        profit_factor=(gains / losses) if losses > 0 else float("inf"),
        max_drawdown=max_dd,
    )


def main(a):
    blob = joblib.load(a.model)
    model = blob["model"]
    feats = blob["features"]

    samples = pd.read_parquet(a.samples)
    resolutions = pd.read_parquet(a.resolutions)
    df = attach_labels(samples, resolutions)
    _, _, test = walk_forward(df)

    entries = pick_entries(test, a.secs)
    entries["p_up"] = model.predict_proba(entries[feats].to_numpy())[:, 1]
    labels = resolutions.assign(
        y=(resolutions["chainlink_close"] >= resolutions["price_to_beat"]).astype(int)
    ).set_index("window_id")["y"].to_dict()

    print(f"out-of-sample windows: {test['window_id'].nunique()}   entry ~{a.secs}s left\n")
    print("slippage sensitivity (the make-or-break test):")
    print(f"{'slip':>6} {'trades':>7} {'hit':>6} {'PnL':>10} {'ROI':>8} {'PF':>6} {'maxDD':>8}")
    for slip in [0.0, 0.005, 0.01, 0.02, 0.03]:
        m = backtest(entries, labels, slip, stake=a.stake, min_edge=a.min_edge)
        print(f"{slip:>6.3f} {m['trades']:>7d} {m['hit_rate']:>6.3f} "
              f"{m['total_pnl']:>10.2f} {m['roi']:>8.4f} {m['profit_factor']:>6.2f} "
              f"{m['max_drawdown']:>8.2f}")

    # Edge-bucket breakdown at the realistic slippage.
    print(f"\nedge-bucket breakdown @ slippage={a.slippage}:")
    e = entries.copy()
    e["fee"] = taker_fee_per_share(e[["pm_up_ask", "pm_down_ask"]].min(axis=1))
    e["edge_up"] = e["p_up"] - e["pm_up_ask"] - e["fee"] - a.slippage
    e["edge_dn"] = (1 - e["p_up"]) - e["pm_down_ask"] - e["fee"] - a.slippage
    e["edge"] = e[["edge_up", "edge_dn"]].max(axis=1)
    for lo, hi in [(-1, 0.04), (0.04, 0.08), (0.08, 0.15), (0.15, 1)]:
        m = e[(e["edge"] >= lo) & (e["edge"] < hi)]
        print(f"  edge [{lo:>5.2f},{hi:>4.2f}): n={len(m):>5}")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", default="model.joblib")
    ap.add_argument("--samples", default="samples.parquet")
    ap.add_argument("--resolutions", default="resolutions.parquet")
    ap.add_argument("--secs", type=float, default=12.0, help="target seconds-left entry")
    ap.add_argument("--stake", type=float, default=25.0)
    ap.add_argument("--min-edge", type=float, default=0.04)
    ap.add_argument("--slippage", type=float, default=0.02)
    main(ap.parse_args())
