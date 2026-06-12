# Research: Web3 Portfolio Manager + Solana Memecoin Trading

> Investigación de viabilidad (jun 2026) para dos features candidatos. Aplica la
> metodología validation-first que nos salvó en Polymarket: separar el edge mecánico
> del predictivo, modelar costos ANTES de construir, y backtest sin survivorship bias.

---

## Feature 1 — Web3 Portfolio Manager (bull-run rebalancer)

### Veredicto: ✅ VIABLE — pero NO como se planteó originalmente

La idea original ("comprar/vender en fracciones pequeñas pero frecuentes, monitorear
cada segundo") **muere por matemática de costos**, no por falta de señal:

| Parámetro | Valor real |
|-----------|-----------|
| Costo round-trip Uniswap (EVM) | ~0.3% pool fee + gas ($1-30 según red) |
| Costo round-trip Jupiter (Solana) | ~0.1-0.3% + slippage |
| Trading "frecuente" (ej. 20 trades/día al 0.2%) | **−4%/día en costos** = −70%/mes |
| Evidencia de rebalanceo óptimo (backtest Shrimpy) | **Umbral del 15%** de desviación — supera a HODL por 77% y a rebalanceo frecuente |

**El insight de la literatura:** el rebalanceo gana cuando es **por umbral, no por reloj**.
Rebalancear cuando un asset se desvía >15% de su peso objetivo = ~2-10 trades/mes, no
20/día. Los modelos regime-aware además **ensanchan** los umbrales en bull (dejan correr
ganadores) — lo contrario de tradear más seguido.

### La versión con evidencia a favor

1. **Detector de régimen (el "activar en bull run")** — señal lenta y robusta:
   BTC > 200d MA + breadth (% de top-20 sobre su 50d MA) + drawdown desde ATH.
   Esto es backtesteable HOY con nuestra infra de Binance klines.
2. **Rotación por momentum cross-sectional** — rankear top-N majors por momentum
   12-1 semanas, sobreponderar ganadores. Documentado en crypto (edge decae pero existe).
3. **Rebalanceo por umbral (15%)** con costos modelados (fee+gas+slippage por red).
4. **Monitoreo cada segundo SÍ — pero para ALERTAS y riesgo** (stop de régimen,
   drawdown guard), no para tradear. Trading: diario/horario como máximo.

### Qué ya tenemos (alto reuso)
- `evm-trader` (Uniswap), `solana-trader` (Jupiter swaps), `wallet-manager` (claves),
  endpoints `/api/wallets/{quote,swap,transfer}`, balances multi-chain.
- `fetch_candles` (Binance klines) + motor de backtest → el regime detector y el
  momentum scoring se backtestean con lo que ya existe.
- Guardrails + portfolio guard (patrón ya probado).

### Plan por fases (gate entre cada una)
- **P1 — Backtest (1 semana):** regime detector + momentum rotation + threshold
  rebalancer sobre 3+ años de klines, CON costos reales por red. Gate: ¿supera a
  HODL BTC/ETH después de costos en bull Y protege en bear? Si no → no construir.
- **P2 — Paper portfolio (2 semanas):** runner `portfolio_manager` kind que simula
  con precios live. Mismo patrón Dry Run de siempre.
- **P3 — Live chico** con guardrails (max % por trade, max trades/día, kill switch).

### Riesgos honestos
- El momentum crypto **decae** (cada vez más fondos lo arbitran). El edge esperado es
  modesto: el valor real del feature es **disciplina + protección de régimen**, no alpha.
- Gas en EVM mainnet hace inviable rebalancear posiciones <$2-5k; priorizar
  Solana/L2s o tamaños grandes.

### RESULTADO P1 BACKTEST (2026-06, `scripts/ml/portfolio_backtest.py`)

Probado con klines diarios reales, costo 30bps/trade, 8 majors. **El resultado depende
CRÍTICAMENTE de si el periodo incluye un bear market:**

| Periodo | Estrategia (net) | HODL BTC | Sharpe estrat. vs BTC | Lectura |
|---------|------------------|----------|----------------------|---------|
| 2023-06 → 2026-06 (puro bull) | **−16% a +5%** | +46% | 0.16-0.34 vs 0.56 | ❌ Pierde — costos de rotación sin beneficio defensivo |
| ~2022 → 2026 (incluye bear) | **+204%** | +154% | **0.78 vs 0.75** | ✅ Gana risk-adjusted — el regime gate protegió en 2022 |

**Veredicto honesto:** el feature NO es un generador de alpha; es un **protector de
régimen con costo**. En bull puro, HODL gana (no pagues por rotar). Su único valor es
**evitar el bear** — y eso solo se cobra cuando hay bear. maxDD de la estrategia (-57%)
sigue siendo PEOR que HODL (-51%) porque la rotación entre alts añade volatilidad.

**Recomendación ajustada:** construir el feature SOLO como **"regime-protected HODL"** —
regime gate (BTC>200d SMA) que rota a USDC en bear, SIN la capa de momentum-rotation de
alts (que añade costo y drawdown sin mejorar Sharpe). Más simple, defendible, y honesto
con el usuario: "esto no te hace ganar más en bull, te saca antes del bear." Mejor mensaje
que prometer alpha que no existe.

### ✅ VEREDICTO FINAL — la versión simple GANA (`--simple`)

| Periodo | Estrategia | HODL BTC | Sharpe | maxDD | Trades |
|---------|-----------|----------|--------|-------|--------|
| Con bear (2022+) | +164% | +155% | **0.87 vs 0.75** | **−34% vs −51%** | 36 |
| Puro bull | +36% | +46% | 0.52 vs 0.56 | **−34% vs −51%** | 32 |

Regime-protected HODL BTC **recorta el maxDD de −51% a −34% en ambos periodos** (la mejora
de riesgo más importante), con Sharpe mejor-o-igual y solo ~36 trades en 4 años. La capa de
momentum-rotation de alts queda DESCARTADA (añadía costo y drawdown). El feature a construir
es: regime gate sobre 1 asset (BTC, o BTC+ETH), rota a USDC bajo el SMA. Honesto y útil.

---

## Feature 2 — Trading de memecoins en Solana

### Veredicto: ⚠️ VIABLE EL BACKTESTING; el edge depende de CUÁL estrategia (2 de 3 familias mueren al analizarlas)

### Los datos del campo (2025-2026)
- pump.fun: **40-50k lanzamientos/día**, 6M+ tokens acumulados.
- **Supervivencia <8% a 60 días**; 97% pierde desde el pico. → **El survivorship bias
  es LA trampa metodológica**: un backtest que solo incluya tokens vivos infla
  retornos masivamente. Cualquier backtest serio DEBE incluir tokens muertos/ruggeados.
- Bots sniper: 87% de trades rentables, hasta $6.8M/mes un solo bot, ejecución
  **sub-segundo** tras el evento de liquidez.
- Paper académico (arXiv 2601.08641): bots de manipulación generan actividad falsa
  **específicamente para cebar copy-traders**; 62.9% de tokens explotados tuvieron
  wash trading previo.

### Datos históricos onchain para backtest — SÍ existen
| Fuente | Qué da |
|--------|--------|
| **Bitquery (GraphQL)** | pump.fun completo: lanzamientos, trades, OHLCV, bonding curve progress, **migraciones a PumpSwap/Raydium**, top traders. Histórico + streams |
| **Birdeye API** | OHLCV histórico agregado de 50+ DEXs de Solana |
| DexScreener/GeckoTerminal | precios/pares complementarios |

### Las 3 familias de estrategia, evaluadas

**a) Sniping de lanzamientos — ❌ NO para nosotros.**
Es el juego de latencia sub-segundo otra vez (= bonereaper). Los bots establecidos
ejecutan en <1s desde el evento de liquidez. A nuestra latencia somos la exit liquidity.

