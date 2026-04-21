import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import { type MarketSeries, POLY_BINARY_PRESETS } from '../hooks/useBacktestState'
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
  series_id?: string
  resolution_logic?: string
  threshold?: number | null
}

interface RunnerStatus {
  id: string
  status: 'starting' | 'running' | 'stopped' | 'error'
  started_at: string
  last_tick_at?: string
  next_tick_at?: string
  error?: string
}

interface LiveTrade {
  timestamp: string
  side: string
  price: number
  size: number
  pnl: number
  balance: number
}

interface RunnerResult {
  total_return_pct: number
  balance: number
  position: number
  total_trades: number
  win_rate_pct: number
  sharpe_ratio: number
  max_drawdown_pct: number
  all_trades: LiveTrade[]
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

// ── Live Equity Chart ────────────────────────────────────────────────

interface LiveEquityChartProps {
  trades: LiveTrade[]
  initialBalance: number
}

function LiveEquityChart({ trades, initialBalance }: LiveEquityChartProps) {
  const W = 480
  const H = 110
  const PAD = { top: 8, right: 8, bottom: 20, left: 44 }
  const inner = { w: W - PAD.left - PAD.right, h: H - PAD.top - PAD.bottom }

  // Build balance series starting from initialBalance
  const points: { x: number; y: number; trade: LiveTrade; i: number }[] = []
  const balances = [initialBalance, ...trades.map(t => t.balance)]

  const minBal = Math.min(...balances)
  const maxBal = Math.max(...balances)
  const spread = maxBal - minBal || initialBalance * 0.01

  const toX = (i: number) => PAD.left + (i / Math.max(balances.length - 1, 1)) * inner.w
  const toY = (b: number) => PAD.top + (1 - (b - minBal) / spread) * inner.h

  for (let i = 1; i < balances.length; i++) {
    points.push({ x: toX(i), y: toY(balances[i]), trade: trades[i - 1], i: i - 1 })
  }

  const polyPts = [
    `${toX(0)},${toY(balances[0])}`,
    ...points.map(p => `${p.x},${p.y}`),
  ].join(' ')

  const areaPath = [
    `M${toX(0)},${toY(balances[0])}`,
    ...points.map(p => `L${p.x},${p.y}`),
    `L${toX(balances.length - 1)},${PAD.top + inner.h}`,
    `L${toX(0)},${PAD.top + inner.h}`,
    'Z',
  ].join(' ')

  const isProfit = balances[balances.length - 1] >= initialBalance
  const lineColor = isProfit ? 'var(--color-accent)' : 'var(--color-danger)'

  // Y-axis labels
  const yLabels = [minBal, (minBal + maxBal) / 2, maxBal]

  // X-axis: show first, last, and one mid trade timestamp
  const xLabels: { x: number; label: string }[] = []
  if (trades.length > 0) {
    const fmtTime = (iso: string) => {
      try { return new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) } catch { return '' }
    }
    xLabels.push({ x: toX(1), label: fmtTime(trades[0].timestamp) })
    if (trades.length > 2) {
      const mid = Math.floor(trades.length / 2)
      xLabels.push({ x: toX(mid + 1), label: fmtTime(trades[mid].timestamp) })
    }
    xLabels.push({ x: toX(trades.length), label: fmtTime(trades[trades.length - 1].timestamp) })
  }

  const [hoveredIdx, setHoveredIdx] = useState<number | null>(null)
  const hoveredTrade = hoveredIdx !== null ? trades[hoveredIdx] : null

