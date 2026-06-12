#!/usr/bin/env python3
"""
memecoin_backtest.py — Solana memecoin P1: dataset + survivorship-aware backtest (Feature 2).

Tests the ONE family that survived the viability research: "graduation momentum" —
when a token graduates onto a DEX (appears with real liquidity), does it drift
predictably in the minutes/hours after? This is event-driven (not sub-second sniping),
so it's reachable at our latency and validatable with our edge_validator methodology.

DATA: GeckoTerminal (free, no key) — new_pools (launches), trending_pools, and per-pool
OHLCV history. CRITICAL: we sample BOTH currently-live pools AND pools that have since
died (low/zero current volume) to AVOID SURVIVORSHIP BIAS — the trap that makes every
naive memecoin backtest look profitable.

The backtest models a brutal-but-honest entry: buy at the OPEN of bar K after first
liquidity, exit at bar K+H, paying `slippage_pct` each side (memecoin books are thin),
and treating a price collapse as the −100% it really is.

Usage:
  ./memecoin_backtest.py --collect --pages 5      # ingest pools + OHLCV → jsonl
  ./memecoin_backtest.py --backtest --entry-bar 3 --hold-bars 12 --slippage 3
"""
import argparse, json, os, sys, time, urllib.request
import numpy as np

GT = "https://api.geckoterminal.com/api/v2"
DATA = os.path.expanduser("~/.traderclaw/workspace/data/memecoins")
UA = {"Accept": "application/json", "User-Agent": "trader-claw/memecoin-research"}


def http(url, retries=3):
    for i in range(retries):
        try:
            return json.load(urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=20))
        except Exception as e:
            if "429" in str(e):
                time.sleep(3 * (i + 1))  # GeckoTerminal free tier: ~30 req/min
            else:
                return None
    return None


def collect(pages):
    """Ingest pools from new_pools + trending + top, then per-pool OHLCV. Mix of live and
    dead pools is what kills survivorship bias — new_pools naturally includes future deaths."""
    os.makedirs(DATA, exist_ok=True)
    pools = {}
    for endpoint in ["new_pools", "trending_pools", "pools"]:
        for pg in range(1, pages + 1):
            d = http(f"{GT}/networks/solana/{endpoint}?page={pg}")
            if not d or not d.get("data"):
                break
            for p in d["data"]:
                a = p["attributes"]
                addr = a.get("address")
                if addr and addr not in pools:
                    pools[addr] = {
                        "address": addr, "name": a.get("name", "?"),
                        "created_at": a.get("pool_created_at"),
                        "vol_h24": float((a.get("volume_usd") or {}).get("h24", 0) or 0),
                        "reserve_usd": float(a.get("reserve_in_usd", 0) or 0),
                    }
            time.sleep(2.2)  # rate limit
    print(f"Collected {len(pools)} unique pools. Fetching OHLCV…", file=sys.stderr)

    out_path = f"{DATA}/pools_ohlcv.jsonl"
    with open(out_path, "w") as f:
        for i, (addr, meta) in enumerate(pools.items()):
            # minute OHLCV from creation — captures the post-launch window
            d = http(f"{GT}/networks/solana/pools/{addr}/ohlcv/minute?aggregate=5&limit=200")
            ohlcv = (((d or {}).get("data") or {}).get("attributes") or {}).get("ohlcv_list", []) if d else []
            meta["ohlcv"] = ohlcv  # [ts, o, h, l, c, vol]
            f.write(json.dumps(meta) + "\n")
            if (i + 1) % 10 == 0:
                print(f"  {i+1}/{len(pools)}", file=sys.stderr)
            time.sleep(2.2)
    print(f"Wrote {out_path}", file=sys.stderr)


