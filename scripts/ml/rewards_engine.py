#!/usr/bin/env python3
"""
rewards_engine.py — Market-selection + timing brain for LP-rewards maker quoting.

Two jobs:
  1. SELECT the most convenient markets (reward per unit of noise), continuously.
  2. TIME each market: emit QUOTE when the book is calm, PAUSE during high-volatility /
     noisy periods so resting quotes don't get picked off (e.g. a World Cup market during
     a live match, a Fed market on announcement, any spread blow-out or fast mid move).

This is the advisory/validation engine — it runs read-only, polls real CLOB books, logs a
per-market QUOTE/PAUSE decision every tick, and its --summarize PROVES the gate works by
comparing simulated adverse-selection cost WITH the timing gate vs WITHOUT it. Once proven,
the same logic embeds into the Rust quoting runner (the gate = "pause during noise").

Timing signals (rolling window, from book polling — no external feed needed):
  - mid_vol_c   : stdev of per-sample mid moves over the window (¢). Core noise measure.
  - velocity_c  : |mid_now − mid_window_ago| (¢). Trend/jump detector.
  - spread_c    : current spread (¢). Wide spread = thin/uncertain book.
State = QUOTE iff mid_vol_c<=vol_thresh AND velocity_c<=vel_thresh AND spread_c<=max_spread.
Else PAUSE. Convenience score = daily_rate / (1 + k·mid_vol_c).

Usage:
  ./rewards_engine.py --run --hours 24 [--filter "world cup"] [--log <path>]
  ./rewards_engine.py --summarize [--log <path>]
"""
import argparse, json, os, time, urllib.request, datetime as dt
from collections import deque, defaultdict
import statistics as st

CLOB = "https://clob.polymarket.com"
LOG = os.path.expanduser('~/.traderclaw/workspace/data/rewards_engine.jsonl')
UA = {'User-Agent': 'trader-claw/rewards-engine'}

POLL_S = 15
WINDOW_S = 300            # 5-min rolling window for vol/velocity
VOL_THRESH_C = 0.6        # stdev of mid moves (¢) above which the market is "noisy"
VEL_THRESH_C = 2.0        # |mid move over window| (¢) above which we pause (trend/jump)
HALF_BAND_C = 1.0         # where our stale quote rests, per side (¢ inside the band)
K_NOISE = 1.5             # convenience-score noise penalty


def http(u):
    return json.load(urllib.request.urlopen(urllib.request.Request(u, headers=UA), timeout=20))


def pick_targets(n=4, filt=None, min_days=20):
    out, cursor = [], ''
    for _ in range(6):
        d = http(f"{CLOB}/sampling-markets" + (f"?next_cursor={cursor}" if cursor else ""))
        for m in d.get('data', []):
            r = m.get('rewards') or {}
            rate = max([x.get('rewards_daily_rate', 0) for x in (r.get('rates') or [{}])], default=0)
            q = m.get('question', '').lower()
            tags = [t.lower() for t in (m.get('tags') or [])]
            if rate <= 0 or not m.get('accepting_orders'):
                continue
            if ('up or down' in q) or ('crypto' in tags) or ('hourly' in tags):
                continue
            if filt and filt.lower() not in q and filt.lower() not in ' '.join(tags) \
               and filt.lower() not in (m.get('market_slug', '') or '').lower():
                continue
            try:
                days = (dt.datetime.fromisoformat(m['end_date_iso'].replace('Z', '+00:00')) - dt.datetime.now(dt.timezone.utc)).days
            except Exception:
                days = -999
            if days < min_days:
                continue
            yes = next((t.get('token_id') for t in (m.get('tokens') or []) if t.get('outcome') == 'Yes'), None)
            if not yes:
                continue
            out.append({'q': m.get('question', '')[:46], 'token': yes, 'rate': rate,
                        'max_spread': r.get('max_spread', 3.5), 'min_size': r.get('min_size', 0), 'days': days})
        cursor = d.get('next_cursor', '')
        if not cursor or cursor == 'LTE=':
            break
    out.sort(key=lambda x: -x['rate'])
    return out[:n]


def book_mid(token):
    try:
        b = http(f"{CLOB}/book?token_id={token}")
        bids, asks = b.get('bids', []), b.get('asks', [])
        if not bids or not asks:
            return None
        bb, ba = float(bids[-1]['price']), float(asks[-1]['price'])
        return {'bid': bb, 'ask': ba, 'mid': (bb + ba) / 2, 'spread': ba - bb}
    except Exception:
        return None


