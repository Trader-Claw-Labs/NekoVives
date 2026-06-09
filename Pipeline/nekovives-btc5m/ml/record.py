"""record.py — record REAL data for the btc5m pipeline (paper mode).

Runs three concurrent feeds and writes the exact schema train.py/backtest.py
expect:
  samples.parquet     : one row per (window_id, t) with FEATURE_ORDER + pm_*_ask/spread
  resolutions.parquet : one row per window_id with chainlink_close, price_to_beat

Feeds (all public, no auth needed for reads):
  1. Binance combined WS  — aggTrade (signed flow + spot) + depth (imbalance)
  2. Polymarket RTDS       — crypto_prices_chainlink btc/usd = the RESOLVING price
  3. Polymarket CLOB market— Up/Down token best bid/ask (what you actually trade)

Verified message formats (mid-2026):
  RTDS  wss://ws-live-data.polymarket.com
        subscribe {"action":"subscribe","subscriptions":[{"topic":"crypto_prices_chainlink",
                   "type":"*","filters":"{\\"symbol\\":\\"btc/usd\\"}"}]}  ; PING every 5s
  CLOB  wss://ws-subscriptions-clob.polymarket.com/ws/market
        subscribe {"assets_ids":[up,down],"type":"market","custom_feature_enabled":true}
        events: book, price_change(best_bid/best_ask), best_bid_ask ; PING every 10s

VERIFY before a long run (these can drift):
  * Gamma discovery: slug pattern `btc-updown-5m-{close_unix_s}`, 300s-aligned.
    GET https://gamma-api.polymarket.com/markets?slug=<slug> -> [{clobTokenIds, outcomes,
    conditionId, endDate, ...}]. We assume slug ts = window CLOSE. Confirm against a
    live market page once, then trust it.
  * Outcome order: we map by the `outcomes` strings ("Up"/"Down"), not by index.

Usage:
  python record.py --out data --hours 0     # run until Ctrl-C, flush periodically
  python record.py --selftest               # no network; prove feature+IO path works
"""

from __future__ import annotations

import argparse
import asyncio
import json
import math
import time
from collections import deque

import pandas as pd

from features import FEATURE_ORDER, compute_features

BINANCE_URL = (
    "wss://stream.binance.com:9443/stream"
    "?streams=btcusdt@aggTrade/btcusdt@depth10@100ms"
)
RTDS_URL = "wss://ws-live-data.polymarket.com"
CLOB_URL = "wss://ws-subscriptions-clob.polymarket.com/ws/market"
GAMMA_MARKETS = "https://gamma-api.polymarket.com/markets"

HISTORY_MS = 90_000
SAMPLE_EVERY_S = 5.0
SAMPLE_WHEN_SECS_LEFT = (5.0, 60.0)  # record only inside the last 60s, not under 5s


class State:
    def __init__(self):
        self.now_ms = 0
        self.spot = 0.0
        self.trades: deque = deque()
        self.bids: list = []  # (price, qty) desc
        self.asks: list = []  # (price, qty) asc
        self.chainlink = 0.0
        self.chainlink_hist: deque = deque()  # (ts_ms, price)
        self.window = None  # dict
        self.pm = {}  # token_id -> dict(best_bid, best_ask, bid_size, ask_size)

    def prune(self):
        cut = self.now_ms - HISTORY_MS
        while self.trades and self.trades[0]["ts_ms"] < cut:
            self.trades.popleft()
        while self.chainlink_hist and self.chainlink_hist[0][0] < cut:
            self.chainlink_hist.popleft()


# --------------------------------------------------------------------------- #
# Feeds
# --------------------------------------------------------------------------- #
async def binance_feed(st: State, stop: asyncio.Event):
    import websockets

    backoff = 0.5
    while not stop.is_set():
        try:
            async with websockets.connect(BINANCE_URL, ping_interval=None) as ws:
                backoff = 0.5
                async for raw in ws:
                    msg = json.loads(raw)
                    stream, data = msg.get("stream", ""), msg.get("data", {})
                    if stream.endswith("aggTrade"):
                        ts = int(data["T"])
                        st.now_ms = max(st.now_ms, ts)
                        st.spot = float(data["p"])
                        st.trades.append({
                            "ts_ms": ts,
                            "price": float(data["p"]),
                            "qty": float(data["q"]),
                            "buyer_is_maker": bool(data["m"]),
                        })
                        st.prune()
                    elif "depth" in stream:
                        st.bids = [(float(p), float(q)) for p, q in data["bids"]]
                        st.asks = [(float(p), float(q)) for p, q in data["asks"]]
                    if stop.is_set():
                        break
        except Exception as e:  # noqa: BLE001
            print(f"[binance] {e}; reconnect in {backoff:.1f}s")
            await asyncio.sleep(backoff)
            backoff = min(backoff * 2, 15)


