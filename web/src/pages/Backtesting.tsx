import { useState, useEffect } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import {
  FlaskConical, Play, FileCode2, BarChart2, TrendingDown,
  AlertCircle, ChevronDown, ChevronRight, RefreshCw, Trash2,
  Pencil, Save, X, FolderOpen, Activity, Check, Bitcoin, Vote,
} from 'lucide-react'
import clsx from 'clsx'

// ── Types ─────────────────────────────────────────────────────────

interface BacktestScript {
  name: string
  path: string
  description?: string
  last_modified?: string
  last_run_stats?: {
    total_return_pct: number
    sharpe_ratio: number | null
    win_rate_pct: number
    total_trades: number
    run_date: string
  }
}

type MarketType = 'crypto' | 'polymarket'

interface BacktestConfig {
  script: string
  market_type: MarketType
  symbol: string
  interval: string
  from_date: string
  to_date: string
  initial_balance: number
  fee_pct: number
}

const CRYPTO_INTERVALS = [
  { value: '1m', label: '1m' },
  { value: '3m', label: '3m' },
  { value: '5m', label: '5m' },
  { value: '15m', label: '15m' },
  { value: '30m', label: '30m' },
  { value: '1h', label: '1h' },
  { value: '2h', label: '2h' },
  { value: '4h', label: '4h' },
  { value: '6h', label: '6h' },
  { value: '12h', label: '12h' },
  { value: '1d', label: '1d' },
  { value: '1w', label: '1w' },
]

const POLYMARKET_INTERVALS = [
  { value: '1m', label: '1m' },
  { value: '5m', label: '5m' },
  { value: '15m', label: '15m' },
  { value: '1h', label: '1h' },
  { value: '4h', label: '4h' },
  { value: '1d', label: '1d' },
]

interface TradeLog {
  timestamp: string
  side: string
  price: number
  size: number
  pnl: number
}

interface BacktestResult {
  script: string
  symbol: string
  total_return_pct: number
  sharpe_ratio: number | null
  max_drawdown_pct: number
  win_rate_pct: number
  total_trades: number
  worst_trades: TradeLog[]
  analysis?: string
}

// ── Helpers ───────────────────────────────────────────────────────

function fmt(n: number, dec = 2): string {
  return n.toFixed(dec)
}

function colorFor(n: number): string {
  return n >= 0 ? 'var(--color-accent)' : 'var(--color-danger)'
}

function log(msg: string, data?: unknown) {
  const ts = new Date().toISOString().slice(11, 23)
  if (data !== undefined) {
    console.log(`[Backtest ${ts}] ${msg}`, data)
  } else {
    console.log(`[Backtest ${ts}] ${msg}`)
  }
}

// ── Sub-components ────────────────────────────────────────────────

function MetricCard({
  label,
  value,
  sub,
  color,
}: {
  label: string
  value: string
  sub?: string
  color?: string
}) {
  return (
    <div
      className="rounded-lg border p-4"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      <p className="text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>
        {label}
      </p>
      <p className="text-xl font-bold font-mono" style={{ color: color ?? 'var(--color-text)' }}>
        {value}
      </p>
      {sub && <p className="text-xs mt-0.5" style={{ color: 'var(--color-text-muted)' }}>{sub}</p>}
    </div>
  )
}

