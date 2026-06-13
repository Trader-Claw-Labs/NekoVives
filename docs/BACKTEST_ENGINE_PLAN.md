# Plan: Motor de Backtesting Unificado para Polymarket (latencia paramétrica + HFT)

> Objetivo: UN solo motor event-driven capaz de (1) correr TODAS las estrategias del
> repo — Rhai `on_candle`/`on_tick` Y los engines nativos (arb_binary, rewards_maker,
> fair_value, fv_momentum, minting_mm, arb_hedge, rotation_compounder) —
> (2) sobre TODOS los datos históricos de Polymarket (archivo pmxt v2 event-level +
> resoluciones oficiales Gamma + Chainlink histórico), (3) con la latencia como
> PARÁMETRO de simulación (feed y orden por separado), cubriendo desde estrategias
> de 5 minutos hasta el régimen HFT sub-segundo.
>
> Regla de la casa: este motor existe para RECHAZAR estrategias barato. Pasar el
> backtest no es edge — el edge_validator de 3 legs sigue siendo el gate final.

---

## 1. Lo que YA tenemos (inventario verificado en código)

### 1.1 Datos

| Fuente | Qué es | Estado |
|---|---|---|
| **pmxt.dev v2 archive** | Parquets horarios desde **2026-04-13 19:00 UTC** (`r2v2.pmxt.dev`). Event-level: `price_change` (best_bid/best_ask por evento) y `last_trade_price` (price/size/side) con `timestamp_received` — granularidad **sub-segundo** | ✅ pipeline de descarga + queries DuckDB remotas (`tools/orderbook_parser.py`) |
| **Ticks 1Hz JSONL** | `data/ticks/<slug>/*.jsonl`: yes_bid/ask, no_bid/ask, binance_price, depth, window_ts, `window_yes_won` | ✅ 33-41 días BTC/ETH/SOL/XRP 5m+15m. **PERO: `to-ticks` decima el archivo a 1Hz** → pierde sub-segundo y dirección del trade flow |
| **Resoluciones oficiales** | Gamma API `outcomePrices` por slug `{series}-{window_ts}` | ✅ para series updown (`backfill-resolutions`, `fetch_polymarket_window_resolutions`) — ❌ no generalizado a mercados arbitrarios |
| **Binance 1m klines** | REST paginado, cache JSON en `data/` | ✅ |
| **Chainlink** | Poll live en runner (`chainlink_endpoint_url`) + `chainlink_price`/`oracle_lag_ms` en ticks live | ❌ **NO hay histórico** — los ticks del archivo lo llevan en 0 |
| **Tick recorder live 1Hz** | depth real del `/book` cada 10s | ✅ (`src/tick_recorder.rs`) |

### 1.2 Motor actual (`src/tools/backtest.rs`, ~5.700 líneas)

- **4 modos** vía `run_backtest_engine()` (l.3214): `clob_1hz` (on_tick 1Hz), `archive_candles`
  (ticks→velas 1m + token price real en decisión), `polymarket_binary` (ventanas con velas
  Binance/clima), rhai legacy (velas crypto).
- **Fill model**: `sim_fill_vwap()` (l.4482) — book sintético de 3 niveles (50/30/20% del
  depth a ask, ask+spread, ask+2×spread); sin depth → fill a best ask. **Fill same-tick:
  CERO latencia simulada en todo el motor Rust.**
- **Fees**: % del stake al entrar. **La fórmula real crypto-taker `1.8%×p(1−p)` NO existe
  en el motor Rust** (solo en los scripts Python).
- **Resolución**: `window_yes_won` oficial > fallback Binance, con contadores de calidad
  (`res_official`/`res_binance`).
- **Métricas**: Sharpe anualizado, MDD, WR, 5 worst trades, avg_token_price, break-even WR,
  coverage, recommended_max_stake.
- **Guardrails live-parity**: kelly_size_cap, min/max_entry_price, max_consecutive_losses,
  stop_loss + `TickGates` (spread, horas).
- **Performance**: AST Rhai compilado una vez, `spawn_blocking`, cache de velas. Sin
  paralelismo de datos (suficiente por ahora).

### 1.3 Piezas reutilizables ya construidas

- **`strategy-core`**: trait `StrategyEngine` con `on_tick(MarketSnapshot)` y
  `on_book(BookSnapshot)` + `ExecutionMode::Backtest` — la abstracción para unificar YA existe.