async def rtds_feed(st: State, stop: asyncio.Event):
    import websockets

    sub = {
        "action": "subscribe",
        "subscriptions": [{
            "topic": "crypto_prices_chainlink",
            "type": "*",
            "filters": json.dumps({"symbol": "btc/usd"}),
        }],
    }
    backoff = 0.5
    while not stop.is_set():
        try:
            async with websockets.connect(RTDS_URL, ping_interval=None) as ws:
                await ws.send(json.dumps(sub))
                backoff = 0.5

                async def pinger():
                    while True:
                        await asyncio.sleep(5)
                        await ws.send("PING")

                pt = asyncio.create_task(pinger())
                try:
                    async for raw in ws:
                        if raw == "PONG":
                            continue
                        msg = json.loads(raw)
                        pl = msg.get("payload", {})
                        if "value" not in pl:
                            continue
                        ts = int(pl.get("timestamp", msg.get("timestamp", 0)))
                        px = float(pl["value"])
                        st.now_ms = max(st.now_ms, ts)
                        st.chainlink = px
                        st.chainlink_hist.append((ts, px))
                        st.prune()
                        if stop.is_set():
                            break
                finally:
                    pt.cancel()
        except Exception as e:  # noqa: BLE001
            print(f"[rtds] {e}; reconnect in {backoff:.1f}s")
            await asyncio.sleep(backoff)
            backoff = min(backoff * 2, 15)


async def clob_feed(st: State, stop: asyncio.Event, sub_q: asyncio.Queue):
    """Maintains best bid/ask for whatever token IDs are pushed onto `sub_q`."""
    import websockets

    current = set()
    backoff = 0.5
    while not stop.is_set():
        try:
            async with websockets.connect(CLOB_URL, ping_interval=None) as ws:
                backoff = 0.5
                if current:
                    await ws.send(json.dumps({
                        "assets_ids": list(current), "type": "market",
                        "custom_feature_enabled": True,
                    }))

                async def pinger():
                    while True:
                        await asyncio.sleep(10)
                        await ws.send("PING")

                pt = asyncio.create_task(pinger())
                try:
                    while not stop.is_set():
                        # drain any new subscription requests
                        try:
                            tokens = sub_q.get_nowait()
                            current = set(tokens)
                            await ws.send(json.dumps({
                                "assets_ids": list(current), "type": "market",
                                "operation": "subscribe", "custom_feature_enabled": True,
                            }))
                        except asyncio.QueueEmpty:
                            pass
                        try:
                            raw = await asyncio.wait_for(ws.recv(), timeout=1.0)
                        except asyncio.TimeoutError:
                            continue
                        if raw == "PONG":
                            continue
                        for ev in _as_list(json.loads(raw)):
                            _apply_clob_event(st, ev)
                finally:
                    pt.cancel()
        except Exception as e:  # noqa: BLE001
            print(f"[clob] {e}; reconnect in {backoff:.1f}s")
            await asyncio.sleep(backoff)
            backoff = min(backoff * 2, 15)


def _as_list(obj):
    return obj if isinstance(obj, list) else [obj]


def _apply_clob_event(st: State, ev: dict):
    et = ev.get("event_type")
    tok = ev.get("asset_id")
    if et == "book" and tok:
        bids = ev.get("bids", [])
        asks = ev.get("asks", [])
        bb = max((float(b["price"]) for b in bids), default=0.0)
        ba = min((float(a["price"]) for a in asks), default=0.0)
        bsz = next((float(b["size"]) for b in bids if float(b["price"]) == bb), 0.0)
        asz = next((float(a["size"]) for a in asks if float(a["price"]) == ba), 0.0)
        st.pm[tok] = {"best_bid": bb, "best_ask": ba, "bid_size": bsz, "ask_size": asz}
    elif et in ("price_change", "best_bid_ask") and tok:
        d = st.pm.setdefault(tok, {"best_bid": 0.0, "best_ask": 0.0,
                                   "bid_size": 0.0, "ask_size": 0.0})
        changes = ev.get("price_changes", [ev])
        for pc in changes:
            if "best_bid" in pc:
                d["best_bid"] = float(pc["best_bid"])
            if "best_ask" in pc:
                d["best_ask"] = float(pc["best_ask"])


