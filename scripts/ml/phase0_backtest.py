#!/usr/bin/env python3
"""
phase0_backtest.py — Fase 0 del VPS Execution Plan (docs/VPS_EXECUTION_PLAN.md).

"Backtest de la verdad": ambas ideas direccionales se miden sobre los 33-41 días de
ticks 1Hz REALES (binance_price + book + window_yes_won oficial) ANTES de arriesgar $1.

  0a — Señal "Binance imbalance → Polymarket" (idea #1).
       Proxy implementable a resolución 1Hz: movimiento brusco de Binance (>N bps en 2s).
       La orden se ENVÍA al ver la señal y LLEGA con latencia simulada (110-220ms desde
       la VPS) → el fill usa el ask del PRIMER tick posterior a ts_señal + latencia
       (con datos 1Hz eso es el tick siguiente: cota pesimista-honesta).
       Se mantiene hasta resolución OFICIAL (window_yes_won) — sin asumir exit liquidity.
       GATE 0a: EV/trade > 0 tras fee 1.8%×p(1-p), con n ≥ 500.

  0b — Replay fiel de clob_1hz_late_certainty.rhai (idea #2) con resolución oficial,
       mismo fill con latencia, y el edge_validator de 3 legs encima.
       GATE 0b: PASS en los 3 legs (CI>0, random-side null, shuffled-outcome null).

  Si AMBOS fallan → NO se despliega trading direccional; solo rewards_maker (Fase 1).

Requiere los ticks locales en ~/.traderclaw/workspace/data/ticks/<slug>/*.jsonl
con window_yes_won backfilleado (tools/orderbook_parser.py backfill-resolutions).

Usage:
  ./phase0_backtest.py                            # 0a + 0b sobre btc_5m, todos los días
  ./phase0_backtest.py --phase 0a --slug btc_5m --slug eth_5m --move-bps 15
  ./phase0_backtest.py --phase 0b --latency-ms 220
"""
import argparse, csv, glob, json, os, sys
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge_validator import validate, ev_per_trade  # noqa: E402

TICKS = os.environ.get('NV_TICKS_DIR') or os.path.expanduser('~/.traderclaw/workspace/data/ticks')
CRYPTO_FEE = 0.018


# ── Tick loading ────────────────────────────────────────────────────────────────
def load_windows(slug, days=0):
    """Load ticks grouped by window_ts. Returns {window_ts: {'ticks': [...], 'won': bool}}.
    Only windows with OFFICIAL resolution (window_yes_won not null) are kept — that is
    the whole point of Fase 0: no Binance-provisional, no synthetic prices."""
    files = sorted(glob.glob(f'{TICKS}/{slug}/*.jsonl'))
    if days > 0:
        files = files[-days:]
    if not files:
        print(f"  [{slug}] no tick files under {TICKS}/{slug}/ — run the ingest first.")
        return {}
    windows, dropped = {}, 0
    for f in files:
        for line in open(f):
            try:
                t = json.loads(line)
            except Exception:
                continue
            w = t.get('window_ts', 0)
            if not w:
                continue
            windows.setdefault(w, []).append(t)
    out = {}
    for w, ticks in windows.items():
        ticks.sort(key=lambda t: t.get('ts_ms', 0))
        won = ticks[-1].get('window_yes_won')
        for t in ticks:  # any tick may carry it
            if t.get('window_yes_won') is not None:
                won = t['window_yes_won']
                break
        if won is None:
            dropped += 1
            continue
        out[w] = {'ticks': ticks, 'yes_won': bool(won)}
    print(f"  [{slug}] {len(files)} files, {len(out)} windows with OFFICIAL resolution "
          f"({dropped} dropped without window_yes_won)")
    return out


def fill_after_latency(ticks, i, latency_ms):
    """Index of the first tick at ts >= ticks[i].ts + latency_ms (the simulated fill).
    With 1Hz data and 110-220ms latency this is the next tick — pessimistic by ~4×,
    which is the honest direction. Returns None if the window ends first."""
    t0 = ticks[i].get('ts_ms', 0)
    for j in range(i + 1, len(ticks)):
        if ticks[j].get('ts_ms', 0) >= t0 + latency_ms:
            return j
    return None


