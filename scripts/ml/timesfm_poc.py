#!/usr/bin/env python3
"""
timesfm_poc.py — Can a zero-shot time-series foundation model (Google TimesFM 2.5)
detect ANY directional signal in BTC 5m UP/DOWN windows that the market hasn't
already priced in?

WHY THIS IS DIFFERENT FROM THE 59 HAND-CRAFTED STRATEGIES
  TimesFM is FROZEN, pretrained on an external corpus. It never saw this data and we
  fit NOTHING on it. So there is no in-sample overfit (the failure mode that produced
  every prior phantom edge). If TimesFM can't beat the market here, that's a strong,
  overfit-free "no".

LOOKAHEAD DISCIPLINE
  - Exp A (Binance underlying): at each window open W, context = the last 256 CLOSED 1m
    candle closes ending at W (open_time <= W-60). anchor = price at W = close of the
    candle (open_time == W-60). Forecast +5 steps → predicted price at W+300. NEVER reads
    tick `binance_price` (which has the 59s broadcast lookahead).
  - Exp B (Polymarket YES path): decide at second 240 of the window (the live "P4"
    decision minute). context = within-window yes_mid 1Hz [0..240]. forecast 60 steps.
  - Label = official `window_yes_won`. Windows without it are dropped.

OUTPUT
  - Directional accuracy of TimesFM vs baselines (coin / momentum / market).
  - The decisive comparison: does TimesFM beat the MARKET's own call on the same windows?
  - Saves per-window CSV (entry_price, won) for the disagreement bets, ready for
    edge_validator.py (Stage 3, only if Stage 1/2 show skill).
"""
import os, sys, glob, json, bisect, time
import numpy as np

WS = os.path.expanduser("~/.traderclaw/workspace")
TICKS = f"{WS}/data/ticks/btc_5m"
# clean 1m candle file(s) covering the tick range (04-23 .. 05-25); this one spans 04-04..06-04
BINANCE_FILES = sorted(glob.glob(f"{WS}/data/BTCUSDT_1m_2026-04-04_2026-06-04.json"))
CACHE = "/tmp/timesfm_poc_dataset.npz"
CTX_A = 256          # Binance context candles
HZN_A = 5            # forecast 5 minutes
DECIDE_B = 240       # second within window to decide for Exp B
HZN_B = 60           # forecast 60s to window end
WINDOW = 300


# ── 1. Load Binance 1m closes → open_time_sec -> close ───────────────────────────
def load_binance():
    ot, cl = [], []
    seen = set()
    for fp in BINANCE_FILES:
        for c in json.load(open(fp)):
            t = c["open_time_ms"] // 1000
            if t in seen:
                continue
            seen.add(t)
            ot.append(t); cl.append(float(c["close"]))
    order = np.argsort(ot)
    ot = np.array(ot)[order]; cl = np.array(cl)[order]
    # report gaps
    d = np.diff(ot)
    gaps = int((d != 60).sum())
    print(f"  binance: {len(ot)} candles, {gaps} non-60s steps "
          f"({(ot[-1]-ot[0])/86400:.1f}d span)")
    return ot, cl


# ── 2. Load ticks → per-window features ──────────────────────────────────────────
def load_windows():
    """Return list of dicts: window_ts, yes_won, open_yes_ask/mid, dec_yes_mid (@240),
    and yes_mid 1Hz path within window."""
    wins = {}
    for fp in sorted(glob.glob(f"{TICKS}/*.jsonl")):
        for line in open(fp):
            r = json.loads(line)
            w = r["window_ts"]
            sec = WINDOW - int(r.get("window_secs_left", 0))   # 0..300 elapsed
            d = wins.setdefault(w, {"w": w, "won": None, "path": {}})
            if r.get("window_yes_won") is not None:
                d["won"] = bool(r["window_yes_won"])
            ym = r.get("yes_mid")
            ya = r.get("yes_ask")
            if 0 <= sec <= WINDOW and ym is not None:
                d["path"][sec] = (ym, ya)
    out = []
    for w, d in wins.items():
        if d["won"] is None:
            continue
        path = d["path"]
        # first available tick (window open) and the @240 decision tick
        if not path:
            continue
        s0 = min(path)
        open_ym, open_ya = path[s0]
        # decision @ DECIDE_B: nearest tick at or before 240
        dec_secs = [s for s in path if s <= DECIDE_B]
        if not dec_secs:
            continue
        sd = max(dec_secs)
        dec_ym, dec_ya = path[sd]
        out.append({
            "w": w, "won": int(d["won"]),
            "open_ym": open_ym, "open_ya": open_ya, "open_sec": s0,
            "dec_ym": dec_ym, "dec_ya": dec_ya, "dec_sec": sd,
            "path": path,
        })
    out.sort(key=lambda x: x["w"])
    print(f"  windows: {len(out)} official-resolved with usable path")
    return out


