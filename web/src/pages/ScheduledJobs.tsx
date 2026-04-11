import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete, apiPut } from '../hooks/useApi'
import {
  Clock, Plus, Trash2, RefreshCw, X, Play, Pause, Sparkles, Loader2, Eye
} from 'lucide-react'
import clsx from 'clsx'

interface CronJob {
  id: string
  name?: string
  command: string
  prompt?: string
  next_run?: string
  last_run?: string
  last_status?: string
  last_output?: string
  enabled: boolean
}

interface CronResponse {
  jobs?: CronJob[]
}

// Friendly schedule options
const SCHEDULE_OPTIONS = [
  { label: 'Every minute', value: '*/1 * * * *' },
  { label: 'Every 5 minutes', value: '*/5 * * * *' },
  { label: 'Every 15 minutes', value: '*/15 * * * *' },
  { label: 'Every 30 minutes', value: '*/30 * * * *' },
  { label: 'Every hour', value: '0 * * * *' },
  { label: 'Every 2 hours', value: '0 */2 * * *' },
  { label: 'Every 6 hours', value: '0 */6 * * *' },
  { label: 'Every 12 hours', value: '0 */12 * * *' },
  { label: 'Daily at midnight', value: '0 0 * * *' },
  { label: 'Daily at 9am', value: '0 9 * * *' },
  { label: 'Custom (cron)', value: 'custom' },
]

// Alert job presets with real agent prompts
const PRESET_JOBS = [
  {
    id: 'wallet_scanner',
    name: 'Wallet Balance Alert',
    description: 'Alert if wallet balance changes by +/-3%',
    schedule: '*/30 * * * *',
    icon: '💰',
    prompt: `Check my wallet balances and compare them with the last check.
If any wallet balance has changed by more than 3% (up or down), send me a Telegram alert with:
- Wallet address
- Previous balance
- Current balance
- Percentage change
Store the current balances for the next comparison.`,
  },
  {
    id: 'polymarket_monitor',
    name: 'Polymarket Monitor',
    description: 'Monitor prediction markets for price movements',
    schedule: '*/10 * * * *',
    icon: '🎯',
    prompt: `Check the top Polymarket prediction markets.
Look for markets where:
- YES price moved more than 5% in the last hour
- Volume is above $10,000
- Market ends within 7 days

If you find any matching markets, send a Telegram alert with:
- Market question
- Current YES/NO prices
- Recent price movement
- Volume`,
  },
  {
    id: 'btc_price_alert',
    name: 'BTC Price Alert',
    description: 'Alert if BTC moves +/-3% in 15 minutes',
    schedule: '*/5 * * * *',
    icon: '📈',
    prompt: `Check the current Bitcoin (BTC/USDT) price from Binance.
Compare it with the price from 15 minutes ago (stored in memory).

If the price has moved more than 3% up or down:
- Send a Telegram alert with:
  - Current price
  - Price 15 minutes ago
  - Percentage change
  - Direction (UP or DOWN)

Always store the current price with timestamp for the next check.`,
  },
  {
    id: 'eth_price_target',
    name: 'ETH Price Target',
    description: 'Alert when ETH crosses $2,500',
    schedule: '*/1 * * * *',
    icon: '⚡',
    prompt: `Check the current Ethereum (ETH/USDC) price from Binance.

If the price is above $2,500 and I haven't been alerted yet today:
- Send a Telegram alert: "ETH has crossed $2,500! Current price: $X.XX"
- Mark that the alert was sent today

If the price drops below $2,400, reset the alert flag so I get notified again next time it crosses $2,500.`,
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

function getScheduleLabel(cron: string): string {
  const opt = SCHEDULE_OPTIONS.find(o => o.value === cron)
  return opt?.label ?? cron
}

interface ViewOutputModalProps {
  job: CronJob
  onClose: () => void
}

function ViewOutputModal({ job, onClose }: ViewOutputModalProps) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div
        className="rounded-lg border w-full max-w-2xl max-h-[80vh] flex flex-col"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center justify-between p-4 border-b" style={{ borderColor: 'var(--color-border)' }}>
          <h2 className="font-semibold">{job.name ?? job.id} - Last Output</h2>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>
        <div className="flex-1 overflow-auto p-4">
          <div className="mb-4">
            <h3 className="text-xs font-semibold mb-2" style={{ color: 'var(--color-text-muted)' }}>Prompt</h3>
            <pre
              className="text-xs whitespace-pre-wrap p-3 rounded"
              style={{ backgroundColor: 'var(--color-base)' }}
            >
              {job.prompt || job.command}
            </pre>
          </div>
          <div>
            <h3 className="text-xs font-semibold mb-2" style={{ color: 'var(--color-text-muted)' }}>Output</h3>
            <pre
              className="text-xs whitespace-pre-wrap p-3 rounded"
              style={{ backgroundColor: 'var(--color-base)' }}
            >
              {job.last_output || 'No output yet'}
            </pre>
          </div>
        </div>
        <div className="p-4 border-t flex justify-end" style={{ borderColor: 'var(--color-border)' }}>
          <button
            onClick={onClose}
            className="px-4 py-2 rounded text-sm font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            Close
          </button>
        </div>
      </div>
    </div>
  )
}

