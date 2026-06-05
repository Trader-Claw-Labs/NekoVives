#!/usr/bin/env python3
"""
signal_scan.py — Detect whether ANY feature has predictive power BEFORE building models.

This is the gatekeeper. We've been burned by phantom edges, so before training
any classifier we measure, with brutal honesty, whether the features carry signal.

Method (numpy only, no sklearn needed):
  1. CALIBRATION TEST: the market's own p4 IS a probability prediction. Bin p4 into
     deciles and check if realized yes_won matches. If p4=0.65 bins win ~65%, the
     market is well-calibrated → there is NO mispricing to exploit from price alone.
  2. CONDITIONAL EDGE: for each feature, bin it and measure if yes_won deviates from
     the market's p4 within that bin. A feature has edge ONLY if, holding p4 roughly
     constant, the feature still moves the outcome. That's residual signal the market
     hasn't priced.
  3. WALK-FORWARD STABILITY: split data into time-ordered thirds. A real edge appears
     in ALL thirds with the same sign. Edge in one third only = noise/overfit.

Output: a ranked table of features by their *residual* predictive lift, with a
verdict on whether ML modeling is worth pursuing.

Usage:
  ./signal_scan.py --csv /tmp/features_all.csv
  ./signal_scan.py --csv /tmp/features_all.csv --slug btc_5m   # single asset
"""
import argparse, sys
import numpy as np
import pandas as pd


def calibration_test(df):
    """Is the market's p4 a well-calibrated probability?"""
    print("\n" + "=" * 72)
    print("1. CALIBRATION TEST — is the market's p4 already an accurate probability?")
    print("=" * 72)
    bins = np.linspace(0, 1, 11)
    df = df.copy()
    df['p4_bin'] = pd.cut(df['p4'], bins, include_lowest=True)
    print(f"  {'p4 range':<16} {'n':<8} {'predicted':<11} {'actual':<10} {'gap':<8}")
    print("  " + "-" * 60)
    total_gap = 0.0
    total_n = 0
    for interval, g in df.groupby('p4_bin', observed=True):
        if len(g) < 30:
            continue
        pred = g['p4'].mean()
        actual = g['yes_won'].mean()
        gap = actual - pred
        total_gap += abs(gap) * len(g)
        total_n += len(g)
        flag = '  ⚠' if abs(gap) > 0.05 else ''
        print(f"  {str(interval):<16} {len(g):<8} {pred:<11.3f} {actual:<10.3f} {gap:+.3f}{flag}")
    mae = total_gap / total_n if total_n else 0
    print(f"\n  Mean abs calibration error: {mae:.4f}")
    if mae < 0.02:
        print("  → Market is WELL CALIBRATED. Price alone has no exploitable mispricing.")
    elif mae < 0.04:
        print("  → Market is mostly calibrated. Marginal mispricing in some bins.")
    else:
        print("  → Market shows MISCALIBRATION. There may be exploitable bins.")
    return mae


def conditional_edge(df, feature, n_bins=5):
    """Within bins of p4, does `feature` still predict residual outcome?
    Returns max |lift| across feature bins (lift = actual_yes - mean_p4 in bin)."""
    df = df.copy()
    # Control for p4 by bucketing it coarsely
    df['p4_ctrl'] = pd.cut(df['p4'], [0, 0.4, 0.6, 1.0], include_lowest=True)
    try:
        df['feat_bin'] = pd.qcut(df[feature], n_bins, duplicates='drop')
    except Exception:
        return None
    lifts = []
    for (p4c, fb), g in df.groupby(['p4_ctrl', 'feat_bin'], observed=True):
        if len(g) < 50:
            continue
        # Residual: how much does outcome deviate from what price predicted?
        residual = g['yes_won'].mean() - g['p4'].mean()
        lifts.append((p4c, fb, len(g), residual))
    if not lifts:
        return None
    max_lift = max(abs(l[3]) for l in lifts)
    return max_lift, lifts


