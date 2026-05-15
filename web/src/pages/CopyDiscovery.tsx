import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import { Search, GraduationCap, Ban, Clock, Award, Plus, RefreshCw, Trash2, BarChart2, X } from 'lucide-react'
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
  const [showAdd, setShowAdd] = useState(false)
  const [statusFilter, setStatusFilter] = useState<'all' | 'candidate' | 'graduated' | 'blacklisted'>(
    'candidate'
  )
  const [newAddr, setNewAddr] = useState('')
  const [newVenue, setNewVenue] = useState('polymarket')
  const [newScore, setNewScore] = useState('')
  const [formError, setFormError] = useState<string | null>(null)
  const [scoreAddr, setScoreAddr] = useState<string | null>(null)

  const { data, isLoading } = useQuery({
    queryKey: ['copy-discovery'],
    queryFn: () => apiFetch<{ candidates: Candidate[] }>('/api/copy/discovery'),
  })

  const { data: scoreData } = useQuery({
    queryKey: ['copy-score', scoreAddr],
    queryFn: () => apiFetch<Record<string, unknown>>(`/api/copy/score/${scoreAddr}`),
    enabled: !!scoreAddr,
  })

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['copy-discovery'] })

  const addMutation = useMutation({
    mutationFn: (body: { address: string; venue: string; discovery_score?: number }) =>
      apiPost('/api/copy/discovery', body),
    onSuccess: () => {
      setNewAddr('')
      setNewScore('')
      setShowAdd(false)
      setFormError(null)
      invalidate()
    },
    onError: (err: Error) => setFormError(err.message),
  })

  const refreshMutation = useMutation({
    mutationFn: () => apiPost('/api/copy/discovery/refresh', {}),
    onSuccess: () => setTimeout(invalidate, 1500),
  })

  const graduateMutation = useMutation({
    mutationFn: (addr: string) => apiPost(`/api/copy/discovery/${addr}/graduate`, {}),
    onSuccess: () => {
      invalidate()
      queryClient.invalidateQueries({ queryKey: ['copy-leaders'] })
    },
  })

  const blacklistMutation = useMutation({
    mutationFn: (addr: string) => apiPost(`/api/copy/discovery/${addr}/blacklist`, {}),
    onSuccess: invalidate,
  })

  const removeMutation = useMutation({
    mutationFn: (addr: string) => apiDelete(`/api/copy/discovery/${addr}`),
    onSuccess: invalidate,
  })

  const candidates = data?.candidates ?? []
  const filtered = candidates.filter((c) => {
    if (statusFilter !== 'all' && c.status !== statusFilter) return false
    return c.wallet_address.toLowerCase().includes(filter.toLowerCase())
  })

  function handleAdd(e: React.FormEvent) {
    e.preventDefault()
    const addr = newAddr.trim().toLowerCase()
    if (!/^0x[0-9a-f]{40}$/.test(addr)) {
      setFormError('Address must be 0x + 40 hex chars')
      return
    }
    const score = newScore.trim() === '' ? undefined : Number(newScore)
    if (score !== undefined && (Number.isNaN(score) || score < 0 || score > 100)) {
      setFormError('Score must be a number 0–100')
      return
    }
    addMutation.mutate({ address: addr, venue: newVenue, discovery_score: score })
  }

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6 gap-3">
        <div className="flex items-center gap-3">
          <Search size={22} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-xl font-bold" style={{ color: 'var(--color-text)' }}>
            Discovery
          </h1>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => refreshMutation.mutate()}
            disabled={refreshMutation.isPending}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors"
            style={{
              backgroundColor: 'var(--color-surface)',
              color: 'var(--color-text)',
              border: '1px solid var(--color-border)',
              opacity: refreshMutation.isPending ? 0.6 : 1,
            }}
            title="Run the Polymarket indexer to fetch new candidates from the public leaderboard"
          >
            <RefreshCw size={14} className={refreshMutation.isPending ? 'animate-spin' : ''} />
            {refreshMutation.isPending ? 'Refreshing…' : 'Refresh from leaderboard'}
          </button>
          <button
            onClick={() => {
              setShowAdd((v) => !v)
              setFormError(null)
            }}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            <Plus size={14} />
            Add wallet
          </button>
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
      </div>

      {/* Add wallet form */}
      {showAdd && (
        <form
          onSubmit={handleAdd}
          className="rounded-xl border p-4 mb-4 flex flex-wrap gap-3 items-end"
          style={{
            backgroundColor: 'var(--color-surface)',
            borderColor: 'var(--color-border)',
          }}
        >
          <div className="flex-1 min-w-[260px]">
            <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>
              Wallet address (0x…)
            </label>
            <input
              type="text"
              value={newAddr}
              onChange={(e) => setNewAddr(e.target.value)}
              placeholder="0xabc…"
              className="w-full px-3 py-1.5 rounded-lg text-sm font-mono border outline-none"
              style={{
                backgroundColor: 'var(--color-base)',
                borderColor: 'var(--color-border)',
                color: 'var(--color-text)',
              }}
              autoFocus
            />
          </div>
          <div>
            <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>
              Venue
            </label>
            <select
              value={newVenue}
              onChange={(e) => setNewVenue(e.target.value)}
              className="px-3 py-1.5 rounded-lg text-sm border outline-none"
              style={{
                backgroundColor: 'var(--color-base)',
                borderColor: 'var(--color-border)',
                color: 'var(--color-text)',
              }}
            >
              <option value="polymarket">polymarket</option>
              <option value="hyperliquid">hyperliquid</option>
            </select>
          </div>
          <div>
            <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>
              Manual score (0–100)
            </label>
            <input
              type="number"
              min={0}
              max={100}
              step={0.1}
              value={newScore}
              onChange={(e) => setNewScore(e.target.value)}
              placeholder="optional"
              className="w-32 px-3 py-1.5 rounded-lg text-sm border outline-none"
              style={{
                backgroundColor: 'var(--color-base)',
                borderColor: 'var(--color-border)',
                color: 'var(--color-text)',
              }}
            />
          </div>
          <button
            type="submit"
            disabled={addMutation.isPending}
            className="px-4 py-1.5 rounded-lg text-xs font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            {addMutation.isPending ? 'Adding…' : 'Add'}
          </button>
          {formError && (
            <p className="w-full text-xs" style={{ color: '#ef4444' }}>
              {formError}
            </p>
          )}
        </form>
      )}

      {/* Filters */}
      <div className="mb-6 flex flex-wrap gap-3 items-center">
        <input
          type="text"
          placeholder="Filter by wallet address..."
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          className="flex-1 max-w-md px-4 py-2 rounded-lg text-sm border outline-none focus:ring-1"
          style={{
            backgroundColor: 'var(--color-surface)',
            borderColor: 'var(--color-border)',
            color: 'var(--color-text)',
          }}
        />
        <div className="flex gap-1">
          {(['candidate', 'graduated', 'blacklisted', 'all'] as const).map((s) => (
            <button
              key={s}
              onClick={() => setStatusFilter(s)}
              className="px-3 py-1.5 rounded-lg text-xs font-medium transition-colors capitalize"
              style={{
                backgroundColor:
                  statusFilter === s ? 'var(--color-accent)' : 'var(--color-surface)',
                color: statusFilter === s ? '#000' : 'var(--color-text-muted)',
                border: '1px solid var(--color-border)',
              }}
            >
              {s}
            </button>
          ))}
        </div>
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
          <Award size={32} className="mx-auto mb-3" style={{ color: 'var(--color-text-muted)' }} />
          <p className="text-sm font-medium mb-1" style={{ color: 'var(--color-text)' }}>
            No candidates {statusFilter !== 'all' ? `with status "${statusFilter}"` : 'yet'}
          </p>
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            Click <b>Add wallet</b> to track one manually, or <b>Refresh from leaderboard</b> to
            pull the top Polymarket wallets via the indexer.
          </p>
        </div>
      ) : (
        <div className="grid gap-3">
          {filtered.map((candidate) => (
            <div
              key={`${candidate.venue}:${candidate.wallet_address}`}
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
                      {candidate.wallet_address.slice(0, 10)}…
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
                        {candidate.shadow_pnl !== undefined && candidate.shadow_pnl !== null
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

                <div className="flex items-center gap-2">
                  {candidate.status === 'candidate' && (
                    <>
                      <button
                        onClick={() => graduateMutation.mutate(candidate.wallet_address)}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors"
                        style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
                        title="Promote to active leader watchlist"
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
                        title="Mark as rejected"
                      >
                        <Ban size={14} />
                      </button>
                    </>
                  )}
                  <button
                    onClick={() => setScoreAddr(candidate.wallet_address)}
                    className="flex items-center gap-1.5 px-2 py-1.5 rounded-lg text-xs transition-colors hover:bg-white/5"
                    style={{ backgroundColor: 'transparent', color: 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}
                    title="Score breakdown"
                  >
                    <BarChart2 size={14} />
                  </button>
                  <button
                    onClick={() => {
                      if (confirm(`Remove ${candidate.wallet_address} from discovery?`)) {
                        removeMutation.mutate(candidate.wallet_address)
                      }
                    }}
                    className="flex items-center gap-1.5 px-2 py-1.5 rounded-lg text-xs transition-colors hover:bg-white/5"
                    style={{
                      backgroundColor: 'transparent',
                      color: 'var(--color-text-muted)',
                      border: '1px solid var(--color-border)',
                    }}
                    title="Delete row"
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
      {/* Score Breakdown Modal */}
      {scoreAddr && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setScoreAddr(null)}>
          <div className="rounded-xl border w-full max-w-sm p-5" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }} onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-sm font-semibold">Score Breakdown</h3>
              <button onClick={() => setScoreAddr(null)}><X size={14} style={{ color: 'var(--color-text-muted)' }} /></button>
            </div>
            <p className="text-xs font-mono mb-3" style={{ color: 'var(--color-text-muted)' }}>{scoreAddr.slice(0, 10)}…{scoreAddr.slice(-8)}</p>
            {!scoreData ? (
              <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>Loading…</p>
            ) : (scoreData as Record<string, unknown>).error ? (
              <p className="text-xs" style={{ color: 'var(--color-danger)' }}>Score not yet computed for this wallet.</p>
            ) : (
              <div className="space-y-2">
                {['pnl_norm', 'winrate_score', 'drawdown_score', 'sharpe_score', 'consistency_score', 'diversity_score'].map((k) => (
                  <div key={k} className="flex items-center justify-between text-xs">
                    <span style={{ color: 'var(--color-text-muted)' }}>{k.replace(/_/g, ' ')}</span>
                    <span className="font-mono font-medium" style={{ color: 'var(--color-text)' }}>
                      {scoreData[k] != null ? Number(scoreData[k]).toFixed(3) : '—'}
                    </span>
                  </div>
                ))}
                <div className="flex items-center justify-between text-sm font-semibold pt-2 border-t" style={{ borderColor: 'var(--color-border)' }}>
                  <span>Wallet Score</span>
                  <span style={{ color: 'var(--color-accent)' }}>{scoreData.wallet_score != null ? Number(scoreData.wallet_score).toFixed(1) : '—'}</span>
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
