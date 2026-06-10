import { useState, useEffect } from 'react'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import { Coins, Shield, RefreshCw, AlertCircle, AlertTriangle, Save } from 'lucide-react'
import clsx from 'clsx'
import ArbScanner from '../components/ArbScanner'
import RewardsPositions from '../components/RewardsPositions'

// ── Types ─────────────────────────────────────────────────────────────────
interface RewardMarket {
  condition_id: string
  question: string
  market_slug: string
  end_date_iso: string | null
  daily_rate: number
  max_spread: number
  min_size: number
  neg_risk: boolean
  tags: string[]
  is_toxic: boolean
  days_to_end: number | null
  score: number
  safety: string
}

interface RewardsResponse {
  markets: RewardMarket[]
  total_incentivized: number
  toxic_excluded: number
  eligible: number
  fetched_at: string
}

// Local config for the maker-quoting mechanism (consumed by the future quoting bot).
interface RewardsConfig {
  spread_offset_c: number   // how far inside the eligible band to rest each side (¢)
  order_size_usd: number    // size per side (must be >= market min_size)
  max_markets: number       // how many top markets to quote simultaneously
  reprice_secs: number      // how often to re-center quotes on the mid
  min_safety: 'high' | 'medium' | 'low'
}

const CONFIG_KEY = 'rewards_config'
const DEFAULT_CONFIG: RewardsConfig = {
  spread_offset_c: 1.0,
  order_size_usd: 200,
  max_markets: 3,
  reprice_secs: 60,
  min_safety: 'high',
}

function loadConfig(): RewardsConfig {
  try {
    const raw = localStorage.getItem(CONFIG_KEY)
    if (raw) return { ...DEFAULT_CONFIG, ...JSON.parse(raw) }
  } catch { /* ignore */ }
  return DEFAULT_CONFIG
}

const SAFETY_COLOR: Record<string, string> = {
  high: 'var(--color-accent)',
  medium: '#f59e0b',
  low: '#f87171',
  toxic: '#ef4444',
  expiring: 'var(--color-text-muted)',
}

const SAFETY_RANK: Record<string, number> = { high: 3, medium: 2, low: 1 }