# ── Fase 0a — Binance move → Polymarket taker, latency-delayed, hold to resolution ──
def run_0a(slugs, days, move_bps, latency_ms, max_secs_left, min_secs_left):
    print(f"\n{'='*74}\nFASE 0a — Binance move >{move_bps}bps/2s → taker, latencia {latency_ms}ms, "
          f"hold a resolución oficial\n{'='*74}")
    entries, wons, slugs_of = [], [], []
    for slug in slugs:
        windows = load_windows(slug, days)
        n_slug = 0
        for w, wd in sorted(windows.items()):
            ticks = wd['ticks']
            for i in range(2, len(ticks)):
                a, b = ticks[i - 2], ticks[i]
                if b.get('ts_ms', 0) - a.get('ts_ms', 0) > 3000:
                    continue  # gap — not a contiguous 2s move
                bp0, bp1 = a.get('binance_price', 0), b.get('binance_price', 0)
                if bp0 <= 0 or bp1 <= 0:
                    continue
                secs_left = b.get('window_secs_left', 0)
                if secs_left > max_secs_left or secs_left <= min_secs_left:
                    continue
                ret_bps = (bp1 - bp0) / bp0 * 1e4
                if abs(ret_bps) < move_bps:
                    continue
                j = fill_after_latency(ticks, i, latency_ms)
                if j is None:
                    break
                up = ret_bps > 0
                entry = ticks[j].get('yes_ask', 0) if up else ticks[j].get('no_ask', 0)
                if not (0.03 < entry < 0.97):
                    break  # extreme price — min_entry_price guard, skip this window
                entries.append(entry)
                wons.append(1 if (wd['yes_won'] == up) else 0)
                slugs_of.append(slug)
                n_slug += 1
                break  # one trade per window — keeps trades independent
        print(f"  [{slug}] {n_slug} trades")

    entries, wons = np.array(entries), np.array(wons)
    n = len(entries)
    if n == 0:
        print("  Sin trades — sube --days o baja --move-bps.")
        return {'n': 0, 'gate': False}
    ev = ev_per_trade(entries, wons)
    print(f"\n  POOLED: n={n}  WR={wons.mean()*100:.1f}%  avg_entry={entries.mean():.3f}  "
          f"EV/trade={ev.mean()*100:+.2f}%  P&L/$1stake={ev.sum():+.2f}")
    res = validate(entries, wons)
    gate = ev.mean() > 0 and n >= 500
    print(f"  GATE 0a (EV/trade > 0 AND n >= 500): {'✅ PASS' if gate else '❌ FAIL'}"
          f"  (EV={ev.mean()*100:+.2f}%, n={n})")
    if gate and not res['edge']:
        print("  ⚠ Gate nominal PASS pero el edge_validator NO confirma — tratar como FAIL.")
        gate = False
    export_csv('phase0_0a_trades.csv', entries, wons, slugs_of)
    return {'n': n, 'ev': float(ev.mean()), 'gate': gate}


# ── Fase 0b — late_certainty replay (port fiel de clob_1hz_late_certainty.rhai) ─────
def run_0b(slugs, days, latency_ms):
    print(f"\n{'='*74}\nFASE 0b — late_certainty replay (setups A-D), latencia {latency_ms}ms, "
          f"resolución oficial + edge_validator 3 legs\n{'='*74}")
    entries, wons, slugs_of, setups = [], [], [], []
    for slug in slugs:
        windows = load_windows(slug, days)
        n_slug = 0
        for w, wd in sorted(windows.items()):
            ticks = wd['ticks']
            wopen = next((t['binance_price'] for t in ticks if t.get('binance_price', 0) > 0), 0)
            if wopen <= 0:
                continue
            for i, t in enumerate(ticks):
                bp = t.get('binance_price', 0)
                if bp <= 0:
                    continue
                secs_left = t.get('window_secs_left', 0)
                # Entry window: last 35s, excluding final 5s (same as the rhai script)
                if secs_left > 35 or secs_left <= 5:
                    continue
                ya, yb = t.get('yes_ask', 0), t.get('yes_bid', 0)
                na = t.get('no_ask', 0)
                if ya <= 0 or yb <= 0 or na <= 0:
                    continue
                if (ya - yb) * 100.0 > 4.0:        # spread gate
                    continue
                if ya < 0.03 or na < 0.03:          # extreme-price guard
                    continue
                ymid = (ya + yb) / 2
                change_pct = (bp - wopen) / wopen * 100.0
                side = None
                setup = None
                if change_pct > 0.04 and ya < 0.80:
                    side, setup = 'yes', 'A'
                elif change_pct < -0.04 and na < 0.80:
                    side, setup = 'no', 'B'
                elif ymid > 0.75 and change_pct < -0.02 and na < 0.30:
                    side, setup = 'no', 'C'
                elif ymid < 0.25 and change_pct > 0.02 and ya < 0.30:
                    side, setup = 'yes', 'D'
                if side is None:
                    continue
                j = fill_after_latency(ticks, i, latency_ms)
                if j is None:
                    break
                entry = ticks[j].get('yes_ask', 0) if side == 'yes' else ticks[j].get('no_ask', 0)
                if not (0.03 < entry < 0.97):
                    break
                entries.append(entry)
                wons.append(1 if (wd['yes_won'] == (side == 'yes')) else 0)
                slugs_of.append(slug)
                setups.append(setup)
                n_slug += 1
                break  # one position per window (ctx.position != 0 → return)
        print(f"  [{slug}] {n_slug} trades")

    entries, wons = np.array(entries), np.array(wons)
    n = len(entries)
    if n == 0:
        print("  Sin trades."); return {'n': 0, 'gate': False}
    for s in 'ABCD':
        m = np.array([x == s for x in setups])
        if m.sum():
            ev_s = ev_per_trade(entries[m], wons[m])
            print(f"  setup {s}: n={m.sum():4d}  WR={wons[m].mean()*100:5.1f}%  "
                  f"EV/trade={ev_s.mean()*100:+.2f}%")
    ev = ev_per_trade(entries, wons)
    print(f"\n  POOLED: n={n}  WR={wons.mean()*100:.1f}%  avg_entry={entries.mean():.3f}  "
          f"EV/trade={ev.mean()*100:+.2f}%  P&L/$1stake={ev.sum():+.2f}")
    res = validate(entries, wons)
    print(f"  GATE 0b (3 legs PASS): {'✅ PASS' if res['edge'] else '❌ FAIL'}  legs={res['legs']}")
    export_csv('phase0_0b_late_certainty_trades.csv', entries, wons, slugs_of, setups)
    return {'n': n, 'ev': float(ev.mean()), 'gate': res['edge']}


