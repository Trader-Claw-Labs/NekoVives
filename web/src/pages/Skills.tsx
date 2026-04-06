import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import {
  Zap, Clock, Play, Pause, Settings, Plus, Trash2, RefreshCw, X
} from 'lucide-react'
import clsx from 'clsx'

interface CronJob {
  id: string
  name?: string
  command: string
  next_run?: string
  last_run?: string
  last_status?: string
  enabled: boolean
}

interface CronResponse {
  jobs?: CronJob[]
}

const PRESET_SKILLS = [
  {
    id: 'market_scanner',
    name: 'Market Scanner',
    description: 'Scan TV indicators every 5 minutes',
    schedule: '*/5 * * * *',
    command: 'scan_markets',
    icon: '📊',
  },
  {
    id: 'polymarket_monitor',
    name: 'Polymarket Monitor',
    description: 'Monitor top crypto prediction markets',
    schedule: '*/10 * * * *',
    command: 'monitor_polymarket',
    icon: '🎯',
  },
  {
    id: 'rsi_alert',
    name: 'RSI Alert',
    description: 'Alert when RSI < 30 or > 70',
    schedule: '*/15 * * * *',
    command: 'check_rsi',
    icon: '📈',
  },
  {
    id: 'price_alert',
    name: 'Price Alert',
    description: 'Monitor custom price targets',
    schedule: '*/1 * * * *',
    command: 'check_prices',
    icon: '💰',
  },
]

function formatDate(iso?: string): string {
  if (!iso) return '—'
  try {
    return new Date(iso).toLocaleString()
  } catch {
    return iso
  }
}

interface AddModalProps {
  onClose: () => void
  onAdded: () => void
  preset?: typeof PRESET_SKILLS[0]
}

