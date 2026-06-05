import { useState, useEffect, useRef, useCallback } from 'react'
import { useNavigate } from 'react-router-dom'
import { useQuery, useMutation } from '@tanstack/react-query'
import CodeMirror from '@uiw/react-codemirror'
import { rust } from '@codemirror/lang-rust'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import { useBacktestState, type BacktestConfig, type ProgressState, type MarketType, type BacktestResult, type TradeLog, type MarketSeries, POLY_BINARY_PRESETS } from '../hooks/useBacktestState'
import { CreateModal } from './LiveStrategies'
import EngineParamsForm, { defaultEngineParams } from '../components/EngineParamsForm'
import EngineKindInfoCard from '../components/EngineKindInfoCard'
import { ENGINE_KINDS, engineKindOptionLabel } from '../components/engineKindMeta'
import {
  FlaskConical, Play, FileCode2, BarChart2, TrendingDown,
  AlertCircle, AlertTriangle, ChevronDown, ChevronRight, RefreshCw, Trash2,
  Pencil, Save, X, FolderOpen, Activity, Check, Eye, Code2,
  Info, Zap, ArrowUpDown, ListChecks, Database, TrendingUp,
  Download, CloudDownload,
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
    final_balance: number
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

function ResultPanel({
  result,
  onRunPaper,
  onRunLive,
}: {
  result: BacktestResult
  onRunPaper?: () => void
  onRunLive?: () => void
}) {
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

      {/* Historical Data Indicator */}
      {isBinary && result.windows_with_real_price != null && result.windows_with_real_price > 0 && (
        <div
          className="rounded-lg border p-3 text-sm"
          style={{
            backgroundColor: 'rgba(34, 197, 94, 0.08)',
            borderColor: 'rgba(34, 197, 94, 0.35)',
          }}
        >
          <div className="flex items-center gap-2 mb-1">
            <Database size={14} style={{ color: '#22c55e', flexShrink: 0 }} />
            <span className="font-semibold" style={{ color: '#22c55e' }}>
              Real On-Chain Data
            </span>
            <span
              className="px-1.5 py-0.5 rounded text-[10px] font-bold uppercase"
              style={{ backgroundColor: 'rgba(34, 197, 94, 0.15)', color: '#22c55e' }}
            >
              {result.historical_data_coverage_pct != null
                ? `${result.historical_data_coverage_pct.toFixed(1)}% coverage`
                : `${result.windows_with_real_price.toLocaleString()} windows`}
            </span>
          </div>
          <div className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            {result.windows_with_real_price.toLocaleString()} windows used real CLOB token prices
            {result.windows_with_estimated_price != null && result.windows_with_estimated_price > 0
              ? ` · ${result.windows_with_estimated_price.toLocaleString()} estimated from momentum model`
              : ''}
            {result.recommended_max_stake_usd != null && (
              <span className="ml-1">
                {' · '}
                <span style={{ color: 'var(--color-accent)' }}>
                  Recommended max stake: ${result.recommended_max_stake_usd.toLocaleString()}
                </span>
                {' (based on observed liquidity)'}
              </span>
            )}
          </div>
        </div>
      )}
      {isBinary && result.windows_with_real_price != null && result.windows_with_real_price === 0 && (
        <div
          className="rounded-lg border p-3 text-sm"
          style={{
            backgroundColor: 'rgba(234, 179, 8, 0.06)',
            borderColor: 'rgba(234, 179, 8, 0.25)',
          }}
        >
          <div className="flex items-center gap-2">
            <AlertCircle size={14} style={{ color: '#eab308', flexShrink: 0 }} />
            <span style={{ color: 'var(--color-text-muted)' }}>
              No historical on-chain data available. All prices estimated from momentum model.
              {' '}
              <span className="font-semibold" style={{ color: '#eab308' }}>
                Run: trader-claw backtest-sync --series btc_5m --from YYYY-MM-DD --to YYYY-MM-DD
              </span>
            </span>
          </div>
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

      {/* Promote to live/paper */}
      {(onRunPaper || onRunLive) && (
        <div
          className="pt-3 space-y-2"
          style={{ borderTop: '1px solid var(--color-border)' }}
        >
          <div
            className="flex items-start gap-2 rounded px-3 py-2 text-[11px] leading-snug"
            style={{
              backgroundColor: 'rgba(245,158,11,0.08)',
              border: '1px solid rgba(245,158,11,0.35)',
              color: 'var(--color-text)',
            }}
          >
            <AlertTriangle size={14} style={{ color: 'var(--color-warning)', flexShrink: 0, marginTop: 2 }} />
            <div>
              <span className="font-semibold">Backtest results don't guarantee live performance.</span>{' '}
              Backtests assume mid-price fills with no slippage; the real CLOB has wider spreads, partial fills and depth limits.
              <span className="block mt-1" style={{ color: 'var(--color-text-muted)' }}>
                Always start in <span className="font-semibold" style={{ color: '#818cf8' }}>Dry Run</span> for at least a day before promoting to Live, and re-tune <span className="font-mono">max_position_usd</span> / <span className="font-mono">edge_threshold</span> for live conditions.
              </span>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <span className="text-xs" style={{ color: 'var(--color-text-muted)', marginRight: 'auto' }}>
              Deploy this strategy:
            </span>
            {onRunPaper && (
              <button
                type="button"
                onClick={onRunPaper}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs font-semibold"
                style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
                title="Recommended — run with simulated funds first."
              >
                <Play size={11} />
                Run in Dry Run (recommended)
              </button>
            )}
            {onRunLive && (
              <button
                type="button"
                onClick={() => {
                  const ok = window.confirm(
                    'Promoting straight to Live will place real orders with real funds.\n\n' +
                    'Recommended: run in Dry Run first for at least 24h to validate the strategy under live conditions.\n\n' +
                    'Continue to Live anyway?'
                  )
                  if (ok) onRunLive()
                }}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs font-semibold"
                style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-warning)', color: 'var(--color-warning)' }}
                title="Skip Dry Run and place real orders."
              >
                <Zap size={11} />
                Run Live
              </button>
            )}
          </div>
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

// ── Rhai API Reference Panel ──────────────────────────────────────

const RHAI_API = [
  {
    category: 'Candle data (on_candle)',
    items: [
      { name: 'ctx.close', desc: 'Close price of current candle' },
      { name: 'ctx.open', desc: 'Open price' },
      { name: 'ctx.high / ctx.low', desc: 'High / low price' },
      { name: 'ctx.volume', desc: 'Volume of current candle' },
      { name: 'ctx.index', desc: 'Current candle index (int)' },
      { name: 'ctx.close_at(i)', desc: 'Close price at candle i' },
    ],
  },
  {
    category: 'Signals & position',
    items: [
      { name: 'ctx.buy(size)', desc: 'Open long (size: 0-1, fraction of balance)' },
      { name: 'ctx.sell(size)', desc: 'Close long or open short' },
      { name: 'ctx.position', desc: 'Current position (+1 long, -1 short, 0 flat)' },
      { name: 'ctx.entry_price', desc: 'Price at which current position was opened' },
      { name: 'ctx.entry_index', desc: 'Candle index when position was opened' },
    ],
  },
  {
    category: 'Indicators',
    items: [
      { name: 'ctx.rsi(n)', desc: 'RSI over last n candles' },
      { name: 'ctx.ema(n)', desc: 'Exponential MA (n candles)' },
      { name: 'ctx.sma(n)', desc: 'Simple MA (n candles)' },
      { name: 'ctx.atr(n)', desc: 'Average True Range (n candles)' },
    ],
  },
  {
    category: 'State & logging',
    items: [
      { name: 'ctx.get("key", default)', desc: 'Read persisted state value' },
      { name: 'ctx.set("key", val)', desc: 'Write persisted state value' },
      { name: 'ctx.log("msg")', desc: 'Append to debug log for this tick' },
    ],
  },
  {
    category: 'Binary market extras',
    items: [
      { name: 'ctx.token_price', desc: 'YES token price P4 (0-1)' },
      { name: 'ctx.token_drift', desc: 'P4 – P3 drift signal' },
      { name: 'ctx.window_secs_left', desc: 'Seconds until market resolves' },
      { name: 'ctx.binance_mark', desc: 'Binance spot at decision time' },
    ],
  },
  {
    category: 'CLOB 1 Hz (on_tick)',
    items: [
      { name: 'ctx.yes_bid / yes_ask', desc: 'YES best bid / ask (0-1)' },
      { name: 'ctx.yes_mid', desc: 'YES mid price' },
      { name: 'ctx.no_bid / no_ask', desc: 'NO best bid / ask (0-1)' },
      { name: 'ctx.spread_pct', desc: '(yes_ask−yes_bid)×100 in ¢' },
      { name: 'ctx.binance_price', desc: 'Binance spot at this tick' },
      { name: 'ctx.window_secs_left', desc: 'Seconds until window closes' },
      { name: 'ctx.second_in_window', desc: 'Seconds elapsed (0 = first tick)' },
      { name: 'ctx.bet_yes(size)', desc: 'Bet YES (size = fraction of balance)' },
      { name: 'ctx.bet_no(size)', desc: 'Bet NO (size = fraction of balance)' },
    ],
  },
]

function RhaiApiPanel() {
  const [open, setOpen] = useState(false)

  return (
    <div
      className="flex-shrink-0 border-l flex flex-col"
      style={{ borderColor: 'var(--color-border)', width: open ? '220px' : '36px', transition: 'width 150ms' }}
    >
      <button
        type="button"
        onClick={() => setOpen(v => !v)}
        className="flex items-center justify-center p-2 border-b text-xs font-semibold"
        style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)', gap: '6px', whiteSpace: 'nowrap' }}
        title={open ? 'Hide API reference' : 'Show API reference'}
      >
        <Info size={13} style={{ flexShrink: 0 }} />
        {open && <span>API Reference</span>}
      </button>
      {open && (
        <div className="flex-1 overflow-y-auto px-2 py-2 space-y-3 text-[11px]">
          {RHAI_API.map((section) => (
            <div key={section.category}>
              <p className="font-semibold uppercase tracking-widest mb-1" style={{ color: 'var(--color-text-muted)', fontSize: '10px' }}>
                {section.category}
              </p>
              {section.items.map((item) => (
                <div key={item.name} className="mb-1.5">
                  <code
                    className="block px-1 rounded font-mono"
                    style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-accent)', fontSize: '10px' }}
                  >
                    {item.name}
                  </code>
                  <p style={{ color: 'var(--color-text-muted)', marginTop: '1px' }}>{item.desc}</p>
                </div>
              ))}
            </div>
          ))}
        </div>
      )}
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
  const onChangeEditor = useCallback((val: string) => setContent(val), [])

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
        <div className="flex-1 overflow-hidden flex">
          {/* Editor */}
          <div className="flex-1 overflow-auto p-4">
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
              <div
                className="w-full h-full min-h-[400px] rounded overflow-auto"
                style={{ border: '1px solid var(--color-border)' }}
              >
                <CodeMirror
                  value={content}
                  extensions={[rust()]}
                  onChange={onChangeEditor}
                  theme="dark"
                  style={{ fontSize: '13px', minHeight: '400px' }}
                  basicSetup={{ lineNumbers: true, foldGutter: true, highlightActiveLine: true }}
                />
              </div>
            )}
          </div>
          {/* API Reference panel */}
          <RhaiApiPanel />
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
  const navigate = useNavigate()

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

  // Load available CLOB 1 HZ tick slugs
  interface TickSlugInfo {
    slug: string
    dates: string[]
    tick_count: number
    from_date: string
    to_date: string
  }
  const { data: tickSlugsData, refetch: refetchTickSlugs } = useQuery<{ slugs: TickSlugInfo[] }>({
    queryKey: ['backtest-tick-slugs'],
    queryFn: () => apiFetch('/api/backtest/tick-slugs'),
    staleTime: 30 * 1000,
    enabled: config.market_type === 'clob_1hz' || config.market_type === 'archive_candles',
  })
  const tickSlugs: TickSlugInfo[] = tickSlugsData?.slugs ?? []

  // Migrate stale 'polymarket' CLOB state to 'polymarket_binary'
  useEffect(() => {
    if ((config.market_type as string) === 'polymarket') {
      setFullConfig({ ...config, market_type: 'polymarket_binary', series_id: 'btc_5m', symbol: 'BTCUSDT', interval: '5m', fee_pct: 1.5 })
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // ── Dataset Download panel state ───────────────────────────────────────────
  const [datasetPanelOpen, setDatasetPanelOpen] = useState(false)
  const [ingestDays, setIngestDays] = useState(7)
  const [ingestMarket, setIngestMarket] = useState('')
  const [ingestSlug, setIngestSlug] = useState('btc_5m')
  const [ingestBinance, setIngestBinance] = useState('BTCUSDT')

  interface IngestProgress {
    running: boolean
    phase: string        // "downloading" | "converting" | "done" | ""
    done: number
    total: number
    current_hour: string
    downloaded: number
    skipped: number
    errors: string[]
    slug: string
    started_at?: string
    finished_at?: string
  }
  const [ingestProgress, setIngestProgress] = useState<IngestProgress | null>(null)

  // Poll ingest progress
  const { data: rawIngestProgress } = useQuery<IngestProgress>({
    queryKey: ['orderbook-ingest-status'],
    queryFn: () => apiFetch('/api/orderbook/download/status'),
    refetchInterval: ingestProgress?.running ? 3000 : false,
  })
  useEffect(() => {
    if (rawIngestProgress) setIngestProgress(rawIngestProgress)
  }, [rawIngestProgress])

  const ingestMutation = useMutation({
    mutationFn: () => apiPost('/api/orderbook/ingest', {
      days: ingestDays,
      market: ingestMarket.trim(),
      slug: ingestSlug.trim(),
      binance_symbol: ingestBinance.trim(),
    }),
    onSuccess: () => {
      // Start polling
      setIngestProgress(p => p ? { ...p, running: true, phase: 'downloading' } : null)
    },
  })

  const cancelIngestMutation = useMutation({
    mutationFn: () => apiPost('/api/orderbook/download/cancel', {}),
  })

  // Local UI state (doesn't need to persist)
  const [scriptsExpanded, setScriptsExpanded] = useState(true)
  const [viewingScript, setViewingScript] = useState<BacktestScript | null>(null)
  const [showLiveModal, setShowLiveModal] = useState(false)
  const [selectedScripts, setSelectedScripts] = useState<string[]>([])
  const [batchProgress, setBatchProgress] = useState<{ current: number; total: number; script: string } | null>(null)
  // ── Optimizer state (parameter sweep with TRAIN/TEST split) ──
  type OptParam = 'min_entry_price' | 'sizing_value' | 'max_spread_pct' | 'kelly_size_cap'
  const [showOptimizer, setShowOptimizer] = useState(false)
  const [optParam, setOptParam] = useState<OptParam>('min_entry_price')
  const [optGrid, setOptGrid] = useState<string>('0.10,0.15,0.20,0.25,0.30')
  const [optProgress, setOptProgress] = useState<{ phase: string; current: number; total: number } | null>(null)
  type OptRow = {
    label: string
    train: { trades: number; wr: number; ret: number; sharpe: number; dd: number } | null
    test:  { trades: number; wr: number; ret: number; sharpe: number; dd: number } | null
    isBaseline: boolean
    isWinner: boolean
  }
  const [optResults, setOptResults] = useState<OptRow[] | null>(null)
  const [optVerdict, setOptVerdict] = useState<{ kind: 'accept' | 'marginal' | 'reject'; msg: string; bestValue?: number } | null>(null)
  type SortMode = 'default' | 'win_rate_desc' | 'trades_desc' | 'balance_desc'
  const [sortBy, setSortBy] = useState<SortMode>('default')
  const SORT_MODES: SortMode[] = ['default', 'win_rate_desc', 'trades_desc', 'balance_desc']
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

  // Start the CLOB tick recorder for a built-in series — auto-resolves the
  // current condition_id from Polymarket so a non-expert never has to copy
  // a hex condition_id by hand.
  const startTickRecorderMutation = useMutation({
    mutationFn: async (seriesId: string) => {
      const series = allSeries.find(s => s.id === seriesId)
      if (!series) throw new Error(`Unknown series '${seriesId}'`)
      const active = await apiFetch<{ condition_id?: string }>(`/api/polymarket/active-token?series_id=${encodeURIComponent(seriesId)}`)
      if (!active?.condition_id) throw new Error('No active Polymarket window for this series right now — try again in a minute.')
      return apiPost('/api/tick-recorder/start', {
        slug: seriesId,
        condition_id: active.condition_id,
        binance_symbol: series.symbol,
      })
    },
    onSuccess: () => {
      // Recorder is now writing JSONL — refresh the slug list so the picker
      // shows it (will report 0 ticks until enough seconds elapse).
      setTimeout(() => refetchTickSlugs(), 5_000)
    },
  })

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
            final_balance: result.final_balance,
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

  const isEngineKind = (config.kind ?? 'rhai_candle') !== 'rhai_candle'

  const isBatchMode = selectedScripts.length > 1
  const isArchiveMode = config.market_type === 'clob_1hz' || config.market_type === 'archive_candles'
  const hasClob1HzSlug = !isArchiveMode || !!(config.clob_slug ?? config.symbol)
  const canRun = (isBatchMode || !!config.script || isEngineKind) && hasClob1HzSlug && !isRunning && !batchProgress

  // Sort scripts by selected metric descending
  const sortedScripts = [...scripts].sort((a, b) => {
    if (sortBy === 'win_rate_desc') {
      const awr = a.last_run_stats?.win_rate_pct ?? -1
      const bwr = b.last_run_stats?.win_rate_pct ?? -1
      return bwr - awr
    }
    if (sortBy === 'trades_desc') {
      const at = a.last_run_stats?.total_trades ?? -1
      const bt = b.last_run_stats?.total_trades ?? -1
      return bt - at
    }
    if (sortBy === 'balance_desc') {
      const ab = a.last_run_stats?.final_balance ?? -1
      const bb = b.last_run_stats?.final_balance ?? -1
      return bb - ab
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
          final_balance: res.final_balance,
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

  // ── Optimizer: parameter sweep with TRAIN/TEST split ─────────────────────
  // Runs a sweep over `optParam` values, each on a 70%/30% TRAIN/TEST split
  // of the configured date range. The "winner" is the value with highest
  // TRAIN Sharpe; we then verify it on TEST out-of-sample. If TEST collapses
  // below baseline, we REJECT (overfit). Mirrors optimize_runner.py logic.
  const runOptimization = async () => {
    if (!config.script || !config.from_date || !config.to_date) return
    setOptResults(null)
    setOptVerdict(null)

    // Parse grid
    const grid = optGrid.split(',').map(s => parseFloat(s.trim())).filter(v => !isNaN(v))
    if (grid.length < 2) {
      setOptVerdict({ kind: 'reject', msg: 'Grid debe tener al menos 2 valores separados por coma' })
      return
    }

    // 70/30 split
    const fromMs = new Date(config.from_date).getTime()
    const toMs = new Date(config.to_date).getTime()
    const totalDays = Math.floor((toMs - fromMs) / 86400000)
    if (totalDays < 7) {
      setOptVerdict({ kind: 'reject', msg: 'Rango de fechas muy corto (mínimo 7 días)' })
      return
    }
    const splitMs = fromMs + Math.floor(totalDays * 0.7) * 86400000
    const trainTo = new Date(splitMs).toISOString().slice(0, 10)
    const testFrom = new Date(splitMs + 86400000).toISOString().slice(0, 10)
    const trainRange = { from: config.from_date, to: trainTo }
    const testRange = { from: testFrom, to: config.to_date }

    const baselineParam = (config as any)[optParam]
    const totalRuns = 2 + grid.length + 1 // baseline TRAIN + baseline TEST + each grid + winner TEST
    let runIdx = 0

    const runOne = async (cfg: BacktestConfig, fromD: string, toD: string) => {
      const c: BacktestConfig = { ...cfg, from_date: fromD, to_date: toD }
      try {
        const r = await runBacktestAsync(c)
        return {
          trades: r.total_trades || 0,
          wr: r.win_rate_pct || 0,
          ret: r.total_return_pct || 0,
          sharpe: r.sharpe_ratio || 0,
          dd: r.max_drawdown_pct || 0,
        }
      } catch (e) {
        log('opt run error:', e)
        return null
      }
    }

    setOptProgress({ phase: 'Baseline TRAIN', current: ++runIdx, total: totalRuns })
    const baselineTrain = await runOne(config, trainRange.from, trainRange.to)
    setOptProgress({ phase: 'Baseline TEST', current: ++runIdx, total: totalRuns })
    const baselineTest = await runOne(config, testRange.from, testRange.to)

    const rows: OptRow[] = [{
      label: `BASELINE (${optParam}=${baselineParam ?? 'default'})`,
      train: baselineTrain,
      test: baselineTest,
      isBaseline: true,
      isWinner: false,
    }]

    // Sweep
    const candidates: { value: number; train: any }[] = []
    for (const v of grid) {
      setOptProgress({ phase: `Sweep ${optParam}=${v}`, current: ++runIdx, total: totalRuns })
      const cfgWithParam = { ...config, [optParam]: v } as BacktestConfig
      const t = await runOne(cfgWithParam, trainRange.from, trainRange.to)
      rows.push({
        label: `${optParam}=${v}`,
        train: t,
        test: null,
        isBaseline: false,
        isWinner: false,
      })
      if (t && t.trades >= 50) candidates.push({ value: v, train: t })
    }

    if (candidates.length === 0) {
      setOptVerdict({ kind: 'reject', msg: 'Ningún candidato superó el mínimo de 50 trades en TRAIN' })
      setOptResults(rows)
      setOptProgress(null)
      return
    }

    candidates.sort((a, b) => b.train.sharpe - a.train.sharpe)
    const best = candidates[0]
    setOptProgress({ phase: `OOS verify ${optParam}=${best.value}`, current: ++runIdx, total: totalRuns })
    const winnerCfg = { ...config, [optParam]: best.value } as BacktestConfig
    const winnerTest = await runOne(winnerCfg, testRange.from, testRange.to)

    // Mark winner row
    const winnerIdx = rows.findIndex(r => r.label === `${optParam}=${best.value}`)
    if (winnerIdx >= 0) {
      rows[winnerIdx].isWinner = true
      rows[winnerIdx].test = winnerTest
    }

    // Verdict
    const blRet = baselineTest?.ret ?? 0
    const oosRet = winnerTest?.ret ?? 0
    const trainRet = best.train.ret
    const delta = oosRet - blRet
    const ratio = Math.abs(trainRet) > 0.1 ? oosRet / trainRet : 0

    let verdict: { kind: 'accept' | 'marginal' | 'reject'; msg: string; bestValue?: number }
    if (!winnerTest || winnerTest.trades < 30) {
      verdict = { kind: 'reject', msg: 'TEST tiene <30 trades — muestra insuficiente' }
    } else if (delta > 5 && ratio > 0.4) {
      verdict = {
        kind: 'accept',
        msg: `Aplicar ${optParam}=${best.value}. Mejora OOS: ${delta.toFixed(2)}pts vs baseline. Generalización ${(ratio * 100).toFixed(0)}%.`,
        bestValue: best.value,
      }
    } else if (delta > 0 && ratio > 0.4) {
      verdict = {
        kind: 'marginal',
        msg: `Mejora marginal de ${delta.toFixed(2)}pts OOS. Decisión tuya.`,
        bestValue: best.value,
      }
    } else if (ratio < 0.3 && Math.abs(trainRet) > 20) {
      verdict = {
        kind: 'reject',
        msg: `OVERFIT: TRAIN +${trainRet.toFixed(0)}% no se transfiere a TEST (${oosRet.toFixed(2)}%). Ratio ${ratio.toFixed(2)}.`,
      }
    } else {
      verdict = { kind: 'reject', msg: 'No hay mejora significativa OOS' }
    }

    setOptResults(rows)
    setOptVerdict(verdict)
    setOptProgress(null)
  }

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex items-center gap-3 mb-6">
        <FlaskConical size={20} style={{ color: 'var(--color-accent)' }} />
        <div className="flex-1">
          <h1 className="text-xl font-bold" style={{ color: 'var(--color-accent)' }}>
            Strategy Backtesting
          </h1>
          <p className="text-xs mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
            Run Rhai scripts or strategy-core engines against historical data
          </p>
        </div>
        <button
          onClick={() => setDatasetPanelOpen(v => !v)}
          className="flex items-center gap-1.5 px-3 py-2 rounded text-xs font-semibold"
          style={{
            backgroundColor: datasetPanelOpen ? 'var(--color-accent)' : 'var(--color-surface-2)',
            border: '1px solid var(--color-border)',
            color: datasetPanelOpen ? '#000' : 'var(--color-text)',
          }}
          title="Download Orderbook Archive dataset for backtesting"
        >
          <CloudDownload size={13} />
          Archive Dataset
        </button>
      </div>

      {/* Polymarket historical data sync */}
      <PolyHistoricalSyncPanel
        seriesOptions={allSeries}
        currentSeriesId={config.series_id ?? config.poly_binary_preset}
      />

      {/* Configuration - Horizontal layout */}
      <div
        className="rounded-lg border p-4 mb-4"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <h2 className="text-xs font-semibold uppercase tracking-widest mb-3" style={{ color: 'var(--color-text-muted)' }}>
          Configuration
        </h2>

        <div className="space-y-3">
          {/* Engine Kind selector */}
          <div className="grid grid-cols-1 gap-3">
            <div>
              <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Strategy Engine</label>
              <select
                value={config.kind ?? 'rhai_candle'}
                onChange={(e) => {
                  const k = e.target.value
                  if (k === 'rhai_candle') {
                    setFullConfig({
                      ...config,
                      kind: k,
                      engine_params: undefined,
                    })
                    return
                  }
                  // Engine kinds backtest against a Polymarket recurring series.
                  // Default to btc_5m (matches the live runner's default) and keep
                  // symbol/interval in sync with the series so the synthetic
                  // backtester fetches the right Binance candles for normalization.
                  const seriesId = config.series_id ?? 'btc_5m'
                  const series = allSeries.find(s => s.id === seriesId)
                  const preset = POLY_BINARY_PRESETS.find(p => p.id === seriesId) ?? POLY_BINARY_PRESETS[0]
                  setFullConfig({
                    ...config,
                    kind: k,
                    market_type: 'polymarket_binary',
                    engine_params: defaultEngineParams(k),
                    script: '',
                    series_id: series?.id ?? preset.id,
                    poly_binary_preset: series?.id ?? preset.id,
                    symbol: series?.symbol ?? preset.symbol,
                    interval: series?.cadence ?? preset.defaultInterval,
                    resolution_logic: series?.resolution_logic ?? 'price_up',
                    threshold: series?.threshold ?? undefined,
                    fee_pct: 1.5,
                  })
                }}
                className="w-full rounded px-2 py-2 text-sm"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                }}
              >
                <option value="rhai_candle">Rhai Script (default)</option>
                {ENGINE_KINDS.map((e) => (
                  <option key={e.id} value={e.id}>{engineKindOptionLabel(e.id)}</option>
                ))}
              </select>
              {isEngineKind && <EngineKindInfoCard kind={config.kind ?? ''} />}
              {isEngineKind && (
                <p className="text-[10px] mt-1.5" style={{ color: 'var(--color-text-muted)' }}>
                  {config.market_type === 'clob_1hz'
                    ? 'Engine replays the recorded YES/NO order-book ticks for the selected slug — no Rhai script needed.'
                    : config.market_type === 'archive_candles'
                    ? 'Engine aggregates archive ticks into 1m OHLC candles and replays them — no Rhai script needed.'
                    : 'Engine simulates the selected Polymarket series using Binance candles as the underlying signal — no Rhai script needed.'}
                </p>
              )}
            </div>
          </div>

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
                } else if (newType === 'clob_1hz' || newType === 'archive_candles') {
                  // Archive modes use recorded tick JSONL slugs.
                  // clob_1hz → on_tick(ctx) scripts
                  // archive_candles → on_candle(ctx) scripts (aggregated to 1m OHLC)
                  // CRITICAL: symbol must equal the slug (not "BTCUSDT") because the backend
                  // reads ticks from data/ticks/<symbol>/. Default to series_id or btc_5m.
                  const slug = config.clob_slug ?? config.series_id ?? 'btc_5m'
                  setFullConfig({
                    ...config,
                    market_type: newType,
                    interval: '5m',
                    fee_pct: 1.5,
                    clob_slug: slug,
                    symbol: slug,
                  })
                  refetchTickSlugs()
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
              <option value="clob_1hz">Orderbook Archive (on_tick)</option>
              <option value="archive_candles">Orderbook Archive (on_candle)</option>
            </select>
          </div>

          {/* Script select — hidden for engine kinds */}
          {!isEngineKind && (
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
          )}

          {/* Market Series + Threshold — shown for engine kinds. We reuse the
              recurring-series picker (the same data the live runner uses) so
              the backend can resolve `series_id` → current Polymarket window
              slug. CLOB 1 HZ mode uses the recorded-tick slug picker rendered
              below instead. */}
          {isEngineKind && (
            <>
              {config.market_type !== 'clob_1hz' && (
                <div className="col-span-2 lg:col-span-3">
                  <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
                    Market Series
                  </label>
                  <select
                    value={config.series_id ?? 'btc_5m'}
                    onChange={(e) => {
                      const sid = e.target.value
                      const s = allSeries.find(x => x.id === sid)
                      const preset = POLY_BINARY_PRESETS.find(p => p.id === sid) ?? POLY_BINARY_PRESETS[0]
                      setFullConfig({
                        ...config,
                        series_id: sid,
                        poly_binary_preset: sid,
                        symbol: s?.symbol ?? preset.symbol,
                        interval: s?.cadence ?? preset.defaultInterval,
                        resolution_logic: s?.resolution_logic ?? 'price_up',
                        threshold: s?.threshold ?? undefined,
                      })
                    }}
                    className="w-full rounded px-2 py-2 text-sm"
                    style={{
                      backgroundColor: 'var(--color-surface-2)',
                      border: '1px solid var(--color-border)',
                      color: 'var(--color-text)',
                    }}
                  >
                    {(allSeries.length ? allSeries : POLY_BINARY_PRESETS).map(s => (
                      <option key={s.id} value={s.id}>{s.label}</option>
                    ))}
                  </select>
                  <p className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
                    Backtest resolves to the current Polymarket window slug
                    (e.g. <span className="font-mono">btc-updown-5m-&lt;ts&gt;</span>) — same as the live runner.
                  </p>
                </div>
              )}
              <div className="lg:col-span-2">
                <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
                  Threshold / Edge
                </label>
                <input
                  type="number"
                  step="0.001"
                  value={config.threshold ?? ''}
                  onChange={(e) => set('threshold', e.target.value === '' ? undefined : Number(e.target.value))}
                  placeholder="default"
                  className="w-full rounded px-3 py-2 text-sm"
                  style={{
                    backgroundColor: 'var(--color-surface-2)',
                    border: '1px solid var(--color-border)',
                    color: 'var(--color-text)',
                  }}
                />
              </div>
            </>
          )}

          {/* Engine-specific parameter panel */}
          {isEngineKind && (
            <div className="col-span-2 sm:col-span-4 lg:col-span-12">
              <EngineParamsForm
                kind={config.kind ?? ''}
                params={config.engine_params ?? defaultEngineParams(config.kind ?? '')}
                onChange={(p) => set('engine_params', p)}
              />
            </div>
          )}

          {/* Symbol / Market selector — adapts to market type. For archive modes we
              always show the recorded-tick slug picker (Rhai on_tick/on_candle scripts
              and engine kinds both consume the same recorded ticks). */}
          {isArchiveMode ? (
            <div className="col-span-2 lg:col-span-4">
              <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Tick Slug</label>
              {tickSlugs.length === 0 ? (
                <div
                  className="rounded px-3 py-2 text-xs space-y-2"
                  style={{
                    backgroundColor: 'var(--color-surface-2)',
                    border: '1px solid var(--color-border)',
                    color: 'var(--color-text-muted)',
                  }}
                >
                  <div>
                    <span className="font-semibold" style={{ color: 'var(--color-text)' }}>No tick data yet.</span>{' '}
                    Orderbook Archive backtests replay 1-second snapshots from <code className="text-[10px]" style={{ color: 'var(--color-accent)' }}>data/ticks/&lt;slug&gt;/</code>.
                    Use the <span className="font-semibold">Download Dataset</span> panel below to ingest archive data, or pick a series and click <span className="font-semibold">Start recorder</span> to record live ticks.
                  </div>
                  <div className="flex items-center gap-2 flex-wrap">
                    <select
                      defaultValue="btc_5m"
                      id="clob1hz-bootstrap-series"
                      className="rounded px-2 py-1 text-xs"
                      style={{ backgroundColor: 'var(--color-base)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                    >
                      {(allSeries.length ? allSeries : POLY_BINARY_PRESETS).map(s => (
                        <option key={s.id} value={s.id}>{s.label}</option>
                      ))}
                    </select>
                    <button
                      type="button"
                      disabled={startTickRecorderMutation.isPending}
                      onClick={() => {
                        const sel = document.getElementById('clob1hz-bootstrap-series') as HTMLSelectElement | null
                        const seriesId = sel?.value || 'btc_5m'
                        startTickRecorderMutation.mutate(seriesId)
                      }}
                      className="px-3 py-1 rounded text-xs font-semibold disabled:opacity-50"
                      style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
                    >
                      {startTickRecorderMutation.isPending ? 'Starting…' : 'Start recorder'}
                    </button>
                    {startTickRecorderMutation.isSuccess && (
                      <span style={{ color: 'var(--color-accent)' }}>
                        Recorder started — slug will appear here once enough ticks accumulate.
                      </span>
                    )}
                    {startTickRecorderMutation.isError && (
                      <span style={{ color: 'var(--color-danger)' }}>
                        {(startTickRecorderMutation.error as Error)?.message ?? 'Failed to start recorder.'}
                      </span>
                    )}
                  </div>
                </div>
              ) : (
                <select
                  value={config.clob_slug ?? config.symbol ?? ''}
                  onChange={(e) => {
                    const slug = e.target.value
                    const info = tickSlugs.find(s => s.slug === slug)
                    setFullConfig({
                      ...config,
                      clob_slug: slug,
                      symbol: slug,
                      from_date: info?.from_date ?? config.from_date,
                      to_date: info?.to_date ?? config.to_date,
                    })
                  }}
                  className="w-full rounded px-2 py-2 text-sm font-mono"
                  style={{
                    backgroundColor: 'var(--color-surface-2)',
                    border: '1px solid var(--color-border)',
                    color: 'var(--color-text)',
                  }}
                >
                  <option value="">Select a slug…</option>
                  {tickSlugs.map(s => (
                    <option key={s.slug} value={s.slug}>
                      {s.slug} — {s.tick_count.toLocaleString()} ticks ({s.from_date} → {s.to_date})
                    </option>
                  ))}
                </select>
              )}
              {config.clob_slug && tickSlugs.find(s => s.slug === config.clob_slug) && (
                <p className="text-[10px] mt-1" style={{ color: 'var(--color-text-muted)' }}>
                  {tickSlugs.find(s => s.slug === config.clob_slug)!.tick_count.toLocaleString()} ticks recorded
                  · {tickSlugs.find(s => s.slug === config.clob_slug)!.dates.length} day(s)
                  {isEngineKind
                    ? <> · Engine receives recorded YES/NO order book ticks</>
                    : <> · Script must use <code style={{ color: 'var(--color-accent)' }}>on_tick(ctx)</code></>}
                </p>
              )}
            </div>
          ) : !isEngineKind && config.market_type === 'crypto' ? (
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

          {/* Interval / Window — hidden for engine kinds and archive tick modes */}
          {!isEngineKind && !isArchiveMode && <div className={config.market_type === 'crypto' ? 'lg:col-span-3' : 'lg:col-span-2'}>
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
          </div>}

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

          {/* Max Entry Price — skip trades above this price/token price */}
          <div className="lg:col-span-2">
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
              Max Entry Price
              <span
                className="ml-1 px-1 rounded text-[9px]"
                style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}
                title={config.market_type === 'polymarket_binary' ? 'Skip bets when token price exceeds this threshold' : 'Skip buys when market price exceeds this threshold'}
              >
                skip above
              </span>
            </label>
            <input
              type="number"
              min={0}
              step={0.01}
              value={config.max_entry_price ?? ''}
              onChange={(e) => {
                const val = e.target.value
                set('max_entry_price', val === '' ? undefined : Number(val))
              }}
              placeholder="No limit"
              className="w-full rounded px-2 py-2 text-sm font-mono"
              style={{
                backgroundColor: 'var(--color-surface-2)',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text)',
              }}
            />
          </div>

          {/* Price Mode (Polymarket binary only) */}
          {config.market_type === 'polymarket_binary' && (
            <div className="lg:col-span-2">
              <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
                Entry Price Mode
              </label>
              <select
                value={config.price_mode ?? 'historical'}
                onChange={(e) => set('price_mode', e.target.value as 'historical' | 'mid')}
                className="w-full rounded px-2 py-2 text-sm"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                }}
              >
                <option value="historical">Histórico real (CLOB)</option>
                <option value="mid">Precio medio (bid/ask)</option>
              </select>
            </div>
          )}

          {/* Hour Gate (Polymarket binary only) */}
          {(config.market_type === 'polymarket_binary' || config.market_type === 'archive_candles') && (
            <div className="lg:col-span-4">
              <label className="block text-xs mb-1.5 flex items-center gap-1.5" style={{ color: 'var(--color-text-muted)' }}>
                Hour Gate (UTC)
                <span
                  className="px-1 rounded text-[9px]"
                  style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}
                  title="Skip windows outside these UTC hours. Empirically, hours 01/03/04/07/08/14/17/20 show ~34% WR."
                >hot hours only</span>
              </label>
              <div className="flex flex-wrap gap-1 mb-1.5">
                {Array.from({ length: 24 }, (_, h) => {
                  const active = (config.allowed_hours ?? []).includes(h)
                  return (
                    <button
                      key={h}
                      type="button"
                      onClick={() => {
                        const cur = config.allowed_hours ?? []
                        const next = active ? cur.filter(x => x !== h) : [...cur, h].sort((a, b) => a - b)
                        set('allowed_hours', next)
                      }}
                      className="w-8 h-7 rounded text-[11px] font-mono transition-colors"
                      style={{
                        background: active ? '#059669' : 'var(--color-surface-2)',
                        color: active ? '#fff' : 'var(--color-text-muted)',
                        border: `1px solid ${active ? '#059669' : 'var(--color-border)'}`,
                      }}
                    >{String(h).padStart(2, '0')}</button>
                  )
                })}
              </div>
              <div className="flex gap-2 text-[11px]" style={{ color: 'var(--color-text-muted)' }}>
                <button
                  type="button"
                  className="underline"
                  onClick={() => set('allowed_hours', [0, 1, 6, 18, 21, 23])}
                >Preset: hot hours</button>
                <button
                  type="button"
                  className="underline"
                  onClick={() => set('allowed_hours', [])}
                >Clear (24/7)</button>
                <span>
                  {(config.allowed_hours ?? []).length > 0
                    ? `Active: ${(config.allowed_hours ?? []).join(', ')} UTC — ${(config.allowed_hours ?? []).length * (60 / 5)} windows/day max`
                    : 'No restriction — trades all 24 hours'}
                </span>
              </div>
            </div>
          )}

          {/* RV Floor (Polymarket binary + archive_candles) */}
          {(config.market_type === 'polymarket_binary' || config.market_type === 'archive_candles') && (
            <div className="lg:col-span-2">
              <label className="block text-xs mb-1.5 flex items-center gap-1.5" style={{ color: 'var(--color-text-muted)' }}>
                BTC RV Floor
                <span
                  className="px-1 rounded text-[9px]"
                  style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}
                  title="Skip windows when BTC 60-period realized vol is below this value. Flat markets degrade drift signal."
                >flat-mkt filter</span>
              </label>
              <input
                type="number"
                min={0}
                step={0.000005}
                value={config.rv_min_btc ?? ''}
                onChange={(e) => {
                  const val = e.target.value
                  set('rv_min_btc', val === '' ? undefined : Number(val))
                }}
                placeholder="0.00015 (empirical)"
                className="w-full rounded px-2 py-2 text-sm font-mono"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                }}
              />
              <div className="text-[11px] mt-1" style={{ color: 'var(--color-text-muted)' }}>
                {config.rv_min_btc
                  ? `Skip when RV < ${config.rv_min_btc.toFixed(5)} — filters flat consolidation`
                  : 'Disabled — no RV filter applied'}
              </div>
            </div>
          )}

          {/* Spread Guard (archive_candles only — real CLOB bid/ask required) */}
          {config.market_type === 'archive_candles' && (
            <div className="lg:col-span-2">
              <label className="block text-xs mb-1.5 flex items-center gap-1.5" style={{ color: 'var(--color-text-muted)' }}>
                Spread Guard
                <span
                  className="px-1 rounded text-[9px]"
                  style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}
                  title="Skip windows where CLOB spread (yes_ask − yes_bid) > threshold at decision time. Mirrors live runner max_spread_pct. Default 3%."
                >live parity</span>
              </label>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  min={0.005}
                  max={0.5}
                  step={0.005}
                  value={config.max_spread_pct != null ? (config.max_spread_pct * 100).toFixed(1) : ''}
                  onChange={(e) => {
                    const val = e.target.value
                    set('max_spread_pct', val === '' ? undefined : Number(val) / 100)
                  }}
                  placeholder="3.0 (default)"
                  className="flex-1 rounded px-2 py-2 text-sm font-mono"
                  style={{
                    backgroundColor: 'var(--color-surface-2)',
                    border: '1px solid var(--color-border)',
                    color: 'var(--color-text)',
                  }}
                />
                <span className="text-sm" style={{ color: 'var(--color-text-muted)' }}>%</span>
                {config.max_spread_pct != null && (
                  <button
                    onClick={() => set('max_spread_pct', undefined)}
                    className="text-xs px-2 py-1 rounded"
                    style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}
                  >reset</button>
                )}
              </div>
              <div className="text-[11px] mt-1" style={{ color: 'var(--color-text-muted)' }}>
                {config.max_spread_pct != null
                  ? `Skip windows with spread > ${(config.max_spread_pct * 100).toFixed(1)}% — matches live runner gate`
                  : 'Default 3% — matches live runner default (max_spread_pct = 0.03)'}
              </div>
            </div>
          )}

          {/* Sizing Mode */}
          <div className="lg:col-span-2">
            <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
              Sizing Mode
            </label>
            <select
              value={config.sizing_mode ?? 'percent'}
              onChange={(e) => {
                const mode = e.target.value as 'fixed' | 'percent'
                set('sizing_mode', mode)
                // Default to 5% on percent (backend expects 0-100 percent, NOT fraction)
                if (mode === 'percent' && (config.sizing_value == null || config.sizing_value < 0.5 || config.sizing_value > 100)) {
                  set('sizing_value', 5)
                }
              }}
              className="w-full rounded px-2 py-2 text-sm"
              style={{
                backgroundColor: 'var(--color-surface-2)',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text)',
              }}
            >
              <option value="percent">% of Balance</option>
              <option value="fixed">Fixed USD</option>
            </select>
          </div>

          {/* Sizing Value */}
          {(() => {
            const isPercent = (config.sizing_mode ?? 'percent') === 'percent'
            // Backend convention: percent mode stores 0-100 (e.g. 5 = 5%) — same as live runner.
            // Migrate legacy fractional values (≤ 1.5 in percent mode is assumed to be 0-1 fraction).
            const sv = config.sizing_value ?? (isPercent ? 5 : 100)
            const normalizedPct = isPercent && sv > 0 && sv <= 1.5 ? sv * 100 : sv
            const displayVal = isPercent ? normalizedPct : sv
            const maxPct = isPercent && config.max_position_usd && config.initial_balance > 0
              ? Math.min(100, (config.max_position_usd / config.initial_balance) * 100)
              : 100
            const effectiveDollar = isPercent
              ? Math.min(
                  config.initial_balance * (normalizedPct / 100),
                  config.max_position_usd ?? Infinity
                )
              : sv
            const exceedsMax = isPercent && config.max_position_usd != null
              && config.initial_balance * (normalizedPct / 100) > config.max_position_usd

            return (
              <div className="lg:col-span-2">
                <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
                  {isPercent ? 'Max % of Balance' : 'Amount ($)'}
                </label>
                <input
                  type="number"
                  min={isPercent ? 0.1 : 1}
                  max={isPercent ? maxPct : undefined}
                  step={isPercent ? 0.1 : 1}
                  value={displayVal}
                  onChange={(e) => {
                    const v = Number(e.target.value)
                    // Store directly as percent (0-100). Backend expects this.
                    set('sizing_value', isPercent ? Math.min(v, maxPct) : v)
                  }}
                  className="w-full rounded px-2 py-2 text-sm font-mono"
                  style={{
                    backgroundColor: 'var(--color-surface-2)',
                    border: `1px solid ${exceedsMax ? 'var(--color-danger, #ef4444)' : 'var(--color-border)'}`,
                    color: 'var(--color-text)',
                  }}
                />
                <p className="text-[10px] mt-0.5" style={{ color: exceedsMax ? 'var(--color-danger, #ef4444)' : 'var(--color-text-muted)' }}>
                  {isPercent
                    ? exceedsMax
                      ? `Exceeds Max Pos — capped at $${config.max_position_usd?.toFixed(0)}`
                      : effectiveDollar === Infinity
                        ? 'Fraction of balance per trade'
                        : `≈ $${effectiveDollar.toFixed(0)} per trade (capped at Max Pos)`
                    : 'Fixed USDC amount per trade'}
                </p>
              </div>
            )
          })()}

          {/* ── Guardrails (mirrors live runner risk controls) ─────────────────── */}
          {(config.market_type === 'polymarket_binary' || config.market_type === 'archive_candles') && (
            <div className="lg:col-span-4 border-t pt-3" style={{ borderColor: 'var(--color-border)' }}>
              <div className="text-xs font-semibold mb-2" style={{ color: 'var(--color-warning)' }}>
                Guardrails (live parity)
              </div>
              <div className="grid grid-cols-2 lg:grid-cols-4 gap-3">
                <div>
                  <label className="block text-[11px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Kelly Cap</label>
                  <input type="number" min={1.0} max={3.0} step={0.1} placeholder="1.5"
                    value={(config as any).kelly_size_cap ?? ''}
                    onChange={e => set('kelly_size_cap', e.target.value === '' ? undefined : Number(e.target.value))}
                    className="w-full rounded px-2 py-1.5 text-xs font-mono"
                    style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }} />
                  <div className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>max kelly × (default 1.5)</div>
                </div>
                <div>
                  <label className="block text-[11px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Min Entry ¢</label>
                  <input type="number" min={1} max={50} step={1} placeholder="5"
                    value={(config as any).min_entry_price != null ? Math.round((config as any).min_entry_price * 100) : ''}
                    onChange={e => {
                      const val = Number(e.target.value)
                      set('min_entry_price', e.target.value === '' ? undefined : val / 100)
                    }}
                    className="w-full rounded px-2 py-1.5 text-xs font-mono"
                    style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }} />
                  <div className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>skip bets &lt; N¢ (default 5¢)</div>
                </div>
                <div>
                  <label className="block text-[11px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Max Loss Streak</label>
                  <input type="number" min={0} max={100} step={1} placeholder="off"
                    value={(config as any).max_consecutive_losses ?? ''}
                    onChange={e => set('max_consecutive_losses', e.target.value === '' ? undefined : Number(e.target.value))}
                    className="w-full rounded px-2 py-1.5 text-xs font-mono"
                    style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }} />
                  <div className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>stop sim after N losses</div>
                </div>
                <div>
                  <label className="block text-[11px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Stop Loss %</label>
                  <input type="number" min={5} max={90} step={5} placeholder="off"
                    value={(config as any).stop_loss_pct != null ? Math.round((config as any).stop_loss_pct * 100) : ''}
                    onChange={e => {
                      const val = Number(e.target.value)
                      set('stop_loss_pct', e.target.value === '' ? undefined : val / 100)
                    }}
                    className="w-full rounded px-2 py-1.5 text-xs font-mono"
                    style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }} />
                  <div className="text-[10px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>exit early if token drops N%</div>
                </div>
              </div>
            </div>
          )}

          {/* Run + Live Trading buttons */}
          <div className={clsx('col-span-2 flex gap-2', config.market_type !== 'polymarket_binary' && 'lg:col-span-2')}>
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
              <>
                <button
                  onClick={() => setShowOptimizer(v => !v)}
                  className="flex items-center justify-center gap-1.5 px-3 py-2.5 rounded font-semibold text-sm whitespace-nowrap"
                  style={{
                    backgroundColor: showOptimizer ? 'var(--color-warning)' : 'var(--color-surface-2)',
                    border: '1px solid var(--color-border)',
                    color: showOptimizer ? '#000' : 'var(--color-text)',
                  }}
                  title="Parameter sweep with TRAIN/TEST split (out-of-sample validation)"
                >
                  <FlaskConical size={14} style={{ color: showOptimizer ? '#000' : 'var(--color-warning)' }} />
                  Optimize
                </button>
                <button
                  onClick={() => setShowLiveModal(true)}
                  className="flex items-center justify-center gap-1.5 px-3 py-2.5 rounded font-semibold text-sm whitespace-nowrap"
                  style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                  title="Launch this strategy in Live Trading"
                >
                  <Zap size={14} style={{ color: 'var(--color-accent)' }} />
                  Live
                </button>
              </>
            )}
          </div>

          {/* ── OPTIMIZER PANEL ──────────────────────────────────────────── */}
          {showOptimizer && !isBatchMode && config.script && (
            <div className="col-span-2 lg:col-span-4 rounded border p-3 mt-2"
              style={{ backgroundColor: 'var(--color-surface-2)', borderColor: 'var(--color-warning)' }}>
              <div className="flex items-center gap-2 mb-2">
                <FlaskConical size={14} style={{ color: 'var(--color-warning)' }} />
                <span className="text-sm font-semibold">Parameter Optimizer (TRAIN/TEST split 70/30)</span>
                <span className="text-[10px] px-1.5 py-0.5 rounded" style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)' }}>
                  out-of-sample validation
                </span>
              </div>
              <div className="flex flex-wrap items-end gap-3 mb-3">
                <div>
                  <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Parámetro</label>
                  <select
                    value={optParam}
                    onChange={e => {
                      const p = e.target.value as OptParam
                      setOptParam(p)
                      // Suggest sensible default grid per param
                      if (p === 'min_entry_price') setOptGrid('0.10,0.15,0.20,0.25,0.30')
                      else if (p === 'sizing_value') setOptGrid('2,3,5,7,10')
                      else if (p === 'max_spread_pct') setOptGrid('0.02,0.03,0.04,0.05,0.06')
                      else if (p === 'kelly_size_cap') setOptGrid('1.0,1.25,1.5,2.0,2.5')
                    }}
                    className="rounded border px-2 py-1.5 text-xs"
                    style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                  >
                    <option value="min_entry_price">min_entry_price</option>
                    <option value="sizing_value">sizing_value (%)</option>
                    <option value="max_spread_pct">max_spread_pct</option>
                    <option value="kelly_size_cap">kelly_size_cap</option>
                  </select>
                </div>
                <div className="flex-1 min-w-[200px]">
                  <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>
                    Grid (valores separados por coma)
                  </label>
                  <input
                    value={optGrid}
                    onChange={e => setOptGrid(e.target.value)}
                    placeholder="0.10,0.15,0.20,0.25,0.30"
                    className="w-full rounded border px-2 py-1.5 text-xs font-mono"
                    style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
                  />
                </div>
                <button
                  onClick={() => runOptimization()}
                  disabled={isRunning || !!optProgress}
                  className="flex items-center gap-1.5 px-4 py-2 rounded font-semibold text-xs disabled:opacity-40"
                  style={{ backgroundColor: 'var(--color-warning)', color: '#000' }}
                >
                  {optProgress ? (
                    <>
                      <RefreshCw size={12} className="animate-spin" />
                      {optProgress.phase} ({optProgress.current}/{optProgress.total})
                    </>
                  ) : (
                    <>
                      <Play size={12} />
                      Run Sweep
                    </>
                  )}
                </button>
              </div>

              {/* Results table */}
              {optResults && (
                <div className="rounded border overflow-hidden mt-2" style={{ borderColor: 'var(--color-border)' }}>
                  <table className="w-full text-xs">
                    <thead style={{ backgroundColor: 'var(--color-base)' }}>
                      <tr>
                        <th className="text-left px-2 py-1.5">Config</th>
                        <th className="text-right px-2 py-1.5" colSpan={3}>TRAIN (70%)</th>
                        <th className="text-right px-2 py-1.5" colSpan={3}>TEST OOS (30%)</th>
                      </tr>
                      <tr style={{ color: 'var(--color-text-muted)' }}>
                        <th className="text-left px-2 py-1 text-[10px]"></th>
                        <th className="text-right px-2 py-1 text-[10px]">Trades</th>
                        <th className="text-right px-2 py-1 text-[10px]">Ret</th>
                        <th className="text-right px-2 py-1 text-[10px]">Sharpe</th>
                        <th className="text-right px-2 py-1 text-[10px]">Trades</th>
                        <th className="text-right px-2 py-1 text-[10px]">Ret</th>
                        <th className="text-right px-2 py-1 text-[10px]">Sharpe</th>
                      </tr>
                    </thead>
                    <tbody>
                      {optResults.map((row, i) => (
                        <tr key={i} style={{
                          backgroundColor: row.isBaseline ? 'rgba(129,140,248,0.08)' : row.isWinner ? 'rgba(34,197,94,0.10)' : undefined,
                          borderTop: '1px solid var(--color-border)',
                        }}>
                          <td className="px-2 py-1 font-mono text-[11px]">
                            {row.isBaseline && '◆ '}{row.isWinner && '★ '}{row.label}
                          </td>
                          <td className="px-2 py-1 text-right">{row.train?.trades ?? '—'}</td>
                          <td className="px-2 py-1 text-right" style={{ color: (row.train?.ret ?? 0) >= 0 ? 'var(--color-accent)' : 'var(--color-danger)' }}>
                            {row.train ? `${row.train.ret >= 0 ? '+' : ''}${row.train.ret.toFixed(1)}%` : '—'}
                          </td>
                          <td className="px-2 py-1 text-right">{row.train?.sharpe.toFixed(2) ?? '—'}</td>
                          <td className="px-2 py-1 text-right">{row.test?.trades ?? '—'}</td>
                          <td className="px-2 py-1 text-right" style={{ color: (row.test?.ret ?? 0) >= 0 ? 'var(--color-accent)' : 'var(--color-danger)' }}>
                            {row.test ? `${row.test.ret >= 0 ? '+' : ''}${row.test.ret.toFixed(1)}%` : '—'}
                          </td>
                          <td className="px-2 py-1 text-right">{row.test?.sharpe.toFixed(2) ?? '—'}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}

              {/* Verdict */}
              {optVerdict && (
                <div className="rounded border p-3 mt-3 text-xs"
                  style={{
                    backgroundColor: optVerdict.kind === 'accept' ? 'rgba(34,197,94,0.10)' : optVerdict.kind === 'marginal' ? 'rgba(245,158,11,0.10)' : 'rgba(239,68,68,0.10)',
                    borderColor: optVerdict.kind === 'accept' ? 'var(--color-accent)' : optVerdict.kind === 'marginal' ? 'var(--color-warning)' : 'var(--color-danger)',
                  }}>
                  <div className="font-semibold mb-1" style={{
                    color: optVerdict.kind === 'accept' ? 'var(--color-accent)' : optVerdict.kind === 'marginal' ? 'var(--color-warning)' : 'var(--color-danger)',
                  }}>
                    {optVerdict.kind === 'accept' ? '✓ ACCEPT' : optVerdict.kind === 'marginal' ? '~ MARGINAL' : '✗ REJECT'}
                  </div>
                  <div style={{ color: 'var(--color-text)' }}>{optVerdict.msg}</div>
                  {optVerdict.kind === 'accept' && optVerdict.bestValue !== undefined && (
                    <div className="mt-2 pt-2 border-t text-[11px]" style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
                      Para aplicar a un runner live, ve a Live Strategies → editar el runner → cambiar <span className="font-mono">{optParam}</span> a <span className="font-mono font-semibold">{optVerdict.bestValue}</span>.
                    </div>
                  )}
                </div>
              )}
            </div>
          )}
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

      {/* Dataset Download Panel — shown for archive modes or when manually opened */}
      {(isArchiveMode || datasetPanelOpen) && (
        <div
          className="rounded-lg border mb-4 overflow-hidden"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <button
            onClick={() => setDatasetPanelOpen(v => !v)}
            className="w-full flex items-center justify-between px-4 py-3 text-sm font-semibold hover:bg-[var(--color-surface-2)]"
            style={{ color: 'var(--color-text)' }}
          >
            <span className="flex items-center gap-2">
              <CloudDownload size={15} style={{ color: 'var(--color-accent)' }} />
              Download Orderbook Archive Dataset
            </span>
            <span className="text-xs font-normal" style={{ color: 'var(--color-text-muted)' }}>
              {ingestProgress?.running
                ? `${ingestProgress.phase === 'downloading' ? '⬇ Downloading' : '⚙ Converting'}…`
                : ingestProgress?.finished_at
                ? `Last run: ${ingestProgress.downloaded ?? 0} files downloaded`
                : 'pmxt.dev v2 archive → tick JSONL'}
            </span>
          </button>

          <div className="px-4 pb-4 space-y-4">
            <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              Downloads hourly Parquet files from the <strong>pmxt.dev v2</strong> Polymarket CLOB archive,
              then converts them to <code className="font-mono">data/ticks/&lt;slug&gt;/</code> JSONL format
              ready for backtesting. Use <em>Orderbook Archive (on_tick)</em> or <em>Orderbook Archive (on_candle)</em> modes.
            </p>

            <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
              <div>
                <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>Days to download</label>
                <input
                  type="number" min={1} max={30}
                  value={ingestDays}
                  onChange={e => setIngestDays(Math.max(1, Math.min(30, Number(e.target.value))))}
                  className="w-full rounded px-2 py-2 text-sm"
                  style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                />
              </div>
              <div className="col-span-2 sm:col-span-1">
                <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>Condition ID (0x...)</label>
                <input
                  type="text"
                  value={ingestMarket}
                  onChange={e => setIngestMarket(e.target.value)}
                  placeholder="0x13dec97d..."
                  className="w-full rounded px-2 py-2 text-sm font-mono"
                  style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                />
              </div>
              <div>
                <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>Tick slug</label>
                <input
                  type="text"
                  value={ingestSlug}
                  onChange={e => setIngestSlug(e.target.value)}
                  placeholder="btc_5m"
                  className="w-full rounded px-2 py-2 text-sm font-mono"
                  style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                />
              </div>
              <div>
                <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>Binance symbol</label>
                <input
                  type="text"
                  value={ingestBinance}
                  onChange={e => setIngestBinance(e.target.value)}
                  placeholder="BTCUSDT"
                  className="w-full rounded px-2 py-2 text-sm font-mono"
                  style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                />
              </div>
            </div>

            {/* Progress bar */}
            {ingestProgress?.running && (
              <div className="space-y-1.5">
                <div className="flex justify-between text-xs" style={{ color: 'var(--color-text-muted)' }}>
                  <span>
                    {ingestProgress.phase === 'downloading'
                      ? `Downloading ${ingestProgress.downloaded ?? 0} files (${ingestProgress.done}/${ingestProgress.total})`
                      : `Converting to ticks/${ingestProgress.slug}…`}
                  </span>
                  {ingestProgress.total > 0 && (
                    <span>{Math.round((ingestProgress.done / ingestProgress.total) * 100)}%</span>
                  )}
                </div>
                <div className="w-full rounded-full h-1.5" style={{ backgroundColor: 'var(--color-surface-2)' }}>
                  <div
                    className="h-1.5 rounded-full transition-all"
                    style={{
                      backgroundColor: 'var(--color-accent)',
                      width: ingestProgress.total > 0
                        ? `${Math.round((ingestProgress.done / ingestProgress.total) * 100)}%`
                        : ingestProgress.phase === 'converting' ? '85%' : '5%',
                    }}
                  />
                </div>
                {ingestProgress.current_hour && (
                  <div className="text-[10px] font-mono truncate" style={{ color: 'var(--color-text-muted)' }}>
                    {ingestProgress.current_hour}
                  </div>
                )}
              </div>
            )}

            {/* Completion / errors */}
            {!ingestProgress?.running && ingestProgress?.finished_at && (
              <div
                className="rounded px-3 py-2 text-xs"
                style={{
                  backgroundColor: ingestProgress.errors?.length ? 'rgba(239,68,68,0.08)' : 'rgba(0,255,136,0.06)',
                  border: `1px solid ${ingestProgress.errors?.length ? 'rgba(239,68,68,0.3)' : 'rgba(0,255,136,0.2)'}`,
                  color: ingestProgress.errors?.length ? '#ef4444' : 'var(--color-accent)',
                }}
              >
                {ingestProgress.errors?.length
                  ? `⚠ Completed with ${ingestProgress.errors.length} error(s): ${ingestProgress.errors.slice(0, 2).join(', ')}`
                  : `✓ Done — ${ingestProgress.downloaded ?? 0} files downloaded, slug '${ingestProgress.slug}' ready for backtesting`}
              </div>
            )}

            <div className="flex gap-2">
              <button
                disabled={ingestMutation.isPending || ingestProgress?.running || !ingestMarket.trim() || !ingestSlug.trim()}
                onClick={() => ingestMutation.mutate()}
                className="flex items-center gap-1.5 px-3 py-2 rounded text-sm font-semibold disabled:opacity-40"
                style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
              >
                <Download size={13} />
                {ingestProgress?.running ? 'Running…' : 'Start Download + Convert'}
              </button>
              {ingestProgress?.running && (
                <button
                  onClick={() => cancelIngestMutation.mutate()}
                  className="px-3 py-2 rounded text-sm"
                  style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                >
                  Cancel
                </button>
              )}
              {ingestProgress?.finished_at && !ingestProgress.running && (
                <button
                  onClick={() => {
                    setFullConfig({ ...config, market_type: 'archive_candles', clob_slug: ingestSlug, symbol: ingestSlug })
                    refetchTickSlugs()
                  }}
                  className="px-3 py-2 rounded text-sm flex items-center gap-1.5"
                  style={{ backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-accent)', color: 'var(--color-accent)' }}
                >
                  <Database size={13} />
                  Use for backtesting
                </button>
              )}
            </div>

            {ingestMutation.isError && (
              <p className="text-xs" style={{ color: '#ef4444' }}>
                {(ingestMutation.error as Error)?.message ?? 'Failed to start ingest.'}
              </p>
            )}
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
                    onClick={() => setSortBy(prev => {
                      const idx = SORT_MODES.indexOf(prev)
                      return SORT_MODES[(idx + 1) % SORT_MODES.length]
                    })}
                    className="flex items-center gap-1 text-[10px] px-2 py-1 rounded hover:bg-[var(--color-surface-2)]"
                    style={{ color: sortBy !== 'default' ? 'var(--color-accent)' : 'var(--color-text-muted)' }}
                    title={sortBy === 'default' ? 'Click to sort' : `Sorted by ${sortBy.replace('_desc', '').replace(/_/g, ' ')}`}
                  >
                    <ArrowUpDown size={10} />
                    {sortBy === 'default' ? 'Sort' : sortBy === 'win_rate_desc' ? 'Win Rate ↓' : sortBy === 'trades_desc' ? 'Trades ↓' : 'Balance ↓'}
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
                <ResultPanel
                  result={displayResult}
                  onRunPaper={() =>
                    navigate('/live', {
                      state: {
                        prefill: {
                          kind: config.kind ?? 'rhai_candle',
                          script: config.script,
                          symbol: config.symbol,
                          market_type: config.market_type,
                          series_id: config.series_id,
                          engine_params: config.engine_params,
                          mode: 'paper',
                        },
                      },
                    })
                  }
                  onRunLive={() =>
                    navigate('/live', {
                      state: {
                        prefill: {
                          kind: config.kind ?? 'rhai_candle',
                          script: config.script,
                          symbol: config.symbol,
                          market_type: config.market_type,
                          series_id: config.series_id,
                          engine_params: config.engine_params,
                          mode: 'live',
                        },
                      },
                    })
                  }
                />
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

// ── Polymarket historical data sync panel ─────────────────────────
//
// Downloads the last N days of real Polymarket token prices (P4 + P3) for
// a chosen recurring series. Lets first-time users populate the
// `data/polymarket_historical/` cache without touching the CLI, so their
// backtests reflect real CLOB pricing instead of the momentum fallback.

interface PolySyncProgress {
  running: boolean
  series_id: string
  from_date: string
  to_date: string
  stage: string
  windows_total: number
  windows_fetched: number
  min4_count: number
  min3_count: number
  error: string | null
  started_at: string | null
  completed_at: string | null
}

interface PolyDatasetSummary {
  series_id: string
  min4_count: number | null
  min3_count: number | null
  last_modified: string | null
}

interface PolySyncStatus {
  progress: PolySyncProgress
  datasets: PolyDatasetSummary[]
}

function fmtRelativeDate(iso: string | null): string {
  if (!iso) return '—'
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return '—'
  const mins = Math.round((Date.now() - d.getTime()) / 60000)
  if (mins < 1) return 'just now'
  if (mins < 60) return `${mins}m ago`
  const hrs = Math.round(mins / 60)
  if (hrs < 48) return `${hrs}h ago`
  return d.toISOString().slice(0, 10)
}

export function PolyHistoricalSyncPanel({ seriesOptions, currentSeriesId }: {
  seriesOptions: MarketSeries[]
  currentSeriesId?: string
}) {
  const [seriesId, setSeriesId] = useState<string>(currentSeriesId ?? 'btc_5m')
  const [daysBack, setDaysBack] = useState<number>(60)
  const [expanded, setExpanded] = useState(false)

  // Keep selector in sync when the Backtesting page switches series.
  useEffect(() => {
    if (currentSeriesId) setSeriesId(currentSeriesId)
  }, [currentSeriesId])

  const { data: status, refetch } = useQuery<PolySyncStatus>({
    queryKey: ['poly-historical-status'],
    queryFn: () => apiFetch('/api/backtest/polymarket-historical/status'),
    refetchInterval: (query) => (query.state.data?.progress.running ? 1500 : false),
  })

  const syncMutation = useMutation({
    mutationFn: () => apiPost('/api/backtest/polymarket-historical/sync', {
      series_id: seriesId,
      days_back: daysBack,
    }),
    onSuccess: () => refetch(),
  })

  const cancelMutation = useMutation({
    mutationFn: () => apiPost('/api/backtest/polymarket-historical/cancel', {}),
    onSuccess: () => refetch(),
  })

  const progress = status?.progress
  const running = progress?.running ?? false
  const dataset = (status?.datasets ?? []).find(d => d.series_id === seriesId)

  const pct = progress && progress.windows_total > 0
    ? Math.min(100, (progress.windows_fetched / progress.windows_total) * 100)
    : 0

  const stageLabel = (() => {
    switch (progress?.stage) {
      case 'min4': return 'Minute-4 prices (decision)'
      case 'min3': return 'Minute-3 prices (drift signal)'
      case 'done': return 'Completed'
      case 'error': return 'Error'
      case 'cancelled': return 'Cancelled'
      default: return 'Idle'
    }
  })()

  // Polymarket series only — weather and other data sources don't have CLOB token prices.
  const pmSeries = seriesOptions.filter(s => s.data_source !== 'open_meteo')

  return (
    <div
      className="rounded-lg border mb-4"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      {/* Header */}
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center justify-between p-3 text-left"
      >
        <div className="flex items-center gap-2">
          <CloudDownload size={16} style={{ color: 'var(--color-accent)' }} />
          <span className="text-sm font-semibold" style={{ color: 'var(--color-text)' }}>
            Historical Polymarket data (CLOB)
          </span>
          {dataset && (dataset.min4_count ?? 0) > 0 && (
            <span
              className="px-1.5 py-0.5 rounded text-[10px] font-bold uppercase"
              style={{ backgroundColor: 'rgba(34, 197, 94, 0.15)', color: '#22c55e' }}
            >
              {(dataset.min4_count ?? 0).toLocaleString()} P4
              {(dataset.min3_count ?? 0) > 0 && ` · ${(dataset.min3_count ?? 0).toLocaleString()} P3`}
            </span>
          )}
          {running && (
            <span
              className="px-1.5 py-0.5 rounded text-[10px] font-bold uppercase animate-pulse"
              style={{ backgroundColor: 'rgba(59, 130, 246, 0.15)', color: '#3b82f6' }}
            >
              Syncing…
            </span>
          )}
        </div>
        {expanded ? <ChevronDown size={16} style={{ color: 'var(--color-text-muted)' }} /> : <ChevronRight size={16} style={{ color: 'var(--color-text-muted)' }} />}
      </button>

      {expanded && (
        <div className="px-3 pb-3 space-y-3">
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            Fetches the last N days of real Polymarket token prices from the CLOB API
            for the selected recurring series. Captures both minute-4 (decision) and
            minute-3 (drift signal) prices. Your backtests will use these prices instead
            of the momentum-model fallback, giving you far more realistic results.
          </p>

          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 items-end">
            <div>
              <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Series</label>
              <select
                value={seriesId}
                onChange={(e) => setSeriesId(e.target.value)}
                disabled={running}
                className="w-full rounded px-2 py-2 text-sm"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                }}
              >
                {pmSeries.length === 0
                  ? <option value="btc_5m">BTC 5m (default)</option>
                  : pmSeries.map(s => <option key={s.id} value={s.id}>{s.label}</option>)
                }
              </select>
            </div>
            <div>
              <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Days back</label>
              <input
                type="number"
                min={1}
                max={365}
                value={daysBack}
                onChange={(e) => setDaysBack(Math.max(1, Math.min(365, parseInt(e.target.value || '60', 10))))}
                disabled={running}
                className="w-full rounded px-2 py-2 text-sm"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  border: '1px solid var(--color-border)',
                  color: 'var(--color-text)',
                }}
              />
            </div>
            <div>
              {running ? (
                <button
                  onClick={() => cancelMutation.mutate()}
                  disabled={cancelMutation.isPending}
                  className="w-full flex items-center justify-center gap-2 rounded px-3 py-2 text-sm font-semibold transition-opacity disabled:opacity-50 disabled:cursor-not-allowed"
                  style={{
                    backgroundColor: 'var(--color-danger, #ef4444)',
                    color: '#fff',
                  }}
                  title="Cancel sync — partial data will still be saved"
                >
                  <X size={14} />
                  {cancelMutation.isPending ? 'Cancelling…' : 'Stop sync'}
                </button>
              ) : (
                <button
                  onClick={() => syncMutation.mutate()}
                  disabled={syncMutation.isPending}
                  className="w-full flex items-center justify-center gap-2 rounded px-3 py-2 text-sm font-semibold transition-opacity disabled:opacity-50 disabled:cursor-not-allowed"
                  style={{
                    backgroundColor: 'var(--color-accent)',
                    color: 'var(--color-bg)',
                  }}
                >
                  <Download size={14} />
                  Cargar datos históricos de Polymarket (CLOB)
                </button>
              )}
            </div>
          </div>

          {/* Progress bar */}
          {running && progress && (
            <div className="space-y-1.5">
              <div className="flex items-center justify-between text-xs" style={{ color: 'var(--color-text-muted)' }}>
                <span>{stageLabel}</span>
                <span>
                  {progress.windows_fetched.toLocaleString()} / {progress.windows_total.toLocaleString()} windows
                  {' · '}
                  {pct.toFixed(1)}%
                </span>
              </div>
              <div
                className="w-full h-1.5 rounded overflow-hidden"
                style={{ backgroundColor: 'var(--color-surface-2)' }}
              >
                <div
                  className="h-full transition-all duration-300"
                  style={{ width: `${pct}%`, backgroundColor: 'var(--color-accent)' }}
                />
              </div>
              <p className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
                Window: {progress.from_date} → {progress.to_date}
              </p>
            </div>
          )}

          {/* Last result / error */}
          {!running && progress?.stage === 'done' && (
            <div
              className="rounded border p-2 text-xs flex items-center gap-2"
              style={{
                backgroundColor: 'rgba(34, 197, 94, 0.08)',
                borderColor: 'rgba(34, 197, 94, 0.35)',
                color: '#22c55e',
              }}
            >
              <Check size={14} />
              Last sync: {progress.min4_count.toLocaleString()} P4 + {progress.min3_count.toLocaleString()} P3 records
              {progress.completed_at && ` · ${fmtRelativeDate(progress.completed_at)}`}
            </div>
          )}
          {!running && progress?.stage === 'cancelled' && (
            <div
              className="rounded border p-2 text-xs flex items-center gap-2"
              style={{
                backgroundColor: 'rgba(245, 158, 11, 0.08)',
                borderColor: 'rgba(245, 158, 11, 0.35)',
                color: '#f59e0b',
              }}
            >
              <X size={14} />
              Cancelled — saved {progress.min4_count.toLocaleString()} P4 + {progress.min3_count.toLocaleString()} P3 records before stopping
              {progress.completed_at && ` · ${fmtRelativeDate(progress.completed_at)}`}
            </div>
          )}
          {!running && progress?.stage === 'error' && progress.error && (
            <div
              className="rounded border p-2 text-xs flex items-center gap-2"
              style={{
                backgroundColor: 'rgba(239, 68, 68, 0.08)',
                borderColor: 'rgba(239, 68, 68, 0.35)',
                color: '#ef4444',
              }}
            >
              <AlertCircle size={14} />
              Sync error: {progress.error}
            </div>
          )}

          {/* Cached datasets summary */}
          {(status?.datasets ?? []).length > 0 && (
            <div className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              <p className="font-semibold uppercase tracking-widest text-[10px] mb-1.5">Cached datasets</p>
              <div className="space-y-1">
                {(status?.datasets ?? []).map(d => (
                  <div key={d.series_id} className="flex items-center justify-between gap-2">
                    <span className="font-mono">{d.series_id}</span>
                    <span>
                      {(d.min4_count ?? 0).toLocaleString()} P4
                      {' · '}
                      {(d.min3_count ?? 0).toLocaleString()} P3
                      {' · '}
                      {fmtRelativeDate(d.last_modified)}
                    </span>
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
