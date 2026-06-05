#!/usr/bin/env python3
"""
optimize_runner.py — Systematic backtest sweep for runner parameter optimization.

Validates whether parameter changes to a runner (drift_v3_regime, etc.) actually
improve risk-adjusted returns vs the current baseline.  Avoids the "p-hacking
trap" of trying many params and picking whatever looks best on a single window.

Methodology applied:
  1. BASELINE: backtest the runner with its current production params over the
     full data range, splitting into TRAIN (first 70%) and TEST (last 30%).
  2. SWEEP: vary one parameter at a time over a documented grid. Run backtest on
     TRAIN only.  Record return/Sharpe/WR/trades.
  3. SELECT: pick the param value with the best TRAIN Sharpe AND total trades >
     a min-floor (statistical significance).
  4. VERIFY OUT-OF-SAMPLE: re-run the selected config on TEST window. If TEST
     return is within 50% of TRAIN return, accept.  If TEST collapses, reject —
     the "improvement" was overfit.
  5. REPORT: print baseline-vs-optimized table with TRAIN/TEST splits so the
     human can judge.  Never auto-apply changes — this is a diagnostic.

Usage:
  ./optimize_runner.py \
    --script polymarket_btc_updown_5m_drift_v3_regime.rhai \
    --series btc_5m \
    --symbol BTCUSDT \
    --from 2026-04-23 --to 2026-05-25 \
    --param max_spread_pct --grid 0.02,0.03,0.04,0.05,0.06

Requires the gateway running on http://127.0.0.1:42617 and a paired token at
/tmp/bt_token.txt (or pass --token).
"""
import argparse, json, sys, time, urllib.request, urllib.error
from datetime import datetime, timedelta
from typing import Optional

PORT = 42617