  return (
    <div className="relative">
      <svg
        viewBox={`0 0 ${W} ${H}`}
        style={{ width: '100%', height: H, overflow: 'visible' }}
      >
        {/* Grid lines */}
        {yLabels.map((v, i) => (
          <g key={i}>
            <line
              x1={PAD.left} x2={PAD.left + inner.w}
              y1={toY(v)} y2={toY(v)}
              stroke="var(--color-border)" strokeWidth={0.5} strokeDasharray="3,3"
            />
            <text
              x={PAD.left - 4} y={toY(v) + 3.5}
              textAnchor="end" fontSize={8}
              fill="var(--color-text-muted)"
            >
              ${v >= 1000 ? `${(v / 1000).toFixed(1)}k` : v.toFixed(0)}
            </text>
          </g>
        ))}

        {/* X-axis labels */}
        {xLabels.map((l, i) => (
          <text key={i} x={l.x} y={H - 3} textAnchor="middle" fontSize={8} fill="var(--color-text-muted)">
            {l.label}
          </text>
        ))}

        {/* Area fill */}
        {balances.length > 1 && (
          <path d={areaPath} fill={lineColor} fillOpacity={0.08} />
        )}

        {/* Equity line */}
        {balances.length > 1 && (
          <polyline
            points={polyPts}
            fill="none"
            stroke={lineColor}
            strokeWidth={1.5}
            strokeLinejoin="round"
            strokeLinecap="round"
          />
        )}

        {/* Start dot */}
        <circle cx={toX(0)} cy={toY(balances[0])} r={2.5} fill="var(--color-text-muted)" />

        {/* Trade markers */}
        {points.map((p) => {
          const isBuy = p.trade.side === 'buy'
          const col = isBuy ? 'var(--color-accent)' : 'var(--color-danger)'
          const size = hoveredIdx === p.i ? 5 : 3.5
          const path = isBuy
            ? `M${p.x},${p.y - size} L${p.x - size * 0.85},${p.y + size * 0.5} L${p.x + size * 0.85},${p.y + size * 0.5} Z`
            : `M${p.x},${p.y + size} L${p.x - size * 0.85},${p.y - size * 0.5} L${p.x + size * 0.85},${p.y - size * 0.5} Z`
          return (
            <g key={p.i}
              style={{ cursor: 'pointer' }}
              onMouseEnter={() => setHoveredIdx(p.i)}
              onMouseLeave={() => setHoveredIdx(null)}
            >
              {/* Hit area */}
              <circle cx={p.x} cy={p.y} r={7} fill="transparent" />
              <path d={path} fill={col} opacity={hoveredIdx === p.i ? 1 : 0.8} />
            </g>
          )
        })}

        {/* Hover vertical line */}
        {hoveredIdx !== null && points[hoveredIdx] && (
          <line
            x1={points[hoveredIdx].x} x2={points[hoveredIdx].x}
            y1={PAD.top} y2={PAD.top + inner.h}
            stroke="var(--color-border)" strokeWidth={1} strokeDasharray="3,2"
          />
        )}
      </svg>

      {/* Hover tooltip */}
      {hoveredTrade && (
        <div
          className="absolute top-0 right-0 rounded border px-2 py-1.5 text-xs space-y-0.5 pointer-events-none z-10"
          style={{
            backgroundColor: 'var(--color-surface)',
            borderColor: 'var(--color-border)',
            minWidth: 130,
          }}
        >
          <div className="flex justify-between gap-3">
            <span style={{ color: 'var(--color-text-muted)' }}>Side</span>
            <span style={{ color: hoveredTrade.side === 'buy' ? 'var(--color-accent)' : 'var(--color-danger)', fontWeight: 600 }}>
              {hoveredTrade.side.toUpperCase()}
            </span>
          </div>
          <div className="flex justify-between gap-3">
            <span style={{ color: 'var(--color-text-muted)' }}>Price</span>
            <span>${hoveredTrade.price.toFixed(2)}</span>
          </div>
          <div className="flex justify-between gap-3">
            <span style={{ color: 'var(--color-text-muted)' }}>Size</span>
            <span>{hoveredTrade.size.toFixed(4)}</span>
          </div>
          <div className="flex justify-between gap-3">
            <span style={{ color: 'var(--color-text-muted)' }}>PnL</span>
            <span style={{ color: hoveredTrade.pnl >= 0 ? 'var(--color-accent)' : 'var(--color-danger)' }}>
              {hoveredTrade.pnl >= 0 ? '+' : ''}{hoveredTrade.pnl.toFixed(2)}
            </span>
          </div>
          <div className="flex justify-between gap-3">
            <span style={{ color: 'var(--color-text-muted)' }}>Balance</span>
            <span>${hoveredTrade.balance.toFixed(2)}</span>
          </div>
          <div style={{ color: 'var(--color-text-muted)', marginTop: 2 }}>
            {(() => { try { return new Date(hoveredTrade.timestamp).toLocaleString() } catch { return hoveredTrade.timestamp } })()}
          </div>
        </div>
      )}
    </div>
  )
}