- **`tools/engine_backtest.rs`**: wrapper que corre los engines nativos en modo Backtest…
  pero con **precios SINTÉTICOS** (closes de BTC normalizados a 0.10-0.90). Sirve de humo,
  no de verdad.
- **`scripts/ml/phase0_backtest.py`**: latencia simulada (fill al tick siguiente) +
  resolución oficial + edge_validator — el patrón de honestidad a portar al motor.
- **`scripts/ml/edge_validator.py`**: 3 legs (bootstrap CI, random-side null, shuffle null).
- **UI Backtesting page** + rutas `/api/backtest/*`: solo hay que añadir modos/params.

---

## 2. Lo que FALTA — mapeado a los 4 requisitos

### R1 — "Todas las estrategias"
- ❌ Engines nativos sobre **datos reales** (hoy solo sintético). Falta alimentar
  `StrategyEngine` con `MarketSnapshot`/`BookSnapshot` reconstruidos del archivo.
- ❌ **Modelo MAKER**: rewards_maker y minting_mm colocan límites resting. No existe
  simulación de queue position, probabilidad de fill, adverse selection ni earnings de
  rewards. Sin esto, los dos engines de Grupo 2 no son backtesteables honestamente.
- ⚠️ API legacy 2-param: documentada como no soportada — queda fuera (ok).

### R2 — "Todos los datos históricos de Polymarket"
- ❌ Solo convertimos series UP/DOWN 5m/15m/1h. El archivo trae **todos los mercados**
  (cualquier condition_id: deportes, elecciones, mercados largos). Falta conversión
  genérica + descubrimiento/resolución vía Gamma para mercados arbitrarios (no solo
  ventanas updown).
- ❌ **Chainlink histórico**: imprescindible por dos razones — la resolución oficial de los
  updown ES Chainlink, y el desfase Binance↔Chainlink es la señal HFT central. Fuente:
  rounds onchain en Polygon (`getRoundData` del aggregator BTC/USD etc. vía RPC de archivo)
  o un proveedor de data streams. Hoy: nada.
- ⚠️ Cobertura pre-2026-04-13: el v2 no llega más atrás. Para antes: CLOB
  `/prices-history` (1m, ya integrado) + actividad onchain. Documentar el límite, no fingir.
