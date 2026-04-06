import { useState } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import {
  FlaskConical, Play, FileCode2, BarChart2, TrendingDown,
  AlertCircle, ChevronDown, ChevronRight, RefreshCw,
} from 'lucide-react'
import clsx from 'clsx'

// ── Types ─────────────────────────────────────────────────────────

interface BacktestScript {
  name: string
  path: string
  description?: string
  last_modified?: string
}

interface BacktestConfig {
  script: string
  symbol: string
  from_date: string
  to_date: string
  initial_balance: number
  fee_pct: number
}

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

// ── Main page ─────────────────────────────────────────────────────

const TODAY = new Date().toISOString().slice(0, 10)
const THREE_MONTHS_AGO = new Date(Date.now() - 90 * 86400 * 1000).toISOString().slice(0, 10)

export default function Backtesting() {
  const [config, setConfig] = useState<BacktestConfig>({
    script: '',
    symbol: 'BTCUSDT',
    from_date: THREE_MONTHS_AGO,
    to_date: TODAY,
    initial_balance: 10000,
    fee_pct: 0.1,
  })
  const [result, setResult] = useState<BacktestResult | null>(null)

  // Load available scripts
  const { data: scriptsData, isLoading: scriptsLoading } = useQuery<{ scripts: BacktestScript[] }>({
    queryKey: ['backtest-scripts'],
    queryFn: () => apiFetch('/api/backtest/scripts'),
  })

  const scripts = scriptsData?.scripts ?? []

  // Run backtest
  const mutation = useMutation({
    mutationFn: (cfg: BacktestConfig) =>
      apiPost<BacktestResult>('/api/backtest/run', cfg),
    onSuccess: (data) => setResult(data),
  })

  function set<K extends keyof BacktestConfig>(k: K, v: BacktestConfig[K]) {
    setConfig((c) => ({ ...c, [k]: v }))
  }

  const canRun = config.script && !mutation.isPending

  return (
    <div className="p-6 max-w-5xl mx-auto">
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

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4 mb-6">
        {/* Config panel */}
        <div
          className="lg:col-span-1 rounded-lg border p-4 space-y-4"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <h2 className="text-xs font-semibold uppercase tracking-widest" style={{ color: 'var(--color-text-muted)' }}>
            Configuration
          </h2>

          {/* Script select */}
          <div>
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
              Strategy Script (.rhai)
            </label>
            {scriptsLoading ? (
              <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>Loading…</p>
            ) : scripts.length === 0 ? (
              <div
                className="rounded px-3 py-2 text-xs"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text-muted)',
                }}
              >
                No scripts in /scripts/. Ask the agent to generate one.
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
                <option value="">Select a script…</option>
                {scripts.map((s) => (
                  <option key={s.path} value={s.path}>
                    {s.name}
                  </option>
                ))}
              </select>
            )}
            {config.script && (
              <p className="text-xs mt-1 font-mono" style={{ color: 'var(--color-text-muted)' }}>
                {config.script}
              </p>
            )}
          </div>

          {/* Symbol */}
          <div>
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Symbol</label>
            <input
              value={config.symbol}
              onChange={(e) => set('symbol', e.target.value.toUpperCase())}
              placeholder="BTCUSDT"
              className="w-full rounded px-3 py-2 text-sm font-mono"
              style={{
                backgroundColor: 'var(--color-surface-2)',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text)',
              }}
            />
          </div>

          {/* Date range */}
          <div className="grid grid-cols-2 gap-2">
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
          </div>

          {/* Balance + fee */}
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
                Initial Balance ($)
              </label>
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
            <div>
              <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
                Fee (%)
              </label>
              <input
                type="number"
                min={0}
                max={5}
                step={0.01}
                value={config.fee_pct}
                onChange={(e) => set('fee_pct', Number(e.target.value))}
                className="w-full rounded px-2 py-2 text-sm font-mono"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                }}
              />
            </div>
          </div>

          {/* Run button */}
          <button
            onClick={() => mutation.mutate(config)}
            disabled={!canRun}
            className="w-full flex items-center justify-center gap-2 py-2.5 rounded font-semibold text-sm transition-opacity disabled:opacity-40"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            {mutation.isPending ? (
              <>
                <RefreshCw size={14} className="animate-spin" />
                Running…
              </>
            ) : (
              <>
                <Play size={14} />
                Run Backtest
              </>
            )}
          </button>

          {mutation.isError && (
            <div
              className="flex items-center gap-2 text-xs px-3 py-2 rounded"
              style={{ backgroundColor: 'rgba(255,68,68,0.1)', color: 'var(--color-danger)' }}
            >
              <AlertCircle size={12} />
              {String(mutation.error)}
            </div>
          )}
        </div>

        {/* Script browser */}
        <div
          className="lg:col-span-2 rounded-lg border p-4"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <h2 className="text-xs font-semibold uppercase tracking-widest mb-3" style={{ color: 'var(--color-text-muted)' }}>
            Strategy Scripts
          </h2>
          {scripts.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-12 text-center gap-3">
              <FileCode2 size={32} style={{ color: 'var(--color-border)' }} />
              <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
                No .rhai scripts found in /scripts/
              </p>
              <p className="text-xs max-w-xs" style={{ color: 'var(--color-text-muted)' }}>
                Ask the agent to generate a strategy. Example prompt:
              </p>
              <div
                className="text-xs font-mono px-4 py-2 rounded text-left max-w-sm"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  color: 'var(--color-accent)',
                  border: '1px solid var(--color-border)',
                }}
              >
                "Create a Rhai strategy that buys BTC when RSI &lt; 30 and sells when RSI &gt; 70"
              </div>
            </div>
          ) : (
            <div className="space-y-2">
              {scripts.map((s) => (
                <div
                  key={s.path}
                  onClick={() => set('script', s.path)}
                  className={clsx(
                    'flex items-start gap-3 rounded-lg border p-3 cursor-pointer transition-colors',
                    config.script === s.path
                      ? 'border-[var(--color-accent)]'
                      : 'border-[var(--color-border)] hover:border-[rgba(0,255,136,0.3)]',
                  )}
                  style={{ backgroundColor: 'var(--color-surface-2)' }}
                >
                  <FileCode2
                    size={16}
                    className="mt-0.5 flex-shrink-0"
                    style={{
                      color: config.script === s.path
                        ? 'var(--color-accent)'
                        : 'var(--color-text-muted)',
                    }}
                  />
                  <div className="min-w-0">
                    <p className="text-sm font-mono font-semibold truncate" style={{ color: 'var(--color-text)' }}>
                      {s.name}
                    </p>
                    {s.description && (
                      <p className="text-xs mt-0.5 truncate" style={{ color: 'var(--color-text-muted)' }}>
                        {s.description}
                      </p>
                    )}
                    <p className="text-xs font-mono mt-1" style={{ color: 'var(--color-text-muted)' }}>
                      {s.path}
                    </p>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Results */}
      {result && (
        <div>
          <div className="flex items-center gap-2 mb-3">
            <BarChart2 size={14} style={{ color: 'var(--color-accent)' }} />
            <h2 className="text-sm font-semibold">
              Results — {result.script.split('/').pop()} / {result.symbol}
            </h2>
          </div>
          <ResultPanel result={result} />
        </div>
      )}
    </div>
  )
}
