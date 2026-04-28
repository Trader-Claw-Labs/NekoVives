import { useState, useEffect, useRef } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import { type MarketSeries, POLY_BINARY_PRESETS } from '../hooks/useBacktestState'
import { useProfitCelebration } from '../hooks/useProfitCelebration'
import {
  Bot, Plus, Trash2, RefreshCw, X, StopCircle, RotateCcw,
  TrendingUp, TrendingDown, Activity, ChevronDown, ChevronUp, AlertCircle, ExternalLink, Copy,
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
  live_sizing_mode?: 'fixed' | 'percent'
  live_sizing_value?: number
  stop_loss_pct?: number | null
  early_fire_secs?: number | null
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

interface BacktestScript {
  name: string
  path: string
  description?: string
  last_run_stats?: {
    total_return_pct: number
    sharpe_ratio: number | null
    win_rate_pct: number
    total_trades: number
    run_date: string
  }
}

interface LiveFeedData {
  current_btc_price: number
  market_slug: string
  window_timestamp: number
  window_seconds_left: number
  price_to_beat: number
  yes_token_price: number
  no_token_price: number
  price_history: [number, number][]
}

interface LiveOrder {
  timestamp: string
  window_ts: number
  side: string
  token_id: string
  amount_usdc: number
  order_id: string
  status: string
  entry_price?: number
  result?: string
  pnl?: number
  stop_loss_triggered?: boolean
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
  live_feed?: LiveFeedData
  wallet_address?: string
  wallet_balance_usdc?: number
  live_orders?: LiveOrder[]
  live_wins?: number
  live_total_trades?: number
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

function fmtPct(v: number | null | undefined) {
  const safe = v ?? 0
  const color = safe >= 0 ? 'var(--color-accent)' : 'var(--color-danger)'
  return <span style={{ color }}>{safe >= 0 ? '+' : ''}{safe.toFixed(2)}%</span>
}

/** Format a number as USD with commas and 2 decimals. */
function fmtUSD(v: number | null | undefined): string {
  const safe = v ?? 0
  return safe.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
}

/** Compute absolute P&L in USD for a runner. */
function runnerPnlUSD(r: StoredRunner): number {
  if (r.result?.live_orders && r.result.live_orders.length > 0) {
    return r.result.live_orders.reduce((s, o) => s + (o.pnl ?? 0), 0)
  }
  if (r.result?.total_return_pct != null && r.config.initial_balance != null) {
    return (r.result.total_return_pct / 100) * r.config.initial_balance
  }
  return 0
}

/** Convert live_orders to LiveTrade[] for equity chart rendering. */
function liveOrdersToTrades(orders: LiveOrder[], initialBalance: number): LiveTrade[] {
  let balance = initialBalance
  return orders
    .filter(o => o.pnl != null)
    .map(o => {
      balance += o.pnl!
      return {
        timestamp: o.timestamp,
        side: o.side,
        price: o.entry_price ?? 0.5,
        size: o.amount_usdc,
        pnl: o.pnl!,
        balance,
      }
    })
}

/** Resettable total P&L — baseline is captured on Reset so the badge
 *  shows gains/losses since that point. Deleted strategies no longer
 *  contribute because we use the live current sum, not a monotonic max. */
interface StatsBaseline {
  pnl: number
  trades: number
  wins: number
}

function useResettableStats(runners: StoredRunner[]) {
  const [baseline, setBaseline] = useState<StatsBaseline>(() => {
    try {
      return JSON.parse(localStorage.getItem('live-strategies-stats-baseline') || '{"pnl":0,"trades":0,"wins":0}')
    } catch {
      return { pnl: 0, trades: 0, wins: 0 }
    }
  })

  const currentPnl = runners.reduce((s, r) => s + runnerPnlUSD(r), 0)
  const currentTrades = runners.reduce((s, r) => s + (r.config.mode === 'live' ? (r.result?.live_total_trades ?? 0) : (r.result?.total_trades ?? 0)), 0)
  const currentWins = runners.reduce((s, r) => {
    if (r.config.mode === 'live') {
      return s + (r.result?.live_wins ?? 0)
    }
    return s + Math.round((r.result?.win_rate_pct ?? 0) / 100 * (r.result?.total_trades ?? 0))
  }, 0)

  const reset = () => {
    const next: StatsBaseline = { pnl: currentPnl, trades: currentTrades, wins: currentWins }
    setBaseline(next)
    try {
      localStorage.setItem('live-strategies-stats-baseline', JSON.stringify(next))
    } catch {}
  }

  return {
    pnlDisplay: currentPnl - baseline.pnl,
    tradesDisplay: currentTrades - baseline.trades,
    winsDisplay: currentWins - baseline.wins,
    reset,
  }
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
            <span>${fmtUSD(hoveredTrade.balance)}</span>
          </div>
          <div style={{ color: 'var(--color-text-muted)', marginTop: 2 }}>
            {(() => { try { return new Date(hoveredTrade.timestamp).toLocaleString() } catch { return hoveredTrade.timestamp } })()}
          </div>
        </div>
      )}
    </div>
  )
}

// ── Missing API Key Modal ─────────────────────────────────────────────

function MissingApiKeyModal({ onClose }: { onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div
        className="rounded-lg border w-full max-w-md"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="p-4 border-b flex items-center justify-between" style={{ borderColor: 'var(--color-border)' }}>
          <div className="flex items-center gap-2">
            <AlertCircle size={16} style={{ color: 'var(--color-danger)' }} />
            <h2 className="font-semibold">Polymarket API Credentials Required</h2>
          </div>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10" style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>
        <div className="p-4 space-y-3 text-sm">
          <p style={{ color: 'var(--color-text-muted)' }}>
            Live trading on Polymarket requires API credentials to be configured.
          </p>
          <p style={{ color: 'var(--color-text-muted)' }}>
            Please go to <strong>Settings → Config</strong> and set:
          </p>
          <ul className="list-disc list-inside space-y-1 text-xs font-mono" style={{ color: 'var(--color-text-muted)' }}>
            <li>polymarket.api_key</li>
            <li>polymarket.secret</li>
            <li>polymarket.passphrase</li>
          </ul>
        </div>
        <div className="p-4 border-t flex gap-2" style={{ borderColor: 'var(--color-border)' }}>
          <button
            onClick={onClose}
            className="flex-1 py-2 rounded text-sm font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            Got it
          </button>
        </div>
      </div>
    </div>
  )
}

