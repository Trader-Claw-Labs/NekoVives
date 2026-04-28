import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { apiFetch } from '../hooks/useApi'
import {
  Wallet, Zap, Send, Brain, Activity, Clock, TrendingUp, Shield,
  BarChart2, MessageSquare, Settings, ChevronRight, Timer,
  Briefcase, Bot,
} from 'lucide-react'
import clsx from 'clsx'

interface ComponentHealth {
  status: string
  last_ok?: string
  last_error?: string
  restart_count?: number
  updated_at?: string
}

interface StatusData {
  provider?: string
  model?: string
  temperature?: number
  channels?: Record<string, boolean>
  health?: {
    uptime_seconds?: number
    components?: Record<string, ComponentHealth>
  }
  paired?: boolean
}

interface CronData {
  jobs?: { id: string; name: string; enabled: boolean; next_run?: string }[]
  market_scanner?: { enabled: boolean; interval_seconds: number }
}

interface PolymarketConfig {
  configured?: boolean
  wallet_address?: string
}

interface LiveRunner {
  config: { id: string; name: string; mode: string; market_type: string }
  status: { status: string }
  result?: {
    total_trades?: number
    win_rate_pct?: number
    live_total_trades?: number
    live_wins?: number
    live_orders?: { pnl?: number }[]
    all_trades?: { pnl?: number }[]
  }
}

interface LiveListResponse {
  runners: LiveRunner[]
}

interface StatCard {
  label: string
  value: string | number
  icon: React.ReactNode
  status?: 'online' | 'offline' | 'warning'
  sub?: string
}

function Card({ label, value, icon, status, sub }: StatCard) {
  return (
    <div
      className="rounded-lg p-4 border card-hover"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      <div className="flex items-start justify-between mb-3">
        <div
          className="p-2 rounded"
          style={{ backgroundColor: 'var(--color-accent-dim)', color: 'var(--color-accent)' }}
        >
          {icon}
        </div>
        {status && (
          <span className={clsx('status-dot mt-1', status)} />
        )}
      </div>
      <div className="text-2xl font-bold mb-1">{value}</div>
      <div className="text-xs" style={{ color: 'var(--color-text-muted)' }}>{label}</div>
      {sub && (
        <div className="text-xs mt-1 truncate" style={{ color: 'var(--color-accent)' }}>{sub}</div>
      )}
    </div>
  )
}

const quickLinks = [
  { to: '/wallets',        icon: <Wallet size={16} />,       label: 'Web3 Wallets', desc: 'EVM · Solana · TON wallets' },
  { to: '/polymarket',     icon: <BarChart2 size={16} />,    label: 'Polymarket',   desc: 'Prediction market trading' },
  { to: '/telegram',       icon: <Send size={16} />,         label: 'Telegram',     desc: 'Bot commands & alerts' },
  { to: '/skills',         icon: <Zap size={16} />,          label: 'Strategies',   desc: 'Scheduled cron jobs' },
  { to: '/chat',           icon: <MessageSquare size={16} />,label: 'Terminal',     desc: 'Parallel AI trading sessions' },
  { to: '/settings/llm',  icon: <Brain size={16} />,         label: 'LLM',          desc: 'Model & provider config' },
  { to: '/settings/config',icon: <Settings size={16} />,     label: 'Config',       desc: 'Advanced settings' },
]

