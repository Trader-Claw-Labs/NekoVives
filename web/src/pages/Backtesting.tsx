import { useState, useEffect, useRef } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import { useBacktestState, type BacktestConfig, type ProgressState, type MarketType, type BacktestResult, type TradeLog, type MarketSeries, POLY_BINARY_PRESETS } from '../hooks/useBacktestState'
import { CreateModal } from './LiveStrategies'
import {
  FlaskConical, Play, FileCode2, BarChart2, TrendingDown,
  AlertCircle, ChevronDown, ChevronRight, RefreshCw, Trash2,
  Pencil, Save, X, FolderOpen, Activity, Check, Eye, Code2,
  Info, Zap, ArrowUpDown, ListChecks,
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

// Window sizes for binary markets (resolution windows)
const BINARY_WINDOW_INTERVALS = [
  { value: '1m',  label: '1 min' },
  { value: '2m',  label: '2 min' },
  { value: '3m',  label: '3 min' },
  { value: '5m',  label: '5 min' },
  { value: '10m', label: '10 min' },
  { value: '15m', label: '15 min' },
  { value: '30m', label: '30 min' },
  { value: '1h',  label: '1 hour' },
]

interface PolymarketMarket {
  id: string
  question: string
  yes_price?: number
  volume?: number
}

// ── Helpers ───────────────────────────────────────────────────────

function fmt(n: number, dec = 2): string {
  return n.toFixed(dec)
}

// Compact financial format: avoids scientific notation for very large numbers
function fmtCompact(n: number, prefix = ''): string {
  if (!isFinite(n) || isNaN(n)) return '—'
  const abs = Math.abs(n)
  const sign = n < 0 ? '-' : ''
  if (abs >= 1e15) return `${sign}${prefix}${(n / 1e15).toFixed(2)}Q`
  if (abs >= 1e12) return `${sign}${prefix}${(n / 1e12).toFixed(2)}T`
  if (abs >= 1e9)  return `${sign}${prefix}${(n / 1e9).toFixed(2)}B`
  if (abs >= 1e6)  return `${sign}${prefix}${(n / 1e6).toFixed(2)}M`
  if (abs >= 1e4)  return `${sign}${prefix}${(n / 1e3).toFixed(1)}K`
  return `${sign}${prefix}${abs.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`
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
  // Scale font size down for long values to prevent overflow
  const valueFontSize = value.length > 12 ? 'text-sm' : value.length > 8 ? 'text-base' : 'text-xl'
  return (
    <div
      className="rounded-lg border p-4 min-w-0"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      <p className="text-xs mb-1 truncate" style={{ color: 'var(--color-text-muted)' }}>
        {label}
      </p>
      <p
        className={`${valueFontSize} font-bold font-mono truncate`}
        style={{ color: color ?? 'var(--color-text)' }}
        title={value}
      >
        {value}
      </p>
      {sub && <p className="text-xs mt-0.5" style={{ color: 'var(--color-text-muted)' }}>{sub}</p>}
    </div>
  )
}

// ── Equity Curve Chart ────────────────────────────────────────────

function EquityChart({
  trades,
  initialBalance,
  selectedIndex,
  onSelectTrade,
}: {
  trades: TradeLog[]
  initialBalance: number
  selectedIndex?: number
  onSelectTrade?: (index: number, trade: TradeLog) => void
}) {
  const svgRef = useRef<SVGSVGElement>(null)

  if (trades.length === 0) return null

  const W = 800
  const H = 220
  const PAD = { top: 16, right: 24, bottom: 36, left: 64 }

  // Build equity curve: start at initialBalance, then each trade balance
  const points: { x: number; y: number; trade?: TradeLog }[] = []
  const allBalances = [initialBalance, ...trades.map(t => t.balance ?? initialBalance)]
  const minB = Math.min(...allBalances) * 0.995
  const maxB = Math.max(...allBalances) * 1.005

  const chartW = W - PAD.left - PAD.right
  const chartH = H - PAD.top - PAD.bottom

  // First point = initial balance at time 0
  points.push({ x: PAD.left, y: PAD.top + chartH - ((initialBalance - minB) / (maxB - minB)) * chartH })

  trades.forEach((t, i) => {
    const x = PAD.left + ((i + 1) / trades.length) * chartW
    const bal = t.balance ?? initialBalance
    const y = PAD.top + chartH - ((bal - minB) / (maxB - minB)) * chartH
    points.push({ x, y, trade: t })
  })

  const polyline = points.map(p => `${p.x},${p.y}`).join(' ')

  // Y axis labels
  const yTicks = 4
  const yLabels = Array.from({ length: yTicks + 1 }, (_, i) => {
    const val = minB + (i / yTicks) * (maxB - minB)
    const y = PAD.top + chartH - (i / yTicks) * chartH
    return { val, y }
  })

  // X axis labels (dates)
  const xTicks = Math.min(5, trades.length)
  const xLabels = Array.from({ length: xTicks }, (_, i) => {
    const idx = Math.floor((i / (xTicks - 1)) * (trades.length - 1))
    const trade = trades[idx]
    const x = PAD.left + ((idx + 1) / trades.length) * chartW
    const label = trade ? new Date(trade.timestamp).toLocaleDateString('en-US', { month: 'short', day: 'numeric' }) : ''
    return { x, label }
  })

  return (
    <div
      className="rounded-lg border overflow-hidden"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      <div className="px-4 py-3 border-b flex items-center gap-2" style={{ borderColor: 'var(--color-border)' }}>
        <BarChart2 size={12} style={{ color: 'var(--color-accent)' }} />
        <span className="text-xs font-semibold uppercase tracking-widest" style={{ color: 'var(--color-text-muted)' }}>
          Equity Curve — {trades.length} trades
        </span>
      </div>
      <div className="p-2 overflow-x-auto">
        <svg ref={svgRef} viewBox={`0 0 ${W} ${H}`} style={{ width: '100%', minWidth: 400, height: H }}>
          {/* Grid lines */}
          {yLabels.map((l, i) => (
            <g key={i}>
              <line
                x1={PAD.left} y1={l.y} x2={W - PAD.right} y2={l.y}
                stroke="var(--color-border)" strokeWidth="0.5" strokeDasharray="4,4"
              />
              <text
                x={PAD.left - 6} y={l.y + 4}
                textAnchor="end" fontSize="9" fill="var(--color-text-muted)"
              >
                {fmtCompact(l.val, '$')}
              </text>
            </g>
          ))}

          {/* X axis labels */}
          {xLabels.map((l, i) => (
            <text key={i} x={l.x} y={H - 4} textAnchor="middle" fontSize="9" fill="var(--color-text-muted)">
              {l.label}
            </text>
          ))}

          {/* Equity line fill */}
          <defs>
            <linearGradient id="equityGrad" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="var(--color-accent)" stopOpacity="0.25" />
              <stop offset="100%" stopColor="var(--color-accent)" stopOpacity="0.02" />
            </linearGradient>
          </defs>
          <polygon
            points={`${PAD.left},${PAD.top + chartH} ${polyline} ${W - PAD.right},${PAD.top + chartH}`}
            fill="url(#equityGrad)"
          />

          {/* Equity line */}
          <polyline
            points={polyline}
            fill="none"
            stroke="var(--color-accent)"
            strokeWidth="1.5"
          />

          {/* Trade markers */}
          {points.slice(1).map((p, i) => {
            const t = p.trade!
            const isWin = t.pnl >= 0
            const color = isWin ? '#00ff88' : '#ff4444'
            const isSelected = selectedIndex === i
            return (
              <g
                key={i}
                onClick={() => onSelectTrade?.(i, t)}
                style={{ cursor: 'pointer' }}
              >
                <circle cx={p.x} cy={p.y} r={10} fill="transparent" />
                {t.side === 'buy' ? (
                  <polygon
                    points={`${p.x},${p.y - (isSelected ? 10 : 8)} ${p.x - (isSelected ? 6 : 5)},${p.y + (isSelected ? 3 : 2)} ${p.x + (isSelected ? 6 : 5)},${p.y + (isSelected ? 3 : 2)}`}
                    fill={color}
                    opacity={isSelected ? '1' : '0.85'}
                    stroke={isSelected ? '#ffffff' : 'none'}
                    strokeWidth={isSelected ? '1' : '0'}
                  >
                    <title>{t.side.toUpperCase()} @ ${t.price.toFixed(2)} | PnL: ${t.pnl.toFixed(2)}</title>
                  </polygon>
                ) : (
                  <polygon
                    points={`${p.x},${p.y + (isSelected ? 10 : 8)} ${p.x - (isSelected ? 6 : 5)},${p.y - (isSelected ? 3 : 2)} ${p.x + (isSelected ? 6 : 5)},${p.y - (isSelected ? 3 : 2)}`}
                    fill={color}
                    opacity={isSelected ? '1' : '0.85'}
                    stroke={isSelected ? '#ffffff' : 'none'}
                    strokeWidth={isSelected ? '1' : '0'}
                  >
                    <title>{t.side.toUpperCase()} @ ${t.price.toFixed(2)} | PnL: ${t.pnl.toFixed(2)}</title>
                  </polygon>
                )}
              </g>
            )
          })}

          {/* Axes */}
          <line x1={PAD.left} y1={PAD.top} x2={PAD.left} y2={PAD.top + chartH} stroke="var(--color-border)" strokeWidth="1" />
          <line x1={PAD.left} y1={PAD.top + chartH} x2={W - PAD.right} y2={PAD.top + chartH} stroke="var(--color-border)" strokeWidth="1" />
        </svg>

        {/* Legend */}
        <div className="flex gap-4 px-2 pb-1 text-xs" style={{ color: 'var(--color-text-muted)' }}>
          <span className="flex items-center gap-1">
            <span style={{ display: 'inline-block', width: 8, height: 8, background: '#00ff88', clipPath: 'polygon(50% 0%, 0% 100%, 100% 100%)' }} />
            Win
          </span>
          <span className="flex items-center gap-1">
            <span style={{ display: 'inline-block', width: 8, height: 8, background: '#ff4444', clipPath: 'polygon(50% 0%, 0% 100%, 100% 100%)' }} />
            Loss
          </span>
          <span className="flex items-center gap-1">
            <span style={{ display: 'inline-block', width: 16, height: 2, background: 'var(--color-accent)', marginBottom: 2 }} />
            Equity
          </span>
        </div>
      </div>
    </div>
  )
}

function ResultPanel({ result }: { result: BacktestResult }) {
  const [showTrades, setShowTrades] = useState(false)
  const [selectedTradeIndex, setSelectedTradeIndex] = useState<number | null>(null)
  const isBinary = result.avg_token_price != null

  const initialBalance = result.initial_balance ?? 10000
  const finalBalance = result.final_balance ?? (initialBalance * (1 + result.total_return_pct / 100))
  const avgTicket = result.total_trades > 0 && result.all_trades?.length
    ? result.all_trades.reduce((sum, t) => sum + Math.abs(t.price * t.size), 0) / result.all_trades.length
    : null

  useEffect(() => {
    if (result.all_trades && result.all_trades.length > 0) {
      setSelectedTradeIndex(result.all_trades.length - 1)
    } else {
      setSelectedTradeIndex(null)
    }
  }, [result])

  const selectedTrade = selectedTradeIndex != null && result.all_trades?.[selectedTradeIndex]
    ? result.all_trades[selectedTradeIndex]
    : null

  return (
    <div className="space-y-4">
      {/* Metrics grid */}
      <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-3">
        <MetricCard
          label="Total Return"
          value={`${result.total_return_pct >= 0 ? '+' : ''}${fmt(result.total_return_pct)}%`}
          color={colorFor(result.total_return_pct)}
        />
        <MetricCard
          label="Final Balance"
          value={fmtCompact(finalBalance, '$')}
          sub={`from $${initialBalance.toLocaleString('en-US', { maximumFractionDigits: 0 })}`}
          color={colorFor(finalBalance - initialBalance)}
        />
        <MetricCard
          label="Avg Ticket"
          value={avgTicket != null ? fmtCompact(avgTicket, '$') : '—'}
          sub="avg stake per trade"
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

      {/* Binary-specific metrics row */}
      {isBinary && result.markets_tested != null && (
        <div
          className="flex items-center gap-2 px-3 py-2 rounded text-xs mb-1"
          style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}
        >
          <Activity size={11} style={{ color: 'var(--color-accent)', flexShrink: 0 }} />
          <span>
            Tested <span className="font-semibold font-mono" style={{ color: 'var(--color-text)' }}>{result.markets_tested.toLocaleString()}</span> real
            {' '}Polymarket slug windows (btc-updown-*) u2014 decision at minute Nu22122, resolved at window close
          </span>
        </div>
      )}
      {isBinary && (
        <div className="grid grid-cols-3 gap-3">
          <MetricCard
            label="Avg Token Price"
            value={`$${fmt(result.avg_token_price!, 3)}`}
            sub="per YES/NO token"
            color={result.avg_token_price! < 0.65 ? 'var(--color-accent)' : 'var(--color-warning)'}
          />
          <MetricCard
            label="Break-even Rate"
            value={`${fmt(result.break_even_win_rate!)}%`}
            sub="win rate needed to profit"
            color={
              result.win_rate_pct > result.break_even_win_rate!
                ? 'var(--color-accent)'
                : 'var(--color-danger)'
            }
          />
          <MetricCard
            label="Direction Accuracy"
            value={`${fmt(result.correct_direction_pct!)}%`}
            sub="called direction correctly"
            color={result.correct_direction_pct! >= 50 ? 'var(--color-accent)' : 'var(--color-warning)'}
          />
        </div>
      )}

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
      {/* Equity curve chart */}
      {result.all_trades && result.all_trades.length > 0 && (
        <>
          <EquityChart
            trades={result.all_trades}
            initialBalance={result.initial_balance ?? 10000}
            selectedIndex={selectedTradeIndex ?? undefined}
            onSelectTrade={(index) => setSelectedTradeIndex(index)}
          />
          {selectedTrade && (
            <div
              className="rounded-lg border p-4"
              style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
            >
              <div className="flex items-center justify-between mb-3">
                <p className="text-xs font-semibold uppercase tracking-widest" style={{ color: 'var(--color-text-muted)' }}>
                  Trade Detail
                </p>
                <p className="text-xs font-mono" style={{ color: 'var(--color-text-muted)' }}>
                  #{(selectedTradeIndex ?? 0) + 1} / {result.all_trades.length}
                </p>
              </div>
              <div className="grid grid-cols-2 md:grid-cols-3 gap-3 text-xs">
                <div>
                  <p style={{ color: 'var(--color-text-muted)' }}>Timestamp</p>
                  <p className="font-mono" style={{ color: 'var(--color-text)' }}>
                    {new Date(selectedTrade.timestamp).toLocaleString()}
                  </p>
                </div>
                <div>
                  <p style={{ color: 'var(--color-text-muted)' }}>Side / Position</p>
                  <p
                    className="font-mono"
                    style={{
                      color: ['buy', 'yes_win', 'no_win', 'close', 'take_profit'].includes(selectedTrade.side)
                        ? 'var(--color-accent)'
                        : 'var(--color-danger)',
                    }}
                  >
                    {selectedTrade.side.replace(/_/g, ' ').toUpperCase()}
                  </p>
                </div>
                <div>
                  <p style={{ color: 'var(--color-text-muted)' }}>
                    {['yes_win','yes_loss','no_win','no_loss'].includes(selectedTrade.side) ? 'Token Price' : 'Price'}
                  </p>
                  <p className="font-mono" style={{ color: 'var(--color-text)' }}>
                    {fmtCompact(selectedTrade.price, '$')}
                  </p>
                </div>
                <div>
                  <p style={{ color: 'var(--color-text-muted)' }}>
                    {['yes_win','yes_loss','no_win','no_loss'].includes(selectedTrade.side) ? 'Stake (USD)' : 'Size'}
                  </p>
                  <p className="font-mono" style={{ color: 'var(--color-text)' }}>
                    {['yes_win','yes_loss','no_win','no_loss'].includes(selectedTrade.side)
                      ? fmtCompact(selectedTrade.size * selectedTrade.price, '$')
                      : fmtCompact(selectedTrade.size)}
                  </p>
                </div>
                <div>
                  <p style={{ color: 'var(--color-text-muted)' }}>PnL</p>
                  <p className="font-mono" style={{ color: colorFor(selectedTrade.pnl) }}>
                    {selectedTrade.pnl >= 0 ? '+' : ''}{fmtCompact(selectedTrade.pnl, '$')}
                  </p>
                </div>
                <div>
                  <p style={{ color: 'var(--color-text-muted)' }}>Balance After Trade</p>
                  <p className="font-mono" style={{ color: 'var(--color-text)' }}>
                    {selectedTrade.balance != null ? fmtCompact(selectedTrade.balance, '$') : '—'}
                  </p>
                </div>
              </div>
            </div>
          )}
        </>
      )}

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

/**
 * Estimates a download progress percentage (0–95) from elapsed seconds.
 * Uses a logarithmic curve: fast at first, slows near 95%.
 * At 5s → ~50%, 15s → ~75%, 60s → ~92%, never reaches 95 until done.
 */
function estimatePct(elapsedMs: number): number {
  const t = elapsedMs / 1000
  return Math.min(94, Math.round(95 * (1 - Math.exp(-t / 20))))
}

function ProgressPanel({ state }: { state: ProgressState }) {
  const [elapsedMs, setElapsedMs] = useState(0)

  useEffect(() => {
    if (state.startTime && state.step !== 'done' && state.step !== 'error') {
      const tick = setInterval(() => {
        setElapsedMs(Date.now() - state.startTime!)
      }, 100)
      return () => clearInterval(tick)
    }
  }, [state.startTime, state.step])

  const elapsed = Math.floor(elapsedMs / 1000)

  const steps = [
    { key: 'preparing', label: 'Preparing' },
    { key: 'fetching', label: 'Fetching Data' },
    { key: 'running', label: 'Running Engine' },
    { key: 'analyzing', label: 'Analyzing' },
  ]

  const currentIdx = steps.findIndex(s => s.key === state.step)

  // Calculate per-step progress bar
  // - preparing / running / analyzing: spin across full width quickly
  // - fetching: show estimated percentage of download
  const isFetching = state.step === 'fetching'
  const fetchPct = isFetching ? estimatePct(elapsedMs) : 0

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

      {/* Download progress bar — shown only during the fetching step */}
      {isFetching && (
        <div className="mb-3">
          <div className="flex items-center justify-between mb-1">
            <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              Downloading dataset…
            </span>
            <span className="text-xs font-mono font-semibold" style={{ color: 'var(--color-accent)' }}>
              {fetchPct}%
            </span>
          </div>
          <div className="w-full h-1.5 rounded-full overflow-hidden" style={{ backgroundColor: 'var(--color-base)' }}>
            <div
              className="h-full rounded-full"
              style={{
                width: `${fetchPct}%`,
                backgroundColor: 'var(--color-accent)',
                transition: 'width 0.4s ease-out',
              }}
            />
          </div>
        </div>
      )}

      {/* Indeterminate sliding bar for non-fetch active steps */}
      {!isFetching && state.step !== 'idle' && (
        <div className="mb-3 w-full h-1.5 rounded-full overflow-hidden relative" style={{ backgroundColor: 'var(--color-base)' }}>
          <div
            className="absolute h-full rounded-full"
            style={{
              width: '35%',
              backgroundColor: 'var(--color-accent)',
              animation: 'indeterminate-slide 1.6s cubic-bezier(0.65,0.05,0.35,0.95) infinite',
            }}
          />
          <style>{`
            @keyframes indeterminate-slide {
              0%   { left: -35%; }
              100% { left: 100%; }
            }
          `}</style>
        </div>
      )}

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
  isRunning: boolean
  isChecked: boolean
  onSelect: () => void
  onToggleCheck: () => void
  onDelete: () => void
  onRename: (newName: string) => void
  onUpdateDescription: (desc: string) => void
  onView: () => void
}

function ScriptItem({ script, isSelected, isRunning, isChecked, onSelect, onToggleCheck, onDelete, onRename, onUpdateDescription, onView }: ScriptItemProps) {
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
      className={clsx(
        'flex items-start gap-2 rounded-lg border p-3 cursor-pointer transition-colors group',
        isSelected
          ? 'border-[var(--color-accent)]'
          : 'border-[var(--color-border)] hover:border-[rgba(0,255,136,0.3)]',
      )}
      style={{ backgroundColor: 'var(--color-surface-2)' }}
    >
      {/* Checkbox for batch selection */}
      <input
        type="checkbox"
        checked={isChecked}
        onChange={(e) => {
          e.stopPropagation()
          onToggleCheck()
        }}
        className="mt-1 flex-shrink-0 cursor-pointer"
        onClick={(e) => e.stopPropagation()}
      />
      {isRunning ? (
        <RefreshCw
          size={16}
          className="mt-0.5 flex-shrink-0 animate-spin"
          style={{ color: 'var(--color-accent)' }}
        />
      ) : (
        <FileCode2
          size={16}
          className="mt-0.5 flex-shrink-0"
          style={{
            color: isSelected ? 'var(--color-accent)' : 'var(--color-text-muted)',
          }}
        />
      )}
      <div className="min-w-0 flex-1" onClick={onSelect}>
        <div className="flex items-center gap-2">
          <p className="text-sm font-mono font-semibold truncate" style={{ color: 'var(--color-text)' }}>
            {script.name}
          </p>
          {isRunning && (
            <span
              className="text-[10px] px-1.5 py-0.5 rounded font-semibold uppercase tracking-wider"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              Running
            </span>
          )}
        </div>
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
            <span style={{ color: (script.last_run_stats.win_rate_pct ?? 0) >= 50 ? 'var(--color-accent)' : 'var(--color-warning)' }}>
              {fmt(script.last_run_stats.win_rate_pct ?? 0)}% WR
            </span>
            <span>{script.last_run_stats.total_trades} trades</span>
          </div>
        )}
      </div>
      <div className="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity" onClick={(e) => e.stopPropagation()}>
        <button
          onClick={onView}
          className="p-1.5 rounded hover:bg-[var(--color-surface)]"
          title="View/Edit Code"
        >
          <Eye size={12} style={{ color: 'var(--color-text-muted)' }} />
        </button>
        <button
          onClick={() => setIsEditing(true)}
          className="p-1.5 rounded hover:bg-[var(--color-surface)]"
          title="Edit Name/Description"
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

// ── Script Viewer Modal ───────────────────────────────────────────

interface ScriptViewerProps {
  script: BacktestScript | null
  onClose: () => void
  onSave: (path: string, content: string) => void
}

function ScriptViewer({ script, onClose, onSave }: ScriptViewerProps) {
  const [content, setContent] = useState('')
  const [originalContent, setOriginalContent] = useState('')
  const [isLoading, setIsLoading] = useState(true)
  const [isSaving, setIsSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!script) return

    setIsLoading(true)
    setError(null)

    apiFetch<{ content: string }>(`/api/backtest/scripts/content?path=${encodeURIComponent(script.path)}`)
      .then((data) => {
        setContent(data.content)
        setOriginalContent(data.content)
        setIsLoading(false)
      })
      .catch((err) => {
        setError(err.message || 'Failed to load script')
        setIsLoading(false)
      })
  }, [script])

  if (!script) return null

  const hasChanges = content !== originalContent

  const handleSave = async () => {
    setIsSaving(true)
    try {
      await onSave(script.path, content)
      setOriginalContent(content)
    } catch (err) {
      setError((err as Error).message || 'Failed to save')
    }
    setIsSaving(false)
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ backgroundColor: 'rgba(0,0,0,0.8)' }}
      onClick={onClose}
    >
      <div
        className="w-full max-w-4xl max-h-[90vh] rounded-lg border flex flex-col"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div
          className="flex items-center justify-between px-4 py-3 border-b"
          style={{ borderColor: 'var(--color-border)' }}
        >
          <div className="flex items-center gap-3">
            <Code2 size={18} style={{ color: 'var(--color-accent)' }} />
            <div>
              <h3 className="text-sm font-semibold font-mono" style={{ color: 'var(--color-text)' }}>
                {script.name}
              </h3>
              {script.description && (
                <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                  {script.description}
                </p>
              )}
            </div>
          </div>
          <div className="flex items-center gap-2">
            {hasChanges && (
              <span className="text-xs px-2 py-0.5 rounded" style={{ backgroundColor: 'var(--color-warning)', color: '#000' }}>
                Unsaved changes
              </span>
            )}
            <button
              onClick={onClose}
              className="p-1.5 rounded hover:bg-[var(--color-surface-2)]"
              title="Close"
            >
              <X size={16} style={{ color: 'var(--color-text-muted)' }} />
            </button>
          </div>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-hidden p-4">
          {isLoading ? (
            <div className="flex items-center justify-center h-64">
              <RefreshCw size={24} className="animate-spin" style={{ color: 'var(--color-accent)' }} />
            </div>
          ) : error ? (
            <div
              className="flex items-center gap-2 p-4 rounded"
              style={{ backgroundColor: 'rgba(255,68,68,0.1)', color: 'var(--color-danger)' }}
            >
              <AlertCircle size={16} />
              <span className="text-sm">{error}</span>
            </div>
          ) : (
            <textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              className="w-full h-full min-h-[400px] rounded p-3 font-mono text-sm resize-none"
              style={{
                backgroundColor: 'var(--color-surface-2)',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text)',
              }}
              spellCheck={false}
            />
          )}
        </div>

        {/* Footer */}
        <div
          className="flex items-center justify-between px-4 py-3 border-t"
          style={{ borderColor: 'var(--color-border)' }}
        >
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            {content.split('\n').length} lines · Rhai Script
          </p>
          <div className="flex items-center gap-2">
            <button
              onClick={onClose}
              className="px-3 py-1.5 rounded text-sm"
              style={{ color: 'var(--color-text-muted)' }}
            >
              Cancel
            </button>
            <button
              onClick={handleSave}
              disabled={!hasChanges || isSaving}
              className="px-3 py-1.5 rounded text-sm font-semibold flex items-center gap-1.5 disabled:opacity-40"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              {isSaving ? (
                <>
                  <RefreshCw size={12} className="animate-spin" />
                  Saving...
                </>
              ) : (
                <>
                  <Save size={12} />
                  Save Changes
                </>
              )}
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}