// ── Create Runner Modal ───────────────────────────────────────────────

export interface CreateModalProps {
  scripts: string[]
  onClose: () => void
  onCreated: () => void
  defaultScript?: string
}

export function CreateModal({ scripts, onClose, onCreated, defaultScript }: CreateModalProps) {
  // Load market series for polymarket_binary picker
  const { data: seriesData } = useQuery<{ series: MarketSeries[] }>({
    queryKey: ['backtest-series'],
    queryFn: () => apiFetch('/api/backtest/series'),
    staleTime: 10 * 60 * 1000,
  })
  const allSeries: MarketSeries[] = seriesData?.series ?? []

  const [form, setForm] = useState({
    name: '',
    script: defaultScript ?? scripts[0] ?? '',
    market_type: 'crypto',
    symbol: 'BTCUSDT',
    interval: '5m',
    mode: 'paper',
    initial_balance: 1000,
    fee_pct: 0.1,
    warmup_days: 7,
    series_id: '' as string,
    resolution_logic: 'price_up' as string,
    threshold: null as number | null,
  })
  const [error, setError] = useState('')

  function friendlyCreateError(message: string) {
    const m = message.toLowerCase()
    if (m.includes('wallet_address')) {
      return 'Live mode needs your Polymarket wallet address. Go to Settings → Config and set polymarket.wallet_address, then try again.'
    }
    if (m.includes('no active polymarket market') || m.includes('market series') || m.includes('token')) {
      return 'No active token was found for the selected series right now. Choose another built-in BTC/ETH series or try again in a minute.'
    }
    if (m.includes('insufficient wallet balance') || m.includes('required at least')) {
      return `Insufficient Polymarket wallet balance for live mode. ${message}`
    }
    if (m.includes('api_key') || m.includes('passphrase') || m.includes('secret')) {
      return 'Polymarket API credentials are incomplete. Please configure api_key, secret, and passphrase in Settings → Config.'
    }
    return message
  }

  const mutation = useMutation({
    mutationFn: () => apiPost('/api/live/strategies', form),
    onSuccess: () => { onCreated(); onClose() },
    onError: (e: Error) => setError(friendlyCreateError(e.message)),
  })

  function set<K extends keyof typeof form>(k: K, v: typeof form[K]) {
    setForm(f => ({ ...f, [k]: v }))
  }

  function onMarketTypeChange(mt: string) {
    if (mt === 'polymarket_binary') {
      // Auto-select first series (BTC 5m default)
      const firstSeries = allSeries[0] ?? POLY_BINARY_PRESETS[0]
      const sym = 'symbol' in firstSeries ? firstSeries.symbol : 'BTCUSDT'
      const cadence = 'cadence' in firstSeries ? firstSeries.cadence : ('defaultInterval' in firstSeries ? (firstSeries as { defaultInterval: string }).defaultInterval : '5m')
      const seriesId = 'id' in firstSeries ? firstSeries.id : ''
      const rl = 'resolution_logic' in firstSeries
        ? String(firstSeries.resolution_logic)
        : 'price_up'
      const th = 'threshold' in firstSeries ? (firstSeries.threshold ?? null) : null
      setForm(f => ({
        ...f,
        market_type: mt,
        // Keep the current mode — don't reset it
        symbol: sym,
        interval: cadence,
        fee_pct: 1.5,
        series_id: seriesId,
        resolution_logic: rl,
        threshold: th,
      }))
    } else {
      setForm(f => ({
        ...f,
        market_type: mt,
        mode: 'paper', // crypto only supports paper for now
        symbol: 'BTCUSDT',
        interval: '5m',
        fee_pct: 0.1,
        series_id: '',
        resolution_logic: 'price_up',
        threshold: null,
      }))
    }
  }

  function onSeriesChange(seriesId: string) {
    const s = allSeries.find(s => s.id === seriesId)
    if (s) {
      const rl = String(s.resolution_logic)
      setForm(f => ({
        ...f,
        series_id: s.id,
        symbol: s.symbol,
        interval: s.cadence,
        resolution_logic: rl,
        threshold: s.threshold ?? null,
      }))
    }
  }

  const currentSeries = allSeries.length > 0
    ? allSeries.find(s => s.symbol === form.symbol && s.cadence === form.interval)
    : POLY_BINARY_PRESETS.find(p => p.symbol === form.symbol && p.defaultInterval === form.interval)

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
                <option value="polymarket_binary">Polymarket Binary</option>
              </select>
            </div>
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Mode</label>
              <select className="w-full rounded px-3 py-2 text-sm" value={form.mode}
                onChange={e => set('mode', e.target.value)}
                disabled={form.market_type !== 'polymarket_binary'}>
                <option value="paper">Paper Trading</option>
                <option value="live" disabled={form.market_type !== 'polymarket_binary'}>Live Trading</option>
              </select>
            </div>
          </div>

          {form.market_type === 'polymarket_binary' ? (
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Market Series</label>
              <select
                className="w-full rounded px-3 py-2 text-sm"
                value={currentSeries?.id ?? ''}
                onChange={e => onSeriesChange(e.target.value)}
              >
                {(allSeries.length > 0 ? allSeries : POLY_BINARY_PRESETS).map(s => (
                  <option key={s.id} value={s.id}>{s.label}</option>
                ))}
              </select>
              <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                Underlying: <span className="font-mono">{form.symbol}</span>
                {' · '}Window: {form.interval}
                {' · '}Logic: <span className="font-mono">{form.resolution_logic}</span>
                {form.threshold !== null ? <> {' · '}Threshold: <span className="font-mono">{form.threshold}</span></> : null}
              </p>
            </div>
          ) : (
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Symbol</label>
                <input className="w-full rounded px-3 py-2 text-sm font-mono" value={form.symbol}
                  onChange={e => set('symbol', e.target.value.toUpperCase())}
                  placeholder="BTCUSDT" />
              </div>
              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Interval</label>
                <select className="w-full rounded px-3 py-2 text-sm" value={form.interval}
                  onChange={e => set('interval', e.target.value)}>
                  {['1m','3m','5m','15m','30m','1h','4h','1d'].map(i => <option key={i} value={i}>{i}</option>)}
                </select>
              </div>
            </div>
          )}

          {/* Paper trading fields — hidden in live mode */}
          {form.mode === 'paper' && (
            <div className="grid grid-cols-3 gap-3">
              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Initial Balance ($)</label>
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
          )}

          {/* Live mode notice */}
          {form.mode === 'live' && (
            <div
              className="rounded border px-3 py-2.5 text-xs space-y-1"
              style={{ backgroundColor: 'rgba(245,158,11,0.08)', borderColor: 'var(--color-warning)', color: 'var(--color-warning)' }}
            >
              <div className="flex items-center gap-1.5 font-semibold">
                <AlertCircle size={13} />
                Live Trading — Real Orders
              </div>
              <p style={{ color: 'var(--color-text-muted)' }}>
                This will send <strong>real orders</strong> to Polymarket via the CLOB API using your configured wallet.
                Ensure your Polymarket API key, secret, and passphrase are set in <strong>Settings → Config</strong> before starting.
              </p>
              {form.market_type !== 'polymarket_binary' && (
                <p className="font-medium">Live mode is only supported for Polymarket Binary markets.</p>
              )}
            </div>
          )}

          {error && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{error}</p>}
        </div>

        <div className="p-4 border-t flex gap-2" style={{ borderColor: 'var(--color-border)' }}>
          <button
            onClick={() => mutation.mutate()}
            disabled={
              !form.script || !form.symbol || mutation.isPending ||
              (form.mode === 'live' && form.market_type !== 'polymarket_binary')
            }
            className="flex-1 py-2 rounded text-sm font-medium disabled:opacity-50"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            {mutation.isPending ? 'Starting...' : form.mode === 'live' ? 'Start Live Strategy' : 'Start Strategy'}
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
            {config.mode === 'live' && (
              <span
                className="text-xs px-1.5 py-0.5 rounded flex-shrink-0 font-semibold"
                style={{ backgroundColor: 'rgba(245,158,11,0.15)', color: 'var(--color-warning)' }}
              >
                LIVE
              </span>
            )}
          </div>
          <p className="text-xs font-mono truncate" style={{ color: 'var(--color-text-muted)' }}>
            {config.script} · {config.symbol} · {config.interval} · {config.mode}
            {config.market_type === 'polymarket_binary' ? ` · ${config.resolution_logic ?? 'price_up'}${config.threshold !== undefined && config.threshold !== null ? `(${config.threshold})` : ''}` : ''}
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

      {/* P&L summary — paper mode only */}
      {result && config.mode === 'paper' && (
        <div className="grid grid-cols-4 gap-2 px-4 pb-3 text-xs">
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

      {/* Live mode summary — signal only, no paper P&L */}
      {config.mode === 'live' && (
        <div className="grid grid-cols-2 gap-2 px-4 pb-3 text-xs">
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Last Signal</div>
            <div className="font-semibold"
              style={{ color: result?.last_signal === 'buy' ? 'var(--color-accent)' : result?.last_signal === 'sell' ? 'var(--color-danger)' : 'var(--color-text-muted)' }}>
              {result?.last_signal || 'waiting...'}
            </div>
          </div>
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Next Tick</div>
            <div className="font-semibold" style={{ color: 'var(--color-text-muted)' }}>
              {status.next_tick_at ? fmt(status.next_tick_at) : '—'}
            </div>
          </div>
        </div>
      )}

      {/* Equity Chart — paper mode only */}
      {config.mode === 'paper' && result && result.all_trades?.length > 0 && (
        <div className="px-4 pb-2 border-t pt-3" style={{ borderColor: 'var(--color-border)' }}>
          <div className="flex items-center justify-between mb-1.5">
            <span className="text-xs font-medium" style={{ color: 'var(--color-text-muted)' }}>
              Equity Curve
            </span>
            <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              {result.all_trades.length} trades · ${result.balance.toFixed(2)}
            </span>
          </div>
          <LiveEquityChart trades={result.all_trades} initialBalance={config.initial_balance} />
        </div>
      )}

      {/* Live order/activity log */}
      {config.mode === 'live' && status.error && (
        <div className="mx-4 mb-3 px-3 py-2 rounded text-xs whitespace-pre-line"
          style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}>
          {status.error}
        </div>
      )}

      {config.mode === 'live' && !status.error && status.status === 'running' && (
        <div className="mx-4 mb-3 px-3 py-3 rounded text-xs text-center"
          style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)' }}>
          <Activity size={12} className="inline mr-1 animate-pulse" />
          Live runner active. Waiting for next signal...
        </div>
      )}

      {/* Placeholder when running but no trades yet */}
      {result && (!result.all_trades || result.all_trades.length === 0) && status.status === 'running' && (
        <div className="mx-4 mb-3 px-3 py-3 rounded text-xs text-center"
          style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)' }}>
          <Activity size={12} className="inline mr-1 animate-pulse" />
          Waiting for first trade...
        </div>
      )}

      {/* Error */}
      {status.error && status.status === 'error' && (
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
              <span style={{ color: 'var(--color-text-muted)' }}>{config.mode === 'live' ? 'Wallet Balance' : 'Balance'}</span>
              <span>${result?.balance.toFixed(2) ?? (config.mode === 'live' ? '—' : config.initial_balance)}</span>
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
