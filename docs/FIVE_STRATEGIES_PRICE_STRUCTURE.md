# Las 5 estrategias que pasan Leg 1 + Leg 2 pero fallan Leg 3

> Complemento de [EDGE_VALIDATOR_EXPLAINED.md](EDGE_VALIDATOR_EXPLAINED.md). Aquí
> diseccionamos 5 estrategias reales con datos reales (33 días de btc_5m / btc_5m_ev,
> fee crypto_taker, resolución oficial Polymarket). Las 5 **se ven ganadoras** (WR 54-80%,
> EV +11% a +231%) y aun así dan **NO_EDGE**: pasan los dos primeros legs y mueren en el
> tercero con `p_shuffle = 1.000`. Entender *por qué* enseña qué necesitaría una estrategia
> para pasar de verdad.

## Tabla resumen (datos reales del backtest)

| Estrategia | n | WR | precio medio | EV/trade | EV barajado | Leg 3 | % trades en ~0.5 |
|---|---|---|---|---|---|---|---|
| drift_v1 | 3254 | 62% | 48¢ | +27% | **+33%** | p=1.000 ❌ | 67% |
| late_certainty | 4474 | 69% | 46¢ | +89% | **+130%** | p=1.000 ❌ | 59% |
| latency_arb | 4040 | 72% | 50¢ | +58% | **+69%** | p=1.000 ❌ | 68% |
| midband_no | 2090 | 54% | 48¢ | +11% | +12% | p=1.000 ❌ | 87% |
| btc_binary | 4664 | 80% | 44¢ | +231% | **+306%** | p=1.000 ❌ | 50% |

**El patrón que las condena, visible en una sola columna:** en TODAS, el `EV barajado ≥ EV real`.
El azar (reasignar cuál trade ganó) lo hace igual o mejor que la estrategia. Eso es `p=1.000`:
la secuencia de aciertos no tiene NADA de especial.

---

## Cómo funciona cada una y por qué falla

### 1. `drift_v1` — drift-fade (la familia más grande, 9 variantes)
**Lógica:** mira el "drift" del token (`token_price` vs `token_price_prev`, 60s antes). Cuando
el precio está en la banda media (0.18-0.82) y el drift es negativo, compra YES (apuesta a
que el drift revierte); drift positivo → vende. Es mean-reversion sobre el precio del token.

**Estructura de precios (real):**
```
 precio   n     WR   fair   gap
  0.4    548   41%   40%   +1pp   ← clava el fair
  0.5   2173   71%   50%  +21pp   ← TODO el "edge" está aquí
  0.6    241   62%   60%   +2pp   ← clava el fair
```
**Por qué falla:** el 67% de los trades caen en precio ~0.5, y solo ahí el WR (71%) supera al
fair (50%). En todos los demás precios el WR ≈ fair (gap ~0). Como casi todo está al mismo
precio, el EV depende solo de *cuántos* ganaste, no de *cuáles* → barajar no cambia nada
(EV barajado +33% > real +27%). El +21pp en 0.5 es el sesgo del periodo: esos días, el lado
que eligió a 0.5 ganó 71%. No es predicción.

### 2. `late_certainty` — fade de certeza tardía
**Lógica:** en los últimos 30-35s de la ventana, si Binance ya se movió claro pero el token
no está en el techo, compra el lado "correcto" a descuento. Setups A-D.

**Estructura de precios (real):** WR > fair en casi TODOS los buckets (+25 a +38pp). A primera
vista parece que SÍ predice. Pero el 59% está en 0.5, y el detalle mortal: **el EV barajado
(+130%) es MAYOR que el real (+89%)**. ¿Cómo? La estrategia acierta sus pocos trades caros
(0.7-0.8, pago chico) y falla algunos baratos (pago grande). El azar, al barajar, a veces
pone los aciertos en los baratos → cobra más. **La estrategia asigna sus aciertos PEOR que
el azar respecto al pago.** Su WR alto es del periodo, no de skill.

