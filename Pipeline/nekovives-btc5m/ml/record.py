"""record.py — multi-asset live recorder for the btc5m pipeline.

Graba datos reales para BTC, ETH, XRP, DOGE, BNB (o cualquier subconjunto)
en paralelo. Cada asset corre su propia máquina de estado, ventana y sampler,
pero comparte las tres conexiones WebSocket (Binance, RTDS, CLOB) para no
multiplicar conexiones innecesariamente.

Salida por asset (en --out/{asset}/):
  samples.parquet     : (window_id, asset, t, <FEATURE_ORDER>, pm_*_ask/spread)
  resolutions.parquet : (window_id, asset, chainlink_close, price_to_beat)

También escribe un parquet combinado en --out/all/:
  samples_all.parquet / resolutions_all.parquet

Feeds:
  Binance  — aggTrade + depth10 para cada symbol (un WS combinado)
  RTDS     — crypto_prices_chainlink para cada symbol en una sola conexión
  CLOB     — todos los token IDs activos en una sola conexión

Formatos verificados (mid-2026):
  RTDS: {"action":"subscribe","subscriptions":[{"topic":"crypto_prices_chainlink",
         "type":"*","filters":"{\\"symbol\\":\\"btc/usd\\"}"}]}  PING cada 5s
  CLOB: {"assets_ids":[...],"type":"market","custom_feature_enabled":true}  PING cada 10s

VERIFY una vez antes de un run largo:
  * Slug pattern para cada asset: {asset}-updown-5m-{close_unix_s}
    Confirmado para BTC (btc-updown-5m-...) y BNB (bnb-updown-5m-...).
  * bnb/usd y doge/usd pueden no estar documentados en RTDS pero el feed existe en
    Chainlink; si no llegan ticks, `chainlink_ok` queda False y el recorder lo reporta.
  * El recorder detecta automáticamente si un símbolo RTDS no entrega datos.

Uso:
  python record.py --selftest                           # verifica sin red
  python record.py                                      # todos los assets, hasta Ctrl-C
  python record.py --assets btc eth xrp                 # solo esos tres
  python record.py --assets btc --out data --hours 48   # solo BTC, 48h
"""

from __future__ import annotations

import argparse
import asyncio
import json
import time
from collections import deque
from dataclasses import dataclass, field
from pathlib import Path

import pandas as pd

from features import FEATURE_ORDER, compute_features

# --------------------------------------------------------------------------- #
# Asset catalog
# Todos los campos son verificables contra polymarket.com/crypto/5M y data.chain.link
# --------------------------------------------------------------------------- #

@dataclass
class AssetConfig:
    name: str          # etiqueta en los parquets y directorios
    binance_sym: str   # símbolo Binance WS, ej "btcusdt"
    rtds_sym: str      # símbolo RTDS Chainlink, ej "btc/usd"
    slug_prefix: str   # prefijo del slug Gamma, ej "btc-updown-5m"

ASSETS: dict[str, AssetConfig] = {
    "btc":  AssetConfig("btc",  "btcusdt",  "btc/usd",  "btc-updown-5m"),
    "eth":  AssetConfig("eth",  "ethusdt",  "eth/usd",  "eth-updown-5m"),
    "xrp":  AssetConfig("xrp",  "xrpusdt",  "xrp/usd",  "xrp-updown-5m"),
    "doge": AssetConfig("doge", "dogeusdt", "doge/usd", "doge-updown-5m"),
    "bnb":  AssetConfig("bnb",  "bnbusdt",  "bnb/usd",  "bnb-updown-5m"),
}

RTDS_URL  = "wss://ws-live-data.polymarket.com"
CLOB_URL  = "wss://ws-subscriptions-clob.polymarket.com/ws/market"
GAMMA_URL = "https://gamma-api.polymarket.com/markets"

HISTORY_MS       = 90_000
SAMPLE_EVERY_S   = 5.0
SAMPLE_SECS_LO   = 5.0    # no samplear con menos de 5s restantes
SAMPLE_SECS_HI   = 60.0   # no samplear con más de 60s restantes

# --------------------------------------------------------------------------- #
# Estado por asset
# --------------------------------------------------------------------------- #