def http_post(path: str, body: dict, token: str, timeout: int = 120) -> dict:
    req = urllib.request.Request(
        f"http://127.0.0.1:{PORT}{path}",
        data=json.dumps(body).encode(),
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def split_dates(from_date: str, to_date: str, train_frac: float = 0.7) -> tuple:
    """Returns ((train_from, train_to), (test_from, test_to))."""
    fmt = "%Y-%m-%d"
    f = datetime.strptime(from_date, fmt)
    t = datetime.strptime(to_date, fmt)
    total_days = (t - f).days
    split_days = int(total_days * train_frac)
    split_date = f + timedelta(days=split_days)
    train = (from_date, split_date.strftime(fmt))
    test_start = split_date + timedelta(days=1)
    test = (test_start.strftime(fmt), to_date)
    return train, test


def run_backtest(token: str, market_type: str, symbol: str, series: str,
                 script: str, from_date: str, to_date: str,
                 fee_pct: float = 1.5, sizing_value: float = 5.0,
                 **params) -> dict:
    """One backtest call. params can include max_spread_pct, min_entry_price, etc."""
    body = {
        "market_type": market_type,
        "symbol": symbol,
        "series_id": series,
        "script": script,
        "from_date": from_date,
        "to_date": to_date,
        "initial_balance": 1000,
        "fee_pct": fee_pct,
        "interval": "5m",
        "resolution_logic": "price_up",
        "sizing_mode": "percent",
        "sizing_value": sizing_value,
        "max_position_usd": 500,
    }
    body.update({k: v for k, v in params.items() if v is not None})
    try:
        r = http_post("/api/backtest/run", body, token, timeout=180)
    except (urllib.error.URLError, urllib.error.HTTPError) as e:
        return {"error": str(e)}

    return {
        "trades": r.get("total_trades", 0),
        "wr": round(r.get("win_rate_pct", 0), 1),
        "ret": round(r.get("total_return_pct", 0), 2),
        "sharpe": round(r.get("sharpe_ratio", 0), 2),
        "dd": round(r.get("max_drawdown_pct", 0), 2),
    }


def fmt_row(label: str, m: dict, baseline_ret: Optional[float] = None) -> str:
    if "error" in m:
        return f"  {label:30} ERROR: {m['error'][:50]}"
    delta = ""
    if baseline_ret is not None and abs(baseline_ret) > 0.01:
        diff = m['ret'] - baseline_ret
        delta = f"  Δ={diff:+7.1f}pts"
    return (f"  {label:30}  T={m['trades']:4}  WR={m['wr']:5.1f}%  "
            f"Ret={m['ret']:+8.2f}%  Sharpe={m['sharpe']:+5.2f}  "
            f"DD={m['dd']:5.1f}%{delta}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--script", required=True, help="Script filename (.rhai)")
    ap.add_argument("--series", required=True, help="series_id e.g. btc_5m")
    ap.add_argument("--symbol", default="BTCUSDT")
    ap.add_argument("--market-type", default="polymarket_binary",
                    help="polymarket_binary or archive_candles")
    ap.add_argument("--from", dest="from_date", required=True)
    ap.add_argument("--to", dest="to_date", required=True)
    ap.add_argument("--param", required=True,
                    help="Parameter name to sweep (e.g. max_spread_pct, min_entry_price, sizing_value)")
    ap.add_argument("--grid", required=True,
                    help="Comma-separated values e.g. 0.02,0.03,0.04")
    ap.add_argument("--baseline-params", default="",
                    help="key1=val1,key2=val2 — baseline config (current production)")
    ap.add_argument("--min-trades", type=int, default=100,
                    help="Reject candidates with fewer than this many trades on TRAIN")
    ap.add_argument("--token-file", default="/tmp/bt_token.txt")
    ap.add_argument("--token", default=None)
    ap.add_argument("--train-frac", type=float, default=0.7)
    args = ap.parse_args()

    if args.token:
        token = args.token
    else:
        with open(args.token_file) as f:
            token = f.read().strip()

    grid = [float(v) for v in args.grid.split(",")]

    baseline = {}
    for kv in args.baseline_params.split(","):
        if "=" in kv:
            k, v = kv.split("=", 1)
            try:
                baseline[k.strip()] = float(v)
            except ValueError:
                baseline[k.strip()] = v.strip()

    train, test = split_dates(args.from_date, args.to_date, args.train_frac)
    print(f"\n{'='*78}")
    print(f"OPTIMIZATION SWEEP — {args.script}")
    print(f"{'='*78}")
    print(f"Series:       {args.series}  ({args.market_type})")
    print(f"Full range:   {args.from_date} → {args.to_date}")
    print(f"TRAIN:        {train[0]} → {train[1]}  ({args.train_frac*100:.0f}% of data)")
    print(f"TEST  (OOS):  {test[0]} → {test[1]}")
    print(f"Sweep param:  {args.param}")
    print(f"Grid:         {grid}")
    print(f"Baseline:     {baseline}")
    print(f"Min trades floor: {args.min_trades} (rejects underpowered candidates)")
    print()

    common = dict(token=token, market_type=args.market_type, symbol=args.symbol,
                  series=args.series, script=args.script)

    print("─── PHASE 1: Baseline on TRAIN + TEST ─────────────────────────────")
    bl_train = run_backtest(**common, from_date=train[0], to_date=train[1], **baseline)
    bl_test = run_backtest(**common, from_date=test[0], to_date=test[1], **baseline)
    print(fmt_row("BASELINE TRAIN", bl_train))
    print(fmt_row("BASELINE TEST ", bl_test))

    print()
    print(f"─── PHASE 2: Sweep {args.param} on TRAIN ──────────────────────────")
    candidates = []
    for v in grid:
        params = dict(baseline)
        params[args.param] = v
        m = run_backtest(**common, from_date=train[0], to_date=train[1], **params)
        label = f"{args.param}={v}"
        print(fmt_row(label, m, baseline_ret=bl_train.get("ret")))
        if "error" not in m and m["trades"] >= args.min_trades:
            candidates.append((v, m, params))
        time.sleep(0.5)

    if not candidates:
        print(f"\n⚠ No candidate met the min_trades floor ({args.min_trades}). Sweep failed.")
        sys.exit(1)

    candidates.sort(key=lambda x: x[1]["sharpe"], reverse=True)
    best_v, best_m, best_params = candidates[0]
    print(f"\n─── PHASE 3: Best by TRAIN Sharpe ─────────────────────────────────")
    print(f"  Best on TRAIN: {args.param}={best_v}  Sharpe={best_m['sharpe']}")

    print(f"\n─── PHASE 4: OUT-OF-SAMPLE verify on TEST ─────────────────────────")
    oos = run_backtest(**common, from_date=test[0], to_date=test[1], **best_params)
    print(fmt_row(f"OPT TEST ({args.param}={best_v})", oos))

    print(f"\n─── VERDICT ────────────────────────────────────────────────────────")
    if "error" in oos:
        print("  ✗ OOS run errored. No conclusion.")
        sys.exit(2)

    bl_test_ret = bl_test.get("ret", 0)
    oos_ret = oos.get("ret", 0)
    train_ret = best_m.get("ret", 0)

    delta_vs_baseline = oos_ret - bl_test_ret
    train_test_ratio = (oos_ret / train_ret) if abs(train_ret) > 0.1 else 0

    print(f"  Baseline TEST return:         {bl_test_ret:+.2f}%")
    print(f"  Optimized TEST return:        {oos_ret:+.2f}%")
    print(f"  Δ TEST return:                {delta_vs_baseline:+.2f}pts")
    print(f"  TRAIN return:                 {train_ret:+.2f}%")
    print(f"  TEST/TRAIN ratio:             {train_test_ratio:.2f}  (1.0 = perfect generalization)")
    print()

    if oos.get("trades", 0) < 30:
        print("  ⚠  TEST has too few trades to draw conclusions.")
    elif delta_vs_baseline > 5 and train_test_ratio > 0.4:
        print("  ✓ ACCEPT — optimized config beats baseline OOS, holds up vs TRAIN.")
        print(f"    Recommended: PATCH the runner with {args.param}={best_v}")
    elif delta_vs_baseline > 0 and train_test_ratio > 0.4:
        print("  ~ MARGINAL — small improvement OOS, decision is yours.")
    elif train_test_ratio < 0.3 and abs(train_ret) > 20:
        print("  ✗ REJECT — large TRAIN gain did not transfer to TEST. Likely overfit.")
    else:
        print("  ✗ REJECT — no meaningful OOS improvement.")

    print()


if __name__ == "__main__":
    main()