### 3. `latency_arb` — sigue el movimiento de Binance
**Lógica:** versión on_event del anterior — cuando Binance se mueve >0.04% en la ventana,
compra el lado que ese movimiento favorece. Idéntico patrón: WR 72%, 68% en 0.5, EV barajado
(+69%) > real (+58%). Ya vimos en el análisis de latencia que además **muere a ≥30ms** porque
el book se corrige en la misma ráfaga de timestamp. Doble condena: ni señal real ni capturable.

### 4. `midband_no` — solo-NO en banda estrecha
**Lógica:** solo vende NO, en precio 0.50-0.65, con gate de RSI y drift. La más conservadora.
**Estructura:** el caso más puro del problema — **87% de trades en 0.5**, WR 57% vs 50% fair
(+7pp). EV real +11% ≈ EV barajado +12%. Cuando casi el 100% de los trades están al mismo
precio, barajar es matemáticamente inocuo: el EV solo cuenta victorias. `p=1.000` garantizado.

### 5. `btc_binary` — score de momentum (el más engañoso)
**Lógica:** construye un score compuesto con el movimiento intra-ventana (`win_pct =
(close − window_open)`) + momentum de 2-3 candles + RSI. Apuesta cuando el score supera un
umbral. **WR 80%, EV +231%** — el más "ganador" de todos.
**Estructura:** gaps ENORMES en todo precio (+20 a +54pp; WR 100% a precio 0.8). Parece el
santo grial. Pero NO es lookahead clásico — usa el `close` de la candle de decisión, no el
cierre futuro. El gap refleja **momentum intra-ventana que ya está incorporado en el precio
del token**: cuando BTC subió en la ventana, el token YES ya cotiza caro, y apostar al
movimiento ya ocurrido "acierta" pero a un precio que descuenta ese movimiento. **EV barajado
(+306%) > real (+231%)** → otra vez, la asignación de aciertos es peor que el azar. WR 80% es
el periodo + el descuento del precio, no edge.

---

## El hilo común (la lección)

Las 5 comparten la misma firma de **falso edge**:
1. **Concentran los trades en precio ~0.5** (50-87%). Donde el precio es casi constante, el
   EV depende solo del número de aciertos, no de cuáles → el shuffle no puede distinguirlas
   del azar.
2. **El WR alto es del periodo**, no de predicción. En esos 33 días el lado elegido a 0.5
   ganó >50%. En otro periodo, perderá.
3. **El EV barajado iguala o supera al real** en las 5. La prueba definitiva: si reasignar al
   azar cuál trade ganó da el mismo resultado, no hay relación señal↔acierto.

---

## Qué necesitaría una estrategia para PASAR Leg 3

El validador no es imposible de pasar — exige una propiedad concreta que ninguna de estas
tiene: **los aciertos deben estar correlacionados con el precio, concentrados donde el pago
es mayor (los baratos).**

Un EDGE real se ve así (medido con datos sintéticos de control):
```
  CASO A — aciertos en los BARATOS:   WR 55%  EV +69%  Leg3 p=0.000  ✅ PASA
  CASO B — WR alto pero PLANO (65%):   WR 64%  EV +73%  Leg3 p=0.42   ❌ FALLA
```
El Caso A pasa con WR de solo 55% — **menos que las 5 de arriba** — porque sus aciertos están
*donde importa*: en los precios bajos de pago alto. Barajarlos los mueve a precios caros y el
EV cae bajo el real → la correlación señal↔precio es real y el azar no la replica.

**En concreto, una estrategia ganadora en Polymarket 5m necesitaría:**
- **Variar el precio de entrada** (no clavarse en 0.5): entrar barato cuando la señal es
  fuerte, y ahí ganar desproporcionadamente.
- **WR que SUBA al bajar el precio**: ganar 60% a precio 0.3 (fair 30% → +30pp donde el pago
  es 3:1) vale infinitamente más que ganar 71% a 0.5.
- **Una señal con poder predictivo genuino sobre el resultado**, no un sesgo del periodo. En
  un mercado calibrado 5m, esa señal no existe direccionalmente — por eso 0 de 59 pasaron.

La consecuencia: el edge en estos mercados **no es direccional**. Es estructural (maker /
rewards), donde no se necesita predecir *qué* gana, sino cobrar por proveer liquidez. Esa es
la única veta que el validador deja viva.
