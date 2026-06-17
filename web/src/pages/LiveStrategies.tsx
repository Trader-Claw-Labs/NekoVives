import { useState, useEffect, useRef } from 'react'
import { useLocation } from 'react-router-dom'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete, apiPatch } from '../hooks/useApi'
import { type MarketSeries, POLY_BINARY_PRESETS } from '../hooks/useBacktestState'
import PortfolioGuardWidget from '../components/PortfolioGuardWidget'
import CapitalAllocator from '../components/CapitalAllocator'
import EngineParamsForm, { defaultEngineParams } from '../components/EngineParamsForm'
import EngineKindInfoCard from '../components/EngineKindInfoCard'
import { ENGINE_KINDS, engineKindOptionLabel } from '../components/engineKindMeta'

const ENGINE_KIND_LABELS: Record<string, string> = {
  arb_binary: 'Arb Binary',
  fair_value: 'Fair Value',
  fv_momentum: 'FV + Momentum',
  rotation_compounder: 'Rotation Compounder',
  arb_hedge: 'Arb + Hedge Overlay',
  minting_mm: 'Minting MM',
  rhai_tick: 'Rhai Tick (1Hz on_tick)',
}

function strategyDisplayLabel(config: { kind?: string; script?: string }): string {
  const kind = config.kind ?? 'rhai_candle'
  if (kind !== 'rhai_candle' && ENGINE_KIND_LABELS[kind]) {
    return ENGINE_KIND_LABELS[kind]
  }
  return config.script?.split('/').pop() ?? kind
}
import { useProfitCelebration } from '../hooks/useProfitCelebration'
import {
  Bot, Plus, Trash2, RefreshCw, X, StopCircle, RotateCcw,
  TrendingUp, TrendingDown, Activity, ChevronDown, ChevronUp, AlertCircle, ExternalLink, Copy,
  Eye, EyeOff, Zap,
} from 'lucide-react'
import clsx from 'clsx'

// ── Types ─────────────────────────────────────────────────────────────

interface RunnerConfig {
  id: string
  name: string
  kind?: string
  script: string
  market_type: string
  symbol: string
  interval: string
  mode: string
  initial_balance: number
  fee_pct: number
  warmup_days: number
  series_id?: string
  polymarket_wallet_id?: string | null
  resolution_logic?: string
  threshold?: number | null
  live_sizing_mode?: 'fixed' | 'percent'
  live_sizing_value?: number
  stop_loss_pct?: number | null
  early_fire_secs?: number | null
  max_entry_price?: number | null
  max_spread_pct?: number | null
  max_slippage_pct?: number | null
  allowed_hours?: number[]
  rv_min_btc?: number | null
  // Guardrails
  kelly_size_cap?: number
  max_runner_loss_pct?: number | null
  max_consecutive_losses?: number | null
  min_entry_price?: number
}

interface PolyWalletProfile {
  id: string
  label: string
  configured: boolean
  wallet_address_masked?: string | null
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
  hidden?: boolean
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

/** Derive the wallet a runner trades on, mirroring the backend's
 *  /api/live/wallets-summary precedence so the UI groupings reconcile.
 *  Returns null for paper runners (no real wallet). */
function runnerWallet(r: StoredRunner): string | null {
  if (r.config.mode !== 'live') return null
  const w = r.result?.wallet_address || r.config.polymarket_wallet_id
  return w ? w.toLowerCase() : 'unknown'
}

function maskWallet(w: string): string {
  if (w === 'unknown') return 'unknown / legacy'
  if (w.length <= 12 || !w.startsWith('0x')) return w
  return `${w.slice(0, 6)}…${w.slice(-4)}`
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

// ── EngineMarketPicker ────────────────────────────────────────────────

interface PolyMarket {
  slug: string
  question: string
  volume: number
  liquidity: number
  end_date: string | null
  yes_price: number | null
  category: string | null
}

function daysUntil(iso: string | null): string {
  if (!iso) return '—'
  const diff = Math.round((new Date(iso).getTime() - Date.now()) / 86_400_000)
  if (diff <= 0) return 'expired'
  if (diff === 1) return '1 day'
  return `${diff} days`
}

function fmtVol(v: number): string {
  if (v >= 1_000_000) return `$${(v / 1_000_000).toFixed(1)}M`
  if (v >= 1_000) return `$${(v / 1_000).toFixed(0)}k`
  return `$${v.toFixed(0)}`
}

interface EngineMarketPickerProps {
  /// Array of fixed slugs (used when seriesId is empty).
  selected: string[]
  onChange: (slugs: string[]) => void
  /// Built-in recurring series (BTC 5m, ETH 5m, …) loaded from /api/backtest/series.
  series: MarketSeries[]
  /// Currently selected recurring-series id (empty = fixed-slug mode).
  seriesId: string
  onSeriesChange: (seriesId: string) => void
}

function EngineMarketPicker({
  selected,
  onChange,
  series,
  seriesId,
  onSeriesChange,
}: EngineMarketPickerProps) {
  const mode: 'series' | 'slugs' = seriesId ? 'series' : 'slugs'
  const [search, setSearch] = useState('')
  const [open, setOpen] = useState(false)
  const [sortMode, setSortMode] = useState<'volume' | 'liquidity'>('volume')
  const containerRef = useRef<HTMLDivElement>(null)

  // Gamma's `question_mid_partial` filter returns 0 results for single-letter
  // queries, so we only forward `q` once the user has typed >= 2 chars; for
  // shorter input we fetch the default crypto-tagged list instead.
  const effectiveQuery = search.trim().length >= 2 ? search.trim() : ''
  const { data, isFetching, error } = useQuery<{ markets: PolyMarket[] }>({
    queryKey: ['engine-markets', effectiveQuery, sortMode],
    queryFn: () => {
      // min_days=0 so short-lived markets (UP/DOWN 5m, daily temperature
      // markets, etc.) surface here too — engines run on whatever slug the
      // user picks, including ones closing today.
      const base = `/api/polymarket/markets?min_days=0&max_days=180&limit=80&sort=${sortMode}`
      return apiFetch(effectiveQuery ? `${base}&q=${encodeURIComponent(effectiveQuery)}` : `${base}&tag=crypto`)
    },
    staleTime: 5 * 60 * 1000,
    placeholderData: prev => prev,
    enabled: mode === 'slugs',
  })

  const markets = data?.markets ?? []

  // Close on outside click
  useEffect(() => {
    function handle(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false)
      }
    }
    document.addEventListener('mousedown', handle)
    return () => document.removeEventListener('mousedown', handle)
  }, [])

  function toggle(slug: string) {
    onChange(selected.includes(slug) ? selected.filter(s => s !== slug) : [...selected, slug])
  }

  function pickMode(m: 'series' | 'slugs') {
    if (m === 'series') {
      // Default to BTC 5m if available; clears fixed slug list.
      const first = series.find(s => s.id === 'btc_5m') ?? series[0]
      onSeriesChange(first?.id ?? 'btc_5m')
      onChange([])
      setOpen(false)
    } else {
      onSeriesChange('')
      // Reveal the default list immediately so the user sees there *are* markets
      // to pick — without this the dropdown only appears after typing.
      setOpen(true)
    }
  }

