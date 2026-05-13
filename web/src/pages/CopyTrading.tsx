import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { Users, ToggleLeft, ToggleRight, TrendingUp, Shield, AlertCircle } from 'lucide-react'
import clsx from 'clsx'

// ── Types ─────────────────────────────────────────────────────────────

interface Leader {
  address: string
  venue: string
  category?: string
  mirror_enabled: boolean
  consensus_weight: number
  wallet_score: number
  size_factor: number
}

interface MirrorPosition {
  leader_address: string
  leader_fill_id: string
  venue: string
  symbol: string
  side: string
  notional: number
  entry_price: number
  status: string
  opened_at: string
}

// ── Components ────────────────────────────────────────────────────────

export default function CopyTrading() {
  const queryClient = useQueryClient()
  const [activeTab, setActiveTab] = useState<'leaders' | 'positions'>('leaders')

  const { data: leadersData, isLoading: leadersLoading } = useQuery({
    queryKey: ['copy-leaders'],
    queryFn: () => apiFetch<{ leaders: Leader[] }>('/api/copy/leaders'),
  })

  const { data: positionsData, isLoading: positionsLoading } = useQuery({
    queryKey: ['copy-positions'],
    queryFn: () => apiFetch<{ positions: MirrorPosition[] }>('/api/copy/positions'),
  })

  const toggleMutation = useMutation({
    mutationFn: (addr: string) =>
      apiPost<{ mirror_enabled: boolean }>(`/api/copy/leaders/${addr}/toggle`, {}),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['copy-leaders'] }),
  })

  const leaders = leadersData?.leaders ?? []
  const positions = positionsData?.positions ?? []

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <Users size={22} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-xl font-bold" style={{ color: 'var(--color-text)' }}>
            Copy Trading
          </h1>
        </div>
        <div
          className="flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-medium"
          style={{
            backgroundColor: 'var(--color-surface)',
            color: 'var(--color-text-muted)',
            border: '1px solid var(--color-border)',
          }}
        >
          <Shield size={14} />
          <span>Discovery Mode — No capital at risk</span>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 mb-6">
        {(['leaders', 'positions'] as const).map((tab) => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            className={clsx(
              'px-4 py-2 rounded-lg text-sm font-medium transition-colors',
              activeTab === tab ? 'text-black' : 'hover:bg-white/5'
            )}
            style={{
              backgroundColor:
                activeTab === tab ? 'var(--color-accent)' : 'transparent',
              color: activeTab === tab ? '#000' : 'var(--color-text-muted)',
            }}
          >
            {tab === 'leaders' ? 'Leader Studio' : 'Mirror Positions'}
          </button>
        ))}
      </div>

      {/* Leaders Tab */}
      {activeTab === 'leaders' && (
        <div>
          {leadersLoading ? (
            <div className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
              Loading leaders...
            </div>
          ) : leaders.length === 0 ? (
            <div
              className="rounded-xl border p-8 text-center"
              style={{
                backgroundColor: 'var(--color-surface)',
                borderColor: 'var(--color-border)',
              }}
            >
              <AlertCircle
                size={32}
                className="mx-auto mb-3"
                style={{ color: 'var(--color-text-muted)' }}
              />
              <p className="text-sm font-medium mb-1" style={{ color: 'var(--color-text)' }}>
                No active leaders yet
              </p>
              <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                Graduate candidates from the Discovery page to start tracking wallets.
              </p>
            </div>
          ) : (
            <div className="grid gap-3">
              {leaders.map((leader) => (
                <div
                  key={leader.address}
                  className="rounded-xl border p-4 flex items-center justify-between"
                  style={{
                    backgroundColor: 'var(--color-surface)',
                    borderColor: 'var(--color-border)',
                  }}
                >
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 mb-1">
                      <span
                        className="text-sm font-mono font-medium truncate"
                        style={{ color: 'var(--color-text)' }}
                      >
                        {leader.address.slice(0, 8)}...{leader.address.slice(-6)}
                      </span>
                      <span
                        className="text-xs px-2 py-0.5 rounded-full"
                        style={{
                          backgroundColor: 'var(--color-base)',
                          color: 'var(--color-text-muted)',
                        }}
                      >
                        {leader.venue}
                      </span>
                      {leader.category && (
                        <span
                          className="text-xs px-2 py-0.5 rounded-full"
                          style={{
                            backgroundColor: 'var(--color-base)',
                            color: 'var(--color-text-muted)',
                          }}
                        >
                          {leader.category}
                        </span>
                      )}
                    </div>
                    <div className="flex items-center gap-4 text-xs">
                      <span style={{ color: 'var(--color-text-muted)' }}>
                        Score:{' '}
                        <span
                          className="font-medium"
                          style={{
                            color:
                              leader.wallet_score >= 80
                                ? '#22c55e'
                                : leader.wallet_score >= 65
                                ? '#eab308'
                                : '#ef4444',
                          }}
                        >
                          {leader.wallet_score.toFixed(1)}
                        </span>
                      </span>
                      <span style={{ color: 'var(--color-text-muted)' }}>
                        Size factor: {leader.size_factor.toFixed(2)}x
                      </span>
                    </div>
                  </div>

                  <div className="flex items-center gap-3 ml-4">
                    <button
                      onClick={() => toggleMutation.mutate(leader.address)}
                      className="flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors"
                      style={{
                        backgroundColor: leader.mirror_enabled
                          ? 'rgba(34, 197, 94, 0.15)'
                          : 'var(--color-base)',
                        color: leader.mirror_enabled ? '#22c55e' : 'var(--color-text-muted)',
                        border: '1px solid var(--color-border)',
                      }}
                    >
                      {leader.mirror_enabled ? (
                        <ToggleRight size={16} />
                      ) : (
                        <ToggleLeft size={16} />
                      )}
                      {leader.mirror_enabled ? 'Mirror ON' : 'Mirror OFF'}
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Positions Tab */}
      {activeTab === 'positions' && (
        <div>
          {positionsLoading ? (
            <div className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
              Loading positions...
            </div>
          ) : positions.length === 0 ? (
            <div
              className="rounded-xl border p-8 text-center"
              style={{
                backgroundColor: 'var(--color-surface)',
                borderColor: 'var(--color-border)',
              }}
            >
              <TrendingUp
                size={32}
                className="mx-auto mb-3"
                style={{ color: 'var(--color-text-muted)' }}
              />
              <p className="text-sm font-medium" style={{ color: 'var(--color-text)' }}>
                No open mirror positions
              </p>
            </div>
          ) : (
            <div className="grid gap-3">
              {positions.map((pos) => (
                <div
                  key={pos.leader_fill_id}
                  className="rounded-xl border p-4"
                  style={{
                    backgroundColor: 'var(--color-surface)',
                    borderColor: 'var(--color-border)',
                  }}
                >
                  <div className="flex items-center justify-between mb-2">
                    <span className="text-sm font-medium" style={{ color: 'var(--color-text)' }}>
                      {pos.symbol}
                    </span>
                    <span
                      className={clsx(
                        'text-xs px-2 py-0.5 rounded-full font-medium',
                        pos.side === 'buy' ? 'text-green-600' : 'text-red-600'
                      )}
                      style={{
                        backgroundColor:
                          pos.side === 'buy'
                            ? 'rgba(34, 197, 94, 0.15)'
                            : 'rgba(239, 68, 68, 0.15)',
                      }}
                    >
                      {pos.side.toUpperCase()}
                    </span>
                  </div>
                  <div className="grid grid-cols-3 gap-4 text-xs">
                    <div>
                      <span style={{ color: 'var(--color-text-muted)' }}>Notional</span>
                      <p className="font-medium" style={{ color: 'var(--color-text)' }}>
                        ${pos.notional.toFixed(2)}
                      </p>
                    </div>
                    <div>
                      <span style={{ color: 'var(--color-text-muted)' }}>Entry</span>
                      <p className="font-medium" style={{ color: 'var(--color-text)' }}>
                        ${pos.entry_price.toFixed(4)}
                      </p>
                    </div>
                    <div>
                      <span style={{ color: 'var(--color-text-muted)' }}>Leader</span>
                      <p className="font-mono font-medium" style={{ color: 'var(--color-text)' }}>
                        {pos.leader_address.slice(0, 6)}...{pos.leader_address.slice(-4)}
                      </p>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
