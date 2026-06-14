# Cómo funciona el validador de edge (3 legs) — explicación a fondo

> Para qué sirve: separar **"gané dinero en el backtest"** (un hecho del pasado) de
> **"tengo una estrategia que ganará en el futuro"** (una afirmación sobre habilidad
> repetible). Son cosas distintas. Un backtest puede mostrar +89% de retorno y aun así
> ser una máquina de perder en vivo. El validador es el filtro que detecta eso ANTES de
> arriesgar capital.
>
> Código: `src/tools/edge_validator.rs`. Se corre sobre los trades de cualquier backtest
> (`validate_edge: true` en la API, o `trader-claw validate-all` para todas las estrategias).

---

## El problema que resuelve

Imagina que tiras una moneda 33 veces y sale cara 23 (70%). ¿Tienes una moneda mágica?
No — tuviste un periodo afortunado. El **conteo (23/33) es un hecho real**, pero la
**conclusión ("moneda mágica") es falsa**.

Un backtest es exactamente esto: corre tu estrategia sobre un periodo histórico y reporta
el P&L. Si ese periodo fue favorable a tu sesgo, el backtest muestra ganancias **reales pero
no repetibles**. El validador existe para responder: *¿el resultado viene de una señal
predictiva, o de la suerte de este periodo concreto?*

---

## La base: EV por trade

Cada trade binario tiene un **precio de entrada** `p` (entre 0 y 1) y un **resultado**
(ganó / perdió). El P&L por cada $1 apostado (`ev1` en el código):

- **Si ganó:** `(1/p) × (1 − fee) − 1`. Compraste un token a `p` que paga $1 → tu apuesta
  se multiplica por `1/p`, menos el fee crypto-taker (`1.8% × p × (1−p)`).
- **Si perdió:** `−1` (pierdes el stake completo).

El **EV observado** = promedio de ese P&L sobre todos tus trades. Positivo = "en promedio
ganaste". Los 3 legs interrogan ese número desde tres ángulos independientes. **Se exige
pasar los tres** (`edge = leg1 && leg2 && leg3`).

> Guardia previa: si tienes menos de **30 trades**, el veredicto es `INSUFFICIENT` — muestra
> demasiado chica para concluir nada. (No es ni EDGE ni NO_EDGE: simplemente no hay datos.)

---

## Leg 1 — Bootstrap CI: *"¿tu EV es robusto, o lo cargan unos pocos golpes de suerte?"*

**Qué hace:** re-muestrea tus N trades **con reemplazo** 5000 veces. Cada vez arma una
cartera alternativa (algunos trades repetidos, otros ausentes) y calcula su EV. Eso da una
distribución de 5000 EVs posibles. Toma los percentiles 2.5% y 97.5% → un **intervalo de
confianza del 95%**.

**Pasa si:** `ci_lo > 0` — incluso el escenario pesimista (percentil 2.5) sigue siendo
positivo.

**Qué refuta:** que tu EV positivo dependa de **2-3 trades enormes con suerte**. Si tu
ganancia viene de pocos outliers, al re-muestrear caerán muchas carteras sin ellos → el
`ci_lo` baja de cero → falla. Con miles de trades, normalmente pasa: es el test más fácil.

---

## Leg 2 — Random-side null: *"¿le ganas a alguien que tira una moneda a tus mismos precios?"*

**Qué hace:** 5000 veces simula un apostador **sin habilidad** que, entrando a **tus mismos
precios**, gana cada trade con probabilidad 50/50. Calcula su EV. El p-valor `p_random` =
fracción de veces que ese apostador iguala o supera tu EV.

**Pasa si:** `p_random < 0.05` — tu EV bate al ≥95% de los apostadores aleatorios.

**Qué refuta:** que tu "edge" sea solo el **sesgo de las cuotas**. Entrar siempre a 0.10
(cuotas 10:1) da pagos grandes en las pocas victorias; un random a esos precios ya tiene
cierto EV sin predecir nada. Leg 2 pregunta: *¿tu selección de lado aporta algo sobre tirar
una moneda a esos precios?* Si tu WR > 50%, normalmente pasa.

