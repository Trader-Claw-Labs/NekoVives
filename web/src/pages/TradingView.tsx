import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import { useNavigate } from 'react-router-dom'
import { useBacktestState } from '../hooks/useBacktestState'
import {
  TrendingUp, TrendingDown, Minus, RefreshCw, AlertCircle, Activity, X, BarChart2, FlaskConical,
} from 'lucide-react'
import clsx from 'clsx'

interface MarketData {
  symbol: string
  price: number
  rsi: number | null
  macd: number | null
  macd_signal: number | null
  change_pct: number | null
}

interface ScreenerResponse {
  data: MarketData[]
  fetched_at: string
}

const DEFAULT_SYMBOLS = ['BTCUSDT', 'ETHUSDT', 'SOLUSDT', 'BNBUSDT', 'XRPUSDT', 'AVAXUSDT']

function rsiLabel(rsi: number | null): { text: string; color: string } {
  if (rsi == null) return { text: '—', color: 'var(--color-text-muted)' }
  if (rsi >= 70) return { text: `${rsi.toFixed(1)} OB`, color: 'var(--color-danger)' }
  if (rsi <= 30) return { text: `${rsi.toFixed(1)} OS`, color: 'var(--color-accent)' }
  return { text: rsi.toFixed(1), color: 'var(--color-text)' }
}

function macdSignal(macd: number | null, signal: number | null): { text: string; color: string } {
  if (macd == null || signal == null) return { text: '—', color: 'var(--color-text-muted)' }
  const cross = macd > signal ? 'Bull' : 'Bear'
  const color = macd > signal ? 'var(--color-accent)' : 'var(--color-danger)'
  return { text: `${macd.toFixed(2)} / ${cross}`, color }
}

// ── TradingView Chart Widget ───────────────────────────────────────────────

