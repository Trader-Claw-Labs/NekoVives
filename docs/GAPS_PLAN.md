# UI Gaps — Plan de resolución (jun 2026)

## Gap 1 — Copy Discovery: integrar en Copy Trading con validación HFT

**Estado actual:** `/copy-discovery` existe como página separada con funcionalidad básica
(`/api/copy/discovery/{addr}/stats`). Está oculta del sidebar pero accesible.

**Plan:**
Convertir Copy Trading en una página de 2 tabs:
- **Tab "Leaders"** — el panel actual (watchlist, scores, mirroring) + botón 🔍 Validate.
- **Tab "Discover"** — el contenido actual de CopyDiscovery, pero enriquecido:
  1. Escanear wallets activos del leaderboard público (`/api/copy/discovery`).
  2. **Filtro HFT automático**: si `trades_per_hour > 100` → badge 🤖 HFT, excluir.
  3. **Pre-validación rápida**: para wallets no-HFT con n ≥ 30 pares resueltos,
     corre el validador 3-legs y muestra el EV onchain antes de añadir al watchlist.
  4. Botón "Add to watchlist" solo habilitado cuando `verdict != HFT AND n >= 30`.
  
**Trabajo estimado:** 1 día.

---

## Gap 2 — Risk Center: conectar al wallet real + guardrails multirunner

### Análisis del estado actual

`PortfolioGuard` (src/portfolio_guard.rs) ya existe y funciona:
- `check(current_usdc) → bool` — dispara si wallet cae X% desde baseline.
- `stop_all_live()` en `StrategyRunnerStore` — para todos los runners live.
- **PROBLEMA:** el `total_capital=25000` es hardcoded, no viene del balance onchain real.
  El guard existe en código pero **no se llama periódicamente** desde ningún task.

### Guardrails existentes por runner

Cada runner ya tiene:
- `kelly_size_cap` (default 1.5)
- `max_runner_loss_pct` — detiene el runner individual si baja X%
- `max_consecutive_losses` — detiene tras N pérdidas seguidas
- `min_entry_price` — bloquea long-shots

**El gap real:** cuando múltiples runners operan el mismo wallet,
sus pérdidas individuales se suman — pero ningún guardrail vigila el
**portfolio total**. Un runner puede estar "dentro del límite" mientras el
conjunto ya perdió el 50% del wallet.

### Plan

**Opción A (recomendada): Activar PortfolioGuard conectado al balance real**

1. En `polymarket_runner_loop`, cada N ciclos (5 min) leer el balance onchain real
   via `GET /api/polymarket/balance` y llamar `portfolio_guard.check(balance)`.
2. Si dispara: llamar `store.stop_all_live()` + loguear la razón.
3. En `/live` (UI): mostrar el panel de Risk Center como widget colapsable en la
   parte superior, mostrando:
   - Balance onchain real (de `GET /api/polymarket/balance`)
   - Drawdown desde baseline (%)
   - Threshold configurado
   - Botón "Halt All Live" (ya existe `POST /api/live/stop-all-live`)
4. Eliminar `total_capital` hardcoded del Risk Center.

**Opción B: Eliminar Risk Center como página, embeber el widget en /live**

Más limpio — el halt/resume vive donde pertenece (junto a los runners),
no como página separada. La página `/risk` desaparecería.

**Recomendación:** Opción A — un widget pequeño en `/live` conectado al balance
real, con botón de halt. No mantener `/risk` como página separada.

**Trabajo estimado:** 1 día (backend: conectar el guard al balance real;
frontend: widget en /live).

---

## Gap 3 — Historial de rewards: research de APIs

### Resultado de la investigación

**No hay API pública de rewards** en ningún endpoint conocido de Polymarket:
- `https://clob.polymarket.com/rewards/*` → HTTP 405 (Method Not Allowed, requiere auth L2)
- `https://data-api.polymarket.com/rewards` → 404
- `https://lb-api.polymarket.com/earnings` → sin respuesta

**Los endpoints de rewards SÍ existen pero requieren autenticación L2** (la misma
firma que usan las órdenes). Endpoints conocidos del CLOB que requieren auth:
- `GET /rewards/markets-scores` — score por mercado del maker
- `GET /rewards/aggregate-stats` — stats agregadas del maker
- `GET /rewards/distributions` — distribuciones históricas

**Alternativa onchain (Polygon):**
Polymarket distribuye rewards en USDC directamente a la proxy wallet. La forma
más fiable es **comparar el balance USDC de la proxy wallet** antes y después de
la distribución (medianoche UTC) — la diferencia ES el reward. No hay token
especial; es USDC directo a la wallet.

El Polygon subgraph ha sido deprecado (ya no en thegraph.com) pero los transfers
ERC20 son consultables vía `polygonscan.com` API o directamente vía RPC
(`getLogs` con el event `Transfer` del contrato USDC en Polygon).

### Plan

**Fase inmediata (sin API privada):**
Snapshot diario del balance USDC del proxy wallet antes y después de medianoche UTC.
`reward_today = balance_after_midnight - balance_before_midnight - fills_pnl_today`

Implementar como:
1. Background task que guarda snapshots del balance cada hora en
   `~/.traderclaw/workspace/data/balance_snapshots.jsonl`.
2. Endpoint `GET /api/rewards/history` que calcula la diferencia diaria.
3. Widget en `/rewards` mostrando el histórico de reward/día.

**Fase futura (con auth L2):**
Usar las credenciales CLOB ya configuradas para llamar a
`GET /rewards/aggregate-stats` — que da el reward exacto por mercado.
Esto daría granularidad por mercado (cuánto pagó SpaceX vs Fed).

**Trabajo estimado:** 1 día (snapshot + cálculo diferencial).

---

## Gap 4 — Strategy Builder: decisión

**Veredicto: dejar oculto, no eliminar.**

El Strategy Builder permite crear y editar scripts `.rhai` con CodeMirror
+ previsualizar el backtest. **Funciona** — guarda en `/api/backtest/scripts/content`.

**Por qué no eliminar:**
- Es el único editor de scripts en la UI (útil para pequeños ajustes sin salir del dashboard).
- Ocupa cero espacio en el sidebar ahora que está oculto.
- Podría integrarse como tab en Backtesting en el futuro.

**Acción:** ninguna por ahora. La ruta `/strategy-builder` sigue existiendo y
accesible. Si en 30 días nadie la usa, se elimina en la siguiente ronda de limpieza.

---

## Orden de ejecución sugerido

1. **Gap 3 inmediato** — snapshot de balance + `GET /api/rewards/history`.
   Conecta directamente con el piloto activo de rewards ($100).
2. **Gap 2** — widget Risk Center en /live + PortfolioGuard conectado al balance real.
3. **Gap 1** — Discover tab en Copy Trading con validación HFT.
4. **Gap 4** — ninguna acción.
