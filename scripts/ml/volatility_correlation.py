#!/usr/bin/env python3
"""
volatility_correlation.py — Does BTC volatility regime predict WHEN drift strategies win/lose?

The hypothesis (much stronger than naive direction prediction): drift-fade strategies
have edge ONLY in certain volatility regimes. In calm markets, mean-reversion of the
token price works; in trending/high-vol markets it fails. If true, we can GATE the
runner to only trade in favorable regimes — a real, mechanistic edge.

We reconstruct 5-min BTC OHLC from tick binance_price, compute volatility indicators
using ONLY past data (no lookahead), then for each window where the drift signal would
fire, we record win/loss and bucket by each indicator.

Indicators (all computable from OHLC, no external feed needed):
  - atr_14          : Average True Range (14 windows)
  - stdev_30        : rolling 30-window stdev of returns (proxy for 30d realized vol)
  - bb_width        : Bollinger Band width (20-window, 2σ) / SMA — normalized
  - keltner_pos     : where price sits in the Keltner channel (EMA20 ± 2·ATR)
  - chaikin_vol     : Chaikin Volatility = % change of EMA(H-L) over 10 windows
  - rv_ratio        : realized-vol-1h / realized-vol-6h (the drift_v3 regime filter)
  - range_pct       : current window (H-L)/O — instantaneous volatility

For the WIN/LOSS label we replay the drift-fade core signal (drift = P4 token price
minus P3 = price 60s earlier; fade it) and use the OFFICIAL window_yes_won resolution.

Note: BVIV / BVOL24H / DVOL are Deribit/options-implied indices NOT in our dataset.
We use realized-volatility proxies derived from price, which capture the same regime
information for this purpose.

Usage:
  ./volatility_correlation.py --slug btc_5m
"""
import argparse, json, os, glob, sys, collections
import numpy as np

HIST_DIR = os.path.expanduser('~/.traderclaw/workspace/data/polymarket_historical')
TICKS_DIR = os.path.expanduser('~/.traderclaw/workspace/data/ticks')


def reconstruct_windows(slug, require_resolution=True):
    """From tick JSONL: {window_ts: dict(o,h,l,c, yes_won)} using binance_price + official res.

    STREAMING reconstruction — accumulates only o/h/l/c/yes_won per window (5 floats),
    never the full per-tick price list, so it stays memory-safe on multi-GB tick dirs.
    Pass require_resolution=False to keep windows lacking an official outcome (yes_won=None).
    """
    files = sorted(glob.glob(f'{TICKS_DIR}/{slug}/*.jsonl'))
    win = {}  # wts -> [o, h, l, c, yes_won, n]
    for f in files:
        with open(f) as fp:
            for line in fp:
                try:
                    t = json.loads(line)
                    wts = t.get('window_ts', 0)
                    if wts <= 0:
                        continue
                    bp = t.get('binance_price', 0)
                    if wts not in win:
                        win[wts] = [None, None, None, None, None, 0]
                    w = win[wts]
                    if bp > 0:
                        if w[0] is None:
                            w[0] = bp; w[1] = bp; w[2] = bp
                        w[1] = max(w[1], bp); w[2] = min(w[2], bp); w[3] = bp; w[5] += 1
                    wyw = t.get('window_yes_won')
                    if wyw is not None:
                        w[4] = 1 if wyw else 0
                except Exception:
                    pass
    out = {}
    for wts, w in win.items():
        if w[5] < 10:
            continue
        if require_resolution and w[4] is None:
            continue
        out[wts] = {'o': w[0], 'h': w[1], 'l': w[2], 'c': w[3], 'yes_won': w[4]}
    return out


def load_token_prices(slug):
    """{window_ts: (p4, p3)} from historical datasets for the drift signal."""
    p4f = f'{HIST_DIR}/{slug}.jsonl'
    p3f = f'{HIST_DIR}/min3_{slug}.jsonl'
    p4 = {}
    if os.path.exists(p4f):
        for line in open(p4f):
            try:
                d = json.loads(line); wts = d.get('window_open_ts'); yp = d.get('yes_token_price')
                if wts is not None and yp is not None:
                    p4[int(wts)] = [float(yp), None]
            except Exception: pass
    if os.path.exists(p3f):
        for line in open(p3f):
            try:
                d = json.loads(line); wts = d.get('window_open_ts'); yp = d.get('yes_token_price')
                if wts is not None and yp is not None and int(wts) in p4:
                    p4[int(wts)][1] = float(yp)
            except Exception: pass
    return p4


def ema(arr, period):
    if len(arr) < 1:
        return np.array([])
    alpha = 2 / (period + 1)
    out = np.zeros(len(arr))
    out[0] = arr[0]
    for i in range(1, len(arr)):
        out[i] = alpha * arr[i] + (1 - alpha) * out[i - 1]
    return out