def build_dataset():
    if os.path.exists(CACHE):
        print("  (using cached dataset)")
        z = np.load(CACHE, allow_pickle=True)
        return z["A_ctx"], z["A_anchor"], z["B_ctx"], z["meta"]
    ot, cl = load_binance()
    wins = load_windows()

    A_ctx, A_anchor, B_ctx, meta = [], [], [], []
    for d in wins:
        W = d["w"]
        # anchor = price at W = close of candle open_time == W-60
        ia = bisect.bisect_left(ot, W - 60)
        if not (ia < len(ot) and ot[ia] == W - 60):
            continue
        anchor = cl[ia]
        # context = CTX_A closes ending at that candle (inclusive), all open_time <= W-60
        lo = ia - CTX_A + 1
        if lo < 0:
            continue
        ctx = cl[lo:ia + 1]
        if len(ctx) != CTX_A or not np.all(np.isfinite(ctx)):
            continue
        # Exp B context: within-window yes_mid 1Hz from open..DECIDE_B
        path = d["path"]
        secs = sorted(s for s in path if s <= DECIDE_B)
        b = np.array([path[s][0] for s in secs], dtype="float32")
        if len(b) < 30:        # need a minimally populated path
            continue
        A_ctx.append(ctx.astype("float32"))
        A_anchor.append(anchor)
        B_ctx.append(b)
        meta.append({"w": W, "won": d["won"],
                     "open_ya": d["open_ya"], "open_ym": d["open_ym"],
                     "dec_ya": d["dec_ya"], "dec_ym": d["dec_ym"]})
    A_ctx = np.array(A_ctx, dtype="float32")
    A_anchor = np.array(A_anchor, dtype="float32")
    B_ctx = np.array(B_ctx, dtype=object)
    meta = np.array(meta, dtype=object)
    np.savez(CACHE, A_ctx=A_ctx, A_anchor=A_anchor, B_ctx=B_ctx, meta=meta)
    print(f"  built dataset: {len(A_ctx)} windows  (cached → {CACHE})")
    return A_ctx, A_anchor, B_ctx, meta


# ── 3. TimesFM forecasting ───────────────────────────────────────────────────────
def p_up_from_quantiles(qrow, anchor):
    """qrow = 10 values [mean, q0.1..q0.9]. Return P(value > anchor) via CDF interp."""
    deciles = qrow[1:10]               # q0.1 .. q0.9 (monotone)
    cdf_x = deciles
    cdf_y = np.arange(1, 10) / 10.0    # 0.1 .. 0.9
    if anchor <= cdf_x[0]:
        cdf = 0.05
    elif anchor >= cdf_x[-1]:
        cdf = 0.95
    else:
        cdf = float(np.interp(anchor, cdf_x, cdf_y))
    return 1.0 - cdf


def run_timesfm(A_ctx, A_anchor, B_ctx):
    import timesfm
    t0 = time.time()
    m = timesfm.TimesFM_2p5_200M_torch.from_pretrained("google/timesfm-2.5-200m-pytorch")
    m.compile(timesfm.ForecastConfig(
        max_context=CTX_A, max_horizon=max(HZN_A, HZN_B),
        normalize_inputs=True, use_continuous_quantile_head=True,
        fix_quantile_crossing=True))
    print(f"  model ready in {time.time()-t0:.1f}s")

    def forecast_batched(inputs, horizon, label):
        pts, qs = [], []
        BS = 256
        t1 = time.time()
        for i in range(0, len(inputs), BS):
            chunk = [np.asarray(x, dtype="float32") for x in inputs[i:i + BS]]
            pt, q = m.forecast(horizon=horizon, inputs=chunk)
            pts.append(np.asarray(pt)); qs.append(np.asarray(q))
            print(f"    {label}: {min(i+BS,len(inputs))}/{len(inputs)} "
                  f"({time.time()-t1:.0f}s)", end="\r")
        print()
        return np.concatenate(pts), np.concatenate(qs)

    # Exp A: Binance underlying, full 5-step horizon
    ptA, qA = forecast_batched(list(A_ctx), HZN_A, "A/binance")
    A_predclose = ptA[:, HZN_A - 1]
    A_pup = np.array([p_up_from_quantiles(qA[i, HZN_A - 1], A_anchor[i])
                      for i in range(len(A_anchor))])
    A_dir = (A_predclose > A_anchor).astype(int)

    # Exp B: Polymarket YES path
    ptB, qB = forecast_batched(list(B_ctx), HZN_B, "B/yes_path")
    B_predend = ptB[:, HZN_B - 1]
    B_dir = (B_predend > 0.5).astype(int)

    return A_dir, A_pup, A_predclose, B_dir, B_predend


