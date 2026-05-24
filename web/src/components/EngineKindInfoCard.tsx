/**
 * Compact, dismissable info card that explains the currently selected engine
 * kind. Rendered under the Strategy Engine dropdown on both Backtesting and
 * LiveStrategies so a non-expert sees what the engine does before they spend
 * time tuning its parameters.
 */

import { Info } from 'lucide-react'
import { engineKindMeta, RISK_COLORS } from './engineKindMeta'

interface Props {
  kind: string
}

export default function EngineKindInfoCard({ kind }: Props) {
  const meta = engineKindMeta(kind)
  if (!meta) return null

  const riskLabel = meta.risk[0].toUpperCase() + meta.risk.slice(1)

  return (
    <div
      className="mt-2 rounded border px-3 py-2 text-[11px] leading-snug"
      style={{
        backgroundColor: 'var(--color-surface)',
        borderColor: 'var(--color-border)',
        color: 'var(--color-text)',
      }}
    >
      <div className="flex items-center gap-2 mb-1">
        <Info size={12} style={{ color: 'var(--color-text-muted)' }} />
        <span className="font-semibold">{meta.label}</span>
        <span
          className="px-1.5 py-0.5 rounded text-[9px] uppercase tracking-wider"
          style={{
            backgroundColor: RISK_COLORS[meta.risk],
            color: meta.risk === 'low' ? '#000' : '#fff',
          }}
          title="Subjective risk profile based on hedging, allocation cap and exposure to drawdown"
        >
          {riskLabel} risk
        </span>
      </div>
      <p style={{ color: 'var(--color-text-muted)' }}>{meta.description}</p>
      <p className="mt-1" style={{ color: 'var(--color-text-muted)' }}>
        <span className="font-semibold" style={{ color: 'var(--color-text)' }}>When to use: </span>
        {meta.example}
      </p>
    </div>
  )
}