  return (
    <div ref={containerRef} className="relative">
      <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
        Market <span style={{ color: 'var(--color-danger)' }}>*</span>
      </label>

      {/* Mode toggle */}
      <div className="flex gap-1 mb-2 text-[11px]">
        <button
          type="button"
          onClick={() => pickMode('series')}
          className="px-2 py-1 rounded"
          style={{
            background: mode === 'series' ? 'var(--color-accent)' : 'var(--color-surface)',
            color: mode === 'series' ? '#000' : 'var(--color-text-muted)',
            border: '1px solid var(--color-border)',
          }}
        >
          Recurring series (BTC/ETH 5m…)
        </button>
        <button
          type="button"
          onClick={() => pickMode('slugs')}
          className="px-2 py-1 rounded"
          style={{
            background: mode === 'slugs' ? 'var(--color-accent)' : 'var(--color-surface)',
            color: mode === 'slugs' ? '#000' : 'var(--color-text-muted)',
            border: '1px solid var(--color-border)',
          }}
        >
          Fixed market slugs
        </button>
      </div>

      {mode === 'series' ? (
        <div>
          <select
            className="w-full rounded px-3 py-2 text-sm"
            value={seriesId}
            onChange={e => onSeriesChange(e.target.value)}
          >
            {series.length === 0 && <option value="">(loading…)</option>}
            {series.map(s => (
              <option key={s.id} value={s.id}>{s.label}</option>
            ))}
          </select>
          <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
            The runner auto-resolves the current window slug each poll
            (BTC 5m → <span className="font-mono">btc-updown-5m-&lt;timestamp&gt;</span>) — no need to paste a slug.
          </p>
        </div>
      ) : (
        <>
          {/* Selected chips */}
          {selected.length > 0 && (
            <div className="flex flex-wrap gap-1 mb-1.5">
              {selected.map(slug => (
                <span
                  key={slug}
                  className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-mono"
                  style={{ background: 'var(--color-accent)', color: '#000' }}
                >
                  {slug}
                  <button onClick={() => toggle(slug)} className="opacity-60 hover:opacity-100">×</button>
                </span>
              ))}
            </div>
          )}

          {/* Sort toggle */}
          <div className="flex items-center gap-2 mb-1 text-[10px]">
            <span style={{ color: 'var(--color-text-muted)' }}>Sort:</span>
            <button
              type="button"
              onClick={() => setSortMode('volume')}
              className="px-1.5 py-0.5 rounded"
              style={{
                background: sortMode === 'volume' ? 'var(--color-accent)' : 'transparent',
                color: sortMode === 'volume' ? '#000' : 'var(--color-text-muted)',
                border: '1px solid var(--color-border)',
              }}
            >volume</button>
            <button
              type="button"
              onClick={() => setSortMode('liquidity')}
              className="px-1.5 py-0.5 rounded"
              style={{
                background: sortMode === 'liquidity' ? 'var(--color-accent)' : 'transparent',
                color: sortMode === 'liquidity' ? '#000' : 'var(--color-text-muted)',
                border: '1px solid var(--color-border)',
              }}
            >top liquidity</button>
          </div>

          {/* Search input */}
          <div className="relative">
            <input
              className="w-full rounded px-3 py-2 text-sm pr-8"
              placeholder={selected.length ? 'Add another market…' : 'Search markets or paste slug…'}
              value={search}
              onChange={e => { setSearch(e.target.value); setOpen(true) }}
              onFocus={() => setOpen(true)}
              autoFocus
            />
            {isFetching && (
              <span className="absolute right-2 top-1/2 -translate-y-1/2 text-[10px]"
                style={{ color: 'var(--color-text-muted)' }}>…</span>
            )}
          </div>

          {/* Dropdown — always rendered while open so the user sees loading,
              empty and error states explicitly instead of staring at nothing. */}
          {open && (
            <div
              className="absolute z-50 w-full mt-1 rounded border overflow-auto max-h-56 shadow-lg"
              style={{ background: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
            >
              {error ? (
                <div className="px-3 py-2 text-xs" style={{ color: 'var(--color-danger)' }}>
                  Polymarket API error: {(error as Error).message}
                </div>
              ) : markets.length === 0 ? (
                <div className="px-3 py-2 text-xs" style={{ color: 'var(--color-text-muted)' }}>
                  {isFetching
                    ? 'Loading markets…'
                    : search.trim().length === 1
                      ? 'Type at least 2 characters to search.'
                      : 'No markets matched. Try a different search or switch sort to "top liquidity".'}
                </div>
              ) : (
                markets.map(m => {
                  const isSelected = selected.includes(m.slug)
                  return (
                    <button
                      key={m.slug}
                      onClick={() => { toggle(m.slug); setSearch('') }}
                      className="w-full text-left px-3 py-2 text-xs flex items-start justify-between gap-2 hover:bg-white/5"
                      style={isSelected ? { background: 'rgba(0,200,100,0.08)' } : undefined}
                    >
                      <span className="flex-1 min-w-0">
                        <span className="block truncate" style={{ color: 'var(--color-text)' }}>{m.question}</span>
                        <span className="font-mono text-[10px]" style={{ color: 'var(--color-text-muted)' }}>{m.slug}</span>
                      </span>
                      <span className="shrink-0 text-right text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                        <span className="block">{daysUntil(m.end_date)}</span>
                        <span className="block">
                          {sortMode === 'liquidity' ? `liq ${fmtVol(m.liquidity)}` : fmtVol(m.volume)}
                        </span>
                      </span>
                    </button>
                  )
                })
              )}
            </div>
          )}

          <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
            Active Polymarket markets. <b>Top liquidity</b> sorts by book depth and allows windows &lt; 1 day.
          </p>
        </>
      )}
    </div>
  )
}

export interface BacktestPrefill {
  kind?: string
  script?: string
  symbol?: string
  market_type?: string
  series_id?: string
  engine_params?: Record<string, unknown>
  mode?: 'paper' | 'live'
}

export interface CreateModalProps {
  scripts: BacktestScript[]
  onClose: () => void
  onCreated: () => void
  defaultScript?: string
  prefill?: BacktestPrefill
}

export function CreateModal({ scripts, onClose, onCreated, defaultScript, prefill }: CreateModalProps) {
  // Load market series for polymarket_binary picker
  const { data: seriesData } = useQuery<{ series: MarketSeries[] }>({
    queryKey: ['backtest-series'],
    queryFn: () => apiFetch('/api/backtest/series'),
    staleTime: 10 * 60 * 1000,
  })
  const allSeries: MarketSeries[] = seriesData?.series ?? []

  // Polymarket wallet profiles — let the user pick which wallet this runner trades on.
  const { data: walletData } = useQuery<{ wallets: PolyWalletProfile[] }>({
    queryKey: ['polymarket-wallets'],
    queryFn: () => apiFetch('/api/polymarket/wallets'),
    staleTime: 60 * 1000,
  })
  const walletProfiles: PolyWalletProfile[] = walletData?.wallets ?? []

  const [form, setForm] = useState({
    kind: (prefill?.kind ?? 'rhai_candle') as string,
    name: '',
    script: prefill?.script ?? defaultScript ?? scripts[0]?.path ?? '',
    market_type: prefill?.market_type ?? 'polymarket_binary',
    symbol: prefill?.symbol ?? 'BTCUSDT',
    interval: '5m',
    mode: prefill?.mode ?? 'paper',
    initial_balance: 1000,
    fee_pct: 1.5,
    warmup_days: 7,
    series_id: prefill?.series_id ?? 'btc_5m',
    polymarket_wallet_id: '' as string,
    poly_condition_id: '' as string,
    resolution_logic: 'price_up',
    threshold: null as number | null,
    live_sizing_mode: 'percent' as 'fixed' | 'percent',
    live_sizing_value: 5,
    stop_loss_pct: null as number | null,
    early_fire_secs: null as number | null,
    max_entry_price: 0.65 as number | null,
    max_spread_pct: 0.03 as number | null,
    max_slippage_pct: 10 as number | null,
    price_mode: 'historical' as string,
    allowed_hours: [] as number[],
    rv_min_btc: null as number | null,
    // ── Guardrails (risk controls) ──
    kelly_size_cap: 1.5 as number,
    max_runner_loss_pct: 0.30 as number | null,
    max_consecutive_losses: 8 as number | null,
    min_entry_price: 0.10 as number,
    wallet_password: '',
    binance_api_key: '',
    binance_api_secret: '',
    funding_watchlist: 'BTC,ETH,SOL,AVAX',
    min_apr_diff: 10,
    force_close_diff: 2,
    max_open_pairs: 4,
    max_pos_pct: 15,
    funding_poll_secs: 60,
    fee_buffer_bps: 12,
    force_live: false,  // override the validation gate (Rec-1) when going Live on a NO_EDGE strategy
    engine_params: (prefill?.engine_params ?? {}) as Record<string, unknown>,
  })

  const isRhaiTick = form.kind === 'rhai_tick'
  const isEngineKind = form.kind !== 'rhai_candle' && !isRhaiTick
  // rewards_maker quotes ONE fixed slow market (politics/macro), not a recurring
  // crypto series — it needs a condition_id, not the series picker.
  const isRewardsMaker = form.kind === 'rewards_maker'
  // rewards_orchestrator auto-selects markets at runtime — no market input at all.
  const isRewardsOrchestrator = form.kind === 'rewards_orchestrator'
  // Quick Start: hide the advanced risk/timing controls behind a disclosure so a
  // new user's first Dry Run only needs Engine + Script + Series + Mode.
  const [showAdvanced, setShowAdvanced] = useState(false)

  // Guard against the React controlled-<select> trap: when `form.script` is
  // '' (or doesn't match any rendered <option>), the browser shows the first
  // item but no `change` event fires unless the user manually picks a
  // different option. Auto-sync to the first visible option to keep the
  // form value in lock-step with what the user actually sees.
  useEffect(() => {
    if (isEngineKind) return
    const visible = scripts.filter(s => !isRhaiTick || s.path.includes('clob_1hz') || s.name.includes('clob_1hz'))
    if (visible.length === 0) return
    if (!form.script || !visible.some(s => s.path === form.script)) {
      setForm(f => ({ ...f, script: visible[0].path }))
    }
  }, [isEngineKind, isRhaiTick, scripts, form.script])

  const [error, setError] = useState('')
  const [showMissingApiKeyModal, setShowMissingApiKeyModal] = useState(false)
  const [showMissingPrivateKeyModal, setShowMissingPrivateKeyModal] = useState(false)
  const [balanceFetching, setBalanceFetching] = useState(false)
  const [balanceFetchError, setBalanceFetchError] = useState('')

  // When user switches to live mode, auto-fetch the real Polymarket wallet
  // balance and pre-populate initial_balance so sizing is based on actual funds.
  useEffect(() => {
    if (form.mode !== 'live') return
    setBalanceFetching(true)
    setBalanceFetchError('')
    apiFetch('/api/polymarket/balance')
      .then((data: unknown) => {
        const d = data as { balance?: number }
        if (typeof d.balance === 'number' && d.balance > 0) {
          setForm(f => ({ ...f, initial_balance: Math.floor(d.balance as number) }))
        }
      })
      .catch((e: Error) => setBalanceFetchError(e.message ?? 'Could not fetch wallet balance'))
      .finally(() => setBalanceFetching(false))
  }, [form.mode])

  // Tick recorder fields (only relevant for polymarket_binary)
  const [tickRecord, setTickRecord] = useState(false)
  const [tickConditionId, setTickConditionId] = useState('')
  const [tickDetecting, setTickDetecting] = useState(false)
  const [tickDetectError, setTickDetectError] = useState('')
  const slug = form.series_id ?? form.symbol?.toLowerCase().replace('usdt', '_5m') ?? ''

  function autoDetectConditionIdModal(seriesId: string) {
    if (!seriesId) return
    setTickDetecting(true)
    setTickDetectError('')
    apiFetch(`/api/polymarket/active-token?series_id=${encodeURIComponent(seriesId)}`)
      .then((data: any) => {
        if (data?.condition_id) {
          setTickConditionId(data.condition_id)
        } else {
          setTickDetectError('No active market found — enter manually')
        }
      })
      .catch(() => setTickDetectError('Detection failed — enter manually'))
      .finally(() => setTickDetecting(false))
  }

  function friendlyCreateError(message: string) {
    const m = message.toLowerCase()
    // Rec-1 validation gate: the strategy showed NO EDGE on its history. Offer the override.
    if (m.includes('validation gate') || (m.includes('no edge') && m.includes('blocked'))) {
      return `🛡 ${message}\n\nTo go Live anyway, check "Override validation gate" below and resubmit.`
    }
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
    if (m.includes('hyperliquid') || m.includes('wallet_label') || m.includes('signer')) {
      return 'Live CEX trading requires a Hyperliquid wallet. Go to Wallets → create an EVM wallet, then set hyperliquid.wallet_label in Settings → Config.'
    }
    if (m.includes('password') || m.includes('decrypt') || m.includes('private key')) {
      return 'Failed to decrypt wallet private key. Check your wallet password and try again.'
    }
    return message
  }

  const mutation = useMutation({
    mutationFn: () => {
      const payload: Record<string, unknown> = {
        ...form,
        funding_watchlist: form.funding_watchlist
          ? form.funding_watchlist.split(',').map((s: string) => s.trim().toUpperCase()).filter(Boolean)
          : undefined,
        min_apr_diff: form.min_apr_diff != null ? form.min_apr_diff / 100 : undefined,
        force_close_diff: form.force_close_diff != null ? form.force_close_diff / 100 : undefined,
        max_pos_pct: form.max_pos_pct != null ? form.max_pos_pct / 100 : undefined,
      }
      // Engine kinds (arb_binary, fair_value, …) ignore the Rhai script and
      // are driven directly by their typed config. Stripping `script` keeps
      // RunnerCard from displaying a misleading filename.
      if (form.kind && form.kind !== 'rhai_candle') {
        delete payload.script
      }
      // Remove undefined values so serde deserializes them as absent (triggering defaults)
      Object.keys(payload).forEach((k) => {
        if (payload[k] === undefined || payload[k] === '') delete payload[k]
      })
      return apiPost('/api/live/strategies', payload)
    },
    onSuccess: async () => {
      // Auto-start tick recorder if requested
      if (tickRecord && tickConditionId.trim() && form.market_type === 'polymarket_binary') {
        try {
          await apiPost('/api/tick-recorder/start', {
            slug: slug || form.series_id || 'market',
            condition_id: tickConditionId.trim(),
            binance_symbol: form.symbol || 'BTCUSDT',
          })
        } catch {
          // Non-fatal — recorder can be started manually later
        }
      }
      onCreated()
      onClose()
    },
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
    } else if (mt === 'funding_arb') {
      setForm(f => ({
        ...f,
        market_type: mt,
        symbol: 'FUNDING_ARB',
        interval: '1h',
        fee_pct: 0.0,
        series_id: '',
        resolution_logic: 'price_up',
        threshold: null,
      }))
    } else {
      setForm(f => ({
        ...f,
        market_type: mt,
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

  function onKindChange(newKind: string) {
    if (newKind === 'rhai_candle') {
      setForm(f => ({
        ...f,
        kind: newKind,
        symbol: 'BTCUSDT',
        script: defaultScript ?? scripts[0]?.path ?? '',
        engine_params: {},
      }))
    } else if (newKind === 'rhai_tick') {
      // Rhai Tick needs both a script (on_tick) and a series_id (for token resolution).
      const defaultSeries = allSeries.find(s => s.id === 'btc_5m') ?? allSeries[0]
      const tickScript = scripts.find(s => s.path.includes('clob_1hz'))?.path
        ?? scripts[0]?.path
        ?? ''
      setForm(f => ({
        ...f,
        kind: newKind,
        market_type: 'polymarket_binary',
        symbol: defaultSeries?.symbol ?? 'BTCUSDT',
        series_id: defaultSeries?.id ?? 'btc_5m',
        interval: defaultSeries?.cadence ?? '5m',
        script: tickScript,
        engine_params: {},
      }))
    } else if (newKind === 'rewards_maker' || newKind === 'rewards_orchestrator') {
      // rewards_maker quotes ONE fixed market by condition_id; rewards_orchestrator
      // auto-selects markets at runtime. Neither uses a recurring series/symbol slug.
      setForm(f => ({
        ...f,
        kind: newKind,
        market_type: 'polymarket_binary',
        mode: 'paper',
        symbol: '',
        series_id: '',
        script: '',
        engine_params: defaultEngineParams(newKind),
      }))
    } else {
      // Default to the BTC 5m recurring series so the engine picks up the
      // current window slug each poll. Without a series_id the engine would
      // try to resolve config.symbol ("BTCUSDT") as a Polymarket slug and
      // fail with "No active market with valid tokens for slug: BTCUSDT".
      const defaultSeries = allSeries.find(s => s.id === 'btc_5m') ?? allSeries[0]
      setForm(f => ({
        ...f,
        kind: newKind,
        market_type: 'polymarket_binary',
        mode: 'paper',
        symbol: defaultSeries?.symbol ?? '',
        series_id: defaultSeries?.id ?? 'btc_5m',
        interval: defaultSeries?.cadence ?? '5m',
        script: '',
        engine_params: defaultEngineParams(newKind),
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
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Strategy Engine</label>
            <select className="w-full rounded px-3 py-2 text-sm" value={form.kind}
              onChange={e => onKindChange(e.target.value)}>
              <option value="rhai_candle">Rhai Script (on_candle, default)</option>
              <option value="rhai_tick">Rhai Tick (1Hz on_tick) — runs CLOB scalpers second-by-second</option>
              {ENGINE_KINDS.map((e) => (
                <option key={e.id} value={e.id}>{engineKindOptionLabel(e.id)}</option>
              ))}
            </select>
            {isEngineKind && <EngineKindInfoCard kind={form.kind} />}
            {isEngineKind && (
              <p className="text-[10px] mt-1.5" style={{ color: 'var(--color-text-muted)' }}>
                Engine strategies run on Polymarket Binary markets. Start in <span className="font-semibold">Dry Run</span> first — you can promote to Live once you trust the simulated PnL.
              </p>
            )}
          </div>

          {/* Per-engine tunable parameters */}
          {isEngineKind && (
            <EngineParamsForm
              kind={form.kind}
              params={form.engine_params}
              onChange={(p) => setForm(f => ({ ...f, engine_params: p }))}
            />
          )}

          {!isEngineKind && (
            <div>
              {!isRhaiTick && (
                <>
                  <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Market Type</label>
                  <select className="w-full rounded px-3 py-2 text-sm" value={form.market_type}
                    onChange={e => onMarketTypeChange(e.target.value)}>
                    <option value="crypto">Crypto</option>
                    <option value="funding_arb">Funding Arb</option>
                    <option value="polymarket_binary">Polymarket Binary</option>
                  </select>
                </>
              )}
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Strategy Script</label>
              <select className="w-full rounded px-3 py-2 text-sm font-mono" value={form.script}
                onChange={e => set('script', e.target.value)}>
                {scripts
                  .filter(s => !isRhaiTick || s.path.includes('clob_1hz') || s.name.includes('clob_1hz'))
                  .map(s => (
                    <option key={s.path} value={s.path}>
                      {s.name} {s.last_run_stats ? `(${(s.last_run_stats.win_rate_pct ?? 0).toFixed(1)}% WR)` : ''}
                    </option>
                  ))}
              </select>
              {/* Show the selected script's description so the user knows what it does. */}
              {(() => {
                const sel = scripts.find(s => s.path === form.script)
                return sel?.description ? (
                  <p className="text-[11px] mt-1.5 px-2 py-1.5 rounded" style={{ background: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}>
                    {sel.description}
                  </p>
                ) : null
              })()}
              {isRhaiTick && (
                <p className="text-[10px] mt-1" style={{ color: 'var(--color-text-muted)' }}>
                  Only <span className="font-mono">clob_1hz_*</span> scripts are listed (they implement <span className="font-mono">on_tick(ctx)</span> for 1Hz CLOB execution).
                </p>
              )}
            </div>
          )}

          {isEngineKind && (
            <div className="space-y-3">
              {isRewardsOrchestrator ? (
                <div className="rounded px-3 py-2 text-[11px]" style={{ background: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}>
                  This engine <span className="font-semibold">auto-selects</span> the top reward markets at runtime
                  (set the pool size + min-safety in Engine Parameters below). No market to pick —
                  just assign capital + wallet and start. It closes and rotates out of any market that turns toxic.
                </div>
              ) : isRewardsMaker ? (
                <div>
                  <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Market condition_id</label>
                  <input
                    type="text"
                    className="w-full rounded px-3 py-2 text-sm font-mono"
                    value={form.poly_condition_id}
                    onChange={e => set('poly_condition_id', e.target.value.trim())}
                    placeholder="0x… (a SLOW market — politics / macro / far-dated)"
                  />
                  <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                    Quotes ONE fixed market. Pick a high-safety reward market on the
                    <span className="font-mono"> /rewards </span> page (NEVER crypto 5m/15m — toxic).
                    The engine resolves YES/NO tokens from this condition_id.
                  </p>
                </div>
              ) : (
                <>
                  <EngineMarketPicker
                    selected={form.symbol ? form.symbol.split(',').map(s => s.trim()).filter(Boolean) : []}
                    onChange={slugs => set('symbol', slugs.join(','))}
                    series={allSeries}
                    seriesId={form.series_id}
                    onSeriesChange={(sid) => {
                      // When a recurring series is chosen, drop any pasted fixed
                      // slugs and let the runner resolve the current window's
                      // slug automatically each poll (see series_helper.rs).
                      setForm(f => ({ ...f, series_id: sid, symbol: sid ? '' : f.symbol }))
                    }}
                  />
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Threshold / Edge</label>
                    <input
                      type="number"
                      step="0.001"
                      className="w-full rounded px-3 py-2 text-sm"
                      value={form.threshold ?? ''}
                      onChange={e => set('threshold', e.target.value === '' ? null : Number(e.target.value))}
                      placeholder="0.03"
                    />
                    <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>Min edge / arb spread (engine default if blank)</p>
                  </div>
                </>
              )}
              {/* Wallet profile — engine runners also trade Polymarket */}
              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Wallet</label>
                <select
                  className="w-full rounded px-3 py-2 text-sm"
                  value={form.polymarket_wallet_id}
                  onChange={e => set('polymarket_wallet_id', e.target.value)}
                >
                  <option value="">Default wallet</option>
                  {walletProfiles
                    .filter(w => w.id !== 'default')
                    .map(w => (
                      <option key={w.id} value={w.id} disabled={form.mode === 'live' && !w.configured}>
                        {w.label}{w.wallet_address_masked ? ` · ${w.wallet_address_masked}` : ''}{!w.configured ? ' (incomplete)' : ''}
                      </option>
                    ))}
                </select>
                <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                  Manage wallets on the Polymarket page → Wallet Profiles.
                </p>
              </div>
            </div>
          )}

          <div className={`grid gap-3 ${isEngineKind || isRhaiTick ? 'grid-cols-1' : 'grid-cols-2'}`}>
            {!isEngineKind && !isRhaiTick && (
              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Market Type</label>
                <select className="w-full rounded px-3 py-2 text-sm" value={form.market_type}
                  onChange={e => onMarketTypeChange(e.target.value)}>
                  <option value="crypto">Crypto</option>
                  <option value="polymarket_binary">Polymarket Binary</option>
                </select>
              </div>
            )}
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Mode</label>
              <SegmentedToggle
                value={form.mode === 'live'}
                onChange={(v) => set('mode', v ? 'live' : 'paper')}
                leftLabel="Dry Run"
                rightLabel="Live"
                activeColor={form.mode === 'live' ? 'var(--color-warning)' : 'var(--color-accent)'}
                disabled={!isEngineKind && !isRhaiTick && form.market_type !== 'polymarket_binary'}
              />
              {/* Validation-gate override — only relevant in Live mode */}
              {form.mode === 'live' && (
                <label className="flex items-center gap-1.5 mt-1.5 text-[10px] cursor-pointer" style={{ color: 'var(--color-text-muted)' }}>
                  <input type="checkbox" checked={form.force_live}
                    onChange={e => set('force_live', e.target.checked)} />
                  Override validation gate (go Live even if NO_EDGE)
                </label>
              )}
            </div>
          </div>

          {!isEngineKind && form.market_type === 'polymarket_binary' ? (
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

              {/* Wallet profile — which Polymarket wallet this runner trades on */}
              <div className="mt-3">
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Wallet</label>
                <select
                  className="w-full rounded px-3 py-2 text-sm"
                  value={form.polymarket_wallet_id}
                  onChange={e => set('polymarket_wallet_id', e.target.value)}
                >
                  <option value="">Default wallet</option>
                  {walletProfiles
                    .filter(w => w.id !== 'default')
                    .map(w => (
                      <option key={w.id} value={w.id} disabled={form.mode === 'live' && !w.configured}>
                        {w.label}{w.wallet_address_masked ? ` · ${w.wallet_address_masked}` : ''}{!w.configured ? ' (incomplete)' : ''}
                      </option>
                    ))}
                </select>
                <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                  {form.mode === 'live'
                    ? 'Live orders are signed with this wallet’s credentials.'
                    : 'Manage wallets on the Polymarket page → Wallet Profiles.'}
                </p>
              </div>
            </div>
          ) : !isEngineKind && form.market_type === 'funding_arb' ? (
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Watchlist</label>
              <input className="w-full rounded px-3 py-2 text-sm font-mono" value={form.funding_watchlist}
                onChange={e => set('funding_watchlist', e.target.value)}
                placeholder="BTC,ETH,SOL,AVAX" />
              <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                Comma-separated list of coins to monitor for funding rate divergences
              </p>
            </div>
          ) : null}

          {!isEngineKind && form.market_type !== 'polymarket_binary' && (
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

          {/* Balance / warmup row — shown in both modes.
              In live mode the balance is auto-fetched from the real wallet. */}
          <div className="grid grid-cols-3 gap-3">
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
                {form.mode === 'live' ? 'Wallet Balance ($)' : 'Initial Balance ($)'}
                {balanceFetching && (
                  <span className="ml-1 text-xs opacity-60">fetching…</span>
                )}
              </label>
              <input
                type="number"
                min={1}
                className="w-full rounded px-3 py-2 text-sm"
                value={form.initial_balance}
                readOnly={form.mode === 'live' && balanceFetching}
                onChange={e => set('initial_balance', Number(e.target.value))}
              />
              {form.initial_balance <= 0 && (
                <p className="text-xs mt-1" style={{ color: 'var(--color-danger)' }}>Initial balance must be greater than 0.</p>
              )}
              {form.mode === 'live' && balanceFetchError && (
                <p className="text-xs mt-1" style={{ color: 'var(--color-danger)' }}>{balanceFetchError}</p>
              )}
              {form.mode === 'live' && !balanceFetching && !balanceFetchError && form.initial_balance > 0 && (
                <p className="text-xs mt-1 opacity-60">Live wallet balance — used for order sizing</p>
              )}
            </div>
            {form.mode === 'paper' && (
              <>
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
              </>
            )}
          </div>

          {/* Live sizing config — shown for all market types */}
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
                  ? 'Fixed USD amount per order (min $5)'
                  : 'Script fraction is capped at this %'}
              </p>
            </div>

            {/* Wallet password for crypto/funding_arb live mode */}
            {form.mode === 'live' && (form.market_type === 'crypto' || form.market_type === 'funding_arb') && (
              <div className="col-span-2">
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
                  Wallet Password
                </label>
                <input
                  type="password"
                  className="w-full rounded px-3 py-2 text-sm"
                  placeholder="Enter password to decrypt Hyperliquid wallet"
                  value={form.wallet_password}
                  onChange={e => set('wallet_password', e.target.value)}
                />
                <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                  Required to decrypt your EVM wallet private key for Hyperliquid signing. The password is never stored.
                </p>
              </div>
            )}

            {/* Binance API credentials for crypto/funding_arb live mode */}
            {form.mode === 'live' && (form.market_type === 'crypto' || form.market_type === 'funding_arb') && (
              <>
                <div>
                  <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
                    Binance API Key
                  </label>
                  <input
                    type="password"
                    className="w-full rounded px-3 py-2 text-sm font-mono"
                    placeholder={form.market_type === 'funding_arb' ? 'Required for funding arb' : 'Optional — uses Binance instead of Hyperliquid'}
                    value={form.binance_api_key}
                    onChange={e => set('binance_api_key', e.target.value)}
                  />
                </div>
                <div>
                  <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
                    Binance API Secret
                  </label>
                  <input
                    type="password"
                    className="w-full rounded px-3 py-2 text-sm font-mono"
                    placeholder={form.market_type === 'funding_arb' ? 'Required for funding arb' : 'Required with API key for Binance'}
                    value={form.binance_api_secret}
                    onChange={e => set('binance_api_secret', e.target.value)}
                  />
                </div>
                <p className="col-span-2 text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                  {form.market_type === 'funding_arb'
                    ? 'Both Hyperliquid wallet AND Binance credentials are required for funding rate arbitrage.'
                    : 'Leave empty to use Hyperliquid (wallet password required). Fill both to trade on Binance Futures instead.'}
                </p>
              </>
            )}

            {/* Funding arb configuration fields */}
            {form.market_type === 'funding_arb' && (
              <>
                <div className="grid grid-cols-3 gap-3">
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Min APR Diff (%)</label>
                    <input type="number" step="0.5" min="0.5" max="50"
                      className="w-full rounded px-3 py-2 text-sm"
                      value={form.min_apr_diff}
                      onChange={e => set('min_apr_diff', Number(e.target.value))} />
                  </div>
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Close Below (%)</label>
                    <input type="number" step="0.5" min="0.5" max="20"
                      className="w-full rounded px-3 py-2 text-sm"
                      value={form.force_close_diff}
                      onChange={e => set('force_close_diff', Number(e.target.value))} />
                  </div>
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Max Pairs</label>
                    <input type="number" step="1" min="1" max="10"
                      className="w-full rounded px-3 py-2 text-sm"
                      value={form.max_open_pairs}
                      onChange={e => set('max_open_pairs', Number(e.target.value))} />
                  </div>
                </div>
                <div className="grid grid-cols-3 gap-3">
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Max Position %</label>
                    <input type="number" step="1" min="1" max="50"
                      className="w-full rounded px-3 py-2 text-sm"
                      value={form.max_pos_pct}
                      onChange={e => set('max_pos_pct', Number(e.target.value))} />
                  </div>
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Poll Interval (s)</label>
                    <input type="number" step="10" min="10" max="300"
                      className="w-full rounded px-3 py-2 text-sm"
                      value={form.funding_poll_secs}
                      onChange={e => set('funding_poll_secs', Number(e.target.value))} />
                  </div>
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Fee Buffer (bps)</label>
                    <input type="number" step="1" min="1" max="50"
                      className="w-full rounded px-3 py-2 text-sm"
                      value={form.fee_buffer_bps}
                      onChange={e => set('fee_buffer_bps', Number(e.target.value))} />
                  </div>
                </div>
              </>
            )}

            {/* Polymarket-specific fields */}
            {form.market_type === 'polymarket_binary' && (
              <>
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
                  <SegmentedToggle
                    value={form.stop_loss_pct != null}
                    onChange={(v) => set('stop_loss_pct', v ? 0.40 : null)}
                    leftLabel="Off"
                    rightLabel="On"
                    activeColor="var(--color-danger)"
                  />
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
                  <SegmentedToggle
                    value={form.early_fire_secs != null}
                    onChange={(v) => set('early_fire_secs', v ? 10 : null)}
                    leftLabel="Off"
                    rightLabel="On"
                    activeColor="#818cf8"
                  />
                </div>
                <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                  {form.early_fire_secs != null && form.early_fire_secs > 0
                    ? `Order placed ${form.early_fire_secs}s before candle close — avoids bot-crowding at minute boundary`
                    : 'Disabled — order placed at candle close'}
                </p>
              </div>
            </>
          )}
          </div>

          {/* Max Entry Price */}
          <div>
            <label className="block text-[11px] font-medium mb-1" style={{ color: 'var(--color-text-muted)' }}>
              Max Entry Price
            </label>
            <div className="flex items-center gap-2">
              <input
                type="number"
                className="w-20 rounded border px-2 py-1 text-xs"
                style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                placeholder="0.65"
                min={0.01}
                max={0.99}
                step={0.01}
                value={form.max_entry_price != null ? form.max_entry_price : ''}
                onChange={e => set('max_entry_price', e.target.value === '' ? null : Number(e.target.value))}
              />
              <SegmentedToggle
                value={form.max_entry_price != null}
                onChange={(v) => set('max_entry_price', v ? 0.65 : null)}
                leftLabel="Off"
                rightLabel="On"
                activeColor="#818cf8"
              />
            </div>
            <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
              {form.max_entry_price != null
                ? `Skip trades when token price exceeds $${form.max_entry_price.toFixed(2)} — protects against overpriced entries`
                : 'Disabled — trades at any token price'}
            </p>
          </div>

          {/* Max Spread (Polymarket Binary only) */}
          {form.market_type === 'polymarket_binary' && (
            <div>
              <label className="block text-[11px] font-medium mb-1" style={{ color: 'var(--color-text-muted)' }}>
                Max Spread
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  className="w-20 rounded border px-2 py-1 text-xs"
                  style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                  placeholder="3.00"
                  min={0.01}
                  max={50}
                  step={0.01}
                  value={form.max_spread_pct != null ? (form.max_spread_pct * 100).toFixed(2) : ''}
                  onChange={e => set('max_spread_pct', e.target.value === '' ? null : Number(e.target.value) / 100)}
                />
                <span className="text-[11px]" style={{ color: 'var(--color-text-muted)' }}>%</span>
                <SegmentedToggle
                  value={form.max_spread_pct != null}
                  onChange={(v) => set('max_spread_pct', v ? 0.03 : null)}
                  leftLabel="Off"
                  rightLabel="On"
                  activeColor="#818cf8"
                />
              </div>
              <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                {form.max_spread_pct != null
                  ? `Skip windows when yes+no mids deviate >${(form.max_spread_pct * 100).toFixed(2)}% from 1.0 (type percentage: e.g. "2" = 2%) — avoids paper-fill optimism in wide books`
                  : 'Disabled — trades regardless of spread width'}
              </p>
            </div>
          )}

          {/* Price Mode — how entry price is recorded for P&L */}
          {form.market_type === 'polymarket_binary' && (
            <div>
              <label className="block text-[11px] font-medium mb-1" style={{ color: 'var(--color-text-muted)' }}>
                Price Mode
              </label>
              <select
                className="w-full rounded border px-2 py-1 text-xs"
                style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                value={form.price_mode}
                onChange={e => set('price_mode', e.target.value)}
              >
                <option value="historical">Historical — real CLOB ask price (recommended)</option>
                <option value="mid">Mid-price — (bid+ask)/2, more optimistic</option>
              </select>
              <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                {form.price_mode === 'mid'
                  ? 'Mid-price is cheaper than what you\'d actually pay — use only when comparing to BT calibrated on mid'
                  : 'Historical uses the real CLOB ask price, matching actual live execution cost'}
              </p>
            </div>
          )}

          {/* ── Advanced settings disclosure (guardrails, hour gate, RV floor, tick recorder) ── */}
          <button type="button" onClick={() => setShowAdvanced(v => !v)}
            className="flex items-center gap-1.5 text-xs font-medium mt-1"
            style={{ color: 'var(--color-text-muted)' }}>
            {showAdvanced ? '▾' : '▸'} Advanced settings (guardrails, hour gate, tick recorder)
          </button>
          <div style={{ display: showAdvanced ? 'block' : 'none' }} className="space-y-3">

          {/* Slippage Cap (live mode only — controls worst_price on market orders) */}
          {form.market_type === 'polymarket_binary' && form.mode === 'live' && (
            <div>
              <label className="block text-[11px] font-medium mb-1" style={{ color: 'var(--color-text-muted)' }}>
                Max Slippage (live orders)
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  className="w-20 rounded border px-2 py-1 text-xs"
                  style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                  placeholder="10"
                  min={1}
                  max={50}
                  step={1}
                  value={form.max_slippage_pct != null ? form.max_slippage_pct : ''}
                  onChange={e => set('max_slippage_pct', e.target.value === '' ? null : Number(e.target.value))}
                />
                <span className="text-[11px]" style={{ color: 'var(--color-text-muted)' }}>%</span>
              </div>
              <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                {form.max_slippage_pct != null
                  ? `Market orders rejected if fill price > mid × ${(1 + form.max_slippage_pct / 100).toFixed(2)}× — retries up to 3× then skips`
                  : 'Using default 10%'}
              </p>
            </div>
          )}

          {/* ── Guardrails (risk controls — Polymarket Binary) ───────────── */}
          {form.market_type === 'polymarket_binary' && (
            <div className="rounded border p-3" style={{ borderColor: 'var(--color-warning)', backgroundColor: 'rgba(245,158,11,0.05)' }}>
              <div className="text-[11px] font-semibold mb-2" style={{ color: 'var(--color-warning)' }}>
                🛡 Guardrails (risk controls)
              </div>
              <div className="grid grid-cols-2 gap-3">
                {/* Min Entry ¢ */}
                <div>
                  <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Min Entry (¢)</label>
                  <input
                    type="number" min={1} max={50} step={1} placeholder="10"
                    className="w-full rounded border px-2 py-1 text-xs"
                    style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                    value={form.min_entry_price != null ? Math.round(form.min_entry_price * 100) : ''}
                    onChange={e => { const v = Number(e.target.value); if (!Number.isNaN(v) && v > 0) set('min_entry_price', v / 100) }}
                  />
                  <p className="text-[9px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>Skip bets &lt; this — blocks long-shots</p>
                </div>
                {/* Max Loss % */}
                <div>
                  <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Max Loss (%)</label>
                  <div className="flex items-center gap-1.5">
                    <input
                      type="number" min={5} max={100} step={5} placeholder="off"
                      className="w-full rounded border px-2 py-1 text-xs"
                      style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                      value={form.max_runner_loss_pct != null ? Math.round(form.max_runner_loss_pct * 100) : ''}
                      onChange={e => { const raw = e.target.value; if (raw === '') set('max_runner_loss_pct', null); else { const v = Number(raw); if (!Number.isNaN(v)) set('max_runner_loss_pct', v / 100) } }}
                    />
                    <SegmentedToggle
                      value={form.max_runner_loss_pct != null}
                      onChange={v => set('max_runner_loss_pct', v ? 0.30 : null)}
                      leftLabel="Off" rightLabel="On" activeColor="#f87171"
                    />
                  </div>
                  <p className="text-[9px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>Auto-stop + switch to paper</p>
                </div>
                {/* Max Consecutive Losses */}
                <div>
                  <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Max Loss Streak</label>
                  <div className="flex items-center gap-1.5">
                    <input
                      type="number" min={3} max={50} step={1} placeholder="off"
                      className="w-full rounded border px-2 py-1 text-xs"
                      style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                      value={form.max_consecutive_losses != null ? form.max_consecutive_losses : ''}
                      onChange={e => { const raw = e.target.value; if (raw === '') set('max_consecutive_losses', null); else { const v = parseInt(raw, 10); if (!Number.isNaN(v)) set('max_consecutive_losses', v) } }}
                    />
                    <SegmentedToggle
                      value={form.max_consecutive_losses != null}
                      onChange={v => set('max_consecutive_losses', v ? 8 : null)}
                      leftLabel="Off" rightLabel="On" activeColor="#f87171"
                    />
                  </div>
                  <p className="text-[9px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>Auto-stop after N losses</p>
                </div>
                {/* Kelly Cap */}
                <div>
                  <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Kelly Cap (×)</label>
                  <input
                    type="number" min={1.0} max={3.0} step={0.1} placeholder="1.5"
                    className="w-full rounded border px-2 py-1 text-xs"
                    style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                    value={form.kelly_size_cap ?? 1.5}
                    onChange={e => { const v = Number(e.target.value); if (!Number.isNaN(v)) set('kelly_size_cap', v) }}
                  />
                  <p className="text-[9px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>Caps script kelly_size mult</p>
                </div>
              </div>
            </div>
          )}

          {/* Hour Gate (Polymarket Binary only) */}
          {form.market_type === 'polymarket_binary' && (
            <div>
              <label className="block text-[11px] font-medium mb-1 flex items-center gap-1.5" style={{ color: 'var(--color-text-muted)' }}>
                Hour Gate (UTC)
                <span
                  className="px-1 rounded text-[9px]"
                  style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}
                  title="Skip windows outside these UTC hours. Empirically, certain hours show stronger drift signal."
                >hot hours only</span>
              </label>
              <div className="flex items-center gap-2 mb-1.5">
                <SegmentedToggle
                  value={form.allowed_hours.length > 0}
                  onChange={(v) => set('allowed_hours', v ? [0, 1, 6, 18, 21, 23] : [])}
                  leftLabel="Off"
                  rightLabel="On"
                  activeColor="#34d399"
                />
                <span className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                  {form.allowed_hours.length > 0 ? 'Hot hours only' : 'No restriction (24/7)'}
                </span>
              </div>
              {form.allowed_hours.length > 0 && (
                <>
                  <div className="flex flex-wrap gap-1 mb-1.5">
                    {Array.from({ length: 24 }, (_, h) => {
                      const active = form.allowed_hours.includes(h)
                      return (
                        <button
                          key={h}
                          type="button"
                          onClick={() => {
                            const next = active
                              ? form.allowed_hours.filter(x => x !== h)
                              : [...form.allowed_hours, h].sort((a, b) => a - b)
                            set('allowed_hours', next)
                          }}
                          className="w-7 h-6 rounded text-[10px] font-mono transition-colors"
                          style={{
                            background: active ? '#059669' : 'var(--color-surface-2)',
                            color: active ? '#fff' : 'var(--color-text-muted)',
                            border: `1px solid ${active ? '#059669' : 'var(--color-border)'}`,
                          }}
                        >{String(h).padStart(2, '0')}</button>
                      )
                    })}
                  </div>
                  <div className="flex gap-2 text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                    <button
                      type="button"
                      className="underline"
                      onClick={() => set('allowed_hours', [0, 1, 6, 18, 21, 23])}
                    >Preset: hot hours</button>
                    <button
                      type="button"
                      className="underline"
                      onClick={() => set('allowed_hours', [])}
                    >Clear</button>
                    <span>Active: {form.allowed_hours.join(', ')} UTC</span>
                  </div>
                </>
              )}
            </div>
          )}

          {/* RV Floor (Polymarket Binary only) */}
          {form.market_type === 'polymarket_binary' && (
            <div>
              <label className="block text-[11px] font-medium mb-1 flex items-center gap-1.5" style={{ color: 'var(--color-text-muted)' }}>
                BTC RV Floor
                <span
                  className="px-1 rounded text-[9px]"
                  style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}
                  title="Skip windows when BTC 60-period realized vol is below this value. Flat markets degrade drift signal."
                >flat-mkt filter</span>
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  className="w-28 rounded border px-2 py-1 text-xs font-mono"
                  style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                  min={0}
                  step={0.000005}
                  placeholder="0.00015"
                  value={form.rv_min_btc != null ? form.rv_min_btc : ''}
                  onChange={e => set('rv_min_btc', e.target.value === '' ? null : Number(e.target.value))}
                />
                <SegmentedToggle
                  value={form.rv_min_btc != null && form.rv_min_btc > 0}
                  onChange={(v) => set('rv_min_btc', v ? 0.00015 : null)}
                  leftLabel="Off"
                  rightLabel="On"
                  activeColor="#34d399"
                />
              </div>
              <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                {form.rv_min_btc != null && form.rv_min_btc > 0
                  ? `Skip when BTC 1h RV < ${form.rv_min_btc.toFixed(5)} — filters flat consolidation`
                  : 'Disabled — no RV filter applied'}
              </p>
            </div>
          )}

          {/* Tick Recorder — shown for Polymarket Binary only */}
          {form.market_type === 'polymarket_binary' && (
            <div
              className="rounded border px-3 py-2.5 space-y-2"
              style={{ borderColor: tickRecord ? 'var(--color-accent)' : 'var(--color-border)', backgroundColor: 'var(--color-surface-2)' }}
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Activity size={13} style={{ color: tickRecord ? 'var(--color-accent)' : 'var(--color-text-muted)' }} />
                  <span className="text-xs font-medium" style={{ color: 'var(--color-text)' }}>
                    Record CLOB ticks (1 Hz)
                  </span>
                  {tickRecord && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded font-semibold animate-pulse"
                      style={{ backgroundColor: 'rgba(34,197,94,0.15)', color: 'var(--color-accent)' }}>
                      WILL RECORD
                    </span>
                  )}
                </div>
                <button
                  type="button"
                  onClick={() => {
                    const next = !tickRecord
                    setTickRecord(next)
                    if (next && form.series_id && !tickConditionId) {
                      autoDetectConditionIdModal(form.series_id)
                    }
                  }}
                  className={`relative w-9 h-5 rounded-full transition-colors ${tickRecord ? 'bg-[var(--color-accent)]' : 'bg-[var(--color-border)]'}`}
                >
                  <span className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${tickRecord ? 'translate-x-4' : 'translate-x-0'}`} />
                </button>
              </div>
              {tickRecord && (
                <div className="space-y-1.5">
                  <div>
                    <div className="flex items-center justify-between mb-0.5">
                      <label className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                        Condition ID (YES token hex) <span style={{ color: 'var(--color-danger)' }}>*</span>
                      </label>
                      {form.series_id && (
                        <button
                          type="button"
                          onClick={() => autoDetectConditionIdModal(form.series_id)}
                          disabled={tickDetecting}
                          className="text-[10px] px-1.5 py-0.5 rounded disabled:opacity-50"
                          style={{ color: 'var(--color-accent)', backgroundColor: 'rgba(34,197,94,0.1)' }}
                        >
                          {tickDetecting ? '⏳ Detecting…' : '⚡ Auto-detect'}
                        </button>
                      )}
                    </div>
                    <input
                      className="w-full rounded px-2 py-1.5 text-xs font-mono"
                      style={{
                        backgroundColor: 'var(--color-surface)',
                        border: `1px solid ${tickDetecting ? 'var(--color-accent)' : 'var(--color-border)'}`,
                        color: 'var(--color-text)',
                        opacity: tickDetecting ? 0.6 : 1,
                      }}
                      placeholder={tickDetecting ? 'Detecting from Polymarket API…' : '0x1234abcd...'}
                      value={tickConditionId}
                      onChange={e => setTickConditionId(e.target.value)}
                      disabled={tickDetecting}
                    />
                    {tickDetectError && (
                      <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-warning)' }}>{tickDetectError}</p>
                    )}
                  </div>
                  <p className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                    {form.series_id
                      ? <>Auto-detect resolves the current window from Polymarket. Saves YES/NO bid-ask every second to{' '}<code style={{ color: 'var(--color-accent)' }}>data/ticks/{slug}/</code>.</>
                      : <>Saves YES/NO bid-ask + Binance price every second to{' '}<code style={{ color: 'var(--color-accent)' }}>data/ticks/{slug}/</code>. Find the condition_id on the Polymarket market page.</>
                    }
                  </p>
                </div>
              )}
              {!tickRecord && (
                <p className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                  Enable to record live CLOB prices at 1 Hz — required for CLOB 1 Hz backtesting.
                </p>
              )}
            </div>
          )}

          </div>{/* ── end Advanced settings disclosure ── */}

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
              {form.market_type !== 'polymarket_binary' && form.market_type !== 'funding_arb' && (
                <p className="font-medium">Live mode is only supported for Polymarket Binary and Funding Arbitrage markets.</p>
              )}
            </div>
          )}

          {error && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{error}</p>}
        </div>

        <div className="p-4 border-t flex gap-2" style={{ borderColor: 'var(--color-border)' }}>
          {(() => {
            // For engine strategies the user can pick EITHER a fixed slug list
            // (form.symbol) OR a recurring series (form.series_id) — the
            // runner resolves the slug per-window in the latter case.  The
            // legacy Rhai path still requires `form.script` + `form.symbol`.
            const hasMarketTarget = isRewardsOrchestrator
              ? true // auto-selects markets at runtime — no market input needed
              : isRewardsMaker
              ? Boolean(form.poly_condition_id)
              : isEngineKind
              ? Boolean(form.symbol || form.series_id)
              : Boolean(form.symbol)
            const needsScript = !isEngineKind && !form.script
            const liveMarketSupported =
              form.mode !== 'live' ||
              form.market_type === 'polymarket_binary' ||
              form.market_type === 'funding_arb'
            const disabled =
              needsScript ||
              !hasMarketTarget ||
              mutation.isPending ||
              !liveMarketSupported ||
              form.initial_balance <= 0
            return (
              <button
                onClick={() => mutation.mutate()}
                disabled={disabled}
                className="flex-1 py-2 rounded text-sm font-medium disabled:opacity-50"
                style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
              >
                {mutation.isPending ? 'Starting...' : form.mode === 'live' ? 'Start Live Strategy' : 'Start Dry Run'}
              </button>
            )
          })()}
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
              Send USDC.e or pUSD (Polygon) to your wallet:
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

// ── Segmented Toggle (two-option switch) ──────────────────────────────

function SegmentedToggle({
  value,
  onChange,
  leftLabel,
  rightLabel,
  activeColor = 'var(--color-accent)',
  disabled = false,
}: {
  value: boolean
  onChange: (v: boolean) => void
  leftLabel: string
  rightLabel: string
  activeColor?: string
  disabled?: boolean
}) {
  return (
    <div
      className="relative inline-flex h-8 rounded-full border p-0.5 transition-colors"
      style={{
        borderColor: 'var(--color-border)',
        backgroundColor: 'var(--color-surface-2)',
        opacity: disabled ? 0.5 : 1,
        cursor: disabled ? 'not-allowed' : 'pointer',
      }}
      onClick={() => !disabled && onChange(!value)}
    >
      {/* Sliding pill background */}
      <div
        className="absolute top-0.5 h-7 rounded-full transition-all duration-200"
        style={{
          width: 'calc(50% - 2px)',
          left: value ? 'calc(50%)' : '2px',
          backgroundColor: activeColor,
        }}
      />
      <span
        className="relative z-10 flex-1 px-3 text-xs font-semibold flex items-center justify-center select-none transition-colors duration-200"
        style={{ color: !value ? '#000' : 'var(--color-text-muted)' }}
      >
        {leftLabel}
      </span>
      <span
        className="relative z-10 flex-1 px-3 text-xs font-semibold flex items-center justify-center select-none transition-colors duration-200"
        style={{ color: value ? '#000' : 'var(--color-text-muted)' }}
      >
        {rightLabel}
      </span>
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

function formatUptime(startedAt?: string): string {
  if (!startedAt) return ''
  const start = new Date(startedAt).getTime()
  if (Number.isNaN(start)) return ''
  const elapsed = Math.max(0, Date.now() - start)
  const totalMin = Math.floor(elapsed / 60_000)
  const days = Math.floor(totalMin / 1440)
  const hours = Math.floor((totalMin % 1440) / 60)
  const mins = totalMin % 60
  if (days > 0) return `${days}d ${hours}h ${mins}m`
  if (hours > 0) return `${hours}h ${mins}m`
  return `${mins}m`
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
  onToggleHidden: () => void
  onUpdateConfig?: (updates: { live_sizing_mode?: string; live_sizing_value?: number; max_entry_price?: number | null; max_spread_pct?: number | null; max_slippage_pct?: number | null; price_mode?: string; stop_loss_pct?: number | null; early_fire_secs?: number | null; allowed_hours?: number[]; rv_min_btc?: number | null; kelly_size_cap?: number; max_runner_loss_pct?: number | null; max_consecutive_losses?: number | null; min_entry_price?: number }) => void
  isPatching?: boolean
  onUpgradeToLive?: () => void
}

function useTickRecorder(slug: string) {
  const { data, refetch } = useQuery<{ running: string[] }>({
    queryKey: ['tick-recorder-status'],
    queryFn: () => apiFetch('/api/tick-recorder/status'),
    refetchInterval: 10_000,
    staleTime: 5_000,
  })
  const isRecording = (data?.running ?? []).includes(slug)

  const startMutation = useMutation({
    mutationFn: (body: { slug: string; condition_id: string; binance_symbol: string }) =>
      apiPost('/api/tick-recorder/start', body),
    onSuccess: () => refetch(),
  })

  const stopMutation = useMutation({
    mutationFn: (s: string) => apiPost('/api/tick-recorder/stop', { slug: s }),
    onSuccess: () => refetch(),
  })

  return { isRecording, startMutation, stopMutation, isLoading: startMutation.isPending || stopMutation.isPending }
}

function RunnerCard({ runner, onStop, onRestart, onDelete, onToggleHidden, onUpdateConfig, isPatching, onUpgradeToLive }: RunnerCardProps) {
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
  // Sentinel -1 = "not yet seeded". On the first effect run we record the
  // current trade count without firing confetti, so reloading the page does
  // not celebrate historical wins. Subsequent runs fire only on NEW wins.
  const prevTradesRef = useRef<number>(-1)
  const { config, status, result } = runner
  const isRunning = status.status === 'running' || status.status === 'starting'

  // Tick recorder (Polymarket binary only)
  const tickSlug = config.series_id ?? config.symbol?.toLowerCase().replace('usdt', '_5m') ?? ''
  const { isRecording, startMutation, stopMutation, isLoading: tickLoading } = useTickRecorder(
    config.market_type === 'polymarket_binary' ? tickSlug : ''
  )
  const [showTickForm, setShowTickForm] = useState(false)
  const [tickConditionId, setTickConditionId] = useState('')
  const [tickDetecting, setTickDetecting] = useState(false)
  const [tickDetectError, setTickDetectError] = useState('')

  // Onchain sync
  const syncOnchainMutation = useMutation({
    mutationFn: () => apiPost(`/api/live/strategies/${config.id}/sync-onchain`, {}),
  })

  function autoDetectConditionId(seriesId: string) {
    if (!seriesId) return
    setTickDetecting(true)
    setTickDetectError('')
    apiFetch(`/api/polymarket/active-token?series_id=${encodeURIComponent(seriesId)}`)
      .then((data: any) => {
        if (data?.condition_id) {
          setTickConditionId(data.condition_id)
        } else {
          setTickDetectError('No active market found — enter manually')
        }
      })
      .catch(() => setTickDetectError('Detection failed — enter manually'))
      .finally(() => setTickDetecting(false))
  }

  // Tick every 30s so the uptime label refreshes without waiting for the
  // outer 5s polling cycle to swap status props.
  const [, setUptimeTick] = useState(0)
  useEffect(() => {
    if (!isRunning) return
    const t = setInterval(() => setUptimeTick(v => v + 1), 30_000)
    return () => clearInterval(t)
  }, [isRunning])
  const uptime = isRunning ? formatUptime(status.started_at) : ''

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

  // Trigger celebration on profitable trades (paper mode only — live trades are real orders).
  // Only NEW wins that arrive AFTER this component mounts fire confetti. On the
  // first pass we just record the trade count so a page refresh doesn't replay
  // historical celebrations.
  useEffect(() => {
    if (config.mode === 'live') return
    const trades = config.market_type === 'polymarket_binary' ? result?.live_orders : result?.all_trades
    if (!trades) return
    if (prevTradesRef.current < 0) {
      // First observation — seed and skip.
      prevTradesRef.current = trades.length
      return
    }
    if (trades.length <= prevTradesRef.current) {
      // Trades list may shrink (reset / re-fetch ordering quirk). Resync.
      prevTradesRef.current = trades.length
      return
    }
    const newTrades = trades.slice(prevTradesRef.current)
    const hasWin = newTrades.some((t: any) => t.pnl && t.pnl > 0)
    if (hasWin) {
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
            <h3 className="text-sm font-semibold truncate">{config.name || strategyDisplayLabel(config)}</h3>
            <span
              className="text-xs px-1.5 py-0.5 rounded flex-shrink-0"
              style={{ backgroundColor: 'var(--color-base)', color: statusColor(status.status) }}
            >
              {status.status}
            </span>
            {uptime && (
              <span
                className="text-[10px] px-1.5 py-0.5 rounded flex-shrink-0 font-mono"
                style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)' }}
                title={`Started ${fmt(status.started_at)}`}
              >
                {uptime}
              </span>
            )}
            {config.mode === 'live' ? (
              <span
                className="text-xs px-1.5 py-0.5 rounded flex-shrink-0 font-semibold"
                style={{ backgroundColor: 'rgba(245,158,11,0.15)', color: 'var(--color-warning)' }}
                title="Live mode — orders are placed with real funds."
              >
                LIVE
              </span>
            ) : (
              <span
                className="text-xs px-1.5 py-0.5 rounded flex-shrink-0 font-semibold"
                style={{ backgroundColor: 'rgba(129,140,248,0.18)', color: '#818cf8' }}
                title="Dry Run — paper-traded simulation. No real funds at risk; CLOB slippage and partial fills are not modeled."
              >
                DRY RUN
              </span>
            )}
          </div>
          <p className="text-xs font-mono truncate" style={{ color: 'var(--color-text-muted)' }}>
            {strategyDisplayLabel(config)} · {config.series_id || config.symbol || '—'} · {config.interval || '—'} · {config.mode === 'paper' ? 'dry run' : config.mode}
            {config.market_type === 'funding_arb' ? ' · funding arb' : ''}
            {config.market_type === 'polymarket_binary' ? ` · ${config.resolution_logic ?? 'price_up'}${config.threshold !== undefined && config.threshold !== null ? `(${config.threshold})` : ''}` : ''}
            {config.market_type === 'polymarket_binary' && config.live_sizing_mode
              ? ` · ${config.live_sizing_mode === 'percent' ? `${config.live_sizing_value}%` : `$${config.live_sizing_value}`}`
              : ''}
            {config.market_type === 'polymarket_binary' && config.max_entry_price != null
              ? ` · max≤$${config.max_entry_price.toFixed(2)}`
              : ''}
            {config.market_type === 'polymarket_binary' && config.max_spread_pct != null
              ? ` · spread≤${(config.max_spread_pct * 100).toFixed(2)}%`
              : ''}
            {config.market_type === 'polymarket_binary' && config.early_fire_secs != null && config.early_fire_secs > 0
              ? ` · early ${config.early_fire_secs}s`
              : ''}
          </p>
        </div>
        <div className="flex items-center gap-1 flex-shrink-0">
          {/* Tick recorder button — Polymarket Binary only */}
          {config.market_type === 'polymarket_binary' && (
            <button
              onClick={() => {
                if (isRecording) {
                  stopMutation.mutate(tickSlug)
                } else {
                  const next = !showTickForm
                  setShowTickForm(next)
                  if (next && config.series_id && !tickConditionId) {
                    autoDetectConditionId(config.series_id)
                  }
                }
              }}
              disabled={tickLoading}
              title={isRecording ? `Stop tick recorder (${tickSlug})` : 'Start tick recorder'}
              className="flex items-center gap-1 px-2 py-1 rounded text-[10px] font-semibold transition-opacity disabled:opacity-50"
              style={{
                backgroundColor: isRecording ? 'rgba(34,197,94,0.15)' : 'rgba(99,102,241,0.12)',
                color: isRecording ? 'var(--color-accent)' : 'var(--color-text-muted)',
              }}
            >
              <Activity size={11} className={isRecording ? 'animate-pulse' : ''} />
              {isRecording ? 'Recording' : 'Record'}
            </button>
          )}
          {config.mode === 'live' && (
            <button
              onClick={() => syncOnchainMutation.mutate()}
              disabled={syncOnchainMutation.isPending}
              title="Sync untracked onchain transactions into runner log"
              className="flex items-center gap-1 px-2 py-1 rounded text-[10px] font-semibold disabled:opacity-50 transition-opacity"
              style={{ backgroundColor: 'rgba(34,197,94,0.12)', color: 'var(--color-accent)' }}
            >
              <RefreshCw size={11} className={syncOnchainMutation.isPending ? 'animate-spin' : ''} />
              {syncOnchainMutation.isPending ? 'Syncing…' : 'Sync'}
            </button>
          )}
          {config.mode === 'paper' && onUpgradeToLive && (
            <button
              onClick={onUpgradeToLive}
              title="Go Live"
              className="flex items-center gap-1 px-2 py-1 rounded text-[10px] font-semibold"
              style={{ backgroundColor: 'rgba(245,158,11,0.15)', color: 'var(--color-warning)' }}
            >
              <Zap size={11} />
              Go Live
            </button>
          )}
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
          {!isRunning && (
            <button onClick={onToggleHidden} title={runner.hidden ? 'Show' : 'Hide'}
              className="p-1.5 rounded hover:bg-white/5" style={{ color: 'var(--color-text-muted)' }}>
              {runner.hidden ? <Eye size={14} /> : <EyeOff size={14} />}
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

      {/* Tick recorder inline form — appears when "Record" is clicked and not yet recording */}
      {config.market_type === 'polymarket_binary' && showTickForm && !isRecording && (
        <div className="px-4 py-3 border-b flex flex-col gap-2" style={{ borderColor: 'var(--color-border)', backgroundColor: 'rgba(99,102,241,0.06)' }}>
          <div className="flex items-center gap-2">
            <Activity size={12} style={{ color: 'var(--color-accent)' }} />
            <span className="text-xs font-semibold" style={{ color: 'var(--color-text)' }}>Start CLOB 1 Hz recorder</span>
            <span className="text-[10px] font-mono px-1.5 py-0.5 rounded" style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}>
              slug: {tickSlug}
            </span>
          </div>
          <div className="flex gap-2 items-end">
            <div className="flex-1">
              <div className="flex items-center justify-between mb-0.5">
                <label className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                  YES token Condition ID (hex)
                </label>
                {config.series_id && (
                  <button
                    type="button"
                    onClick={() => autoDetectConditionId(config.series_id!)}
                    disabled={tickDetecting}
                    className="text-[10px] px-1.5 py-0.5 rounded disabled:opacity-50 transition-opacity"
                    style={{ color: 'var(--color-accent)', backgroundColor: 'rgba(34,197,94,0.1)' }}
                  >
                    {tickDetecting ? '⏳ Detecting…' : '⚡ Auto-detect'}
                  </button>
                )}
              </div>
              <input
                className="w-full rounded border px-2 py-1.5 text-xs font-mono"
                style={{
                  background: 'var(--color-surface-2)',
                  borderColor: tickDetecting ? 'var(--color-accent)' : 'var(--color-border)',
                  color: 'var(--color-text)',
                  opacity: tickDetecting ? 0.6 : 1,
                }}
                placeholder={tickDetecting ? 'Detecting from Polymarket API…' : '0xabc123...'}
                value={tickConditionId}
                onChange={e => setTickConditionId(e.target.value)}
                disabled={tickDetecting}
              />
              {tickDetectError && (
                <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-warning)' }}>{tickDetectError}</p>
              )}
            </div>
            <button
              onClick={() => {
                if (!tickConditionId.trim()) return
                startMutation.mutate({
                  slug: tickSlug,
                  condition_id: tickConditionId.trim(),
                  binance_symbol: config.symbol || 'BTCUSDT',
                })
                setShowTickForm(false)
              }}
              disabled={!tickConditionId.trim() || tickLoading || tickDetecting}
              className="flex items-center gap-1 px-3 py-1.5 rounded text-xs font-semibold disabled:opacity-50"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              <Activity size={11} />
              Start
            </button>
            <button
              onClick={() => setShowTickForm(false)}
              className="px-3 py-1.5 rounded text-xs border"
              style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
            >
              Cancel
            </button>
          </div>
          <p className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
            {config.series_id
              ? <>Auto-detect resolves the current window's condition_id from Polymarket. Saves YES/NO bid-ask every second to{' '}<code style={{ color: 'var(--color-accent)' }}>data/ticks/{tickSlug}/</code>.</>
              : <>Find the condition ID on the Polymarket market page. Saves YES/NO bid-ask prices every second to{' '}<code style={{ color: 'var(--color-accent)' }}>data/ticks/{tickSlug}/</code>.</>
            }
          </p>
        </div>
      )}

      {/* Sizing config editor — visible when stopped so user can adjust before restart */}
      {!isRunning && onUpdateConfig && (
        <div className="px-4 pb-3 border-b" style={{ borderColor: 'var(--color-border)' }}>
          <div className="flex items-center gap-3 text-xs">
            <span style={{ color: 'var(--color-text-muted)' }} className="font-medium">Sizing:</span>
            <select
              className="rounded border px-1.5 py-0.5 text-xs"
              style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
              value={config.live_sizing_mode ?? 'percent'}
              onChange={e => onUpdateConfig({ live_sizing_mode: e.target.value })}
              disabled={isPatching}
            >
              <option value="percent">% of Balance</option>
              <option value="fixed">Fixed USD</option>
            </select>
            <div className="flex items-center gap-1">
              <input
                type="number"
                className="w-16 rounded border px-1.5 py-0.5 text-xs"
                style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                min={1}
                max={config.live_sizing_mode === 'percent' ? 100 : undefined}
                step={config.live_sizing_mode === 'percent' ? 1 : 1}
                value={config.live_sizing_value ?? (config.live_sizing_mode === 'percent' ? 5 : 50)}
                onChange={e => {
                  const val = Number(e.target.value)
                  if (!Number.isNaN(val)) {
                    onUpdateConfig({ live_sizing_value: val })
                  }
                }}
                disabled={isPatching}
              />
              <span style={{ color: 'var(--color-text-muted)' }}>
                {config.live_sizing_mode === 'percent' ? '%' : 'USDC'}
              </span>
            </div>
            {isPatching && (
              <span className="text-[10px] animate-pulse" style={{ color: 'var(--color-text-muted)' }}>Saving…</span>
            )}
          </div>
          <div className="flex items-center gap-3 text-xs mt-2">
            <span style={{ color: 'var(--color-text-muted)' }} className="font-medium">Max Entry:</span>
            <div className="flex items-center gap-1">
              <input
                type="number"
                className="w-16 rounded border px-1.5 py-0.5 text-xs"
                style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                min={0.01}
                max={0.99}
                step={0.01}
                value={config.max_entry_price != null ? config.max_entry_price : ''}
                onChange={e => {
                  const val = e.target.value === '' ? null : Number(e.target.value)
                  if (val === null || !Number.isNaN(val)) {
                    onUpdateConfig({ max_entry_price: val })
                  }
                }}
                disabled={isPatching}
              />
              <span style={{ color: 'var(--color-text-muted)' }}>$</span>
            </div>
            <SegmentedToggle
              value={config.max_entry_price != null}
              onChange={(v) => onUpdateConfig({ max_entry_price: v ? 0.65 : null })}
              leftLabel="Off"
              rightLabel="On"
              activeColor="#818cf8"
              disabled={isPatching}
            />
          </div>
          {/* Price Mode — affects entry price used for P&L accounting */}
          {config.market_type === 'polymarket_binary' && (
            <div className="flex items-center gap-3 text-xs mt-2">
              <span style={{ color: 'var(--color-text-muted)' }} className="font-medium w-24 shrink-0">Price Mode:</span>
              <select
                className="rounded border px-1.5 py-0.5 text-xs"
                style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                value={(config as any).price_mode ?? 'historical'}
                onChange={e => onUpdateConfig({ price_mode: e.target.value })}
                disabled={isPatching}
              >
                <option value="historical">Historical (real CLOB ask)</option>
                <option value="mid">Mid-price (bid+ask)/2</option>
              </select>
              <span className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                historical = real fill cost from CLOB, mid = optimistic mid-price
              </span>
            </div>
          )}
          {/* Stop Loss per trade */}
          {config.market_type === 'polymarket_binary' && config.mode === 'live' && (
            <div className="flex items-center gap-3 text-xs mt-2">
              <span style={{ color: 'var(--color-text-muted)' }} className="font-medium w-24 shrink-0">Stop Loss:</span>
              <input
                type="number"
                className="w-16 rounded border px-1.5 py-0.5 text-xs"
                style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                min={10} max={90} step={5} placeholder="40"
                value={config.stop_loss_pct != null ? Math.round(config.stop_loss_pct * 100) : ''}
                onChange={e => {
                  const raw = e.target.value
                  if (raw === '') onUpdateConfig({ stop_loss_pct: null })
                  else { const val = Number(raw); if (!Number.isNaN(val)) onUpdateConfig({ stop_loss_pct: val / 100 }) }
                }}
                disabled={isPatching}
              />
              <span style={{ color: 'var(--color-text-muted)' }}>%</span>
              <SegmentedToggle
                value={config.stop_loss_pct != null}
                onChange={v => onUpdateConfig({ stop_loss_pct: v ? 0.40 : null })}
                leftLabel="Off" rightLabel="On" activeColor="#f87171" disabled={isPatching}
              />
              <span className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                {config.stop_loss_pct != null ? `Exit early if token drops ${Math.round(config.stop_loss_pct * 100)}% from entry` : 'Disabled'}
              </span>
            </div>
          )}
          {config.market_type === 'polymarket_binary' && (
            <div className="flex items-center gap-3 text-xs mt-2">
              <span style={{ color: 'var(--color-text-muted)' }} className="font-medium">Max Spread:</span>
              <div className="flex items-center gap-1">
                <input
                  type="number"
                  className="w-16 rounded border px-1.5 py-0.5 text-xs"
                  style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                  min={0.01}
                  max={50}
                  step={0.01}
                  placeholder="3.00"
                  value={config.max_spread_pct != null ? (config.max_spread_pct * 100).toFixed(2) : ''}
                  onChange={e => {
                    const raw = e.target.value
                    if (raw === '') {
                      onUpdateConfig({ max_spread_pct: null })
                    } else {
                      const val = Number(raw) / 100
                      if (!Number.isNaN(val)) onUpdateConfig({ max_spread_pct: val })
                    }
                  }}
                  disabled={isPatching}
                />
                <span style={{ color: 'var(--color-text-muted)' }}>%</span>
              </div>
              <SegmentedToggle
                value={config.max_spread_pct != null}
                onChange={(v) => onUpdateConfig({ max_spread_pct: v ? 0.03 : null })}
                leftLabel="Off"
                rightLabel="On"
                activeColor="#818cf8"
                disabled={isPatching}
              />
            </div>
          )}
          <div className="flex items-center gap-3 text-xs mt-2">
            <span style={{ color: 'var(--color-text-muted)' }} className="font-medium">Early Fire:</span>
            <div className="flex items-center gap-1">
              <input
                type="number"
                className="w-16 rounded border px-1.5 py-0.5 text-xs"
                style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                min={1}
                max={60}
                step={1}
                placeholder="10"
                value={config.early_fire_secs != null ? config.early_fire_secs : ''}
                onChange={e => {
                  const raw = e.target.value
                  if (raw === '') {
                    onUpdateConfig({ early_fire_secs: null })
                  } else {
                    const val = parseInt(raw, 10)
                    if (!Number.isNaN(val)) onUpdateConfig({ early_fire_secs: val })
                  }
                }}
                disabled={isPatching}
              />
              <span style={{ color: 'var(--color-text-muted)' }}>s</span>
            </div>
            <SegmentedToggle
              value={config.early_fire_secs != null}
              onChange={(v) => onUpdateConfig({ early_fire_secs: v ? 10 : null })}
              leftLabel="Off"
              rightLabel="On"
              activeColor="#818cf8"
              disabled={isPatching}
            />
          </div>
          {/* Allowed Hours */}
          {config.market_type === 'polymarket_binary' && (
            <div className="flex items-start gap-3 text-xs mt-2">
              <span style={{ color: 'var(--color-text-muted)' }} className="font-medium pt-0.5 whitespace-nowrap">Hour Gate:</span>
              <div className="flex flex-col gap-1 flex-1">
                <div className="flex items-center gap-1">
                  <SegmentedToggle
                    value={(config.allowed_hours ?? []).length > 0}
                    onChange={(v) => onUpdateConfig({ allowed_hours: v ? [0, 1, 6, 18, 21, 23] : [] })}
                    leftLabel="Off"
                    rightLabel="On"
                    activeColor="#34d399"
                    disabled={isPatching}
                  />
                  <span style={{ color: 'var(--color-text-muted)' }} className="ml-1">Hot hours only</span>
                </div>
                {(config.allowed_hours ?? []).length > 0 && (
                  <div className="flex flex-wrap gap-1 mt-1">
                    {Array.from({length: 24}, (_, h) => {
                      const active = (config.allowed_hours ?? []).includes(h)
                      return (
                        <button
                          key={h}
                          disabled={isPatching}
                          onClick={() => {
                            const cur = config.allowed_hours ?? []
                            const next = active ? cur.filter(x => x !== h) : [...cur, h].sort((a,b) => a-b)
                            onUpdateConfig({ allowed_hours: next })
                          }}
                          className={clsx('w-7 h-6 rounded text-[10px] font-mono transition-colors',
                            active
                              ? 'bg-emerald-600 text-white'
                              : 'bg-transparent border'
                          )}
                          style={!active ? { borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' } : undefined}
                        >{String(h).padStart(2, '0')}</button>
                      )
                    })}
                  </div>
                )}
                <span style={{ color: 'var(--color-text-muted)' }} className="text-[10px]">
                  {(config.allowed_hours ?? []).length > 0
                    ? `Trading only in UTC hours: ${(config.allowed_hours ?? []).join(', ')} — dead hours skip silently`
                    : 'No hour restriction (trades 24/7)'}
                </span>
              </div>
            </div>
          )}
          {/* ── Guardrails (live mode — polymarket binary) ─────────────────── */}
          {config.market_type === 'polymarket_binary' && config.mode === 'live' && (
            <div className="mt-3 pt-2 border-t" style={{ borderColor: 'var(--color-border)' }}>
              <span className="text-[11px] font-semibold block mb-2" style={{ color: 'var(--color-warning)' }}>Guardrails</span>
              <div className="flex flex-col gap-2">
                {/* Max Runner Loss % */}
                <div className="flex items-center gap-3 text-xs">
                  <span style={{ color: 'var(--color-text-muted)' }} className="font-medium w-28 shrink-0">Max Loss %:</span>
                  <input
                    type="number" className="w-16 rounded border px-1.5 py-0.5 text-xs"
                    style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                    min={5} max={100} step={5} placeholder="40"
                    value={config.max_runner_loss_pct != null ? Math.round(config.max_runner_loss_pct * 100) : ''}
                    onChange={e => {
                      const raw = e.target.value
                      if (raw === '') onUpdateConfig({ max_runner_loss_pct: null })
                      else { const val = Number(raw); if (!Number.isNaN(val)) onUpdateConfig({ max_runner_loss_pct: val / 100 }) }
                    }}
                    disabled={isPatching}
                  />
                  <span style={{ color: 'var(--color-text-muted)' }}>%</span>
                  <SegmentedToggle
                    value={config.max_runner_loss_pct != null}
                    onChange={v => onUpdateConfig({ max_runner_loss_pct: v ? 0.40 : null })}
                    leftLabel="Off" rightLabel="On" activeColor="#f87171" disabled={isPatching}
                  />
                  <span className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                    {config.max_runner_loss_pct != null
                      ? `Auto-stop if down ${Math.round(config.max_runner_loss_pct * 100)}% from initial_balance`
                      : 'Disabled'}
                  </span>
                </div>
                {/* Max Consecutive Losses */}
                <div className="flex items-center gap-3 text-xs">
                  <span style={{ color: 'var(--color-text-muted)' }} className="font-medium w-28 shrink-0">Max Streak:</span>
                  <input
                    type="number" className="w-16 rounded border px-1.5 py-0.5 text-xs"
                    style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                    min={3} max={50} step={1} placeholder="8"
                    value={config.max_consecutive_losses != null ? config.max_consecutive_losses : ''}
                    onChange={e => {
                      const raw = e.target.value
                      if (raw === '') onUpdateConfig({ max_consecutive_losses: null })
                      else { const val = parseInt(raw, 10); if (!Number.isNaN(val)) onUpdateConfig({ max_consecutive_losses: val }) }
                    }}
                    disabled={isPatching}
                  />
                  <span style={{ color: 'var(--color-text-muted)' }}>losses</span>
                  <SegmentedToggle
                    value={config.max_consecutive_losses != null}
                    onChange={v => onUpdateConfig({ max_consecutive_losses: v ? 8 : null })}
                    leftLabel="Off" rightLabel="On" activeColor="#f87171" disabled={isPatching}
                  />
                  <span className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                    {config.max_consecutive_losses != null
                      ? `Auto-stop after ${config.max_consecutive_losses} consecutive losses`
                      : 'Disabled'}
                  </span>
                </div>
                {/* Min Entry Price */}
                <div className="flex items-center gap-3 text-xs">
                  <span style={{ color: 'var(--color-text-muted)' }} className="font-medium w-28 shrink-0">Min Entry ¢:</span>
                  <input
                    type="number" className="w-16 rounded border px-1.5 py-0.5 text-xs"
                    style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                    min={1} max={50} step={1} placeholder="5"
                    value={config.min_entry_price != null ? Math.round(config.min_entry_price * 100) : ''}
                    onChange={e => {
                      const val = Number(e.target.value)
                      if (!Number.isNaN(val) && val > 0) onUpdateConfig({ min_entry_price: val / 100 })
                    }}
                    disabled={isPatching}
                  />
                  <span style={{ color: 'var(--color-text-muted)' }}>¢</span>
                  <span className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                    Skip bets when token price &lt; this (default 5¢ = blocks extreme long-shots)
                  </span>
                </div>
                {/* Kelly Cap */}
                <div className="flex items-center gap-3 text-xs">
                  <span style={{ color: 'var(--color-text-muted)' }} className="font-medium w-28 shrink-0">Kelly Cap:</span>
                  <input
                    type="number" className="w-16 rounded border px-1.5 py-0.5 text-xs"
                    style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                    min={1.0} max={3.0} step={0.1} placeholder="1.5"
                    value={config.kelly_size_cap ?? 1.5}
                    onChange={e => {
                      const val = Number(e.target.value)
                      if (!Number.isNaN(val)) onUpdateConfig({ kelly_size_cap: val })
                    }}
                    disabled={isPatching}
                  />
                  <span className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                    × max kelly multiplier (default 1.5 — prevents BNB-style 6× bet explosions)
                  </span>
                </div>
              </div>
            </div>
          )}

          {/* RV Min */}
          {config.market_type === 'polymarket_binary' && (
            <div className="flex items-center gap-3 text-xs mt-2">
              <span style={{ color: 'var(--color-text-muted)' }} className="font-medium whitespace-nowrap">RV Floor:</span>
              <div className="flex items-center gap-1">
                <input
                  type="number"
                  className="w-20 rounded border px-1.5 py-0.5 text-xs font-mono"
                  style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                  min={0}
                  step={0.000005}
                  placeholder="0.00015"
                  value={config.rv_min_btc != null ? config.rv_min_btc : ''}
                  onChange={e => {
                    const raw = e.target.value
                    if (raw === '') { onUpdateConfig({ rv_min_btc: null }) }
                    else { const val = Number(raw); if (!Number.isNaN(val)) onUpdateConfig({ rv_min_btc: val }) }
                  }}
                  disabled={isPatching}
                />
              </div>
              <SegmentedToggle
                value={config.rv_min_btc != null && config.rv_min_btc > 0}
                onChange={(v) => onUpdateConfig({ rv_min_btc: v ? 0.00015 : null })}
                leftLabel="Off"
                rightLabel="On"
                activeColor="#34d399"
                disabled={isPatching}
              />
              <span style={{ color: 'var(--color-text-muted)' }}>
                {config.rv_min_btc != null && config.rv_min_btc > 0
                  ? `Skip when BTC 1h RV < ${config.rv_min_btc.toFixed(5)} (flat market filter)`
                  : 'No RV filter'}
              </span>
            </div>
          )}
        </div>
      )}

      {/* P&L summary — paper mode only (crypto only, not funding_arb) */}
      {result && config.mode === 'paper' && config.market_type === 'crypto' && (
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

      {/* Funding arb summary */}
      {config.market_type === 'funding_arb' && result && (
        <div className="px-4 pb-3 text-xs">
          <div className="grid grid-cols-3 gap-2">
            <div>
              <div style={{ color: 'var(--color-text-muted)' }}>Open Pairs</div>
              <div className="font-semibold">{result.position ?? 0}</div>
            </div>
            <div>
              <div style={{ color: 'var(--color-text-muted)' }}>Orders</div>
              <div className="font-semibold">{result.live_orders?.length ?? 0}</div>
            </div>
            <div>
              <div style={{ color: 'var(--color-text-muted)' }}>Status</div>
              <div className="font-semibold truncate" style={{ color: 'var(--color-text-muted)' }}>
                {result.last_signal || '—'}
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Polymarket binary trade summary — same metrics for paper and live, but
          paper-mode metrics are clearly labeled "(simulated)" so a non-expert
          doesn't confuse a green dry-run PnL with a real one. */}
      {config.market_type === 'polymarket_binary' && (() => {
        const isPaper = config.mode === 'paper'
        const pnlLabel = isPaper ? 'Simulated P&L' : 'P&L'
        const tradesLabel = isPaper ? 'Simulated Trades' : 'Total Trades'
        return (
          <div className="grid grid-cols-5 gap-2 px-4 pb-3 text-xs">
            <div>
              <div style={{ color: 'var(--color-text-muted)' }}>Last Signal</div>
              <div className="font-semibold"
                style={{ color: result?.last_signal === 'buy' ? 'var(--color-accent)' : result?.last_signal === 'sell' ? 'var(--color-danger)' : 'var(--color-text-muted)' }}>
                {result?.last_signal || 'waiting...'}
              </div>
            </div>
            <div>
              <div style={{ color: 'var(--color-text-muted)' }} title={isPaper ? 'Paper trades — recorded by the runner but never sent to the exchange.' : undefined}>
                {tradesLabel}
              </div>
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
              <div style={{ color: 'var(--color-text-muted)' }} title={isPaper ? 'Paper PnL: assumes mid-price fills with no slippage and instant order entry. Real CLOB execution may differ.' : undefined}>
                {pnlLabel}
              </div>
              <div className="font-semibold" style={{
                color: (result?.live_orders?.reduce((s, o) => s + (o.pnl ?? 0), 0) ?? 0) >= 0
                  ? (isPaper ? '#818cf8' : 'var(--color-accent)')
                  : 'var(--color-danger)'
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
        )
      })()}

      {expanded && (
        <>

      {/* Equity Chart — paper mode (crypto only) */}
      {config.mode === 'paper' && config.market_type === 'crypto' && result && result.all_trades?.length > 0 && (
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

      {/* Equity Chart for polymarket_binary live_orders.
          Each runner tracks its own P&L from its own live_orders — completely
          independent of the shared wallet balance. Using wallet balance here
          would corrupt the chart when multiple runners share the same wallet. */}
      {config.market_type === 'polymarket_binary' && result?.live_orders && result.live_orders.length > 0 && (() => {
        const cumPnl = runnerPnlUSD(runner)
        const startBalance = config.initial_balance
        const currentBalance = startBalance + cumPnl
        return (
          <div className="px-4 pb-2 border-t pt-3" style={{ borderColor: 'var(--color-border)' }}>
            <div className="flex items-center justify-between mb-1.5">
              <span className="text-xs font-medium" style={{ color: 'var(--color-text-muted)' }}>
                Equity Curve
              </span>
              <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                {result.live_orders.filter(o => o.pnl != null).length} trades · ${fmtUSD(currentBalance)}
                {config.mode === 'live' && typeof result.wallet_balance_usdc === 'number' && (
                  <> (wallet: ${fmtUSD(result.wallet_balance_usdc)})</>
                )}
              </span>
            </div>
            <LiveEquityChart trades={liveOrdersToTrades(result.live_orders, startBalance)} initialBalance={startBalance} />
          </div>
        )
      })()}

      {/* Onchain vs Simulado discrepancy widget — live mode only */}
      {config.mode === 'live' && config.market_type === 'polymarket_binary' && result?.live_orders && (() => {
        const simPnl = result.live_orders.reduce((s, o) => s + (o.pnl ?? 0), 0)
        const walletBal = result.wallet_balance_usdc
        const initBal = config.initial_balance
        // Estimated onchain balance = initial wallet funding - simulated losses (rough proxy)
        const simBalance = initBal + simPnl
        const hasWallet = typeof walletBal === 'number'
        const discrepancy = hasWallet ? (walletBal - simBalance) : null
        const discrepancyAbs = discrepancy !== null ? Math.abs(discrepancy) : 0
        const showDiscrepancy = discrepancyAbs > 5  // Only show if > $5 diff
        const untrackedCount = result.live_orders.filter(o => o.result === 'UNTRACKED').length

        return (
          <div className="mx-4 mb-3 rounded border overflow-hidden" style={{ borderColor: showDiscrepancy ? 'rgba(239,68,68,0.4)' : 'var(--color-border)' }}>
            <div className="px-3 py-2 text-xs flex items-center justify-between gap-3 flex-wrap"
              style={{ backgroundColor: showDiscrepancy ? 'rgba(239,68,68,0.06)' : 'var(--color-base)' }}>
              <span className="font-semibold flex items-center gap-1.5" style={{ color: showDiscrepancy ? 'var(--color-danger)' : 'var(--color-text-muted)' }}>
                <Activity size={11} />
                Onchain vs Simulado
              </span>
              <div className="flex items-center gap-4 flex-wrap">
                <span style={{ color: 'var(--color-text-muted)' }}>
                  Sim P&L: <strong style={{ color: simPnl >= 0 ? 'var(--color-accent)' : 'var(--color-danger)' }}>{simPnl >= 0 ? '+' : ''}{simPnl.toFixed(2)} USDC</strong>
                </span>
                {hasWallet && (
                  <span style={{ color: 'var(--color-text-muted)' }}>
                    Wallet real: <strong style={{ color: walletBal! >= initBal * 0.5 ? 'var(--color-text)' : 'var(--color-danger)' }}>${walletBal!.toFixed(2)}</strong>
                  </span>
                )}
                {showDiscrepancy && discrepancy !== null && (
                  <span className="font-semibold" style={{ color: 'var(--color-danger)' }}>
                    ⚠ Discrepancia: {discrepancy > 0 ? '+' : ''}{discrepancy.toFixed(2)} USDC
                  </span>
                )}
                {untrackedCount > 0 && (
                  <span className="px-1.5 py-0.5 rounded text-[10px]" style={{ backgroundColor: 'rgba(245,158,11,0.15)', color: 'var(--color-warning)' }}>
                    {untrackedCount} tx sin rastrear
                  </span>
                )}
              </div>
              <button
                onClick={() => syncOnchainMutation.mutate()}
                disabled={syncOnchainMutation.isPending}
                className="flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-semibold disabled:opacity-50"
                style={{ backgroundColor: 'rgba(34,197,94,0.12)', color: 'var(--color-accent)' }}
              >
                <RefreshCw size={9} className={syncOnchainMutation.isPending ? 'animate-spin' : ''} />
                {syncOnchainMutation.isPending ? 'Syncing…' : 'Sync Onchain'}
              </button>
            </div>
          </div>
        )
      })()}

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
      {(config.market_type === 'polymarket_binary' || config.market_type === 'funding_arb') && result?.live_orders && result.live_orders.length > 0 && (
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
                    {/* Resolution quality badge */}
                    {(order as any).resolution_source && (
                      <span
                        className="ml-1 text-[8px] px-1 rounded"
                        title={(order as any).resolution_source}
                        style={{
                          background: (order as any).resolution_source === 'polymarket' ? 'rgba(74,222,128,0.15)' : 'rgba(251,191,36,0.12)',
                          color: (order as any).resolution_source === 'polymarket' ? '#4ade80' : '#fbbf24',
                        }}
                      >
                        {(order as any).resolution_source === 'polymarket' ? '✓' : '~'}
                      </span>
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
            {config.mode === 'live' && result?.live_orders && result.live_orders.length > 0 && (
              <div className="flex justify-between">
                <span style={{ color: 'var(--color-text-muted)' }}>Runner P&L</span>
                {(() => {
                  const pnl = result.live_orders.reduce((s, o) => s + (o.pnl ?? 0), 0)
                  return (
                    <span style={{ color: pnl >= 0 ? 'var(--color-accent)' : 'var(--color-danger)', fontWeight: 600 }}>
                      {pnl >= 0 ? '+' : ''}${fmtUSD(pnl)}
                    </span>
                  )
                })()}
              </div>
            )}
            {config.mode === 'live' && (
              typeof result?.wallet_balance_usdc === 'number' ? (
                <div className="flex justify-between">
                  <span style={{ color: 'var(--color-text-muted)' }}>Wallet Balance (shared)</span>
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
  const location = useLocation()
  const routePrefill = (location.state as { prefill?: BacktestPrefill } | null)?.prefill
  // Open the create modal automatically when arriving from onboarding (?create=1).
  const autoCreate = typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('create') === '1'
  const [showCreate, setShowCreate] = useState(() => !!routePrefill || autoCreate)
  const [upgradePrefill, setUpgradePrefill] = useState<BacktestPrefill | null>(null)
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

  const patchMutation = useMutation({
    mutationFn: ({ id, body }: { id: string; body: Record<string, unknown> }) =>
      apiPatch(`/api/live/strategies/${id}`, body),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['live-strategies'] }),
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => apiDelete(`/api/live/strategies/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['live-strategies'] }),
  })

  const allRunners = data?.runners ?? []

  // ── Per-wallet analytics filter ──────────────────────────────────────
  // Live runners are grouped by the wallet they trade on so a fresh pilot
  // wallet's stats don't mix with a legacy wallet's history. Selecting a
  // wallet filters `runners` here, before all downstream totals/lists are
  // derived, so every KPI and card recomputes for that wallet automatically.
  // 'all' keeps the original combined view. Paper runners are always shown
  // (no wallet) so dry-run experiments stay visible regardless of selection.
  const [walletFilter, setWalletFilter] = useState<string>(() => {
    try { return localStorage.getItem('live-strategies-wallet-filter') || 'all' } catch { return 'all' }
  })
  const liveWallets = Array.from(
    allRunners.reduce((m, r) => {
      const w = runnerWallet(r)
      if (w) m.set(w, (m.get(w) ?? 0) + 1)
      return m
    }, new Map<string, number>())
  ).sort((a, b) => b[1] - a[1])
  const setWalletFilterPersisted = (w: string) => {
    setWalletFilter(w)
    try { localStorage.setItem('live-strategies-wallet-filter', w) } catch {}
  }
  const runners = walletFilter === 'all'
    ? allRunners
    : allRunners.filter(r => r.config.mode !== 'live' || runnerWallet(r) === walletFilter)

  const scripts = scriptsData?.scripts ?? []
  const running = runners.filter(r => r.status.status === 'running').length
  const { pnlDisplay: totalPnl, tradesDisplay: totalTradesDelta, winsDisplay: totalWinsDelta, reset: resetStats } = useResettableStats(runners)
  const [deleteTarget, setDeleteTarget] = useState<StoredRunner | null>(null)

  // Sort the strategy cards. Persisted in localStorage so the user's last
  // pick survives reloads. Default is 'default' (creation order, what the
  // backend returns) to preserve the existing UX for new users.
  type SortKey = 'default' | 'pnl_desc' | 'wr_desc'
  const [sortKey, setSortKey] = useState<SortKey>(() => {
    try {
      const saved = localStorage.getItem('live-strategies-sort') as SortKey | null
      return saved && ['default', 'pnl_desc', 'wr_desc'].includes(saved) ? saved : 'default'
    } catch { return 'default' }
  })
  const setSortKeyPersisted = (k: SortKey) => {
    setSortKey(k)
    try { localStorage.setItem('live-strategies-sort', k) } catch {}
  }
  const [showHidden, setShowHidden] = useState(false)

  const visibleRunners = runners.filter(r => !r.hidden)
  const hiddenRunners = runners.filter(r => r.hidden)

  const sortedRunners = (list: StoredRunner[]) => {
    if (sortKey === 'default') return list
    const copy = [...list]
    if (sortKey === 'pnl_desc') {
      copy.sort((a, b) => runnerPnlUSD(b) - runnerPnlUSD(a))
    } else if (sortKey === 'wr_desc') {
      const wr = (r: StoredRunner) => {
        const t = r.config.mode === 'live'
          ? (r.result?.live_total_trades ?? 0)
          : (r.result?.live_total_trades ?? r.result?.total_trades ?? 0)
        if (t === 0) return -1   // no-trade runners sink to the bottom
        const w = r.config.mode === 'live'
          ? (r.result?.live_wins ?? 0)
          : (r.result?.live_wins ?? Math.round((r.result?.win_rate_pct ?? 0) / 100 * (r.result?.total_trades ?? 0)))
        return (w / t) * 100
      }
      copy.sort((a, b) => wr(b) - wr(a))
    }
    return copy
  }
  const sortedVisible = sortedRunners(visibleRunners)
  const sortedHidden = sortedRunners(hiddenRunners)

  // Per-mode breakdown for the KPIs section. Split runners by live vs paper
  // (dry run) so the stats row shows each mode separately. Deleted runners
  // drop out of these totals automatically because we derive from the live
  // `runners` array each render — there's no separate baseline to keep in
  // sync, so deleting a strategy correctly subtracts from all totals.
  const liveRunners = runners.filter(r => r.config.mode === 'live')
  const paperRunners = runners.filter(r => r.config.mode !== 'live')
  const liveRunning = liveRunners.filter(r => r.status.status === 'running').length
  const paperRunning = paperRunners.filter(r => r.status.status === 'running').length
  const liveTotalTrades = liveRunners.reduce((s, r) => s + (r.result?.live_total_trades ?? 0), 0)
  const paperTotalTrades = paperRunners.reduce((s, r) => s + (r.result?.live_total_trades ?? r.result?.total_trades ?? 0), 0)
  const liveTotalWins = liveRunners.reduce((s, r) => s + (r.result?.live_wins ?? 0), 0)
  const paperTotalWins = paperRunners.reduce((s, r) => {
    // Paper runners also populate live_wins when the runner fires, but fall
    // back to win_rate_pct × total_trades so legacy runs still aggregate.
    const liveWins = r.result?.live_wins
    if (liveWins != null) return s + liveWins
    return s + Math.round((r.result?.win_rate_pct ?? 0) / 100 * (r.result?.total_trades ?? 0))
  }, 0)
  const liveWr = liveTotalTrades > 0 ? (liveTotalWins / liveTotalTrades) * 100 : null
  const paperWr = paperTotalTrades > 0 ? (paperTotalWins / paperTotalTrades) * 100 : null
  // Per-mode P&L. Computed directly from the current `runners` array so a
  // deleted strategy is automatically excluded.
  const livePnl = liveRunners.reduce((s, r) => s + runnerPnlUSD(r), 0)
  const paperPnl = paperRunners.reduce((s, r) => s + runnerPnlUSD(r), 0)
  const _unusedTotals = { totalPnl, totalTradesDelta, totalWinsDelta } // legacy hook return

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
          {paperRunners.length > 0 && (
            <span className="text-xs px-2 py-0.5 rounded font-semibold"
              style={{
                backgroundColor: paperPnl >= 0 ? 'rgba(129,140,248,0.15)' : 'rgba(239,68,68,0.15)',
                color: paperPnl >= 0 ? '#818cf8' : 'var(--color-danger)',
              }}
              title="Sum of P&L across Dry Run strategies (auto-updates when you delete one)"
            >
              Dry Run P&L: {paperPnl >= 0 ? '+' : ''}${fmtUSD(paperPnl)}
            </span>
          )}
          {liveRunners.length > 0 && (
            <span className="text-xs px-2 py-0.5 rounded font-semibold"
              style={{
                backgroundColor: livePnl >= 0 ? 'rgba(74,222,128,0.15)' : 'rgba(239,68,68,0.15)',
                color: livePnl >= 0 ? 'var(--color-accent)' : 'var(--color-danger)',
              }}
              title="Sum of P&L across Live strategies (auto-updates when you delete one)"
            >
              Live P&L: {livePnl >= 0 ? '+' : ''}${fmtUSD(livePnl)}
            </span>
          )}
          <button
            onClick={resetStats}
            className="text-[10px] px-2 py-0.5 rounded border hover:bg-white/5"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
            title="Reset stats baseline (legacy)"
          >
            Reset
          </button>
        </div>
        <div className="flex gap-2">
          {liveWallets.length > 1 && (
            <select
              value={walletFilter}
              onChange={e => setWalletFilterPersisted(e.target.value)}
              className="text-xs px-2 rounded border h-[34px] bg-transparent"
              style={{ borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
              title="Filter live analytics by wallet — keeps each wallet's history separate"
            >
              <option value="all">All wallets ({liveWallets.length})</option>
              {liveWallets.map(([w, n]) => (
                <option key={w} value={w}>{maskWallet(w)} · {n} runner{n === 1 ? '' : 's'}</option>
              ))}
            </select>
          )}
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
          {liveRunning > 0 && (
            <button
              onClick={() => {
                if (!confirm(`Stop ALL ${liveRunning} live runner(s) now? This is an emergency action.`)) return
                apiPost('/api/live/stop-all-live', {}).then(() => refetch())
              }}
              className="flex items-center gap-1 px-2 h-[34px] rounded text-xs font-semibold border"
              style={{ borderColor: 'var(--color-danger)', color: 'var(--color-danger)', backgroundColor: 'rgba(239,68,68,0.08)' }}
              title="Emergency: stop all live runners immediately"
            >
              <StopCircle size={12} />
              Stop All Live
            </button>
          )}
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

      {/* Wallet-level portfolio guard — cross-runner safety net (halts all live at -50%) */}
      <PortfolioGuardWidget />

      {/* Capital allocator — sizes on validated edge, not raw P&L */}
      <CapitalAllocator />

      {/* Stats row — split Live vs Dry Run with colour coding.
          Dry Run uses indigo (#818cf8) to signal paper/preview context.
          Live uses the accent green to signal real money. */}
      {runners.length > 0 && (() => {
        const DRY = '#818cf8'
        const LIVE_COLOR = 'var(--color-accent)'
        const kpis: Array<{ label: string; icon: React.ReactElement; dry: React.ReactNode; live: React.ReactNode }> = [
          { label: 'Scripts', icon: <Activity size={14} />, dry: paperRunners.length, live: liveRunners.length },
          { label: 'Running', icon: <Bot size={14} />, dry: paperRunning, live: liveRunning },
          { label: 'Total Trades', icon: <TrendingUp size={14} />, dry: paperTotalTrades, live: liveTotalTrades },
          {
            label: 'Avg Win Rate',
            icon: <TrendingDown size={14} />,
            dry: paperWr != null ? `${paperWr.toFixed(1)}%` : '—',
            live: liveWr != null ? `${liveWr.toFixed(1)}%` : '—',
          },
        ]
        return (
          <div className="grid grid-cols-4 gap-3 mb-6">
            {kpis.map(stat => (
              <div key={stat.label} className="rounded-lg border p-3"
                style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
                <div className="flex items-center gap-1.5 mb-2" style={{ color: 'var(--color-text-muted)' }}>
                  {stat.icon}
                  <span className="text-xs">{stat.label}</span>
                </div>
                <div className="grid grid-cols-2 gap-2 items-end">
                  <div>
                    <div className="text-[9px] uppercase font-bold tracking-widest" style={{ color: DRY }}>Dry Run</div>
                    <div className="text-base font-bold" style={{ color: DRY }}>{stat.dry}</div>
                  </div>
                  <div className="text-right">
                    <div className="text-[9px] uppercase font-bold tracking-widest" style={{ color: LIVE_COLOR }}>Live</div>
                    <div className="text-base font-bold" style={{ color: LIVE_COLOR }}>{stat.live}</div>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )
      })()}

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
        <>
          {/* Sort selector — placed just above the cards. Only renders when
              there's >1 visible runner; with a single card sorting is meaningless. */}
          {visibleRunners.length > 1 && (
            <div className="flex items-center justify-end gap-2 mb-3 text-xs">
              <span style={{ color: 'var(--color-text-muted)' }}>Sort:</span>
              <select
                value={sortKey}
                onChange={e => setSortKeyPersisted(e.target.value as SortKey)}
                className="rounded border px-2 py-1"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  borderColor: 'var(--color-border)',
                  color: 'var(--color-text)',
                }}
              >
                <option value="default">Default (creation order)</option>
                <option value="pnl_desc">P&amp;L ↓ (highest first)</option>
                <option value="wr_desc">Win Rate ↓ (highest first)</option>
              </select>
            </div>
          )}
          <div className="grid grid-cols-1 gap-4">
            {sortedVisible.map(runner => (
              <RunnerCard
                key={runner.config.id}
                runner={runner}
                onStop={() => stopMutation.mutate(runner.config.id)}
                onRestart={() => restartMutation.mutate(runner.config.id)}
                onDelete={() => setDeleteTarget(runner)}
                onToggleHidden={() => patchMutation.mutate({ id: runner.config.id, body: { hidden: true } })}
                onUpdateConfig={(updates) =>
                  patchMutation.mutate({ id: runner.config.id, body: updates })
                }
                isPatching={patchMutation.isPending}
                onUpgradeToLive={runner.config.mode === 'paper' ? () => {
                  setUpgradePrefill({
                    kind: runner.config.kind ?? 'rhai_candle',
                    script: runner.config.script,
                    symbol: runner.config.symbol,
                    market_type: runner.config.market_type,
                    series_id: runner.config.series_id,
                    mode: 'live',
                  })
                  setShowCreate(true)
                } : undefined}
              />
            ))}
          </div>

          {/* Hidden strategies section */}
          {hiddenRunners.length > 0 && (
            <div className="mt-6">
              <button
                onClick={() => setShowHidden(v => !v)}
                className="flex items-center gap-2 text-xs font-medium mb-3 px-3 py-1.5 rounded border"
                style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
              >
                {showHidden ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
                <EyeOff size={14} />
                <span>Hidden strategies ({hiddenRunners.length})</span>
              </button>
              {showHidden && (
                <div className="grid grid-cols-1 gap-4">
                  {sortedHidden.map(runner => (
                    <RunnerCard
                      key={runner.config.id}
                      runner={runner}
                      onStop={() => stopMutation.mutate(runner.config.id)}
                      onRestart={() => restartMutation.mutate(runner.config.id)}
                      onDelete={() => setDeleteTarget(runner)}
                      onToggleHidden={() => patchMutation.mutate({ id: runner.config.id, body: { hidden: false } })}
                      onUpdateConfig={(updates) =>
                        patchMutation.mutate({ id: runner.config.id, body: updates })
                      }
                      isPatching={patchMutation.isPending}
                    />
                  ))}
                </div>
              )}
            </div>
          )}
        </>
      )}

      {showCreate && (
        <CreateModal
          scripts={scripts}
          onClose={() => { setShowCreate(false); setUpgradePrefill(null) }}
          onCreated={() => { qc.invalidateQueries({ queryKey: ['live-strategies'] }); setUpgradePrefill(null) }}
          prefill={upgradePrefill ?? routePrefill}
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
