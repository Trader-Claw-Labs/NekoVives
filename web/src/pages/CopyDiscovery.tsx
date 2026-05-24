import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import {
  Search, GraduationCap, Ban, Clock, Award, Plus, RefreshCw,
  Trash2, ChevronDown, ChevronUp, TrendingUp, Activity,
} from 'lucide-react'
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

interface RecentTrade {
  side: string | null
  notional: number | null
  price: number | null
  market_id: string | null
  timestamp: string | null
}

interface DiscoveryStats {
  trade_count: number
  buy_count: number
  sell_count: number
  avg_notional: number
  total_notional: number
  last_trade_at: string | null
  recent_trades: RecentTrade[]
}

// ── Helpers ───────────────────────────────────────────────────────────

function relativeTime(iso: string | null): string {
  if (!iso) return '—'
  const diff = Date.now() - new Date(iso).getTime()
  const mins = Math.floor(diff / 60000)
  if (mins < 1) return 'just now'
  if (mins < 60) return `${mins}m ago`
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return `${hrs}h ago`
  return `${Math.floor(hrs / 24)}d ago`
}

function slugFromMarketId(marketId: string | null): string {
  if (!marketId) return '—'
  // If it looks like a slug (has dashes), show it truncated; otherwise show short hash
  if (marketId.includes('-')) return marketId.length > 38 ? marketId.slice(0, 38) + '…' : marketId
  return marketId.slice(0, 10) + '…'
}

// ── WalletStatsPanel ──────────────────────────────────────────────────

