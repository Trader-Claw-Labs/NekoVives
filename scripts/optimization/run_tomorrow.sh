#!/bin/bash
# ────────────────────────────────────────────────────────────────────────────
# Optimización OOS de drift_v3_regime BTC — corre esto mañana de un solo tirón.
#
# Usa el optimizer con TRAIN/TEST split sobre los datos con resolución oficial
# Polymarket que ya tienes. Sweep dos parámetros que SÍ afectan el resultado en
# polymarket_binary mode: min_entry_price y sizing_value.
#
# Uso:
#   ./run_tomorrow.sh
#
# El script:
#   1. Verifica que el daemon esté vivo (lo arranca si no lo está)
#   2. Re-pairing si el token expiró
#   3. Corre 2 sweeps (min_entry_price, sizing_value) con TRAIN/TEST split
#   4. Imprime un resumen final con la recomendación
#
# Tiempo total: ~10-15 min.
# ────────────────────────────────────────────────────────────────────────────

set -u
PORT=42617
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TOKEN_FILE=/tmp/bt_token.txt
OUT_LOG=/tmp/optimize_tomorrow.log
> "$OUT_LOG"

cd "$REPO_DIR"

# ── 1. Verify daemon ──────────────────────────────────────────────────────
echo "[$(date +%H:%M:%S)] Verificando daemon..." | tee -a "$OUT_LOG"
if ! curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:$PORT/health" | grep -q 200; then
  echo "  Daemon no responde. Arrancando..." | tee -a "$OUT_LOG"
  nohup ./target/release/trader-claw daemon > /tmp/daemon.log 2>&1 &
  sleep 12
fi
echo "  ✓ Daemon vivo" | tee -a "$OUT_LOG"

# ── 2. Re-pair if needed ──────────────────────────────────────────────────
TOKEN=""
[ -f "$TOKEN_FILE" ] && TOKEN=$(cat "$TOKEN_FILE")

# Test current token
test_token() {
  curl -s "http://127.0.0.1:$PORT/api/backtest/tick-slugs" \
    -H "Authorization: Bearer $1" 2>&1 | grep -q '"slugs"'
}

if ! test_token "$TOKEN"; then
  echo "[$(date +%H:%M:%S)] Token inválido, re-pairing..." | tee -a "$OUT_LOG"
  CODE=$(grep -oE "X-Pairing-Code: [0-9]+" /tmp/daemon.log | tail -1 | grep -oE "[0-9]+")
  if [ -z "$CODE" ]; then
    echo "  ✗ No se encontró pairing code en /tmp/daemon.log" | tee -a "$OUT_LOG"
    echo "  Mira el log con: tail /tmp/daemon.log | grep PAIRING" | tee -a "$OUT_LOG"
    exit 1
  fi
  TOKEN=$(curl -s -X POST "http://127.0.0.1:$PORT/pair" \
    -H "X-Pairing-Code: $CODE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('token',''))")
  echo "$TOKEN" > "$TOKEN_FILE"
  echo "  ✓ Token nuevo: ${TOKEN:0:16}..." | tee -a "$OUT_LOG"
fi
echo "  ✓ Auth OK" | tee -a "$OUT_LOG"

# ── 3. Run sweeps ─────────────────────────────────────────────────────────
SCRIPT="polymarket_btc_updown_5m_drift_v3_regime.rhai"
SERIES="btc_5m"
SYMBOL="BTCUSDT"
FROM="2026-04-23"
TO="2026-05-25"

OPTIMIZER="python3 scripts/optimization/optimize_runner.py"

echo "" | tee -a "$OUT_LOG"
echo "════════════════════════════════════════════════════════════════════════" | tee -a "$OUT_LOG"
echo "SWEEP 1: min_entry_price (skip extreme long-shot bets)" | tee -a "$OUT_LOG"
echo "════════════════════════════════════════════════════════════════════════" | tee -a "$OUT_LOG"
$OPTIMIZER \
  --script "$SCRIPT" --series "$SERIES" --symbol "$SYMBOL" \
  --from "$FROM" --to "$TO" \
  --param min_entry_price \
  --grid 0.10,0.15,0.20,0.25,0.30 \
  --baseline-params "min_entry_price=0.15" \
  --min-trades 100 2>&1 | tee -a "$OUT_LOG"

echo "" | tee -a "$OUT_LOG"
echo "════════════════════════════════════════════════════════════════════════" | tee -a "$OUT_LOG"
echo "SWEEP 2: sizing_value (% of balance per bet)" | tee -a "$OUT_LOG"
echo "════════════════════════════════════════════════════════════════════════" | tee -a "$OUT_LOG"
$OPTIMIZER \
  --script "$SCRIPT" --series "$SERIES" --symbol "$SYMBOL" \
  --from "$FROM" --to "$TO" \
  --param sizing_value \
  --grid 2,3,5,7,10 \
  --baseline-params "min_entry_price=0.15,sizing_value=5" \
  --min-trades 100 2>&1 | tee -a "$OUT_LOG"

# ── 4. Final summary ──────────────────────────────────────────────────────
echo "" | tee -a "$OUT_LOG"
echo "════════════════════════════════════════════════════════════════════════" | tee -a "$OUT_LOG"
echo "RESUMEN" | tee -a "$OUT_LOG"
echo "════════════════════════════════════════════════════════════════════════" | tee -a "$OUT_LOG"
echo "" | tee -a "$OUT_LOG"
echo "Log completo guardado en: $OUT_LOG" | tee -a "$OUT_LOG"
echo "" | tee -a "$OUT_LOG"
echo "Lee los VEREDICTOS de cada sweep:" | tee -a "$OUT_LOG"
grep -A1 "VERDICT\|ACCEPT\|REJECT\|MARGINAL" "$OUT_LOG" | tail -20
echo "" | tee -a "$OUT_LOG"
echo "Si algún sweep dice 'ACCEPT', aplica el cambio con:" | tee -a "$OUT_LOG"
echo "  TOKEN=\$(cat $TOKEN_FILE)" | tee -a "$OUT_LOG"
echo "  curl -X PATCH http://127.0.0.1:$PORT/api/live/strategies/<RUNNER_ID> \\" | tee -a "$OUT_LOG"
echo "    -H \"Authorization: Bearer \$TOKEN\" -H 'Content-Type: application/json' \\" | tee -a "$OUT_LOG"
echo "    -d '{\"min_entry_price\": <valor_recomendado>}'" | tee -a "$OUT_LOG"
echo "" | tee -a "$OUT_LOG"
echo "Si todos dicen 'REJECT' o 'MARGINAL', no apliques cambios." | tee -a "$OUT_LOG"
echo "Significa que la config actual de drift_v3_regime ya está cerca del óptimo OOS." | tee -a "$OUT_LOG"