function ChartPanel({ symbol, onClose }: { symbol: string; onClose: () => void }) {
  // TradingView widget URL — no API key needed, uses public TradingView embed
  const src = `https://s.tradingview.com/widgetembed/?symbol=BINANCE%3A${encodeURIComponent(symbol)}&interval=1H&hidesidetoolbar=0&symboledit=1&saveimage=0&toolbarbg=000000&studies=RSI%40tv-basicstudies%2FMACD%40tv-basicstudies&theme=dark&style=1&timezone=Etc%2FUTC&withdateranges=1&showpopupbutton=0&locale=en`

  return (
    <div
      className="rounded-lg border overflow-hidden mt-4"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      <div
        className="flex items-center justify-between px-4 py-2.5 border-b"
        style={{ borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center gap-2">
          <BarChart2 size={14} style={{ color: 'var(--color-accent)' }} />
          <span className="text-sm font-bold" style={{ color: 'var(--color-accent)' }}>
            {symbol}
          </span>
          <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>· 1H · RSI · MACD</span>
        </div>
        <button
          onClick={onClose}
          className="p-1 rounded hover:bg-white/10"
          style={{ color: 'var(--color-text-muted)' }}
        >
          <X size={14} />
        </button>
      </div>
      <iframe
        src={src}
        title={`${symbol} chart`}
        width="100%"
        height="480"
        frameBorder="0"
        allowFullScreen
        style={{ display: 'block' }}
      />
    </div>
  )
}

// ── Price Row ─────────────────────────────────────────────────────────────

function PriceRow({ d, onChartClick }: { d: MarketData; onChartClick: (sym: string) => void }) {
  const rsi = rsiLabel(d.rsi)
  const macd = macdSignal(d.macd, d.macd_signal)
  const change = d.change_pct
  const changeUp = change != null && change >= 0

  return (
    <div
      className="grid text-xs items-center border-b"
      style={{
        gridTemplateColumns: '1fr 1fr 80px 110px 80px',
        borderColor: 'var(--color-border)',
        padding: '10px 0',
        color: 'var(--color-text)',
      }}
    >
      <button
        onClick={() => onChartClick(d.symbol)}
        className="flex items-center gap-1.5 group text-left"
        title="View chart"
      >
        <span className="font-bold font-mono" style={{ color: 'var(--color-accent)' }}>
          {d.symbol}
        </span>
        <BarChart2
          size={11}
          className="opacity-0 group-hover:opacity-100 transition-opacity"
          style={{ color: 'var(--color-accent)' }}
        />
      </button>
      <span className="font-mono text-right pr-4">
        ${d.price.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
      </span>
      <span
        className="font-mono text-right pr-4 flex items-center justify-end gap-1"
        style={{ color: change == null ? 'var(--color-text-muted)' : changeUp ? 'var(--color-accent)' : 'var(--color-danger)' }}
      >
        {change != null ? (
          <>
            {changeUp ? <TrendingUp size={11} /> : <TrendingDown size={11} />}
            {changeUp ? '+' : ''}{change.toFixed(2)}%
          </>
        ) : <Minus size={11} />}
      </span>
      <span className="font-mono text-right pr-4" style={{ color: rsi.color }}>
        {rsi.text}
      </span>
      <span className="font-mono text-right" style={{ color: macd.color }}>
        {macd.text}
      </span>
    </div>
  )
}

function SignalBadge({ d }: { d: MarketData }) {
  const signals: string[] = []
  if (d.rsi != null && d.rsi <= 30) signals.push('RSI Oversold')
  if (d.rsi != null && d.rsi >= 70) signals.push('RSI Overbought')
  if (d.macd != null && d.macd_signal != null && d.macd > d.macd_signal) signals.push('MACD Bullish')
  if (d.macd != null && d.macd_signal != null && d.macd < d.macd_signal) signals.push('MACD Bearish')
  if (signals.length === 0) return null

  return (
    <div className="flex flex-wrap gap-1.5 mt-1">
      {signals.map((s) => (
        <span
          key={s}
          className="text-xs px-2 py-0.5 rounded font-mono"
          style={{
            backgroundColor: s.includes('Bull') || s.includes('Oversold')
              ? 'rgba(0,255,136,0.12)'
              : 'rgba(255,68,68,0.12)',
            color: s.includes('Bull') || s.includes('Oversold')
              ? 'var(--color-accent)'
              : 'var(--color-danger)',
          }}
        >
          {s}
        </span>
      ))}
    </div>
  )
}

export default function TradingViewPage() {
  const [symbols, setSymbols] = useState(DEFAULT_SYMBOLS.join(', '))
  const [submitted, setSubmitted] = useState(DEFAULT_SYMBOLS)
  const [chartSymbol, setChartSymbol] = useState<string | null>(null)
  const navigate = useNavigate()
  const { setFullConfig, config: btConfig } = useBacktestState()

  const { data, isLoading, error, refetch, isFetching, dataUpdatedAt } =
    useQuery<ScreenerResponse>({
      queryKey: ['tradingview', submitted],
      queryFn: () =>
        apiFetch(`/api/tradingview/scan?symbols=${submitted.join(',')}`),
      refetchInterval: 60_000,
    })

  function handleApply() {
    const parsed = symbols
      .split(',')
      .map((s) => s.trim().toUpperCase())
      .filter(Boolean)
    if (parsed.length > 0) setSubmitted(parsed)
  }

  function handleChartClick(sym: string) {
    setChartSymbol((prev) => (prev === sym ? null : sym))
  }

  const rows = data?.data ?? []
  const activeSignals = rows.filter(
    (d) =>
      (d.rsi != null && (d.rsi <= 30 || d.rsi >= 70)) ||
      (d.macd != null && d.macd_signal != null && d.macd !== d.macd_signal),
  )

  return (
    <div className="p-6 max-w-5xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-xl font-bold" style={{ color: 'var(--color-accent)' }}>
            TradingView Screener
          </h1>
          <p className="text-xs mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
            Real-time indicators via TradingView Screener API · auto-refresh 60s · click a pair to view chart
          </p>
        </div>
        <button
          onClick={() => refetch()}
          disabled={isFetching}
          className="flex items-center gap-2 px-3 py-1.5 rounded text-xs border transition-opacity disabled:opacity-50"
          style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
        >
          <RefreshCw size={12} className={clsx(isFetching && 'animate-spin')} />
          Refresh
        </button>
      </div>

      {/* Symbol picker */}
      <div
        className="rounded-lg border p-4 mb-4 flex gap-3 items-end"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex-1">
          <label className="block text-xs mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
            Symbols (comma-separated)
          </label>
          <input
            value={symbols}
            onChange={(e) => setSymbols(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && handleApply()}
            placeholder="BTCUSDT, ETHUSDT, SOLUSDT"
            className="w-full rounded px-3 py-2 text-sm font-mono"
            style={{
              backgroundColor: 'var(--color-surface-2)',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text)',
            }}
          />
        </div>
        <button
          onClick={handleApply}
          className="px-4 py-2 rounded text-sm font-semibold"
          style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
        >
          Apply
        </button>
      </div>

      {/* Error */}
      {error && (
        <div
          className="mb-4 px-4 py-3 rounded text-sm border flex items-center gap-2"
          style={{
            backgroundColor: 'rgba(255,68,68,0.1)',
            borderColor: 'var(--color-danger)',
            color: 'var(--color-danger)',
          }}
        >
          <AlertCircle size={14} />
          {String(error)}
        </div>
      )}

      {/* Active signals */}
      {activeSignals.length > 0 && (
        <div
          className="rounded-lg border p-4 mb-4"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <div className="flex items-center gap-2 mb-3">
            <Activity size={13} style={{ color: 'var(--color-accent)' }} />
            <span className="text-xs font-semibold uppercase tracking-widest" style={{ color: 'var(--color-text-muted)' }}>
              Active Signals
            </span>
          </div>
          <div className="space-y-2">
            {activeSignals.map((d) => (
              <div key={d.symbol} className="flex items-center gap-3">
                <button
                  className="text-xs font-mono font-bold hover:underline"
                  style={{ color: 'var(--color-accent)' }}
                  onClick={() => handleChartClick(d.symbol)}
                >
                  {d.symbol}
                </button>
                <SignalBadge d={d} />
                <button
                  className="ml-auto flex items-center gap-1 text-xs px-2 py-0.5 rounded border hover:bg-white/5 transition-colors"
                  style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
                  title="Open in Backtesting"
                  onClick={() => {
                    setFullConfig({ ...btConfig, symbol: d.symbol, market_type: 'crypto' })
                    navigate('/backtesting')
                  }}
                >
                  <FlaskConical size={10} />
                  Backtest
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Table */}
      <div
        className="rounded-lg border overflow-hidden"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        {/* Table header */}
        <div
          className="grid text-xs font-semibold uppercase tracking-widest px-4 py-2 border-b"
          style={{
            gridTemplateColumns: '1fr 1fr 80px 110px 80px',
            borderColor: 'var(--color-border)',
            color: 'var(--color-text-muted)',
            backgroundColor: 'var(--color-surface-2)',
          }}
        >
          <span>Symbol</span>
          <span className="text-right pr-4">Price</span>
          <span className="text-right pr-4">24h %</span>
          <span className="text-right pr-4">RSI</span>
          <span className="text-right">MACD</span>
        </div>

        <div className="px-4">
          {isLoading ? (
            <p className="text-xs py-6 text-center" style={{ color: 'var(--color-text-muted)' }}>
              Loading…
            </p>
          ) : rows.length === 0 ? (
            <p className="text-xs py-6 text-center" style={{ color: 'var(--color-text-muted)' }}>
              No data
            </p>
          ) : (
            rows.map((d) => (
              <PriceRow key={d.symbol} d={d} onChartClick={handleChartClick} />
            ))
          )}
        </div>
      </div>

      {/* Inline chart */}
      {chartSymbol && (
        <ChartPanel symbol={chartSymbol} onClose={() => setChartSymbol(null)} />
      )}

      {dataUpdatedAt > 0 && (
        <p className="text-xs mt-2 text-right" style={{ color: 'var(--color-text-muted)' }}>
          Last updated: {new Date(dataUpdatedAt).toLocaleTimeString()}
        </p>
      )}
    </div>
  )
}
