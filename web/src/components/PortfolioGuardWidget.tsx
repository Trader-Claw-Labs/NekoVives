import { useQuery, useMutation } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { ShieldAlert, Octagon } from 'lucide-react'

// Wallet-level portfolio guard widget. Shows the REAL Polymarket balance vs baseline,
// drawdown, and a manual "Halt All Live" button. The backend guard auto-halts at -50%.
interface GuardStatus {
  live_runners_running: number
  baseline_usdc: number
  current_usdc: number
  drawdown_pct: number
  halt_threshold_pct: number
  status: 'OK' | 'WARNING' | 'BREACH'
}

export default function PortfolioGuardWidget() {
  const { data, refetch } = useQuery<GuardStatus>({
    queryKey: ['portfolio-guard'],
    queryFn: () => apiFetch('/api/portfolio-guard/status'),
    refetchInterval: 60_000,
  })

  const haltMutation = useMutation({
    mutationFn: () => apiPost('/api/live/stop-all-live', {}),
    onSuccess: () => refetch(),
  })

  if (!data) return null
  const color = data.status === 'BREACH' ? '#ef4444' : data.status === 'WARNING' ? '#f59e0b' : 'var(--color-accent)'
  // Only show prominently if there are live runners or a drawdown
  const hasLive = data.live_runners_running > 0

  return (
    <div className="rounded-lg border p-3 mb-4 flex items-center gap-4"
      style={{ background: 'var(--color-surface)', borderColor: data.status === 'OK' ? 'var(--color-border)' : color }}>
      <ShieldAlert size={16} style={{ color }} />
      <div className="flex items-center gap-4 text-xs flex-1 flex-wrap">
        <span style={{ color: 'var(--color-text)' }}>
          Portfolio Guard: <span style={{ color, fontWeight: 600 }}>{data.status}</span>
        </span>
        <span style={{ color: 'var(--color-text-muted)' }}>
          Wallet: <span className="font-mono" style={{ color: 'var(--color-text)' }}>${data.current_usdc.toFixed(2)}</span>
          {data.baseline_usdc > 0 && <span> / baseline ${data.baseline_usdc.toFixed(2)}</span>}
        </span>
        {data.baseline_usdc > 0 && (
          <span style={{ color: 'var(--color-text-muted)' }}>
            Drawdown: <span className="font-mono" style={{ color: data.drawdown_pct > 0 ? '#f87171' : 'var(--color-accent)' }}>
              {data.drawdown_pct.toFixed(1)}%
            </span> / halt at {data.halt_threshold_pct}%
          </span>
        )}
        <span style={{ color: 'var(--color-text-muted)' }}>
          {data.live_runners_running} live running
        </span>
      </div>
      {hasLive && (
        <button
          onClick={() => { if (confirm('Halt ALL live runners now?')) haltMutation.mutate() }}
          disabled={haltMutation.isPending}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs font-semibold"
          style={{ background: 'rgba(239,68,68,0.15)', color: '#ef4444', border: '1px solid #ef4444' }}
        >
          <Octagon size={13} /> Halt All Live
        </button>
      )}
    </div>
  )
}
