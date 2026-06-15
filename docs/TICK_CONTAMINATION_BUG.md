# Bug de contaminación de datos en to-ticks-multi (y su fix)

> El hallazgo más importante de la auditoría: **los ticks de `data/ticks/<slug>/` estaban
> contaminados** por un bug en el conversor `orderbook_parser.py to-ticks-multi`. Esto
> inflaba el win-rate de las estrategias direccionales con un edge FANTASMA. Confirmado y
> corregido. Disparado por la revisión de un quant externo (ver `QUANT_REVIEW_RESPONSE.md`).

## El bug

`cmd_to_ticks_multi` agregaba los eventos del libro a 1 Hz con:
```sql
SELECT epoch(timestamp_received) AS ts_s,
       max_by(best_bid, timestamp_received) AS yes_bid,
       max_by(best_ask, timestamp_received) AS yes_ask
FROM ... WHERE event_type='price_change' AND market IN (<288 cids del día>)
GROUP BY ts_s          -- ⚠ AGRUPA POR SEGUNDO, MEZCLANDO TODOS LOS MERCADOS
```
Medido: **hasta 6,155 mercados Polymarket distintos emiten un `price_change` en el MISMO
segundo.** El `GROUP BY ts_s` los colapsa en un solo precio y `max_by(timestamp_received)`
se queda con el último que imprimió. Con ~288 ventanas BTC-5m solapadas, el precio del
token de "tu ventana" era frecuentemente el **precio de un mercado vecino que estaba
resolviendo**. Segundo defecto: `window_ts = ts_s % 300` asignaba la ventana por reloj de
pared, no por el mercado real → un tick de un mercado caía en la ventana de otro.

## Por qué creaba un edge fantasma

El precio spliceado incorpora outcomes de mercados ya casi-resueltos. La señal `token_drift`
(P4−P3) "predecía" la resolución porque el precio ya contenía información de mercados
resueltos. Resultado imposible: **drift_v1 ganaba 71.5% a token_price 0.50**, cuando el
mercado real a 0.50 está calibrado al 50% (medido sobre 17,404 ventanas: P(UP|0.50)=51%).

## La prueba del fix (datos limpios vs contaminados)

| drift_v1 | n | WR total | WR @ precio 0.50 |
|---|---|---|---|
| CONTAMINADO (antes) | 3254 | 62.0% | **71.5%** |
| LIMPIO (después)    | 1818 | 43.8% | **51.7%** |

Con datos honestos el WR a 0.50 colapsa a 51.7% (≈ el 50% calibrado) y la estrategia
pierde globalmente. **El "edge" era 100% el artefacto.** En `validate-all`, drift_v1 pasó de
`+16938% / L1✓ L2✓` a `+12.6% / L1✗ L2✗ L3✗` (no pasa ni Leg 1).

## El fix

`cmd_to_ticks_multi` ahora:
1. Agrupa por **`(market, ts_s)`** — un precio por mercado por segundo, sin mezclar.
2. Asigna `window_ts = end_ts − window_secs` y `window_secs_left = end_ts − ts_s` desde el
   **`end_ts` real de cada mercado** (en `markets_info`), no desde `ts_s % 300`.
3. Recorta cada tick a la ventana de SU propio mercado y hace forward-fill **solo dentro de
   cada ventana** (nunca cruzando el cierre de un mercado al open del siguiente).

Regenerar: `to-ticks-multi --slugs btc_5m …` (los ticks viejos quedaron como
`btc_5m_CONTAMINATED`). Aplica a TODAS las series (eth/sol/xrp/… comparten el mismo bug).

## Implicaciones

- **Todos los backtests on_candle / on_tick previos sobre `data/ticks/` estaban
  contaminados.** El veredicto global no cambia (seguía siendo 0 EDGE de 59 — el bug
  *inflaba* el WR, así que con datos limpios es aún más claramente NO_EDGE), pero los
  números intermedios eran ficción.
- El path `clob_events` (de `to-events`, single-market) **NO** tenía este bug — siempre
  separó por mercado. Por eso el latency-sweep y el análisis de arb de paridad eran válidos.
- El path `polymarket_binary` (candles Binance directas + scraped historical) tampoco —
  usa un dataset distinto.

## Leg 4 (añadido en paralelo)

El quant señaló (correctamente) que Leg 3 (shuffle-null) es ciego a edge de precio
constante. Se añadió **Leg 4 — calibration null**: un apostador que gana con prob = precio
de entrada (el fair value implícito). Funciona a precio constante. Además, Leg 3 ahora se
**omite** (no reprueba) cuando la desviación estándar de los precios de entrada < 5¢, donde
es matemáticamente ciego. Verdict EDGE = Leg1 ∧ Leg2 ∧ Leg4 ∧ (Leg3 si hay varianza de precio).
