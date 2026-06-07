#!/usr/bin/env python3
"""
basis_analysis.py — Is the Polymarket book STALE vs Binance, and is the lag capturable?

The latency-arbitrage thesis (from the pasted research): the CLOB book lags the spot
reference, so a fast taker can buy the mispriced side before resting limit orders reprice.
This script measures that lag DIRECTLY from the tick recorder's dual feed (binance_price +
yes_bid/yes_ask every second), which has been running for ~35 days — no new logger needed.

Three measurements:
  1. LEAD-LAG: cross-correlation between Binance returns and book yes_mid changes at lags
     0..N seconds. If the book reacts to Binance moves from K seconds ago, K is the edge
     window. Lag 0 = efficient (no capturable edge); lag > 0 = stale book.
  2. STALENESS MAGNITUDE: after a sharp Binance move, how far does the book mid still have
     to travel (it hasn't caught up yet) — the gross mispricing available to a taker.
  3. NET EDGE after fees: crypto taker fee = shares × 0.018 × p × (1-p). How much of the
     staleness survives the fee, and how often.

Usage:  ./basis_analysis.py --slug btc_5m [--days 10]
"""
import argparse, json, glob, os
import numpy as np

TICKS = os.path.expanduser('~/.traderclaw/workspace/data/ticks')
CRYPTO_TAKER_FEE = 0.018  # Polymarket crypto taker base, per the research


def load_series(slug, days):
    """Return time-ordered arrays (ts_ms, binance, yes_mid, yes_ask, yes_bid) for one slug."""
    files = sorted(glob.glob(f'{TICKS}/{slug}/*.jsonl'))[-days:]
    ts, bn, mid, ask, bid = [], [], [], [], []
    for f in files:
        for line in open(f):
            try:
                t = json.loads(line)
                ya, yb, bp, tm = t.get('yes_ask', 0), t.get('yes_bid', 0), t.get('binance_price', 0), t.get('ts_ms', 0)
                if ya > 0 and yb > 0 and bp > 0 and tm > 0:
                    ts.append(tm); bn.append(bp); mid.append((ya + yb) / 2); ask.append(ya); bid.append(yb)
            except Exception:
                pass
    return (np.array(ts), np.array(bn), np.array(mid), np.array(ask), np.array(bid))


def lead_lag(ts, bn, mid, max_lag=10):
    """Correlate Binance returns at time t-k with book mid changes at time t, for k=0..max_lag.
    Only over contiguous 1-second steps. Peak-correlation lag = the staleness window."""
    # Keep contiguous 1s steps
    dt = np.diff(ts)
    contiguous = np.abs(dt - 1000) < 400  # ~1s apart
    bn_ret = np.diff(bn) / bn[:-1]
    mid_chg = np.diff(mid)
    bn_ret = np.where(contiguous, bn_ret, np.nan)
    mid_chg = np.where(contiguous, mid_chg, np.nan)
    print("  lag(s)  corr(Binance_ret[t-lag], book_mid_chg[t])")
    best = (0, -1)
    for k in range(0, max_lag + 1):
        if k == 0:
            a, b = bn_ret, mid_chg
        else:
            a, b = bn_ret[:-k], mid_chg[k:]
        m = ~np.isnan(a) & ~np.isnan(b)
        if m.sum() < 1000:
            continue
        c = np.corrcoef(a[m], b[m])[0, 1]
        bar = '#' * int(max(c, 0) * 60)
        print(f"   {k:2}     {c:+.3f}  {bar}")
        if c > best[1]:
            best = (k, c)
    print(f"  → peak correlation at lag {best[0]}s (corr {best[1]:+.3f}). "
          f"{'STALE book — edge window exists' if best[0] >= 1 else 'lag 0 = efficient, no capturable lag'}")
    return best[0]


