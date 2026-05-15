import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiPatch, apiDelete } from '../hooks/useApi'
import {
  Users,
  ToggleLeft,
  ToggleRight,
  TrendingUp,
  Shield,
  AlertCircle,
  Plus,
  Trash2,
  Pencil,
  Check,
  X,
  BarChart2,
  List,
  Activity,
  DollarSign,
} from 'lucide-react'
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
  const [activeTab, setActiveTab] = useState<'leaders' | 'positions' | 'history' | 'consensus'>('leaders')
  const [showAdd, setShowAdd] = useState(false)
  const [editing, setEditing] = useState<string | null>(null)
  const [editSize, setEditSize] = useState('')
  const [editWeight, setEditWeight] = useState('')
  const [editCategory, setEditCategory] = useState('')

  // Score breakdown modal
  const [scoreAddr, setScoreAddr] = useState<string | null>(null)
  // Fill audit modal
  const [tradesAddr, setTradesAddr] = useState<string | null>(null)

  // Capital under management
  const [capitalInput, setCapitalInput] = useState('')
  const [capitalEditOpen, setCapitalEditOpen] = useState(false)

  const [newAddr, setNewAddr] = useState('')
  const [newVenue, setNewVenue] = useState('polymarket')
  const [newCategory, setNewCategory] = useState('')
  const [newSize, setNewSize] = useState('0.5')
  const [newWeight, setNewWeight] = useState('1.0')
  const [newScore, setNewScore] = useState('')
  const [newMirror, setNewMirror] = useState(false)
  const [formError, setFormError] = useState<string | null>(null)

  const { data: leadersData, isLoading: leadersLoading } = useQuery({
    queryKey: ['copy-leaders'],
    queryFn: () => apiFetch<{ leaders: Leader[] }>('/api/copy/leaders'),
  })

  const { data: positionsData, isLoading: positionsLoading } = useQuery({
    queryKey: ['copy-positions'],
    queryFn: () => apiFetch<{ positions: MirrorPosition[] }>('/api/copy/positions'),
  })

  const { data: historyData } = useQuery({
    queryKey: ['copy-positions-history'],
    queryFn: () => apiFetch<{ positions: (MirrorPosition & { closed_at?: string; pnl?: number })[] }>('/api/copy/positions/history'),
    enabled: activeTab === 'history',
  })

  const { data: consensusData } = useQuery({
    queryKey: ['copy-consensus'],
    queryFn: () => apiFetch<{ windows: { symbol: string; side: string; leader_count: number; first_seen: string; last_seen: string }[] }>('/api/copy/consensus'),
    enabled: activeTab === 'consensus',
    refetchInterval: 10_000,
  })

  const { data: capitalData } = useQuery({
    queryKey: ['copy-capital'],
    queryFn: () => apiFetch<{ capital_usd: number }>('/api/copy/capital'),
  })

  const { data: scoreData } = useQuery({
    queryKey: ['copy-score', scoreAddr],
    queryFn: () => apiFetch<Record<string, unknown>>(`/api/copy/score/${scoreAddr}`),
    enabled: !!scoreAddr,
  })

  const { data: tradesData } = useQuery({
    queryKey: ['copy-leader-trades', tradesAddr],
    queryFn: () => apiFetch<{ trades: Record<string, unknown>[] }>(`/api/copy/leaders/${tradesAddr}/trades`),
    enabled: !!tradesAddr,
  })

  const capitalMutation = useMutation({
    mutationFn: (capital: number) => apiPost('/api/copy/capital', { capital_usd: capital }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['copy-capital'] })
      setCapitalEditOpen(false)
    },
  })

  const invalidateLeaders = () =>
    queryClient.invalidateQueries({ queryKey: ['copy-leaders'] })

  const toggleMutation = useMutation({
    mutationFn: (addr: string) =>
      apiPost<{ mirror_enabled: boolean }>(`/api/copy/leaders/${addr}/toggle`, {}),
    onSuccess: invalidateLeaders,
  })

  const addMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) => apiPost('/api/copy/leaders', body),
    onSuccess: () => {
      setShowAdd(false)
      setNewAddr('')
      setNewCategory('')
      setNewScore('')
      setFormError(null)
      invalidateLeaders()
    },
    onError: (err: Error) => setFormError(err.message),
  })

  const patchMutation = useMutation({
    mutationFn: (vars: { addr: string; body: Record<string, unknown> }) =>
      apiPatch(`/api/copy/leaders/${vars.addr}`, vars.body),
    onSuccess: () => {
      setEditing(null)
      invalidateLeaders()
    },
  })

  const removeMutation = useMutation({
    mutationFn: (addr: string) => apiDelete(`/api/copy/leaders/${addr}`),
    onSuccess: invalidateLeaders,
  })

  const leaders = leadersData?.leaders ?? []
  const positions = positionsData?.positions ?? []

  function handleAdd(e: React.FormEvent) {
    e.preventDefault()
    const addr = newAddr.trim().toLowerCase()
    if (!/^0x[0-9a-f]{40}$/.test(addr)) {
      setFormError('Address must be 0x + 40 hex chars')
      return
    }
    const size = Number(newSize)
    const weight = Number(newWeight)
    if (Number.isNaN(size) || size < 0 || size > 10) {
      setFormError('Size factor must be 0–10')
      return
    }
    if (Number.isNaN(weight) || weight < 0 || weight > 10) {
      setFormError('Consensus weight must be 0–10')
      return
    }
    const score = newScore.trim() === '' ? undefined : Number(newScore)
    if (score !== undefined && (Number.isNaN(score) || score < 0 || score > 100)) {
      setFormError('Score must be 0–100')
      return
    }
    addMutation.mutate({
      address: addr,
      venue: newVenue,
      category: newCategory.trim() || null,
      size_factor: size,
      consensus_weight: weight,
      wallet_score: score,
      mirror_enabled: newMirror,
    })
  }

  function startEdit(leader: Leader) {
    setEditing(leader.address)
    setEditSize(leader.size_factor.toString())
    setEditWeight(leader.consensus_weight.toString())
    setEditCategory(leader.category ?? '')
  }

  function saveEdit(addr: string) {
    const size = Number(editSize)
    const weight = Number(editWeight)
    if (Number.isNaN(size) || Number.isNaN(weight)) return
    patchMutation.mutate({
      addr,
      body: {
        size_factor: size,
        consensus_weight: weight,
        category: editCategory.trim() || null,
      },
    })
  }

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6 gap-3">
        <div className="flex items-center gap-3">
          <Users size={22} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-xl font-bold" style={{ color: 'var(--color-text)' }}>
            Copy Trading
          </h1>
        </div>
        <div className="flex items-center gap-2">
          {activeTab === 'leaders' && (
            <button
              onClick={() => {
                setShowAdd((v) => !v)
                setFormError(null)
              }}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              <Plus size={14} />
              Add leader
            </button>
          )}
          <div className="flex items-center gap-2 flex-wrap">
            {/* Capital under management */}
            {capitalEditOpen ? (
              <form
                onSubmit={(e) => { e.preventDefault(); const v = Number(capitalInput); if (!Number.isNaN(v) && v >= 0) capitalMutation.mutate(v) }}
                className="flex items-center gap-1"
              >
                <input
                  type="number"
                  min={0}
                  step={100}
                  value={capitalInput}
                  onChange={(e) => setCapitalInput(e.target.value)}
                  placeholder="Capital USD"
                  className="w-28 rounded px-2 py-1 text-xs font-mono"
                  style={{ backgroundColor: 'var(--color-base)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                  autoFocus
                />
                <button type="submit" className="px-2 py-1 rounded text-xs font-semibold" style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
                  Set
                </button>
                <button type="button" onClick={() => setCapitalEditOpen(false)} className="px-2 py-1 rounded text-xs" style={{ color: 'var(--color-text-muted)' }}>
                  Cancel
                </button>
              </form>
            ) : (
              <button
                onClick={() => { setCapitalInput(String(capitalData?.capital_usd ?? '')); setCapitalEditOpen(true) }}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium"
                style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}
              >
                <DollarSign size={12} />
                Capital: {capitalData?.capital_usd != null ? `$${capitalData.capital_usd.toLocaleString()}` : '—'}
              </button>
            )}
            <div
              className="flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-medium"
              style={{
                backgroundColor: 'var(--color-surface)',
                color: 'var(--color-text-muted)',
                border: '1px solid var(--color-border)',
              }}
            >
              <Shield size={14} />
              <span>Discovery Mode</span>
            </div>
          </div>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 mb-6 flex-wrap">
        {([
          { id: 'leaders', label: 'Leader Studio' },
          { id: 'positions', label: 'Mirror Positions' },
          { id: 'history', label: 'Closed Positions' },
          { id: 'consensus', label: 'Consensus' },
        ] as const).map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={clsx(
              'px-4 py-2 rounded-lg text-sm font-medium transition-colors',
              activeTab === tab.id ? 'text-black' : 'hover:bg-white/5'
            )}
            style={{
              backgroundColor:
                activeTab === tab.id ? 'var(--color-accent)' : 'transparent',
              color: activeTab === tab.id ? '#000' : 'var(--color-text-muted)',
            }}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Add leader form */}
      {activeTab === 'leaders' && showAdd && (
        <form
          onSubmit={handleAdd}
          className="rounded-xl border p-4 mb-4 grid gap-3"
          style={{
            backgroundColor: 'var(--color-surface)',
            borderColor: 'var(--color-border)',
          }}
        >
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            Bypass Discovery and add a leader you already trust. Mirror is off by default.
          </p>
          <div className="flex flex-wrap gap-3 items-end">
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
                Category
              </label>
              <input
                type="text"
                value={newCategory}
                onChange={(e) => setNewCategory(e.target.value)}
                placeholder="optional"
                className="w-32 px-3 py-1.5 rounded-lg text-sm border outline-none"
                style={{
                  backgroundColor: 'var(--color-base)',
                  borderColor: 'var(--color-border)',
                  color: 'var(--color-text)',
                }}
              />
            </div>
          </div>
          <div className="flex flex-wrap gap-3 items-end">
            <div>
              <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>
                Size factor (×)
              </label>
              <input
                type="number"
                min={0}
                max={10}
                step={0.05}
                value={newSize}
                onChange={(e) => setNewSize(e.target.value)}
                className="w-24 px-3 py-1.5 rounded-lg text-sm border outline-none"
                style={{
                  backgroundColor: 'var(--color-base)',
                  borderColor: 'var(--color-border)',
                  color: 'var(--color-text)',
                }}
              />
            </div>
            <div>
              <label className="block text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>
                Consensus weight
              </label>
              <input
                type="number"
                min={0}
                max={10}
                step={0.1}
                value={newWeight}
                onChange={(e) => setNewWeight(e.target.value)}
                className="w-24 px-3 py-1.5 rounded-lg text-sm border outline-none"
                style={{
                  backgroundColor: 'var(--color-base)',
                  borderColor: 'var(--color-border)',
                  color: 'var(--color-text)',
                }}
              />
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
                className="w-28 px-3 py-1.5 rounded-lg text-sm border outline-none"
                style={{
                  backgroundColor: 'var(--color-base)',
                  borderColor: 'var(--color-border)',
                  color: 'var(--color-text)',
                }}
              />
            </div>
            <label
              className="flex items-center gap-2 text-xs px-3 py-1.5"
              style={{ color: 'var(--color-text-muted)' }}
            >
              <input
                type="checkbox"
                checked={newMirror}
                onChange={(e) => setNewMirror(e.target.checked)}
              />
              Mirror immediately
            </label>
            <button
              type="submit"
              disabled={addMutation.isPending}
              className="ml-auto px-4 py-1.5 rounded-lg text-xs font-medium"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              {addMutation.isPending ? 'Adding…' : 'Add leader'}
            </button>
          </div>
          {formError && (
            <p className="text-xs" style={{ color: '#ef4444' }}>
              {formError}
            </p>
          )}
        </form>
      )}

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
                Graduate candidates from Discovery, or click <b>Add leader</b> to register a wallet
                you already trust.
              </p>
            </div>
          ) : (
            <div className="grid gap-3">
              {leaders.map((leader) => {
                const isEditing = editing === leader.address
                return (
                  <div
                    key={leader.address}
                    className="rounded-xl border p-4"
                    style={{
                      backgroundColor: 'var(--color-surface)',
                      borderColor: 'var(--color-border)',
                    }}
                  >
                    <div className="flex items-center justify-between gap-3">
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2 mb-1 flex-wrap">
                          <span
                            className="text-sm font-mono font-medium truncate"
                            style={{ color: 'var(--color-text)' }}
                          >
                            {leader.address.slice(0, 8)}…{leader.address.slice(-6)}
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
                          {!isEditing && leader.category && (
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
                        <div className="flex items-center gap-4 text-xs flex-wrap">
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
                          {isEditing ? (
                            <>
                              <label
                                className="flex items-center gap-1"
                                style={{ color: 'var(--color-text-muted)' }}
                              >
                                Size×
                                <input
                                  type="number"
                                  min={0}
                                  max={10}
                                  step={0.05}
                                  value={editSize}
                                  onChange={(e) => setEditSize(e.target.value)}
                                  className="w-20 px-2 py-0.5 rounded text-xs border outline-none"
                                  style={{
                                    backgroundColor: 'var(--color-base)',
                                    borderColor: 'var(--color-border)',
                                    color: 'var(--color-text)',
                                  }}
                                />
                              </label>
                              <label
                                className="flex items-center gap-1"
                                style={{ color: 'var(--color-text-muted)' }}
                              >
                                Weight
                                <input
                                  type="number"
                                  min={0}
                                  max={10}
                                  step={0.1}
                                  value={editWeight}
                                  onChange={(e) => setEditWeight(e.target.value)}
                                  className="w-20 px-2 py-0.5 rounded text-xs border outline-none"
                                  style={{
                                    backgroundColor: 'var(--color-base)',
                                    borderColor: 'var(--color-border)',
                                    color: 'var(--color-text)',
                                  }}
                                />
                              </label>
                              <label
                                className="flex items-center gap-1"
                                style={{ color: 'var(--color-text-muted)' }}
                              >
                                Category
                                <input
                                  type="text"
                                  value={editCategory}
                                  onChange={(e) => setEditCategory(e.target.value)}
                                  placeholder="—"
                                  className="w-28 px-2 py-0.5 rounded text-xs border outline-none"
                                  style={{
                                    backgroundColor: 'var(--color-base)',
                                    borderColor: 'var(--color-border)',
                                    color: 'var(--color-text)',
                                  }}
                                />
                              </label>
                            </>
                          ) : (
                            <>
                              <span style={{ color: 'var(--color-text-muted)' }}>
                                Size factor: {leader.size_factor.toFixed(2)}x
                              </span>
                              <span style={{ color: 'var(--color-text-muted)' }}>
                                Weight: {leader.consensus_weight.toFixed(2)}
                              </span>
                            </>
                          )}
                        </div>
                      </div>

                      <div className="flex items-center gap-2 ml-2 flex-shrink-0">
                        {isEditing ? (
                          <>
                            <button
                              onClick={() => saveEdit(leader.address)}
                              className="p-1.5 rounded-lg"
                              style={{
                                backgroundColor: 'var(--color-accent)',
                                color: '#000',
                              }}
                              title="Save"
                            >
                              <Check size={14} />
                            </button>
                            <button
                              onClick={() => setEditing(null)}
                              className="p-1.5 rounded-lg"
                              style={{
                                backgroundColor: 'var(--color-base)',
                                color: 'var(--color-text-muted)',
                                border: '1px solid var(--color-border)',
                              }}
                              title="Cancel"
                            >
                              <X size={14} />
                            </button>
                          </>
                        ) : (
                          <>
                            <button
                              onClick={() => toggleMutation.mutate(leader.address)}
                              className="flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors"
                              style={{
                                backgroundColor: leader.mirror_enabled
                                  ? 'rgba(34, 197, 94, 0.15)'
                                  : 'var(--color-base)',
                                color: leader.mirror_enabled
                                  ? '#22c55e'
                                  : 'var(--color-text-muted)',
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
                            <button
                              onClick={() => setScoreAddr(leader.address)}
                              className="p-1.5 rounded-lg hover:bg-white/5"
                              style={{ backgroundColor: 'transparent', color: 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}
                              title="Score breakdown"
                            >
                              <BarChart2 size={14} />
                            </button>
                            <button
                              onClick={() => setTradesAddr(leader.address)}
                              className="p-1.5 rounded-lg hover:bg-white/5"
                              style={{ backgroundColor: 'transparent', color: 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}
                              title="Fill audit"
                            >
                              <List size={14} />
                            </button>
                            <button
                              onClick={() => startEdit(leader)}
                              className="p-1.5 rounded-lg hover:bg-white/5"
                              style={{
                                backgroundColor: 'transparent',
                                color: 'var(--color-text-muted)',
                                border: '1px solid var(--color-border)',
                              }}
                              title="Edit"
                            >
                              <Pencil size={14} />
                            </button>
                            <button
                              onClick={() => {
                                if (confirm(`Remove leader ${leader.address}?`)) {
                                  removeMutation.mutate(leader.address)
                                }
                              }}
                              className="p-1.5 rounded-lg hover:bg-red-500/15"
                              style={{
                                backgroundColor: 'transparent',
                                color: '#ef4444',
                                border: '1px solid var(--color-border)',
                              }}
                              title="Remove"
                            >
                              <Trash2 size={14} />
                            </button>
                          </>
                        )}
                      </div>
                    </div>
                  </div>
                )
              })}
            </div>
          )}
        </div>
      )}

      {/* History Tab */}
      {activeTab === 'history' && (
        <div>
          {(historyData?.positions ?? []).length === 0 ? (
            <div className="rounded-xl border p-8 text-center" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
              <TrendingUp size={32} className="mx-auto mb-3" style={{ color: 'var(--color-text-muted)' }} />
              <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>No closed positions yet</p>
            </div>
          ) : (
            <div className="grid gap-3">
              {historyData!.positions.map((pos, i) => (
                <div key={i} className="rounded-xl border p-4" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
                  <div className="flex items-center justify-between mb-2">
                    <span className="text-sm font-medium" style={{ color: 'var(--color-text)' }}>{pos.symbol}</span>
                    <span className="text-xs font-mono" style={{ color: pos.pnl != null && pos.pnl >= 0 ? 'var(--color-accent)' : 'var(--color-danger)' }}>
                      {pos.pnl != null ? `${pos.pnl >= 0 ? '+' : ''}$${pos.pnl.toFixed(2)}` : '—'}
                    </span>
                  </div>
                  <div className="grid grid-cols-3 gap-4 text-xs">
                    <div><span style={{ color: 'var(--color-text-muted)' }}>Side</span><p style={{ color: 'var(--color-text)' }}>{pos.side}</p></div>
                    <div><span style={{ color: 'var(--color-text-muted)' }}>Closed</span><p style={{ color: 'var(--color-text)' }}>{pos.closed_at ? new Date(pos.closed_at).toLocaleDateString() : '—'}</p></div>
                    <div><span style={{ color: 'var(--color-text-muted)' }}>Notional</span><p style={{ color: 'var(--color-text)' }}>${pos.notional?.toFixed(2) ?? '—'}</p></div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Consensus Tab */}
      {activeTab === 'consensus' && (
        <div>
          {(consensusData?.windows ?? []).length === 0 ? (
            <div className="rounded-xl border p-8 text-center" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
              <Activity size={32} className="mx-auto mb-3" style={{ color: 'var(--color-text-muted)' }} />
              <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>No active consensus signals in the last 5 minutes</p>
            </div>
          ) : (
            <div className="grid gap-3">
              {consensusData!.windows.map((w, i) => (
                <div key={i} className="rounded-xl border p-4" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
                  <div className="flex items-center justify-between mb-2">
                    <span className="text-sm font-medium" style={{ color: 'var(--color-text)' }}>{w.symbol}</span>
                    <span className="text-xs px-2 py-0.5 rounded-full font-semibold" style={{ backgroundColor: w.side === 'buy' ? 'rgba(34,197,94,0.15)' : 'rgba(239,68,68,0.15)', color: w.side === 'buy' ? '#22c55e' : '#ef4444' }}>
                      {w.side.toUpperCase()}
                    </span>
                  </div>
                  <div className="grid grid-cols-3 gap-4 text-xs">
                    <div><span style={{ color: 'var(--color-text-muted)' }}>Leaders agreeing</span><p className="font-mono font-medium" style={{ color: 'var(--color-accent)' }}>{w.leader_count}</p></div>
                    <div><span style={{ color: 'var(--color-text-muted)' }}>First seen</span><p style={{ color: 'var(--color-text)' }}>{new Date(w.first_seen).toLocaleTimeString()}</p></div>
                    <div><span style={{ color: 'var(--color-text-muted)' }}>Last seen</span><p style={{ color: 'var(--color-text)' }}>{new Date(w.last_seen).toLocaleTimeString()}</p></div>
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
                        {pos.leader_address.slice(0, 6)}…{pos.leader_address.slice(-4)}
                      </p>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
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
            ) : scoreData.error ? (
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
                  <span style={{ color: 'var(--color-text)' }}>Wallet Score</span>
                  <span style={{ color: 'var(--color-accent)' }}>{scoreData.wallet_score != null ? Number(scoreData.wallet_score).toFixed(1) : '—'}</span>
                </div>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Fill Audit Modal */}
      {tradesAddr && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setTradesAddr(null)}>
          <div className="rounded-xl border w-full max-w-lg max-h-[75vh] flex flex-col" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }} onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between p-4 border-b" style={{ borderColor: 'var(--color-border)' }}>
              <div>
                <h3 className="text-sm font-semibold">Fill Audit</h3>
                <p className="text-xs font-mono" style={{ color: 'var(--color-text-muted)' }}>{tradesAddr.slice(0, 10)}…{tradesAddr.slice(-8)}</p>
              </div>
              <button onClick={() => setTradesAddr(null)}><X size={14} style={{ color: 'var(--color-text-muted)' }} /></button>
            </div>
            <div className="overflow-auto flex-1 p-3">
              {!tradesData ? (
                <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>Loading…</p>
              ) : tradesData.trades.length === 0 ? (
                <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>No recorded fills for this wallet.</p>
              ) : (
                <table className="w-full text-xs">
                  <thead>
                    <tr style={{ color: 'var(--color-text-muted)' }}>
                      <th className="text-left pb-2">Time</th>
                      <th className="text-left pb-2">Market</th>
                      <th className="text-left pb-2">Side</th>
                      <th className="text-right pb-2">Notional</th>
                      <th className="text-right pb-2">PnL</th>
                    </tr>
                  </thead>
                  <tbody>
                    {tradesData.trades.map((t, i) => (
                      <tr key={i} className="border-t" style={{ borderColor: 'var(--color-border)' }}>
                        <td className="py-1.5" style={{ color: 'var(--color-text-muted)' }}>{t.timestamp ? new Date(String(t.timestamp)).toLocaleDateString() : '—'}</td>
                        <td className="py-1.5 font-mono truncate max-w-[80px]" style={{ color: 'var(--color-text)' }}>{String(t.market_id ?? '—').slice(0, 12)}</td>
                        <td className="py-1.5" style={{ color: String(t.side) === 'buy' ? '#22c55e' : '#ef4444' }}>{String(t.side ?? '—').toUpperCase()}</td>
                        <td className="py-1.5 text-right font-mono" style={{ color: 'var(--color-text)' }}>{t.notional != null ? `$${Number(t.notional).toFixed(2)}` : '—'}</td>
                        <td className="py-1.5 text-right font-mono" style={{ color: t.pnl != null && Number(t.pnl) >= 0 ? 'var(--color-accent)' : 'var(--color-danger)' }}>
                          {t.pnl != null ? `${Number(t.pnl) >= 0 ? '+' : ''}$${Number(t.pnl).toFixed(2)}` : '—'}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