export default function RewardsPage() {
  const [includeToxic, setIncludeToxic] = useState(false)
  const [maxPages, setMaxPages] = useState(3)
  const [limit, setLimit] = useState(60)
  const [cfg, setCfg] = useState<RewardsConfig>(loadConfig)
  const [saved, setSaved] = useState(false)

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery<RewardsResponse>({
    queryKey: ['rewards', includeToxic, maxPages, limit],
    queryFn: () =>
      apiFetch(`/api/rewards/markets?limit=${limit}&include_toxic=${includeToxic}&max_pages=${maxPages}`),
    staleTime: 5 * 60 * 1000,
  })

  useEffect(() => {
    if (!saved) return
    const t = setTimeout(() => setSaved(false), 1500)
    return () => clearTimeout(t)
  }, [saved])

  function saveConfig() {
    localStorage.setItem(CONFIG_KEY, JSON.stringify(cfg))
    setSaved(true)
  }

  // Markets filtered by the configured minimum safety, for the strategy view.
  const markets = (data?.markets ?? []).filter(
    m => m.is_toxic || (SAFETY_RANK[m.safety] ?? 0) >= (SAFETY_RANK[cfg.min_safety] ?? 0),
  )

  return (
    <div className="p-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-1">
        <div className="flex items-center gap-2">
          <Coins size={22} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-xl font-bold" style={{ color: 'var(--color-accent)' }}>
            Liquidity Rewards
          </h1>
        </div>
        <button
          onClick={() => refetch()}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs font-medium"
          style={{ background: 'var(--color-surface-2)', color: 'var(--color-text)' }}
        >
          <RefreshCw size={13} className={isFetching ? 'animate-spin' : ''} /> Refresh
        </button>
      </div>
      <p className="text-xs mb-4" style={{ color: 'var(--color-text-muted)' }}>
        Earn USDC for posting two-sided resting orders near the midpoint in incentivized markets.
        The edge is structural (not directional) — rank favors slow markets where adverse selection
        is low. Crypto / UP-DOWN markets are flagged <span style={{ color: SAFETY_COLOR.toxic }}>toxic</span> (fast
        fair value → quotes get picked off).
      </p>

      {/* Strategy mechanism config */}
      <div
        className="rounded-lg border p-4 mb-4"
        style={{ background: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center gap-2 mb-3">
          <Shield size={15} style={{ color: 'var(--color-accent)' }} />
          <span className="text-sm font-semibold" style={{ color: 'var(--color-text)' }}>
            Rewards Mechanism Config
          </span>
        </div>
        <div className="grid grid-cols-2 md:grid-cols-5 gap-3">
          <NumField label="Spread offset (¢)" hint="inside the band, per side"
            value={cfg.spread_offset_c} min={0.2} max={5} step={0.1}
            onChange={v => setCfg({ ...cfg, spread_offset_c: v })} />
          <NumField label="Order size ($)" hint="per side · ≥ min_size"
            value={cfg.order_size_usd} min={1} max={100000} step={50}
            onChange={v => setCfg({ ...cfg, order_size_usd: v })} />
          <NumField label="Max markets" hint="quoted at once"
            value={cfg.max_markets} min={1} max={20} step={1}
            onChange={v => setCfg({ ...cfg, max_markets: v })} />
          <NumField label="Reprice (s)" hint="re-center cadence"
            value={cfg.reprice_secs} min={5} max={600} step={5}
            onChange={v => setCfg({ ...cfg, reprice_secs: v })} />
          <div>
            <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Min safety</label>
            <select
              className="w-full rounded border px-2 py-1.5 text-xs"
              style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
              value={cfg.min_safety}
              onChange={e => setCfg({ ...cfg, min_safety: e.target.value as RewardsConfig['min_safety'] })}
            >
              <option value="high">high only</option>
              <option value="medium">medium+</option>
              <option value="low">low+</option>
            </select>
          </div>
        </div>
        <div className="flex items-center gap-3 mt-3">
          <button
            onClick={saveConfig}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs font-medium"
            style={{ background: 'var(--color-accent)', color: '#000' }}
          >
            <Save size={13} /> Save config
          </button>
          {saved && <span className="text-xs" style={{ color: 'var(--color-accent)' }}>Saved ✓</span>}
          <span className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>
            Saved locally — consumed by the maker-quoting runner (start with a small real pilot to confirm payout).
          </span>
        </div>
      </div>

      {/* Active maker quotes + balance */}
      <RewardsPositions />

      {/* Structural arb scanner — set-arb + monotonicity violations on slow events */}
      <ArbScanner />

      {/* Scanner controls + summary */}
      <div className="flex flex-wrap items-center gap-3 mb-3">
        <label className="flex items-center gap-1.5 text-xs" style={{ color: 'var(--color-text)' }}>
          <input type="checkbox" checked={includeToxic} onChange={e => setIncludeToxic(e.target.checked)} />
          Show toxic
        </label>
        <label className="flex items-center gap-1.5 text-xs" style={{ color: 'var(--color-text-muted)' }}>
          Pages
          <input type="number" min={1} max={20} value={maxPages}
            onChange={e => setMaxPages(Number(e.target.value) || 1)}
            className="w-14 rounded border px-2 py-1 text-xs"
            style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }} />
        </label>
        <label className="flex items-center gap-1.5 text-xs" style={{ color: 'var(--color-text-muted)' }}>
          Limit
          <input type="number" min={10} max={500} value={limit}
            onChange={e => setLimit(Number(e.target.value) || 60)}
            className="w-16 rounded border px-2 py-1 text-xs"
            style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }} />
        </label>
        {data && (
          <span className="text-xs ml-auto" style={{ color: 'var(--color-text-muted)' }}>
            {data.eligible} eligible · {data.toxic_excluded} toxic excluded · {data.total_incentivized} scanned
          </span>
        )}
      </div>

      {/* States */}
      {isLoading && (
        <div className="text-center py-12 text-sm" style={{ color: 'var(--color-text-muted)' }}>
          <RefreshCw size={20} className="animate-spin mx-auto mb-2" /> Scanning incentivized markets…
        </div>
      )}
      {isError && (
        <div className="flex items-center gap-2 p-4 rounded text-sm"
          style={{ background: 'rgba(239,68,68,0.1)', color: '#f87171' }}>
          <AlertCircle size={16} /> {(error as Error)?.message ?? 'Failed to load rewards markets'}
        </div>
      )}

      {/* Table */}
      {data && !isLoading && (
        <div className="rounded-lg border overflow-hidden"
          style={{ borderColor: 'var(--color-border)', background: 'var(--color-surface)' }}>
          <div className="grid text-[11px] font-semibold px-3 py-2 border-b"
            style={{
              gridTemplateColumns: '1fr 70px 80px 70px 70px 70px',
              borderColor: 'var(--color-border)', color: 'var(--color-text-muted)',
            }}>
            <span>Market</span>
            <span className="text-right">Reward/d</span>
            <span className="text-right">Max spread</span>
            <span className="text-right">Min size</span>
            <span className="text-right">Days left</span>
            <span className="text-right">Safety</span>
          </div>
          {markets.map((m, i) => (
            <div key={m.condition_id + i}
              className="grid text-xs items-center px-3 py-2.5 border-b hover:bg-white/5"
              style={{
                gridTemplateColumns: '1fr 70px 80px 70px 70px 70px',
                borderColor: 'var(--color-border)', color: 'var(--color-text)',
              }}>
              <div className="flex items-center gap-1.5 pr-2 min-w-0">
                {m.is_toxic && <AlertTriangle size={12} style={{ color: SAFETY_COLOR.toxic, flexShrink: 0 }} />}
                <span className="truncate" title={m.question}>{m.question}</span>
                {m.tags?.[0] && (
                  <span className="text-[9px] px-1 rounded shrink-0"
                    style={{ background: 'var(--color-surface-2)', color: 'var(--color-text-muted)' }}>
                    {m.tags[0]}
                  </span>
                )}
              </div>
              <span className="text-right font-mono" style={{ color: 'var(--color-accent)' }}>
                {m.daily_rate >= 1 ? m.daily_rate.toFixed(0) : m.daily_rate.toFixed(3)}
              </span>
              <span className="text-right font-mono">{m.max_spread.toFixed(1)}¢</span>
              <span className="text-right font-mono">${m.min_size}</span>
              <span className="text-right font-mono" style={{ color: 'var(--color-text-muted)' }}>
                {m.days_to_end != null ? Math.round(m.days_to_end) : '—'}
              </span>
              <span className="text-right">
                <span className={clsx('px-1.5 py-0.5 rounded text-[10px] font-semibold')}
                  style={{ color: SAFETY_COLOR[m.safety] ?? 'var(--color-text)', background: 'var(--color-surface-2)' }}>
                  {m.safety}
                </span>
              </span>
            </div>
          ))}
          {markets.length === 0 && (
            <div className="text-center py-8 text-xs" style={{ color: 'var(--color-text-muted)' }}>
              No markets match the current safety filter.
            </div>
          )}
        </div>
      )}
    </div>
  )
}

// ── Small numeric field ────────────────────────────────────────────────────
function NumField({ label, hint, value, min, max, step, onChange }: {
  label: string; hint?: string; value: number; min: number; max: number; step: number; onChange: (v: number) => void
}) {
  return (
    <div>
      <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>{label}</label>
      <input type="number" value={value} min={min} max={max} step={step}
        onChange={e => onChange(Number(e.target.value))}
        className="w-full rounded border px-2 py-1.5 text-xs"
        style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }} />
      {hint && <p className="text-[9px] mt-0.5" style={{ color: 'var(--color-text-muted)' }}>{hint}</p>}
    </div>
  )
}
