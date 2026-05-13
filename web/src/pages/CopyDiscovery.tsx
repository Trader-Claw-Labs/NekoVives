import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { Search, GraduationCap, Ban, TrendingUp, Clock, Award } from 'lucide-react'
import clsx from 'clsx'

// ── Types ─────────────────────────────────────────────────────────────

interface Candidate {
  wallet_address: string
  venue: string
  discovery_score: number
  shadow_pnl?: number
  shadow_sharpe?: number
  status: string
  discovered_at: string
  graduated_at?: string
}

// ── Components ────────────────────────────────────────────────────────

export default function CopyDiscovery() {
  const queryClient = useQueryClient()
  const [filter, setFilter] = useState('')

  const { data, isLoading } = useQuery({
    queryKey: ['copy-discovery'],
    queryFn: () => apiFetch<{ candidates: Candidate[] }>('/api/copy/discovery'),
  })

  const graduateMutation = useMutation({
    mutationFn: (addr: string) =>
      apiPost(`/api/copy/discovery/${addr}/graduate`, {}),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['copy-discovery'] }),
  })

  const blacklistMutation = useMutation({
    mutationFn: (addr: string) =>
      apiPost(`/api/copy/discovery/${addr}/blacklist`, {}),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['copy-discovery'] }),
  })

  const candidates = data?.candidates ?? []
  const filtered = candidates.filter((c) =>
    c.wallet_address.toLowerCase().includes(filter.toLowerCase())
  )

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <Search size={22} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-xl font-bold" style={{ color: 'var(--color-text)' }}>
            Discovery
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
          <Clock size={14} />
          <span>Shadow tracking — 0% capital at risk</span>
        </div>
      </div>

      {/* Search */}
      <div className="mb-6">
        <input
          type="text"
          placeholder="Filter by wallet address..."
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          className="w-full max-w-md px-4 py-2 rounded-lg text-sm border outline-none focus:ring-1"
          style={{
            backgroundColor: 'var(--color-surface)',
            borderColor: 'var(--color-border)',
            color: 'var(--color-text)',
          }}
        />
      </div>

      {/* Candidates List */}
      {isLoading ? (
        <div className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
          Loading candidates...
        </div>
      ) : filtered.length === 0 ? (
        <div
          className="rounded-xl border p-8 text-center"
          style={{
            backgroundColor: 'var(--color-surface)',
            borderColor: 'var(--color-border)',
          }}
        >
          <Award
            size={32}
            className="mx-auto mb-3"
            style={{ color: 'var(--color-text-muted)' }}
          />
          <p className="text-sm font-medium mb-1" style={{ color: 'var(--color-text)' }}>
            No candidates yet
          </p>
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            Run the nightly indexer to discover and score wallets. Discovery mode tracks wallets
            without executing real trades for 30+ days.
          </p>
        </div>
      ) : (
        <div className="grid gap-3">
          {filtered.map((candidate) => (
            <div
              key={candidate.wallet_address}
              className="rounded-xl border p-4"
              style={{
                backgroundColor: 'var(--color-surface)',
                borderColor: 'var(--color-border)',
              }}
            >
              <div className="flex items-start justify-between gap-4">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-2">
                    <span
                      className="text-sm font-mono font-medium truncate"
                      style={{ color: 'var(--color-text)' }}
                    >
                      {candidate.wallet_address.slice(0, 10)}...
                      {candidate.wallet_address.slice(-8)}
                    </span>
                    <span
                      className="text-xs px-2 py-0.5 rounded-full"
                      style={{
                        backgroundColor: 'var(--color-base)',
                        color: 'var(--color-text-muted)',
                      }}
                    >
                      {candidate.venue}
                    </span>
                    <span
                      className={clsx(
                        'text-xs px-2 py-0.5 rounded-full font-medium',
                        candidate.status === 'candidate'
                          ? 'text-yellow-600'
                          : candidate.status === 'graduated'
                          ? 'text-green-600'
                          : 'text-red-600'
                      )}
                      style={{
                        backgroundColor:
                          candidate.status === 'candidate'
                            ? 'rgba(234, 179, 8, 0.15)'
                            : candidate.status === 'graduated'
                            ? 'rgba(34, 197, 94, 0.15)'
                            : 'rgba(239, 68, 68, 0.15)',
                      }}
                    >
                      {candidate.status}
                    </span>
                  </div>

                  <div className="grid grid-cols-4 gap-4 text-xs mb-3">
                    <div>
                      <span style={{ color: 'var(--color-text-muted)' }}>Discovery Score</span>
                      <p
                        className="font-medium"
                        style={{
                          color:
                            candidate.discovery_score >= 80
                              ? '#22c55e'
                              : candidate.discovery_score >= 65
                              ? '#eab308'
                              : '#ef4444',
                        }}
                      >
                        {candidate.discovery_score.toFixed(1)}
                      </p>
                    </div>
                    <div>
                      <span style={{ color: 'var(--color-text-muted)' }}>Shadow PnL</span>
                      <p className="font-medium" style={{ color: 'var(--color-text)' }}>
                        {candidate.shadow_pnl !== undefined
                          ? `$${candidate.shadow_pnl.toFixed(2)}`
                          : '—'}
                      </p>
                    </div>
                    <div>
                      <span style={{ color: 'var(--color-text-muted)' }}>Shadow Sharpe</span>
                      <p className="font-medium" style={{ color: 'var(--color-text)' }}>
                        {candidate.shadow_sharpe?.toFixed(2) ?? '—'}
                      </p>
                    </div>
                    <div>
                      <span style={{ color: 'var(--color-text-muted)' }}>Discovered</span>
                      <p className="font-medium" style={{ color: 'var(--color-text)' }}>
                        {new Date(candidate.discovered_at).toLocaleDateString()}
                      </p>
                    </div>
                  </div>
                </div>

                {candidate.status === 'candidate' && (
                  <div className="flex items-center gap-2">
                    <button
                      onClick={() => graduateMutation.mutate(candidate.wallet_address)}
                      className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors"
                      style={{
                        backgroundColor: 'var(--color-accent)',
                        color: '#000',
                      }}
                    >
                      <GraduationCap size={14} />
                      Graduate
                    </button>
                    <button
                      onClick={() => blacklistMutation.mutate(candidate.wallet_address)}
                      className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors hover:bg-red-500/15"
                      style={{
                        backgroundColor: 'var(--color-base)',
                        color: '#ef4444',
                        border: '1px solid var(--color-border)',
                      }}
                    >
                      <Ban size={14} />
                    </button>
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