# --------------------------------------------------------------------------- #
# Window discovery + resolution
# --------------------------------------------------------------------------- #
async def discover_window(close_unix_s: int):
    """Fetch the 5m BTC market for a given close boundary. Returns dict or None."""
    import requests  # sync; called via to_thread

    slug = f"btc-updown-5m-{close_unix_s}"

    def _get():
        r = requests.get(GAMMA_MARKETS, params={"slug": slug}, timeout=5)
        r.raise_for_status()
        return r.json()

    try:
        rows = await asyncio.to_thread(_get)
    except Exception as e:  # noqa: BLE001
        print(f"[window] discovery failed for {slug}: {e}")
        return None
    if not rows:
        return None
    m = rows[0]
    outcomes = json.loads(m["outcomes"]) if isinstance(m["outcomes"], str) else m["outcomes"]
    tokens = (json.loads(m["clobTokenIds"]) if isinstance(m["clobTokenIds"], str)
              else m["clobTokenIds"])
    omap = {o.lower(): t for o, t in zip(outcomes, tokens)}
    up = omap.get("up")
    down = omap.get("down")
    if not (up and down):
        return None
    return {
        "window_id": close_unix_s,
        "slug": slug,
        "condition_id": m.get("conditionId"),
        "up_token": up,
        "down_token": down,
        "open_ms": (close_unix_s - 300) * 1000,
        "close_ms": close_unix_s * 1000,
        "price_to_beat": None,
    }