def walk_forward_sign(df, feature):
    """Does the feature's correlation with residual hold sign across time thirds?"""
    df = df.sort_values('wts').reset_index(drop=True)
    n = len(df)
    thirds = [df.iloc[:n//3], df.iloc[n//3:2*n//3], df.iloc[2*n//3:]]
    residual = df['yes_won'] - df['p4']
    signs = []
    for t in thirds:
        r = t['yes_won'] - t['p4']
        f = t[feature]
        if f.std() < 1e-9 or r.std() < 1e-9:
            signs.append(0)
            continue
        corr = np.corrcoef(f, r)[0, 1]
        signs.append(np.sign(corr) if abs(corr) > 0.01 else 0)
    stable = (signs[0] != 0 and signs[0] == signs[1] == signs[2])
    return signs, stable


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--csv', required=True)
    ap.add_argument('--slug', default=None, help='Filter to one slug')
    args = ap.parse_args()

    df = pd.read_csv(args.csv)
    if args.slug:
        df = df[df['slug'] == args.slug].reset_index(drop=True)
        print(f"Filtered to {args.slug}: {len(df)} rows")
    print(f"Loaded {len(df)} rows, base YES rate {df['yes_won'].mean()*100:.1f}%")

    # 1. Calibration
    mae = calibration_test(df)

    # 2. Conditional edge per feature
    print("\n" + "=" * 72)
    print("2. CONDITIONAL EDGE — residual signal AFTER controlling for market price")
    print("=" * 72)
    print(f"  {'feature':<20} {'max_residual_lift':<20} {'verdict'}")
    print("  " + "-" * 60)
    features = ['drift', 'dist_mid', 'hour_utc', 'dow', 'minute_of_hour',
                'streak_dir', 'streak_len', 'prev_won', 'roll_yes_rate_12', 'p4_vs_roll']
    results = []
    for feat in features:
        if feat not in df.columns:
            continue
        ce = conditional_edge(df, feat)
        if ce is None:
            continue
        max_lift, _ = ce
        verdict = 'SIGNAL' if max_lift > 0.05 else ('weak' if max_lift > 0.03 else 'noise')
        results.append((feat, max_lift, verdict))

    results.sort(key=lambda x: x[1], reverse=True)
    for feat, lift, verdict in results:
        mark = '★' if verdict == 'SIGNAL' else (' ' if verdict == 'weak' else ' ')
        print(f"  {mark}{feat:<19} {lift:<20.4f} {verdict}")

    # 3. Walk-forward stability for the top candidates
    print("\n" + "=" * 72)
    print("3. WALK-FORWARD STABILITY — does the signal hold across time? (anti-overfit)")
    print("=" * 72)
    print(f"  {'feature':<20} {'signs (3 thirds)':<22} {'stable?'}")
    print("  " + "-" * 55)
    stable_features = []
    for feat, lift, verdict in results:
        if verdict == 'noise':
            continue
        signs, stable = walk_forward_sign(df, feat)
        sign_str = ' '.join(f'{int(s):+d}' for s in signs)
        marker = '✓ STABLE' if stable else '✗ unstable'
        if stable:
            stable_features.append(feat)
        print(f"  {feat:<20} [{sign_str:<18}] {marker}")

    # Verdict
    print("\n" + "=" * 72)
    print("VEREDICTO")
    print("=" * 72)
    if mae < 0.02 and not stable_features:
        print("  ✗ El mercado está bien calibrado y NINGUNA feature muestra señal residual")
        print("    estable. Construir un modelo ML NO vale la pena — no hay edge que aprender.")
    elif stable_features:
        print(f"  ✓ Features con señal residual ESTABLE: {', '.join(stable_features)}")
        print("    Vale la pena entrenar un modelo (logistic/GBM) usando SOLO estas features.")
        print("    El edge real estará en los bins donde estas features mueven el residual.")
    else:
        print("  ~ Hay features con lift marginal pero inestable en el tiempo.")
        print("    Riesgo alto de overfit. Si se modela, validación walk-forward estricta.")
    print()