// ── Main page ─────────────────────────────────────────────────────

export default function Backtesting() {
  // Use persisted backtest state hook - survives navigation
  const {
    config,
    result,
    progress,
    isRunning,
    runningScriptPath,
    scriptResults,
    setConfig: setConfigField,
    setFullConfig,
    runBacktest,
    runBacktestAsync,
  } = useBacktestState()

  // Load market series from API
  const { data: seriesData } = useQuery<{ series: MarketSeries[] }>({
    queryKey: ['backtest-series'],
    queryFn: () => apiFetch('/api/backtest/series'),
    staleTime: 10 * 60 * 1000,
  })
  const allSeries: MarketSeries[] = seriesData?.series ?? []
  const currentSeries = allSeries.find(s => s.id === (config.series_id ?? config.poly_binary_preset))

  // Migrate stale 'polymarket' CLOB state to 'polymarket_binary'
  useEffect(() => {
    if ((config.market_type as string) === 'polymarket') {
      setFullConfig({ ...config, market_type: 'polymarket_binary', series_id: 'btc_5m', symbol: 'BTCUSDT', interval: '5m', fee_pct: 1.5 })
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Local UI state (doesn't need to persist)
  const [scriptsExpanded, setScriptsExpanded] = useState(true)
  const [viewingScript, setViewingScript] = useState<BacktestScript | null>(null)
  const [showLiveModal, setShowLiveModal] = useState(false)
  const [selectedScripts, setSelectedScripts] = useState<string[]>([])
  const [batchProgress, setBatchProgress] = useState<{ current: number; total: number; script: string } | null>(null)
  type SortMode = 'default' | 'win_rate_desc'
  const [sortBy, setSortBy] = useState<SortMode>('default')
  // Show result only when it belongs to the currently selected script; fall back to cached
  const displayResult = (result && result.script === config.script)
    ? result
    : (config.script ? scriptResults[config.script] ?? null : null)
  const isShowingCachedResult = !(result && result.script === config.script) && !!displayResult

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
        setConfigField('script', '')
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

  // Save script content mutation
  const saveScriptMutation = useMutation({
    mutationFn: ({ path, content }: { path: string; content: string }) => {
      log('Saving script content:', path)
      return apiPost('/api/backtest/scripts/content', { path, content })
    },
    onSuccess: () => {
      log('Script content saved successfully')
      refetchScripts()
    },
    onError: (err) => {
      log('Save script error:', err)
    }
  })

  // Save stats to script after successful backtest
  useEffect(() => {
    if (result && config.script) {
      const selectedScript = scripts.find(s => s.path === config.script)
      if (selectedScript) {
        apiPost('/api/backtest/scripts/stats', {
          path: config.script,
          stats: {
            total_return_pct: result.total_return_pct,
            sharpe_ratio: result.sharpe_ratio,
            win_rate_pct: result.win_rate_pct,
            total_trades: result.total_trades,
            run_date: new Date().toISOString(),
          }
        }).then(() => {
          log('Stats saved to script')
          refetchScripts()
        }).catch(err => {
          log('Failed to save stats:', err)
        })
      }
    }
  }, [result?.total_return_pct]) // Only run when result changes

  function set<K extends keyof BacktestConfig>(k: K, v: BacktestConfig[K]) {
    setConfigField(k, v)
  }

  const isBatchMode = selectedScripts.length > 1
  const canRun = (isBatchMode || !!config.script) && !isRunning && !batchProgress

  // Sort scripts by win rate descending when requested
  const sortedScripts = [...scripts].sort((a, b) => {
    if (sortBy === 'win_rate_desc') {
      const awr = a.last_run_stats?.win_rate_pct ?? -1
      const bwr = b.last_run_stats?.win_rate_pct ?? -1
      return bwr - awr
    }
    return a.name.localeCompare(b.name)
  })

  // Toggle script selection for batch runs
  const toggleScriptSelection = (path: string) => {
    setSelectedScripts(prev =>
      prev.includes(path) ? prev.filter(p => p !== path) : [...prev, path]
    )
  }

  const selectAllScripts = () => {
    if (selectedScripts.length === scripts.length) {
      setSelectedScripts([])
    } else {
      setSelectedScripts(scripts.map(s => s.path))
    }
  }

  // Save stats helper (used after each batch run too)
  const saveStatsForResult = async (scriptPath: string, res: BacktestResult) => {
    try {
      await apiPost('/api/backtest/scripts/stats', {
        path: scriptPath,
        stats: {
          total_return_pct: res.total_return_pct,
          sharpe_ratio: res.sharpe_ratio,
          win_rate_pct: res.win_rate_pct,
          total_trades: res.total_trades,
          run_date: new Date().toISOString(),
        }
      })
    } catch (err) {
      log('Failed to save stats:', err)
    }
  }

  // Batch backtest runner — sequential, one script at a time
  const runBatchBacktest = async () => {
    if (selectedScripts.length === 0) return
    setBatchProgress({ current: 0, total: selectedScripts.length, script: '' })

    for (let i = 0; i < selectedScripts.length; i++) {
      const scriptPath = selectedScripts[i]
      setBatchProgress({ current: i + 1, total: selectedScripts.length, script: scriptPath })
      setConfigField('script', scriptPath)

      const cfg: BacktestConfig = { ...config, script: scriptPath }
      try {
        const res = await runBacktestAsync(cfg)
        await saveStatsForResult(scriptPath, res)
      } catch (err) {
        log(`Batch run failed for ${scriptPath}:`, err)
        // Continue with next script — don't abort the whole batch
      }
    }

    setBatchProgress(null)
    refetchScripts()
  }

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

        <div className="space-y-3">
          {/* Row 1: Market, Script, Symbol/Series, Window */}
          <div className="grid grid-cols-2 sm:grid-cols-4 lg:grid-cols-12 gap-3 items-end">
            {/* Market Type Select */}
            <div className="lg:col-span-2">
              <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Market</label>
            <select
              value={config.market_type}
              onChange={(e) => {
                const newType = e.target.value as MarketType
                if (newType === 'polymarket_binary') {
                  const preset = POLY_BINARY_PRESETS.find(p => p.id === (config.poly_binary_preset ?? 'btc_5m'))
                    ?? POLY_BINARY_PRESETS[0]
                  setFullConfig({
                    ...config,
                    market_type: newType,
                    symbol: preset.symbol,
                    interval: preset.defaultInterval,
                    fee_pct: 1.5,
                    poly_binary_preset: preset.id,
                  })
                } else {
                  setFullConfig({
                    ...config,
                    market_type: newType,
                    symbol: newType === 'crypto' ? 'BTCUSDT' : '',
                    interval: newType === 'crypto' ? '1m' : '5m',
                    fee_pct: newType === 'crypto' ? 0.1 : 1.5,
                    max_position_usd: newType === 'crypto' ? undefined : 500,
                  })
                }
              }}
              className="w-full rounded px-2 py-2 text-sm"
              style={{
                backgroundColor: 'var(--color-surface-2)',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text)',
              }}
            >
              <option value="crypto">Crypto</option>
              <option value="polymarket_binary">Polymarket Binary</option>
            </select>
          </div>

          {/* Script select */}
          <div className="col-span-2 lg:col-span-4">
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
            ) : isBatchMode ? (
              <div
                className="w-full rounded px-3 py-2 text-sm font-mono"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                }}
              >
                <ListChecks size={12} className="inline mr-1.5" style={{ color: 'var(--color-accent)' }} />
                {selectedScripts.length} scripts selected
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

          {/* Symbol / Market selector — adapts to market type */}
          {config.market_type === 'crypto' ? (
            <div className="lg:col-span-3">
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
          ) : config.market_type === 'polymarket_binary' ? (
            <div className="col-span-2 lg:col-span-4">
              <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Market Series</label>
              <select
                value={config.series_id ?? config.poly_binary_preset ?? 'btc_5m'}
                onChange={(e) => {
                  const s = allSeries.find(s => s.id === e.target.value)
                  if (s) {
                    setFullConfig({
                      ...config,
                      series_id: s.id,
                      poly_binary_preset: s.id,
                      symbol: s.symbol,
                      interval: s.cadence,
                      resolution_logic: s.resolution_logic,
                      threshold: s.threshold ?? undefined,
                      fee_pct: 1.5,
                    })
                  }
                }}
                className="w-full rounded px-2 py-2 text-sm"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                }}
              >
                {allSeries.length === 0
                  ? POLY_BINARY_PRESETS.map(p => <option key={p.id} value={p.id}>{p.label}</option>)
                  : allSeries.map(s => (
                    <option key={s.id} value={s.id}>{s.label}</option>
                  ))
                }
              </select>
              {currentSeries && (
                <p className="text-[10px] mt-1 leading-tight" style={{ color: 'var(--color-text-muted)' }}>
                  &quot;{currentSeries.description}&quot;
                  {' \u00b7 '}
                  {currentSeries.data_source === 'open_meteo' ? 'Open-Meteo' : `Binance ${currentSeries.symbol}`}
                  {currentSeries.threshold != null && ` \u00b7 threshold: ${currentSeries.threshold}${currentSeries.unit ?? ''}` }
                </p>
              )}
              {currentSeries?.resolution_logic !== 'price_up' && (
                <div className="mt-1.5 flex items-center gap-2">
                  <label className="text-[10px] whitespace-nowrap" style={{ color: 'var(--color-text-muted)' }}>
                    Threshold ({currentSeries?.unit ?? ''})
                  </label>
                  <input
                    type="number"
                    step="0.5"
                    value={config.threshold ?? currentSeries?.threshold ?? 0}
                    onChange={(e) => set('threshold', Number(e.target.value))}
                    className="w-20 rounded px-2 py-1 text-xs font-mono"
                    style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                  />
                </div>
              )}
            </div>
          ) : null}

          {/* Interval / Window */}
          <div className={config.market_type === 'crypto' ? 'lg:col-span-3' : 'lg:col-span-2'}>
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
              {config.market_type === 'polymarket_binary' ? 'Window' : 'Interval'}
            </label>
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
              {(config.market_type === 'polymarket_binary'
                ? BINARY_WINDOW_INTERVALS
                : CRYPTO_INTERVALS
              ).map((i) => (
                <option key={i.value} value={i.value}>
                  {i.label}
                </option>
              ))}
            </select>
          </div>

          {/* Row 2: Dates, Balance, Fee, MaxPos, Run */}
          </div>
          <div className="grid grid-cols-2 sm:grid-cols-4 lg:grid-cols-12 gap-3 items-end">

          {/* From date */}
          <div className="lg:col-span-2">
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
          <div className="lg:col-span-2">
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
          <div className="lg:col-span-2">
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

          {/* Fee % */}
          <div className="lg:col-span-2">
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
              Fee %
              <span
                className="ml-1 px-1 rounded text-[9px]"
                style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}
              >
                {config.market_type === 'polymarket_binary' ? '~1.5' : '~0.1'}
              </span>
            </label>
            <input
              type="number"
              min={0}
              max={10}
              step={0.1}
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

          {/* Max Position USD — only for Polymarket binary */}
          {config.market_type === 'polymarket_binary' && (
            <div className="lg:col-span-2">
              <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
                Max Pos ($)
                <span
                  className="ml-1 px-1 rounded text-[9px]"
                  style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}
                  title="Max stake per trade. Real Polymarket 5-min windows have ~$500-$3,000 USDC liquidity each."
                >
                  liq cap
                </span>
              </label>
              <input
                type="number"
                min={5}
                step={100}
                value={config.max_position_usd ?? 500}
                onChange={(e) => set('max_position_usd', Number(e.target.value))}
                className="w-full rounded px-2 py-2 text-sm font-mono"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                }}
              />
            </div>
          )}

          {/* Run + Live Trading buttons */}
          <div className={clsx('col-span-2 flex gap-2', config.market_type !== 'polymarket_binary' && 'lg:col-span-4')}>
            <button
              onClick={() => {
                if (isBatchMode) {
                  runBatchBacktest()
                } else {
                  runBacktest()
                }
              }}
              disabled={!canRun}
              className="flex-1 flex items-center justify-center gap-2 py-2.5 rounded font-semibold text-sm transition-opacity disabled:opacity-40"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              {isRunning || batchProgress ? (
                <>
                  <RefreshCw size={14} className="animate-spin" />
                  {batchProgress
                    ? `${batchProgress.current} / ${batchProgress.total}`
                    : 'Running'}
                </>
              ) : isBatchMode ? (
                <>
                  <Play size={14} />
                  Run {selectedScripts.length} Backtests
                </>
              ) : (
                <>
                  <Play size={14} />
                  Run Backtesting
                </>
              )}
            </button>
            {!isBatchMode && config.script && !isRunning && !batchProgress && (
              <button
                onClick={() => setShowLiveModal(true)}
                className="flex items-center justify-center gap-1.5 px-3 py-2.5 rounded font-semibold text-sm whitespace-nowrap"
                style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                title="Launch this strategy in Live Trading"
              >
                <Zap size={14} style={{ color: 'var(--color-accent)' }} />
                Live
              </button>
            )}
          </div>
          </div>
        </div>
      </div>

      {/* Binary mode info banner */}
      {config.market_type === 'polymarket_binary' && (
        <div
          className="rounded-lg border px-4 py-3 mb-4 flex gap-3 items-start text-sm"
          style={{
            backgroundColor: 'rgba(0,255,136,0.04)',
            borderColor: 'rgba(0,255,136,0.2)',
          }}
        >
          <Info size={14} className="mt-0.5 flex-shrink-0" style={{ color: 'var(--color-accent)' }} />
          <div style={{ color: 'var(--color-text-muted)' }}>
            <span className="font-semibold" style={{ color: 'var(--color-accent)' }}>Slug-aligned binary simulation</span>
            {' '}— Mirrors real Polymarket <code className="text-xs px-1 py-0.5 rounded font-mono" style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text)' }}>btc-updown-{config.interval}-{'<ts>'}</code> markets.
            Each window starts at a Unix timestamp divisible by {config.interval === '5m' ? '300s' : config.interval === '4m' ? '240s' : config.interval === '15m' ? '900s' : config.interval === '1m' ? '60s' : config.interval}.
            {' '}Strategy fires at the <em>decision candle</em> (last complete 1m before window close) using Binance data
            as a Chainlink BTC/USD proxy. Resolution: close at window end vs window open.
            {' '}Token prices reflect momentum — stronger signals cost more ($0.55–$0.92/token), so higher win rates are needed to profit.
          </div>
        </div>
      )}

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
              {/* Header: Select All + Sort toggle */}
              {scripts.length > 0 && (
                <div className="flex items-center justify-between mb-2 pb-2 border-b" style={{ borderColor: 'var(--color-border)' }}>
                  <label className="flex items-center gap-1.5 text-xs cursor-pointer" style={{ color: 'var(--color-text-muted)' }}>
                    <input
                      type="checkbox"
                      checked={selectedScripts.length === scripts.length && scripts.length > 0}
                      onChange={selectAllScripts}
                      className="cursor-pointer"
                    />
                    Select All
                    {selectedScripts.length > 0 && (
                      <span className="text-[10px] px-1.5 py-0.5 rounded font-mono" style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
                        {selectedScripts.length}
                      </span>
                    )}
                  </label>
                  <button
                    onClick={() => setSortBy(prev => prev === 'default' ? 'win_rate_desc' : 'default')}
                    className="flex items-center gap-1 text-[10px] px-2 py-1 rounded hover:bg-[var(--color-surface-2)]"
                    style={{ color: sortBy === 'win_rate_desc' ? 'var(--color-accent)' : 'var(--color-text-muted)' }}
                    title={sortBy === 'win_rate_desc' ? 'Sorted by Win Rate' : 'Sort by Win Rate'}
                  >
                    <ArrowUpDown size={10} />
                    {sortBy === 'win_rate_desc' ? 'Win Rate ↓' : 'Sort'}
                  </button>
                </div>
              )}

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
                  {sortedScripts.map((s) => (
                    <ScriptItem
                      key={s.path}
                      script={s}
                      isSelected={config.script === s.path}
                      isRunning={isRunning && runningScriptPath === s.path}
                      isChecked={selectedScripts.includes(s.path)}
                      onSelect={() => set('script', s.path)}
                      onToggleCheck={() => toggleScriptSelection(s.path)}
                      onDelete={() => deleteMutation.mutate(s.path)}
                      onRename={(newName) => renameMutation.mutate({ oldPath: s.path, newName })}
                      onUpdateDescription={(desc) => updateDescMutation.mutate({ path: s.path, description: desc })}
                      onView={() => setViewingScript(s)}
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

            {/* Batch progress banner */}
            {batchProgress && (
              <div
                className="rounded-lg border px-4 py-3 mb-3 flex items-center gap-3"
                style={{ backgroundColor: 'rgba(0,255,136,0.06)', borderColor: 'rgba(0,255,136,0.2)' }}
              >
                <RefreshCw size={14} className="animate-spin" style={{ color: 'var(--color-accent)' }} />
                <div className="text-xs">
                  <span className="font-semibold" style={{ color: 'var(--color-accent)' }}>
                    Batch Backtest
                  </span>
                  {' '}— Running {batchProgress.current} of {batchProgress.total}:{' '}
                  <span className="font-mono">{batchProgress.script.split('/').pop()}</span>
                </div>
              </div>
            )}

            {/* Show progress when running */}
            {isRunning && <ProgressPanel state={progress} />}

            {/* Show error */}
            {progress.step === 'error' && (
              <div
                className="flex flex-col gap-2 text-sm px-4 py-3 rounded"
                style={{ backgroundColor: 'rgba(255,68,68,0.1)', color: 'var(--color-danger)', border: '1px solid rgba(255,68,68,0.2)' }}
              >
                <div className="flex items-center gap-2 font-semibold">
                  <AlertCircle size={14} />
                  Backtest failed
                </div>
                <p className="font-mono text-xs break-all opacity-80">
                  {progress.message.replace('Error: ', '')}
                </p>
                <p className="text-xs mt-1" style={{ color: 'var(--color-text-muted)' }}>
                  Check the browser console (F12) for detailed debug logs.
                </p>
              </div>
            )}

            {/* Show results */}
            {!isRunning && displayResult && (
              <>
                <div className="mb-3 flex items-center justify-between">
                  <div className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                    <span className="font-mono">{displayResult.script.split('/').pop()}</span> / {displayResult.symbol}
                    {isShowingCachedResult && (
                      <span
                        className="ml-2 px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase tracking-wider"
                        style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}
                      >
                        Previous run
                      </span>
                    )}
                  </div>
                  {isShowingCachedResult && (
                    <button
                      onClick={() => runBacktest()}
                      disabled={!canRun}
                      className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs font-semibold disabled:opacity-40"
                      style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
                    >
                      <Play size={11} />
                      Run Again
                    </button>
                  )}
                </div>
                <ResultPanel result={displayResult} />
              </>
            )}

            {/* Empty state */}
            {!isRunning && !displayResult && progress.step !== 'error' && (
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

      {/* Script Viewer Modal */}
      {viewingScript && (
        <ScriptViewer
          script={viewingScript}
          onClose={() => setViewingScript(null)}
          onSave={(path, content) => saveScriptMutation.mutateAsync({ path, content })}
        />
      )}

      {/* Live Trading Modal */}
      {showLiveModal && (
        <CreateModal
          scripts={scripts}
          defaultScript={config.script}
          onClose={() => setShowLiveModal(false)}
          onCreated={() => setShowLiveModal(false)}
        />
      )}
    </div>
  )
}
