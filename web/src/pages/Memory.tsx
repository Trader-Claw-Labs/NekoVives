import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import { Brain, Search, Plus, Trash2, RefreshCw, X, CheckSquare, Square } from 'lucide-react'

// ── Types ─────────────────────────────────────────────────────────────

interface MemoryEntry {
  key: string
  content: string
  category?: string
  created_at?: string
  updated_at?: string
}

interface MemoryListResponse {
  entries: MemoryEntry[]
}

// ── Helpers ───────────────────────────────────────────────────────────

const CATEGORIES = ['core', 'daily', 'conversation', 'custom']

function categoryColor(cat?: string) {
  if (cat === 'core') return 'var(--color-accent)'
  if (cat === 'daily') return '#f59e0b'
  if (cat === 'conversation') return '#a78bfa'
  return 'var(--color-text-muted)'
}

function fmt(iso?: string) {
  if (!iso) return ''
  try { return new Date(iso).toLocaleString() } catch { return iso }
}

// ── Add Entry Modal ───────────────────────────────────────────────────

function AddModal({ onClose, onAdded }: { onClose: () => void; onAdded: () => void }) {
  const [key, setKey] = useState('')
  const [content, setContent] = useState('')
  const [category, setCategory] = useState('core')
  const [error, setError] = useState('')

  const mutation = useMutation({
    mutationFn: () => apiPost('/api/memory', { key, content, category }),
    onSuccess: () => { onAdded(); onClose() },
    onError: (e: Error) => setError(e.message),
  })

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div className="rounded-lg border w-full max-w-md"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
        <div className="flex items-center justify-between p-4 border-b" style={{ borderColor: 'var(--color-border)' }}>
          <h2 className="font-semibold flex items-center gap-2"><Brain size={15} /> New Memory</h2>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}><X size={16} /></button>
        </div>
        <div className="p-4 space-y-3">
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Key</label>
            <input className="w-full rounded px-3 py-2 text-sm font-mono" value={key}
              onChange={e => setKey(e.target.value)} placeholder="unique-key" />
          </div>
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Category</label>
            <select className="w-full rounded px-3 py-2 text-sm" value={category}
              onChange={e => setCategory(e.target.value)}>
              {CATEGORIES.map(c => <option key={c} value={c}>{c}</option>)}
            </select>
          </div>
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Content</label>
            <textarea className="w-full rounded px-3 py-2 text-sm h-28 resize-y leading-relaxed"
              value={content} onChange={e => setContent(e.target.value)}
              placeholder="Memory content..."
              style={{ backgroundColor: 'var(--color-base)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }} />
          </div>
          {error && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{error}</p>}
        </div>
        <div className="p-4 border-t flex gap-2" style={{ borderColor: 'var(--color-border)' }}>
          <button onClick={() => mutation.mutate()} disabled={!key || !content || mutation.isPending}
            className="flex-1 py-2 rounded text-sm font-medium disabled:opacity-50"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
            {mutation.isPending ? 'Saving...' : 'Save'}
          </button>
          <button onClick={onClose} className="px-4 py-2 rounded text-sm border hover:bg-white/5"
            style={{ borderColor: 'var(--color-border)' }}>Cancel</button>
        </div>
      </div>
    </div>
  )
}

// ── Entry Card ────────────────────────────────────────────────────────

interface EntryCardProps {
  entry: MemoryEntry
  selected: boolean
  onToggleSelect: () => void
  onDelete: () => void
  selectMode: boolean
}

function EntryCard({ entry, selected, onToggleSelect, onDelete, selectMode }: EntryCardProps) {
  const [expanded, setExpanded] = useState(false)
  const preview = entry.content.length > 120 ? entry.content.slice(0, 120) + '…' : entry.content

  return (
    <div
      className="rounded-lg border p-3 transition-colors"
      style={{
        backgroundColor: selected ? 'rgba(74,222,128,0.07)' : 'var(--color-surface)',
        borderColor: selected ? 'var(--color-accent)' : 'var(--color-border)',
      }}
    >
      <div className="flex items-start justify-between gap-2 mb-1.5">
        <div className="flex items-center gap-2 min-w-0">
          {/* Checkbox for multi-select */}
          <button
            onClick={onToggleSelect}
            className="flex-shrink-0 p-0.5 rounded hover:bg-white/5"
            style={{ color: selected ? 'var(--color-accent)' : 'var(--color-text-muted)' }}
          >
            {selected ? <CheckSquare size={13} /> : <Square size={13} />}
          </button>
          <span className="font-mono text-xs font-semibold truncate">{entry.key}</span>
          {entry.category && (
            <span className="text-xs px-1.5 py-0.5 rounded flex-shrink-0"
              style={{ backgroundColor: 'var(--color-base)', color: categoryColor(entry.category) }}>
              {entry.category}
            </span>
          )}
        </div>
        {!selectMode && (
          <button onClick={onDelete} className="p-1 rounded hover:bg-white/5 flex-shrink-0"
            style={{ color: 'var(--color-text-muted)' }}>
            <Trash2 size={12} />
          </button>
        )}
      </div>

      <p className="text-xs leading-relaxed cursor-pointer pl-6" style={{ color: 'var(--color-text-muted)' }}
        onClick={() => setExpanded(e => !e)}>
        {expanded ? entry.content : preview}
      </p>

      {entry.updated_at && (
        <p className="text-xs mt-1.5 pl-6" style={{ color: 'var(--color-text-muted)', opacity: 0.5 }}>
          {fmt(entry.updated_at)}
        </p>
      )}
    </div>
  )
}

// ── Main Page ─────────────────────────────────────────────────────────