// ── Missing Private Key Modal ─────────────────────────────────────────

function MissingPrivateKeyModal({ onClose }: { onClose: () => void }) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div
        className="rounded-lg border w-full max-w-md"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="p-4 border-b flex items-center justify-between" style={{ borderColor: 'var(--color-border)' }}>
          <div className="flex items-center gap-2">
            <AlertCircle size={16} style={{ color: 'var(--color-danger)' }} />
            <h2 className="font-semibold">Private Key Required for Live Trading</h2>
          </div>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10" style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>
        <div className="p-4 space-y-3 text-sm">
          <p style={{ color: 'var(--color-text-muted)' }}>
            Live trading on Polymarket requires your wallet's <strong>private key</strong> to cryptographically sign each order (EIP-712).
          </p>
          <p style={{ color: 'var(--color-text-muted)' }}>
            Please go to <strong>Polymarket → Builder API Credentials</strong> and paste your private key in the <em>Private Key</em> field, then click Save.
          </p>
          <div className="rounded p-2.5 text-xs" style={{ backgroundColor: 'rgba(255,170,0,0.08)', borderLeft: '2px solid var(--color-warning)', color: 'var(--color-warning)' }}>
            Your private key is stored locally in your config file and is never sent to our servers. It is only used to sign orders on your machine.
          </div>
        </div>
        <div className="p-4 border-t flex gap-2" style={{ borderColor: 'var(--color-border)' }}>
          <button
            onClick={onClose}
            className="flex-1 py-2 rounded text-sm font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            Got it
          </button>
        </div>
      </div>
    </div>
  )
}

// ── Create Runner Modal ───────────────────────────────────────────────