function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`
  return `${Math.floor(secs / 86400)}d`
}

function formatInterval(secs: number): string {
  if (secs < 60) return `${secs}s`
  if (secs % 60 === 0) return `${secs / 60}m`
  return `${Math.floor(secs / 60)}m ${secs % 60}s`
}

function fmtUSD(v: number): string {
  return v.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })
}

export default function Dashboard() {
  const { data: status, isLoading, error } = useQuery<StatusData>({
    queryKey: ['status'],
    queryFn: () => apiFetch('/api/status'),
    refetchInterval: 10_000,
  })

  const { data: wallets } = useQuery<{ wallets?: unknown[] }>({
    queryKey: ['wallets'],
    queryFn: (): Promise<{ wallets?: unknown[] }> =>
      apiFetch<{ wallets?: unknown[] }>('/api/wallets').catch(() => ({ wallets: [] })),
  })

  const { data: pmConfig } = useQuery<PolymarketConfig>({
    queryKey: ['polymarket-config'],
    queryFn: (): Promise<PolymarketConfig> =>
      apiFetch<PolymarketConfig>('/api/polymarket/configure').catch(() => ({})),
  })

  const { data: cronData } = useQuery<CronData>({
    queryKey: ['skills'],
    queryFn: (): Promise<CronData> =>
      apiFetch<CronData>('/api/cron').catch(() => ({ jobs: [] })),
  })

  const { data: liveData } = useQuery<LiveListResponse>({
    queryKey: ['live-strategies'],
    queryFn: (): Promise<LiveListResponse> =>
      apiFetch<LiveListResponse>('/api/live/strategies').catch(() => ({ runners: [] })),
    refetchInterval: 10_000,
  })

  const uptime = status?.health?.uptime_seconds ?? 0

  // Wallets: Web3 + Polymarket
  const web3WalletCount = wallets?.wallets?.length ?? 0
  const hasPolyWallet = !!(pmConfig?.wallet_address && pmConfig.wallet_address.length > 0)
  const walletCount = web3WalletCount + (hasPolyWallet ? 1 : 0)

  // Live strategies
  const runners = liveData?.runners ?? []
  const liveCount = runners.length
  const runningCount = runners.filter(r => r.status.status === 'running').length

  // Live KPIs (only live mode strategies)
  let liveTrades = 0
  let liveWins = 0
  let livePnl = 0
  runners.forEach(r => {
    if (r.config.mode !== 'live') return
    if (r.config.market_type === 'polymarket_binary' && r.result?.live_orders) {
      liveTrades += r.result.live_total_trades ?? 0
      liveWins += r.result.live_wins ?? 0
      livePnl += r.result.live_orders.reduce((s, o) => s + (o.pnl ?? 0), 0)
    }
  })
  const liveWinRate = liveTrades > 0 ? `${((liveWins / liveTrades) * 100).toFixed(1)}%` : '—'

  // Dry Run KPIs (only paper mode strategies)
  let dryTrades = 0
  let dryWins = 0
  let dryPnl = 0
  runners.forEach(r => {
    if (r.config.mode === 'live') return
    if (r.result?.all_trades) {
      dryTrades += r.result.total_trades ?? 0
      dryWins += Math.round((r.result.win_rate_pct ?? 0) / 100 * (r.result.total_trades ?? 0))
      dryPnl += r.result.all_trades.reduce((s, t) => s + (t.pnl ?? 0), 0)
    }
  })
  const dryWinRate = dryTrades > 0 ? `${((dryWins / dryTrades) * 100).toFixed(1)}%` : '—'

  // Cron jobs
  const jobCount = cronData?.jobs?.length ?? 0
  const scanInterval = cronData?.market_scanner?.interval_seconds

  const cards: StatCard[] = [
    {
      label: 'Connected Wallets',
      value: walletCount,
      icon: <Wallet size={16} />,
      sub: walletCount > 0
        ? `${web3WalletCount} Web3${hasPolyWallet ? ' · 1 Polymarket' : ''}`
        : 'No wallets yet',
    },
    {
      label: 'Active Strategies',
      value: liveCount,
      icon: <Bot size={16} />,
      sub: liveCount > 0 ? `${runningCount} running` : 'No strategies running',
    },
    {
      label: 'Jobs',
      value: jobCount,
      icon: <Briefcase size={16} />,
      sub: jobCount > 0 ? `${jobCount} scheduled` : 'No jobs configured',
    },
    {
      label: 'LLM Model',
      value: status?.model ? status.model.split('/').pop() ?? status.model : '—',
      icon: <Brain size={16} />,
      status: status?.provider ? 'online' : 'offline',
      sub: status?.provider ?? 'No provider',
    },
    {
      label: 'Live KPIs',
      value: liveTrades,
      icon: <TrendingUp size={16} />,
      sub: `${liveWinRate} WR · ${livePnl >= 0 ? '+' : ''}$${fmtUSD(livePnl)} P&L`,
    },
    {
      label: 'Dry Run KPIs',
      value: dryTrades,
      icon: <TrendingUp size={16} />,
      sub: `${dryWinRate} WR · ${dryPnl >= 0 ? '+' : ''}$${fmtUSD(dryPnl)} P&L`,
    },
  ]

  const components = status?.health?.components ?? {}

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="mb-6">
        {/* ASCII Logo */}
        <pre
          className="text-xs leading-tight font-mono mb-3 select-none"
          style={{ color: 'var(--color-accent)', opacity: 0.85 }}
        >{`
  ███╗   ██╗███████╗██╗  ██╗ ██████╗     ██╗   ██╗██╗██╗   ██╗███████╗███████╗
  ████╗  ██║██╔════╝██║ ██╔╝██╔═══██╗    ██║   ██║██║██║   ██║██╔════╝██╔════╝
  ██╔██╗ ██║█████╗  █████╔╝ ██║   ██║    ██║   ██║██║██║   ██║█████╗  ███████╗
  ██║╚██╗██║██╔══╝  ██╔═██╗ ██║   ██║    ╚██╗ ██╔╝██║╚██╗ ██╔╝██╔══╝  ╚════██║
  ██║ ╚████║███████╗██║  ██╗╚██████╔╝     ╚████╔╝ ██║ ╚████╔╝ ███████╗███████║
  ╚═╝  ╚═══╝╚══════╝╚═╝  ╚═╝ ╚═════╝       ╚═══╝  ╚═╝  ╚═══╝  ╚══════╝╚══════╝`}</pre>

        <div className="flex items-center justify-between">
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            VIVE TRADING CAT AGENT
          </p>
          <div className="flex items-center gap-2 text-xs" style={{ color: 'var(--color-text-muted)' }}>
            <Clock size={12} />
            <span>Uptime: {formatUptime(uptime)}</span>
          </div>
        </div>
      </div>

      {error && (
        <div
          className="mb-4 px-4 py-3 rounded text-sm border"
          style={{ backgroundColor: 'rgba(255,68,68,0.1)', borderColor: 'var(--color-danger)', color: 'var(--color-danger)' }}
        >
          Failed to load status: {String(error)}
        </div>
      )}

      {isLoading && (
        <div className="text-xs mb-4" style={{ color: 'var(--color-text-muted)' }}>
          Loading...
        </div>
      )}

      {/* Stat cards */}
      <div className="grid grid-cols-2 lg:grid-cols-5 gap-4 mb-6">
        {cards.map((card) => (
          <Card key={card.label} {...card} />
        ))}
      </div>

      {/* Quick Access */}
      <div className="mb-6">
        <h2 className="text-xs font-semibold uppercase tracking-widest mb-3" style={{ color: 'var(--color-text-muted)' }}>
          Quick Access
        </h2>
        <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-7 gap-2">
          {quickLinks.map((link) => (
            <Link
              key={link.to}
              to={link.to}
              className="flex flex-col items-center gap-2 p-3 rounded-lg border text-center card-hover transition-colors group"
              style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
            >
              <span
                className="p-2 rounded transition-colors"
                style={{ backgroundColor: 'var(--color-accent-dim)', color: 'var(--color-accent)' }}
              >
                {link.icon}
              </span>
              <span className="text-xs font-medium" style={{ color: 'var(--color-text)' }}>
                {link.label}
              </span>
              <span className="text-xs leading-tight hidden sm:block" style={{ color: 'var(--color-text-muted)' }}>
                {link.desc}
              </span>
            </Link>
          ))}
        </div>
      </div>

      {/* Two-column section */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        {/* System Health */}
        <div
          className="rounded-lg border p-4"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <div className="flex items-center gap-2 mb-4">
            <Activity size={14} style={{ color: 'var(--color-accent)' }} />
            <h2 className="text-sm font-semibold">System Health</h2>
          </div>
          {Object.keys(components).length === 0 ? (
            <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>No components reported</p>
          ) : (
            <div className="space-y-2">
              {Object.entries(components).map(([name, comp]) => {
                const st = comp?.status ?? 'unknown'
                const isOk = st === 'ok'
                return (
                  <div key={name} className="flex items-center justify-between text-xs">
                    <span style={{ color: 'var(--color-text-muted)' }}>{name}</span>
                    <span
                      className="flex items-center gap-1"
                      style={{ color: isOk ? 'var(--color-accent)' : 'var(--color-warning)' }}
                    >
                      <span className={clsx('status-dot', isOk ? 'online' : 'warning')} />
                      {st}
                    </span>
                  </div>
                )
              })}
            </div>
          )}
        </div>

        {/* Configuration */}
        <div
          className="rounded-lg border p-4"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <div className="flex items-center gap-2 mb-4">
            <TrendingUp size={14} style={{ color: 'var(--color-accent)' }} />
            <h2 className="text-sm font-semibold">Configuration</h2>
          </div>
          <div className="space-y-2 text-xs">
            {[
              ['Provider', status?.provider ?? '—'],
              ['Model', status?.model ?? '—'],
              ['Temperature', status?.temperature?.toFixed(2) ?? '—'],
              ['Paired', status?.paired ? 'Yes' : 'No'],
            ].map(([k, v]) => (
              <div key={k} className="flex items-center justify-between">
                <span style={{ color: 'var(--color-text-muted)' }}>{k}</span>
                <span className="font-mono truncate max-w-[180px]" style={{ color: 'var(--color-text)' }}>{v}</span>
              </div>
            ))}
          </div>
        </div>

        {/* Market Scanner */}
        <div
          className="rounded-lg border p-4"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <div className="flex items-center justify-between mb-4">
            <div className="flex items-center gap-2">
              <Timer size={14} style={{ color: 'var(--color-accent)' }} />
              <h2 className="text-sm font-semibold">Market Scanner</h2>
            </div>
            <Link
              to="/skills"
              className="text-xs flex items-center gap-1 transition-opacity hover:opacity-70"
              style={{ color: 'var(--color-accent)' }}
            >
              Manage <ChevronRight size={11} />
            </Link>
          </div>
          <div className="space-y-2 text-xs">
            <div className="flex items-center justify-between">
              <span style={{ color: 'var(--color-text-muted)' }}>Status</span>
              <span className="flex items-center gap-1" style={{ color: cronData?.market_scanner?.enabled ? 'var(--color-accent)' : 'var(--color-text-muted)' }}>
                <span className={clsx('status-dot', cronData?.market_scanner?.enabled ? 'online' : 'offline')} />
                {cronData?.market_scanner?.enabled ? 'Running' : 'Stopped'}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span style={{ color: 'var(--color-text-muted)' }}>Scan interval</span>
              <span className="font-mono" style={{ color: 'var(--color-text)' }}>
                {scanInterval != null ? formatInterval(scanInterval) : '—'}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span style={{ color: 'var(--color-text-muted)' }}>Active strategies</span>
              <span className="font-mono" style={{ color: 'var(--color-text)' }}>{liveCount}</span>
            </div>
          </div>
        </div>

        {/* Security */}
        <div
          className="rounded-lg border p-4"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <div className="flex items-center gap-2 mb-4">
            <Shield size={14} style={{ color: 'var(--color-accent)' }} />
            <h2 className="text-sm font-semibold">Security</h2>
          </div>
          <div className="grid grid-cols-2 gap-4 text-xs">
            <div>
              <p style={{ color: 'var(--color-text-muted)' }} className="mb-1">Authentication</p>
              <p style={{ color: status?.paired ? 'var(--color-accent)' : 'var(--color-warning)' }}>
                {status?.paired ? 'Bearer token active' : 'Not paired — open access'}
              </p>
            </div>
            <div>
              <p style={{ color: 'var(--color-text-muted)' }} className="mb-1">All secrets masked</p>
              <p style={{ color: 'var(--color-accent)' }}>API keys encrypted at rest</p>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
