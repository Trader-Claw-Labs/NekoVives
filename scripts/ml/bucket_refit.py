#!/usr/bin/env python3
"""
bucket_refit.py — Per-asset refit of the drift-fade buckets, UTC hour-edge map,
and the |drift|-proportional sizing hypothesis. All on OFFICIAL Polymarket
resolution with a TRAIN/TEST time split (70/30) to reject overfit.

Data: polymarket_historical/<slug>.jsonl (P4 + resolution) + min3_<slug>.jsonl (P3).

The drift-fade family bets NO (ctx.sell) when the YES token drifted UP in the last
60s (fade the move). A NO bet wins when resolution == 'down'. entry = no_token_price.
EV/trade (with fee) = won ? (1/entry)*(1-fee) - 1 : -1.

Usage:
  ./bucket_refit.py --slug btc_5m
  ./bucket_refit.py --slug all
"""
import argparse, json, os, sys
import numpy as np
from datetime import datetime, timezone

HIST = os.path.expanduser('~/.traderclaw/workspace/data/polymarket_historical')
FEE = 1.5
ASSETS = ['btc_5m', 'eth_5m', 'sol_5m', 'xrp_5m', 'bnb_5m', 'doge_5m']


def load_asset(slug):
    """Returns list of {wts, p4, no_price, drift, hour, yes_won} sorted by time."""
    p4f = f'{HIST}/{slug}.jsonl'
    p3f = f'{HIST}/min3_{slug}.jsonl'
    p3 = {}
    if os.path.exists(p3f):
        for line in open(p3f):
            try:
                d = json.loads(line); w = d.get('window_open_ts'); yp = d.get('yes_token_price')
                if w is not None and yp is not None:
                    p3[int(w)] = float(yp)
            except Exception:
                pass
    rows = []
    for line in open(p4f):
        try:
            d = json.loads(line)
            res = d.get('resolution')
            if res not in ('up', 'down'):
                continue
            w = int(d['window_open_ts'])
            p4 = float(d['yes_token_price'])
            nop = d.get('no_token_price')
            nop = float(nop) if nop is not None else (1.0 - p4)
            if not (0.0 < p4 < 1.0):
                continue
            p3v = p3.get(w)
            drift = (p4 - p3v) if p3v is not None else 0.0
            hour = datetime.fromtimestamp(w, tz=timezone.utc).hour
            rows.append({'wts': w, 'p4': p4, 'no_price': nop, 'drift': drift,
                         'hour': hour, 'yes_won': 1 if res == 'up' else 0,
                         'has_drift': p3v is not None})
        except Exception:
            pass
    rows.sort(key=lambda x: x['wts'])
    return rows


def ev_no(rows):
    """EV/trade for betting NO on each row. Returns (n, wr, avg_entry, ev_pct)."""
    if not rows:
        return (0, 0, 0, 0)
    evs, wins, entries = [], 0, []
    for r in rows:
        e = r['no_price']
        if e <= 0:
            continue
        won = (r['yes_won'] == 0)  # NO wins when YES lost
        wins += 1 if won else 0
        entries.append(e)
        evs.append((1.0 / e) * (1 - FEE / 100) - 1 if won else -1.0)
    n = len(evs)
    if n == 0:
        return (0, 0, 0, 0)
    return (n, wins / n * 100, np.mean(entries) * 100, np.mean(evs) * 100)


def split(rows, frac=0.7):
    c = int(len(rows) * frac)
    return rows[:c], rows[c:]


def fmt(label, tr, te):
    ntr, wtr, etr, vtr = ev_no(tr)
    nte, wte, ete, vte = ev_no(te)
    if ntr < 15 and nte < 15:
        return None
    hold = '✓ HOLDS' if (vtr > 2 and vte > 2) else ('✗ OOS-fail' if (vtr > 2 and vte < 0) else '~')
    return (f"   {label:30} TRAIN n={ntr:4} EV={vtr:+6.1f}%  |  TEST n={nte:4} EV={vte:+6.1f}%  {hold}")