export interface CreateModalProps {
  scripts: BacktestScript[]
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
    script: defaultScript ?? scripts[0]?.path ?? '',
    market_type: 'polymarket_binary',
    symbol: 'BTCUSDT',
    interval: '5m',
    mode: 'paper',
    initial_balance: 1000,
    fee_pct: 1.5,
    warmup_days: 7,
    series_id: 'btc_5m',
    resolution_logic: 'price_up',
    threshold: null as number | null,
    live_sizing_mode: 'percent' as 'fixed' | 'percent',
    live_sizing_value: 5,
    stop_loss_pct: null as number | null,
    early_fire_secs: null as number | null,
  })
  const [error, setError] = useState('')
  const [showMissingApiKeyModal, setShowMissingApiKeyModal] = useState(false)
  const [showMissingPrivateKeyModal, setShowMissingPrivateKeyModal] = useState(false)

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
    // Credentials rejected by CLOB (401): keys are present but do not match the wallet.
    // Must be checked BEFORE the "missing credentials" rule so the user sees a
    // regenerate-keys hint instead of the generic "credentials required" modal.
    if (m.includes('credentials rejected') || m.includes('invalid api key') || m.includes('401')) {
      return `Polymarket rejected your API credentials. They do not match the configured wallet. Open the Polymarket page and click "Regenerate API Credentials" with your current private key, then try again. (${message})`
    }
    if (m.includes('private_key') || m.includes('private key')) {
      setShowMissingPrivateKeyModal(true)
      return ''
    }
    // Only surface the "missing credentials" modal for the actual missing-config case.
    if (m.includes('requires polymarket.api_key') || m.includes('credentials incomplete')) {
      setShowMissingApiKeyModal(true)
      return ''
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
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={(e) => { if (e.target === e.currentTarget) onClose() }}>
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
            <select className="w-full rounded px-3 py-2 text-sm font-mono" value={form.script}
              onChange={e => set('script', e.target.value)}>
              {scripts.map(s => (
                <option key={s.path} value={s.path}>
                  {s.name} {s.last_run_stats ? `(${(s.last_run_stats.win_rate_pct ?? 0).toFixed(1)}% WR)` : ''}
                </option>
              ))}
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
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 ${
                    form.mode === 'live' ? 'bg-blue-500' : 'bg-gray-300 dark:bg-gray-600'
                  }`}
                  onClick={() => set('mode', form.mode === 'live' ? 'paper' : 'live')}
                  disabled={form.market_type !== 'polymarket_binary'}
                  aria-pressed={form.mode === 'live'}
                >
                  <span
                    className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                      form.mode === 'live' ? 'translate-x-6' : 'translate-x-1'
                    }`}
                  />
                </button>
                <span className="text-sm font-medium">
                  {form.mode === 'live' ? 'Live Trading' : 'Dry Run'}
                </span>
              </div>
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

          {/* Dry Run fields — hidden in live mode */}
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

          {/* Live sizing config — polymarket_binary supports these in both paper and live */}
          {form.market_type === 'polymarket_binary' && (
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Sizing Mode</label>
                <select
                  className="w-full rounded px-3 py-2 text-sm"
                  value={form.live_sizing_mode}
                  onChange={e => set('live_sizing_mode', e.target.value as 'fixed' | 'percent')}
                >
                  <option value="percent">% of Balance</option>
                  <option value="fixed">Fixed USD</option>
                </select>
              </div>
              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
                  {form.live_sizing_mode === 'fixed' ? 'Amount (USD)' : 'Max % of Balance'}
                </label>
                <input
                  type="number"
                  step={form.live_sizing_mode === 'fixed' ? 1 : 0.1}
                  min={form.live_sizing_mode === 'fixed' ? 5 : 0.1}
                  max={form.live_sizing_mode === 'fixed' ? undefined : 100}
                  className="w-full rounded px-3 py-2 text-sm"
                  value={form.live_sizing_value}
                  onChange={e => set('live_sizing_value', Number(e.target.value))}
                />
                <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                  {form.live_sizing_mode === 'fixed'
                    ? 'Fixed USDC amount per order (min $5)'
                    : 'Script fraction is capped at this %'}
                </p>
              </div>

              {/* Stop-loss */}
              <div>
                <label className="block text-xs font-medium mb-1" style={{ color: 'var(--color-text-muted)' }}>
                  Stop-Loss (% drop from entry)
                </label>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    className="flex-1 px-2 py-1.5 rounded text-xs"
                    style={{ background: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                    placeholder="e.g. 40 → exit if price drops 40%"
                    step={5}
                    min={5}
                    max={90}
                    value={form.stop_loss_pct != null ? form.stop_loss_pct * 100 : ''}
                    onChange={e => set('stop_loss_pct', e.target.value === '' ? null : Number(e.target.value) / 100)}
                  />
                  <button
                    type="button"
                    className="text-[10px] px-2 py-1 rounded"
                    style={{ background: form.stop_loss_pct == null ? 'var(--color-surface-3)' : 'rgba(239,68,68,0.15)', color: form.stop_loss_pct == null ? 'var(--color-text-muted)' : 'var(--color-danger)' }}
                    onClick={() => set('stop_loss_pct', form.stop_loss_pct == null ? 0.40 : null)}
                  >
                    {form.stop_loss_pct == null ? 'Enable' : 'Disable'}
                  </button>
                </div>
                <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                  {form.stop_loss_pct != null
                    ? `Exit early if token drops ${(form.stop_loss_pct * 100).toFixed(0)}% from entry — limits max loss per trade`
                    : 'Disabled — position held until market resolves'}
                </p>
              </div>

              {/* Early fire */}
              <div>
                <label className="block text-[11px] font-medium mb-1" style={{ color: 'var(--color-text-muted)' }}>
                  Early Fire (seconds before candle close)
                </label>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    className="w-20 rounded border px-2 py-1 text-xs"
                    style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                    placeholder="0"
                    min={0}
                    max={55}
                    value={form.early_fire_secs != null ? form.early_fire_secs : ''}
                    onChange={e => set('early_fire_secs', e.target.value === '' ? null : Number(e.target.value))}
                  />
                  <button
                    type="button"
                    className="text-[10px] px-2 py-1 rounded"
                    style={{ background: form.early_fire_secs == null ? 'var(--color-surface-3)' : 'rgba(99,102,241,0.15)', color: form.early_fire_secs == null ? 'var(--color-text-muted)' : '#818cf8' }}
                    onClick={() => set('early_fire_secs', form.early_fire_secs == null ? 10 : null)}
                  >
                    {form.early_fire_secs == null ? 'Enable' : 'Disable'}
                  </button>
                </div>
                <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                  {form.early_fire_secs != null && form.early_fire_secs > 0
                    ? `Order placed ${form.early_fire_secs}s before candle close — avoids bot-crowding at minute boundary`
                    : 'Disabled — order placed at candle close'}
                </p>
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
      {showMissingApiKeyModal && <MissingApiKeyModal onClose={() => setShowMissingApiKeyModal(false)} />}
      {showMissingPrivateKeyModal && <MissingPrivateKeyModal onClose={() => setShowMissingPrivateKeyModal(false)} />}
    </div>
  )
}

// ── Low Balance Modal ─────────────────────────────────────────────────

function LowBalanceModal({
  balance,
  walletAddress,
  onClose
}: {
  balance: number
  walletAddress: string
  onClose: () => void
}) {
  const handleCopy = () => {
    navigator.clipboard.writeText(walletAddress)
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={(e) => { if (e.target === e.currentTarget) onClose() }}>
      <div
        className="rounded-lg border w-full max-w-md"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-warning)' }}
      >
        <div className="p-4 border-b flex items-center justify-between" style={{ borderColor: 'var(--color-border)' }}>
          <div className="flex items-center gap-2">
            <AlertCircle size={18} style={{ color: 'var(--color-warning)' }} />
            <h2 className="font-semibold" style={{ color: 'var(--color-warning)' }}>Insufficient Wallet Balance</h2>
          </div>
          <button onClick={onClose} className="p-1 rounded hover:bg-white/10" style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>

        <div className="p-4 space-y-4 text-sm">
          <p style={{ color: 'var(--color-text-muted)' }}>
            Your Polymarket wallet does not have enough balance to run live trades effectively.
          </p>

          <div className="grid grid-cols-2 gap-4">
            <div className="rounded p-3 border" style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)' }}>
              <div className="text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>Current Balance</div>
              <div className="font-semibold text-lg" style={{ color: 'var(--color-danger)' }}>${fmtUSD(balance)}</div>
            </div>
            <div className="rounded p-3 border" style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)' }}>
              <div className="text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>Minimum Required</div>
              <div className="font-semibold text-lg">$10.00</div>
            </div>
          </div>

          <div className="rounded p-3 border space-y-2" style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)' }}>
            <div className="text-xs font-medium" style={{ color: 'var(--color-text-muted)' }}>
              Send USDC.e (Polygon) to your wallet:
            </div>
            <div className="flex items-center gap-2">
              <code className="flex-1 px-2 py-1.5 rounded text-xs break-all" style={{ backgroundColor: 'rgba(0,0,0,0.2)' }}>
                {walletAddress}
              </code>
              <button
                onClick={handleCopy}
                className="p-1.5 rounded hover:bg-white/10"
                title="Copy Address"
              >
                <Copy size={14} />
              </button>
            </div>
          </div>
        </div>

        <div className="p-4 border-t flex justify-end" style={{ borderColor: 'var(--color-border)' }}>
          <button
            onClick={onClose}
            className="px-4 py-2 rounded text-sm font-medium hover:bg-white/5"
          >
            I understand
          </button>
        </div>
      </div>
    </div>
  )
}

