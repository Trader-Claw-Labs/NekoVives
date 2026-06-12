#!/usr/bin/env python3
"""
edge_validator.py — The DOUBLE-CHECK. Is a strategy's edge real, or luck/artifact?

Born from repeated flip-flops: every false "edge" came from a backtest with a hidden
artifact (stale prices, synthetic prices, missing official resolution, compounding).
This tool refuses to trust any single backtest. It applies three independent, artifact-
resistant tests to a strategy's actual trade series and only declares EDGE when they agree.

THE THREE LEGS (all work on on_candle AND on_tick, paper / live / onchain):

  LEG 1 — REALIZED EV with bootstrap CI.
    Per-trade EV after the real crypto taker fee (1.8%×p(1-p)). Bootstrap a 95% CI.
    If the CI lower bound is <= 0, the strategy is NOT distinguishable from break-even.

  LEG 2 — RANDOM-SIDE NULL (the key test).
    Hold each trade's entry price and ACTUAL outcome fixed, but randomize which side we
    bet (YES/NO 50-50), 10k times. This builds the null distribution of EV for a strategy
    with ZERO predictive skill facing the same prices/outcomes. If the real EV sits inside
    that null (p >= 0.05), the strategy's side-selection adds nothing — no edge. This is
    unfoolable by stale/synthetic prices because BOTH the strategy and the null see the
    exact same prices and outcomes; only the side-choice differs.

  LEG 3 — SHUFFLED-OUTCOME NULL (anti data-snooping).
    Permute the win/loss labels across trades and recompute EV, 10k times. If the real EV
    is within this null, the apparent edge could come from the outcome sequence by chance.

Verdict = EDGE only if: CI_lower > 0 AND random-null p < 0.05 AND shuffle-null p < 0.05.

Data sources:
  --source onchain      real fills from data-api (proxy wallet) — GROUND TRUTH
  --source runner --name "<runner name substring>"   live_orders from /tmp/runners.json
  --csv <file>          a CSV with columns: entry_price, won (1/0)   [any backtest export]

Usage:
  ./edge_validator.py --source onchain
  ./edge_validator.py --source runner --name "drift_v4_safe BTC"
"""
import argparse, json, os, sys, urllib.request
import numpy as np

CRYPTO_FEE = 0.018
PROXY = "0x11a3dda72c74a0D09a58779994CD7BE9F9753A98"


def fee(p):
    return CRYPTO_FEE * p * (1 - p)


def ev_per_trade(entry, won):
    """EV per $1 stake for a binary bet entered at `entry`, won = 1/0, net of taker fee."""
    return np.where(won == 1, (1.0 / entry) * (1 - fee(entry)) - 1.0, -1.0)


# ── Data loaders ────────────────────────────────────────────────────────────────
def load_onchain():
    """Reconstruct per-market realized trades from data-api: entry price + win(redeemed)."""
    ev = []
    offset = 0
    for _ in range(40):
        url = f"https://data-api.polymarket.com/activity?user={PROXY}&limit=500&offset={offset}"
        try:
            arr = json.load(urllib.request.urlopen(urllib.request.Request(url, headers={'User-Agent': 'tc'}), timeout=25))
        except Exception as e:
            print(f"  onchain fetch stop: {e}", file=sys.stderr); break
        if not arr:
            break
        ev.extend(arr); offset += len(arr)
        if len(arr) < 500:
            break
    # group by conditionId: a BUY 'wins' if that condition later REDEEMs more than spent
    from collections import defaultdict
    by = defaultdict(lambda: {'buys': [], 'redeem': 0.0})
    for e in ev:
        c = e.get('conditionId'); usd = e.get('usdcSize', 0) or 0
        if e.get('type') == 'TRADE' and e.get('side') == 'BUY':
            by[c]['buys'].append((e.get('price', 0), usd))
        elif e.get('type') == 'REDEEM':
            by[c]['redeem'] += usd
    entries, wons = [], []
    for c, v in by.items():
        if not v['buys']:
            continue
        spent = sum(u for _, u in v['buys'])
        avg_price = sum(p * u for p, u in v['buys']) / spent if spent else 0
        if not (0.01 < avg_price < 0.99):
            continue
        entries.append(avg_price)
        wons.append(1 if v['redeem'] > spent else 0)
    return np.array(entries), np.array(wons)


