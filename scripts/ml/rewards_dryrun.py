#!/usr/bin/env python3
"""
rewards_dryrun.py — 24h SHADOW Dry Run of LP-rewards maker quoting (read-only).

WHAT THIS CAN VALIDATE: the adverse-selection RISK of resting two-sided quotes on the
target slow reward markets. It models a non-HFT maker that reprices every 60s; whenever
the real book mid drifts past our stale quote before we reprice, we'd be adversely filled,
and we book that loss. On slow markets (politics/finance resolving in months) the hypothesis
is adverse selection is near-zero.

WHAT THIS CANNOT VALIDATE: the actual reward payout. Rewards accrue only to REAL on-chain
resting orders, split across all competing makers (a pool we cannot observe). We report
reward-eligible time and the market's daily_rate so you can sanity-check the upside, but the
real $ requires posting real orders. Treat the reward column as an UPPER-BOUND proxy.

Decision rule after 24h: if adverse-selection cost/day << daily_rate (and eligible-time is
high), the economics favor a small real pilot. If quotes get run over often, it is risky.

Usage:
  ./rewards_dryrun.py --run --hours 24      # loop, log to data/rewards_dryrun.jsonl
  ./rewards_dryrun.py --summarize           # read the log, print the verdict
"""
import argparse, json, os, time, urllib.request, datetime as dt

CLOB = "https://clob.polymarket.com"
LOG = os.path.expanduser('~/.traderclaw/workspace/data/rewards_dryrun.jsonl')
POLL_S = 20            # book poll cadence
REPRICE_S = 60        # how often our (slow, non-HFT) maker re-centers its quote
HALF_BAND_C = 1.0     # we rest 1c inside the eligible band on each side
UA = {'User-Agent': 'trader-claw/rewards-dryrun'}


def http(url):
    return json.load(urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=20))


def pick_targets(n=4, filt=None, min_days=30):
    """Reward-eligible markets with their YES token + params.
    filt: optional substring (question/tags) to target a theme, e.g. 'world cup'.
    Paginates the CLOB sampling-markets so themed markets deeper in the list are found."""
    out = []
    cursor = ''
    for _ in range(6):
        url = f"{CLOB}/sampling-markets" + (f"?next_cursor={cursor}" if cursor else "")
        d = http(url)
        for m in d.get('data', []):
            r = m.get('rewards') or {}
            rate = max([x.get('rewards_daily_rate', 0) for x in (r.get('rates') or [{}])], default=0)
            q = m.get('question', '').lower()
            tags = [t.lower() for t in (m.get('tags') or [])]
            toxic = ('up or down' in q) or ('crypto' in tags) or ('hourly' in tags)
            if rate <= 0 or toxic or not m.get('accepting_orders'):
                continue
            if filt and (filt.lower() not in q and filt.lower() not in ' '.join(tags)
                         and filt.lower() not in (m.get('market_slug', '') or '').lower()):
                continue
            try:
                days = (dt.datetime.fromisoformat(m['end_date_iso'].replace('Z', '+00:00')) - dt.datetime.now(dt.timezone.utc)).days
            except Exception:
                days = -999
            if days < min_days:
                continue
            toks = m.get('tokens') or []
            yes = next((t.get('token_id') for t in toks if t.get('outcome') == 'Yes'), None)
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


