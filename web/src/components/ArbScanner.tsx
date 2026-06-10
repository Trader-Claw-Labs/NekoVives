import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import { TrendingUp, RefreshCw, AlertTriangle, ChevronDown, ChevronUp } from 'lucide-react'

// Structural-arb scanner — surfaces set-arb (disjoint covers) and monotonicity violations
// across SLOW Polymarket events (no HFT competition). PRE-FEE gross edges; verify book
// depth + NegRisk fees before posting. Discovery tool, not auto-execute.
interface Leg { market_slug: string; action: string; price: number; token_id: string }
interface Candidate {
  kind: string; event_title: string; event_slug: string
  n_markets: number; gross_edge_c: number; legs: Leg[]; note: string
}
interface ScanResp {
  candidates: Candidate[]; scanned_events_max: number; threshold_c: number; fetched_at: string
}

export default function ArbScanner() {
  const [maxEvents, setMaxEvents] = useState(100)
  const [thresholdC, setThresholdC] = useState(0.5)
  const [expanded, setExpanded] = useState<Record<string, boolean>>({})

  const { data, isFetching, refetch, error } = useQuery<ScanResp>({
    queryKey: ['arb-scan', maxEvents, thresholdC],
    queryFn: () => apiFetch(`/api/arb/scan?max_events=${maxEvents}&threshold_c=${thresholdC}`),
    staleTime: 60_000,
  })

  const cands = data?.candidates ?? []

  return (
    <div className="rounded-lg border p-4 mb-4"
      style={{ background: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
      <div className="flex items-center gap-2 mb-1">
        <TrendingUp size={16} style={{ color: 'var(--color-accent)' }} />
        <span className="text-sm font-semibold" style={{ color: 'var(--color-text)' }}>
          Structural Arb Scanner
        </span>
      </div>
      <p className="text-[11px] mb-3" style={{ color: 'var(--color-text-muted)' }}>
        Detects (1) disjoint <strong>bucket sets</strong> where YES asks sum &lt; $1 (or NO asks &lt; $1),
        and (2) <strong>monotonicity violations</strong> on date-ordered cumulative strikes.
        Slow events only (no HFT). <span style={{ color: 'var(--color-warning)' }}>Pre-fee gross edge</span> —
        verify book depth, NegRisk fees, and tightness before posting.
      </p>

      <div className="flex items-center gap-3 mb-3">
        <label className="flex items-center gap-1.5 text-xs" style={{ color: 'var(--color-text-muted)' }}>
          Events
          <input type="number" min={5} max={500} value={maxEvents}
            onChange={e => setMaxEvents(Number(e.target.value) || 100)}
            className="w-20 rounded border px-2 py-1 text-xs"
            style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}/>
        </label>
        <label className="flex items-center gap-1.5 text-xs" style={{ color: 'var(--color-text-muted)' }}>
          Min edge
          <input type="number" min={0} max={50} step={0.1} value={thresholdC}
            onChange={e => setThresholdC(Number(e.target.value) || 0)}
            className="w-16 rounded border px-2 py-1 text-xs"
            style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}/>
          ¢
        </label>
        <button onClick={() => refetch()}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs font-medium"
          style={{ background: 'var(--color-surface-2)', color: 'var(--color-text)' }}>
          <RefreshCw size={13} className={isFetching ? 'animate-spin' : ''} /> Scan
        </button>
        {data && (
          <span className="text-xs ml-auto" style={{ color: 'var(--color-text-muted)' }}>
            {cands.length} candidates · {data.scanned_events_max} events
          </span>
        )}
      </div>

      {error && (
        <div className="text-xs p-2 rounded mb-2" style={{ background: 'rgba(239,68,68,0.1)', color: '#f87171' }}>
          <AlertTriangle size={12} className="inline" /> {(error as Error).message}
        </div>
      )}

      {!isFetching && cands.length === 0 && (
        <div className="text-xs text-center py-4" style={{ color: 'var(--color-text-muted)' }}>
          No candidates above {thresholdC}¢ right now. Try lowering the threshold or scanning more events.
        </div>
      )}

      <div className="space-y-2">
        {cands.map((c, i) => {
          const key = `${c.event_slug}-${c.kind}-${i}`
          const open = expanded[key]
          const kindLabel = c.kind === 'set_arb_long' ? 'SET ARB (long)'
            : c.kind === 'set_arb_short' ? 'SET ARB (short)' : 'MONOTONICITY'
          return (
            <div key={key} className="rounded border overflow-hidden"
              style={{ borderColor: 'var(--color-border)', background: 'var(--color-surface-2)' }}>
              <button
                onClick={() => setExpanded(e => ({ ...e, [key]: !open }))}
                className="w-full flex items-center gap-3 px-3 py-2 text-left hover:bg-white/5"
              >
                <span className="text-[10px] font-bold px-1.5 py-0.5 rounded shrink-0"
                  style={{ background: c.kind.startsWith('set_arb') ? 'var(--color-accent)' : '#f59e0b', color: '#000' }}>
                  {kindLabel}
                </span>
                <span className="text-xs flex-1 truncate" style={{ color: 'var(--color-text)' }}>
                  {c.event_title}
                </span>
                <span className="text-xs font-mono shrink-0" style={{ color: 'var(--color-accent)' }}>
                  +{c.gross_edge_c.toFixed(2)}¢
                </span>
                <span className="text-[10px] shrink-0" style={{ color: 'var(--color-text-muted)' }}>
                  {c.n_markets} legs
                </span>
                {open ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
              </button>
              {open && (
                <div className="px-3 py-2 border-t text-[11px]"
                  style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
                  <div className="mb-2">{c.note}</div>
                  {c.legs.map((l, li) => (
                    <div key={li} className="flex gap-2 font-mono py-0.5">
                      <span style={{ color: l.action.includes('SELL') || l.action.includes('NO') ? '#f87171' : 'var(--color-accent)' }}>
                        {l.action}
                      </span>
                      <span>@ {l.price.toFixed(3)}</span>
                      <span className="truncate" style={{ color: 'var(--color-text)' }}>· {l.market_slug}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}
