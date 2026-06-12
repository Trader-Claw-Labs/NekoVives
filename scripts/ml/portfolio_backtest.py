#!/usr/bin/env python3
"""
portfolio_backtest.py — Web3 Portfolio Manager P1 backtest (Feature 1).

Validates the EVIDENCE-BASED version of the portfolio manager, NOT the original
"trade every second" idea (which dies on costs). Tests three components together:

  1. REGIME DETECTOR — only hold risk when BTC > its 200d SMA (the "bull run" gate).
     In bear/neutral, rotate to cash (USDC). This is the core protection.
  2. MOMENTUM ROTATION — among majors, overweight the top-N by 12-week momentum.
  3. THRESHOLD REBALANCE — only rebalance when a weight drifts >15% from target
     (the empirically optimal band; frequent rebalancing loses to costs).

Every trade pays a realistic round-trip cost (fee + slippage). Compares against
two baselines: HODL BTC, and equal-weight HODL. Reports CAGR, max drawdown, Sharpe,
and — critically — net-of-cost return + number of trades.

No survivorship bias risk here (majors don't disappear), unlike the memecoin feature.

Usage:
  ./portfolio_backtest.py                       # default majors, 3y daily
  ./portfolio_backtest.py --cost-bps 30 --rebal-threshold 0.15
"""
import argparse, json, sys, time, urllib.request
from datetime import datetime, timezone
import numpy as np

BINANCE = "https://api.binance.com/api/v3/klines"
# Majors with long, liquid history. Memecoins excluded (survivorship bias + Feature 2).
DEFAULT_ASSETS = ["BTCUSDT", "ETHUSDT", "SOLUSDT", "BNBUSDT", "XRPUSDT", "ADAUSDT", "AVAXUSDT", "LINKUSDT"]


def fetch_daily(symbol, days):
    """Daily closes for `symbol`, oldest→newest. Paginates 1000/req."""
    end = int(time.time() * 1000)
    start = end - days * 86400_000
    closes, times = [], []
    cur = start
    while cur < end:
        url = f"{BINANCE}?symbol={symbol}&interval=1d&startTime={cur}&endTime={end}&limit=1000"
        try:
            rows = json.load(urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "tc"}), timeout=20))
        except Exception as e:
            print(f"  [{symbol}] fetch error: {e}", file=sys.stderr)
            break
        if not rows:
            break
        for r in rows:
            times.append(r[0]); closes.append(float(r[4]))
        cur = rows[-1][0] + 86400_000
        if len(rows) < 1000:
            break
    return np.array(times), np.array(closes)


def align(series):
    """Align multiple (times, closes) to a common daily timeline (intersection)."""
    common = None
    for t, _ in series.values():
        s = set(t.tolist())
        common = s if common is None else (common & s)
    common = sorted(common)
    out = {}
    for sym, (t, c) in series.items():
        idx = {ts: i for i, ts in enumerate(t.tolist())}
        out[sym] = np.array([c[idx[ts]] for ts in common])
    return np.array(common), out


