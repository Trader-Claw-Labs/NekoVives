import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import {
  Bot, Plus, Trash2, RefreshCw, X, StopCircle, RotateCcw,
  TrendingUp, TrendingDown, Activity, ChevronDown, ChevronUp, AlertCircle,
} from 'lucide-react'
import clsx from 'clsx'

// ── Types ─────────────────────────────────────────────────────────────

interface RunnerConfig {
  id: string
  name: string
  script: string
  market_type: string
  symbol: string
  interval: string
  mode: string
  initial_balance: number
  fee_pct: number
  warmup_days: number
}

interface RunnerStatus {
  id: string
  status: 'starting' | 'running' | 'stopped' | 'error'
  started_at: string
  last_tick_at?: string
  next_tick_at?: string
  error?: string
}

interface RunnerResult {
  total_return_pct: number
  balance: number
  position: number
  total_trades: number
  win_rate_pct: number
  sharpe_ratio: number
  max_drawdown_pct: number
  last_signal: string
  analysis: string
}

interface StoredRunner {
  config: RunnerConfig
  status: RunnerStatus
  result?: RunnerResult
}

interface LiveListResponse {
  runners: StoredRunner[]
}

// ── Helpers ───────────────────────────────────────────────────────────

function fmt(iso?: string) {
  if (!iso) return '—'
  try { return new Date(iso).toLocaleString() } catch { return iso }
}

function fmtPct(v: number) {
  const color = v >= 0 ? 'var(--color-accent)' : 'var(--color-danger)'
  return <span style={{ color }}>{v >= 0 ? '+' : ''}{v.toFixed(2)}%</span>
}

// ── Create Runner Modal ───────────────────────────────────────────────

interface CreateModalProps {
  scripts: string[]
  onClose: () => void
  onCreated: () => void
}

function CreateModal({ scripts, onClose, onCreated }: CreateModalProps) {
  const [form, setForm] = useState({
    name: '',
    script: scripts[0] ?? '',
    market_type: 'crypto',
    symbol: 'BTCUSDT',
    interval: '5m',
    mode: 'paper',
    initial_balance: 1000,
    fee_pct: 0.1,
    warmup_days: 7,
  })
  const [error, setError] = useState('')

  const mutation = useMutation({
    mutationFn: () => apiPost('/api/live/strategies', form),
    onSuccess: () => { onCreated(); onClose() },
    onError: (e: Error) => setError(e.message),
  })

  function set<K extends keyof typeof form>(k: K, v: typeof form[K]) {
    setForm(f => ({ ...f, [k]: v }))
  }

  function onMarketTypeChange(mt: string) {
    setForm(f => ({
      ...f,
      market_type: mt,
      symbol: mt === 'crypto' ? 'BTCUSDT' : '',
      interval: mt === 'crypto' ? '5m' : '5m',
      fee_pct: mt === 'crypto' ? 0.1 : 1.5,
    }))
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div
        className="rounded-lg border w-full max-w-lg max-h-[90vh] overflow-y-auto"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center justify-between p-4 border-b" style={{ borderColor: 'var(--color-border)' }}>
          <h2 className="font-semibold flex items-center gap-2"><Bot size={16} /> New Live Strategy</h2>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}><X size={16} /></button>
        </div>

        <div className="p-4 space-y-3">
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Name</label>
            <input className="w-full rounded px-3 py-2 text-sm" value={form.name}
              onChange={e => set('name', e.target.value)} placeholder="My BTC Strategy" />
          </div>

          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Strategy Script</label>
            <select className="w-full rounded px-3 py-2 text-sm" value={form.script}
              onChange={e => set('script', e.target.value)}>
              {scripts.map(s => <option key={s} value={s}>{s}</option>)}
            </select>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Market Type</label>
              <select className="w-full rounded px-3 py-2 text-sm" value={form.market_type}
                onChange={e => onMarketTypeChange(e.target.value)}>
                <option value="crypto">Crypto</option>
                <option value="polymarket">Polymarket</option>
              </select>
            </div>
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Symbol / Market ID</label>
              <input className="w-full rounded px-3 py-2 text-sm font-mono" value={form.symbol}
                onChange={e => set('symbol', e.target.value)}
                placeholder={form.market_type === 'crypto' ? 'BTCUSDT' : 'market-slug'} />
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Interval</label>
              <select className="w-full rounded px-3 py-2 text-sm" value={form.interval}
                onChange={e => set('interval', e.target.value)}>
                {form.market_type === 'crypto'
                  ? ['1m','3m','5m','15m','30m','1h','4h'].map(i => <option key={i} value={i}>{i}</option>)
                  : ['5m','4m'].map(i => <option key={i} value={i}>{i}</option>)
                }
              </select>
            </div>
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Mode</label>
              <select className="w-full rounded px-3 py-2 text-sm" value={form.mode}
                onChange={e => set('mode', e.target.value)}>
                <option value="paper">Paper Trading</option>
                <option value="live">Live Trading</option>
              </select>
            </div>
          </div>

          <div className="grid grid-cols-3 gap-3">
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Initial Balance</label>
              <input type="number" className="w-full rounded px-3 py-2 text-sm" value={form.initial_balance}
                onChange={e => set('initial_balance', Number(e.target.value))} />
            </div>
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Fee %</label>
              <input type="number" step="0.01" className="w-full rounded px-3 py-2 text-sm" value={form.fee_pct}
                onChange={e => set('fee_pct', Number(e.target.value))} />
            </div>
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Warmup Days</label>
              <input type="number" className="w-full rounded px-3 py-2 text-sm" value={form.warmup_days}
                onChange={e => set('warmup_days', Number(e.target.value))} />
            </div>
          </div>

          {error && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{error}</p>}
        </div>

        <div className="p-4 border-t flex gap-2" style={{ borderColor: 'var(--color-border)' }}>
          <button
            onClick={() => mutation.mutate()}
            disabled={!form.script || !form.symbol || mutation.isPending}
            className="flex-1 py-2 rounded text-sm font-medium disabled:opacity-50"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            {mutation.isPending ? 'Starting...' : 'Start Strategy'}
          </button>
          <button onClick={onClose} className="px-4 py-2 rounded text-sm border hover:bg-white/5"
            style={{ borderColor: 'var(--color-border)' }}>Cancel</button>
        </div>
      </div>
    </div>
  )
}