class AssetState:
    def __init__(self, cfg: AssetConfig):
        self.cfg = cfg
        self.now_ms: int   = 0
        self.spot: float   = 0.0
        self.trades: deque = deque()
        self.bids: list    = []   # (price, qty) desc
        self.asks: list    = []   # (price, qty) asc
        self.chainlink: float       = 0.0
        self.chainlink_hist: deque  = deque()
        self.chainlink_ok: bool     = False   # ¿llegó al menos un tick RTDS?
        self.window: dict | None    = None
        self.pm: dict               = {}  # token_id → {best_bid,best_ask,bid_size,ask_size}

    def on_trade(self, ts_ms: int, price: float, qty: float, buyer_is_maker: bool):
        self.now_ms = max(self.now_ms, ts_ms)
        self.spot = price
        self.trades.append({"ts_ms": ts_ms, "price": price,
                             "qty": qty, "buyer_is_maker": buyer_is_maker})
        self._prune()

    def on_depth(self, bids: list, asks: list):
        self.bids, self.asks = bids, asks

    def on_chainlink(self, ts_ms: int, price: float):
        self.now_ms = max(self.now_ms, ts_ms)
        self.chainlink = price
        self.chainlink_ok = True
        self.chainlink_hist.append((ts_ms, price))
        self._prune()

    def chainlink_at(self, ts_ms: int) -> float | None:
        for ts, px in self.chainlink_hist:
            if ts >= ts_ms:
                return px
        return None

    def _prune(self):
        cut = self.now_ms - HISTORY_MS
        while self.trades and self.trades[0]["ts_ms"] < cut:
            self.trades.popleft()
        while self.chainlink_hist and self.chainlink_hist[0][0] < cut:
            self.chainlink_hist.popleft()


# --------------------------------------------------------------------------- #
# Binance: un WS combinado para todos los assets activos
# --------------------------------------------------------------------------- #

def _binance_url(cfgs: list[AssetConfig]) -> str:
    streams = []
    for c in cfgs:
        s = c.binance_sym
        streams += [f"{s}@aggTrade", f"{s}@depth10@100ms"]
    return f"wss://stream.binance.com:9443/stream?streams={'/' .join(streams)}"


async def binance_feed(states: dict[str, AssetState], stop: asyncio.Event):
    import websockets

    url = _binance_url([s.cfg for s in states.values()])
    # sym -> asset name
    sym_map = {c.binance_sym: name for name, c in
               {n: s.cfg for n, s in states.items()}.items()}

    backoff = 0.5
    while not stop.is_set():
        try:
            async with websockets.connect(url, ping_interval=None) as ws:
                backoff = 0.5
                async for raw in ws:
                    msg  = json.loads(raw)
                    stream = msg.get("stream", "")
                    data   = msg.get("data", {})
                    sym    = stream.split("@")[0]
                    name   = sym_map.get(sym)
                    if name is None:
                        continue
                    st = states[name]
                    if stream.endswith("aggTrade"):
                        st.on_trade(int(data["T"]), float(data["p"]),
                                    float(data["q"]), bool(data["m"]))
                    elif "depth" in stream:
                        st.on_depth(
                            [(float(p), float(q)) for p, q in data["bids"]],
                            [(float(p), float(q)) for p, q in data["asks"]],
                        )
                    if stop.is_set():
                        break
        except Exception as e:  # noqa: BLE001
            print(f"[binance] {e}; reconectando en {backoff:.1f}s")
            await asyncio.sleep(backoff)
            backoff = min(backoff * 2, 15)


# --------------------------------------------------------------------------- #
# RTDS Chainlink: una sola conexión, multi-suscripción
# --------------------------------------------------------------------------- #