**b) Copy-trading de wallets "smart" — ⚠️ Solo con defensas extra.**
Tenemos la herramienta perfecta (el wallet validator 3-legs), PERO el ecosistema
memecoin tiene bots que **fabrican track records** con wash trading para cebar
copiadores. Requiere añadir al validator: detección de wash trading (self-trades,
volumen circular), y exigir n alto + ventanas largas. Posible, no trivial.

**c) Momentum de graduación (pump.fun → Raydium/PumpSwap) — ✅ LA candidata.**
La "graduación" (bonding curve completa → listing en DEX) es un **evento estructural
observable** con minutos de ventana (no ms). Hipótesis backtesteable: los tokens que
gradúan con ciertas características (holders, velocidad de curva, distribución) tienen
drift post-listing explotable. Bitquery tiene el dataset completo de migraciones.
Es event-driven, no latency-critical, y validatable con nuestro edge_validator.

### Plan por fases (gate entre cada una)
- **P1 — Dataset + backtester sin survivorship bias (1-2 semanas):**
  Ingestar de Bitquery TODOS los lanzamientos de un periodo (vivos Y muertos),
  con OHLCV post-graduación. Construir `memecoin_backtest` que simule entradas/salidas
  con slippage real (los books son finos) y el rug como outcome posible.
  **Gate: el dataset debe incluir ≥90% de los tokens del periodo, no solo los vivos.**
