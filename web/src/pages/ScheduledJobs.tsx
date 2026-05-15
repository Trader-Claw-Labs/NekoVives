import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete, apiPut } from '../hooks/useApi'
import {
  Clock, Plus, Trash2, RefreshCw, X, Play, Pause, Sparkles, Loader2, Eye, Code2,
} from 'lucide-react'
import clsx from 'clsx'

interface CronJob {
  id: string
  name?: string
  command: string
  prompt?: string
  schedule?: string
  next_run?: string
  last_run?: string
  last_status?: string
  last_output?: string
  enabled: boolean
}

interface CronResponse {
  jobs?: CronJob[]
}

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
  { label: 'Custom cron expression', value: 'custom' },
]

// ── Rhai monitoring script templates ─────────────────────────────────

const SCRIPT_WALLET_SCANNER = `// Wallet Balance Scanner
// Alerts via Telegram if any wallet balance changes ±3%

fn on_tick(ctx) {
    let wallets = ctx.wallets();

    for wallet in wallets {
        let address = wallet.address;
        let current = ctx.wallet_balance(address);
        let key = "bal_" + address;
        let prev = ctx.get(key, 0.0);

        if prev > 0.0 {
            let change_pct = (current - prev) / prev * 100.0;

            if change_pct > 3.0 || change_pct < -3.0 {
                let dir = if change_pct > 0.0 { "UP" } else { "DOWN" };
                ctx.telegram("⚠️ Wallet Balance Alert!\\n" +
                    "Address: " + address + "\\n" +
                    "Change: " + change_pct + "% " + dir + "\\n" +
                    "Previous: " + prev + "\\n" +
                    "Current: " + current);
            }
        }

        ctx.set(key, current);
    }
    ctx.log("Wallet scan complete");
}
`

const SCRIPT_POLYMARKET_MONITOR = `// Polymarket Market Monitor
// Alerts when YES price moves more than 5% on watched markets

let WATCH_SLUGS = [
    "will-btc-hit-100k-in-2025",
    "will-eth-reach-5000-in-2025"
];

fn on_tick(ctx) {
    for slug in WATCH_SLUGS {
        let market = ctx.polymarket_market(slug);
        if market == () { continue; }

        let yes_price = market.yes_price;
        let key = "pm_" + slug;
        let prev = ctx.get(key, -1.0);

        if prev >= 0.0 {
            let change = (yes_price - prev) * 100.0;

            if change > 5.0 || change < -5.0 {
                let dir = if change > 0.0 { "▲" } else { "▼" };
                ctx.telegram("🎯 Polymarket Alert!\\n" +
                    "Market: " + market.question + "\\n" +
                    "YES: " + yes_price + "% " + dir + " " + change + "%\\n" +
                    "Volume: $" + market.volume);
            }
        }

        ctx.set(key, yes_price);
    }
}
`

const SCRIPT_BTC_PRICE = `// Bitcoin Price Monitor (15-min window)
// Alerts if BTC/USDT moves ±3% in the last 15 minutes

fn on_tick(ctx) {
    let btc_price = ctx.price("BTCUSDT");
    let now = ctx.timestamp();
    let last_price = ctx.get("btc_price", 0.0);
    let last_time  = ctx.get("btc_time", 0);

    if last_price > 0.0 {
        let elapsed_mins = (now - last_time) / 60;

        if elapsed_mins >= 15 {
            let change_pct = (btc_price - last_price) / last_price * 100.0;

            if change_pct > 3.0 || change_pct < -3.0 {
                let dir = if change_pct > 0.0 { "🚀 UP" } else { "🔴 DOWN" };
                ctx.telegram("📈 BTC Price Alert!\\n" +
                    "Current:   $" + btc_price + "\\n" +
                    "15min ago: $" + last_price + "\\n" +
                    "Change:    " + change_pct + "% " + dir);
            }

            ctx.set("btc_price", btc_price);
            ctx.set("btc_time", now);
        }
    } else {
        ctx.set("btc_price", btc_price);
        ctx.set("btc_time", now);
        ctx.log("BTC baseline set: $" + btc_price);
    }
}
`