def backtest(assets, days, cost_bps, rebal_threshold, top_n, mom_days, regime_sma, simple_mode=False):
    print(f"Fetching {len(assets)} assets, {days}d daily…", file=sys.stderr)
    raw = {}
    for a in assets:
        t, c = fetch_daily(a, days)
        if len(c) > regime_sma + mom_days:
            raw[a] = (t, c)
        else:
            print(f"  [{a}] insufficient history ({len(c)}d), dropping", file=sys.stderr)
    if "BTCUSDT" not in raw:
        print("BTCUSDT required for regime filter."); return
    times, px = align(raw)
    syms = list(px.keys())
    n = len(times)
    cost = cost_bps / 10000.0

    btc = px["BTCUSDT"]
    start_i = max(regime_sma, mom_days) + 1

    # Strategy state
    weights = {s: 0.0 for s in syms}  # current portfolio weights (fraction of equity)
    cash = 1.0
    equity = 1.0
    strat_curve, hodl_curve, eq_curve = [], [], []
    trades = 0
    days_in_market = 0

    btc0 = btc[start_i]
    eqw_assets = syms
    eqw0 = {s: px[s][start_i] for s in eqw_assets}

    for i in range(start_i, n):
        # ── Regime: is BTC above its SMA? ──
        sma = btc[i - regime_sma:i].mean()
        bull = btc[i] > sma

        # ── Target weights ──
        if simple_mode:
            # "Regime-protected HODL": 100% BTC in bull, 100% cash in bear. No alt rotation.
            target = {s: (1.0 if s == "BTCUSDT" and bull else 0.0) for s in syms}
        elif not bull:
            target = {s: 0.0 for s in syms}  # all cash in bear/neutral
        else:
            # momentum = price now / price mom_days ago - 1
            mom = {s: px[s][i] / px[s][i - mom_days] - 1.0 for s in syms}
            ranked = sorted(syms, key=lambda s: mom[s], reverse=True)
            winners = [s for s in ranked[:top_n] if mom[s] > 0]  # only positive momentum
            target = {s: (1.0 / len(winners) if s in winners else 0.0) for s in syms} if winners else {s: 0.0 for s in syms}

        # ── Mark-to-market current weights with today's returns ──
        if i > start_i:
            for s in syms:
                if weights[s] > 0:
                    r = px[s][i] / px[s][i - 1] - 1.0
                    weights[s] *= (1.0 + r)
            tot = sum(weights.values()) + cash
            # normalize back to fractions of equity
            if tot > 0:
                for s in syms:
                    weights[s] /= tot
                cash /= tot
                equity *= tot

        # ── Rebalance only if any weight drifts > threshold from target ──
        cur_invested = sum(weights.values())
        drift = max(abs(weights[s] - target[s]) for s in syms) if syms else 0.0
        regime_flip = (cur_invested > 0.01) != (sum(target.values()) > 0.01)
        if drift > rebal_threshold or regime_flip:
            # compute turnover and apply cost
            turnover = sum(abs(target[s] - weights[s]) for s in syms)
            equity *= (1.0 - turnover * cost)
            trades += sum(1 for s in syms if abs(target[s] - weights[s]) > 0.01)
            weights = dict(target)
            cash = 1.0 - sum(weights.values())

        if sum(weights.values()) > 0.01:
            days_in_market += 1

        strat_curve.append(equity)
        hodl_curve.append(btc[i] / btc0)
        eq_curve.append(np.mean([px[s][i] / eqw0[s] for s in eqw_assets]))

    def stats(curve):
        c = np.array(curve)
        yrs = len(c) / 365.0
        cagr = (c[-1] ** (1 / yrs) - 1) * 100 if yrs > 0 and c[-1] > 0 else 0
        rets = np.diff(c) / c[:-1]
        sharpe = (rets.mean() / rets.std() * np.sqrt(365)) if rets.std() > 0 else 0
        peak = np.maximum.accumulate(c)
        mdd = ((c - peak) / peak).min() * 100
        return cagr, sharpe, mdd, (c[-1] - 1) * 100

    print(f"\n{'='*72}")
    print(f"PORTFOLIO BACKTEST — {len(syms)} assets, {len(strat_curve)} days, cost {cost_bps}bps/trade")
    print(f"  regime: BTC>{regime_sma}d SMA | momentum: {mom_days}d, top-{top_n} | rebal threshold: {rebal_threshold*100:.0f}%")
    print(f"{'='*72}")
    print(f"{'strategy':24} {'total':>9} {'CAGR':>8} {'Sharpe':>7} {'maxDD':>8}")
    for name, curve in [("Regime+Momentum (net)", strat_curve), ("HODL BTC", hodl_curve), ("Equal-weight HODL", eq_curve)]:
        cagr, sh, mdd, tot = stats(curve)
        print(f"{name:24} {tot:>+8.0f}% {cagr:>+7.1f}% {sh:>7.2f} {mdd:>7.1f}%")
    print(f"\n  Trades: {trades} | Days in market: {days_in_market}/{len(strat_curve)} ({days_in_market/len(strat_curve)*100:.0f}%)")
    print(f"  → Verdict: the strategy is worthwhile only if it beats HODL BTC on a")
    print(f"     RISK-ADJUSTED basis (higher Sharpe / shallower maxDD), since the regime")
    print(f"     gate's whole point is protecting the downside, not maximizing raw return.")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--days", type=int, default=1100)
    ap.add_argument("--cost-bps", type=float, default=30)  # 0.3% round-trip (Uniswap-like)
    ap.add_argument("--rebal-threshold", type=float, default=0.15)
    ap.add_argument("--top-n", type=int, default=3)
    ap.add_argument("--mom-days", type=int, default=84)  # 12 weeks
    ap.add_argument("--regime-sma", type=int, default=200)
    ap.add_argument("--simple", action="store_true", help="regime-protected HODL BTC (no alt rotation)")
    args = ap.parse_args()
    backtest(DEFAULT_ASSETS, args.days, args.cost_bps, args.rebal_threshold,
             args.top_n, args.mom_days, args.regime_sma, args.simple)