- **P2 — Validar familia (c) graduación:** features (curve velocity, holder count,
  sniper concentration, dev wallet %) → señal → edge_validator 3-legs sobre el
  backtest. Gate: EDGE en los 3 legs con n≥500.
- **P3 — Si (c) valida: paper runner** con feed live de graduaciones (Bitquery stream).
- **P4 — Copy-trading (b) solo después**, añadiendo wash-trading detection al validator.

### Riesgos honestos
- Mercado adversarial por diseño: ruggers, manipuladores y snipers son la mayoría
  del flujo. El backtest DEBE asumir peor-caso de ejecución.
- Bitquery histórico a esta escala es de pago (estimar costo de API antes de P1).
- Aún con edge validado, el sizing debe ser pequeño: cola izquierda brutal (rug = −100%).

### RESULTADO P1 BACKTEST (2026-06, `scripts/ml/memecoin_backtest.py`)

Datos: GeckoTerminal (gratis, sin key — Bitquery requiere pago). 101 pools de Solana
(new/trending/top, 35% ya muertos = vol24h<$1k). Familia (c) graduación-momentum,
entrada en bar 3, slippage 3%/lado:

| Hold | n | mean | **median** | win% | best | worst |
|------|---|------|-----------|------|------|-------|
| 30min | 52 | +15.7% | **−5.5%** | 27% | +539% | −50% |
| 60min | 52 | +17.6% | **−6.2%** | 31% | +662% | −61% |
| 120min | 52 | +19.0% | **−6.6%** | 27% | +958% | −100% (rug) |

**Veredicto: ✗ NO EDGE explotable.** Firma clásica de **lotería de cola derecha**: mean
positivo inflado por 1-2 tokens (+539%/+958%), pero **mediana negativa y win rate 27-31%**
= el trade típico PIERDE. No es estrategia, es comprar billetes de lotería con EV que
parece positivo solo por outliers irrepetibles.

**Sesgo adicional crítico:** GeckoTerminal trending/top YA está sesgado hacia arriba (son
los que sobrevivieron lo suficiente para aparecer). Con el firehose real de lanzamientos
(92% muere <60d), el resultado sería marcadamente PEOR. Este es un **upper bound optimista**
y aun así no tiene edge en la mediana.

**Recomendación final memecoins: NO construir trading de memecoins.** Las 3 familias caen:
sniping (latencia, fuera de alcance), copy (track records falsos), graduación (lotería sin
edge mediano). El único uso defendible sería **monitoreo/alertas** (no trading) si el
usuario quiere seguir el mercado — pero no como estrategia de profit validada.

---

## Recomendación de orden

1. **Feature 1 - P1 (backtest del portfolio manager)** primero: reusa 100% la infra,
   cero costo de datos, 1 semana, y el regime detector sirve también de gate para todo
   lo demás (no tradear memecoins en bear, p.ej.).
2. **Feature 2 - P1 (dataset sin survivorship bias)** en paralelo si el costo de
   Bitquery es razonable; es el prerequisito de cualquier verdad sobre memecoins.
3. Nada de ejecución real en ninguno hasta pasar sus gates — la regla de la casa.

## Fuentes
- [Bitquery Pump.fun API](https://docs.bitquery.io/docs/blockchain/Solana/Pumpfun/Pump-Fun-API/) · [Migraciones PumpSwap](https://docs.bitquery.io/docs/blockchain/Solana/Pumpfun/pump-fun-to-pump-swap/)
- [Birdeye](https://birdeye.so/) · [Memecoin Statistics 2026 — CoinLaw](https://coinlaw.io/memecoin-statistics/)
- [Resisting Manipulative Bots in Meme Coin Copy Trading (arXiv 2601.08641)](https://arxiv.org/html/2601.08641v2)
- [Market Manipulations in the Meme Coin Ecosystem (arXiv 2507.01963)](https://arxiv.org/pdf/2507.01963)
- [Crypto Portfolio Rebalancing — Zignaly/Shrimpy data](https://zignaly.com/crypto-trading/risk-management/cryptocurrency-portfolio-rebalancing)
- [Systematic Crypto Strategies: Momentum & Regime Filtering](https://medium.com/@briplotnik/systematic-crypto-trading-strategies-momentum-mean-reversion-volatility-filtering-8d7da06d60ed)