const SCRIPT_ETH_PRICE = `// Ethereum Price Target Alert
// Alerts once when ETH/USDC crosses $2,500; resets below $2,400

let TARGET = 2500.0;
let RESET  = 2400.0;

fn on_tick(ctx) {
    let eth_price   = ctx.price("ETHUSDC");
    let alerted     = ctx.get("eth_alerted", false);

    if eth_price >= TARGET && !alerted {
        ctx.telegram("⚡ ETH Price Alert!\\n" +
            "ETH crossed $" + TARGET + "!\\n" +
            "Current: $" + eth_price);
        ctx.set("eth_alerted", true);
        ctx.log("Alert sent: ETH = $" + eth_price);
    }

    if eth_price < RESET && alerted {
        ctx.set("eth_alerted", false);
        ctx.log("Alert flag reset (ETH below $" + RESET + ")");
    }
}
`

const PRESET_JOBS = [
  {
    id: 'wallet_scanner',
    name: 'Wallet Balance Scanner',
    description: 'Alert if any wallet changes ±3%',
    schedule: '*/30 * * * *',
    icon: '💰',
    script: SCRIPT_WALLET_SCANNER,
  },
  {
    id: 'polymarket_monitor',
    name: 'Polymarket Monitor',
    description: 'Alert when YES price moves >5%',
    schedule: '*/10 * * * *',
    icon: '🎯',
    script: SCRIPT_POLYMARKET_MONITOR,
  },
  {
    id: 'btc_price_alert',
    name: 'BTC Price Alert',
    description: 'Alert if BTC/USDT moves ±3% in 15 min',
    schedule: '*/5 * * * *',
    icon: '₿',
    script: SCRIPT_BTC_PRICE,
  },
  {
    id: 'eth_price_target',
    name: 'ETH Price Target',
    description: 'Alert when ETH/USDC crosses $2,500',
    schedule: '*/1 * * * *',
    icon: '⚡',
    script: SCRIPT_ETH_PRICE,
  },
]

// ── Helpers ───────────────────────────────────────────────────────────

function formatDate(iso?: string): string {
  if (!iso) return '—'
  try { return new Date(iso).toLocaleString() } catch { return iso }
}

function getScheduleLabel(expr?: string): string {
  if (!expr) return '—'
  const opt = SCHEDULE_OPTIONS.find(o => o.value === expr)
  return opt?.label ?? expr
}

// ── View Output Modal ─────────────────────────────────────────────────

function ViewOutputModal({ job, onClose }: { job: CronJob; onClose: () => void }) {
  const script = job.prompt || job.command
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div
        className="rounded-lg border w-full max-w-2xl max-h-[85vh] flex flex-col"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center justify-between p-4 border-b" style={{ borderColor: 'var(--color-border)' }}>
          <div>
            <h2 className="font-semibold">{job.name ?? job.id}</h2>
            <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              {getScheduleLabel(job.schedule)} · last run {formatDate(job.last_run)}
            </p>
          </div>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}><X size={16} /></button>
        </div>

        <div className="flex-1 overflow-auto p-4 space-y-4">
          <div>
            <div className="flex items-center gap-1.5 mb-2">
              <Code2 size={12} style={{ color: 'var(--color-accent)' }} />
              <h3 className="text-xs font-semibold" style={{ color: 'var(--color-text-muted)' }}>Rhai Script</h3>
            </div>
            <pre
              className="text-xs font-mono whitespace-pre-wrap p-3 rounded leading-relaxed"
              style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text)' }}
            >
              {script}
            </pre>
          </div>

          {job.last_output && (
            <div>
              <h3 className="text-xs font-semibold mb-2" style={{ color: 'var(--color-text-muted)' }}>Last Output</h3>
              <pre
                className="text-xs whitespace-pre-wrap p-3 rounded"
                style={{ backgroundColor: 'var(--color-base)' }}
              >
                {job.last_output}
              </pre>
            </div>
          )}
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

// ── Add Job Modal ─────────────────────────────────────────────────────

interface AddModalProps {
  onClose: () => void
  onAdded: () => void
  preset?: typeof PRESET_JOBS[0]
}

