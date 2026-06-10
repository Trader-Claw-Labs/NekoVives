#!/usr/bin/env python3
"""
fear_index.py — Prediction Market Fear Index: speed-of-repricing across clusters.

Two modes:
  --collect  : polls CLOB book mid every POLL_S seconds, writes to JSONL log.
               Run this 24/7 as a background process (nohup).
  --backtest : reads the log, computes per-sample index, backtests whether
               "panic spikes → overshoot → revert" actually holds.
               Outputs verdict: defensve gate OR fade-entry signal.

The index for a cluster at time t:
  volatility_score(m, t) = |mid(t) - mid(t-60s)| / mid(t)  (60s velocity)
  cluster_index(t) = mean of top-N market velocity_scores
  z_score(t) = (cluster_index(t) - rolling_mean) / rolling_std  [window = 2h]

A spike = z_score > SPIKE_Z.

Backtest question: after a spike at t, does the market that moved the most
revert toward its pre-spike mid over the next T minutes?
  reversion_pct(m, t, horizon) = (mid(t+horizon) - mid(t)) / (mid(t) - mid(t-60s))
  → negative value = reverted (the fade worked)
  → +1.0 = continued in the same direction (no reversion, trend)

Usage:
  python3 fear_index.py --collect --cluster btc_updown --hours 24
  python3 fear_index.py --backtest --log ~/.traderclaw/workspace/data/fear_index.jsonl
"""
import argparse, json, os, time, datetime as dt, collections
import urllib.request
import statistics as stat

CLOB = "https://clob.polymarket.com"
LOG_DEFAULT = os.path.expanduser("~/.traderclaw/workspace/data/fear_index.jsonl")
POLL_S = 30          # book poll cadence
Z_WINDOW_S = 7200    # 2h rolling window for z-score
SPIKE_Z = 2.0        # z > this = fear spike
UA = {"User-Agent": "trader-claw/fear-index"}

# ── Cluster definitions (YES token_ids of representative slow markets) ──────
# These are long-dated politics/finance markets — slow fair value, so velocity
# spikes here are genuine crowd-panic moments, not HFT repricing.
CLUSTERS = {
    "politics": [
        # Fed rate cut cluster (5 YES tokens = 5 outcomes of the same event)
        "12403602920039269077597917340921667997547115084613238528792639013246536343316",  # no cuts
        "75028752776148090296091099469912621384650554615761384992997579209329182670110",   # hike
        "113379839734351069617987084078322474966003108854908079701423911002443710490196",  # 1 cut
        "72535544017897924722695722172278828562733090748474862987195303914909938482758",   # 2 cuts
        "73197441127256680134600821323583356037261213281680365433623681075249556019477",   # 4 cuts
    ],
}


def http(url):
    try:
        return json.load(urllib.request.urlopen(
            urllib.request.Request(url, headers=UA), timeout=15))
    except Exception:
        return None


def get_mid(token_id):
    b = http(f"{CLOB}/book?token_id={token_id}")
    if not b:
        return None
    bids, asks = b.get("bids", []), b.get("asks", [])
    if not bids or not asks:
        return None
    try:
        bb = float(bids[-1]["price"]); ba = float(asks[-1]["price"])
        return (bb + ba) / 2 if ba > bb else None
    except Exception:
        return None


def collect(cluster_name, hours, log_path):
    tokens = CLUSTERS.get(cluster_name, [])
    if not tokens:
        print(f"Unknown cluster '{cluster_name}'. Available: {list(CLUSTERS)}")
        return
    os.makedirs(os.path.dirname(log_path), exist_ok=True)
    print(f"Collecting {cluster_name} ({len(tokens)} markets) for {hours}h → {log_path}")
    end = time.time() + hours * 3600
    # prev_mids: token_id → (ts, mid) of last known price
    prev = {}
    while time.time() < end:
        now = time.time()
        vels = []
        mids = {}
        for tok in tokens:
            mid = get_mid(tok)
            if mid is None:
                continue
            mids[tok] = mid
            if tok in prev:
                pt, pm = prev[tok]
                age = now - pt
                if 10 < age < 300:  # only use if < 5min old
                    vel = abs(mid - pm) / max(pm, 1e-6)
                    vels.append(vel)
            prev[tok] = (now, mid)
        if vels:
            idx = sum(vels) / len(vels)
            row = {
                "ts": int(now),
                "cluster": cluster_name,
                "n_markets": len(vels),
                "index": round(idx, 6),
                "mids": {k: round(v, 4) for k, v in mids.items()},
            }
            with open(log_path, "a") as f:
                f.write(json.dumps(row) + "\n")
        time.sleep(POLL_S)
    print("Collection complete.")


