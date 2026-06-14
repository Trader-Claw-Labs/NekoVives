# Respuesta al análisis del quant externo sobre el validador

> Un quant externo revisó el validador de 3 legs y argumentó que (a) Leg 3 da falsos
> negativos en estrategias de precio constante, (b) deberíamos modelar EV maker, y
> (c) reemplazar Leg 3 por un "Leg 4" de time-shift. Evaluamos cada punto **con datos
> reales**, no con retórica. Conclusión: acierta en 2 de 4, su solución (Leg 4) está
> rota, y el análisis conjunto destapó la causa real (lookahead de fin-de-ventana).

## ✅ Donde el quant ACIERTA

### 1. Leg 3 es ciego a edge de precio constante (CONFIRMADO)
Prueba directa: a precio fijo 0.50, Leg 3 (shuffle-null) da `p=1.000` tanto con WR 57%
(edge real) como con 50% (sin edge). No los distingue. **Es una limitación real:** el
shuffle solo mide correlación precio↔acierto; a varianza-de-precio cero queda ciego.
El quant identificó esto correctamente.

### 2. El WR persiste en walk-forward (CONFIRMADO)
Split-half cronológico sobre los trades reales:
- drift_v1:    WR train 62.0% → test 62.0%  (clavado)
- midband_no:  WR train 53.7% → test 54.4%
No es el patrón de "suerte que se desvanece". Exigir mirar esto fue correcto.

## ❌ Donde el quant SE EQUIVOCA

### 3. Su "Leg 4" (time-shift) NO resuelve nada — falla por la MISMA ceguera
Corrimos el time-shift circular que propone sobre los trades reales de midband_no:
`p_shift = 1.000`. **Su propia solución reprueba la estrategia que dice rescatar.**
Razón matemática: a precio constante el EV depende solo del *conteo* de aciertos, no de
su orden ni su timing. Barajar (Leg 3) y desplazar (Leg 4) ambos conservan el conteo →
ambos dan p=1.0. El Leg 4 tiene idéntico punto ciego que critica.

### 4. El modelo Maker no era el problema
EV recalculado con modelo maker (sin taker fee + captura de spread δ=1¢, que **medimos**
en el stream: spread mediana 2¢ en el cluster 0.45-0.55, δ capturable ≈1¢ — el quant
acertó el número):

| Estrategia | EV taker | EV maker ideal | EV maker −8pp adverse |
|---|---|---|---|
| drift_v1 | +27.4% | +30.8% | +20.2% |
| midband_no | +10.8% | +13.7% | +4.9% |
| late_certainty | +89.0% | +107.6% | +88.4% |

Todas ya eran "rentables" en taker. El maker solo suma puntos. La discusión maker/taker
es un **desvío** — no cambia la pregunta de fondo. (Nota: el quant ignora la **selección
adversa** del fill maker — una orden resting se llena cuando el flujo va contra ti; lo
modelamos con −3 a −8pp de WR. Aun así sigue positivo, lo que confirma que el problema
no es la ejecución.)

## 🎯 La causa REAL (que ni el quant ni nuestro análisis previo habían aislado)

El test correcto para edge de precio constante **no es Leg 3 ni Leg 4**, sino el
**base-rate condicional del mercado**: ¿con qué probabilidad gana YES, condicionado al
precio, sobre TODAS las ventanas?

> **Resultado: a precio 0.50, el mercado resuelve YES exactamente 50% (8,367 ventanas).**
> Calibración perfecta.

Pero drift_v1 "gana 74%" en el bucket fino 0.49-0.51 (precio medio 50%). Contradicción
imposible bajo azar. La mecánica temporal la explica:

- archive_candles decide a **T+240-300s de una ventana de 300s** (últimos 0-60s).
- A esa altura BTC ya se movió casi toda la ventana, y `token_drift = P4(T+300)−P3(T+180)`
  **ya contiene la información del resultado** que se confirma 60s después.
- El WR 71% no es predicción ni suerte: es **lookahead suave de fin-de-ventana** — entran
  cuando el resultado ya está casi determinado pero el precio nominal aún dice 0.50.

**drift_v1, late_certainty y latency_arb son la misma familia:** explotan ese lookahead
temporal. Por eso el WR es estable (efecto estructural, no aleatorio) Y no es edge real:
en vivo el book ya se ajustó a esos 60s finales — exactamente lo que el latency sweep
mató (muere a ≥30ms).

## Acciones (lo correcto, sin "rescatar" lo que no es edge)

1. **Documentar la limitación de Leg 3** (ciego a precio constante). El quant tiene razón.
2. **Añadir un Leg de base-rate condicional** — el único que funciona a precio fijo y el
   que destapó el lookahead. (No el time-shift, probado igual de ciego.)
3. **Arreglar el lookahead de fin-de-ventana** en archive_candles: la decisión a 60s del
   cierre infla el WR con info casi-resuelta. Mover la decisión más temprano (p.ej. T+60s)
   o medir el token_drift con un P3/P4 que no solape la resolución.

El quant mejoró nuestro entendimiento del validador. Pero su veredicto ("hay edges
legítimos que Leg 3 mata") es incorrecto para estas 5: el WR estable era lookahead, no
señal — y su propio Leg 4 también las reprueba.