function AddModal({ onClose, onAdded, preset }: AddModalProps) {
  const [name, setName] = useState(preset?.name ?? '')
  const [scheduleType, setScheduleType] = useState(
    SCHEDULE_OPTIONS.find(o => o.value === preset?.schedule)
      ? preset?.schedule ?? '*/5 * * * *'
      : 'custom'
  )
  const [customCron, setCustomCron] = useState(preset?.schedule ?? '*/5 * * * *')
  const [script, setScript] = useState(preset?.script ?? '')
  const [aiDescription, setAiDescription] = useState('')
  const [isGenerating, setIsGenerating] = useState(false)
  const [error, setError] = useState('')

  const schedule = scheduleType === 'custom' ? customCron : scheduleType

  const mutation = useMutation({
    mutationFn: () =>
      apiPost('/api/cron/agent', { name, schedule, prompt: script }),
    onSuccess: () => { onAdded(); onClose() },
    onError: (e: Error) => setError(e.message),
  })

  const generateWithAI = async () => {
    if (!aiDescription.trim()) return
    setIsGenerating(true)
    setError('')
    try {
      const res = await apiPost<{ prompt?: string; response?: string }>('/api/chat', {
        message: `Generate a Rhai monitoring script for a scheduled job that: ${aiDescription}

The script MUST:
1. Define a function: fn on_tick(ctx) { ... }
2. Use ctx.price("SYMBOL") to get crypto prices (e.g. ctx.price("BTCUSDT"))
3. Use ctx.telegram("message") to send Telegram alerts
4. Use ctx.get("key", default) and ctx.set("key", value) for persistent state
5. Use ctx.log("message") for logging output
6. Only alert when a condition is newly triggered (use ctx.get/set to track state)

Return ONLY valid Rhai code, no markdown fences, no explanation.`,
      })
      setScript((res.prompt || res.response || '').replace(/^```[a-z]*\n?/, '').replace(/```$/, '').trim())
    } catch (e: any) {
      setError(e.message || 'Failed to generate script')
    } finally {
      setIsGenerating(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div
        className="rounded-lg border w-full max-w-2xl max-h-[95vh] overflow-y-auto"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        {/* Header */}
        <div
          className="flex items-center justify-between p-4 border-b sticky top-0 z-10"
          style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-surface)' }}
        >
          <h2 className="font-semibold">{preset ? `Add: ${preset.name}` : 'Add Scheduled Job'}</h2>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}><X size={16} /></button>
        </div>

        <div className="p-4 space-y-4">
          {/* Name */}
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Job Name</label>
            <input
              type="text"
              value={name}
              onChange={e => setName(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm"
              placeholder="e.g. BTC Price Monitor"
            />
          </div>

          {/* Schedule */}
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Run Frequency</label>
            <select
              value={scheduleType}
              onChange={e => setScheduleType(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm"
            >
              {SCHEDULE_OPTIONS.map(opt => (
                <option key={opt.value} value={opt.value}>{opt.label}</option>
              ))}
            </select>
            {scheduleType === 'custom' && (
              <input
                type="text"
                value={customCron}
                onChange={e => setCustomCron(e.target.value)}
                className="w-full rounded px-3 py-2 text-sm font-mono mt-2"
                placeholder="*/5 * * * *"
              />
            )}
          </div>

          {/* AI Script Generator */}
          <div
            className="rounded-lg border p-3"
            style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-base)' }}
          >
            <div className="flex items-center gap-2 mb-2">
              <Sparkles size={13} style={{ color: 'var(--color-accent)' }} />
              <span className="text-xs font-semibold">Generate Rhai Script with AI</span>
            </div>
            <div className="flex gap-2">
              <input
                type="text"
                value={aiDescription}
                onChange={e => setAiDescription(e.target.value)}
                onKeyDown={e => e.key === 'Enter' && generateWithAI()}
                className="flex-1 rounded px-3 py-2 text-sm"
                placeholder="Describe what to monitor... (e.g. 'alert me if SOL drops below $100')"
              />
              <button
                onClick={generateWithAI}
                disabled={isGenerating || !aiDescription.trim()}
                className="px-3 py-2 rounded text-sm font-medium disabled:opacity-50 flex items-center gap-2 whitespace-nowrap"
                style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
              >
                {isGenerating ? <Loader2 size={13} className="animate-spin" /> : <Sparkles size={13} />}
                Generate
              </button>
            </div>
          </div>

          {/* Rhai Script Editor */}
          <div>
            <div className="flex items-center gap-1.5 mb-1">
              <Code2 size={12} style={{ color: 'var(--color-accent)' }} />
              <label className="text-xs font-semibold" style={{ color: 'var(--color-text-muted)' }}>
                Rhai Script
              </label>
            </div>
            <textarea
              value={script}
              onChange={e => setScript(e.target.value)}
              className="w-full rounded px-3 py-2.5 text-xs font-mono h-64 resize-y leading-relaxed"
              style={{
                backgroundColor: 'var(--color-base)',
                color: 'var(--color-text)',
                border: '1px solid var(--color-border)',
              }}
              placeholder={`// Define on_tick(ctx) — called on each schedule tick
fn on_tick(ctx) {
    let price = ctx.price("BTCUSDT");
    let prev  = ctx.get("last_price", 0.0);

    if prev > 0.0 {
        let change = (price - prev) / prev * 100.0;
        if change > 3.0 || change < -3.0 {
            ctx.telegram("BTC moved " + change + "%! Now: $" + price);
        }
    }

    ctx.set("last_price", price);
}`}
            />
            <p className="text-xs mt-1.5" style={{ color: 'var(--color-text-muted)' }}>
              Available: <code>ctx.price(sym)</code> · <code>ctx.wallet_balance(addr)</code> · <code>ctx.polymarket_market(slug)</code> · <code>ctx.telegram(msg)</code> · <code>ctx.get/set(key, val)</code> · <code>ctx.log(msg)</code>
            </p>
          </div>

          {error && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{error}</p>}
        </div>

        <div className="p-4 border-t flex gap-2" style={{ borderColor: 'var(--color-border)' }}>
          <button
            onClick={() => mutation.mutate()}
            disabled={!script.trim() || !schedule || mutation.isPending}
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

// ── Job Card ──────────────────────────────────────────────────────────

interface JobCardProps {
  job: CronJob
  onDelete: (id: string) => void
  onToggle: (id: string, enabled: boolean) => void
  onView: (job: CronJob) => void
}

function JobCard({ job, onDelete, onToggle, onView }: JobCardProps) {
  const isRunning = job.last_status === 'running'
  const hasFailed = job.last_status === 'failed' || job.last_status === 'error'
  const isActive  = job.enabled && !isRunning

  const statusColor = isRunning
    ? 'var(--color-warning, #f59e0b)'
    : hasFailed
    ? 'var(--color-danger)'
    : job.enabled
    ? 'var(--color-accent)'
    : 'var(--color-text-muted)'

  const statusDot = isRunning ? 'warning' : job.enabled ? 'online' : 'offline'

  return (
    <div
      className={clsx(
        'rounded-lg border p-4 card-hover transition-all flex flex-col gap-3',
        !job.enabled && 'opacity-60'
      )}
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      {/* Header row */}
      <div className="flex items-start justify-between">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <span className={clsx('status-dot', statusDot)} />
            <h3 className="text-sm font-semibold truncate">{job.name ?? job.id}</h3>
          </div>
          <p
            className="text-xs truncate font-mono"
            style={{ color: 'var(--color-text-muted)' }}
            title={job.prompt || job.command}
          >
            {(job.prompt || job.command).slice(0, 55)}...
          </p>
        </div>
        <button
          onClick={() => onView(job)}
          className="ml-2 p-1.5 rounded hover:bg-white/5 transition-colors flex-shrink-0"
          style={{ color: 'var(--color-text-muted)' }}
          title="View script & output"
        >
          <Eye size={13} />
        </button>
      </div>

      {/* Schedule + timing */}
      <div className="space-y-1 text-xs">
        {job.schedule && (
          <div className="flex justify-between">
            <span style={{ color: 'var(--color-text-muted)' }}>Schedule</span>
            <span className="font-mono" style={{ color: 'var(--color-accent)' }}>
              {getScheduleLabel(job.schedule)}
            </span>
          </div>
        )}
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
            <span style={{ color: statusColor }}>{job.last_status}</span>
          </div>
        )}
      </div>

      {/* Actions */}
      <div className="flex items-center gap-2 pt-1 border-t" style={{ borderColor: 'var(--color-border)' }}>
        <button
          onClick={() => onToggle(job.id, !job.enabled)}
          className={clsx(
            'flex-1 flex items-center justify-center gap-1.5 py-1.5 rounded text-xs font-medium transition-colors',
            job.enabled ? 'hover:bg-white/5' : ''
          )}
          style={
            job.enabled
              ? { border: '1px solid var(--color-border)', color: 'var(--color-text-muted)' }
              : { backgroundColor: 'var(--color-accent)', color: '#000' }
          }
          title={job.enabled ? 'Pause job' : 'Resume job'}
        >
          {job.enabled ? <Pause size={11} /> : <Play size={11} />}
          {job.enabled ? 'Pause' : 'Resume'}
        </button>

        <button
          onClick={() => {
            if (confirm(`Delete job "${job.name ?? job.id}"?`)) onDelete(job.id)
          }}
          className="p-1.5 rounded hover:bg-white/5 transition-colors"
          style={{ color: 'var(--color-text-muted)' }}
          title="Delete job"
        >
          <Trash2 size={13} />
        </button>
      </div>
    </div>
  )
}

// ── Main Page ─────────────────────────────────────────────────────────

export default function ScheduledJobs() {
  const [showModal, setShowModal]       = useState(false)
  const [addingPreset, setAddingPreset] = useState<typeof PRESET_JOBS[0] | null>(null)
  const [viewingJob, setViewingJob]     = useState<CronJob | null>(null)
  const qc = useQueryClient()

  const { data, isLoading, refetch } = useQuery<CronResponse>({
    queryKey: ['scheduled-jobs'],
    queryFn: (): Promise<CronResponse> =>
      apiFetch<CronResponse>('/api/cron').catch(() => ({ jobs: [] })),
    refetchInterval: 10_000,
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
  const activeJobs  = jobs.filter(j => j.enabled).length
  const runningJobs = jobs.filter(j => j.last_status === 'running').length

  function openPreset(preset: typeof PRESET_JOBS[0]) {
    setAddingPreset(preset)
    setShowModal(true)
  }

  function closeModal() {
    setShowModal(false)
    setAddingPreset(null)
  }

  return (
    <div className="p-6 max-w-5xl mx-auto">

      {/* Page header */}
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
          {runningJobs > 0 && (
            <span
              className="text-xs px-2 py-0.5 rounded animate-pulse"
              style={{ backgroundColor: 'rgba(245,158,11,0.15)', color: '#f59e0b' }}
            >
              {runningJobs} running
            </span>
          )}
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
            New Job
          </button>
        </div>
      </div>

      {/* Rhai Alert Templates */}
      <div
        className="rounded-lg border p-4 mb-6"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center gap-2 mb-3">
          <Code2 size={14} style={{ color: 'var(--color-accent)' }} />
          <h2 className="text-sm font-semibold">Rhai Alert Templates</h2>
          <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            — click to add a pre-built monitoring script
          </span>
        </div>
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-3">
          {PRESET_JOBS.map(preset => (
            <button
              key={preset.id}
              onClick={() => openPreset(preset)}
              className="text-left p-3 rounded border transition-all hover:border-accent/40 card-hover"
              style={{ borderColor: 'var(--color-border)' }}
            >
              <div className="text-xl mb-1.5">{preset.icon}</div>
              <div className="text-xs font-semibold mb-0.5">{preset.name}</div>
              <div className="text-xs mb-1.5 line-clamp-2" style={{ color: 'var(--color-text-muted)' }}>
                {preset.description}
              </div>
              <div
                className="text-xs font-mono px-1.5 py-0.5 rounded inline-block"
                style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-accent)' }}
              >
                {getScheduleLabel(preset.schedule)}
              </div>
            </button>
          ))}
        </div>
      </div>

      {/* Jobs grid */}
      {isLoading ? (
        <div className="text-sm text-center py-8" style={{ color: 'var(--color-text-muted)' }}>
          Loading jobs...
        </div>
      ) : jobs.length === 0 ? (
        <div className="text-center py-16">
          <Clock size={44} className="mx-auto mb-4 opacity-30" />
          <p className="text-sm mb-1" style={{ color: 'var(--color-text-muted)' }}>
            No scheduled jobs yet
          </p>
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            Choose a template above or click <strong>New Job</strong> to write a custom Rhai script
          </p>
        </div>
      ) : (
        <>
          <h2 className="text-sm font-semibold mb-3" style={{ color: 'var(--color-text-muted)' }}>
            Active Jobs ({jobs.length})
          </h2>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {jobs.map(job => (
              <JobCard
                key={job.id}
                job={job}
                onDelete={id => deleteMutation.mutate(id)}
                onToggle={(id, enabled) => toggleMutation.mutate({ id, enabled })}
                onView={j => setViewingJob(j)}
              />
            ))}
          </div>
        </>
      )}

      {showModal && (
        <AddModal
          preset={addingPreset ?? undefined}
          onClose={closeModal}
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
