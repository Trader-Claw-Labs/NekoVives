# Rewards Maker — Plan de integración (API + UI + runner)

> Documento de contexto previo a los cambios. Convierte el flujo manual de
> liquidity-rewards (probado el 2026-06-09 con un piloto real de $100) en una
> feature nativa de NekoVives.

## Por qué esta es la vía

Tras validar exhaustivamente (ver `strategy/BTC_UPDOWN_5M_FINDINGS.md`):
- El mercado Polymarket 5m/15m crypto es **eficiente**; no hay edge direccional.
  El único edge ahí es **latencia/HFT** (ver el perfil de `@bonereaper`), fuera de
  nuestro alcance a ~120ms de RTT.
- La vía estructural que NO compite contra HFT es **maker / liquidity rewards** en
  mercados **lentos** (política, finanzas a meses): el `rewards_dryrun` midió
  adverse selection ~$0/día ahí, y los rewards se pagan por proveer liquidez.
- El piloto manual ($100 en "Will no Fed rate cuts in 2026?") validó el flujo
  end-to-end: 2 órdenes límite maker resting → elegibles para reward.

## Estado actual (lo que YA existe)

| Pieza | Endpoint / archivo | Estado |
|-------|--------------------|--------|
| Scanner de mercados incentivados | `GET /api/rewards/markets` · `crates/market-analyzer/src/rewards.rs` | ✅ |
| Página de rewards + config | `web/src/pages/Rewards.tsx` (`/rewards`) | ✅ (scanner + config local) |
| Colocar orden | `POST /api/polymarket/order` (límite/market) | ✅ |
| Listar / cancelar órdenes | `GET /api/polymarket/orders` · `DELETE /api/polymarket/order/{id}` | ✅ |
| Balance real | `GET /api/polymarket/balance` | ✅ |
| Motor de timing (gate) | `scripts/ml/rewards_engine.py` (Python, validado) | ✅ advisory |

Falta: la capa de **quoting bilateral**, **tracking de rewards**, y la **automatización**.

---

## Fase 1 — Quoting manual desde la UI (MVP) · ~3-4 días

Objetivo: hacer desde `/rewards` lo que hoy se hizo por `curl`.

### API nueva
- `POST /api/rewards/quote`
  Body: `{ condition_id, yes_token_id, no_token_id, mid, offset_c, size_usd }`.
  Coloca 2 órdenes límite (Buy YES @ mid−offset, Buy NO @ (1−mid)−offset), cada una
  ≥ `min_size`. Devuelve `{ yes_order_id, no_order_id }`. Reusa la lógica de
  `handle_api_polymarket_order_create`.
- `GET /api/rewards/positions`
  Join de `/orders` (quotes resting) + `/positions` (fills) por mercado de reward,
  con el `reward_daily_rate` y `max_spread` de cada uno.
- `DELETE /api/rewards/quote/{condition_id}` — cancela ambos lados.
- `GET /api/rewards/earned` — USDC de reward devengado (API de rewards de Polymarket
  por wallet, o `data-api`), por mercado y total.

### UI (`web/src/pages/Rewards.tsx`)
- Botón **"Quote"** por fila del scanner → modal (offset¢, size$, valida ≥ min_size,
  preview de precios y eligibilidad) → llama `POST /api/rewards/quote`.
- Panel **"Mis Quotes"**: quotes activas (lado, precio, size, status), fills,
  reward devengado por mercado, botón cancelar/re-centrar.

### Riesgo / guardrails
- Validar `size_usd ≥ min_size` antes de enviar.
- Confirmar que las órdenes **descansan** (no cruzan el book) → maker, no taker.
- Tope de capital configurable; warning si el mercado es "toxic" (crypto/sports en vivo).

---

## Fase 2 — Tracking + analítica · ~2-3 días

- Histórico de **reward/día por mercado** (snapshot diario de `/earned`).
- **Net = rewards − fills adversos** (P&L real del farming).
- Indicador de **safety en vivo** por quote (porta la señal de volatilidad del mid de
  `rewards_engine.py`: `mid_vol_c`, `spread_c`).
- Gráfica de P&L neto acumulado del piloto/posiciones.

---

## Fase 3 — Runner de quoting automático (el bot) · ~1-2 semanas

El lift grande. Nuevo `kind = "rewards_maker"` en `src/strategy_runner.rs`:
- Mantiene quotes bilaterales: coloca, vigila **drift del mid**, **re-centra** cada
  `reprice_secs`, y **pausa con el gate de timing** (porta `rewards_engine.py` a Rust —
  el gate valida que caza los spikes de partido / noticias; ver §13 findings).
- Consume la config de la página `/rewards` (spread_offset, order_size, max_markets,
  reprice_secs, min_safety — ya persistida en localStorage; mover a config del runner).
- Risk controls: max capital, max mercados, stop por pérdida, cooldown.
- Reporta como los otros runners (balance, posiciones, rewards), con su propio panel.

### Pre-requisito de Fase 3
**No automatizar hasta que el piloto manual (Fase 1) confirme** que el reward real/día
supera el costo adverso medido. El reward del scanner es un **techo**, no lo que se cobra.

---

## Orden recomendado
1. **Fase 1** (control total del piloto desde UI; base de todo).
2. Fase 2 mientras corre el piloto manual de esta semana.
3. **Fase 3 solo si el piloto valida** el pago real.

## Decisión de negocio (puerta antes de Fase 3)
- Reward neto/día **> $2-3** sobre el piloto → escalar + automatizar.
- Reward **< $1/día** (pool saturado) → no vale el capital; documentar y parar.
