"""Train a CALIBRATED P(Up at close) model with honest, time-ordered validation.

Why calibration, not accuracy: in a binary market you bet on probabilities, so a
model that says 0.62 must be right ~62% of the time. We measure Brier score and a
reliability table, and we split chronologically (walk-forward) by window so the
test set is strictly in the future relative to training — no leakage.

Prefers LightGBM; falls back to sklearn HistGradientBoosting if not installed.
"""

from __future__ import annotations

import argparse

import joblib
import numpy as np
import pandas as pd
from sklearn.calibration import CalibratedClassifierCV
from sklearn.metrics import brier_score_loss, log_loss

from features import FEATURE_ORDER, to_matrix
from label import attach_labels


def make_estimator():
    try:
        from lightgbm import LGBMClassifier

        return LGBMClassifier(
            n_estimators=300, num_leaves=31, learning_rate=0.05,
            subsample=0.8, colsample_bytree=0.8, min_child_samples=50,
        ), "lightgbm"
    except Exception:
        from sklearn.ensemble import HistGradientBoostingClassifier

        return HistGradientBoostingClassifier(
            max_iter=300, learning_rate=0.05, max_leaf_nodes=31,
            min_samples_leaf=50, l2_regularization=1.0,
        ), "sklearn-hgb"


def reliability_table(y, p, bins=10):
    edges = np.linspace(0, 1, bins + 1)
    idx = np.clip(np.digitize(p, edges) - 1, 0, bins - 1)
    rows = []
    for b in range(bins):
        m = idx == b
        if m.sum() == 0:
            continue
        rows.append((f"[{edges[b]:.1f},{edges[b+1]:.1f})", int(m.sum()),
                     float(p[m].mean()), float(y[m].mean())))
    return pd.DataFrame(rows, columns=["bucket", "n", "pred_mean", "obs_freq"])


def walk_forward(df, train_frac=0.6, calib_frac=0.2):
    """Chronological split by window_id (windows are time-ordered)."""
    wids = np.sort(df["window_id"].unique())
    n = len(wids)
    tr = wids[: int(n * train_frac)]
    ca = wids[int(n * train_frac): int(n * (train_frac + calib_frac))]
    te = wids[int(n * (train_frac + calib_frac)):]
    sel = lambda ws: df[df["window_id"].isin(ws)]
    return sel(tr), sel(ca), sel(te)


def main(a):
    samples = pd.read_parquet(a.samples)
    resolutions = pd.read_parquet(a.resolutions)
    df = attach_labels(samples, resolutions)

    train, calib, test = walk_forward(df)
    Xtr, ytr = to_matrix(train), train["y"].to_numpy()
    Xca, yca = to_matrix(calib), calib["y"].to_numpy()
    Xte, yte = to_matrix(test), test["y"].to_numpy()

    base, kind = make_estimator()
    base.fit(Xtr, ytr)

    # Isotonic calibration on the (future-of-train) calibration slice.
    # sklearn >=1.6 removed cv="prefit" in favor of FrozenEstimator; support both.
    try:
        from sklearn.frozen import FrozenEstimator

        cal = CalibratedClassifierCV(FrozenEstimator(base), method="isotonic")
    except ImportError:
        cal = CalibratedClassifierCV(base, method="isotonic", cv="prefit")
    cal.fit(Xca, yca)

    p_te = cal.predict_proba(Xte)[:, 1]
    brier = brier_score_loss(yte, p_te)
    ll = log_loss(yte, np.clip(p_te, 1e-6, 1 - 1e-6))
    # Baseline Brier of always predicting the base rate:
    base_rate = ytr.mean()
    brier_base = brier_score_loss(yte, np.full_like(p_te, base_rate))

    print(f"estimator           : {kind}")
    print(f"test windows        : {test['window_id'].nunique()}  rows: {len(test)}")
    print(f"base rate (train)   : {base_rate:.4f}")
    print(f"Brier (model)       : {brier:.4f}")
    print(f"Brier (base rate)   : {brier_base:.4f}   (lower is better)")
    print(f"LogLoss (model)     : {ll:.4f}")
    print(f"Brier skill score   : {1 - brier / brier_base:.4f}   (>0 means useful)")

    # Directional accuracy in the last bucket (secs_left <= 15), where we trade.
    late = test["secs_left"] <= 15
    if late.sum():
        acc = ((p_te[late.to_numpy()] >= 0.5).astype(int) == yte[late.to_numpy()]).mean()
        print(f"dir. acc (<=15s)    : {acc:.4f}   (n={int(late.sum())})")

    print("\nreliability (test):")
    print(reliability_table(yte, p_te).to_string(index=False))

    joblib.dump({"model": cal, "features": FEATURE_ORDER, "kind": kind}, a.out)
    print(f"\nsaved calibrated model -> {a.out}")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--samples", default="samples.parquet")
    ap.add_argument("--resolutions", default="resolutions.parquet")
    ap.add_argument("--out", default="model.joblib")
    main(ap.parse_args())