def backtest(log_path):
    if not os.path.exists(log_path):
        print(f"No log at {log_path}. Run --collect first."); return
    rows = [json.loads(l) for l in open(log_path)]
    if len(rows) < 20:
        print(f"Only {len(rows)} samples — need more data."); return

    rows.sort(key=lambda r: r["ts"])
    ts = [r["ts"] for r in rows]
    idx_vals = [r["index"] for r in rows]
    span_h = (ts[-1] - ts[0]) / 3600

    # ── Rolling z-score ──────────────────────────────────────────────────────
    z_scores = []
    for i, row in enumerate(rows):
        window = [rows[j]["index"] for j in range(max(0, i - 240), i)]  # ~2h @ 30s
        if len(window) < 10:
            z_scores.append(None)
            continue
        mu = stat.mean(window)
        sd = stat.pstdev(window)
        z = (row["index"] - mu) / sd if sd > 1e-9 else 0.0
        z_scores.append(z)

    spikes = [(i, z) for i, z in enumerate(z_scores) if z is not None and z > SPIKE_Z]

    print(f"\n{'='*72}")
    print(f"FEAR INDEX BACKTEST — {len(rows)} samples over {span_h:.1f}h")
    print(f"{'='*72}")
    print(f"Index: mean={stat.mean(idx_vals)*100:.3f}%  "
          f"max={max(idx_vals)*100:.3f}%  "
          f"p95={sorted(idx_vals)[int(len(idx_vals)*0.95)]*100:.3f}%")
    print(f"Spikes (z>{SPIKE_Z}): {len(spikes)} / {len(rows)} samples ({len(spikes)/len(rows)*100:.1f}%)\n")

    if not spikes:
        print("No spikes yet — need more time running the collector (ideally catch a news event).")
        print("The index IS working as a defensive gate: pause quotes when z > 2.0.\n")
        return

    # ── Reversion analysis for each spike ────────────────────────────────────
    print("Reversion analysis after spikes (negative = reverted = fade worked):")
    print(f"  {'spike_time':16} {'z':>5} {'revert_5m':>9} {'revert_30m':>10} {'revert_2h':>9}")

    rev_5m, rev_30m, rev_2h = [], [], []
    for si, (i, z) in enumerate(spikes[:20]):  # show first 20
        spike_ts = rows[i]["ts"]
        spike_mids = rows[i].get("mids", {})
        if not spike_mids:
            continue

        def mid_at_offset(offset_s):
            target = spike_ts + offset_s
            best = min(rows, key=lambda r: abs(r["ts"] - target), default=None)
            if best and abs(best["ts"] - target) < offset_s * 0.5:
                return best.get("mids", {})
            return {}

        prev_mids = rows[i - 1].get("mids", {}) if i > 0 else {}
        mids_5m = mid_at_offset(300)
        mids_30m = mid_at_offset(1800)
        mids_2h = mid_at_offset(7200)

        revs = collections.defaultdict(list)
        for tok, m_now in spike_mids.items():
            m_prev = prev_mids.get(tok)
            if not m_prev or abs(m_now - m_prev) < 1e-5:
                continue  # no move to fade
            move = m_now - m_prev
            for label, mids_fut in [("5m", mids_5m), ("30m", mids_30m), ("2h", mids_2h)]:
                m_fut = mids_fut.get(tok)
                if m_fut is not None:
                    reversion = -(m_fut - m_now) / abs(move)  # + = reverted
                    revs[label].append(reversion)

        r5 = stat.mean(revs["5m"]) if revs["5m"] else None
        r30 = stat.mean(revs["30m"]) if revs["30m"] else None
        r2h = stat.mean(revs["2h"]) if revs["2h"] else None
        ts_str = dt.datetime.utcfromtimestamp(spike_ts).strftime("%m-%d %H:%M")
        print(f"  {ts_str:16} {z:>5.2f} "
              f"{'+revert' if (r5 or 0) > 0 else '-trend':>9} "
              f"{'+revert' if (r30 or 0) > 0 else '-trend':>10} "
              f"{'+revert' if (r2h or 0) > 0 else '-trend':>9}")
        if r5 is not None: rev_5m.append(r5 > 0)
        if r30 is not None: rev_30m.append(r30 > 0)
        if r2h is not None: rev_2h.append(r2h > 0)

    # ── Summary ──────────────────────────────────────────────────────────────
    print(f"\n{'='*72}")
    print("VERDICT")
    print(f"{'='*72}")
    for label, revs, horizon in [("5m", rev_5m, 5), ("30m", rev_30m, 30), ("2h", rev_2h, 120)]:
        if not revs:
            print(f"  {label:4}: no data")
            continue
        pct = sum(revs) / len(revs) * 100
        verdict = ("✓ REVERSION (fade signal — panic overshoots)" if pct > 65
                   else "✗ NO REVERSION (use as defensive gate only, not fade entry)" if pct < 50
                   else "~ MIXED (marginal)")
        print(f"  {label:4}: reverts {pct:.0f}% of the time after {horizon}m  {verdict}")

    if rev_5m or rev_30m:
        pcts = [sum(r)/len(r)*100 for r in [rev_5m, rev_30m, rev_2h] if r]
        if all(p > 65 for p in pcts):
            print("\n  → USE AS FADE ENTRY: panic consistently overshoots. "
                  "Post quotes AGGRESSIVELY when z > 2.0.")
        else:
            print("\n  → USE AS DEFENSIVE GATE ONLY: pause rewards-maker quotes when z > 2.0. "
                  "Do NOT bet the fade until more spikes confirm reversion.")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--collect", action="store_true")
    ap.add_argument("--backtest", action="store_true")
    ap.add_argument("--cluster", default="politics")
    ap.add_argument("--hours", type=float, default=24)
    ap.add_argument("--log", default=LOG_DEFAULT)
    args = ap.parse_args()
    if args.collect:
        collect(args.cluster, args.hours, args.log)
    elif args.backtest:
        backtest(args.log)
    else:
        ap.print_help()