// ── Delete Confirmation Modal ─────────────────────────────────────────

function DeleteConfirmModal({
  name,
  onConfirm,
  onCancel,
}: {
  name: string
  onConfirm: () => void
  onCancel: () => void
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={(e) => { if (e.target === e.currentTarget) onCancel() }}
    >
      <div
        className="rounded-lg border w-full max-w-sm"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="p-4 border-b flex items-center justify-between" style={{ borderColor: 'var(--color-border)' }}>
          <div className="flex items-center gap-2">
            <AlertCircle size={16} style={{ color: 'var(--color-danger)' }} />
            <h2 className="font-semibold">Delete Strategy</h2>
          </div>
          <button onClick={onCancel} className="p-1 rounded hover:bg-white/10" style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>
        <div className="p-4 text-sm" style={{ color: 'var(--color-text-muted)' }}>
          Are you sure you want to delete <strong style={{ color: 'var(--color-text)' }}>{name}</strong>? This action cannot be undone.
        </div>
        <div className="p-4 border-t flex gap-2" style={{ borderColor: 'var(--color-border)' }}>
          <button
            onClick={onConfirm}
            className="flex-1 py-2 rounded text-sm font-medium"
            style={{ backgroundColor: 'var(--color-danger)', color: '#fff' }}
          >
            Delete
          </button>
          <button
            onClick={onCancel}
            className="px-4 py-2 rounded text-sm border hover:bg-white/5"
            style={{ borderColor: 'var(--color-border)' }}
          >
            Cancel
          </button>
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

function maskAddress(addr?: string): string {
  if (!addr) return '—'
  if (addr.length <= 12) return addr
  return `${addr.slice(0, 6)}...${addr.slice(-4)}`
}

function MiniPriceChart({ history }: { history: [number, number][] }) {
  if (history.length < 2) return null
  const W = 320
  const H = 60
  const PAD = 4
  const times = history.map(([t]) => t)
  const prices = history.map(([, p]) => p)
  const minP = Math.min(...prices)
  const maxP = Math.max(...prices)
  const range = maxP - minP || 1
  const minT = times[0]
  const maxT = times[times.length - 1]
  const timeRange = maxT - minT || 1
  // Use real timestamps for X so gaps in data are reflected accurately.
  const toX = (t: number) => PAD + ((t - minT) / timeRange) * (W - PAD * 2)
  const toY = (p: number) => H - PAD - ((p - minP) / range) * (H - PAD * 2)
  const points = history.map(([t, p]) => `${toX(t)},${toY(p)}`).join(' ')
  const isUp = prices[prices.length - 1] >= prices[0]
  const color = isUp ? 'var(--color-accent)' : 'var(--color-danger)'
  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full" style={{ height: H }}>
      <polyline points={points} fill="none" stroke={color} strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" />
      <circle cx={toX(times[times.length - 1])} cy={toY(prices[prices.length - 1])} r={2.5} fill={color} />
    </svg>
  )
}

function LiveFeedPanel({ feed, walletBalance, liveOrders }: { feed: LiveFeedData; walletBalance?: number; liveOrders?: LiveOrder[] }) {
  // Deterministic countdown: window is always 5 min (300s) from window_timestamp.
  // Recalculates every second from wall-clock time so it stays accurate even
  // if the WebSocket feed has gaps or latency.
  const WINDOW_DURATION = 300
  const [secondsLeft, setSecondsLeft] = useState(() =>
    Math.max(0, feed.window_timestamp + WINDOW_DURATION - Math.floor(Date.now() / 1000))
  )
  const [isNewWindow, setIsNewWindow] = useState(false)
  const prevWindowTsRef = useRef(feed.window_timestamp)

  useEffect(() => {
    const interval = setInterval(() => {
      setSecondsLeft(
        Math.max(0, feed.window_timestamp + WINDOW_DURATION - Math.floor(Date.now() / 1000))
      )
    }, 1000)
    return () => clearInterval(interval)
  }, [feed.window_timestamp])

  // Detect window change and flash animation
  useEffect(() => {
    if (feed.window_timestamp !== prevWindowTsRef.current) {
      prevWindowTsRef.current = feed.window_timestamp
      setIsNewWindow(true)
      const t = setTimeout(() => setIsNewWindow(false), 2500)
      return () => clearTimeout(t)
    }
  }, [feed.window_timestamp])

  const currentAboveBeat = (feed.current_btc_price ?? 0) >= (feed.price_to_beat ?? 0)
  const currentWindowOrder = liveOrders?.find(o => o.window_ts === feed.window_timestamp)
  const mins = Math.floor(Math.max(0, secondsLeft) / 60)
  const secs = Math.max(0, secondsLeft) % 60
  const justStarted = secondsLeft >= 295

  return (
    <div className="mx-4 mb-3 px-4 py-4 rounded border space-y-4" style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)' }}>
      {/* Window change flash banner */}
      {isNewWindow && (
        <div className="-mx-4 -mt-4 mb-2 px-4 py-1.5 text-[10px] font-bold uppercase tracking-wider text-center animate-pulse"
          style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
          New Window Started — Evaluating Strategy…
        </div>
      )}
      {/* Evaluating indicator during first ~5s of window */}
      {justStarted && !isNewWindow && (
        <div className="-mx-4 -mt-4 mb-2 px-4 py-1.5 text-[10px] font-bold uppercase tracking-wider text-center"
          style={{ backgroundColor: 'rgba(245,158,11,0.15)', color: 'var(--color-warning)' }}>
          <Activity size={10} className="inline mr-1" />
          Evaluating Strategy…
        </div>
      )}
      {/* Order placed indicator for current window */}
      {currentWindowOrder && (
        <div className="-mx-4 -mt-4 mb-2 px-4 py-1.5 text-[10px] font-bold uppercase tracking-wider text-center"
          style={{ backgroundColor: 'rgba(74,222,128,0.15)', color: 'var(--color-accent)' }}>
          <TrendingUp size={10} className="inline mr-1" />
          Order Placed — {currentWindowOrder.side.toUpperCase()} ${currentWindowOrder.amount_usdc.toFixed(0)}
        </div>
      )}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <TrendingUp size={16} style={{ color: 'var(--color-accent)' }} />
          <span className="text-sm font-bold">Bitcoin Up or Down - 5 Minutes</span>
        </div>
        <div className="flex items-center gap-4">
          {typeof walletBalance === 'number' && (
            <div className="text-right mr-2">
              <div className="text-[10px] uppercase font-bold" style={{ color: 'var(--color-text-muted)' }}>Wallet</div>
              <div className="text-sm font-bold" style={{ color: walletBalance < 10 ? 'var(--color-warning)' : 'var(--color-accent)' }}>
                ${fmtUSD(walletBalance)}
              </div>
            </div>
          )}
          <div className="text-right">
            <div className="text-[10px] uppercase font-bold" style={{ color: 'var(--color-text-muted)' }}>Mins</div>
            <div className="text-xl font-bold leading-none" style={{ color: 'var(--color-danger)' }}>{String(mins).padStart(2, '0')}</div>
          </div>
          <div className="text-right">
            <div className="text-[10px] uppercase font-bold" style={{ color: 'var(--color-text-muted)' }}>Secs</div>
            <div className="text-xl font-bold leading-none" style={{ color: 'var(--color-danger)' }}>{String(secs).padStart(2, '0')}</div>
          </div>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-6">
        <div>
          <div className="text-xs font-medium mb-1" style={{ color: 'var(--color-text-muted)' }}>Price To Beat</div>
          <div className="text-xl font-bold">${(feed.price_to_beat ?? 0).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}</div>
        </div>
        <div>
          <div className="text-xs font-medium mb-1" style={{ color: 'var(--color-text-muted)' }}>
            Current Price
            <span className="ml-2 text-[10px]" style={{ color: 'var(--color-danger)' }}>
              ▼ ${Math.abs((feed.current_btc_price ?? 0) - (feed.price_to_beat ?? 0)).toFixed(0)}
            </span>
          </div>
          <div className="text-xl font-bold" style={{ color: currentAboveBeat ? 'var(--color-accent)' : '#f59e0b' }}>
            ${(feed.current_btc_price ?? 0).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
          </div>
        </div>
      </div>

      {feed.price_history && feed.price_history.length > 1 && (
        <div className="pt-1">
          <MiniPriceChart history={feed.price_history} />
        </div>
      )}

      <div className="flex items-center gap-3 pt-2">
        <div className="flex-1 flex items-center justify-between p-2 rounded bg-green-500/10 border border-green-500/20">
          <span className="text-xs font-bold text-green-500">Up</span>
          <span className="text-sm font-bold text-green-500">{(feed.yes_token_price ?? 0) > 0 ? `${((feed.yes_token_price ?? 0) * 100).toFixed(0)}¢` : '—'}</span>
        </div>
        <div className="flex-1 flex items-center justify-between p-2 rounded bg-white/5 border border-white/10">
          <span className="text-xs font-bold text-white/40">Down</span>
          <span className="text-sm font-bold text-white/40">{(feed.no_token_price ?? 0) > 0 ? `${((feed.no_token_price ?? 0) * 100).toFixed(0)}¢` : '—'}</span>
        </div>
      </div>

      <div className="flex items-center justify-between pt-1 border-t border-white/5">
        <div className="text-[10px] font-mono opacity-40 truncate flex-1">
          {feed.market_slug}
        </div>
        <a
          href={`https://polymarket.com/event/${feed.market_slug}`}
          target="_blank"
          rel="noreferrer"
          className="text-[10px] font-bold uppercase tracking-wider flex items-center gap-1 ml-2"
          style={{ color: 'var(--color-accent)' }}
        >
          View on Poly <ExternalLink size={10} />
        </a>
      </div>
    </div>
  )
}

interface RunnerCardProps {
  runner: StoredRunner
  onStop: () => void
  onRestart: () => void
  onDelete: () => void
}

function RunnerCard({ runner, onStop, onRestart, onDelete }: RunnerCardProps) {
  const [expanded, setExpanded] = useState(() => {
    try {
      const stored = localStorage.getItem(`runner-expanded-${runner.config.id}`)
      return stored === null ? false : stored === 'true'
    } catch {
      return false
    }
  })
  const toggleExpanded = () => {
    setExpanded(e => {
      const next = !e
      try { localStorage.setItem(`runner-expanded-${runner.config.id}`, String(next)) } catch {}
      return next
    })
  }
  const [showLog, setShowLog] = useState(false)
  const [showLowBalanceModal, setShowLowBalanceModal] = useState(false)
  const [lowBalanceShownOnce, setLowBalanceShownOnce] = useState(() => {
    try {
      return sessionStorage.getItem(`low-balance-shown-${runner.config.id}`) === 'true'
    } catch {
      return false
    }
  })
  const { celebrate } = useProfitCelebration()
  const prevTradesRef = useRef<number>(0)
  const { config, status, result } = runner
  const isRunning = status.status === 'running' || status.status === 'starting'

  useEffect(() => {
    if (
      config.mode === 'live' &&
      typeof result?.wallet_balance_usdc === 'number' &&
      result.wallet_balance_usdc < 10 &&
      !lowBalanceShownOnce
    ) {
      setShowLowBalanceModal(true)
      setLowBalanceShownOnce(true)
      try {
        sessionStorage.setItem(`low-balance-shown-${config.id}`, 'true')
      } catch {
        // ignore
      }
    }
  }, [config.mode, config.id, result?.wallet_balance_usdc, lowBalanceShownOnce])

  // Trigger celebration on profitable trades (paper mode only — live trades are real orders)
  useEffect(() => {
    if (config.mode === 'live') return
    const trades = config.market_type === 'polymarket_binary' ? result?.live_orders : result?.all_trades
    if (!trades) return
    const newTrades = trades.slice(prevTradesRef.current)
    const profitTrades = newTrades.filter((t: any) => t.pnl && t.pnl > 0)
    if (profitTrades.length > 0) {
      celebrate()
    }
    prevTradesRef.current = trades.length
  }, [result?.all_trades, result?.live_orders, celebrate, config.mode, config.market_type])

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
            <h3 className="text-sm font-semibold truncate">{config.name || config.script.split('/').pop()}</h3>
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
            {config.script.split('/').pop()} · {config.symbol} · {config.interval} · {config.mode === 'paper' ? 'dry run' : config.mode}
            {config.market_type === 'polymarket_binary' ? ` · ${config.resolution_logic ?? 'price_up'}${config.threshold !== undefined && config.threshold !== null ? `(${config.threshold})` : ''}` : ''}
            {config.market_type === 'polymarket_binary' && config.live_sizing_mode
              ? ` · ${config.live_sizing_mode === 'percent' ? `${config.live_sizing_value}%` : `$${config.live_sizing_value}`}`
              : ''}
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
          <button onClick={toggleExpanded}
            className="p-1.5 rounded hover:bg-white/5" style={{ color: 'var(--color-text-muted)' }}>
            {expanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
          </button>
        </div>
      </div>

      {/* P&L summary — paper mode only (non-polymarket) */}
      {result && config.mode === 'paper' && config.market_type !== 'polymarket_binary' && (
        <div className="grid grid-cols-4 gap-2 px-4 pb-3 text-xs">
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Return</div>
            <div className="font-semibold">{fmtPct(result.total_return_pct)}</div>
          </div>
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Win Rate</div>
            <div className="font-semibold">{(result.win_rate_pct ?? 0).toFixed(1)}%</div>
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

      {/* Live mode summary — live trades + signal (polymarket_binary in any mode) */}
      {config.market_type === 'polymarket_binary' && (
        <div className="grid grid-cols-5 gap-2 px-4 pb-3 text-xs">
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Last Signal</div>
            <div className="font-semibold"
              style={{ color: result?.last_signal === 'buy' ? 'var(--color-accent)' : result?.last_signal === 'sell' ? 'var(--color-danger)' : 'var(--color-text-muted)' }}>
              {result?.last_signal || 'waiting...'}
            </div>
          </div>
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Total Trades</div>
            <div className="font-semibold">{result?.live_total_trades ?? 0}</div>
          </div>
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>Win Rate</div>
            <div className="font-semibold">
              {(() => {
                const total = result?.live_total_trades ?? 0
                const wins = result?.live_wins ?? 0
                return total > 0 ? `${((wins / total) * 100).toFixed(1)}%` : '—'
              })()}
            </div>
          </div>
          <div>
            <div style={{ color: 'var(--color-text-muted)' }}>P&L</div>
            <div className="font-semibold" style={{
              color: (result?.live_orders?.reduce((s, o) => s + (o.pnl ?? 0), 0) ?? 0) >= 0
                ? 'var(--color-accent)' : 'var(--color-danger)'
            }}>
              {(() => {
                const pnl = result?.live_orders?.reduce((s, o) => s + (o.pnl ?? 0), 0) ?? 0
                return `${pnl >= 0 ? '+' : ''}$${fmtUSD(pnl)}`
              })()}
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

      {expanded && (
        <>

      {/* Equity Chart — paper mode (crypto) or polymarket_binary (any mode) */}
      {config.mode === 'paper' && config.market_type !== 'polymarket_binary' && result && result.all_trades?.length > 0 && (
        <div className="px-4 pb-2 border-t pt-3" style={{ borderColor: 'var(--color-border)' }}>
          <div className="flex items-center justify-between mb-1.5">
            <span className="text-xs font-medium" style={{ color: 'var(--color-text-muted)' }}>
              Equity Curve
            </span>
            <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              {result.all_trades.length} trades · ${fmtUSD(result.balance)}
            </span>
          </div>
          <LiveEquityChart trades={result.all_trades} initialBalance={config.initial_balance} />
        </div>
      )}

      {/* Equity Chart for polymarket_binary live_orders */}
      {config.market_type === 'polymarket_binary' && result?.live_orders && result.live_orders.length > 0 && (
        <div className="px-4 pb-2 border-t pt-3" style={{ borderColor: 'var(--color-border)' }}>
          <div className="flex items-center justify-between mb-1.5">
            <span className="text-xs font-medium" style={{ color: 'var(--color-text-muted)' }}>
              Equity Curve
            </span>
            <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              {result.live_orders.filter(o => o.pnl != null).length} trades · ${fmtUSD(config.initial_balance + runnerPnlUSD(runner))}
            </span>
          </div>
          <LiveEquityChart trades={liveOrdersToTrades(result.live_orders, config.initial_balance)} initialBalance={config.initial_balance} />
        </div>
      )}

      {/* Live order/activity log */}
      {config.market_type === 'polymarket_binary' && status.error && (
        <div className="mx-4 mb-3 rounded border overflow-hidden" style={{ borderColor: 'var(--color-border)' }}>
          <div className="px-3 py-1.5 text-xs font-semibold flex items-center justify-between"
            style={{ backgroundColor: 'var(--color-base)', borderBottom: showLog ? '1px solid var(--color-border)' : 'none' }}>
            <span className="flex items-center gap-1.5">
              <Activity size={12} style={{ color: 'var(--color-text-muted)' }} />
              Activity Log
            </span>
            <button onClick={() => setShowLog(l => !l)} className="text-[10px] hover:underline" style={{ color: 'var(--color-text-muted)' }}>
              {showLog ? 'Hide' : 'Show'}
            </button>
          </div>
          {showLog && (
            <div className="px-3 py-2 text-xs whitespace-pre-wrap"
              style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)', maxHeight: 200, overflowY: 'auto' }}>
              {status.error}
            </div>
          )}
        </div>
      )}

      {/* Live Feed Panel for binary recurring markets */}
      {config.market_type === 'polymarket_binary' && result?.live_feed && (
        <LiveFeedPanel feed={result.live_feed} walletBalance={result.wallet_balance_usdc} liveOrders={result.live_orders} />
      )}

      {/* Live order history */}
      {config.market_type === 'polymarket_binary' && result?.live_orders && result.live_orders.length > 0 && (
        <div className="mx-4 mb-3 rounded border overflow-hidden" style={{ borderColor: 'var(--color-border)' }}>
          <div className="px-3 py-2 text-xs font-semibold border-b flex items-center gap-2" style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-base)' }}>
            <TrendingUp size={12} style={{ color: 'var(--color-accent)' }} />
            Order History
          </div>
          <table className="w-full text-xs">
            <thead>
              <tr style={{ backgroundColor: 'var(--color-base)' }}>
                <th className="px-3 py-1.5 text-left font-medium" style={{ color: 'var(--color-text-muted)' }}>Time</th>
                <th className="px-3 py-1.5 text-left font-medium" style={{ color: 'var(--color-text-muted)' }}>Side</th>
                <th className="px-3 py-1.5 text-right font-medium" style={{ color: 'var(--color-text-muted)' }}>Amount</th>
                <th className="px-3 py-1.5 text-right font-medium" style={{ color: 'var(--color-text-muted)' }}>Entry Price</th>
                <th className="px-3 py-1.5 text-left font-medium" style={{ color: 'var(--color-text-muted)' }}>Status</th>
                <th className="px-3 py-1.5 text-left font-medium" style={{ color: 'var(--color-text-muted)' }}>Result</th>
                <th className="px-3 py-1.5 text-right font-medium" style={{ color: 'var(--color-text-muted)' }}>P&amp;L</th>
              </tr>
            </thead>
            <tbody>
              {[...result.live_orders].reverse().map((order, i) => (
                <tr key={i} className="border-t" style={{ borderColor: 'var(--color-border)' }}>
                  <td className="px-3 py-1.5 font-mono" style={{ color: 'var(--color-text-muted)' }}>
                    {fmt(order.timestamp)}
                  </td>
                  <td className="px-3 py-1.5 font-semibold" style={{
                    color: order.side.startsWith('yes') || order.side === 'buy'
                      ? 'var(--color-accent)'
                      : order.side.startsWith('no') || order.side === 'sell'
                        ? 'var(--color-danger)'
                        : 'var(--color-text-muted)'
                  }}>
                    {order.side.toUpperCase()}
                  </td>
                  <td className="px-3 py-1.5 text-right font-mono">
                    ${order.amount_usdc.toFixed(2)}
                  </td>
                  <td className="px-3 py-1.5 text-right font-mono">
                    {order.entry_price != null ? `$${order.entry_price.toFixed(4)}` : '—'}
                  </td>
                  <td className="px-3 py-1.5">
                    <span className="text-[10px] px-1.5 py-0.5 rounded" style={{
                      backgroundColor: order.status === 'matched' || order.status === 'filled'
                        ? 'rgba(74,222,128,0.15)'
                        : 'rgba(245,158,11,0.15)',
                      color: order.status === 'matched' || order.status === 'filled'
                        ? 'var(--color-accent)'
                        : 'var(--color-warning)',
                    }}>
                      {order.status}
                    </span>
                  </td>
                  <td className="px-3 py-1.5">
                    {order.result ? (
                      <span className="text-[10px] px-1.5 py-0.5 rounded font-semibold" style={{
                        backgroundColor:
                          order.result === 'WIN'  ? 'rgba(74,222,128,0.15)' :
                          order.result === 'STOP' ? 'rgba(251,191,36,0.15)' :
                                                    'rgba(239,68,68,0.15)',
                        color:
                          order.result === 'WIN'  ? 'var(--color-accent)' :
                          order.result === 'STOP' ? '#fbbf24' :
                                                    'var(--color-danger)',
                      }}>
                        {order.result === 'STOP' ? '⏹ STOP' : order.result}
                      </span>
                    ) : (
                      <span style={{ color: 'var(--color-text-muted)' }}>—</span>
                    )}
                  </td>
                  <td className="px-3 py-1.5 text-right font-mono">
                    {order.pnl != null ? (
                      <span style={{ color: order.pnl >= 0 ? 'var(--color-accent)' : 'var(--color-danger)' }}>
                        {order.pnl >= 0 ? '+' : ''}{order.pnl.toFixed(2)}
                      </span>
                    ) : (
                      <span style={{ color: 'var(--color-text-muted)' }}>—</span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {config.mode === 'live' && !status.error && status.status === 'running' && !result?.live_feed && (
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
            {config.mode === 'paper' && (
              <div className="flex justify-between">
                <span style={{ color: 'var(--color-text-muted)' }}>Balance</span>
                <span>${fmtUSD(result?.balance)}</span>
              </div>
            )}
            {config.mode === 'live' && (
              typeof result?.wallet_balance_usdc === 'number' ? (
                <div className="flex justify-between">
                  <span style={{ color: 'var(--color-text-muted)' }}>Wallet Balance</span>
                  <span style={{ color: result.wallet_balance_usdc < 10 ? 'var(--color-warning)' : 'inherit' }}>
                    ${fmtUSD(result.wallet_balance_usdc)}
                  </span>
                </div>
              ) : (
                <div className="flex justify-between text-[10px]" style={{ color: 'var(--color-warning)' }}>
                  <span>Balance Unknown</span>
                  <span className="text-right">Fund your Polymarket wallet</span>
                </div>
              )
            )}
            {config.mode === 'live' && result?.wallet_address && (
              <div className="flex justify-between">
                <span style={{ color: 'var(--color-text-muted)' }}>Wallet</span>
                <span className="font-mono text-xs">{maskAddress(result.wallet_address)}</span>
              </div>
            )}
            {result && (
              <>
                <div className="flex justify-between">
                  <span style={{ color: 'var(--color-text-muted)' }}>Sharpe</span>
                  <span>{(result.sharpe_ratio ?? 0).toFixed(2)}</span>
                </div>
                <div className="flex justify-between">
                  <span style={{ color: 'var(--color-text-muted)' }}>Max DD</span>
                  <span style={{ color: 'var(--color-danger)' }}>{(result.max_drawdown_pct ?? 0).toFixed(2)}%</span>
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
      </>)}
      {showLowBalanceModal && result?.wallet_address && typeof result?.wallet_balance_usdc === 'number' && (
        <LowBalanceModal
          balance={result.wallet_balance_usdc}
          walletAddress={result.wallet_address}
          onClose={() => setShowLowBalanceModal(false)}
        />
      )}
    </div>
  )
}

// ── Main Page ─────────────────────────────────────────────────────────

export default function LiveStrategies() {
  const [showCreate, setShowCreate] = useState(false)
  const [showCelebrationSettings, setShowCelebrationSettings] = useState(false)
  const { settings, setSettings } = useProfitCelebration()
  const qc = useQueryClient()

  const { data, isLoading, refetch } = useQuery<LiveListResponse>({
    queryKey: ['live-strategies'],
    queryFn: () => apiFetch<LiveListResponse>('/api/live/strategies').catch(() => ({ runners: [] })),
    refetchInterval: 5_000,
  })

  const { data: scriptsData } = useQuery<{ scripts: BacktestScript[] }>({
    queryKey: ['backtest-scripts'],
    queryFn: () => apiFetch<{ scripts: BacktestScript[] }>('/api/backtest/scripts').catch(() => ({ scripts: [] })),
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
  const scripts = scriptsData?.scripts ?? []
  const running = runners.filter(r => r.status.status === 'running').length
  const { pnlDisplay: totalPnl, tradesDisplay: totalTradesDelta, winsDisplay: totalWinsDelta, reset: resetStats } = useResettableStats(runners)
  const [deleteTarget, setDeleteTarget] = useState<StoredRunner | null>(null)

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
          <span className="text-xs px-2 py-0.5 rounded font-semibold"
            style={{
              backgroundColor: totalPnl >= 0 ? 'rgba(74,222,128,0.15)' : 'rgba(239,68,68,0.15)',
              color: totalPnl >= 0 ? 'var(--color-accent)' : 'var(--color-danger)',
            }}>
            Total P&L: {totalPnl >= 0 ? '+' : ''}${fmtUSD(totalPnl)}
          </span>
          <button
            onClick={resetStats}
            className="text-[10px] px-2 py-0.5 rounded border hover:bg-white/5"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
            title="Reset Stats"
          >
            Reset
          </button>
        </div>
        <div className="flex gap-2">
          <div className="relative">
            <button
              onClick={() => setShowCelebrationSettings(!showCelebrationSettings)}
              className="p-2 rounded border hover:bg-white/5 h-[34px] flex items-center justify-center"
              style={{ borderColor: 'var(--color-border)', color: settings.enabled ? 'var(--color-accent)' : 'var(--color-text-muted)' }}
              title="Celebration Settings"
            >
              🎉
            </button>
            {showCelebrationSettings && (
              <div
                className="absolute right-0 top-full mt-2 w-48 rounded border p-3 shadow-lg z-50 text-sm space-y-3"
                style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
              >
                <div className="font-semibold mb-2">Trade Celebrations</div>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={settings.enabled}
                    onChange={e => setSettings(s => ({ ...s, enabled: e.target.checked }))}
                  />
                  <span>Enable Confetti</span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={settings.sound}
                    disabled={!settings.enabled}
                    onChange={e => setSettings(s => ({ ...s, sound: e.target.checked }))}
                    className="disabled:opacity-50"
                  />
                  <span className={!settings.enabled ? 'opacity-50' : ''}>Play Sound</span>
                </label>
              </div>
            )}
          </div>
          <button onClick={() => refetch()}
            className="p-2 rounded border hover:bg-white/5 h-[34px] flex items-center justify-center"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
            <RefreshCw size={13} className={isLoading ? 'animate-spin' : ''} />
          </button>
          <button onClick={() => setShowCreate(true)}
            className="flex items-center gap-2 px-3 h-[34px] rounded text-sm font-medium"
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
            { label: 'Total Trades', value: totalTradesDelta, icon: <TrendingUp size={14} /> },
            { label: 'Avg Win Rate', value: totalTradesDelta > 0 ? `${((totalWinsDelta / totalTradesDelta) * 100).toFixed(1)}%` : '—', icon: <TrendingDown size={14} /> },
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
            Start a strategy to run it on live or dry run mode
          </p>
          <button onClick={() => setShowCreate(true)}
            className="px-4 py-2 rounded text-sm font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
            <Plus size={13} className="inline mr-1" />
            New Strategy
          </button>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-4">
          {runners.map(runner => (
            <RunnerCard
              key={runner.config.id}
              runner={runner}
              onStop={() => stopMutation.mutate(runner.config.id)}
              onRestart={() => restartMutation.mutate(runner.config.id)}
              onDelete={() => setDeleteTarget(runner)}
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
      {deleteTarget && (
        <DeleteConfirmModal
          name={deleteTarget.config.name}
          onConfirm={() => {
            deleteMutation.mutate(deleteTarget.config.id)
            setDeleteTarget(null)
          }}
          onCancel={() => setDeleteTarget(null)}
        />
      )}
    </div>
  )
}