def run(hours, filt=None, log=LOG, min_days=20):
    targets = pick_targets(filt=filt, min_days=min_days)
    print(f"Rewards engine on {len(targets)} markets ({filt or 'top'}) for {hours}h → {log}")
    for t in targets:
        print(f"  rate={t['rate']:.0f}/d days={t['days']} max_spread={t['max_spread']}c min=${t['min_size']} | {t['q']}")
    if not targets:
        print("No matching markets."); return
    os.makedirs(os.path.dirname(log), exist_ok=True)

    hist = {t['token']: deque() for t in targets}   # (ts, mid)
    quote = {t['token']: None for t in targets}      # stale resting quote + post ts
    last_reprice = {t['token']: 0 for t in targets}
    end = time.time() + hours * 3600

    while time.time() < end:
        now = time.time()
        for t in targets:
            tok = t['token']
            bk = book_mid(tok)
            if not bk:
                continue
            h = hist[tok]
            h.append((now, bk['mid']))
            while h and now - h[0][0] > WINDOW_S:
                h.popleft()

            # Noise signals over the window
            mids = [m for _, m in h]
            moves = [abs(mids[i] - mids[i - 1]) for i in range(1, len(mids))]
            mid_vol_c = (st.pstdev(moves) * 100) if len(moves) >= 3 else 0.0
            velocity_c = abs(mids[-1] - mids[0]) * 100 if len(mids) >= 2 else 0.0
            spread_c = bk['spread'] * 100

            quotable = (mid_vol_c <= VOL_THRESH_C and velocity_c <= VEL_THRESH_C
                        and spread_c <= t['max_spread'])
            reason = ('ok' if quotable else
                      ('vol' if mid_vol_c > VOL_THRESH_C else
                       'trend' if velocity_c > VEL_THRESH_C else 'spread'))
            score = t['rate'] / (1.0 + K_NOISE * mid_vol_c)

            # Stale-quote adverse model: reprice only every WINDOW (slow maker). If the mid
            # drifts past our resting quote, we'd be adversely filled. We record it whether
            # or not the gate would have paused — summarize compares gated vs ungated.
            band = max((t['max_spread'] / 100.0) / 2 - HALF_BAND_C / 100.0, 0.005)
            if quote[tok] is None or now - last_reprice[tok] >= WINDOW_S:
                quote[tok] = {'bid': bk['mid'] - band, 'ask': bk['mid'] + band}
                last_reprice[tok] = now
            q = quote[tok]
            adverse_c = 0.0
            if bk['mid'] <= q['bid']:
                adverse_c = (q['bid'] - bk['mid']) * 100
            elif bk['mid'] >= q['ask']:
                adverse_c = (bk['mid'] - q['ask']) * 100

            row = {'ts': int(now), 'q': t['q'], 'rate': t['rate'], 'min_size': t['min_size'],
                   'mid': round(bk['mid'], 4), 'spread_c': round(spread_c, 2),
                   'mid_vol_c': round(mid_vol_c, 3), 'velocity_c': round(velocity_c, 2),
                   'quotable': quotable, 'reason': reason, 'score': round(score, 2),
                   'adverse_c': round(adverse_c, 3)}
            with open(log, 'a') as f:
                f.write(json.dumps(row) + '\n')
        time.sleep(POLL_S)
    print("Engine run complete.")


def summarize(log=LOG):
    if not os.path.exists(log):
        print("No log yet."); return
    rows = [json.loads(l) for l in open(log)]
    if not rows:
        print("Empty log."); return
    span_h = (rows[-1]['ts'] - rows[0]['ts']) / 3600
    by = defaultdict(list)
    for r in rows:
        by[r['q']].append(r)

    print(f"\n{'='*92}\nREWARDS ENGINE — {len(rows)} samples / {span_h:.1f}h · convenience + timing gate\n{'='*92}")
    print(f"{'market':40} {'score':>6} {'quot%':>6} {'advUNGATED/d':>13} {'advGATED/d':>11} {'gate cut':>9}")
    for q, rs in by.items():
        n = len(rs)
        msize = rs[0]['min_size'] or 200
        quot = sum(1 for r in rs if r['quotable']) / n * 100
        # ungated: all adverse fills. gated: only fills while quotable (the gate would have
        # pulled quotes otherwise). Cost/day at min_size shares.
        adv_ung = sum(r['adverse_c'] / 100 * msize for r in rs) / max(span_h, .01) * 24
        adv_gat = sum(r['adverse_c'] / 100 * msize for r in rs if r['quotable']) / max(span_h, .01) * 24
        cut = (1 - adv_gat / adv_ung) * 100 if adv_ung > 0 else 0
        avg_score = st.mean(r['score'] for r in rs)
        print(f"{q[:40]:40} {avg_score:>6.0f} {quot:>5.0f}% {adv_ung:>12.2f}$ {adv_gat:>10.2f}$ {cut:>7.0f}%")
    print("\nscore = reward/noise (rank markets by this). quot% = time the gate would quote.")
    print("advUNGATED/d = adverse cost quoting 24/7. advGATED/d = adverse cost only when calm.")
    print("gate cut = % of adverse cost the timing gate avoids. High cut + high quot% = the gate works.")
    # Noise spike inspection (catches match windows even if the average looks calm)
    print(f"\nWorst noise spikes (top mid_vol samples — likely matches/news):")
    for r in sorted(rows, key=lambda x: -x['mid_vol_c'])[:5]:
        ts = dt.datetime.fromtimestamp(r['ts']).strftime('%m-%d %H:%M')
        print(f"  {ts}  vol={r['mid_vol_c']:.2f}c vel={r['velocity_c']:.1f}c spread={r['spread_c']:.1f}c "
              f"[{r['reason']}] {r['q'][:34]}")


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--run', action='store_true')
    ap.add_argument('--summarize', action='store_true')
    ap.add_argument('--hours', type=float, default=24)
    ap.add_argument('--filter', default=None)
    ap.add_argument('--log', default=LOG)
    ap.add_argument('--min-days', type=int, default=20)
    args = ap.parse_args()
    if args.run:
        run(args.hours, filt=args.filter, log=args.log, min_days=args.min_days)
    elif args.summarize:
        summarize(log=args.log)
    else:
        ap.print_help()
