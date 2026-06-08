#!/usr/bin/env bash
# Measure real RTT to Polymarket's CLOB from wherever this runs.
# Compare the "total" number against the home-network baseline (~200ms from QRO).
# If a us-east-1 box gets total < ~50ms, Polymarket's origin is near us-east-1 and
# the HFT latency-arb regime is reachable; if it stays ~200ms, it is not.
set -uo pipefail

echo "=== Polymarket CLOB latency probe — $(date -u +%FT%TZ) ==="
# Which Cloudflare PoP are we hitting (airport code in cf-ray)?
RAY=$(curl -s -D - -o /dev/null https://clob.polymarket.com/ -H 'User-Agent: nv-probe' --max-time 10 | grep -i cf-ray | tr -d '\r')
echo "  $RAY   (the suffix is the Cloudflare PoP nearest THIS host)"
echo ""

probe() {
  local host=$1 path=${2:-/}
  echo "--- https://$host$path"
  for i in 1 2 3 4 5; do
    curl -s -o /dev/null -w "  connect=%{time_connect}s  ttfb=%{time_starttransfer}s  total=%{time_total}s\n" \
      "https://$host$path" -H 'User-Agent: nv-probe' --max-time 10
  done
}

# A real API call (DYNAMIC, hits origin) is the meaningful number — not the root.
probe clob.polymarket.com "/sampling-markets"
probe clob.polymarket.com "/book?token_id=0"
echo ""
echo "Interpretation: 'total' on /sampling-markets is the round-trip to the ORIGIN."
echo "  ~200ms  → far from origin (e.g. home in MX). HFT NOT reachable here."
echo "  <50ms   → near origin (us-east-1). Re-run scripts/ml/basis_analysis.py to"
echo "            check if the taker EV turns positive at this latency before building."