def export_csv(name, entries, wons, slugs_of, setups=None):
    path = os.path.join('/tmp', name)
    with open(path, 'w', newline='') as f:
        wcsv = csv.writer(f)
        wcsv.writerow(['entry_price', 'won', 'slug'] + (['setup'] if setups else []))
        for k in range(len(entries)):
            row = [f'{entries[k]:.4f}', int(wons[k]), slugs_of[k]]
            if setups:
                row.append(setups[k])
            wcsv.writerow(row)
    print(f"  trades exportados → {path} (re-validar: ./edge_validator.py --source csv --csv {path})")


if __name__ == '__main__':
    ap = argparse.ArgumentParser(description='Fase 0 — backtest de la verdad (VPS plan)')
    ap.add_argument('--phase', choices=['0a', '0b', 'all'], default='all')
    ap.add_argument('--slug', action='append', default=[],
                    help='tick slug(s); default btc_5m (repeatable)')
    ap.add_argument('--days', type=int, default=0, help='0 = todos los días disponibles')
    ap.add_argument('--move-bps', type=float, default=15.0, help='umbral de movimiento Binance en 2s (0a)')
    ap.add_argument('--latency-ms', type=int, default=220,
                    help='latencia simulada señal→fill (110ms lectura + 110ms orden desde la VPS)')
    ap.add_argument('--max-secs-left', type=int, default=290, help='0a: no operar antes de este punto')
    ap.add_argument('--min-secs-left', type=int, default=5, help='0a: no operar en los últimos N s (no-fill)')
    args = ap.parse_args()
    slugs = args.slug or ['btc_5m']

    r0a = r0b = None
    if args.phase in ('0a', 'all'):
        r0a = run_0a(slugs, args.days, args.move_bps, args.latency_ms,
                     args.max_secs_left, args.min_secs_left)
    if args.phase in ('0b', 'all'):
        r0b = run_0b(slugs, args.days, args.latency_ms)

    print(f"\n{'='*74}\nDECISIÓN FASE 0 (regla del plan)\n{'='*74}")
    if r0a is not None:
        print(f"  0a (imbalance taker): {'✅ PASS' if r0a['gate'] else '❌ FAIL'} (n={r0a['n']})")
    if r0b is not None:
        print(f"  0b (late_certainty) : {'✅ PASS' if r0b['gate'] else '❌ FAIL'} (n={r0b['n']})")
    any_pass = (r0a and r0a['gate']) or (r0b and r0b['gate'])
    if any_pass:
        print("  → Hay luz verde direccional: ese script va en DRY RUN en la VPS (Fase 1), no live.")
    else:
        print("  → AMBAS fallan: NO desplegar trading direccional. Fase 1 = solo rewards_maker.")