def analyze_buckets(slug, rows):
    print(f"\n{'='*78}\n{slug.upper()} — bucket refit (NO-side fade), {len(rows)} windows, TRAIN/TEST 70/30")
    print('='*78)
    tr, te = split(rows)
    # drift present only
    trd = [r for r in tr if r['has_drift']]
    ted = [r for r in te if r['has_drift']]

    print("\n  ── P4 band × drift threshold (fade UP drift → bet NO) ──")
    # Grid: P4 bands × drift thresholds
    p4_bands = [(0.32, 0.50), (0.50, 0.68), (0.68, 0.82), (0.32, 0.68), (0.50, 0.82)]
    drift_ths = [0.02, 0.05, 0.08]
    best = []
    for lo, hi in p4_bands:
        for dt in drift_ths:
            f = lambda rs: [r for r in rs if lo <= r['p4'] < hi and r['drift'] >= dt]
            line = fmt(f"P4[{lo:.2f},{hi:.2f}) d>={dt:.2f}", f(trd), f(ted))
            if line:
                print(line)
                ntr, wtr, etr, vtr = ev_no(f(trd)); nte, wte, ete, vte = ev_no(f(ted))
                if vtr > 2 and vte > 2:
                    best.append((vte, f"P4[{lo:.2f},{hi:.2f}) d>={dt:.2f}", nte, vte))
    best.sort(reverse=True)
    if best:
        print(f"\n  ★ Best HOLDING bucket: {best[0][1]} (TEST EV {best[0][3]:+.1f}%, n={best[0][2]})")
    else:
        print("\n  ✗ No NO-side fade bucket holds OOS for this asset.")
    return best


def analyze_hours(slug, rows):
    print(f"\n  ── UTC hour edge map (drift-fade NO, P4[0.32,0.82) d>=0.05) ──")
    fire = [r for r in rows if r['has_drift'] and 0.32 <= r['p4'] < 0.82 and r['drift'] >= 0.05]
    tr, te = split(fire)
    good = []
    for h in range(24):
        trh = [r for r in tr if r['hour'] == h]
        teh = [r for r in te if r['hour'] == h]
        ntr, wtr, etr, vtr = ev_no(trh); nte, wte, ete, vte = ev_no(teh)
        if ntr + nte < 20:
            continue
        flag = ''
        if vtr > 3 and vte > 3:
            flag = '  ✓ both+'; good.append(h)
        elif vtr > 3 and vte < 0:
            flag = '  ✗ flips'
        print(f"   h{h:02d}  TRAIN n={ntr:3} EV={vtr:+6.1f}%  TEST n={nte:3} EV={vte:+6.1f}%{flag}")
    print(f"   → Hours with positive edge in BOTH splits: {good if good else 'none'}")
    return good


def analyze_drift_sizing(slug, rows):
    print(f"\n  ── |drift| sizing hypothesis (bigger drift → stronger fade?) ──")
    fire = [r for r in rows if r['has_drift'] and 0.32 <= r['p4'] < 0.82 and r['drift'] >= 0.05]
    if len(fire) < 60:
        print("   (too few firing trades)")
        return
    drifts = np.array([r['drift'] for r in fire])
    q1, q2 = np.percentile(drifts, [33, 67])
    for lo, hi, lbl in [(0.05, q1, 'small'), (q1, q2, 'medium'), (q2, 1.0, 'large')]:
        sub = [r for r in fire if lo <= r['drift'] < hi]
        n, wr, e, ev = ev_no(sub)
        if n < 15:
            continue
        print(f"   drift[{lo:.3f},{hi:.3f}) {lbl:6} n={n:4} WR={wr:4.1f}% EV/trade={ev:+6.1f}%")


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--slug', default='btc_5m')
    ap.add_argument('--mode', default='all', choices=['all', 'buckets', 'hours', 'sizing'])
    args = ap.parse_args()
    slugs = ASSETS if args.slug == 'all' else [args.slug]
    for slug in slugs:
        rows = load_asset(slug)
        if not rows:
            print(f"{slug}: no data"); continue
        if args.mode in ('all', 'buckets'):
            analyze_buckets(slug, rows)
        if args.mode in ('all', 'hours'):
            analyze_hours(slug, rows)
        if args.mode in ('all', 'sizing'):
            analyze_drift_sizing(slug, rows)