export default function Memory() {
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState('')
  const [showAdd, setShowAdd] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const qc = useQueryClient()

  // Use a consistent prefix so invalidation works correctly
  const queryKey = search
    ? ['memory', 'search', search]
    : ['memory', 'list', category]

  const { data, isLoading, refetch } = useQuery<MemoryListResponse>({
    queryKey,
    queryFn: () => {
      const params = new URLSearchParams()
      if (search) params.set('query', search)
      else if (category) params.set('category', category)
      return apiFetch<MemoryListResponse>(`/api/memory${params.size ? '?' + params : ''}`)
        .catch(() => ({ entries: [] }))
    },
    refetchInterval: 30_000,
    select: (data) => {
      // Deduplicate by key
      const seen = new Set<string>()
      const unique = (data.entries ?? []).filter(e => {
        if (seen.has(e.key)) return false
        seen.add(e.key)
        return true
      })
      return { entries: unique }
    },
  })

  const deleteMutation = useMutation({
    mutationFn: (key: string) => apiDelete(`/api/memory/${encodeURIComponent(key)}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['memory'] }),
  })

  const bulkDeleteMutation = useMutation({
    mutationFn: async (keys: string[]) => {
      for (const key of keys) {
        await apiDelete(`/api/memory/${encodeURIComponent(key)}`)
      }
    },
    onSuccess: () => {
      setSelected(new Set())
      qc.invalidateQueries({ queryKey: ['memory'] })
    },
  })

  const entries = data?.entries ?? []
  const selectMode = selected.size > 0

  function toggleSelect(key: string) {
    setSelected(prev => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  function selectAll() {
    if (selected.size === entries.length) setSelected(new Set())
    else setSelected(new Set(entries.map(e => e.key)))
  }

  function handleDelete(key: string) {
    if (confirm(`Delete memory "${key}"?`)) {
      deleteMutation.mutate(key, {
        onSuccess: () => {
          // Also remove from local selected set
          setSelected(prev => { const n = new Set(prev); n.delete(key); return n })
        },
      })
    }
  }

  function handleBulkDelete() {
    const keys = Array.from(selected)
    if (confirm(`Delete ${keys.length} selected memories?`)) {
      bulkDeleteMutation.mutate(keys)
    }
  }

  return (
    <div className="p-6 max-w-4xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-2">
          <Brain size={18} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-lg font-bold">Memory</h1>
          <span className="text-xs px-2 py-0.5 rounded"
            style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-text-muted)' }}>
            {entries.length} entries
          </span>
        </div>
        <div className="flex gap-2 items-center">
          {selectMode && (
            <>
              <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                {selected.size} selected
              </span>
              <button
                onClick={handleBulkDelete}
                disabled={bulkDeleteMutation.isPending}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded text-sm font-medium disabled:opacity-50"
                style={{ backgroundColor: 'rgba(239,68,68,0.15)', color: 'var(--color-danger)', border: '1px solid rgba(239,68,68,0.3)' }}
              >
                <Trash2 size={13} />
                {bulkDeleteMutation.isPending ? 'Deleting...' : 'Delete selected'}
              </button>
              <button
                onClick={() => setSelected(new Set())}
                className="px-3 py-1.5 rounded text-sm border hover:bg-white/5"
                style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
              >
                Cancel
              </button>
            </>
          )}
          <button onClick={() => refetch()} className="p-2 rounded border hover:bg-white/5"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
            <RefreshCw size={13} className={isLoading ? 'animate-spin' : ''} />
          </button>
          <button onClick={() => setShowAdd(true)}
            className="flex items-center gap-2 px-3 py-1.5 rounded text-sm font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
            <Plus size={14} /> New Entry
          </button>
        </div>
      </div>

      {/* Filters */}
      <div className="flex gap-2 mb-4">
        <div className="relative flex-1">
          <Search size={13} className="absolute left-3 top-1/2 -translate-y-1/2"
            style={{ color: 'var(--color-text-muted)' }} />
          <input className="w-full rounded pl-8 pr-3 py-2 text-sm"
            placeholder="Search memories..." value={search}
            onChange={e => { setSearch(e.target.value); setSelected(new Set()) }} />
        </div>
        {!search && (
          <select className="rounded px-3 py-2 text-sm" value={category}
            onChange={e => { setCategory(e.target.value); setSelected(new Set()) }}>
            <option value="">All categories</option>
            {CATEGORIES.map(c => <option key={c} value={c}>{c}</option>)}
          </select>
        )}
        {entries.length > 0 && (
          <button
            onClick={selectAll}
            className="px-3 py-2 rounded text-sm border hover:bg-white/5 flex-shrink-0"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
          >
            {selected.size === entries.length ? 'Deselect all' : 'Select all'}
          </button>
        )}
      </div>

      {/* Entries */}
      {isLoading ? (
        <div className="text-sm text-center py-12" style={{ color: 'var(--color-text-muted)' }}>Loading...</div>
      ) : entries.length === 0 ? (
        <div className="text-center py-16">
          <Brain size={40} className="mx-auto mb-3 opacity-20" />
          <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
            {search ? 'No memories matching your search' : 'No memories stored yet'}
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
          {entries.map(entry => (
            <EntryCard
              key={entry.key}
              entry={entry}
              selected={selected.has(entry.key)}
              onToggleSelect={() => toggleSelect(entry.key)}
              onDelete={() => handleDelete(entry.key)}
              selectMode={selectMode}
            />
          ))}
        </div>
      )}

      {showAdd && (
        <AddModal
          onClose={() => setShowAdd(false)}
          onAdded={() => qc.invalidateQueries({ queryKey: ['memory'] })}
        />
      )}
    </div>
  )
}
