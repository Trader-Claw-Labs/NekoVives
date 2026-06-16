/**
 * Human-readable metadata for the strategy-core engine kinds. Used by
 * Backtesting + LiveStrategies to render the same description card under the
 * engine selector so non-experts can tell what each motor actually does
 * before configuring it.
 */

export interface EngineKindMeta {
  /** Canonical id used by the backend (RunnerConfig.kind / BacktestRunBody.kind). */
  id: string
  /** Short label rendered in the dropdown. */
  label: string
  /** One-sentence summary that completes the sentence "This engine…". */
  summary: string
  /** Longer paragraph shown in the info card. */
  description: string
  /** Concrete example of when to pick this engine. */
  example: string
  /** Risk profile chip rendered next to the label. */
  risk: 'low' | 'medium' | 'high'
}

export const ENGINE_KINDS: EngineKindMeta[] = [
  {
    id: 'arb_binary',
    label: 'Arb Binary',
    summary: 'buys YES and NO at the same time when YES + NO < $1 − fees, locking in a guaranteed payout.',
    description:
      'A pure arbitrage scanner for Polymarket binary markets. It opens a YES leg and a NO leg only when the combined cost is below $1 (minus the fee buffer), so the $1 settlement payout is locked in regardless of which side wins.',
    example: 'Use for high-liquidity recurring markets (BTC 5m UP/DOWN) where pricing imbalances appear and close within seconds.',
    risk: 'low',
  },
  {
    id: 'fair_value',
    label: 'Fair Value',
    summary: 'estimates a probability from candles and fades the market when its quote drifts far from that estimate.',
    description:
      'Builds a fair-value (FV) probability from VWAP, volume balance and recent calibration error, then bets the side whose quoted price is cheaper than FV by more than the configured edge threshold.',
    example: 'Use when you trust the candle-derived signal more than the order book — e.g. low-volume Polymarket markets where the book is thin but Binance is liquid.',
    risk: 'medium',
  },
  {
    id: 'fv_momentum',
    label: 'FV + Momentum',
    summary: 'only takes Fair Value bets when short-term Binance momentum agrees with the FV direction.',
    description:
      'Adds an AND-gate on top of Fair Value: the trade only fires when the FV edge AND a short-window momentum signal point the same way. Closes positions when either FV converges or momentum stalls.',
    example: 'Safer than plain Fair Value during ranging markets; expect fewer trades but higher win rate.',
    risk: 'medium',
  },
  {
    id: 'rotation_compounder',
    label: 'Rotation Compounder',
    summary: 'ranks open markets by Kelly-adjusted score and rotates capital into the best one.',
    description:
      'Scores every market in the pool with a Kelly-criterion proxy (edge, time-to-resolve, drawdown). When a different market beats the current allocation by `switch_threshold`, capital rotates over.',
    example: 'Use when running multiple recurring series at once and you want a single bankroll to chase the best edge automatically.',
    risk: 'high',
  },
  {
    id: 'arb_hedge',
    label: 'Arb + Hedge Overlay',
    summary: 'arb engine plus a directional hedge that opens when the arb leg drifts against you.',
    description:
      'Starts as Arb Binary but adds a counter-position hedge once an open leg is `hedge_trigger_pct` underwater, capping drawdown on positions that would otherwise need to wait until expiry.',
    example: 'Use on longer-dated markets where pure arb capital can sit underwater for hours and you want a defensive overlay.',
    risk: 'medium',
  },
  {
    id: 'minting_mm',
    label: 'Minting MM',
    summary: 'mints YES+NO from collateral, sells both sides above parity, redeems the rest at expiry.',
    description:
      'Posts liquidity by minting CTF complementary tokens from USDC and quoting both sides slightly above parity. Profits from the spread; unsold inventory redeems 1:1 at market resolution.',
    example: 'Best on slow-moving markets with a stable spread (e.g. weekly sports or politics) — not for fast 5-minute crypto windows.',
    risk: 'high',
  },
  {
    id: 'rewards_maker',
    label: 'Liquidity Rewards Maker',
    summary: 'keeps a two-sided resting quote alive on a reward market to farm Polymarket liquidity rewards.',
    description:
      'Continuously posts a BUY YES + BUY NO pair near the mid, re-posting whichever side fills and re-centering when the mid drifts. Earns Polymarket liquidity rewards (paid in USDC at midnight UTC) for providing bilateral liquidity — it does NOT predict direction. The manual approach fails because a human can\'t keep the pair alive; this engine does.',
    example: 'Use ONLY on slow markets (politics, SpaceX FDV, far-dated events) where adverse selection is low. NEVER on crypto 5m/15m (toxic). Start in Dry Run.',
    risk: 'low',
  },
  {
    id: 'rewards_orchestrator',
    label: 'Liquidity Rewards Orchestrator (auto)',
    summary: 'auto-picks the top-N safe reward markets, quotes both sides on each, closes + rotates when one turns toxic.',
    description:
      'The hands-off pilot. Every poll it re-scans the reward-market list, auto-selects the top-N markets that clear your minimum safety (excluding toxic ones), and keeps a two-sided resting quote alive on each. When a held market turns toxic, drops below min-safety, or expires, it closes those quotes and rotates capital into the next-best fresh market. You only pick a wallet, assign capital, and set the pool size — the engine selects the slugs.',
    example: 'This is the autonomous rewards pilot: assign capital, set pool size + min-safety, and let it run. Start in Dry Run to measure eligible% before going Live.',
    risk: 'low',
  },
]

export function engineKindMeta(id: string): EngineKindMeta | undefined {
  return ENGINE_KINDS.find((e) => e.id === id)
}

/** Short label used in dropdowns: "Arb Binary — synthetic arb YES+NO". */
export function engineKindOptionLabel(id: string): string {
  const m = engineKindMeta(id)
  if (!m) return id
  return `${m.label} — ${m.summary.replace(/\.$/, '')}`
}

export const RISK_COLORS: Record<EngineKindMeta['risk'], string> = {
  low: 'var(--color-accent)',
  medium: '#f59e0b',
  high: 'var(--color-danger)',
}
