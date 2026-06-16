import { useState, useEffect } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { Coins, Shield, RefreshCw, AlertCircle, AlertTriangle, Save, X } from 'lucide-react'
import clsx from 'clsx'
import ArbScanner from '../components/ArbScanner'
import RewardsPositions from '../components/RewardsPositions'

// ── Types ─────────────────────────────────────────────────────────────────
interface RewardMarket {
  yes_token_id?: string | null
  no_token_id?: string | null
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
  yes_price?: number
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
  // Quote modal state
  const [quoteMarket, setQuoteMarket] = useState<RewardMarket | null>(null)
  const [quoteSize, setQuoteSize] = useState('')
  const [quoteOffset, setQuoteOffset] = useState('1.0')
  const [quoteBusy, setQuoteBusy] = useState(false)
  const [quoteResult, setQuoteResult] = useState<string | null>(null)
  // Autonomous-pilot launcher state
  const [pilotWallet, setPilotWallet] = useState('')
  const [pilotCapital, setPilotCapital] = useState('2500')
  const [pilotLive, setPilotLive] = useState(false)
  const [pilotBusy, setPilotBusy] = useState(false)
  const [pilotResult, setPilotResult] = useState<string | null>(null)
  const qc = useQueryClient()

  // Wallet profiles for the pilot launcher (same source as the strategy create modal).
  const { data: walletData } = useQuery<{ wallets: { id: string; label: string; configured: boolean; wallet_address_masked?: string | null }[] }>({
    queryKey: ['polymarket-wallets'],
    queryFn: () => apiFetch('/api/polymarket/wallets'),
    staleTime: 60 * 1000,
  })
  const walletProfiles = walletData?.wallets ?? []

  async function launchPilot() {
    setPilotBusy(true); setPilotResult(null)
    try {
      const capital = Number(pilotCapital) || 0
      if (capital <= 0) { setPilotResult('✗ Assign capital greater than 0.'); setPilotBusy(false); return }
      const body: Record<string, unknown> = {
        name: `Rewards Pilot (${cfg.max_markets} markets, ${cfg.min_safety})`,
        kind: 'rewards_orchestrator',
        market_type: 'polymarket_binary',
        mode: pilotLive ? 'live' : 'paper',
        initial_balance: capital,
        polymarket_wallet_id: pilotWallet || undefined,
        force_live: true, // maker engine: not a directional bet, no edge-gate applies
        engine_params: {
          max_markets: cfg.max_markets,
          min_safety: cfg.min_safety,
          offset_cents: cfg.spread_offset_c,
          poll_secs: cfg.reprice_secs,
          size_usd: 0, // auto-split capital across pool × 2 legs
        },
      }
      const r = await apiPost<{ id?: string; error?: string }>('/api/live/strategies', body)
      if (r.error) { setPilotResult('✗ ' + r.error) }
      else {
        setPilotResult(`✓ ${pilotLive ? 'LIVE' : 'Dry-run'} pilot started${r.id ? ` (${r.id})` : ''} — monitor it in Live Strategies.`)
        qc.invalidateQueries({ queryKey: ['live-strategies'] })
      }
    } catch (e) {
      setPilotResult('✗ ' + (e as Error).message)
    } finally {
      setPilotBusy(false)
    }
  }

  async function submitQuote() {
    if (!quoteMarket?.yes_token_id || !quoteMarket?.no_token_id) return
    setQuoteBusy(true); setQuoteResult(null)
    try {
      const r = await apiPost<{ both_placed: boolean; yes: Record<string, unknown>; no: Record<string, unknown> }>(
        '/api/rewards/quote', {
          yes_token_id: quoteMarket.yes_token_id,
          no_token_id: quoteMarket.no_token_id,
          size_usd: Number(quoteSize) || quoteMarket.min_size,
          offset_c: Number(quoteOffset) || 1.0,
        })
      setQuoteResult(r.both_placed ? '✓ Both quotes placed and resting in the book.' : '⚠ Partial: ' + JSON.stringify(r))
      qc.invalidateQueries({ queryKey: ['poly-orders-rewards'] })
    } catch (e) {
      setQuoteResult('✗ ' + (e as Error).message)
    } finally {
      setQuoteBusy(false)
    }
  }

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

