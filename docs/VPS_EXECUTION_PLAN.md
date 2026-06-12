# VPS Execution Plan — $100 pilot → $2,000 scale (validation-gated)

> VPS: tradingvps.io, EWR/Newark, 4GB/1vCPU. /book RTT ≈ 110ms (NOT HFT regime).
> Objetivo honesto: terminar el mes con una estrategia de YIELD POSITIVO VALIDADA y
> capital intacto — NO un target de profit fijo. $5k/mes requiere ~$30-100k de capital
> a tasas sostenibles, o riesgo-lotería. Perseguir el número = el incidente de mayo.

## Respuestas a las 6 inquietudes

### 1) Binance orderbook imbalance → Polymarket trade, a 110ms — ¿vale la pena?
**Probablemente NO, pero es backtesteable antes de arriesgar $1.** Esto es exactamente la
tesis stale-book de `basis_analysis`: el edge vive sub-segundo. A 110ms de RTT por lectura
+ ~110ms de envío de orden = ~220ms, los bots HFT (bonereaper) ya corrigieron el precio que
ves. Seríamos exit liquidity más rápida, no ganadores. **PERO** tenemos 33-41 días de ticks
con `binance_price` embebido — podemos medir el edge EXACTO antes de creer. Gate: backtest de
la señal de imbalance sobre los ticks reales; si EV taker > 0 tras el fee 1.8% → recién ahí
se considera. Apostar a esto sin el backtest = repetir el error de fe.

### 2) late_certainty con $100 — ¿ejecutar ya?
**NO sin backtest primero.** late_certainty fue descartado por P&L real onchain de
−$642/−$816. Su "edge" era payout-explosion (long-shots a precios extremos que pierden) —
NO un problema de latencia, sino de edge fantasma. A 110ms su ventana de 30-45s es menos
sensible a latencia, pero eso no arregla el edge inexistente. Gate: re-correr el
edge_validator de 3 legs sobre los ticks; solo si pasa los 3 → Dry Run en la VPS → live.

### 3) 4GB / 1vCPU, escalable
**Suficiente para un engine ligero.** Un solo runner (rewards_maker o un script) corre de
sobra en 4GB/1vCPU. El build de Rust necesita más RAM (compilar en la VPS puede fallar con
4GB) → compilar local/CI y subir el binario. Escalar después si las pruebas validan.

### 4) Desplegar solo el engine ligero, no todo NekoVives
**Sí — y es la arquitectura correcta para un colo box.** Plan: binario headless `nv-runner`
que corre UN engine + script, sin dashboard web, sin LLM, deps mínimas. Config por archivo/
env. Logs a stdout. ~1 proceso, <100MB RAM. Se controla por SSH. Mantiene NekoVives completo
en tu laptop para análisis/UI; la VPS solo ejecuta.

### 5) WebSocket + FIFO matching para bajar los 110ms
**Distinción técnica importante:** el WS de Polymarket es para DATOS DE MERCADO (book/trades
empujados en tiempo real), NO para enviar órdenes. Reduce la latencia de VER el book (bueno),
pero el envío de orden sigue siendo REST firmado (EIP-712) a ~110ms — ese piso no baja con WS.
La matching engine es price-time (FIFO) sobre el book, pero TÚ no controlas eso; solo mandas
una orden firmada y el engine la matchea. Conclusión:
- **Para sniping taker (idea #1): el WS NO nos hace competitivos** — el piso de envío se queda.
- **Para el MAKER (rewards_maker): el WS SÍ ayuda mucho** — ves tus fills al instante y
  re-cotizas más rápido, reduciendo el tiempo que quedas un-sided. Mejora real ahí.

### 6) Dataset de 4 días (pmxt.dev) — ya lo tenemos y MÁS
Tenemos **33-41 días** de ticks 1Hz (BTC/ETH/SOL/XRP 5m+15m) con binance_price + book depth.
Es el insumo para backtestear las ideas #1 y #2 HONESTAMENTE antes de tocar capital.

---

## El plan por fases (gated — cada gate decide si se avanza)

### Fase 0 — Backtest de la verdad (1-2 días, CERO capital) ⟵ EMPEZAR AQUÍ
Sobre los 33-41 días de ticks reales:
- **0a:** Backtest de la señal "Binance imbalance → Polymarket" (idea #1). Mide EV taker tras
  fee 1.8% a latencia simulada de 110-220ms (entrada retardada). Gate: EV/trade > 0 con n≥500.
- **0b:** edge_validator de 3 legs sobre late_certainty (idea #2) con resolución oficial.
  Gate: PASS en los 3 legs.
- **Si AMBOS fallan** (lo más probable según toda la evidencia) → NO se despliega trading
  direccional. Se procede solo con rewards_maker (lo único validado estructuralmente).

### Fase 1 — Deploy ligero en VPS (1 día)
- Binario `nv-runner` headless (un engine + config por archivo).
- Corre el **rewards_maker LIVE con los $100 ya posteados** desde la VPS (uptime 24/7 +
  menor latencia de cancelación que tu laptop). Esto YA está validado como estructural.
- Si Fase 0 dio luz verde a alguna direccional → ese script en Dry Run en paralelo.

### Fase 2 — Medición real (3-5 días)
- `/rewards/user/markets` (API oficial) → cuánto reward genera de verdad por día.
- Si rewards_maker en $100 mantiene eligible ~100% Y earnings > costo de adverse selection
  → **escalar a $2,000** repartido en 2-3 mercados lentos no-tóxicos (diversificar el pool
  share). Gate: earnings netos positivos medidos, no asumidos.

### Fase 3 — Escala condicional ($2,000)
- Solo si Fase 2 mostró yield neto positivo real.
- Guardrails: portfolio_guard a -20% (más estricto con más capital), max por mercado,
  re-quote por WS.
- Expectativa honesta: ~$200-600/mes sobre $2k en el mejor caso de rewards. NO $5k.

---

## Regla de la casa (la que nos ha salvado)
Ningún capital se escala por un TARGET de profit. Se escala por un GATE de validación
cumplido. El profit es consecuencia de un edge real medido, no una meta que se persigue.
Perseguir el número = mayo 2026 = −$10,351.