async def window_manager(st: State, stop: asyncio.Event, sub_q: asyncio.Queue,
                         resolutions: list):
    active = None
    while not stop.is_set():
        now_s = int(time.time())
        close_b = (now_s // 300 + 1) * 300  # next 300s boundary = current window close

        if active is None or active["window_id"] != close_b:
            w = await discover_window(close_b)
            if w:
                # capture price-to-beat: first chainlink tick at/after open, else REST
                ptb = _chainlink_at(st, w["open_ms"])
                if ptb is None:
                    ptb = await _fetch_ptb(w["slug"])
                w["price_to_beat"] = ptb
                st.window = w
                active = w
                await sub_q.put([w["up_token"], w["down_token"]])
                print(f"[window] active {w['slug']} ptb={ptb}")

        # finalize any window that has closed
        if active and st.now_ms >= active["close_ms"]:
            await asyncio.sleep(2)  # let the close-boundary chainlink tick arrive
            close_px = _chainlink_at(st, active["close_ms"])
            if active["price_to_beat"] and close_px:
                resolutions.append({
                    "window_id": active["window_id"],
                    "chainlink_close": close_px,
                    "price_to_beat": active["price_to_beat"],
                })
                print(f"[window] resolved {active['slug']} "
                      f"close={close_px} ptb={active['price_to_beat']}")
            active = None
            st.window = None

        await asyncio.sleep(1)


def _chainlink_at(st: State, ts_ms: int):
    """First chainlink price with ts >= ts_ms from history, else None."""
    for ts, px in st.chainlink_hist:
        if ts >= ts_ms:
            return px
    return None


async def _fetch_ptb(slug: str):
    import requests

    def _get():
        r = requests.get(
            f"https://polymarket.com/api/equity/price-to-beat/{slug}", timeout=5
        )
        r.raise_for_status()
        return r.json()

    try:
        d = await asyncio.to_thread(_get)
        return float(d.get("priceToBeat", d.get("value")))
    except Exception:  # noqa: BLE001
        return None


# --------------------------------------------------------------------------- #
# Sampler
# --------------------------------------------------------------------------- #
def build_sample(st: State) -> dict | None:
    w = st.window
    if not w or w.get("price_to_beat") is None:
        return None
    secs_left = max(0.0, (w["close_ms"] - st.now_ms) / 1000.0)
    lo, hi = SAMPLE_WHEN_SECS_LEFT
    if not (lo <= secs_left <= hi):
        return None
    up = st.pm.get(w["up_token"])
    down = st.pm.get(w["down_token"])
    if not up or not down or up["best_ask"] <= 0 or down["best_ask"] <= 0:
        return None

    feats = compute_features(
        now_ms=st.now_ms, spot=st.spot, chainlink=st.chainlink,
        price_to_beat=w["price_to_beat"], secs_left=secs_left,
        bids=st.bids, asks=st.asks, trades=list(st.trades),
    )
    row = {"window_id": w["window_id"], "t": st.now_ms}
    row.update(feats)
    row.update({
        "pm_up_ask": up["best_ask"],
        "pm_down_ask": down["best_ask"],
        "pm_up_spread": max(0.0, up["best_ask"] - up["best_bid"]),
        "pm_down_spread": max(0.0, down["best_ask"] - down["best_bid"]),
    })
    return row


async def sampler(st: State, stop: asyncio.Event, samples: list):
    while not stop.is_set():
        await asyncio.sleep(SAMPLE_EVERY_S)
        row = build_sample(st)
        if row:
            samples.append(row)


async def flusher(out: str, samples: list, resolutions: list, stop: asyncio.Event):
    while not stop.is_set():
        await asyncio.sleep(30)
        _flush(out, samples, resolutions)


def _flush(out: str, samples: list, resolutions: list):
    if samples:
        pd.DataFrame(samples).to_parquet(f"{out}/samples.parquet")
    if resolutions:
        pd.DataFrame(resolutions).to_parquet(f"{out}/resolutions.parquet")
    print(f"[flush] samples={len(samples)} resolutions={len(resolutions)}")


# --------------------------------------------------------------------------- #
async def main(a):
    import os

    os.makedirs(a.out, exist_ok=True)
    st = State()
    stop = asyncio.Event()
    sub_q: asyncio.Queue = asyncio.Queue()
    samples, resolutions = [], []

    tasks = [
        asyncio.create_task(binance_feed(st, stop)),
        asyncio.create_task(rtds_feed(st, stop)),
        asyncio.create_task(clob_feed(st, stop, sub_q)),
        asyncio.create_task(window_manager(st, stop, sub_q, resolutions)),
        asyncio.create_task(sampler(st, stop, samples)),
        asyncio.create_task(flusher(a.out, samples, resolutions, stop)),
    ]
    print("recording… Ctrl-C to stop")
    try:
        if a.hours > 0:
            await asyncio.sleep(a.hours * 3600)
            stop.set()
        else:
            await asyncio.gather(*tasks)
    except (KeyboardInterrupt, asyncio.CancelledError):
        pass
    finally:
        stop.set()
        for t in tasks:
            t.cancel()
        _flush(a.out, samples, resolutions)


def selftest():
    """No network: feed fabricated buffers through the sample/IO path."""
    import os

    os.makedirs("data", exist_ok=True)
    st = State()
    now = int(time.time() * 1000)
    st.now_ms = now
    st.spot = 100_050.0
    st.chainlink = 100_040.0
    st.bids = [(0.0, 0.0)]  # spot book (Binance) for imbalance
    st.bids = [(100_039.0, 8.0), (100_038.0, 5.0)]
    st.asks = [(100_041.0, 2.0), (100_042.0, 4.0)]
    st.trades = deque([
        {"ts_ms": now - 3000, "price": 100_020.0, "qty": 1.5, "buyer_is_maker": False},
        {"ts_ms": now - 1000, "price": 100_050.0, "qty": 0.8, "buyer_is_maker": False},
    ])
    st.window = {
        "window_id": (now // 1000 // 300 + 1) * 300, "up_token": "U", "down_token": "D",
        "open_ms": now - 280_000, "close_ms": now + 12_000, "price_to_beat": 100_000.0,
    }
    st.pm = {
        "U": {"best_bid": 0.70, "best_ask": 0.73, "bid_size": 50, "ask_size": 40},
        "D": {"best_bid": 0.27, "best_ask": 0.30, "bid_size": 50, "ask_size": 40},
    }
    row = build_sample(st)
    assert row is not None, "build_sample returned None"
    assert all(k in row for k in FEATURE_ORDER), "missing feature columns"
    df = pd.DataFrame([row])
    df.to_parquet("data/samples.parquet")
    print("selftest OK — sample row:")
    for k in ["window_id", *FEATURE_ORDER, "pm_up_ask", "pm_down_ask"]:
        print(f"  {k:18s} {row[k]}")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="data")
    ap.add_argument("--hours", type=float, default=0.0, help="0 = until Ctrl-C")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()
    if args.selftest:
        selftest()
    else:
        asyncio.run(main(args))