      {/* Autonomous pilot launcher — wallet + capital → start the orchestrator runner */}
      <div
        className="rounded-lg border p-4 mb-4"
        style={{ background: 'var(--color-surface)', borderColor: 'var(--color-accent)' }}
      >
        <div className="flex items-center gap-2 mb-1">
          <Coins size={15} style={{ color: 'var(--color-accent)' }} />
          <span className="text-sm font-semibold" style={{ color: 'var(--color-text)' }}>
            Launch Autonomous Pilot
          </span>
        </div>
        <p className="text-[11px] mb-3" style={{ color: 'var(--color-text-muted)' }}>
          Pick a wallet and assign capital. The engine auto-selects the top{' '}
          <span className="font-mono">{cfg.max_markets}</span> markets at{' '}
          <span className="font-mono">{cfg.min_safety}</span>+ safety (from the config above), quotes both
          sides on each, and closes + rotates out of any market that turns toxic. Manage/stop it in Live Strategies.
        </p>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3 items-end">
          <div>
            <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Wallet</label>
            <select
              className="w-full rounded border px-2 py-1.5 text-xs"
              style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
              value={pilotWallet}
              onChange={e => setPilotWallet(e.target.value)}
            >
              <option value="">Default wallet</option>
              {walletProfiles.filter(w => w.id !== 'default').map(w => (
                <option key={w.id} value={w.id} disabled={pilotLive && !w.configured}>
                  {w.label}{w.wallet_address_masked ? ` · ${w.wallet_address_masked}` : ''}{!w.configured ? ' (incomplete)' : ''}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Capital ($)</label>
            <input
              type="number" min={1} step={100} value={pilotCapital}
              onChange={e => setPilotCapital(e.target.value)}
              className="w-full rounded border px-2 py-1.5 text-xs"
              style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
            />
          </div>
          <div>
            <label className="block text-[10px] mb-1" style={{ color: 'var(--color-text-muted)' }}>Mode</label>
            <label className="flex items-center gap-1.5 text-xs h-[30px]" style={{ color: 'var(--color-text)' }}>
              <input type="checkbox" checked={pilotLive} onChange={e => setPilotLive(e.target.checked)} />
              <span style={{ color: pilotLive ? 'var(--color-warning)' : 'var(--color-text-muted)' }}>
                {pilotLive ? 'LIVE (real orders)' : 'Dry run'}
              </span>
            </label>
          </div>
          <button
            onClick={launchPilot}
            disabled={pilotBusy}
            className="flex items-center justify-center gap-1.5 px-3 py-2 rounded text-xs font-medium disabled:opacity-50"
            style={{ background: pilotLive ? 'var(--color-warning)' : 'var(--color-accent)', color: '#000' }}
          >
            {pilotBusy ? 'Starting…' : pilotLive ? 'Launch LIVE pilot' : 'Launch dry-run pilot'}
          </button>
        </div>
        {pilotLive && (
          <div className="flex items-center gap-1.5 mt-2 text-[11px]" style={{ color: 'var(--color-warning)' }}>
            <AlertTriangle size={12} /> Live mode places real CLOB orders signed with the selected wallet.
            Recommended: run dry-run first to confirm eligible%.
          </div>
        )}
        {pilotResult && (
          <div className="mt-2 text-xs" style={{ color: pilotResult.startsWith('✓') ? 'var(--color-accent)' : '#f87171' }}>
            {pilotResult}
          </div>
        )}
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
              gridTemplateColumns: '1fr 70px 80px 70px 70px 70px 64px',
              borderColor: 'var(--color-border)', color: 'var(--color-text-muted)',
            }}>
            <span>Market</span>
            <span className="text-right">Reward/d</span>
            <span className="text-right">Max spread</span>
            <span className="text-right">Min size</span>
            <span className="text-right">Days left</span>
            <span className="text-right">Safety</span>
            <span className="text-right">Quote</span>
          </div>
          {markets.map((m, i) => (
            <div key={m.condition_id + i}
              className="grid text-xs items-center px-3 py-2.5 border-b hover:bg-white/5"
              style={{
                gridTemplateColumns: '1fr 70px 80px 70px 70px 70px 64px',
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
              <span className="text-right">
                {m.yes_token_id && m.no_token_id ? (
                  <button
                    onClick={() => { setQuoteMarket(m); setQuoteSize(String(m.min_size)); setQuoteResult(null) }}
                    className="px-2 py-0.5 rounded text-[10px] font-semibold"
                    style={{ background: 'var(--color-accent)', color: '#000' }}
                    title="Post a two-sided maker quote"
                  >Quote</button>
                ) : (
                  <span className="text-[10px]" style={{ color: 'var(--color-text-muted)' }}>—</span>
                )}
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

      {/* Quote modal — post a two-sided maker quote */}
      {quoteMarket && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setQuoteMarket(null)}>
          <div className="rounded-xl border w-full max-w-md" style={{ background: 'var(--color-surface)', borderColor: 'var(--color-border)' }} onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between p-4 border-b" style={{ borderColor: 'var(--color-border)' }}>
              <span className="text-sm font-semibold" style={{ color: 'var(--color-text)' }}>Post Maker Quote</span>
              <button onClick={() => setQuoteMarket(null)}><X size={14} style={{ color: 'var(--color-text-muted)' }} /></button>
            </div>
            <div className="p-4 space-y-3">
              <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>{quoteMarket.question}</p>
              <div className="text-[11px] flex gap-3" style={{ color: 'var(--color-text-muted)' }}>
                <span>reward {quoteMarket.daily_rate.toFixed(0)}/d</span>
                <span>band {quoteMarket.max_spread}¢</span>
                <span>min ${quoteMarket.min_size}</span>
                {quoteMarket.is_toxic && <span style={{ color: SAFETY_COLOR.toxic }}>⚠ toxic</span>}
              </div>
              <label className="block text-xs" style={{ color: 'var(--color-text-muted)' }}>
                Size per side ($) — total = 2× this
                <input type="number" value={quoteSize} onChange={e => setQuoteSize(e.target.value)} min={quoteMarket.min_size}
                  className="w-full mt-1 px-2 py-1.5 rounded border text-xs"
                  style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }} />
                {Number(quoteSize) < quoteMarket.min_size && (
                  <span style={{ color: 'var(--color-warning)' }}>Below min_size ${quoteMarket.min_size} — won't earn rewards.</span>
                )}
              </label>
              <label className="block text-xs" style={{ color: 'var(--color-text-muted)' }}>
                Offset inside band (¢) — distance from mid each side
                <input type="number" value={quoteOffset} onChange={e => setQuoteOffset(e.target.value)} min={0.1} step={0.1} max={quoteMarket.max_spread / 2}
                  className="w-full mt-1 px-2 py-1.5 rounded border text-xs"
                  style={{ background: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-text)' }} />
              </label>
              {quoteResult && (
                <p className="text-xs p-2 rounded" style={{
                  background: quoteResult.startsWith('✓') ? 'rgba(74,222,128,0.1)' : 'rgba(239,68,68,0.1)',
                  color: quoteResult.startsWith('✓') ? '#4ade80' : '#f87171' }}>{quoteResult}</p>
              )}
            </div>
            <div className="flex gap-2 p-4 border-t" style={{ borderColor: 'var(--color-border)' }}>
              <button onClick={() => setQuoteMarket(null)} className="flex-1 px-3 py-2 rounded text-xs"
                style={{ background: 'var(--color-base)', color: 'var(--color-text)', border: '1px solid var(--color-border)' }}>Close</button>
              <button onClick={submitQuote} disabled={quoteBusy}
                className="flex-1 px-3 py-2 rounded text-xs font-semibold"
                style={{ background: 'var(--color-accent)', color: '#000' }}>
                {quoteBusy ? 'Placing…' : 'Place 2-sided quote'}
              </button>
            </div>
          </div>
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