function ResultPanel({ result }: { result: BacktestResult }) {
  const [showTrades, setShowTrades] = useState(false)

  return (
    <div className="space-y-4">
      {/* Metrics grid */}
      <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-3">
        <MetricCard
          label="Total Return"
          value={`${result.total_return_pct >= 0 ? '+' : ''}${fmt(result.total_return_pct)}%`}
          color={colorFor(result.total_return_pct)}
        />
        <MetricCard
          label="Sharpe Ratio"
          value={result.sharpe_ratio != null ? fmt(result.sharpe_ratio) : '—'}
          color={
            result.sharpe_ratio == null
              ? 'var(--color-text-muted)'
              : result.sharpe_ratio >= 1
              ? 'var(--color-accent)'
              : 'var(--color-warning)'
          }
        />
        <MetricCard
          label="Max Drawdown"
          value={`-${fmt(result.max_drawdown_pct)}%`}
          color="var(--color-danger)"
        />
        <MetricCard
          label="Win Rate"
          value={`${fmt(result.win_rate_pct)}%`}
          color={result.win_rate_pct >= 50 ? 'var(--color-accent)' : 'var(--color-warning)'}
        />
        <MetricCard
          label="Total Trades"
          value={String(result.total_trades)}
        />
      </div>

      {/* AI Analysis */}
      {result.analysis && (
        <div
          className="rounded-lg border p-4 text-sm"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <p className="text-xs font-semibold uppercase tracking-widest mb-2" style={{ color: 'var(--color-text-muted)' }}>
            Analysis
          </p>
          <p className="leading-relaxed whitespace-pre-wrap" style={{ color: 'var(--color-text)' }}>
            {result.analysis}
          </p>
        </div>
      )}

      {/* Worst trades */}
      {result.worst_trades && result.worst_trades.length > 0 && (
        <div
          className="rounded-lg border overflow-hidden"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <button
            className="w-full flex items-center justify-between px-4 py-3 text-xs font-semibold uppercase tracking-widest"
            style={{ color: 'var(--color-text-muted)' }}
            onClick={() => setShowTrades((v) => !v)}
          >
            <span className="flex items-center gap-1.5">
              <TrendingDown size={12} style={{ color: 'var(--color-danger)' }} />
              5 Worst Trades
            </span>
            {showTrades ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          </button>
          {showTrades && (
            <div className="border-t" style={{ borderColor: 'var(--color-border)' }}>
              <div
                className="grid text-xs px-4 py-2 border-b font-semibold uppercase tracking-widest"
                style={{
                  gridTemplateColumns: '1fr 60px 80px 70px 70px',
                  borderColor: 'var(--color-border)',
                  color: 'var(--color-text-muted)',
                  backgroundColor: 'var(--color-surface-2)',
                }}
              >
                <span>Time</span>
                <span>Side</span>
                <span className="text-right">Price</span>
                <span className="text-right">Size</span>
                <span className="text-right">PnL</span>
              </div>
              {result.worst_trades.map((t, i) => (
                <div
                  key={i}
                  className="grid text-xs px-4 py-2 border-b font-mono"
                  style={{
                    gridTemplateColumns: '1fr 60px 80px 70px 70px',
                    borderColor: 'var(--color-border)',
                    color: 'var(--color-text)',
                  }}
                >
                  <span style={{ color: 'var(--color-text-muted)' }}>
                    {new Date(t.timestamp).toLocaleDateString()}
                  </span>
                  <span style={{ color: t.side === 'buy' ? 'var(--color-accent)' : 'var(--color-danger)' }}>
                    {t.side.toUpperCase()}
                  </span>
                  <span className="text-right">${t.price.toLocaleString()}</span>
                  <span className="text-right">{fmt(t.size, 4)}</span>
                  <span className="text-right" style={{ color: colorFor(t.pnl) }}>
                    {t.pnl >= 0 ? '+' : ''}{fmt(t.pnl)}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

// ── Progress Panel ────────────────────────────────────────────────

interface ProgressState {
  step: 'idle' | 'preparing' | 'fetching' | 'running' | 'analyzing' | 'done' | 'error'
  message: string
  progress?: number
  startTime?: number
}

function ProgressPanel({ state }: { state: ProgressState }) {
  const [elapsed, setElapsed] = useState(0)

  useEffect(() => {
    if (state.startTime && state.step !== 'done' && state.step !== 'error') {
      const interval = setInterval(() => {
        setElapsed(Math.floor((Date.now() - state.startTime!) / 1000))
      }, 100)
      return () => clearInterval(interval)
    }
  }, [state.startTime, state.step])

  const steps = [
    { key: 'preparing', label: 'Preparing' },
    { key: 'fetching', label: 'Fetching Data' },
    { key: 'running', label: 'Running Engine' },
    { key: 'analyzing', label: 'Analyzing' },
  ]

  const currentIdx = steps.findIndex(s => s.key === state.step)

  return (
    <div
      className="rounded-lg border p-6"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-2">
          <Activity size={16} className="animate-pulse" style={{ color: 'var(--color-accent)' }} />
          <span className="text-sm font-semibold" style={{ color: 'var(--color-text)' }}>
            Running Backtest
          </span>
        </div>
        <span className="text-xs font-mono" style={{ color: 'var(--color-text-muted)' }}>
          {elapsed}s elapsed
        </span>
      </div>

      {/* Steps progress */}
      <div className="flex items-center gap-2 mb-4">
        {steps.map((step, idx) => {
          const isActive = step.key === state.step
          const isDone = currentIdx > idx || state.step === 'done'
          return (
            <div key={step.key} className="flex items-center gap-2 flex-1">
              <div
                className={clsx(
                  'w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold transition-colors',
                  isDone && 'bg-[var(--color-accent)] text-black',
                  isActive && 'bg-[var(--color-accent)] text-black animate-pulse',
                  !isDone && !isActive && 'bg-[var(--color-surface-2)] text-[var(--color-text-muted)]'
                )}
              >
                {isDone ? <Check size={12} /> : idx + 1}
              </div>
              <span
                className={clsx(
                  'text-xs hidden sm:block',
                  isActive && 'text-[var(--color-accent)] font-semibold',
                  !isActive && 'text-[var(--color-text-muted)]'
                )}
              >
                {step.label}
              </span>
              {idx < steps.length - 1 && (
                <div
                  className="flex-1 h-0.5 mx-2"
                  style={{
                    backgroundColor: isDone ? 'var(--color-accent)' : 'var(--color-border)'
                  }}
                />
              )}
            </div>
          )
        })}
      </div>

      {/* Current message */}
      <div
        className="text-sm py-2 px-3 rounded font-mono"
        style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}
      >
        {state.message}
      </div>
    </div>
  )
}

// ── Script Item ───────────────────────────────────────────────────

interface ScriptItemProps {
  script: BacktestScript
  isSelected: boolean
  onSelect: () => void
  onDelete: () => void
  onRename: (newName: string) => void
  onUpdateDescription: (desc: string) => void
}

function ScriptItem({ script, isSelected, onSelect, onDelete, onRename, onUpdateDescription }: ScriptItemProps) {
  const [isEditing, setIsEditing] = useState(false)
  const [editName, setEditName] = useState(script.name)
  const [editDesc, setEditDesc] = useState(script.description || '')
  const [confirmDelete, setConfirmDelete] = useState(false)

  const handleSave = () => {
    if (editName !== script.name) {
      onRename(editName)
    }
    if (editDesc !== (script.description || '')) {
      onUpdateDescription(editDesc)
    }
    setIsEditing(false)
  }

  const handleDelete = () => {
    if (confirmDelete) {
      onDelete()
      setConfirmDelete(false)
    } else {
      setConfirmDelete(true)
      setTimeout(() => setConfirmDelete(false), 3000)
    }
  }

  if (isEditing) {
    return (
      <div
        className="rounded-lg border p-3"
        style={{ backgroundColor: 'var(--color-surface-2)', borderColor: 'var(--color-accent)' }}
      >
        <div className="space-y-2">
          <input
            value={editName}
            onChange={(e) => setEditName(e.target.value)}
            className="w-full rounded px-2 py-1 text-sm font-mono"
            style={{
              backgroundColor: 'var(--color-surface)',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text)',
            }}
            placeholder="Script name"
          />
          <textarea
            value={editDesc}
            onChange={(e) => setEditDesc(e.target.value)}
            rows={2}
            className="w-full rounded px-2 py-1 text-xs resize-none"
            style={{
              backgroundColor: 'var(--color-surface)',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text)',
            }}
            placeholder="Description (what this strategy does)"
          />
          <div className="flex justify-end gap-2">
            <button
              onClick={() => setIsEditing(false)}
              className="px-2 py-1 rounded text-xs"
              style={{ color: 'var(--color-text-muted)' }}
            >
              <X size={12} />
            </button>
            <button
              onClick={handleSave}
              className="px-2 py-1 rounded text-xs flex items-center gap-1"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              <Save size={12} /> Save
            </button>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div
      onClick={onSelect}
      className={clsx(
        'flex items-start gap-3 rounded-lg border p-3 cursor-pointer transition-colors group',
        isSelected
          ? 'border-[var(--color-accent)]'
          : 'border-[var(--color-border)] hover:border-[rgba(0,255,136,0.3)]',
      )}
      style={{ backgroundColor: 'var(--color-surface-2)' }}
    >
      <FileCode2
        size={16}
        className="mt-0.5 flex-shrink-0"
        style={{
          color: isSelected ? 'var(--color-accent)' : 'var(--color-text-muted)',
        }}
      />
      <div className="min-w-0 flex-1">
        <p className="text-sm font-mono font-semibold truncate" style={{ color: 'var(--color-text)' }}>
          {script.name}
        </p>
        {script.description && (
          <p className="text-xs mt-0.5 line-clamp-2" style={{ color: 'var(--color-text-muted)' }}>
            {script.description}
          </p>
        )}
        {script.last_run_stats && (
          <div className="flex gap-3 mt-1.5 text-xs font-mono" style={{ color: 'var(--color-text-muted)' }}>
            <span style={{ color: colorFor(script.last_run_stats.total_return_pct) }}>
              {script.last_run_stats.total_return_pct >= 0 ? '+' : ''}{fmt(script.last_run_stats.total_return_pct)}%
            </span>
            <span>SR: {script.last_run_stats.sharpe_ratio != null ? fmt(script.last_run_stats.sharpe_ratio) : '—'}</span>
            <span>{script.last_run_stats.total_trades} trades</span>
          </div>
        )}
      </div>
      <div className="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity" onClick={(e) => e.stopPropagation()}>
        <button
          onClick={() => setIsEditing(true)}
          className="p-1.5 rounded hover:bg-[var(--color-surface)]"
          title="Edit"
        >
          <Pencil size={12} style={{ color: 'var(--color-text-muted)' }} />
        </button>
        <button
          onClick={handleDelete}
          className="p-1.5 rounded hover:bg-[var(--color-surface)]"
          title={confirmDelete ? 'Click again to confirm' : 'Delete'}
        >
          <Trash2 size={12} style={{ color: confirmDelete ? 'var(--color-danger)' : 'var(--color-text-muted)' }} />
        </button>
      </div>
    </div>
  )
}

// ── Main page ─────────────────────────────────────────────────────

const TODAY = new Date().toISOString().slice(0, 10)
const THREE_MONTHS_AGO = new Date(Date.now() - 90 * 86400 * 1000).toISOString().slice(0, 10)

export default function Backtesting() {
  const queryClient = useQueryClient()
  const [config, setConfig] = useState<BacktestConfig>({
    script: '',
    market_type: 'crypto',
    symbol: 'BTCUSDT',
    interval: '1m',
    from_date: THREE_MONTHS_AGO,
    to_date: TODAY,
    initial_balance: 10000,
    fee_pct: 0.1,
  })
  const [result, setResult] = useState<BacktestResult | null>(null)
  const [scriptsExpanded, setScriptsExpanded] = useState(true)
  const [progress, setProgress] = useState<ProgressState>({ step: 'idle', message: '' })

  // Load available scripts
  const { data: scriptsData, isLoading: scriptsLoading, refetch: refetchScripts } = useQuery<{ scripts: BacktestScript[] }>({
    queryKey: ['backtest-scripts'],
    queryFn: () => {
      log('Fetching scripts list')
      return apiFetch('/api/backtest/scripts')
    },
  })

  const scripts = scriptsData?.scripts ?? []

  // Delete script mutation
  const deleteMutation = useMutation({
    mutationFn: (path: string) => {
      log('Deleting script:', path)
      return apiDelete(`/api/backtest/scripts?path=${encodeURIComponent(path)}`)
    },
    onSuccess: () => {
      log('Script deleted successfully')
      refetchScripts()
      if (config.script && !scripts.find(s => s.path !== config.script)) {
        setConfig(c => ({ ...c, script: '' }))
      }
    },
    onError: (err) => {
      log('Delete error:', err)
    }
  })

  // Rename script mutation
  const renameMutation = useMutation({
    mutationFn: ({ oldPath, newName }: { oldPath: string; newName: string }) => {
      log('Renaming script:', { oldPath, newName })
      return apiPost('/api/backtest/scripts/rename', { old_path: oldPath, new_name: newName })
    },
    onSuccess: () => {
      log('Script renamed successfully')
      refetchScripts()
    },
    onError: (err) => {
      log('Rename error:', err)
    }
  })

  // Update description mutation
  const updateDescMutation = useMutation({
    mutationFn: ({ path, description }: { path: string; description: string }) => {
      log('Updating description:', { path, description })
      return apiPost('/api/backtest/scripts/description', { path, description })
    },
    onSuccess: () => {
      log('Description updated successfully')
      refetchScripts()
    },
    onError: (err) => {
      log('Description update error:', err)
    }
  })

  // Run backtest
  const mutation = useMutation({
    mutationFn: async (cfg: BacktestConfig) => {
      log('Starting backtest with config:', cfg)

      setProgress({ step: 'preparing', message: 'Validating configuration...', startTime: Date.now() })
      await new Promise(r => setTimeout(r, 300))

      setProgress({ step: 'fetching', message: `Fetching ${cfg.symbol} ${cfg.interval} candles from Binance (${cfg.from_date} to ${cfg.to_date})...`, startTime: Date.now() })

      const response = await apiPost<BacktestResult>('/api/backtest/run', cfg)

      setProgress({ step: 'running', message: 'Executing Rhai strategy engine...', startTime: Date.now() })
      await new Promise(r => setTimeout(r, 200))

      setProgress({ step: 'analyzing', message: 'Computing metrics and analysis...', startTime: Date.now() })
      await new Promise(r => setTimeout(r, 200))

      log('Backtest complete:', response)
      return response
    },
    onSuccess: (data) => {
      setProgress({ step: 'done', message: 'Backtest complete!' })
      setResult(data)

      // Save stats to the script
      const selectedScript = scripts.find(s => s.path === config.script)
      if (selectedScript) {
        apiPost('/api/backtest/scripts/stats', {
          path: config.script,
          stats: {
            total_return_pct: data.total_return_pct,
            sharpe_ratio: data.sharpe_ratio,
            win_rate_pct: data.win_rate_pct,
            total_trades: data.total_trades,
            run_date: new Date().toISOString(),
          }
        }).then(() => {
          log('Stats saved to script')
          refetchScripts()
        }).catch(err => {
          log('Failed to save stats:', err)
        })
      }
    },
    onError: (err) => {
      log('Backtest error:', err)
      setProgress({ step: 'error', message: `Error: ${(err as Error)?.message ?? String(err)}` })
    },
  })

  function set<K extends keyof BacktestConfig>(k: K, v: BacktestConfig[K]) {
    setConfig((c) => ({ ...c, [k]: v }))
  }

  const canRun = config.script && !mutation.isPending

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex items-center gap-3 mb-6">
        <FlaskConical size={20} style={{ color: 'var(--color-accent)' }} />
        <div>
          <h1 className="text-xl font-bold" style={{ color: 'var(--color-accent)' }}>
            Strategy Backtesting
          </h1>
          <p className="text-xs mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
            Run .rhai strategies against historical data · powered by Rhai + Rayon
          </p>
        </div>
      </div>

      {/* Configuration - Horizontal layout */}
      <div
        className="rounded-lg border p-4 mb-4"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <h2 className="text-xs font-semibold uppercase tracking-widest mb-3" style={{ color: 'var(--color-text-muted)' }}>
          Configuration
        </h2>

        <div className="grid grid-cols-2 sm:grid-cols-4 lg:grid-cols-9 gap-3 items-end">
          {/* Market Type Toggle */}
          <div>
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Market</label>
            <div className="flex rounded overflow-hidden" style={{ border: '1px solid var(--color-border)' }}>
              <button
                onClick={() => {
                  set('market_type', 'crypto')
                  set('symbol', 'BTCUSDT')
                  set('interval', '1m')
                }}
                className={clsx(
                  'flex-1 flex items-center justify-center gap-1.5 py-2 px-2 text-xs font-semibold transition-colors',
                  config.market_type === 'crypto'
                    ? 'bg-[var(--color-accent)] text-black'
                    : 'bg-[var(--color-surface-2)] text-[var(--color-text-muted)] hover:text-[var(--color-text)]'
                )}
              >
                <Bitcoin size={12} />
                Crypto
              </button>
              <button
                onClick={() => {
                  set('market_type', 'polymarket')
                  set('symbol', '')
                  set('interval', '1h')
                }}
                className={clsx(
                  'flex-1 flex items-center justify-center gap-1.5 py-2 px-2 text-xs font-semibold transition-colors',
                  config.market_type === 'polymarket'
                    ? 'bg-[var(--color-accent)] text-black'
                    : 'bg-[var(--color-surface-2)] text-[var(--color-text-muted)] hover:text-[var(--color-text)]'
                )}
              >
                <Vote size={12} />
                Polymarket
              </button>
            </div>
          </div>

          {/* Script select */}
          <div className="col-span-2">
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
              Strategy Script
            </label>
            {scriptsLoading ? (
              <div className="text-xs py-2" style={{ color: 'var(--color-text-muted)' }}>Loading...</div>
            ) : scripts.length === 0 ? (
              <div
                className="rounded px-3 py-2 text-xs"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text-muted)',
                }}
              >
                No scripts found
              </div>
            ) : (
              <select
                value={config.script}
                onChange={(e) => set('script', e.target.value)}
                className="w-full rounded px-3 py-2 text-sm font-mono"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                }}
              >
                <option value="">Select a script...</option>
                {scripts.map((s) => (
                  <option key={s.path} value={s.path}>
                    {s.name}
                  </option>
                ))}
              </select>
            )}
          </div>

          {/* Symbol / Market ID */}
          <div>
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
              {config.market_type === 'crypto' ? 'Symbol' : 'Condition ID'}
            </label>
            <input
              value={config.symbol}
              onChange={(e) => set('symbol', config.market_type === 'crypto' ? e.target.value.toUpperCase() : e.target.value)}
              placeholder={config.market_type === 'crypto' ? 'BTCUSDT' : '0x1234...'}
              className="w-full rounded px-3 py-2 text-sm font-mono"
              style={{
                backgroundColor: 'var(--color-surface-2)',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text)',
              }}
            />
          </div>

          {/* Interval */}
          <div>
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Interval</label>
            <select
              value={config.interval}
              onChange={(e) => set('interval', e.target.value)}
              className="w-full rounded px-2 py-2 text-sm font-mono"
              style={{
                backgroundColor: 'var(--color-surface-2)',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text)',
              }}
            >
              {(config.market_type === 'crypto' ? CRYPTO_INTERVALS : POLYMARKET_INTERVALS).map((i) => (
                <option key={i.value} value={i.value}>
                  {i.label}
                </option>
              ))}
            </select>
          </div>

          {/* From date */}
          <div>
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>From</label>
            <input
              type="date"
              value={config.from_date}
              onChange={(e) => set('from_date', e.target.value)}
              className="w-full rounded px-2 py-2 text-xs font-mono"
              style={{
                backgroundColor: 'var(--color-surface-2)',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text)',
              }}
            />
          </div>

          {/* To date */}
          <div>
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>To</label>
            <input
              type="date"
              value={config.to_date}
              onChange={(e) => set('to_date', e.target.value)}
              className="w-full rounded px-2 py-2 text-xs font-mono"
              style={{
                backgroundColor: 'var(--color-surface-2)',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text)',
              }}
            />
          </div>

          {/* Balance */}
          <div>
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Balance ($)</label>
            <input
              type="number"
              min={100}
              value={config.initial_balance}
              onChange={(e) => set('initial_balance', Number(e.target.value))}
              className="w-full rounded px-2 py-2 text-sm font-mono"
              style={{
                backgroundColor: 'var(--color-surface-2)',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text)',
              }}
            />
          </div>

          {/* Run button */}
          <div>
            <button
              onClick={() => mutation.mutate(config)}
              disabled={!canRun}
              className="w-full flex items-center justify-center gap-2 py-2.5 rounded font-semibold text-sm transition-opacity disabled:opacity-40"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              {mutation.isPending ? (
                <>
                  <RefreshCw size={14} className="animate-spin" />
                  Running
                </>
              ) : (
                <>
                  <Play size={14} />
                  Run
                </>
              )}
            </button>
          </div>
        </div>
      </div>

      {/* Main content - Scripts (left sidebar) + Results (right primary) */}
      <div className="flex gap-4">
        {/* Scripts panel - Collapsible */}
        <div
          className={clsx(
            'rounded-lg border transition-all overflow-hidden flex-shrink-0',
            scriptsExpanded ? 'w-80' : 'w-10'
          )}
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <button
            onClick={() => setScriptsExpanded(!scriptsExpanded)}
            className="w-full flex items-center gap-2 p-3 text-xs font-semibold uppercase tracking-widest hover:bg-[var(--color-surface-2)]"
            style={{ color: 'var(--color-text-muted)' }}
          >
            {scriptsExpanded ? (
              <>
                <ChevronDown size={14} />
                <FolderOpen size={14} />
                <span>Scripts</span>
                <span className="ml-auto text-[10px] font-mono bg-[var(--color-surface-2)] px-1.5 py-0.5 rounded">
                  {scripts.length}
                </span>
              </>
            ) : (
              <FolderOpen size={14} className="mx-auto" />
            )}
          </button>

          {scriptsExpanded && (
            <div className="p-3 pt-0 max-h-[500px] overflow-y-auto">
              {scripts.length === 0 ? (
                <div className="flex flex-col items-center justify-center py-8 text-center gap-2">
                  <FileCode2 size={24} style={{ color: 'var(--color-border)' }} />
                  <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                    No .rhai scripts found
                  </p>
                  <p className="text-[10px] px-2" style={{ color: 'var(--color-text-muted)' }}>
                    Ask the agent to generate a strategy
                  </p>
                </div>
              ) : (
                <div className="space-y-2">
                  {scripts.map((s) => (
                    <ScriptItem
                      key={s.path}
                      script={s}
                      isSelected={config.script === s.path}
                      onSelect={() => set('script', s.path)}
                      onDelete={() => deleteMutation.mutate(s.path)}
                      onRename={(newName) => renameMutation.mutate({ oldPath: s.path, newName })}
                      onUpdateDescription={(desc) => updateDescMutation.mutate({ path: s.path, description: desc })}
                    />
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        {/* Results - Primary panel */}
        <div className="flex-1 min-w-0">
          <div
            className="rounded-lg border p-4"
            style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
          >
            <div className="flex items-center gap-2 mb-4">
              <BarChart2 size={14} style={{ color: 'var(--color-accent)' }} />
              <h2 className="text-sm font-semibold">Results</h2>
            </div>

            {/* Show progress when running */}
            {mutation.isPending && <ProgressPanel state={progress} />}

            {/* Show error */}
            {mutation.isError && progress.step === 'error' && (
              <div
                className="flex flex-col gap-2 text-sm px-4 py-3 rounded"
                style={{ backgroundColor: 'rgba(255,68,68,0.1)', color: 'var(--color-danger)', border: '1px solid rgba(255,68,68,0.2)' }}
              >
                <div className="flex items-center gap-2 font-semibold">
                  <AlertCircle size={14} />
                  Backtest failed
                </div>
                <p className="font-mono text-xs break-all opacity-80">
                  {(mutation.error as Error)?.message ?? String(mutation.error)}
                </p>
                <p className="text-xs mt-1" style={{ color: 'var(--color-text-muted)' }}>
                  Check the browser console (F12) for detailed debug logs.
                </p>
              </div>
            )}

            {/* Show results */}
            {!mutation.isPending && result && (
              <>
                <div className="mb-3 text-xs" style={{ color: 'var(--color-text-muted)' }}>
                  <span className="font-mono">{result.script.split('/').pop()}</span> / {result.symbol}
                </div>
                <ResultPanel result={result} />
              </>
            )}

            {/* Empty state */}
            {!mutation.isPending && !result && !mutation.isError && (
              <div className="flex flex-col items-center justify-center py-16 text-center gap-3">
                <BarChart2 size={48} style={{ color: 'var(--color-border)' }} />
                <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
                  Select a script and run a backtest to see results
                </p>
                <p className="text-xs max-w-md" style={{ color: 'var(--color-text-muted)' }}>
                  The engine will fetch historical data from Binance, execute your Rhai strategy,
                  and compute performance metrics including Sharpe ratio, max drawdown, and win rate.
                </p>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
