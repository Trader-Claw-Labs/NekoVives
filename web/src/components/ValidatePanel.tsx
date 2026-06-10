import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import { ShieldCheck, AlertTriangle, RefreshCw } from 'lucide-react'

// The trusted edge check. Runs the 3-leg validation on a runner's OFFICIAL-resolution
// trades — the honest replacement for the synthetic/stale backtest "edge" numbers.
interface RunnerLite { config: { id: string; name: string } }
interface LegResult {
  n: number; wr_pct: number; break_even_pct: number; ev_per_trade_pct: number
  ci_lo: number; ci_hi: number
  leg1_pass: boolean; p_random: number; leg2_pass: boolean
  p_shuffle: number; leg3_pass: boolean
  verdict: 'EDGE' | 'NO_EDGE' | 'INSUFFICIENT'; note: string
}

export default function ValidatePanel() {
  const [name, setName] = useState('')
  const [submitted, setSubmitted] = useState('')

  const { data: runnersData } = useQuery<{ runners: RunnerLite[] }>({
    queryKey: ['live-strategies-names'],
    queryFn: () => apiFetch('/api/live/strategies'),
    staleTime: 60_000,
  })
  const names = Array.from(new Set((runnersData?.runners ?? []).map(r => r.config.name)))

  const { data, isFetching, refetch } = useQuery<{ result: LegResult; runners_matched: number }>({
    queryKey: ['validate', submitted],
    queryFn: () => apiFetch(`/api/validate/runner?name=${encodeURIComponent(submitted)}`),
    enabled: submitted.length > 0,
  })

  const r = data?.result
  const verdictColor = r?.verdict === 'EDGE' ? 'var(--color-accent)'
    : r?.verdict === 'NO_EDGE' ? '#f87171' : 'var(--color-text-muted)'

  return (
    <div className="rounded-lg border p-4 mb-4"
      style={{ background: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
      <div className="flex items-center gap-2 mb-1">
        <ShieldCheck size={16} style={{ color: 'var(--color-accent)' }} />
        <span className="text-sm font-semibold" style={{ color: 'var(--color-text)' }}>
          Validate — real edge (3-leg test on official trades)
        </span>
      </div>
      <p className="text-[11px] mb-3" style={{ color: 'var(--color-text-muted)' }}>
        The only trusted "is the edge real?" check. Runs on a runner's <strong>official
        Polymarket resolution</strong> trades, priced at the realistic fill. Declares EDGE
        only if it survives a bootstrap CI, a random-outcome null, and a shuffle null —
        unlike the synthetic/stale backtest engines, which over-state edge.
      </p>

      <div className="flex items-center gap-2 mb-3">
        <select
          className="flex-1 rounded border px-2 py-1.5 text-xs"
          style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
          value={name}
          onChange={e => setName(e.target.value)}
        >
          <option value="">Select a runner…</option>
          {names.map(n => <option key={n} value={n}>{n}</option>)}
        </select>
        <button
          onClick={() => { setSubmitted(name); if (name === submitted) refetch() }}
          disabled={!name}
          className="px-3 py-1.5 rounded text-xs font-medium disabled:opacity-40"
          style={{ background: 'var(--color-accent)', color: '#000' }}
        >
          {isFetching ? <RefreshCw size={13} className="animate-spin" /> : 'Validate'}
        </button>
      </div>

      {r && (
        <div>
          <div className="flex items-baseline gap-3 mb-2">
            <span className="text-lg font-bold" style={{ color: verdictColor }}>
              {r.verdict === 'EDGE' ? '✓ EDGE' : r.verdict === 'NO_EDGE' ? '✗ NO EDGE' : '— INSUFFICIENT'}
            </span>
            <span className="text-[11px]" style={{ color: 'var(--color-text-muted)' }}>{r.note}</span>
          </div>

          {r.n >= 30 && (
            <>
              <div className="text-xs mb-2" style={{ color: 'var(--color-text)' }}>
                n={r.n} · WR={r.wr_pct.toFixed(1)}% · break-even={r.break_even_pct.toFixed(1)}% ·
                EV/trade=<span style={{ color: r.ev_per_trade_pct > 0 ? 'var(--color-accent)' : '#f87171' }}>{r.ev_per_trade_pct >= 0 ? '+' : ''}{r.ev_per_trade_pct.toFixed(1)}%</span>
              </div>
              <div className="grid gap-1 text-[11px]">
                <Leg pass={r.leg1_pass} label={`LEG 1 — Bootstrap 95% CI: [${r.ci_lo >= 0 ? '+' : ''}${r.ci_lo.toFixed(1)}%, ${r.ci_hi >= 0 ? '+' : ''}${r.ci_hi.toFixed(1)}%]`} />
                <Leg pass={r.leg2_pass} label={`LEG 2 — Random-outcome null: p=${r.p_random.toFixed(3)}`} />
                <Leg pass={r.leg3_pass} label={`LEG 3 — Shuffle null: p=${r.p_shuffle.toFixed(3)}`} />
              </div>
            </>
          )}
        </div>
      )}
    </div>
  )
}

function Leg({ pass, label }: { pass: boolean; label: string }) {
  return (
    <div className="flex items-center gap-1.5" style={{ color: pass ? 'var(--color-accent)' : '#f87171' }}>
      {pass ? <ShieldCheck size={12} /> : <AlertTriangle size={12} />}
      <span>{pass ? 'PASS' : 'FAIL'}</span>
      <span style={{ color: 'var(--color-text-muted)' }}>· {label}</span>
    </div>
  )
}