def compute_indicators(wts_sorted, W):
    """Return per-window dict of volatility indicators, using only PAST windows (no lookahead)."""
    o = np.array([W[w]['o'] for w in wts_sorted])
    h = np.array([W[w]['h'] for w in wts_sorted])
    l = np.array([W[w]['l'] for w in wts_sorted])
    c = np.array([W[w]['c'] for w in wts_sorted])
    n = len(c)

    # True Range
    tr = np.zeros(n)
    tr[0] = h[0] - l[0]
    for i in range(1, n):
        tr[i] = max(h[i] - l[i], abs(h[i] - c[i-1]), abs(l[i] - c[i-1]))
    atr14 = ema(tr, 14)

    # Returns for stdev
    ret = np.zeros(n)
    ret[1:] = np.diff(c) / c[:-1]

    ind = {}
    for i in range(n):
        wts = wts_sorted[i]
        # All features use data UP TO AND INCLUDING window i-1 close → known at i open.
        # (We use index i's OHLC only for "current range" which is realized during the window,
        #  but we evaluate the signal at decision time so we use i-1 for safety.)
        if i < 30:
            continue
        sma20 = c[i-20:i].mean()
        std20 = c[i-20:i].std()
        bb_width = (4 * std20 / sma20) if sma20 > 0 else 0  # 2σ both sides / mean
        stdev_30 = ret[i-30:i].std()
        atr = atr14[i-1] if i >= 1 else tr[i]
        ema20 = ema(c[:i], 20)[-1] if i >= 20 else c[i-1]
        keltner_up = ema20 + 2 * atr
        keltner_dn = ema20 - 2 * atr
        kr = keltner_up - keltner_dn
        keltner_pos = (c[i-1] - keltner_dn) / kr if kr > 0 else 0.5
        # Chaikin Vol: EMA of (H-L), % change over 10
        hl = h[:i] - l[:i]
        ema_hl = ema(hl, 10)
        chaikin = ((ema_hl[-1] - ema_hl[-11]) / ema_hl[-11] * 100) if len(ema_hl) > 11 and ema_hl[-11] > 0 else 0
        # RV ratio (drift_v3 regime filter): stdev of last 12 vs last 72 (1h vs 6h in 5m windows)
        rv_short = ret[max(0,i-12):i].std()
        rv_long = ret[max(0,i-72):i].std()
        rv_ratio = (rv_short / rv_long) if rv_long > 1e-9 else 1.0
        ind[wts] = {
            'atr_14': atr,
            'atr_pct': atr / c[i-1] * 100 if c[i-1] > 0 else 0,
            'stdev_30': stdev_30 * 100,
            'bb_width': bb_width * 100,
            'keltner_pos': keltner_pos,
            'chaikin_vol': chaikin,
            'rv_ratio': rv_ratio,
        }
    return ind


def drift_signal(p4, p3):
    """drift-fade core: if drift (p4-p3) <= -0.05 → bet YES; if >= +0.05 → bet NO.
    Returns 'yes', 'no', or None."""
    if p3 is None or p4 is None or p4 <= 0 or p4 >= 1:
        return None
    if p4 < 0.30 or p4 > 0.70:
        return None
    drift = p4 - p3
    if drift <= -0.05:
        return 'yes'
    if drift >= 0.05:
        return 'no'
    return None


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--slug', default='btc_5m')
    args = ap.parse_args()

    print(f"Reconstructing windows for {args.slug}...", file=sys.stderr)
    W = reconstruct_windows(args.slug)
    tok = load_token_prices(args.slug)
    wts_sorted = sorted(W.keys())
    print(f"  {len(wts_sorted)} windows with OHLC + official resolution", file=sys.stderr)

    ind = compute_indicators(wts_sorted, W)

    # Build trade records: window where drift fires, with win/loss + indicators
    trades = []
    for wts in wts_sorted:
        if wts not in ind or wts not in tok:
            continue
        p4, p3 = tok[wts]
        sig = drift_signal(p4, p3)
        if sig is None:
            continue
        yes_won = W[wts]['yes_won']
        won = (sig == 'yes' and yes_won == 1) or (sig == 'no' and yes_won == 0)
        rec = dict(ind[wts])
        rec['won'] = 1 if won else 0
        rec['entry'] = p4 if sig == 'yes' else (1 - p4)
        trades.append(rec)

    print(f"\n{'='*74}")
    print(f"VOLATILITY-REGIME CORRELATION — {args.slug} drift-fade signal")
    print(f"{'='*74}")
    print(f"Total signal-firing windows: {len(trades)}")
    if len(trades) < 100:
        print("Too few trades for reliable correlation.")
        sys.exit(0)

    base_wr = np.mean([t['won'] for t in trades]) * 100
    base_entry = np.mean([t['entry'] for t in trades])
    print(f"Overall WR: {base_wr:.1f}%  |  avg entry (break-even): {base_entry*100:.1f}%  "
          f"|  edge: {base_wr - base_entry*100:+.1f}pts\n")

    # For each indicator, bucket into terciles and show WR + edge per bucket
    indicators = ['atr_pct', 'stdev_30', 'bb_width', 'keltner_pos', 'chaikin_vol', 'rv_ratio']
    fee = 1.5
    for key in indicators:
        vals = np.array([t[key] for t in trades])
        won = np.array([t['won'] for t in trades])
        entry = np.array([t['entry'] for t in trades])
        # Terciles
        q1, q2 = np.percentile(vals, [33, 67])
        print(f"── {key} ──")
        for lo, hi, lbl in [(-np.inf, q1, 'LOW '), (q1, q2, 'MID '), (q2, np.inf, 'HIGH')]:
            mask = (vals >= lo) & (vals < hi) if hi != np.inf else (vals >= lo)
            if mask.sum() < 20:
                continue
            wr = won[mask].mean() * 100
            be = entry[mask].mean() * 100
            edge = wr - be
            # EV per $ with fee
            ev = np.mean([(1/e*(1-fee/100)-1) if w else -1 for w, e in zip(won[mask], entry[mask])]) * 100
            flag = '  ← EDGE' if edge > 4 else ('  ✗ losing' if edge < -2 else '')
            print(f"   {lbl} [{lo if lo!=-np.inf else 'min':>6}, {hi if hi!=np.inf else 'max':>6}): "
                  f"n={mask.sum():4} WR={wr:5.1f}% break-even={be:5.1f}% edge={edge:+5.1f}pts EV={ev:+5.1f}%{flag}")
        print()
