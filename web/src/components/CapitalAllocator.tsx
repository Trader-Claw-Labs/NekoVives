import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import { PieChart, RefreshCw, ChevronDown, ChevronUp } from 'lucide-react'

// Honest capital allocator: weight ∝ validated EV / CI-width. Only EDGE-verdict
// runners get capital; NO_EDGE/INSUFFICIENT get 0%. Sizes on confirmed edge, not P&L.
interface Alloc {
  name: string; n: number; verdict: string
  ev_per_trade_pct: number; ci_lo: number; ci_hi: number; weight_pct: number
}

export default function CapitalAllocator() {
  const [open, setOpen] = useState(false)
  const { data, isFetching, refetch } = useQuery<{ allocations: Alloc[] }>({
    queryKey: ['capital-allocator'],
    queryFn: () => apiFetch('/api/capital/allocator'),
    enabled: open,
    staleTime: 5 * 60_000,
  })
  const allocs = data?.allocations ?? []
  const withEdge = allocs.filter(a => a.weight_pct > 0)

  return (
    <div className="rounded-lg border mb-4" style={{ background: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
      <button onClick={() => setOpen(o => !o)} className="w-full flex items-center gap-2 px-4 py-2.5">
        <PieChart size={15} style={{ color: 'var(--color-accent)' }} />
        <span className="text-sm font-semibold" style={{ color: 'var(--color-text)' }}>Capital Allocator</span>
        <span className="text-[11px]" style={{ color: 'var(--color-text-muted)' }}>
          (weight ∝ validated edge / confidence — only EDGE runners get capital)
        </span>
        <span className="ml-auto">{open ? <ChevronUp size={14} /> : <ChevronDown size={14} />}</span>
      </button>
      {open && (
        <div className="px-4 pb-3">
          <div className="flex justify-end mb-2">
            <button onClick={() => refetch()} className="p-1 rounded hover:bg-white/10" style={{ color: 'var(--color-text-muted)' }}>
              <RefreshCw size={12} className={isFetching ? 'animate-spin' : ''} />
            </button>
          </div>
          {allocs.length === 0 ? (
            <p className="text-xs text-center py-3" style={{ color: 'var(--color-text-muted)' }}>
              No runners with ≥30 official-resolution trades yet. Allocations appear once runners accumulate validated history.
            </p>
          ) : (
            <>
              {withEdge.length === 0 && (
                <p className="text-xs mb-2" style={{ color: '#f59e0b' }}>
                  ⚠ No runner currently passes validation — allocator recommends 0% capital to all. This is the honest answer.
                </p>
              )}
              <div className="space-y-1">
                {allocs.map((a, i) => (
                  <div key={i} className="flex items-center gap-2 text-xs">
                    <span className="w-40 truncate" style={{ color: 'var(--color-text)' }}>{a.name}</span>
                    <span className="px-1.5 py-0.5 rounded text-[10px] font-semibold" style={{
                      background: a.verdict === 'EDGE' ? 'rgba(74,222,128,0.15)' : 'rgba(239,68,68,0.12)',
                      color: a.verdict === 'EDGE' ? '#4ade80' : '#f87171',
                    }}>{a.verdict}</span>
                    <span className="font-mono" style={{ color: 'var(--color-text-muted)' }}>
                      n={a.n} EV={a.ev_per_trade_pct >= 0 ? '+' : ''}{a.ev_per_trade_pct.toFixed(1)}%
                    </span>
                    {/* weight bar */}
                    <div className="flex-1 h-3 rounded overflow-hidden" style={{ background: 'var(--color-surface-2)' }}>
                      <div className="h-full" style={{ width: `${a.weight_pct}%`, background: 'var(--color-accent)' }} />
                    </div>
                    <span className="w-12 text-right font-mono font-semibold" style={{ color: a.weight_pct > 0 ? 'var(--color-accent)' : 'var(--color-text-muted)' }}>
                      {a.weight_pct.toFixed(0)}%
                    </span>
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  )
}