async def rtds_feed(states: dict[str, AssetState], stop: asyncio.Event):
    import websockets

    # mapa rtds_sym (lowercase) → AssetState
    sym_map = {s.cfg.rtds_sym.lower(): s for s in states.values()}

    subs = [
        {"topic": "crypto_prices_chainlink", "type": "*",
         "filters": json.dumps({"symbol": sym})}
        for sym in sym_map
    ]
    sub_msg = json.dumps({"action": "subscribe", "subscriptions": subs})

    backoff = 0.5
    while not stop.is_set():
        try:
            async with websockets.connect(RTDS_URL, ping_interval=None) as ws:
                await ws.send(sub_msg)
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
                        pl  = msg.get("payload", {})
                        if "value" not in pl:
                            continue
                        sym = pl.get("symbol", "").lower()
                        st  = sym_map.get(sym)
                        if st is None:
                            continue
                        ts = int(pl.get("timestamp", msg.get("timestamp", 0)))
                        st.on_chainlink(ts, float(pl["value"]))
                        if stop.is_set():
                            break
                finally:
                    pt.cancel()
        except Exception as e:  # noqa: BLE001
            print(f"[rtds] {e}; reconectando en {backoff:.1f}s")
            await asyncio.sleep(backoff)
            backoff = min(backoff * 2, 15)


# --------------------------------------------------------------------------- #
# CLOB: una sola conexión, suscribirse/desuscribirse dinámicamente
# --------------------------------------------------------------------------- #

async def clob_feed(states: dict[str, AssetState], stop: asyncio.Event,
                    sub_q: asyncio.Queue):
    import websockets

    # token_id → AssetState (para rutear eventos book/price_change)
    token_map: dict[str, AssetState] = {}
    current: set[str] = set()

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
                        # nuevas suscripciones / desuscripciones
                        try:
                            op, name, tokens = sub_q.get_nowait()
                            st = states.get(name)
                            if st is None:
                                continue
                            if op == "sub":
                                for t in tokens:
                                    token_map[t] = st
                                current |= set(tokens)
                                await ws.send(json.dumps({
                                    "assets_ids": list(tokens),
                                    "type": "market", "operation": "subscribe",
                                    "custom_feature_enabled": True,
                                }))
                            elif op == "unsub":
                                for t in tokens:
                                    token_map.pop(t, None)
                                current -= set(tokens)
                                if tokens:
                                    await ws.send(json.dumps({
                                        "assets_ids": list(tokens),
                                        "type": "market", "operation": "unsubscribe",
                                    }))
                        except asyncio.QueueEmpty:
                            pass

                        try:
                            raw = await asyncio.wait_for(ws.recv(), timeout=1.0)
                        except asyncio.TimeoutError:
                            continue
                        if raw == "PONG":
                            continue
                        for ev in (raw if isinstance(raw, list) else [json.loads(raw)]):
                            if not isinstance(ev, dict):
                                continue
                            tok = ev.get("asset_id")
                            st  = token_map.get(tok) if tok else None
                            if st:
                                _apply_clob_event(st, ev)
                finally:
                    pt.cancel()
        except Exception as e:  # noqa: BLE001
            print(f"[clob] {e}; reconectando en {backoff:.1f}s")
            await asyncio.sleep(backoff)
            backoff = min(backoff * 2, 15)


def _apply_clob_event(st: AssetState, ev: dict):
    et  = ev.get("event_type")
    tok = ev.get("asset_id")
    if not tok:
        return
    if et == "book":
        bids = ev.get("bids", [])
        asks = ev.get("asks", [])
        bb   = max((float(b["price"]) for b in bids), default=0.0)
        ba   = min((float(a["price"]) for a in asks), default=0.0)
        bsz  = next((float(b["size"]) for b in bids if float(b["price"]) == bb), 0.0)
        asz  = next((float(a["size"]) for a in asks if float(a["price"]) == ba), 0.0)
        st.pm[tok] = {"best_bid": bb, "best_ask": ba, "bid_size": bsz, "ask_size": asz}
    elif et in ("price_change", "best_bid_ask"):
        d = st.pm.setdefault(tok, {"best_bid": 0.0, "best_ask": 0.0,
                                   "bid_size": 0.0, "ask_size": 0.0})
        for pc in ev.get("price_changes", [ev]):
            if "best_bid" in pc:
                d["best_bid"] = float(pc["best_bid"])
            if "best_ask" in pc:
                d["best_ask"] = float(pc["best_ask"])


# --------------------------------------------------------------------------- #
# Window manager (uno por asset)
# --------------------------------------------------------------------------- #

