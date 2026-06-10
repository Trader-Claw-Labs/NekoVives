# NekoVives — Auditoría de limpieza

> Documento de contexto previo a los cambios. Lista features rotas / irreales para
> remover o reparar. **Nada aquí se ejecuta sin tu visto bueno** — quitar features es
> destructivo. Marcado: 🔴 evidencia dura de roto · 🟡 sospechoso, verificar · 🟢 sano.

## Principio rector

NekoVives acumuló features experimentales. La sesión de validación (jun 2026) demostró
que **varias producen resultados irreales** (edge fantasma) que llevan a decisiones malas
con dinero real. La limpieza prioriza: **borrar/marcar lo que engaña, surfacing lo que sí
mide la realidad.**

---

## 1. Backtesting — 🔴 el caso más claro (tu ejemplo)

**Evidencia:** ver `strategy/BTC_UPDOWN_5M_FINDINGS.md` §11-§14. Los fills LIVE reales
onchain (−$10,351) contradijeron TODO backtest que mostraba edge.

| Market type | Problema | Acción propuesta |
|-------------|----------|------------------|
| `archive_candles` | 🔴 Precios stale del archivo pmxt.dev → fabrica +8-13pts de edge fantasma (mismo periodo, signo opuesto a datos limpios) | **Quitar del selector** o relabel "Diagnóstico — NO mide edge" |
| `polymarket_binary` | 🔴 Precio 100% sintético (`polymarket_token_price` momentum) + resolución binance | **Quitar** o "Solo smoke-test (¿corre el script?)" |
| `clob_1hz` | 🔴 Replay de ticks con los mismos precios stale del archivo | Marcar "no fiable para edge" |
| Métrica `total_return_pct` (compuesto) | 🔴 +3,818% de un EV/trade pequeño compuesto → engaña | De-enfatizar; mostrar **EV/trade + P&L a stake fijo** |

**Reemplazo (surfacing lo real):**
- Integrar `scripts/ml/edge_validator.py` como **pestaña "Validate"** en `/backtesting`:
  corre el test de 3 legs (CI bootstrap + random-side null + shuffle null) sobre los
  trades de un runner con **resolución oficial**. Veredicto claro EDGE / NO EDGE.
- Mantener `polymarket` (Binance candles real) y `archive_candles`/`clob_1hz` **solo** si
  se relabela como "diagnóstico de ejecución", nunca como medición de edge.

---

## 2. Scripts de estrategia — 🔴 deprecar los payout-explosion / sin edge

Validados como **sin edge real** (o edge fantasma por long-shots a 0.03-0.10):

| Script | Veredicto |
|--------|-----------|
| `clob_1hz_late_certainty` | 🔴 −$642/−$816 real; "edge" era payout-explosion |
| `clob_1hz_spread_scalper` | 🔴 −$1,803; long-shots avg entry 0.166 |
| `clob_1hz_vwap_revert` | 🔴 −$1,071 |
| `clob_1hz_ofi`, `clob_1hz_fair_value_gap` | 🔴 0 edge real |
| `polymarket_all_updown_5m_adaptive(_NO)` | 🔴 −$3k a −$0.4k por asset |
| `polymarket_btc_updown_5m_hybrid_v12` | 🔴 66-88% long-shots, WR inestable |
| `*_mean_rev`, `mean_reversion`, `btc_opt_v15_bb_mean_rev` | 🔴 sin edge |

**Acción:** mover a `scripts/deprecated/` (no borrar — referencia), y quitar de los
defaults sugeridos en la UI. Dejar solo los validados (`drift_v4_safe`,
`drift_v2_strict_xl`, `random_control`) y marcar que **ninguno tiene edge confirmado** aún.

---

## 3. Páginas del dashboard — feedback del usuario (jun 2026)

El usuario verificó varias en uso. Marcado con su feedback directo:

| Página | Estado | Acción |
|--------|--------|--------|
| `/` Dashboard, `/wallets`, `/polymarket`, `/live`, `/rewards`, `/backtesting`, `/logs`, `/health` | 🟢 en uso | mantener |
| **`/risk` (Risk Center)** | 🔴 "no hace nada" | **eliminar** del menú, O integrarlo con los guardrails reales (`PortfolioGuard`, runner risk controls) si aporta. Decidir: integrar vs borrar. |
| **`/strategy-builder`** | 🔴 "no tiene función real — no se crean estrategias reales" | **eliminar** (las estrategias se editan como `.rhai` directo). Confirmar que no rompe rutas. |
| **`/copy-trading`** | 🔴 "no funciona" | **Primero evaluar viabilidad del copy-trading** (¿hay edge en copiar líderes?), **excluyendo wallets HFT** (como `@bonereaper`). Si viable → reparar; si no → eliminar. |
| **`/copy-discovery`** (Discovery) | 🔴 "no funciona" | igual que copy-trading: depende de la evaluación de viabilidad |
| **`/orderbook` (Orderbook Archive)** | 🟡 reubicar | **Mover/integrar en Backtesting** — el archivo de orderbook es insumo del backtesting, no una sección aparte |
| `/telegram`, `/scheduled-jobs` | 🟡 verificar | ¿se usan? decidir después |

## 4. Métricas y métodos engañosos — 🔴 corregir transversalmente

- **Resolución `binance_provisional` como verdad** → ya mitigado (fixes condition_id +
  sweep + clobber/starvation + `settle_price`, jun 2026). El dashboard muestra oficial
  y el P&L usa el fill realista (no el entry optimista).
- **P&L del dashboard ≠ onchain** (estaba 2.6× mal) → añadir botón "Reconciliar onchain"
  (`data-api/activity`) en `/live`.
- **Retorno compuesto** en cualquier vista → reemplazar por EV/trade + stake fijo.

---

## Plan de ejecución por fases (cuando des luz verde a cada una)

- **Fase A — Backtesting** (en curso): relabel/ocultar engines irreales + pestaña
  Validate (edge_validator 3 legs) + EV/trade en vez de retorno compuesto.
- **Fase B — Páginas muertas**: eliminar `/risk` (Risk Center) y `/strategy-builder`
  (sin función real), o integrarlos si aportan. Reversible primero (ocultar del menú).
- **Fase C — Copy Trading**: evaluar viabilidad (¿edge en copiar líderes, **excluyendo
  wallets HFT**?). Si viable → reparar `/copy-trading` + `/copy-discovery`; si no → eliminar.
- **Fase D — Orderbook → Backtesting**: integrar `/orderbook` (descarga de archivo) dentro
  de la página de Backtesting; el archivo es insumo del BT, no una sección aparte.
- **Fase E — Scripts**: mover los deprecated (payout-explosion) a `scripts/deprecated/`.
- **Fase F — Métricas**: EV/trade + reconciliación onchain en `/live`.

> Regla: antes de **eliminar** cualquier cosa, la marco/oculto primero (reversible) y
> confirmas; el borrado duro es el último paso.

> Regla: antes de **eliminar** cualquier cosa, la marco/oculto primero (reversible) y
> confirmas; el borrado duro es el último paso.