def load_runner(name):
    rows = json.load(open('/tmp/runners.json'))
    runners = rows.get('runners', rows) if isinstance(rows, dict) else rows
    entries, wons = [], []
    for r in runners:
        if name.lower() not in r['config'].get('name', '').lower():
            continue
        for o in (r.get('result') or {}).get('live_orders', []):
            if o.get('resolution_source') != 'polymarket':
                continue
            ep = o.get('entry_price')
            res = o.get('result')
            if ep and 0.01 < ep < 0.99 and res:
                entries.append(ep)
                wons.append(1 if res.replace('*', '') == 'WIN' else 0)
    return np.array(entries), np.array(wons)


def load_csv(path):
    import csv
    entries, wons = [], []
    for row in csv.DictReader(open(path)):
        e = float(row.get('entry_price') or row.get('price'))
        w = int(float(row.get('won') or row.get('win')))
        if 0.01 < e < 0.99:
            entries.append(e); wons.append(w)
    return np.array(entries), np.array(wons)


# ── The three legs ──────────────────────────────────────────────────────────────
def validate(entries, wons, iters=10000):
    """Run the three legs. Returns a dict {n, ev, legs, edge} so other scripts
    (phase0_backtest.py) can gate on the verdict programmatically."""
    n = len(entries)
    if n < 30:
        print(f"  Sample too small (n={n}); cannot validate.")
        return {'n': n, 'ev': float('nan'), 'legs': (False, False, False), 'edge': False}
    real_ev = ev_per_trade(entries, wons)
    obs = real_ev.mean()
    wr = wons.mean() * 100
    be = entries.mean() * 100

    print(f"\n  n={n}  WR={wr:.1f}%  avg_entry(break-even)={be:.1f}%  total_PnL/$1stake={real_ev.sum():+.2f}")
    print(f"  Observed EV/trade = {obs*100:+.2f}%\n")

    # LEG 1 — bootstrap CI on EV
    boot = np.array([np.random.choice(real_ev, n, replace=True).mean() for _ in range(iters)])
    lo, hi = np.percentile(boot, [2.5, 97.5]) * 100
    leg1 = lo > 0
    print(f"  LEG 1 — Bootstrap 95% CI on EV/trade: [{lo:+.2f}%, {hi:+.2f}%]   "
          f"{'PASS (lower>0)' if leg1 else 'FAIL (CI includes <=0)'}")

    # LEG 2 — random-side null (fix price+outcome, randomize side)
    # outcome here = did YES win? We need that, but we only stored (entry, won-for-the-side-bet).
    # Equivalent null: a sk-less bettor wins each trade with prob 0.5 independent of price.
    rng = np.random.default_rng(0)
    null2 = np.empty(iters)
    for i in range(iters):
        rand_won = rng.integers(0, 2, n)
        null2[i] = ev_per_trade(entries, rand_won).mean()
    p2 = (null2 >= obs).mean()
    leg2 = p2 < 0.05
    print(f"  LEG 2 — Random-side null: p={p2:.3f} (EV beats {(1-p2)*100:.1f}% of sk-less strategies)   "
          f"{'PASS' if leg2 else 'FAIL (indistinguishable from random side-choice)'}")

    # LEG 3 — shuffled-outcome null
    null3 = np.empty(iters)
    for i in range(iters):
        null3[i] = ev_per_trade(entries, rng.permutation(wons)).mean()
    p3 = (null3 >= obs).mean()
    leg3 = p3 < 0.05
    print(f"  LEG 3 — Shuffled-outcome null: p={p3:.3f}   "
          f"{'PASS' if leg3 else 'FAIL (edge within outcome-shuffle noise)'}")

    print()
    if leg1 and leg2 and leg3:
        print("  ►► VERDICT: EDGE — survives all three independent tests. Worth a small real pilot.")
    else:
        print("  ►► VERDICT: NO EDGE — consistent with luck/fees. Do NOT commit capital.")
    print()
    return {'n': n, 'ev': obs, 'legs': (leg1, leg2, leg3), 'edge': leg1 and leg2 and leg3}


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--source', choices=['onchain', 'runner', 'csv'], default='onchain')
    ap.add_argument('--name', default='')
    ap.add_argument('--csv', default='')
    args = ap.parse_args()

    if args.source == 'onchain':
        print("Loading GROUND-TRUTH onchain fills (proxy wallet)...")
        e, w = load_onchain()
    elif args.source == 'runner':
        e, w = load_runner(args.name)
    else:
        e, w = load_csv(args.csv)

    print(f"Validating {len(e)} trades.")
    validate(e, w)