def backtest(entry_bar, hold_bars, slippage_pct, min_liq):
    path = f"{DATA}/pools_ohlcv.jsonl"
    if not os.path.exists(path):
        print("No data. Run --collect first."); return
    pools = [json.loads(l) for l in open(path)]
    slip = slippage_pct / 100.0

    # ── Survivorship-bias accounting ──
    total = len(pools)
    with_ohlcv = [p for p in pools if len(p.get("ohlcv", [])) > entry_bar + hold_bars]
    dead = [p for p in pools if p.get("vol_h24", 0) < min_liq]
    print(f"\n{'='*72}")
    print(f"MEMECOIN GRADUATION BACKTEST — {total} pools sampled")
    print(f"{'='*72}")
    print(f"  Pools with enough OHLCV: {len(with_ohlcv)}")
    print(f"  Currently dead (vol24h<${min_liq:.0f}): {len(dead)} ({len(dead)/max(total,1)*100:.0f}%)  "
          f"← these MUST be in the backtest or it's survivorship-biased")

    # ── The strategy: buy at open of bar `entry_bar`, sell at bar entry_bar+hold_bars ──
    rets = []
    rugs = 0
    for p in with_ohlcv:
        # GeckoTerminal returns newest-first; reverse to chronological
        bars = list(reversed(p["ohlcv"]))
        if len(bars) <= entry_bar + hold_bars:
            continue
        entry_px = bars[entry_bar][1]   # open of entry bar
        exit_px = bars[entry_bar + hold_bars][4]  # close of exit bar
        if entry_px <= 0:
            continue
        gross = exit_px / entry_px - 1.0
        net = (exit_px * (1 - slip)) / (entry_px * (1 + slip)) - 1.0
        # treat a >90% collapse as a rug (−100% realistically — you can't exit a dead book)
        if exit_px / entry_px < 0.1:
            net = -1.0
            rugs += 1
        rets.append(net)

    if len(rets) < 20:
        print(f"\n  Only {len(rets)} tradeable samples — collect more pages."); return
    rets = np.array(rets)
    fee_note = f"entry bar {entry_bar}, hold {hold_bars} bars (×5min), slippage {slippage_pct}%/side"
    print(f"\n  Strategy: {fee_note}")
    print(f"  n={len(rets)}  mean={rets.mean()*100:+.1f}%  median={np.median(rets)*100:+.1f}%  "
          f"win%={np.mean(rets>0)*100:.0f}%  rugs={rugs} ({rugs/len(rets)*100:.0f}%)")
    print(f"  Best={rets.max()*100:+.0f}%  Worst={rets.min()*100:+.0f}%  "
          f"total if equal-weight={rets.mean()*len(rets)*100:+.0f}% over {len(rets)} trades")
    # The honest test: is the MEAN positive after the left tail (rugs)?
    print(f"\n  VERDICT: ", end="")
    if rets.mean() > 0.02 and np.median(rets) > -0.05:
        print("⚠ POSITIVE mean — worth a TRAIN/TEST split + edge_validator. NOT yet proven.")
    else:
        print("✗ NEGATIVE/zero mean after rugs — graduation-momentum has no edge in this sample.")
    print("  (Caveat: GeckoTerminal trending/top is itself survivorship-skewed UP. A truly")
    print("   unbiased test needs the FULL launch firehose, e.g. paid Bitquery. Treat positive")
    print("   results here as an OPTIMISTIC upper bound, not proof.)")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--collect", action="store_true")
    ap.add_argument("--backtest", action="store_true")
    ap.add_argument("--pages", type=int, default=5)
    ap.add_argument("--entry-bar", type=int, default=3)
    ap.add_argument("--hold-bars", type=int, default=12)
    ap.add_argument("--slippage", type=float, default=3.0)
    ap.add_argument("--min-liq", type=float, default=1000)
    args = ap.parse_args()
    if args.collect:
        collect(args.pages)
    elif args.backtest:
        backtest(args.entry_bar, args.hold_bars, args.slippage, args.min_liq)
    else:
        ap.print_help()