async def _gamma_fetch(slug: str) -> dict | None:
    import requests

    def _get():
        r = requests.get(GAMMA_URL, params={"slug": slug}, timeout=5)
        r.raise_for_status()
        return r.json()

    try:
        rows = await asyncio.to_thread(_get)
    except Exception as e:  # noqa: BLE001
        print(f"  [gamma] {slug}: {e}")
        return None
    if not rows:
        return None
    m        = rows[0]
    outcomes = json.loads(m["outcomes"]) if isinstance(m["outcomes"], str) else m["outcomes"]
    tokens   = json.loads(m["clobTokenIds"]) if isinstance(m["clobTokenIds"], str) else m["clobTokenIds"]
    omap     = {o.lower(): t for o, t in zip(outcomes, tokens)}
    up, down = omap.get("up"), omap.get("down")
    if not (up and down):
        return None
    return {"up": up, "down": down, "condition_id": m.get("conditionId")}


async def _fetch_ptb(slug: str) -> float | None:
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


async def window_manager(st: AssetState, stop: asyncio.Event,
                         sub_q: asyncio.Queue, resolutions: list):
    active     = None
    active_toks: list[str] = []

    while not stop.is_set():
        now_s   = int(time.time())
        close_b = (now_s // 300 + 1) * 300

        # descubrir/renovar ventana
        if active is None or active["window_id"] != close_b:
            slug = f"{st.cfg.slug_prefix}-{close_b}"
            info = await _gamma_fetch(slug)
            if info:
                ptb = st.chainlink_at((close_b - 300) * 1000)
                if ptb is None:
                    ptb = await _fetch_ptb(slug)
                w = {
                    "window_id": close_b, "slug": slug,
                    "condition_id": info["condition_id"],
                    "up_token": info["up"], "down_token": info["down"],
                    "open_ms": (close_b - 300) * 1000, "close_ms": close_b * 1000,
                    "price_to_beat": ptb,
                }
                # desuscribir tokens del ciclo anterior
                if active_toks:
                    await sub_q.put(("unsub", st.cfg.name, active_toks))
                active_toks = [info["up"], info["down"]]
                await sub_q.put(("sub", st.cfg.name, active_toks))
                st.window = active = w
                print(f"[{st.cfg.name}] ventana {slug} ptb={ptb}")

        # resolver ventana cerrada
        if active and st.now_ms >= active["close_ms"]:
            await asyncio.sleep(2)
            close_px = st.chainlink_at(active["close_ms"])
            if active["price_to_beat"] and close_px:
                resolutions.append({
                    "asset":           st.cfg.name,
                    "window_id":       active["window_id"],
                    "chainlink_close": close_px,
                    "price_to_beat":   active["price_to_beat"],
                })
                label = "UP" if close_px >= active["price_to_beat"] else "DOWN"
                print(f"[{st.cfg.name}] resuelto {active['slug']} → {label} "
                      f"close={close_px:.4f} ptb={active['price_to_beat']:.4f}")
            active = None
            st.window = None

        await asyncio.sleep(1)


# --------------------------------------------------------------------------- #
# Sampler (uno por asset)
# --------------------------------------------------------------------------- #

def build_sample(st: AssetState) -> dict | None:
    w = st.window
    if not w or w.get("price_to_beat") is None:
        return None
    if not st.chainlink_ok:
        return None
    secs_left = max(0.0, (w["close_ms"] - st.now_ms) / 1000.0)
    if not (SAMPLE_SECS_LO <= secs_left <= SAMPLE_SECS_HI):
        return None
    up   = st.pm.get(w["up_token"])
    down = st.pm.get(w["down_token"])
    if not up or not down or up["best_ask"] <= 0 or down["best_ask"] <= 0:
        return None

    feats = compute_features(
        now_ms=st.now_ms, spot=st.spot, chainlink=st.chainlink,
        price_to_beat=w["price_to_beat"], secs_left=secs_left,
        bids=st.bids, asks=st.asks, trades=list(st.trades),
    )
    row = {"asset": st.cfg.name, "window_id": w["window_id"], "t": st.now_ms}
    row.update(feats)
    row.update({
        "pm_up_ask":    up["best_ask"],
        "pm_down_ask":  down["best_ask"],
        "pm_up_spread": max(0.0, up["best_ask"]   - up["best_bid"]),
        "pm_down_spread": max(0.0, down["best_ask"] - down["best_bid"]),
    })
    return row


async def sampler(st: AssetState, stop: asyncio.Event, samples: list):
    while not stop.is_set():
        await asyncio.sleep(SAMPLE_EVERY_S)
        row = build_sample(st)
        if row:
            samples.append(row)


# --------------------------------------------------------------------------- #
# Flush: por asset + combinado
# --------------------------------------------------------------------------- #

def _flush(out: str, asset_samples: dict, asset_resolutions: dict):
    p = Path(out)
    all_s, all_r = [], []
    for name in asset_samples:
        ss = asset_samples[name]
        rs = asset_resolutions[name]
        if ss:
            d = p / name
            d.mkdir(parents=True, exist_ok=True)
            pd.DataFrame(ss).to_parquet(d / "samples.parquet")
            all_s.extend(ss)
        if rs:
            d = p / name
            d.mkdir(parents=True, exist_ok=True)
            pd.DataFrame(rs).to_parquet(d / "resolutions.parquet")
            all_r.extend(rs)
    if all_s:
        (p / "all").mkdir(parents=True, exist_ok=True)
        pd.DataFrame(all_s).to_parquet(p / "all" / "samples_all.parquet")
    if all_r:
        (p / "all").mkdir(parents=True, exist_ok=True)
        pd.DataFrame(all_r).to_parquet(p / "all" / "resolutions_all.parquet")
    totals = {n: len(v) for n, v in asset_samples.items() if v}
    print(f"[flush] muestras por asset: {totals}  resoluciones: "
          f"{{n: len(v) for n, v in asset_resolutions.items() if v}}")


async def flusher(out: str, asset_samples: dict, asset_resolutions: dict,
                  stop: asyncio.Event):
    while not stop.is_set():
        await asyncio.sleep(30)
        _flush(out, asset_samples, asset_resolutions)


# --------------------------------------------------------------------------- #
# Monitor: avisa si algún feed RTDS no está llegando
# --------------------------------------------------------------------------- #

async def monitor(states: dict[str, AssetState], stop: asyncio.Event):
    await asyncio.sleep(60)  # dar tiempo a que arranquen los feeds
    while not stop.is_set():
        for name, st in states.items():
            if not st.chainlink_ok:
                print(f"[monitor] ⚠️  {name}: sin ticks Chainlink en 60s — "
                      f"verifica que el símbolo RTDS '{st.cfg.rtds_sym}' existe")
        await asyncio.sleep(60)


# --------------------------------------------------------------------------- #
# Main
# --------------------------------------------------------------------------- #

async def main(a):
    chosen = {n: ASSETS[n] for n in a.assets if n in ASSETS}
    if not chosen:
        print(f"Assets no reconocidos. Disponibles: {list(ASSETS)}")
        return

    print(f"Grabando assets: {list(chosen)}")
    states = {name: AssetState(cfg) for name, cfg in chosen.items()}

    stop  = asyncio.Event()
    sub_q: asyncio.Queue = asyncio.Queue()

    asset_samples     = {n: [] for n in chosen}
    asset_resolutions = {n: [] for n in chosen}

    # samples/resolutions por asset, compartidos con sus respectivos tasks
    def make_sample_list(name):
        return asset_samples[name]
    def make_res_list(name):
        return asset_resolutions[name]

    tasks = [
        asyncio.create_task(binance_feed(states, stop)),
        asyncio.create_task(rtds_feed(states, stop)),
        asyncio.create_task(clob_feed(states, stop, sub_q)),
        asyncio.create_task(monitor(states, stop)),
        asyncio.create_task(flusher(a.out, asset_samples, asset_resolutions, stop)),
    ]
    for name, st in states.items():
        tasks.append(asyncio.create_task(
            window_manager(st, stop, sub_q, asset_resolutions[name])
        ))
        tasks.append(asyncio.create_task(
            sampler(st, stop, asset_samples[name])
        ))

    print("Grabando… Ctrl-C para detener")
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
        _flush(a.out, asset_samples, asset_resolutions)
        # resumen de cobertura RTDS
        for name, st in states.items():
            ok = "✓" if st.chainlink_ok else "✗ SIN TICKS"
            print(f"  {name} Chainlink: {ok}")


# --------------------------------------------------------------------------- #
# Selftest (sin red)
# --------------------------------------------------------------------------- #

def selftest():
    import os
    from collections import deque

    now = int(time.time() * 1000)
    errors = []

    for name, cfg in ASSETS.items():
        st = AssetState(cfg)
        st.now_ms    = now
        st.spot      = 1000.0
        st.chainlink = 999.0
        st.chainlink_ok = True
        st.bids = [(999.5, 8.0), (999.0, 5.0)]
        st.asks = [(1000.5, 2.0), (1001.0, 4.0)]
        st.trades = deque([
            {"ts_ms": now - 3000, "price": 990.0, "qty": 1.5, "buyer_is_maker": False},
            {"ts_ms": now - 1000, "price": 1000.0, "qty": 0.8, "buyer_is_maker": False},
        ])
        close_b = now // 1000 + 20  # 20s from now → secs_left=20, inside [5,60]
        st.window = {
            "window_id": close_b,
            "up_token": "U", "down_token": "D",
            "open_ms": (close_b - 300) * 1000,
            "close_ms": close_b * 1000,
            "price_to_beat": 995.0,
        }
        st.pm = {
            "U": {"best_bid": 0.60, "best_ask": 0.63, "bid_size": 50, "ask_size": 40},
            "D": {"best_bid": 0.37, "best_ask": 0.40, "bid_size": 50, "ask_size": 40},
        }
        row = build_sample(st)
        if row is None:
            errors.append(f"{name}: build_sample devolvió None")
            continue
        missing = [k for k in FEATURE_ORDER if k not in row]
        if missing:
            errors.append(f"{name}: features faltantes {missing}")
            continue
        print(f"  {name}: dist_bps={row['dist_bps']:.2f} "
              f"basis_bps={row['basis_bps']:.3f} "
              f"secs_left={row['secs_left']:.0f} "
              f"pm_up_ask={row['pm_up_ask']}")

    if errors:
        print("\nERRORES:")
        for e in errors:
            print(f"  {e}")
        raise SystemExit(1)

    # verificar escritura a parquet
    os.makedirs("data/selftest", exist_ok=True)
    rows = []
    for name, cfg in ASSETS.items():
        st = AssetState(cfg)
        st.now_ms = now; st.spot = 1000.0; st.chainlink = 999.0
        st.chainlink_ok = True
        st.bids = [(999.5, 8.0)]; st.asks = [(1000.5, 2.0)]
        st.trades = deque([{"ts_ms": now-1000, "price": 1000.0,
                            "qty": 1.0, "buyer_is_maker": False}])
        close_b = now // 1000 + 20
        st.window = {"window_id": close_b, "up_token": "U", "down_token": "D",
                     "open_ms": (close_b-300)*1000, "close_ms": close_b*1000,
                     "price_to_beat": 995.0}
        st.pm = {"U": {"best_bid": 0.60, "best_ask": 0.63, "bid_size": 50, "ask_size": 40},
                 "D": {"best_bid": 0.37, "best_ask": 0.40, "bid_size": 50, "ask_size": 40}}
        row = build_sample(st)
        if row:
            rows.append(row)
    df = pd.DataFrame(rows)
    df.to_parquet("data/selftest/samples.parquet")
    print(f"\nselftest OK — {len(df)} assets, parquet escrito en data/selftest/")
    print(df[["asset"] + FEATURE_ORDER[:4]].to_string(index=False))


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--assets", nargs="+", default=list(ASSETS),
                    choices=list(ASSETS), metavar="ASSET",
                    help=f"assets a grabar (default: todos). Opciones: {list(ASSETS)}")
    ap.add_argument("--out", default="data")
    ap.add_argument("--hours", type=float, default=0.0, help="0 = hasta Ctrl-C")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()
    if args.selftest:
        selftest()
    else:
        asyncio.run(main(args))