def run(hours, filt=None, log=LOG, min_days=30):
    targets = pick_targets(filt=filt, min_days=min_days)
    print(f"Shadow Dry Run on {len(targets)} markets ({filt or 'top'}) for {hours}h. Logging → {log}")
    for t in targets:
        print(f"  rate={t['rate']:.0f}/d days={t['days']} max_spread={t['max_spread']}c min=${t['min_size']} | {t['q']}")
    if not targets:
        print("No matching markets found."); return
    os.makedirs(os.path.dirname(log), exist_ok=True)
    end = time.time() + hours * 3600
    # state per market: our resting quote (bid_px, ask_px) + when posted
    quotes = {t['token']: None for t in targets}
    last_reprice = {t['token']: 0 for t in targets}
    while time.time() < end:
        now = time.time()
        for t in targets:
            tok = t['token']
            bk = book_mid(tok)
            if not bk:
                continue
            band = (t['max_spread'] / 100.0) / 2 - (HALF_BAND_C / 100.0)
            band = max(band, 0.005)
            # (Re)post our quote every REPRICE_S, centered on current mid.
            if quotes[tok] is None or now - last_reprice[tok] >= REPRICE_S:
                quotes[tok] = {'bid': bk['mid'] - band, 'ask': bk['mid'] + band, 'ref_mid': bk['mid']}
                last_reprice[tok] = now
            qd = quotes[tok]
            # Adverse fill detection: if the mid drifted past our STALE quote, we'd be filled
            # on the wrong side. Loss ≈ how far past our quote the new mid is.
            adverse = 0.0; filled = ''
            if bk['mid'] <= qd['bid']:           # market fell to/below our bid → we bought too high
                adverse = qd['bid'] - bk['mid']; filled = 'bid'
            elif bk['mid'] >= qd['ask']:          # market rose to/above our ask → we sold too low
                adverse = bk['mid'] - qd['ask']; filled = 'ask'
            eligible = abs(bk['mid'] - qd['ref_mid']) <= (t['max_spread'] / 100.0)
            row = {'ts': int(now), 'token': tok[:12], 'q': t['q'], 'rate': t['rate'],
                   'mid': round(bk['mid'], 4), 'spread_c': round(bk['spread'] * 100, 2),
                   'eligible': eligible, 'adverse_c': round(adverse * 100, 3), 'filled': filled,
                   'min_size': t['min_size']}
            with open(log, 'a') as f:
                f.write(json.dumps(row) + '\n')
        time.sleep(POLL_S)
    print("Dry Run complete.")


def summarize(log=LOG):
    if not os.path.exists(log):
        print("No log yet."); return
    rows = [json.loads(l) for l in open(log)]
    if not rows:
        print("Empty log."); return
    span_h = (rows[-1]['ts'] - rows[0]['ts']) / 3600
    by = {}
    for r in rows:
        by.setdefault(r['q'], []).append(r)
    print(f"\n{'='*78}\nREWARDS DRY RUN — {len(rows)} samples over {span_h:.1f}h\n{'='*78}")
    print(f"{'market':40} {'rate/d':>7} {'elig%':>6} {'advFills':>8} {'advCost/d':>10}")
    for q, rs in by.items():
        n = len(rs)
        elig = sum(1 for r in rs if r['eligible']) / n * 100
        fills = [r for r in rs if r['filled']]
        # adverse cost per day: each adverse sample ≈ a fill event at min_size shares
        msize = rs[0]['min_size'] or 200
        adv_per_sample_usd = sum(r['adverse_c'] / 100 * (msize) for r in fills)
        adv_per_day = adv_per_sample_usd / max(span_h, 0.01) * 24
        rate = rs[0]['rate']
        flag = ''
        if span_h >= 1:
            flag = '  ✓ rewards>>risk' if adv_per_day < rate * 0.3 else ('  ✗ risky' if adv_per_day > rate else '')
        print(f"{q[:40]:40} {rate:>7.0f} {elig:>5.0f}% {len(fills):>8} {adv_per_day:>9.2f}${flag}")
    print("\nadvCost/d = simulated adverse-selection $/day at min_size. Compare to rate/d (reward UPPER bound).")
    print("Verdict: if advCost/d << rate/d AND elig% high → favorable; do a small REAL pilot to confirm payout.")


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--run', action='store_true')
    ap.add_argument('--summarize', action='store_true')
    ap.add_argument('--hours', type=float, default=24)
    ap.add_argument('--filter', default=None, help="theme substring, e.g. 'world cup'")
    ap.add_argument('--log', default=LOG, help='log file path')
    ap.add_argument('--min-days', type=int, default=30)
    args = ap.parse_args()
    if args.run:
        run(args.hours, filt=args.filter, log=args.log, min_days=args.min_days)
    elif args.summarize:
        summarize(log=args.log)
    else:
        ap.print_help()