> **Trampa clave:** pasar Leg 1 y Leg 2 se siente como "tengo edge" — WR alto, EV robusto,
> le gano al azar. Es exactamente donde el 99% de los traders se autoengaña. Falta el test
> que de verdad importa.

---

## Leg 3 — Shuffled-outcome null: *"¿tu señal predice CUÁLES ganan, o solo cosechaste un periodo favorable?"*

**Qué hace:** toma tus **resultados reales** (los win/loss exactos que tuviste) y **baraja
cuál trade ganó** (Fisher-Yates), manteniendo fijos los precios y el número total de
aciertos. 5000 veces. El p-valor `p_shuffle` = fracción en que el barajado iguala o supera
tu EV.

**Pasa si:** `p_shuffle < 0.05`.

**Qué refuta** (y es el que reprueba a casi todo): que tu ganancia venga de **qué ventanas
acertaste en este periodo**, no de habilidad. Si barajar tus aciertos da el mismo EV o
mejor, entonces **no hay vínculo entre tu señal y qué ganó** — ganaste porque el periodo
favoreció tu sesgo de precios, no porque predijiste algo.

### Lo crucial: qué hace pasar Leg 3 (verificado con datos)

Barajar mantiene **fijos los precios** y reasigna los aciertos. El EV de un trade depende de
a qué **precio** cayó el acierto. Entonces:

- **PASA** cuando tus aciertos están **concentrados en los precios baratos** (pago alto):
  tu señal predice específicamente esas ventanas. Barajar mueve los aciertos a precios caros
  (pago chico) → el EV barajado **cae** bajo el tuyo → `p < 0.05`. Hay correlación
  señal↔precio que el azar no replica. *(Ejemplo medido: WR 55% pero aciertos en los
  baratos → `p=0.000` ✅.)*
- **FALLA** cuando tu WR es **alto pero plano** (mismo % de acierto en todo precio, o todos
  los trades al mismo precio): el EV solo depende de *cuántos* ganaste, no de *cuáles* →
  barajar no cambia nada → `p≈1.0`. *(Ejemplo medido: WR 64% plano → `p=0.42` ❌.)*

`p_shuffle = 1.000` (el peor caso) significa que el barajado **siempre** iguala o supera tu
EV — tu secuencia de aciertos no tiene absolutamente nada de especial.

---

## Por qué los 3 juntos son difíciles de engañar

| Leg | Mantiene fijo | Aleatoriza | Caza... |
|-----|---------------|------------|---------|
| 1 | tus trades | cuáles entran (resample) | EV+ que cargan pocos outliers |
| 2 | tus precios | el resultado (50/50) | "edge" que es solo sesgo de cuotas |
| 3 | tus resultados + precios | cuál trade ganó | aciertos sin relación con la señal |

**Leg 2 y Leg 3 son inmunes a artefactos del backtest** (precios stale, sintéticos, fees mal
puestos), porque tanto tu estrategia como el null ven *los mismos precios y resultados* — lo
único que cambia es la pieza que cada leg aleatoriza. Si tu estrategia no le gana a su propia
versión aleatorizada, no hay edge que defender.

**Verdicto = EDGE solo si pasan los 3.** Está diseñado para **rechazar barato**, no para
aprobar. Es deliberadamente estricto: un EDGE aquí es una licencia para un piloto pequeño,
no una garantía. Un NO_EDGE es "no comprometas capital".

---

## Backtest ≠ predicción (la regla de oro)

El backtest responde **"¿cuánto habría ganado en el pasado?"** — y su número es correcto.
NO responde **"¿cuánto ganaré?"**. La traducción pasado→futuro la hace el validador:

- Backtest +89% → hecho histórico (ganaste en esos días).
- Leg 3 `p=1.000` → **no es repetible** (no despliegues).

Un backtest positivo es necesario pero **no suficiente**. La secuencia completa antes de
arriesgar capital: **backtest positivo → EDGE en los 3 legs → sobrevive walk-forward (OOS)
→ sobrevive el latency-sweep a tu latencia real → dry-run en paper ≈ simulado → pilot
pequeño con guardrails.** Cada paso mata barato.
