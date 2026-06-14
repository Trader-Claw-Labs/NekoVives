# Plan: generar un runner rentable, validado antes de arriesgar capital

> Contexto duro (no lo endulzamos): `validate-all` sobre 59 estrategias dio **0 EDGE**.
> Todas las direccionales 5m pasan Leg 1/2 pero fallan el shuffle-null (Leg 3) — el
> mercado 5m está calibrado (ver [[polymarket-5m-market-efficiency]] y
> [[batch-validation-no-edge]]). Buscar "otro runner direccional 5m mejor calibrado" es
> repetir lo que ya falló. Este plan apunta a las DOS vetas que NO hemos agotado.

---

## 0. Los 13 INSUFFICIENT — por qué no operaron (no son candidatos a profit)

Ninguno de estos "casi opera" — la mayoría son ruido o no-aplicables. Clasificación:

| Script | Motor | Trades | Causa de 0 trades |
|---|---|---|---|
| `crypto_4min` | on_candle | 0 | Estrategia **crypto-spot** (buy/sell BTC), no binaria. El validador de 3 legs no aplica. |
| `correlation_arb` | on_candle | 0 | Idem — spot/multi-asset, no Polymarket binario. |
| `dca_bot` | on_candle | 5 | DCA spot — sides no binarios; n=4 < 30. No aplica. |
| `grid_trading` | on_candle | 0 | Grid spot. No aplica al dato binario. |
| `liquidation_hunt` | on_candle | 0 | Spot/volumen. No aplica. |
| `pump_detection` | on_candle | 0 | Spot. No aplica. |
| `event_driven` | on_candle | 1 | Session-based spot; 1 trade. No aplica. |
| `weather_binary` | on_candle | 0 | Mercado de clima (Open-Meteo), no crypto 5m — sin datos en este slug. |
| `strategy` | on_candle | 0 | Plantilla de referencia, no opera. |
| `clob_1hz_volatility_regime` | on_tick | 0 | Gate de régimen nunca se cumple sobre btc_5m (sample-then-trade demasiado estricto). |
| `polymarket_btc_updown_5m_drift_v2_kelly` | on_candle | 0 | Exige `ctx.token_drift` (p3>0) y **aborta sin fallback** cuando el dataset P3 no cubre el rango. El `drift_v2_combo` sí opera porque tiene fallback. |
| `polymarket_hype_updown_5m_thinmkt` | on_candle | 0 | HYPE es drift-only sin velas Binance; el gate de drift no dispara en el rango. |
| `polymarket_btc_updown_5m_hybrid` | on_candle | 4 | Proxy híbrido muy restrictivo; n=4 < 30. |

**Conclusión:** de los 13, **ninguno es un candidato a profit** — 8 son estrategias crypto-spot
que ni siquiera pertenecen al mercado binario (el validador no les aplica), y 5 son binarias
que no dispararon por gates rotos/datos faltantes. Arreglarlos daría más NO_EDGE, no edge.

---

## 1. Por qué NO insistir en direccional 5m

Las 46 NO_EDGE incluyen drift, mean-reversion, momentum, ensembles, per-asset (eth/sol/xrp/
doge), extreme-fade y late-certainty a toda latencia. Todas fallan Leg 3. El `random_control`
da el mismo NO_EDGE. **La señal direccional 5m no existe** — el precio del token ya incorpora
la deriva. Más variantes 5m = más tokens quemados en backtest.

---

## 2. Las dos vetas no agotadas (orden de prioridad)

### Veta A — MAKER / rewards (la más prometedora, no es direccional)
El edge aquí **no es predecir dirección** sino **cobrar el spread + los rewards de Polymarket
por proveer liquidez bilateral**. Es estructural, no estadístico, así que NO tiene por qué
fallar el shuffle-null (ese test es para señales direccionales).
- Ya existe el motor `run_maker_backtest` (Fase D) que mide **eligible_uptime_pct** y
  **adverse_selection_pct** sobre el event stream — exactamente las dos cosas que el pilot
  manual de rewards hizo mal.
- **Hipótesis a validar:** en mercados de baja volatilidad / spread estable, un maker
  bilateral mantiene >X% uptime con <Y% adverse selection → rewards netos positivos.
- Métrica de éxito ≠ edge_validator de 3 legs (ese es para apuestas direccionales).
  El éxito es: `adverse_selection_pct` bajo + `eligible_uptime_pct` alto + (rewards
  estimados − pérdidas por adverse fills) > 0.

### Veta B — Latencia real sub-segundo (HFT), SOLO si los datos lo soportan
El sweep de late_certainty mostró NO EDGE a 0-110ms, pero esa señal era direccional. La
veta HFT genuina es **arbitraje de libro stale** (el book de Polymarket reacciona tarde a
Binance), medible con `clob_events` a resolución ms.
- Requiere primero correr `basis_analysis.py` sobre el event stream para ver si EXISTE
  lag capturable tras fees. Si lag=0 (mercado eficiente a 1s), no hay veta y se descarta.
- Solo vale si el VPS nuevo (~50ms) cae dentro de la ventana de lag medida.

---

## 3. Pipeline para CADA runner candidato (gate-driven, barato primero)

```
  (0) Hipótesis escrita: ¿qué ineficiencia explota? ¿por qué el mercado no la ha cerrado?
       └─ si no hay respuesta estructural → no construir.
  (1) Señal/diagnóstico en Python sobre los datos REALES (basis_analysis / un notebook)
       └─ GATE: la ineficiencia existe en los datos (lag>0, o spread>adverse). Si no → stop.
  (2) Escribir el script .rhai (on_event para maker/HFT; on_candle solo si A/B lo justifica)
  (3) Backtest con el motor correcto + fee crypto_taker + resolución oficial
       └─ GATE maker: adverse% bajo, uptime% alto, neto>0.
       └─ GATE direccional: edge_validator 3 legs = EDGE (n≥30).
  (4) Walk-forward (train/test split) — el edge debe sobrevivir OOS (holds_out_of_sample)
       └─ GATE: EDGE en train Y en test. Si solo train → overfit → stop.
  (5) Latency sweep — el edge sobrevive a la latencia real del VPS
       └─ GATE: EV>0 a la latencia del VPS (~50ms). Si se evapora → no desplegar.
  (6) Dry-run en vivo (paper) N días — fills/uptime reales ≈ backtest
       └─ GATE: realized ≈ simulated (sin sorpresas de fill).
  (7) Pilot real PEQUEÑO ($50-100) con guardrails (kelly_cap, max_loss, min_entry)
```
Cada gate mata barato. La regla de la casa: **pasar el backtest NO es edge** — solo un EDGE
del validador (direccional) o un neto-positivo robusto de maker (estructural), confirmado
out-of-sample y a la latencia real, justifica capital.

## 4. Primer paso concreto recomendado
Veta A (maker) es la de mayor probabilidad porque no depende de predecir dirección.
Arrancar por el paso (1): correr `basis_analysis.py` + un diagnóstico de spread/adverse
sobre el event stream de 33 días (btc_5m_ev) y mercados de baja-vol, para decidir si la
hipótesis de maker tiene sustento ANTES de escribir un script.
