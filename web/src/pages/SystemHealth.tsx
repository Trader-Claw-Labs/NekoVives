import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import { Heart, RefreshCw, DollarSign, Stethoscope, CheckCircle, AlertCircle, XCircle, Zap } from 'lucide-react'

// ── Types ─────────────────────────────────────────────────────────────

interface ComponentHealth {
  status: 'ok' | 'warn' | 'error' | 'unknown'
  message?: string
  latency_ms?: number
}

interface HealthSnapshot {
  uptime_seconds: number
  components: Record<string, ComponentHealth>
}

interface CostSummary {
  session_cost_usd: number
  daily_cost_usd: number
  monthly_cost_usd: number
  total_tokens: number
  request_count: number
  by_model: Record<string, { tokens: number; cost_usd: number; requests: number }>
}

interface DiagResult {
  name: string
  severity: 'ok' | 'warn' | 'error' | 'info'
  message: string
}

// ── Helpers ───────────────────────────────────────────────────────────

function fmtUptime(secs: number): string {
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  const s = Math.floor(secs % 60)
  if (h > 0) return `${h}h ${m}m`
  if (m > 0) return `${m}m ${s}s`
  return `${s}s`
}

function fmtUsd(v: number): string {
  return `$${v.toFixed(4)}`
}

function statusIcon(s: string) {
  if (s === 'ok') return <CheckCircle size={13} style={{ color: 'var(--color-accent)' }} />
  if (s === 'warn' || s === 'info') return <AlertCircle size={13} style={{ color: '#f59e0b' }} />
  if (s === 'error') return <XCircle size={13} style={{ color: 'var(--color-danger)' }} />
  return <AlertCircle size={13} style={{ color: 'var(--color-text-muted)' }} />
}