interface AddModalProps {
  onClose: () => void
  onAdded: () => void
  preset?: typeof PRESET_JOBS[0]
}

function AddModal({ onClose, onAdded, preset }: AddModalProps) {
  const [name, setName] = useState(preset?.name ?? '')
  const [scheduleType, setScheduleType] = useState(
    SCHEDULE_OPTIONS.find(o => o.value === preset?.schedule) ? preset?.schedule ?? '*/5 * * * *' : 'custom'
  )
  const [customCron, setCustomCron] = useState(preset?.schedule ?? '*/5 * * * *')
  const [prompt, setPrompt] = useState(preset?.prompt ?? '')
  const [aiDescription, setAiDescription] = useState('')
  const [isGenerating, setIsGenerating] = useState(false)
  const [error, setError] = useState('')

  const schedule = scheduleType === 'custom' ? customCron : scheduleType

  const mutation = useMutation({
    mutationFn: () =>
      apiPost('/api/cron/agent', { name, schedule, prompt }),
    onSuccess: () => { onAdded(); onClose() },
    onError: (e: Error) => setError(e.message),
  })

  const generateWithAI = async () => {
    if (!aiDescription.trim()) return
    setIsGenerating(true)
    setError('')
    try {
      const res = await apiPost<{ prompt: string }>('/api/chat', {
        message: `Generate an agent prompt for a scheduled job that: ${aiDescription}

The prompt should:
1. Be clear and specific about what to check/monitor
2. Specify what conditions trigger an alert
3. Tell the agent to send a Telegram alert when conditions are met
4. Include details about what information to include in the alert

Only return the prompt text, nothing else.`,
      })
      setPrompt(res.prompt || (res as any).response || '')
    } catch (e: any) {
      setError(e.message || 'Failed to generate prompt')
    } finally {
      setIsGenerating(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div
        className="rounded-lg border w-full max-w-2xl max-h-[90vh] overflow-y-auto"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center justify-between p-4 border-b sticky top-0" style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-surface)' }}>
          <h2 className="font-semibold">{preset ? `Add: ${preset.name}` : 'Add Scheduled Job'}</h2>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>

        <div className="p-4 space-y-4">
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Name</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm"
              placeholder="Job name"
            />
          </div>

          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Schedule</label>
            <select
              value={scheduleType}
              onChange={(e) => setScheduleType(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm"
            >
              {SCHEDULE_OPTIONS.map((opt) => (
                <option key={opt.value} value={opt.value}>{opt.label}</option>
              ))}
            </select>
            {scheduleType === 'custom' && (
              <input
                type="text"
                value={customCron}
                onChange={(e) => setCustomCron(e.target.value)}
                className="w-full rounded px-3 py-2 text-sm font-mono mt-2"
                placeholder="*/5 * * * *"
              />
            )}
          </div>

          {/* AI Generator */}
          <div
            className="rounded-lg border p-3"
            style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-base)' }}
          >
            <div className="flex items-center gap-2 mb-2">
              <Sparkles size={14} style={{ color: 'var(--color-accent)' }} />
              <span className="text-xs font-semibold">Generate with AI</span>
            </div>
            <div className="flex gap-2">
              <input
                type="text"
                value={aiDescription}
                onChange={(e) => setAiDescription(e.target.value)}
                className="flex-1 rounded px-3 py-2 text-sm"
                placeholder="Describe what you want to monitor... (e.g., 'Alert me when SOL price drops below $100')"
              />
              <button
                onClick={generateWithAI}
                disabled={isGenerating || !aiDescription.trim()}
                className="px-3 py-2 rounded text-sm font-medium disabled:opacity-50 flex items-center gap-2"
                style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
              >
                {isGenerating ? <Loader2 size={14} className="animate-spin" /> : <Sparkles size={14} />}
                Generate
              </button>
            </div>
          </div>

          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
              Agent Prompt
            </label>
            <textarea
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm font-mono h-48 resize-y"
              placeholder="Instructions for the AI agent...

Example:
Check the current BTC price from Binance.
If it's above $50,000, send me a Telegram alert."
            />
          </div>

          {error && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{error}</p>}
        </div>

        <div className="p-4 border-t flex gap-2" style={{ borderColor: 'var(--color-border)' }}>
          <button
            onClick={() => mutation.mutate()}
            disabled={!prompt || !schedule || mutation.isPending}
            className="flex-1 py-2 rounded text-sm font-medium disabled:opacity-50"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            {mutation.isPending ? 'Adding...' : 'Add Job'}
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
  onToggle: (id: string, enabled: boolean) => void
  onView: (job: CronJob) => void
}

function JobCard({ job, onDelete, onToggle, onView }: JobCardProps) {
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
            className="text-xs truncate"
            style={{ color: 'var(--color-text-muted)' }}
            title={job.prompt || job.command}
          >
            {(job.prompt || job.command).slice(0, 60)}...
          </p>
        </div>
        <div className="flex items-center gap-1 ml-2">
          <button
            onClick={() => onView(job)}
            className="p-1 rounded hover:bg-white/5 transition-colors"
            style={{ color: 'var(--color-text-muted)' }}
            title="View details"
          >
            <Eye size={13} />
          </button>
          <button
            onClick={() => onToggle(job.id, !job.enabled)}
            className="p-1 rounded hover:bg-white/5 transition-colors"
            style={{ color: job.enabled ? 'var(--color-accent)' : 'var(--color-text-muted)' }}
            title={job.enabled ? 'Pause' : 'Resume'}
          >
            {job.enabled ? <Pause size={13} /> : <Play size={13} />}
          </button>
          <button
            onClick={() => onDelete(job.id)}
            className="p-1 rounded hover:bg-white/5 transition-colors"
            style={{ color: 'var(--color-text-muted)' }}
            title="Delete"
          >
            <Trash2 size={13} />
          </button>
        </div>
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
          className="flex-1 h-1 rounded-full"
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

export default function ScheduledJobs() {
  const [showModal, setShowModal] = useState(false)
  const [addingPreset, setAddingPreset] = useState<typeof PRESET_JOBS[0] | null>(null)
  const [viewingJob, setViewingJob] = useState<CronJob | null>(null)
  const qc = useQueryClient()

  const { data, isLoading, refetch } = useQuery<CronResponse>({
    queryKey: ['scheduled-jobs'],
    queryFn: (): Promise<CronResponse> =>
      apiFetch<CronResponse>('/api/cron').catch(() => ({ jobs: [] })),
    refetchInterval: 15_000,
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => apiDelete(`/api/cron/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['scheduled-jobs'] }),
  })

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      apiPut(`/api/cron/${id}`, { enabled }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['scheduled-jobs'] }),
  })

  const jobs = data?.jobs ?? []
  const activeJobs = jobs.filter(j => j.enabled).length

  return (
    <div className="p-6 max-w-5xl mx-auto">
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-2">
          <Clock size={18} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-lg font-bold">Scheduled Jobs</h1>
          <span
            className="text-xs px-2 py-0.5 rounded"
            style={{ backgroundColor: 'var(--color-accent-dim)', color: 'var(--color-accent)' }}
          >
            {activeJobs} active
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
            Add Job
          </button>
        </div>
      </div>

      {/* Alert Presets */}
      <div
        className="rounded-lg border p-4 mb-6"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <h2 className="text-sm font-semibold mb-3">Alert Templates</h2>
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-3">
          {PRESET_JOBS.map((preset) => (
            <button
              key={preset.id}
              onClick={() => { setAddingPreset(preset); setShowModal(true) }}
              className="text-left p-3 rounded border transition-all hover:border-accent/30 card-hover"
              style={{ borderColor: 'var(--color-border)' }}
            >
              <div className="text-lg mb-1">{preset.icon}</div>
              <div className="text-xs font-medium mb-0.5">{preset.name}</div>
              <div className="text-xs line-clamp-2" style={{ color: 'var(--color-text-muted)' }}>
                {preset.description}
              </div>
            </button>
          ))}
        </div>
      </div>

      {/* Active jobs grid */}
      {isLoading ? (
        <div className="text-sm text-center py-8" style={{ color: 'var(--color-text-muted)' }}>
          Loading jobs...
        </div>
      ) : jobs.length === 0 ? (
        <div className="text-center py-12">
          <Clock size={40} className="mx-auto mb-3" style={{ color: 'var(--color-text-muted)' }} />
          <p className="text-sm mb-2" style={{ color: 'var(--color-text-muted)' }}>
            No scheduled jobs configured
          </p>
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            Add an alert template or create a custom job above
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {jobs.map((job) => (
            <JobCard
              key={job.id}
              job={job}
              onDelete={(id) => deleteMutation.mutate(id)}
              onToggle={(id, enabled) => toggleMutation.mutate({ id, enabled })}
              onView={(job) => setViewingJob(job)}
            />
          ))}
        </div>
      )}

      {(showModal || addingPreset) && (
        <AddModal
          preset={addingPreset ?? undefined}
          onClose={() => { setShowModal(false); setAddingPreset(null) }}
          onAdded={() => qc.invalidateQueries({ queryKey: ['scheduled-jobs'] })}
        />
      )}

      {viewingJob && (
        <ViewOutputModal
          job={viewingJob}
          onClose={() => setViewingJob(null)}
        />
      )}
    </div>
  )
}