// ── Runner Card ───────────────────────────────────────────────────────

function statusColor(s: string) {
  if (s === 'running') return 'var(--color-accent)'
  if (s === 'starting') return '#f59e0b'
  if (s === 'error') return 'var(--color-danger)'
  return 'var(--color-text-muted)'
}

function statusDot(s: string): 'online' | 'warning' | 'offline' {
  if (s === 'running') return 'online'
  if (s === 'starting') return 'warning'
  return 'offline'
}

interface RunnerCardProps {
  runner: StoredRunner
  onStop: () => void
  onRestart: () => void
  onDelete: () => void
}

function RunnerCard({ runner, onStop, onRestart, onDelete }: RunnerCardProps) {
  const [expanded, setExpanded] = useState(false)
  const { config, status, result } = runner
  const isRunning = status.status === 'running' || status.status === 'starting'

  return (
    <div
      className="rounded-lg border flex flex-col"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      {/* Header */}
      <div className="p-4 flex items-start gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-0.5">
            <span className={clsx('status-dot', statusDot(status.status))} />
            <h3 className="text-sm font-semibold truncate">{config.name || config.script}</h3>
            <span
              className="text-xs px-1.5 py-0.5 rounded flex-shrink-0"
              style={{ backgroundColor: 'var(--color-base)', color: statusColor(status.status) }}
            >
              {status.status}
            </span>
          </div>
          <p className="text-xs font-mono truncate" style={{ color: 'var(--color-text-muted)' }}>
            {config.script} · {config.symbol} · {config.interval} · {config.mode}
          </p>
        </div>
        <div className="flex items-center gap-1 flex-shrink-0">
          {isRunning ? (
            <button onClick={onStop} title="Stop"
              className="p-1.5 rounded hover:bg-white/5" style={{ color: 'var(--color-danger)' }}>
              <StopCircle size={14} />
            </button>
          ) : (
            <button onClick={onRestart} title="Restart"
              className="p-1.5 rounded hover:bg-white/5" style={{ color: 'var(--color-accent)' }}>
              <RotateCcw size={14} />
            </button>
          )}
          <button onClick={onDelete} title="Delete"
            className="p-1.5 rounded hover:bg-white/5" style={{ color: 'var(--color-text-muted)' }}>
            <Trash2 size={14} />
          </button>
          <button onClick={() => setExpanded(e => !e)}
            className="p-1.5 rounded hover:bg-white/5" style={{ color: 'var(--color-text-muted)' }}>
            {expanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
          </button>
        </div>
      </div>

      {/* P&L summary */}
      {result && (
        <div
          className="grid grid-cols-4 gap-2 px-4 pb-3 text-xs"
        >
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Return</div>
            <div className="font-semibold">{fmtPct(result.total_return_pct)}</div>
          </div>
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Win Rate</div>
            <div className="font-semibold">{result.win_rate_pct.toFixed(1)}%</div>
          </div>
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Trades</div>
            <div className="font-semibold">{result.total_trades}</div>
          </div>
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Signal</div>
            <div className="font-semibold truncate"
              style={{ color: result.last_signal === 'buy' ? 'var(--color-accent)' : result.last_signal === 'sell' ? 'var(--color-danger)' : 'var(--color-text-muted)' }}>
              {result.last_signal || '—'}
            </div>
          </div>
        </div>
      )}

      {/* Error */}
      {status.error && (
        <div className="mx-4 mb-3 px-3 py-2 rounded text-xs flex items-start gap-2"
          style={{ backgroundColor: 'rgba(239,68,68,0.1)', color: 'var(--color-danger)' }}>
          <AlertCircle size={12} className="mt-0.5 flex-shrink-0" />
          {status.error}
        </div>
      )}

      {/* Expanded details */}
      {expanded && (
        <div
          className="border-t px-4 py-3 space-y-3 text-xs"
          style={{ borderColor: 'var(--color-border)' }}
        >
          <div className="grid grid-cols-2 gap-x-4 gap-y-1.5">
            <div className="flex justify-between">
              <span style={{ color: 'var(--color-text-muted)' }}>Started</span>
              <span>{fmt(status.started_at)}</span>
            </div>
            <div className="flex justify-between">
              <span style={{ color: 'var(--color-text-muted)' }}>Last tick</span>
              <span>{fmt(status.last_tick_at)}</span>
            </div>
            <div className="flex justify-between">
              <span style={{ color: 'var(--color-text-muted)' }}>Next tick</span>
              <span>{fmt(status.next_tick_at)}</span>
            </div>
            <div className="flex justify-between">
              <span style={{ color: 'var(--color-text-muted)' }}>Balance</span>
              <span>${result?.balance.toFixed(2) ?? config.initial_balance}</span>
            </div>
            {result && (
              <>
                <div className="flex justify-between">
                  <span style={{ color: 'var(--color-text-muted)' }}>Sharpe</span>
                  <span>{result.sharpe_ratio.toFixed(2)}</span>
                </div>
                <div className="flex justify-between">
                  <span style={{ color: 'var(--color-text-muted)' }}>Max DD</span>
                  <span style={{ color: 'var(--color-danger)' }}>{result.max_drawdown_pct.toFixed(2)}%</span>
                </div>
              </>
            )}
          </div>
          {result?.analysis && (
            <p className="leading-relaxed" style={{ color: 'var(--color-text-muted)' }}>
              {result.analysis}
            </p>
          )}
        </div>
      )}
    </div>
  )
}

// ── Main Page ─────────────────────────────────────────────────────────

export default function LiveStrategies() {
  const [showCreate, setShowCreate] = useState(false)
  const qc = useQueryClient()

  const { data, isLoading, refetch } = useQuery<LiveListResponse>({
    queryKey: ['live-strategies'],
    queryFn: () => apiFetch<LiveListResponse>('/api/live/strategies').catch(() => ({ runners: [] })),
    refetchInterval: 5_000,
  })

  const { data: scriptsData } = useQuery<{ scripts: Array<{ name: string } | string> }>({
    queryKey: ['backtest-scripts'],
    queryFn: () => apiFetch<{ scripts: Array<{ name: string } | string> }>('/api/backtest/scripts').catch(() => ({ scripts: [] })),
  })

  const stopMutation = useMutation({
    mutationFn: (id: string) => apiPost(`/api/live/strategies/${id}/stop`, {}),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['live-strategies'] }),
  })

  const restartMutation = useMutation({
    mutationFn: (id: string) => apiPost(`/api/live/strategies/${id}/restart`, {}),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['live-strategies'] }),
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => apiDelete(`/api/live/strategies/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['live-strategies'] }),
  })

  const runners = data?.runners ?? []
  const scripts = (scriptsData?.scripts ?? []).map(s => typeof s === 'string' ? s : s.name)
  const running = runners.filter(r => r.status.status === 'running').length
  const totalReturn = runners.reduce((s, r) => s + (r.result?.total_return_pct ?? 0), 0)

  return (
    <div className="p-6 max-w-5xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-2">
          <Bot size={18} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-lg font-bold">Live Strategies</h1>
          {running > 0 && (
            <span className="text-xs px-2 py-0.5 rounded animate-pulse"
              style={{ backgroundColor: 'rgba(74,222,128,0.15)', color: 'var(--color-accent)' }}>
              {running} running
            </span>
          )}
          {runners.length > 0 && (
            <span className="text-xs px-2 py-0.5 rounded"
              style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)' }}>
              Total P&L: {totalReturn >= 0 ? '+' : ''}{totalReturn.toFixed(2)}%
            </span>
          )}
        </div>
        <div className="flex gap-2">
          <button onClick={() => refetch()}
            className="p-2 rounded border hover:bg-white/5"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
            <RefreshCw size={13} className={isLoading ? 'animate-spin' : ''} />
          </button>
          <button onClick={() => setShowCreate(true)}
            className="flex items-center gap-2 px-3 py-1.5 rounded text-sm font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
            <Plus size={14} />
            New Strategy
          </button>
        </div>
      </div>

      {/* Stats row */}
      {runners.length > 0 && (
        <div className="grid grid-cols-4 gap-3 mb-6">
          {[
            { label: 'Runners', value: runners.length, icon: <Activity size={14} /> },
            { label: 'Running', value: running, icon: <Bot size={14} /> },
            { label: 'Total Trades', value: runners.reduce((s, r) => s + (r.result?.total_trades ?? 0), 0), icon: <TrendingUp size={14} /> },
            { label: 'Avg Win Rate', value: runners.length ? `${(runners.reduce((s, r) => s + (r.result?.win_rate_pct ?? 0), 0) / runners.length).toFixed(1)}%` : '—', icon: <TrendingDown size={14} /> },
          ].map(stat => (
            <div key={stat.label} className="rounded-lg border p-3"
              style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
              <div className="flex items-center gap-1.5 mb-1" style={{ color: 'var(--color-text-muted)' }}>
                {stat.icon}
                <span className="text-xs">{stat.label}</span>
              </div>
              <div className="text-lg font-bold">{stat.value}</div>
            </div>
          ))}
        </div>
      )}

      {/* Runners grid */}
      {isLoading ? (
        <div className="text-sm text-center py-12" style={{ color: 'var(--color-text-muted)' }}>Loading...</div>
      ) : runners.length === 0 ? (
        <div className="text-center py-20">
          <Bot size={48} className="mx-auto mb-4 opacity-20" />
          <p className="text-sm mb-1" style={{ color: 'var(--color-text-muted)' }}>No live strategies running</p>
          <p className="text-xs mb-4" style={{ color: 'var(--color-text-muted)' }}>
            Start a strategy to run it on live or paper market data
          </p>
          <button onClick={() => setShowCreate(true)}
            className="px-4 py-2 rounded text-sm font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
            <Plus size={13} className="inline mr-1" />
            New Strategy
          </button>
        </div>
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
          {runners.map(runner => (
            <RunnerCard
              key={runner.config.id}
              runner={runner}
              onStop={() => stopMutation.mutate(runner.config.id)}
              onRestart={() => restartMutation.mutate(runner.config.id)}
              onDelete={() => {
                if (confirm(`Delete "${runner.config.name}"?`)) deleteMutation.mutate(runner.config.id)
              }}
            />
          ))}
        </div>
      )}

      {showCreate && (
        <CreateModal
          scripts={scripts}
          onClose={() => setShowCreate(false)}
          onCreated={() => qc.invalidateQueries({ queryKey: ['live-strategies'] })}
        />
      )}
    </div>
  )
}