def staleness_after_move(ts, bn, mid, move_bps=10, horizon_s=5):
    """After Binance moves > move_bps within 2s, how much does the book mid move over the next
    horizon_s (catch-up), and how much remains? Approximates the takeable mispricing."""
    catchups = []
    for i in range(2, len(ts) - horizon_s):
        if ts[i] - ts[i-2] > 3000:
            continue
        ret_bps = (bn[i] - bn[i-2]) / bn[i-2] * 1e4
        if abs(ret_bps) < move_bps:
            continue
        # book mid move over next horizon, in the direction of the binance move
        j = i
        while j < len(ts) and ts[j] - ts[i] < horizon_s * 1000:
            j += 1
        if j >= len(ts):
            continue
        book_move = (mid[j] - mid[i]) * np.sign(ret_bps)  # positive = book caught up the right way
        catchups.append(book_move)
    catchups = np.array(catchups)
    if len(catchups) == 0:
        print("  (no qualifying moves)"); return
    print(f"  After a >{move_bps}bps Binance move ({len(catchups)} events): book mid moves "
          f"{np.median(catchups)*100:+.2f}¢ (median) / {catchups.mean()*100:+.2f}¢ (mean) over next {horizon_s}s")
    print(f"  → that catch-up is the gross mispricing a faster taker could have captured first.")


def net_edge_after_fee(ts, bn, mid, ask, bid, move_bps=10, lag_s=2):
    """Simulate the research's taker play: on a Binance move, take the lagging side at the
    ask, exit at mid after lag_s. Net of the 1.8% crypto taker fee. Reports EV/trade."""
    evs = []
    for i in range(2, len(ts) - lag_s - 1):
        if ts[i] - ts[i-2] > 3000:
            continue
        ret_bps = (bn[i] - bn[i-2]) / bn[i-2] * 1e4
        if abs(ret_bps) < move_bps:
            continue
        # Binance up → YES underpriced → buy YES at ask; down → buy NO (1-bid side)
        up = ret_bps > 0
        entry = ask[i] if up else (1 - bid[i])
        if not (0.02 < entry < 0.98):
            continue
        # exit at book mid after lag_s seconds
        j = i
        while j < len(ts) and ts[j] - ts[i] < lag_s * 1000:
            j += 1
        if j >= len(ts):
            continue
        exit_p = mid[j] if up else (1 - mid[j])
        fee = CRYPTO_TAKER_FEE * entry * (1 - entry)  # per-share, peaks at p=0.5
        # P&L per $1 stake: (exit/entry - 1) minus fee fraction
        pnl = (exit_p / entry - 1) - fee / entry
        evs.append(pnl)
    evs = np.array(evs)
    if len(evs) == 0:
        print("  (no qualifying trades)"); return
    print(f"  Taker sim (move>{move_bps}bps, exit +{lag_s}s, fee 1.8%×p(1-p)): "
          f"n={len(evs)} EV/trade={evs.mean()*100:+.2f}%  median={np.median(evs)*100:+.2f}%  "
          f"win={np.mean(evs>0)*100:.0f}%")
    print(f"  → POSITIVE net EV here = capturable latency edge AT THIS (1Hz) RESOLUTION. "
          f"Negative = the lag is finer than 1s / eaten by fees.")


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--slug', default='btc_5m')
    ap.add_argument('--days', type=int, default=10)
    args = ap.parse_args()

    ts, bn, mid, ask, bid = load_series(args.slug, args.days)
    print(f"\n{'='*70}\nBASIS / STALE-BOOK ANALYSIS — {args.slug}, {len(ts)} ticks ({args.days} days)\n{'='*70}")
    print(f"\n[1] LEAD-LAG (does the book trail Binance?)")
    lead_lag(ts, bn, mid)
    print(f"\n[2] STALENESS MAGNITUDE")
    for mv in (8, 15, 30):
        staleness_after_move(ts, bn, mid, move_bps=mv, horizon_s=5)
    print(f"\n[3] NET EDGE AFTER FEE (the research's taker play)")
    for mv in (8, 15, 30):
        net_edge_after_fee(ts, bn, mid, ask, bid, move_bps=mv, lag_s=2)
