# Búsqueda de edge en BTC 5m sobre datos limpios — resultado

> Tras arreglar el bug de contaminación multi-mercado (ver `TICK_CONTAMINATION_BUG.md`),
> corrimos un feature-sweep sistemático sobre los ticks LIMPIOS de btc_5m (33 días) para
> ver si queda algún edge direccional. Resultado: el único candidato que apareció era un
> **artefacto de lookahead** del binance_price. El mercado 5m sigue siendo eficiente.

## Método (honesto, no curve-fitting)

En vez de escribir otra estrategia a mano (lo que falló 59 veces), barrimos features y
medimos cuáles predicen la resolución **por encima del fair value que el precio ya implica**
(residuo = resultado − yes_mid). Features probadas: token drift, hora UTC, calibración de
precio, y momentum de Binance intra-ventana.

- **token drift, hora UTC, calibración de precio**: ruido puro (z ≈ 0). El token está
  perfectamente calibrado (mid 0.5 → P(YES)=51%, 0.3→30%, 0.7→70% …).
- **momentum de Binance intra-ventana**: señal ENORME (z ≈ ±20). Único candidato.

## El candidato y por qué parecía real

"momentum-fade @240s": a 4min del cierre, si Binance ya se movió >0.04% desde el window_open,
comprar el lado favorecido. Pasó 5 tests que matan falsos edges:
1. WR 77%, EV +50%/trade a 5s de latencia de fill.
2. Bate el fair value en TODOS los buckets de precio (+15 a +32pp).
3. Walk-forward: EV+ estable en las 6 semanas (no es periodo afortunado).
4. corr(binance_chg@240s, yes_mid) = 0.05 → el token "no precifica" el momentum (parecía
   ineficiencia del mercado).
5. El edge persistía decenas de segundos (no moría a ms como el arb de paridad).

## Por qué es ARTEFACTO (el 6º test lo mató)

`binance_price` proviene de **klines de Binance de 1 MINUTO** (`fetch_binance_prices`,
orderbook_parser.py:601-633): el `close` de cada vela de 1min se difunde a los 60 segundos
de ese minuto. Confirmado en los datos: una ventana de 300s tiene solo **6 valores distintos
de binance_price** (no ~300).

Las ventanas arrancan en múltiplos de 60s (window_ts mod 60 == 0 en el 100%). La señal a
240s lee el `binance_price` del minuto [wt+60, wt+119] — cuyo close **no se conoce hasta
wt+119, que es 59 segundos DESPUÉS de la decisión**. Es lookahead de 59s en el 100% de las
ventanas.

**Prueba definitiva:** recalculando con solo el minuto YA cerrado (sin el futuro), la
estrategia dispara **0 trades** — el 100% de las señales venían de información futura. El
"corr=0.05 (mercado lento)" era ilusorio: el book no puede correlacionar con un close de
Binance que aún no ocurrió. El book a 240s YA es eficiente (su propio signo predice la
resolución 65% sin ninguna feature externa).

## Conclusión

- **No hay edge direccional en BTC 5m**, ni siquiera sobre datos limpios. Reconfirma la
  eficiencia del mercado (0 EDGE de 59 + base-rate calibrado).
- **Tercer artefacto de datos documentado** (tras contaminación multi-mercado y lookahead
  de fin-de-ventana). Patrón común: features de momentum sobre `binance_price` son
  traicioneras porque el dato es de 1min, no de 1s.
- **Para medir momentum intra-ventana honestamente** haría falta ingerir trades/klines de
  Binance a resolución de segundo con timestamp real, NO el close de 1min difundido. Hasta
  entonces, cualquier feature que use binance_price para momentum sub-minuto está contaminada.
- El edge sigue estando, si acaso, en **maker/rewards** (estructural, no direccional).
