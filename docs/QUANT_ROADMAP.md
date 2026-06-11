# NekoVives — Quant Roadmap (jun 2026)

> Posicionamiento: NekoVives es una **plataforma de validación de edge**, no de
> señales. Tras probar onchain (-$10.4k) que las estrategias direccionales en
> Polymarket 5m no tienen edge (mercado eficiente; el único edge ahí es
> latencia/HFT fuera de nuestro alcance), el foso de NekoVives es la honestidad
> estadística: nada va a Live sin pasar el validador de 3 legs.

## Las 3 recomendaciones de quant (orden de ROI)

### Rec 1 — Validation-first: nada a Live sin pasar Validate
**Estado:** parcial. El `edge_validator` (3 legs) existe y se usa en backtesting
(panel Validate) y en copy trading (bloqueo HFT). Falta hacerlo **bloqueante
universal**: ningún runner de cualquier tipo pasa a `mode=live` sin un veredicto
EDGE (o un override explícito con warning).

**Trabajo:**
- Gate en `POST /api/live/strategies` y en el PATCH a `mode=live`: si el runner
  tiene historial, correr edge_validator; bloquear si HFT/no-edge sin override.
- Badge de veredicto del validador en cada runner card de `/live`.

### Rec 2 — Activar los 3 edges estructurales (no-direccionales)
El único tipo de edge que sobrevivió todo el análisis: **mecánico, no predictivo**.

1. **Liquidity Rewards** — en validación (piloto $100 activo).
2. **`minting_mm`** — engine YA construido (853 líneas, modos BT/DryRun/Live).
   Mint 1 USDC → YES+NO, vende ambos a `mid + premium`, captura 2×premium/ciclo.
   ACCIÓN: validar su DryRun end-to-end, exponer como runner validable.
3. **Funding arb (Hyperliquid)** — delta-neutral, no compite con HFT crypto.
   Endpoint `/api/funding/comparison` existe. ACCIÓN: verificar si emite señal real.

### Rec 3 — Capital Allocator honesto
Asignar capital ∝ edge_validado × (1/varianza) (fractional Kelly sobre el EV del
validator, NO sobre P&L crudo). Cierra el loop validación→ejecución. Cada runner
ya tiene un veredicto 3-legs; automatizar la asignación.

## Lo que se quita / congela (AL FINAL, tras las 3 recs)
- Estrategias direccionales 5m (drift_*, hybrid_*) → `scripts/deprecated/`.
- EVM/Solana/TON swaps si no alimentan una estrategia (superficie de claves sin retorno).
- Confirmar Strategy Builder / Discovery (ya ocultos).

## Lo que NO se construye
- HFT / latency arb (bonereaper gana; fuera de alcance a nuestra latencia).
- Más estrategias direccionales (cualquier timeframe).
- ML predictivo de precio (mercado eficiente, probado 3×).
