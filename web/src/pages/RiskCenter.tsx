import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { AlertTriangle, Play, Pause, Shield } from 'lucide-react'
import clsx from 'clsx'

interface RiskStatus {
  status: 'ok' | 'halted' | 'disabled'
  total_capital?: number
  drawdown_pct?: number
  daily_pnl_pct?: number
  open_positions?: number
  message?: string
}

function useRiskStatus() {
  return useQuery<RiskStatus>({
    queryKey: ['risk-status'],
    queryFn: () => apiFetch<RiskStatus>('/api/risk/status'),
    refetchInterval: 5_000,
  })
}

export default function RiskCenter() {
  const queryClient = useQueryClient()
  const { data: status, isLoading } = useRiskStatus()
  const [actionError, setActionError] = useState<string | null>(null)

  const haltMutation = useMutation({
    mutationFn: () => apiPost('/api/risk/halt', {}),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['risk-status'] }),
    onError: (e: Error) => setActionError(e.message),
  })

  const resumeMutation = useMutation({
    mutationFn: () => apiPost('/api/risk/resume', {}),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['risk-status'] }),
    onError: (e: Error) => setActionError(e.message),
  })

  const halted = status?.status === 'halted'
  const disabled = status?.status === 'disabled'

  return (
    <div className="p-6 max-w-2xl mx-auto">
      <div className="flex items-center gap-3 mb-6">
        <Shield size={22} style={{ color: 'var(--color-accent)' }} />
        <h1 className="text-xl font-bold" style={{ color: 'var(--color-text)' }}>
          Risk Center
        </h1>
      </div>

      {actionError && (
        <div
          className="mb-4 rounded-lg border px-4 py-3 text-sm flex items-center gap-2"
          style={{
            backgroundColor: 'rgba(255,90,90,0.08)',
            borderColor: 'var(--color-danger)',
            color: 'var(--color-danger)',
          }}
        >
          <AlertTriangle size={16} />
          {actionError}
        </div>
      )}

      {isLoading && (
        <div className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
          Loading risk status...
        </div>
      )}

      {disabled && (
        <div
          className="rounded-lg border px-4 py-3 text-sm"
          style={{
            backgroundColor: 'rgba(255,170,0,0.06)',
            borderColor: 'rgba(255,170,0,0.3)',
            color: 'var(--color-text-muted)',
          }}
        >
          <AlertTriangle size={16} className="inline mr-2" />
          {status?.message ?? 'Trading risk gate is not initialized.'}
        </div>
      )}

      {!isLoading && !disabled && status && (
        <>
          {/* Status cards */}
          <div className="grid grid-cols-2 gap-4 mb-6">
            <div
              className="rounded-xl border p-4"
              style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
            >
              <div className="text-xs font-medium uppercase tracking-widest mb-1" style={{ color: 'var(--color-text-muted)' }}>
                Gate Status
              </div>
              <div className="flex items-center gap-2">
                <span
                  className={clsx('inline-block w-2.5 h-2.5 rounded-full', halted ? 'bg-red-500' : 'bg-emerald-500')}
                />
                <span className="font-bold text-lg" style={{ color: 'var(--color-text)' }}>
                  {halted ? 'HALTED' : 'ACTIVE'}
                </span>
              </div>
            </div>

            <div
              className="rounded-xl border p-4"
              style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
            >
              <div className="text-xs font-medium uppercase tracking-widest mb-1" style={{ color: 'var(--color-text-muted)' }}>
                Open Positions
              </div>
              <div className="font-bold text-lg" style={{ color: 'var(--color-text)' }}>
                {status.open_positions ?? 0}
              </div>
            </div>

            <div
              className="rounded-xl border p-4"
              style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
            >
              <div className="text-xs font-medium uppercase tracking-widest mb-1" style={{ color: 'var(--color-text-muted)' }}>
                Daily PnL %
              </div>
              <div
                className="font-bold text-lg"
                style={{
                  color:
                    (status.daily_pnl_pct ?? 0) >= 0
                      ? 'var(--color-success)'
                      : 'var(--color-danger)',
                }}
              >
                {(status.daily_pnl_pct ?? 0).toFixed(2)}%
              </div>
            </div>

            <div
              className="rounded-xl border p-4"
              style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
            >
              <div className="text-xs font-medium uppercase tracking-widest mb-1" style={{ color: 'var(--color-text-muted)' }}>
                Drawdown %
              </div>
              <div
                className="font-bold text-lg"
                style={{
                  color:
                    (status.drawdown_pct ?? 0) > 5
                      ? 'var(--color-danger)'
                      : 'var(--color-text)',
                }}
              >
                {(status.drawdown_pct ?? 0).toFixed(2)}%
              </div>
            </div>
          </div>

          {/* Action buttons */}
          <div className="flex gap-4">
            <button
              onClick={() => {
                setActionError(null)
                haltMutation.mutate()
              }}
              disabled={halted || haltMutation.isPending}
              className={clsx(
                'flex-1 flex items-center justify-center gap-2 rounded-lg px-4 py-3 font-bold text-sm transition-colors',
                halted || haltMutation.isPending
                  ? 'opacity-50 cursor-not-allowed'
                  : 'hover:brightness-110'
              )}
              style={{
                backgroundColor: halted ? 'var(--color-border)' : 'var(--color-danger)',
                color: '#fff',
              }}
            >
              <Pause size={16} />
              HALT TRADING
            </button>

            <button
              onClick={() => {
                setActionError(null)
                resumeMutation.mutate()
              }}
              disabled={!halted || resumeMutation.isPending}
              className={clsx(
                'flex-1 flex items-center justify-center gap-2 rounded-lg px-4 py-3 font-bold text-sm transition-colors',
                !halted || resumeMutation.isPending
                  ? 'opacity-50 cursor-not-allowed'
                  : 'hover:brightness-110'
              )}
              style={{
                backgroundColor: !halted ? 'var(--color-border)' : 'var(--color-success)',
                color: '#fff',
              }}
            >
              <Play size={16} />
              RESUME TRADING
            </button>
          </div>
        </>
      )}
    </div>
  )
}