- ⚠️ El parser solo consulta `price_change` y `last_trade_price`; correr `summary`
  (GROUP BY event_type) para enumerar TODOS los event_types reales del v2 — si existe un
  snapshot de book completo (L2), el fill model mejora un nivel entero. **(Confirmar contra
  https://archive.pmxt.dev/docs/v2-data-overview — sin acceso de red desde esta sesión.)**

### R3 — "Latencia como parámetro"
- ❌ El motor Rust llena same-tick. Falta `latency` como parámetro de primera clase, con
  **dos componentes separados**: `feed_latency_ms` (cuándo VES el evento) y
  `order_latency_ms` (cuándo LLEGA tu orden). El fill se evalúa contra el book en
  `t_señal + feed + order`, no contra el book que disparó la señal.
- ❌ Con datos 1Hz la latencia mínima simulable es ~1000ms → la latencia fina exige el
  modo event-level (R4).
- ❌ **Latency sweep**: correr la misma estrategia a 0/50/110/220/500/1000ms y graficar
  EV vs latencia — ESA curva es la que decide si una idea es HFT-viable o no (la pregunta
  del VPS plan, automatizada).

### R4 — "Incluir HFT"
- ❌ `to-events`: conversión parquet → stream de eventos por mercado conservando
  `timestamp_received` en ms (hoy se tira todo lo sub-segundo).
- ❌ Modo `clob_events` en el motor: replay evento a evento.
- ❌ Fee model crypto-taker real (`1.8%×p(1−p)`) y maker (0% + rewards) en Rust.
- ⚠️ Límite honesto: el archivo es **top-of-book + trades**, no L2 completo → el slippage
  profundo seguirá siendo estimado y NO podemos simular nuestro propio impacto en el book
  (replay = somos invisibles). Esto sobrestima el edge de tamaños grandes; mitigación:
  cap de stake vs depth observado + reporte de "% del volumen del book que consumirías".

---

## 3. Arquitectura propuesta

```
                    ┌─ pmxt v2 parquets ─ to-events ─┐
  FUENTES           ├─ Gamma resolutions ────────────┤
                    ├─ Chainlink rounds (Polygon) ───┼──► MarketEvent stream (por mercado,
                    ├─ Binance klines/aggTrades ─────┤     ordenado por ts_ms)
                    └─ ticks 1Hz live recorder ──────┘
                                   │
                       ┌───────────▼───────────┐
                       │  EventBus / Replayer  │  dos relojes:
                       │  (exchange time  vs   │  strategy_time = exchange_time
                       │   strategy time)      │                + feed_latency_ms
                       └───────────┬───────────┘
              ┌────────────────────┼─────────────────────┐
   ┌──────────▼─────────┐ ┌────────▼────────┐  ┌─────────▼─────────┐
   │ StrategyAdapter:   │ │ StrategyAdapter:│  │ StrategyAdapter:  │
   │ Rhai on_tick (1Hz  │ │ Rhai on_candle  │  │ Native engines    │
   │ muestreado o raw)  │ │ (agrega velas)  │  │ (on_tick/on_book) │
   └──────────┬─────────┘ └────────┬────────┘  └─────────┬─────────┘
              └────────────────────┼─────────────────────┘
                       ┌───────────▼───────────┐
                       │  ExecutionSimulator   │  order_latency_ms; taker = VWAP sobre el
                       │  (taker + maker)      │  book FUTURO en t+lat; maker = resting
                       │                       │  order + queue model + cancel latency
                       └───────────┬───────────┘
                       ┌───────────▼───────────┐
                       │ FeeModel pluggable    │  pct simple │ crypto 1.8%×p(1−p) │ maker
                       ├───────────────────────┤
                       │ Resolución oficial    │  window_yes_won / Gamma genérico
                       ├───────────────────────┤
                       │ Métricas + guardrails │  (reusar) + curva EV-vs-latencia
                       └───────────────────────┘
```

`MarketEvent = { ts_ms, kind: BookTop{bid,ask} | Trade{price,size,side} |
WindowBoundary | Resolution{yes_won} | OracleMark{chainlink|binance} }`

Principio: **no reescribir lo que funciona**. El core nuevo vive junto a los 4 modos
actuales; los modos viejos se mantienen hasta que el nuevo reproduzca sus resultados
(gate de paridad), y solo entonces se migran.

---

## 4. Fases (gated — cada una con entregable y criterio de validación)

### Fase B0 — Quick win: latencia en el motor actual (1-2 días) ⟵ EMPEZAR AQUÍ
- Añadir `latency_ms` a `BacktestRunBody` → en `clob_1hz` el fill usa el book del primer
  tick con `ts ≥ t_señal + latency` (hoy: same-tick). Igual en `archive_candles` para el
  precio de decisión.
- Añadir fee model `crypto_taker` (1.8%×p(1−p)) seleccionable.
- Endpoint `latency sweep`: misma config × lista de latencias → tabla EV/WR por latencia.
- UI: campo Latency (ms) + selector de fee en la página Backtesting.
- **Validación:** con `latency=0` y fee `pct`, resultados bit-a-bit idénticos a hoy;
  con `latency=1000` sobre btc_5m, EV de late_certainty ≈ phase0_backtest.py (paridad
  Rust↔Python).

### Fase A — Cimientos de datos (2-4 días, paralelizable con B0)
- ✅ `to-events` (`tools/orderbook_parser.py`): parquet → `data/events/<slug>/YYYY-MM-DD.jsonl.gz`
  con TODOS los eventos en resolución **ms (sin decimar)** y **ambos tokens YES/NO con su
  book real** (no `no = 1 − yes`). Modo `--market 0x…` (mercado arbitrario, deriva YES/NO
  por conteo si no hay metadata) o `--series-prefix btc-updown-5m` (descubre cada ventana).
  Header `kind:"meta"` por archivo con tokens + resolución oficial. Eventos:
  `{ts_ms, kind:"book"|"trade", token, cid, …}`. **Dedup de book-top sin cambio por defecto**
  (el archivo re-emite ~93% sin cambio → 36× menos eventos; `--no-dedup` para conservarlos).
- ✅ `list-markets`: enumera TODOS los condition_ids de los parquets locales (crypto,
  política, deportes, esports) por volumen; `--enrich` añade título/slug vía CLOB,
  `--filter <kw>` busca por keyword. Hace descubrible cualquier mercado para `to-events --market`.
- ✅ **Fix de resolución oficial**: Gamma `/markets?condition_id=` ignora el filtro y devolvía
  un mercado arbitrario → `fetch_polymarket_window_resolutions`, `resolve_market_tokens` y
  el path de `to-ticks-multi` ahora usan el CLOB `/markets/{cid}` (`fetch_clob_market`, con
  `winner` por token). Verificado: 8/8 ventanas conocidas coinciden. Los ticks ya generados
  vía `backfill-resolutions` (que usa `?slug=`, sí funciona) estaban correctos.
- ❌ `summary` de event_types del v2 → confirmar si existe snapshot L2 (mejoraría el fill model).
- ❌ Resoluciones genéricas para mercados arbitrarios en JSONL 1Hz existentes (patcher).
- ❌ **Chainlink histórico**: script que baja rounds del aggregator (Polygon RPC con archivo)
  por rango de fechas → `data/chainlink/<feed>/*.jsonl`; merge como `OracleMark` en el
  stream y backfill de `chainlink_price` en los ticks existentes.
- **Validación:** ✅ `to-events` sobre parquet REAL (archivo pmxt v2, 23-abr→25-may, 777 horas
  sin huecos) conserva el 100% de los eventos a ms, etiqueta ambos tokens, dedup 2.8M→78k.
  ✅ flujo política end-to-end (Republicans 2028). ❌ Pendiente: Chainlink histórico.

### Fase C — Motor event-driven `clob_events` (✅ implementado)
- ✅ Replayer de `MarketEvent` (`load_events_for_range` lee los `.jsonl.gz` de Fase A,
  gunzip + sort por ts_ms) con **dos relojes**: `feed_latency_ms` (cuándo el script
  PERCIBE el evento → `ctx.ts_ms` desplazado) y `order_latency_ms` (la orden se llena
  contra el book FUTURO en t+lat, o se descarta si la ventana cierra antes).
- ✅ Motor `run_clob_events_backtest` con **book de dos lados reconstruido** desde los
  eventos (yes/no bid/ask vivos) + API `on_event(ctx)`. Settlement por ventana con
  resolución oficial (`window_yes_won` por su propio cid — bug de resolución cruzada
  encontrado y corregido en la validación). Fee `pct`/`crypto_taker`, mismos gates y
  `sim_fill_vwap` que clob_1hz (live-parity).
- ✅ Dispatch `market_type = "clob_events"` en `run_backtest_engine`; `feed_latency_ms`
  nuevo en `BacktestRunBody`/`LatencySweepBody`; ruta `GET /api/backtest/event-slugs`;
  script de ejemplo `clob_events_latency_arb.rhai` (bundled).
- ✅ **Validación:** 2 tests unitarios (settlement oficial + miss por order-latency) +
  smoke sobre datos REALES (btc_5m, 78k eventos/h): latency sweep 0ms→250ms muestra el
  edge degradándose (+17.07% → +16.73%), 100% resolución oficial. ⚠️ n=4 trades en 1h =
  validación de MECÁNICA, no evidencia de edge — eso lo decide el edge_validator sobre
  el mes completo.
- ❌ Pendiente (no bloqueante): cap por depth observado + reporte de consumo de book
  (hoy `sim_fill_vwap` con depth=0 → fill a best ask); compat `on_tick` por muestreo 1Hz
  desde eventos (los scripts on_tick siguen usando el motor clob_1hz, que queda intacto
  como gate de paridad).

### Fase D — Modelo MAKER (✅ implementado)
- ✅ `run_maker_backtest` (`engine_backtest.rs`): replica el `rewards_maker` (quote
  bilateral a `mid ± offset`, re-center on drift) sobre el event stream ms. Fill model:
  una BUY resting se llena cuando un `trade` imprime a precio ≤ tu quote (aprox. top-of-book,
  asume front-of-queue cuando eres el mejor precio). Mide **adverse selection** (movimiento
  del YES-mid 10s post-fill en tu contra), **eligible uptime** (% de book events con ambas
  patas dentro de `reprice_threshold` del mid), y fills YES/NO.
- ✅ Dispatch: `market_type=clob_events` + `kind=rewards_maker|minting_mm` → maker backtest,
  con `maker_stats` (eligible_uptime_pct, adverse_selection_pct, fills) en el response.
- ⚠️ **rewards earnings NO modelado** (depende del pool de rewards de Polymarket; se calibra
  con datos reales del pilot). Este modelo mide FILL QUALITY + UPTIME — lo que el pilot
  manual hizo mal. Queue position es aproximada (sin order-by-order data); cancel-latency y
  fills parciales quedan como mejora futura.

### Fase E — Engines nativos sobre datos reales (✅ implementado)
- ✅ `run_engine_clob_events_backtest`: alimenta `arb_binary`, `fair_value`, `fv_momentum`,
  `arb_hedge` con un `BookSnapshot` de **dos lados REAL** (YES y NO reconstruidos
  independientemente del event stream, NO `no=1-yes`) a resolución ms, vía `on_book`.
  Settlement por ventana con resolución oficial (`window_yes_won` por su cid).
- Mejora sobre `run_engine_clob_1hz_backtest` (que usaba ticks 1Hz, NO book derivado, depth
  fijo, resolución Binance). El 1Hz queda como path alterno.
- ⚠️ Pendiente: cap por depth observado (hoy depth fijo 1000); `rotation_compounder`/`minting_mm`
  no soportados en este path (no son book-takers puros).

### Fase F — Validación estadística y UI (✅ implementado)
- ✅ edge_validator nativo (3 legs, `src/tools/edge_validator.rs`) integrado en
  `POST /api/backtest/run` vía `validate_edge: true` → `edge_validation` en el response
  (CI bootstrap, random-null, shuffle-null, verdict EDGE/NO_EDGE/INSUFFICIENT).
- ✅ Walk-forward: `POST /api/backtest/walk-forward?train_frac=0.7` parte el rango en
  train/test, corre el backtest en cada uno + edge_validator, y reporta `holds_out_of_sample`
  (cazа overfit: edge que solo sobrevive in-sample no es edge).
- ✅ UI: modo "Orderbook Archive (on_event)" en la página Backtesting (slug picker desde
  `/api/backtest/event-slugs`, campos Order/Feed latency, fee model selector).
- ✅ Latency sweep ya existía (`/api/backtest/latency-sweep`); el agente lo corrió sobre
  datos reales (ver `scripts/clob_events_latency_sweep/`).
- ⚠️ Pendiente: curva EV-vs-latencia graficada en la UI; comparador A/B de configs.

**Orden recomendado:** B0 → A → C → (D ∥ E) → F. **(todas las fases implementadas)**

### Hallazgo de la primera corrida real (latency sweep + edge_validator)
El script demo `clob_events_latency_arb` sobre 3 días reales de btc_5m_ev (n=375):
**NO EDGE a ninguna latencia (0-1000ms).** Pasa Leg 1 (CI>0) y Leg 2 (random-null) pero
**falla Leg 3 (shuffle-null, p≈1.0)** — la ganancia aparente viene de qué ventanas ganaron
en este sample corto, no de skill repetible. Latencia-insensible (375→373 trades a 1s). Es
exactamente el tipo de falso-edge que esta infraestructura existe para rechazar barato.
Detalle en `scripts/clob_events_latency_sweep/`. (n corto: re-correr sobre el mes completo.)

---

## 5. Limitaciones honestas (no las escondemos)

1. **Top-of-book**: sin L2 completo, el slippage profundo es modelo, no medición. El motor
   reporta qué % del depth consumirías; stakes > depth observado se marcan como no fiables.
2. **Sin impacto propio**: replay histórico = el mercado no reacciona a tus órdenes. El
   edge HFT real será ≤ al simulado. La curva EV-vs-latencia es cota superior.
3. **Maker queue = aproximación**: sin order-by-order data, la queue position es estimada.
   Calibrar contra fills reales del pilot de rewards antes de confiar.
4. **Cobertura temporal**: v2 empieza 2026-04-13. Antes de eso solo hay velas 1m del CLOB.
5. **Gaps de horas en el archivo** (`filter_available_urls` ya los maneja) — el motor debe
   reportar huecos, nunca interpolarlos en silencio.
6. **Chainlink histórico** depende de un RPC Polygon con estado de archivo (público
   limitado; evaluar Alchemy/QuickNode free tier).

## 6. Decisiones a confirmar antes de la Fase C

- [ ] Confirmar event_types reales del v2 (correr `summary --days 1` desde la laptop;
      esta sesión no tiene egress a pmxt.dev) — ¿hay snapshots L2?
- [ ] Fuente de Chainlink histórico: ¿RPC Polygon archive (gratis, lento) o proveedor?
- [ ] ¿`on_event(ctx)` en Rhai para HFT, o las estrategias HFT se escriben como engines
      nativos en Rust? (Rhai a >100 eventos/s por mercado puede ser el cuello de botella;
      propuesta: Rhai hasta 1Hz, nativo para sub-segundo.)
- [ ] Almacenamiento de eventos: JSONL.gz (simple) vs parquet local (rápido). Propuesta:
      parquet local + DuckDB para queries, JSONL para el replayer.