function fmtTok(usd: number): string {
  const tokens = Math.round(usd / 0.000003)
  if (tokens === 0) return '0'
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(0)}K`
  return tokens.toLocaleString()
}

function statusColor(s: string) {
  if (s === 'ok') return 'var(--color-accent)'
  if (s === 'warn' || s === 'info') return '#f59e0b'
  if (s === 'error') return 'var(--color-danger)'
  return 'var(--color-text-muted)'
}

// ── Sections ──────────────────────────────────────────────────────────

function HealthPanel() {
  const { data, isLoading, refetch } = useQuery<{ health: HealthSnapshot }>({
    queryKey: ['health'],
    queryFn: () => apiFetch<{ health: HealthSnapshot }>('/api/health'),
    refetchInterval: 10_000,
  })

  const health = data?.health
  const components = Object.entries(health?.components ?? {})
  const allOk = components.every(([, c]) => c.status === 'ok')

  return (
    <div className="rounded-lg border" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
      <div className="flex items-center justify-between px-4 py-3 border-b" style={{ borderColor: 'var(--color-border)' }}>
        <div className="flex items-center gap-2">
          <Heart size={14} style={{ color: 'var(--color-accent)' }} />
          <h2 className="text-sm font-semibold">System Health</h2>
          {health && (
            <span className="text-xs px-2 py-0.5 rounded"
              style={{ backgroundColor: allOk ? 'rgba(74,222,128,0.1)' : 'rgba(239,68,68,0.1)', color: allOk ? 'var(--color-accent)' : 'var(--color-danger)' }}>
              {allOk ? 'All systems OK' : 'Issues detected'}
            </span>
          )}
        </div>
        <button onClick={() => refetch()} className="p-1.5 rounded hover:bg-white/5"
          style={{ color: 'var(--color-text-muted)' }}>
          <RefreshCw size={12} className={isLoading ? 'animate-spin' : ''} />
        </button>
      </div>

      {health && (
        <div className="px-4 py-3 border-b text-xs flex gap-6" style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
          <div>Uptime: <span className="font-semibold" style={{ color: 'var(--color-text)' }}>{fmtUptime(health.uptime_seconds)}</span></div>
          <div>Components: <span className="font-semibold" style={{ color: 'var(--color-text)' }}>{components.length}</span></div>
        </div>
      )}

      {isLoading ? (
        <div className="p-6 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>Loading...</div>
      ) : components.length === 0 ? (
        <div className="p-6 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>No component data</div>
      ) : (
        <div className="divide-y" style={{ borderColor: 'var(--color-border)' }}>
          {components.map(([name, comp]) => (
            <div key={name} className="flex items-center justify-between px-4 py-2.5 text-sm">
              <div className="flex items-center gap-2">
                {statusIcon(comp.status)}
                <span className="font-mono text-xs">{name}</span>
              </div>
              <div className="flex items-center gap-3 text-xs">
                {comp.latency_ms !== undefined && (
                  <span style={{ color: 'var(--color-text-muted)' }}>{comp.latency_ms}ms</span>
                )}
                {comp.message && (
                  <span style={{ color: 'var(--color-text-muted)' }} className="max-w-48 truncate">{comp.message}</span>
                )}
                <span style={{ color: statusColor(comp.status) }}>{comp.status}</span>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function CostPanel() {
  const { data, isLoading } = useQuery<{ cost: CostSummary }>({
    queryKey: ['cost'],
    queryFn: () => apiFetch<{ cost: CostSummary }>('/api/cost'),
    refetchInterval: 30_000,
  })

  const cost = data?.cost
  const byModel = Object.entries(cost?.by_model ?? {})

  return (
    <div className="rounded-lg border" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
      <div className="flex items-center gap-2 px-4 py-3 border-b" style={{ borderColor: 'var(--color-border)' }}>
        <DollarSign size={14} style={{ color: 'var(--color-accent)' }} />
        <h2 className="text-sm font-semibold">LLM Cost</h2>
      </div>

      {isLoading ? (
        <div className="p-6 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>Loading...</div>
      ) : (
        <div className="p-4 space-y-4">
          {/* Summary cards — tokens first, no dollar signs */}
          <div className="grid grid-cols-4 gap-3">
            {[
              { label: 'Total Tokens', value: (cost?.total_tokens ?? 0).toLocaleString() },
              { label: 'Requests', value: (cost?.request_count ?? 0).toLocaleString() },
              { label: 'Session tok', value: fmtTok(cost?.session_cost_usd ?? 0) },
              { label: 'Today tok', value: fmtTok(cost?.daily_cost_usd ?? 0) },
            ].map(s => (
              <div key={s.label} className="rounded p-3 text-center"
                style={{ backgroundColor: 'var(--color-base)' }}>
                <div className="text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>{s.label}</div>
                <div className="text-sm font-bold" style={{ color: 'var(--color-accent)' }}>{s.value}</div>
              </div>
            ))}
          </div>

          {/* Per-model breakdown */}
          {byModel.length > 0 && (
            <div>
              <div className="text-xs font-semibold mb-2" style={{ color: 'var(--color-text-muted)' }}>By Model</div>
              <div className="space-y-1.5">
                {byModel.map(([model, stats]) => (
                  <div key={model} className="flex items-center justify-between text-xs rounded px-3 py-1.5"
                    style={{ backgroundColor: 'var(--color-base)' }}>
                    <span className="font-mono truncate max-w-40">{model}</span>
                    <div className="flex gap-4" style={{ color: 'var(--color-text-muted)' }}>
                      <span>{stats.requests} req</span>
                      <span style={{ color: 'var(--color-accent)' }}>{stats.tokens.toLocaleString()} tok</span>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

function DoctorPanel() {
  const { data, isLoading, refetch } = useQuery<{ results: DiagResult[] }>({
    queryKey: ['doctor'],
    queryFn: () => apiFetch<{ results: DiagResult[] }>('/api/doctor'),
    refetchInterval: 60_000,
  })

  const results = data?.results ?? []
  const errors = results.filter(r => r.severity === 'error').length
  const warns = results.filter(r => r.severity === 'warn').length

  return (
    <div className="rounded-lg border" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
      <div className="flex items-center justify-between px-4 py-3 border-b" style={{ borderColor: 'var(--color-border)' }}>
        <div className="flex items-center gap-2">
          <Stethoscope size={14} style={{ color: 'var(--color-accent)' }} />
          <h2 className="text-sm font-semibold">Diagnostics</h2>
          {!isLoading && (
            <span className="text-xs" style={{ color: errors > 0 ? 'var(--color-danger)' : warns > 0 ? '#f59e0b' : 'var(--color-accent)' }}>
              {errors > 0 ? `${errors} error${errors > 1 ? 's' : ''}` : warns > 0 ? `${warns} warning${warns > 1 ? 's' : ''}` : 'All checks passed'}
            </span>
          )}
        </div>
        <button onClick={() => refetch()} className="p-1.5 rounded hover:bg-white/5"
          style={{ color: 'var(--color-text-muted)' }}>
          <RefreshCw size={12} className={isLoading ? 'animate-spin' : ''} />
        </button>
      </div>

      {isLoading ? (
        <div className="p-6 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>Running diagnostics...</div>
      ) : results.length === 0 ? (
        <div className="p-6 text-center">
          <Zap size={28} className="mx-auto mb-2" style={{ color: 'var(--color-text-muted)' }} />
          <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>No diagnostic results</p>
        </div>
      ) : (
        <div className="divide-y" style={{ borderColor: 'var(--color-border)' }}>
          {results.map((r, i) => (
            <div key={i} className="flex items-start gap-3 px-4 py-3 text-sm">
              <div className="mt-0.5 flex-shrink-0">{statusIcon(r.severity)}</div>
              <div className="flex-1 min-w-0">
                <div className="font-medium text-xs">{r.name}</div>
                <div className="text-xs mt-0.5 leading-relaxed" style={{ color: 'var(--color-text-muted)' }}>{r.message}</div>
              </div>
              <span className="text-xs flex-shrink-0" style={{ color: statusColor(r.severity) }}>{r.severity}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ── Main Page ─────────────────────────────────────────────────────────

export default function SystemHealth() {
  return (
    <div className="p-6 max-w-4xl mx-auto space-y-6">
      <div className="flex items-center gap-2 mb-2">
        <Heart size={18} style={{ color: 'var(--color-accent)' }} />
        <h1 className="text-lg font-bold">System Health</h1>
      </div>

      <HealthPanel />
      <CostPanel />
      <DoctorPanel />
    </div>
  )
}