# ── 4. Evaluation ────────────────────────────────────────────────────────────────
def report(meta, A_dir, A_pup, B_dir):
    won = np.array([m["won"] for m in meta])
    dec_ym = np.array([m["dec_ym"] for m in meta])      # market call @240
    open_ym = np.array([m["open_ym"] for m in meta])    # market call @open
    n = len(won)
    base_rate = won.mean()

    def acc(pred):
        return (pred == won).mean()

    print("\n" + "=" * 64)
    print(f"  N = {n} windows   |   base rate P(UP won) = {base_rate*100:.1f}%")
    print("=" * 64)

    # Baselines
    coin = np.full(n, 1)                      # always UP (majority-ish)
    market_open = (open_ym > 0.5).astype(int) # market's call at window open
    market_dec = (dec_ym > 0.5).astype(int)   # market's call at second 240

    print("\n  DIRECTIONAL ACCURACY (predict UP/DOWN, compare to official outcome)")
    print(f"    always-UP            : {acc(coin)*100:5.1f}%")
    print(f"    market @ open  (>0.5): {acc(market_open)*100:5.1f}%   <- the price you'd pay")
    print(f"    market @ 240s  (>0.5): {acc(market_dec)*100:5.1f}%   <- market after 4min")
    print(f"    TimesFM A (binance)  : {acc(A_dir)*100:5.1f}%   <- pre-window 5m forecast")
    print(f"    TimesFM B (yes path) : {acc(B_dir)*100:5.1f}%   <- @240s forecast to close")

    # The decisive test: when TimesFM disagrees with the market @open, who's right?
    disagree = A_dir != market_open
    nd = disagree.sum()
    if nd:
        tf_right = (A_dir[disagree] == won[disagree]).mean()
        print(f"\n  HEAD-TO-HEAD (TimesFM A vs market @open): {nd} disagreements "
              f"({100*nd/n:.0f}% of windows)")
        print(f"    when they disagree, TimesFM is right {tf_right*100:.1f}% of the time")
        print(f"    (>52-53% net of the ~1.5% fee would be the bar for a real edge)")

    # Calibration of TimesFM's P_up (does p_up=0.6 win ~60%?)
    print("\n  TimesFM A P(UP) calibration (does forecast prob match realized?)")
    bins = [0, .35, .45, .5, .55, .65, 1.01]
    for a, b in zip(bins, bins[1:]):
        mask = (A_pup >= a) & (A_pup < b)
        if mask.sum() >= 20:
            print(f"    p_up in [{a:.2f},{b:.2f}): n={mask.sum():4d}  realized UP={won[mask].mean()*100:5.1f}%")

    # Save disagreement bets for Stage 3 edge_validator
    # Bet the side TimesFM picks, at the market price you'd pay at open.
    import csv
    out = "/tmp/timesfm_poc_bets.csv"
    with open(out, "w", newline="") as f:
        wr = csv.writer(f); wr.writerow(["entry_price", "won"])
        rows = 0
        for i in range(n):
            side_yes = A_dir[i] == 1
            entry = open_ym[i] if side_yes else (1 - open_ym[i])
            bet_won = 1 if (side_yes == bool(won[i])) else 0
            if 0.02 < entry < 0.98:
                wr.writerow([round(float(entry), 4), bet_won]); rows += 1
    print(f"\n  wrote {rows} TimesFM-side bets → {out}")
    print(f"  (Stage 3: scripts/ml/edge_validator.py --source csv --csv {out})")


if __name__ == "__main__":
    print("[1/3] building lookahead-safe dataset ...")
    A_ctx, A_anchor, B_ctx, meta = build_dataset()
    print(f"[2/3] running TimesFM zero-shot forecasts ({len(meta)} windows) ...")
    A_dir, A_pup, A_predclose, B_dir, B_predend = run_timesfm(A_ctx, A_anchor, B_ctx)
    print("[3/3] evaluating ...")
    report(meta, A_dir, A_pup, B_dir)
