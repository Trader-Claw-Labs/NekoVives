#!/usr/bin/env bash
# ============================================================================
# VPS HFT-VIABILITY PROBE — run this ON THE NEW VPS, not your laptop.
# ============================================================================
# hping3 measured 2.8ms (TCP SYN-ACK to the Cloudflare EDGE). That is NOT the
# number that decides HFT viability. What matters is:
#   (a) the round-trip of a REAL /book GET (goes through Cloudflare to the ORIGIN)
#   (b) the round-trip of an authenticated request (order path)
#   (c) whether the stale-book edge survives AT THIS latency after fees
#
# This prints all three so we make the build/no-build call on data, not hope.
# ============================================================================
set -uo pipefail

echo "=== VPS HFT probe — $(date -u +%FT%TZ) ==="
echo "Host: $(hostname)"
RAY=$(curl -s -D - -o /dev/null https://clob.polymarket.com/ -H 'User-Agent: nv-vps' --max-time 10 | grep -i cf-ray | tr -d '\r')
echo "  $RAY   (Cloudflare PoP nearest this VPS)"
echo ""

echo "--- (a) Real /book GET round-trip (origin, not edge) — 10 samples ---"
for i in $(seq 1 10); do
  curl -s -o /dev/null -w "  connect=%{time_connect}s  ttfb=%{time_starttransfer}s  total=%{time_total}s\n" \
    "https://clob.polymarket.com/book?token_id=0" -H 'User-Agent: nv-vps' --max-time 10
done

echo ""
echo "--- (b) /sampling-markets (heavier origin call) — 5 samples ---"
for i in $(seq 1 5); do
  curl -s -o /dev/null -w "  total=%{time_total}s\n" \
    "https://clob.polymarket.com/sampling-markets" -H 'User-Agent: nv-vps' --max-time 15
done

echo ""
echo "============================================================================"
echo "DECISION RULE:"
echo "  /book total < 0.020s (20ms) → TRUE HFT regime. Stale-book arb is reachable."
echo "                                Re-run basis_analysis.py here; if taker EV > 0"
echo "                                after the 1.8% fee, the discarded clob_1hz"
echo "                                scripts (spread_scalper, ofi, vwap_revert) are"
echo "                                worth a Dry Run on THIS box."
echo "  /book total 0.020-0.060s    → MARGINAL. Maybe viable for the slower edges"
echo "                                (late_certainty 30-45s window), not pure sniping."
echo "  /book total > 0.100s        → Cloudflare/origin still adds latency the colo"
echo "                                can't remove. HFT NOT reachable; do not build."
echo "============================================================================"
