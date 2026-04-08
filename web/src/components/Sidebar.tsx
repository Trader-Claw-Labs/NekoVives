import { useState } from 'react'
import { NavLink } from 'react-router-dom'
import {
  LayoutDashboard,
  Wallet,
  BarChart2,
  Send,
  Zap,
  MessageSquare,
  Brain,
  Settings,
  ChevronLeft,
  ChevronRight,
  Activity,
  TrendingUp,
  FlaskConical,
} from 'lucide-react'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import clsx from 'clsx'

interface NavItem {
  to: string
  icon: React.ReactNode
  label: string
}

const navItems: NavItem[] = [
  { to: '/', icon: <LayoutDashboard size={18} />, label: 'Dashboard' },
  { to: '/wallets', icon: <Wallet size={18} />, label: 'Web3 Wallets' },
  { to: '/polymarket', icon: <BarChart2 size={18} />, label: 'Polymarket' },
  { to: '/telegram', icon: <Send size={18} />, label: 'Telegram' },
  { to: '/skills', icon: <Zap size={18} />, label: 'Skills' },
  { to: '/chat', icon: <MessageSquare size={18} />, label: 'Terminal' },
  { to: '/tradingview', icon: <TrendingUp size={18} />, label: 'TradingView' },
  { to: '/backtesting', icon: <FlaskConical size={18} />, label: 'Backtesting' },
  { to: '/settings/llm', icon: <Brain size={18} />, label: 'LLM Settings' },
  { to: '/settings/config', icon: <Settings size={18} />, label: 'Config' },
]

export default function Sidebar() {
  const [collapsed, setCollapsed] = useState(false)

  const { data: status } = useQuery({
    queryKey: ['status'],
    queryFn: () => apiFetch<Record<string, unknown>>('/api/status'),
    refetchInterval: 10_000,
  })

  const telegramConfigured = !!(status?.channels && typeof status.channels === 'object' && (status.channels as Record<string, boolean>)['telegram'])
  const telegramHealth = (status as any)?.health?.components?.['channel:telegram']
  const telegramStatus: 'online' | 'offline' | 'warning' = !telegramConfigured
    ? 'offline'
    : telegramHealth?.status === 'ok'
    ? 'online'
    : telegramHealth?.status === 'error'
    ? 'offline'
    : 'warning'
  const llmOnline = !!status?.provider

  return (
    <aside
      className={clsx(
        'flex flex-col h-full border-r transition-all duration-200',
        collapsed ? 'w-14' : 'w-56'
      )}
      style={{
        backgroundColor: 'var(--color-surface)',
        borderColor: 'var(--color-border)',
      }}
    >
      {/* Logo */}
      <div
        className="flex items-center gap-2 px-3 py-4 border-b"
        style={{ borderColor: 'var(--color-border)' }}
      >
        <div
          className="flex items-center justify-center w-8 h-8 rounded text-xs font-bold flex-shrink-0"
          style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
        >
          TC
        </div>
        {!collapsed && (
          <span className="font-bold text-sm tracking-widest" style={{ color: 'var(--color-accent)' }}>
            TRADER CLAW
          </span>
        )}
      </div>

      {/* Status bar */}
      {!collapsed && (
        <div
          className="flex items-center gap-3 px-3 py-2 text-xs border-b"
          style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
        >
          <div className="flex items-center gap-1">
            <span className={clsx('status-dot', telegramStatus)} />
            <span>TG</span>
          </div>
          <div className="flex items-center gap-1">
            <span className={clsx('status-dot', llmOnline ? 'online' : 'offline')} />
            <span>LLM</span>
          </div>
          <div className="flex items-center gap-1 ml-auto">
            <Activity size={10} style={{ color: 'var(--color-accent)' }} />
          </div>
        </div>
      )}

      {/* Navigation */}
      <nav className="flex-1 py-2 overflow-y-auto">
        {navItems.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.to === '/'}
            className={({ isActive }) =>
              clsx(
                'flex items-center gap-3 px-3 py-2.5 mx-1 my-0.5 rounded text-sm transition-colors',
                collapsed && 'justify-center',
                isActive
                  ? 'text-black font-medium'
                  : 'hover:bg-white/5'
              )
            }
            style={({ isActive }) =>
              isActive
                ? { backgroundColor: 'var(--color-accent)', color: '#000' }
                : { color: 'var(--color-text-muted)' }
            }
            title={collapsed ? item.label : undefined}
          >
            <span className="flex-shrink-0">{item.icon}</span>
            {!collapsed && <span>{item.label}</span>}
          </NavLink>
        ))}
      </nav>

      {/* Collapse toggle */}
      <button
        onClick={() => setCollapsed((c) => !c)}
        className="flex items-center justify-center py-3 border-t transition-colors hover:bg-white/5"
        style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
      >
        {collapsed ? <ChevronRight size={16} /> : <ChevronLeft size={16} />}
      </button>
    </aside>
  )
}