function WalletStatsPanel({ address }: { address: string }) {
  const { data, isLoading, isError } = useQuery<DiscoveryStats>({
    queryKey: ['discovery-stats', address],
    queryFn: () => apiFetch(`/api/copy/discovery/${address}/stats`),
    staleTime: 30_000,
    refetchInterval: 60_000,
  })

  if (isLoading) {
    return (
      <div className="mt-3 pt-3 border-t text-xs" style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
        Loading trades…
      </div>
    )
  }
  if (isError || !data) {
    return (
      <div className="mt-3 pt-3 border-t text-xs" style={{ borderColor: 'var(--color-border)', color: '#ef4444' }}>
        Could not load trades.
      </div>
    )
  }

  const buyPct = data.trade_count > 0 ? Math.round((data.buy_count / data.trade_count) * 100) : 0

  return (
    <div className="mt-3 pt-3 border-t" style={{ borderColor: 'var(--color-border)' }}>
      {/* Stats row */}
      <div className="grid grid-cols-4 gap-3 mb-3 text-xs">
        <div>
          <p style={{ color: 'var(--color-text-muted)' }}>Trades tracked</p>
          <p className="font-semibold mt-0.5" style={{ color: 'var(--color-text)' }}>
            {data.trade_count.toLocaleString()}
          </p>
        </div>
        <div>
          <p style={{ color: 'var(--color-text-muted)' }}>Avg size</p>
          <p className="font-semibold mt-0.5" style={{ color: 'var(--color-text)' }}>
            ${data.avg_notional > 0 ? data.avg_notional.toFixed(2) : '—'}
          </p>
        </div>
        <div>
          <p style={{ color: 'var(--color-text-muted)' }}>Buy ratio</p>
          <p
            className="font-semibold mt-0.5"
            style={{ color: buyPct >= 60 ? '#22c55e' : buyPct <= 40 ? '#ef4444' : 'var(--color-text)' }}
          >
            {data.trade_count > 0 ? `${buyPct}%` : '—'}
          </p>
        </div>
        <div>
          <p style={{ color: 'var(--color-text-muted)' }}>Last active</p>
          <p className="font-semibold mt-0.5" style={{ color: 'var(--color-text)' }}>
            {relativeTime(data.last_trade_at)}
          </p>
        </div>
      </div>

      {/* Recent trades */}
      {data.recent_trades.length === 0 ? (
        <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>No trades recorded yet — polling every 5 s.</p>
      ) : (
        <div className="space-y-1">
          <p className="text-xs font-medium mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
            Last {data.recent_trades.length} trades
          </p>
          {data.recent_trades.map((t, i) => (
            <div
              key={i}
              className="flex items-center gap-2 text-xs rounded-lg px-2 py-1"
              style={{ backgroundColor: 'var(--color-base)' }}
            >
              <span
                className="w-8 text-center font-bold rounded text-[10px] py-0.5"
                style={{
                  backgroundColor: t.side === 'buy' ? 'rgba(34,197,94,0.15)' : 'rgba(239,68,68,0.15)',
                  color: t.side === 'buy' ? '#22c55e' : '#ef4444',
                }}
              >
                {t.side?.toUpperCase() ?? '?'}
              </span>
              <span className="flex-1 truncate font-mono" style={{ color: 'var(--color-text)' }}>
                {slugFromMarketId(t.market_id)}
              </span>
              <span className="font-medium" style={{ color: 'var(--color-text)' }}>
                ${t.notional != null ? t.notional.toFixed(2) : '—'}
              </span>
              <span style={{ color: 'var(--color-text-muted)' }}>
                @{t.price != null ? t.price.toFixed(4) : '—'}
              </span>
              <span className="w-16 text-right" style={{ color: 'var(--color-text-muted)' }}>
                {relativeTime(t.timestamp)}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ── Main ─────────────────────────────────────────────────────────────

export default function CopyDiscovery() {
  const queryClient = useQueryClient()
  const [filter, setFilter] = useState('')
  const [showAdd, setShowAdd] = useState(false)
  const [statusFilter, setStatusFilter] = useState<'all' | 'candidate' | 'graduated' | 'blacklisted'>('candidate')
  const [newAddr, setNewAddr] = useState('')
  const [newVenue, setNewVenue] = useState('polymarket')
  const [newScore, setNewScore] = useState('')
  const [formError, setFormError] = useState<string | null>(null)
  const [expandedAddr, setExpandedAddr] = useState<string | null>(null)

  const { data, isLoading } = useQuery({
    queryKey: ['copy-discovery'],
    queryFn: () => apiFetch<{ candidates: Candidate[] }>('/api/copy/discovery'),
    refetchInterval: 30_000,
  })

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['copy-discovery'] })

  const addMutation = useMutation({
    mutationFn: (body: { address: string; venue: string; discovery_score?: number }) =>
      apiPost('/api/copy/discovery', body),
    onSuccess: () => {
      setNewAddr(''); setNewScore(''); setShowAdd(false); setFormError(null)
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
    if (!/^0x[0-9a-f]{40}$/.test(addr)) { setFormError('Address must be 0x + 40 hex chars'); return }
    const score = newScore.trim() === '' ? undefined : Number(newScore)
    if (score !== undefined && (Number.isNaN(score) || score < 0 || score > 100)) { setFormError('Score must be 0–100'); return }
    addMutation.mutate({ address: addr, venue: newVenue, discovery_score: score })
  }

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6 gap-3">
        <div className="flex items-center gap-3">
          <Search size={22} style={{ color: 'var(--color-accent)' }} />
          <div>
            <h1 className="text-xl font-bold" style={{ color: 'var(--color-text)' }}>Discovery</h1>
            <p className="text-xs mt-0.5" style={{ color: 'var(--color-text-muted)' }}>
              Shadow-tracking wallets — 0% capital at risk. Polled every 5 s.
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => refreshMutation.mutate()}
            disabled={refreshMutation.isPending}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors"
            style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-text)', border: '1px solid var(--color-border)', opacity: refreshMutation.isPending ? 0.6 : 1 }}
            title="Pull top wallets from Polymarket public leaderboard"
          >
            <RefreshCw size={14} className={refreshMutation.isPending ? 'animate-spin' : ''} />
            {refreshMutation.isPending ? 'Refreshing…' : 'Refresh from leaderboard'}
          </button>
          <button
            onClick={() => { setShowAdd((v) => !v); setFormError(null) }}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            <Plus size={14} />
            Add wallet
          </button>
        </div>
      </div>

      {/* Add wallet form */}
      {showAdd && (
        <form onSubmit={handleAdd} className="rounded-xl border p-4 mb-4 flex flex-wrap gap-3 items-end"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
          <div className="flex-1 min-w-[260px]">
            <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>Wallet address (0x…)</label>
            <input type="text" value={newAddr} onChange={(e) => setNewAddr(e.target.value)} placeholder="0xabc…"
              className="w-full px-3 py-1.5 rounded-lg text-sm font-mono border outline-none"
              style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }} autoFocus />
          </div>
          <div>
            <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>Venue</label>
            <select value={newVenue} onChange={(e) => setNewVenue(e.target.value)}
              className="px-3 py-1.5 rounded-lg text-sm border outline-none"
              style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}>
              <option value="polymarket">polymarket</option>
              <option value="hyperliquid">hyperliquid</option>
            </select>
          </div>
          <div>
            <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>Manual score (0–100)</label>
            <input type="number" min={0} max={100} step={0.1} value={newScore} onChange={(e) => setNewScore(e.target.value)}
              placeholder="optional" className="w-32 px-3 py-1.5 rounded-lg text-sm border outline-none"
              style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }} />
          </div>
          <button type="submit" disabled={addMutation.isPending}
            className="px-4 py-1.5 rounded-lg text-xs font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
            {addMutation.isPending ? 'Adding…' : 'Add'}
          </button>
          {formError && <p className="w-full text-xs" style={{ color: '#ef4444' }}>{formError}</p>}
        </form>
      )}

      {/* Filters */}
      <div className="mb-4 flex flex-wrap gap-3 items-center">
        <input type="text" placeholder="Filter by address…" value={filter} onChange={(e) => setFilter(e.target.value)}
          className="flex-1 max-w-md px-4 py-2 rounded-lg text-sm border outline-none"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }} />
        <div className="flex gap-1">
          {(['candidate', 'graduated', 'blacklisted', 'all'] as const).map((s) => (
            <button key={s} onClick={() => setStatusFilter(s)}
              className="px-3 py-1.5 rounded-lg text-xs font-medium transition-colors capitalize"
              style={{ backgroundColor: statusFilter === s ? 'var(--color-accent)' : 'var(--color-surface)', color: statusFilter === s ? '#000' : 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}>
              {s}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-1.5 text-xs" style={{ color: 'var(--color-text-muted)' }}>
          <Activity size={13} />
          <span>{candidates.length} wallets tracked</span>
        </div>
      </div>

      {/* Candidates */}
      {isLoading ? (
        <div className="text-sm" style={{ color: 'var(--color-text-muted)' }}>Loading candidates…</div>
      ) : filtered.length === 0 ? (
        <div className="rounded-xl border p-8 text-center" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
          <Award size={32} className="mx-auto mb-3" style={{ color: 'var(--color-text-muted)' }} />
          <p className="text-sm font-medium mb-1" style={{ color: 'var(--color-text)' }}>
            No candidates {statusFilter !== 'all' ? `with status "${statusFilter}"` : 'yet'}
          </p>
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            Click <b>Add wallet</b> or <b>Refresh from leaderboard</b>.
          </p>
        </div>
      ) : (
        <div className="grid gap-3">
          {filtered.map((candidate) => {
            const isExpanded = expandedAddr === candidate.wallet_address
            return (
              <div key={`${candidate.venue}:${candidate.wallet_address}`}
                className="rounded-xl border p-4 transition-all"
                style={{ backgroundColor: 'var(--color-surface)', borderColor: isExpanded ? 'var(--color-accent)' : 'var(--color-border)' }}>

                {/* Top row: address + badges + actions */}
                <div className="flex items-center justify-between gap-4">
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="text-sm font-mono font-medium" style={{ color: 'var(--color-text)' }}>
                      {candidate.wallet_address.slice(0, 10)}…{candidate.wallet_address.slice(-8)}
                    </span>
                    <span className="text-xs px-2 py-0.5 rounded-full" style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)' }}>
                      {candidate.venue}
                    </span>
                    <span className={clsx('text-xs px-2 py-0.5 rounded-full font-medium',
                      candidate.status === 'candidate' ? 'text-yellow-600' : candidate.status === 'graduated' ? 'text-green-600' : 'text-red-600')}
                      style={{ backgroundColor: candidate.status === 'candidate' ? 'rgba(234,179,8,.15)' : candidate.status === 'graduated' ? 'rgba(34,197,94,.15)' : 'rgba(239,68,68,.15)' }}>
                      {candidate.status}
                    </span>
                    <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                      since {new Date(candidate.discovered_at).toLocaleDateString()}
                    </span>
                  </div>

                  <div className="flex items-center gap-2 flex-shrink-0">
                    {candidate.status === 'candidate' && (
                      <>
                        <button onClick={() => graduateMutation.mutate(candidate.wallet_address)}
                          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium"
                          style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
                          title="Promote to active leader watchlist">
                          <GraduationCap size={14} /> Graduate
                        </button>
                        <button onClick={() => blacklistMutation.mutate(candidate.wallet_address)}
                          className="flex items-center gap-1.5 px-2 py-1.5 rounded-lg text-xs transition-colors hover:bg-red-500/15"
                          style={{ backgroundColor: 'var(--color-base)', color: '#ef4444', border: '1px solid var(--color-border)' }}
                          title="Blacklist">
                          <Ban size={14} />
                        </button>
                      </>
                    )}
                    {/* Expand / collapse */}
                    <button
                      onClick={() => setExpandedAddr(isExpanded ? null : candidate.wallet_address)}
                      className="flex items-center gap-1 px-2 py-1.5 rounded-lg text-xs transition-colors"
                      style={{ backgroundColor: isExpanded ? 'var(--color-accent)' : 'var(--color-base)', color: isExpanded ? '#000' : 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}
                      title="Show trades & stats">
                      <TrendingUp size={13} />
                      {isExpanded ? <ChevronUp size={12} /> : <ChevronDown size={12} />}
                    </button>
                    <button onClick={() => { if (confirm(`Remove ${candidate.wallet_address}?`)) removeMutation.mutate(candidate.wallet_address) }}
                      className="flex items-center px-2 py-1.5 rounded-lg text-xs transition-colors hover:bg-white/5"
                      style={{ backgroundColor: 'transparent', color: 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}
                      title="Delete">
                      <Trash2 size={14} />
                    </button>
                  </div>
                </div>

                {/* Expanded trades panel */}
                {isExpanded && <WalletStatsPanel address={candidate.wallet_address} />}
              </div>
            )
          })}
        </div>
      )}

      {/* Legend */}
      <div className="mt-6 flex items-center gap-4 text-xs" style={{ color: 'var(--color-text-muted)' }}>
        <div className="flex items-center gap-1.5"><Clock size={12} /> <span>Polling every 5 s · trades stored in <code>wallet_trades</code></span></div>
        <div className="flex items-center gap-1.5"><TrendingUp size={12} /> <span>Click the chart icon on any wallet to see its last 10 trades</span></div>
      </div>
    </div>
  )
}
