import { useState } from 'react'
import { NavLink, useLocation } from 'react-router-dom'
import {
  LayoutDashboard,
  Wallet,
  BarChart2,
  Send,
  Brain,
  Settings,
  ChevronLeft,
  ChevronRight,
  ChevronDown,
  Activity,
  FlaskConical,
  Bot,
  Heart,
  FileText,
  Blocks,
  HelpCircle,
} from 'lucide-react'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import clsx from 'clsx'

interface NavItem {
  to: string
  icon: React.ReactNode
  label: string
}

const mainNavItems: NavItem[] = [
  { to: '/', icon: <LayoutDashboard size={18} />, label: 'Dashboard' },
  { to: '/wallets', icon: <Wallet size={18} />, label: 'Web3 Wallets' },
  { to: '/polymarket', icon: <BarChart2 size={18} />, label: 'Polymarket' },
  { to: '/telegram', icon: <Send size={18} />, label: 'Telegram' },
  { to: '/strategy-builder', icon: <Blocks size={18} />, label: 'Strategy Builder' },
  { to: '/backtesting', icon: <FlaskConical size={18} />, label: 'Backtesting' },
  { to: '/live', icon: <Bot size={18} />, label: 'Live Strategies' },
  { to: '/scheduled-jobs', icon: <Activity size={18} />, label: 'Scheduled Jobs' },
  { to: '/help', icon: <HelpCircle size={18} />, label: 'Help' },
]

const settingsNavItems: NavItem[] = [
  { to: '/health', icon: <Heart size={18} />, label: 'System Health' },
  { to: '/logs', icon: <FileText size={18} />, label: 'Gateway Log' },
  { to: '/settings/llm', icon: <Brain size={18} />, label: 'LLM Settings' },
  { to: '/settings/config', icon: <Settings size={18} />, label: 'Config' },
]

export default function Sidebar() {
  const [collapsed, setCollapsed] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const location = useLocation()

  // Auto-open settings group if current route is a settings route
  const isInSettings = settingsNavItems.some(item => location.pathname.startsWith(item.to))

  const { data: status } = useQuery({
    queryKey: ['status'],
    queryFn: () => apiFetch<Record<string, unknown>>('/api/status'),
    refetchInterval: 10_000,
  })

  const channelsMap = status?.channels && typeof status.channels === 'object'
    ? Object.fromEntries(Object.entries(status.channels as Record<string, boolean>).map(([k, v]) => [k.toLowerCase(), v]))
    : {}
  const telegramConfigured = !!channelsMap['telegram']
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
        <img
          src="/neko.png"
          alt="Neko Vives"
          className="flex-shrink-0 rounded"
          style={{ width: 32, height: 32, objectFit: 'cover' }}
        />
        {!collapsed && (
          <span className="font-bold text-sm tracking-widest" style={{ color: 'var(--color-accent)' }}>
            NEKO VIVES
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
        {mainNavItems.map((item) => (
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

        {/* Settings group */}
        {!collapsed && (
          <div className="mx-1 mt-2">
            <button
              onClick={() => setSettingsOpen(v => !v)}
              className="w-full flex items-center gap-3 px-3 py-2 rounded text-xs font-semibold uppercase tracking-widest hover:bg-white/5 transition-colors"
              style={{ color: 'var(--color-text-muted)' }}
            >
              <Settings size={14} />
              <span className="flex-1 text-left">Settings</span>
              <ChevronDown
                size={12}
                className="transition-transform"
                style={{ transform: (settingsOpen || isInSettings) ? 'rotate(180deg)' : 'rotate(0deg)' }}
              />
            </button>
            {(settingsOpen || isInSettings) && (
              <div className="ml-2 border-l pl-2 mt-0.5" style={{ borderColor: 'var(--color-border)' }}>
                {settingsNavItems.map((item) => (
                  <NavLink
                    key={item.to}
                    to={item.to}
                    className={({ isActive }) =>
                      clsx(
                        'flex items-center gap-3 px-3 py-2 my-0.5 rounded text-sm transition-colors',
                        isActive ? 'text-black font-medium' : 'hover:bg-white/5'
                      )
                    }
                    style={({ isActive }) =>
                      isActive
                        ? { backgroundColor: 'var(--color-accent)', color: '#000' }
                        : { color: 'var(--color-text-muted)' }
                    }
                  >
                    <span className="flex-shrink-0">{item.icon}</span>
                    <span>{item.label}</span>
                  </NavLink>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Collapsed settings icons */}
        {collapsed && (
          <>
            {settingsNavItems.map((item) => (
              <NavLink
                key={item.to}
                to={item.to}
                className={({ isActive }) =>
                  clsx(
                    'flex items-center justify-center px-3 py-2.5 mx-1 my-0.5 rounded text-sm transition-colors',
                    isActive ? 'text-black font-medium' : 'hover:bg-white/5'
                  )
                }
                style={({ isActive }) =>
                  isActive
                    ? { backgroundColor: 'var(--color-accent)', color: '#000' }
                    : { color: 'var(--color-text-muted)' }
                }
                title={item.label}
              >
                <span className="flex-shrink-0">{item.icon}</span>
              </NavLink>
            ))}
          </>
        )}
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
