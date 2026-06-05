#!/usr/bin/env python3
"""
extract_features.py — Build a feature matrix from on_candle P3/P4 datasets for ML.

Reads polymarket_historical/<slug>.jsonl (P4 = decision price, has resolution)
and min3_<slug>.jsonl (P3 = price 60s earlier) and produces a CSV with engineered
features + the binary label (yes_won).

Features (all known AT decision time — no lookahead):
  - p4              : YES token price at decision (the market's implied P(up))
  - p3             : YES token price 60s earlier
  - drift          : p4 - p3  (short-term token momentum)
  - dist_mid       : |p4 - 0.50|  (how far from coin-flip the market is)
  - hour_utc       : 0-23
  - dow            : day of week 0-6
  - minute_of_hour : 0-55 (5-min bucket)
  - streak_dir     : direction of prior consecutive same-direction windows (-1/0/+1)
  - streak_len     : length of that streak (0-N, capped 10)
  - prev_won       : did the previous window resolve YES? (1/0)
  - prev2_won      : window before that
  - roll_yes_rate_12 : fraction of last 12 windows that resolved YES (regime)
  - p4_vs_roll     : p4 minus the rolling YES rate (divergence of market vs recent base rate)

Label:
  - yes_won : 1 if resolution == "up", 0 if "down". Rows without resolution dropped.

Usage:
  ./extract_features.py --slug btc_5m --out /tmp/features_btc_5m.csv
  ./extract_features.py --slug btc_5m,eth_5m,sol_5m --out /tmp/features_multi.csv
"""
import argparse, json, os, sys
import numpy as np
import pandas as pd
from datetime import datetime, timezone

HIST_DIR = os.path.expanduser('~/.traderclaw/workspace/data/polymarket_historical')


def load_slug(slug):
    """Returns dict {window_open_ts: {p4, p3, yes_won}} sorted by ts."""
    p4_file = f'{HIST_DIR}/{slug}.jsonl'
    p3_file = f'{HIST_DIR}/min3_{slug}.jsonl'
    if not os.path.exists(p4_file):
        print(f"  [{slug}] no P4 file, skipping", file=sys.stderr)
        return {}

    p4 = {}
    with open(p4_file) as f:
        for line in f:
            try:
                d = json.loads(line)
                wts = d.get('window_open_ts')
                res = d.get('resolution')
                yp = d.get('yes_token_price')
                if wts is None or yp is None:
                    continue
                yes_won = 1 if res == 'up' else (0 if res == 'down' else None)
                p4[int(wts)] = {'p4': float(yp), 'yes_won': yes_won}
            except Exception:
                pass

    # P3 (earlier price)
    if os.path.exists(p3_file):
        with open(p3_file) as f:
            for line in f:
                try:
                    d = json.loads(line)
                    wts = d.get('window_open_ts')
                    yp = d.get('yes_token_price')
                    if wts is not None and yp is not None and int(wts) in p4:
                        p4[int(wts)]['p3'] = float(yp)
                except Exception:
                    pass
    return p4


def build_features(slug):
    data = load_slug(slug)
    if not data:
        return pd.DataFrame()

    rows = []
    sorted_ts = sorted(data.keys())

    # Rolling history for streak / base-rate features
    recent_outcomes = []  # list of yes_won (1/0), most recent last

    for i, wts in enumerate(sorted_ts):
        rec = data[wts]
        yes_won = rec.get('yes_won')
        if yes_won is None:
            # Can't use as a label row, but still update history if we know direction
            continue

        p4 = rec['p4']
        p3 = rec.get('p3', p4)  # fall back to p4 if no P3 (drift=0)
        drift = p4 - p3
        dt = datetime.fromtimestamp(wts, tz=timezone.utc)

        # Streak from recent_outcomes (only PRIOR windows — no lookahead)
        streak_dir = 0
        streak_len = 0
        if recent_outcomes:
            last = recent_outcomes[-1]
            streak_dir = 1 if last == 1 else -1
            for o in reversed(recent_outcomes):
                if o == last:
                    streak_len += 1
                else:
                    break
        streak_len = min(streak_len, 10)

        prev_won = recent_outcomes[-1] if len(recent_outcomes) >= 1 else 0
        prev2_won = recent_outcomes[-2] if len(recent_outcomes) >= 2 else 0
        roll12 = recent_outcomes[-12:]
        roll_yes_rate_12 = (sum(roll12) / len(roll12)) if roll12 else 0.5

        rows.append({
            'slug': slug,
            'wts': wts,
            'p4': p4,
            'p3': p3,
            'drift': drift,
            'dist_mid': abs(p4 - 0.50),
            'hour_utc': dt.hour,
            'dow': dt.weekday(),
            'minute_of_hour': dt.minute,
            'streak_dir': streak_dir,
            'streak_len': streak_len,
            'prev_won': prev_won,
            'prev2_won': prev2_won,
            'roll_yes_rate_12': roll_yes_rate_12,
            'p4_vs_roll': p4 - roll_yes_rate_12,
            'yes_won': yes_won,
        })

        recent_outcomes.append(yes_won)
        if len(recent_outcomes) > 50:
            recent_outcomes.pop(0)

    return pd.DataFrame(rows)


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--slug', required=True, help='Comma-separated slugs (btc_5m,eth_5m)')
    ap.add_argument('--out', required=True)
    args = ap.parse_args()

    slugs = [s.strip() for s in args.slug.split(',')]
    dfs = []
    for s in slugs:
        df = build_features(s)
        print(f"  [{s}] {len(df)} labeled windows", file=sys.stderr)
        dfs.append(df)

    full = pd.concat(dfs, ignore_index=True) if dfs else pd.DataFrame()
    full.to_csv(args.out, index=False)
    print(f"Wrote {len(full)} rows × {len(full.columns)} cols → {args.out}", file=sys.stderr)
    # Quick sanity
    if len(full):
        print(f"  Base YES rate: {full['yes_won'].mean()*100:.1f}%", file=sys.stderr)