function AddModal({ onClose, onAdded, preset }: AddModalProps) {
  const [name, setName] = useState(preset?.name ?? '')
  const [schedule, setSchedule] = useState(preset?.schedule ?? '*/5 * * * *')
  const [command, setCommand] = useState(preset?.command ?? '')
  const [error, setError] = useState('')

  const mutation = useMutation({
    mutationFn: () =>
      apiPost('/api/cron', { name, schedule, command }),
    onSuccess: () => { onAdded(); onClose() },
    onError: (e: Error) => setError(e.message),
  })

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div
        className="rounded-lg border w-full max-w-md p-6"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center justify-between mb-4">
          <h2 className="font-semibold">{preset ? `Add: ${preset.name}` : 'Add Custom Strategy'}</h2>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>

        <div className="space-y-3 mb-4">
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Name</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm"
              placeholder="Strategy name"
            />
          </div>
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
              Schedule (cron expression)
            </label>
            <input
              type="text"
              value={schedule}
              onChange={(e) => setSchedule(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm font-mono"
              placeholder="*/5 * * * *"
            />
          </div>
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Command</label>
            <input
              type="text"
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm font-mono"
              placeholder="shell command or agent prompt"
            />
          </div>
        </div>

        {error && <p className="text-xs mb-3" style={{ color: 'var(--color-danger)' }}>{error}</p>}

        <div className="flex gap-2">
          <button
            onClick={() => mutation.mutate()}
            disabled={!command || !schedule || mutation.isPending}
            className="flex-1 py-2 rounded text-sm font-medium disabled:opacity-50"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            {mutation.isPending ? 'Adding...' : 'Add Strategy'}
          </button>
          <button
            onClick={onClose}
            className="px-4 py-2 rounded text-sm border hover:bg-white/5 transition-colors"
            style={{ borderColor: 'var(--color-border)' }}
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  )
}

interface JobCardProps {
  job: CronJob
  onDelete: (id: string) => void
}

function JobCard({ job, onDelete }: JobCardProps) {
  const isRunning = job.last_status === 'running'
  const hasFailed = job.last_status === 'failed' || job.last_status === 'error'

  return (
    <div
      className={clsx(
        'rounded-lg border p-4 card-hover transition-all',
        job.enabled ? '' : 'opacity-60'
      )}
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      <div className="flex items-start justify-between mb-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <span
              className={clsx(
                'status-dot',
                isRunning ? 'warning' : job.enabled ? 'online' : 'offline'
              )}
            />
            <h3 className="text-sm font-semibold truncate">{job.name ?? job.id}</h3>
          </div>
          <p
            className="text-xs font-mono truncate"
            style={{ color: 'var(--color-text-muted)' }}
          >
            {job.command}
          </p>
        </div>
        <button
          onClick={() => onDelete(job.id)}
          className="ml-2 p-1 rounded hover:bg-white/5 transition-colors"
          style={{ color: 'var(--color-text-muted)' }}
        >
          <Trash2 size={13} />
        </button>
      </div>

      <div className="space-y-1 text-xs mb-3">
        <div className="flex justify-between">
          <span style={{ color: 'var(--color-text-muted)' }}>Next run</span>
          <span>{formatDate(job.next_run)}</span>
        </div>
        <div className="flex justify-between">
          <span style={{ color: 'var(--color-text-muted)' }}>Last run</span>
          <span>{formatDate(job.last_run)}</span>
        </div>
        {job.last_status && (
          <div className="flex justify-between">
            <span style={{ color: 'var(--color-text-muted)' }}>Status</span>
            <span style={{ color: hasFailed ? 'var(--color-danger)' : 'var(--color-accent)' }}>
              {job.last_status}
            </span>
          </div>
        )}
      </div>

      <div className="flex items-center gap-2">
        <div
          className={clsx(
            'flex-1 h-1 rounded-full',
          )}
          style={{ backgroundColor: job.enabled ? 'var(--color-accent-dim)' : 'var(--color-border)' }}
        >
          <div
            className="h-full rounded-full"
            style={{
              width: job.enabled ? '100%' : '0%',
              backgroundColor: 'var(--color-accent)',
              transition: 'width 0.3s',
            }}
          />
        </div>
        <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
          {job.enabled ? 'active' : 'paused'}
        </span>
      </div>
    </div>
  )
}

export default function Skills() {
  const [showModal, setShowModal] = useState(false)
  const [addingPreset, setAddingPreset] = useState<typeof PRESET_SKILLS[0] | null>(null)
  const qc = useQueryClient()

  const { data, isLoading, refetch } = useQuery<CronResponse>({
    queryKey: ['skills'],
    queryFn: (): Promise<CronResponse> =>
      apiFetch<CronResponse>('/api/cron').catch(() => ({ jobs: [] })),
    refetchInterval: 15_000,
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => apiDelete(`/api/cron/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['skills'] }),
  })

  const jobs = data?.jobs ?? []

  const runningPresets = new Set(jobs.map((j) => j.command))

  return (
    <div className="p-6 max-w-5xl mx-auto">
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-2">
          <Zap size={18} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-lg font-bold">Skills</h1>
          <span
            className="text-xs px-2 py-0.5 rounded"
            style={{ backgroundColor: 'var(--color-accent-dim)', color: 'var(--color-accent)' }}
          >
            {jobs.length} active
          </span>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => refetch()}
            className="p-2 rounded border hover:bg-white/5 transition-colors"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
          >
            <RefreshCw size={13} className={isLoading ? 'animate-spin' : ''} />
          </button>
          <button
            onClick={() => { setAddingPreset(null); setShowModal(true) }}
            className="flex items-center gap-2 px-3 py-1.5 rounded text-sm font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            <Plus size={14} />
            Add Custom
          </button>
        </div>
      </div>

      {/* Preset strategies */}
      <div
        className="rounded-lg border p-4 mb-6"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <h2 className="text-sm font-semibold mb-3">Quick Add Presets</h2>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          {PRESET_SKILLS.map((preset) => {
            const active = runningPresets.has(preset.command)
            return (
              <button
                key={preset.id}
                onClick={() => { setAddingPreset(preset); setShowModal(false) }}
                disabled={active}
                className={clsx(
                  'text-left p-3 rounded border transition-all',
                  active ? 'opacity-50 cursor-not-allowed' : 'hover:border-accent/30 card-hover'
                )}
                style={{ borderColor: 'var(--color-border)' }}
              >
                <div className="text-lg mb-1">{preset.icon}</div>
                <div className="text-xs font-medium mb-0.5">{preset.name}</div>
                <div className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                  {active ? 'Active' : preset.schedule}
                </div>
              </button>
            )
          })}
        </div>
      </div>

      {/* Active jobs grid */}
      {isLoading ? (
        <div className="text-sm text-center py-8" style={{ color: 'var(--color-text-muted)' }}>
          Loading strategies...
        </div>
      ) : jobs.length === 0 ? (
        <div className="text-center py-12">
          <Zap size={40} className="mx-auto mb-3" style={{ color: 'var(--color-text-muted)' }} />
          <p className="text-sm mb-2" style={{ color: 'var(--color-text-muted)' }}>
            No strategies configured
          </p>
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            Add a preset or create a custom strategy above
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {jobs.map((job) => (
            <JobCard
              key={job.id}
              job={job}
              onDelete={(id) => deleteMutation.mutate(id)}
            />
          ))}
        </div>
      )}

      {(showModal || addingPreset) && (
        <AddModal
          preset={addingPreset ?? undefined}
          onClose={() => { setShowModal(false); setAddingPreset(null) }}
          onAdded={() => qc.invalidateQueries({ queryKey: ['skills'] })}
        />
      )}
    </div>
  )
}
