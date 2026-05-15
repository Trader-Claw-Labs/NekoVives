import { useState, useEffect } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost, apiDelete } from '../hooks/useApi'
import { BarChart2, Eye, EyeOff, RefreshCw, TrendingUp, X, CheckCircle, AlertCircle, Trash2 } from 'lucide-react'
import PolymarketSetupWizard from '../components/PolymarketSetupWizard'

interface Market {
  id?: string
  question?: string
  yes_price?: number
  volume?: number
  end_date?: string
  yes_token_id?: string
}

interface MarketsResponse {
  markets?: Market[]
  error?: string
}

interface PolyConfigData {
  configured?: boolean
  api_key_masked?: string
  wallet_address?: string
  has_secret?: boolean
  has_passphrase?: boolean
  has_private_key?: boolean
}

interface ApiKeyEntry {
  id: string
  label: string
  key: string
  secret: string
  passphrase: string
  private_key: string
  show: boolean
  expanded: boolean
}

function MaskedInput({
  label,
  value,
  onChange,
  placeholder,
}: {
  label: string
  value: string
  onChange: (v: string) => void
  placeholder?: string
}) {
  const [show, setShow] = useState(false)
  return (
    <div>
      <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
        {label}
      </label>
      <div className="relative">
        <input
          type={show ? 'text' : 'password'}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="w-full rounded px-3 py-2 text-sm pr-10 font-mono"
          placeholder={placeholder ?? '••••••••'}
        />
        <button
          className="absolute right-2 top-1/2 -translate-y-1/2"
          onClick={() => setShow((s) => !s)}
          style={{ color: 'var(--color-text-muted)' }}
          type="button"
        >
          {show ? <EyeOff size={14} /> : <Eye size={14} />}
        </button>
      </div>
    </div>
  )
}

// ── Sparkline ─────────────────────────────────────────────────────────

interface SparkPoint { t: number; p: number }

function Sparkline({ tokenId }: { tokenId: string }) {
  const { data } = useQuery<{ history: SparkPoint[] }>({
    queryKey: ['poly-sparkline', tokenId],
    queryFn: () => apiFetch(`/api/polymarket/prices-history?token_id=${encodeURIComponent(tokenId)}&interval=1d`),
    staleTime: 5 * 60_000,
    retry: false,
  })

  const pts = data?.history ?? []
  if (pts.length < 2) {
    return <div className="w-20 h-8 opacity-20 text-xs flex items-center justify-center" style={{ color: 'var(--color-text-muted)' }}>—</div>
  }

  const prices = pts.map(p => p.p)
  const min = Math.min(...prices)
  const max = Math.max(...prices)
  const range = max - min || 0.01
  const W = 80, H = 32
  const points = prices.map((p, i) => {
    const x = (i / (prices.length - 1)) * W
    const y = H - ((p - min) / range) * (H - 4) - 2
    return `${x},${y}`
  }).join(' ')

  const first = prices[0]
  const last = prices[prices.length - 1]
  const up = last >= first
  const color = up ? 'var(--color-accent)' : 'var(--color-danger)'

  return (
    <svg width={W} height={H} viewBox={`0 0 ${W} ${H}`} className="flex-shrink-0">
      <polyline points={points} fill="none" stroke={color} strokeWidth="1.5" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  )
}

function PriceBar({ yes }: { yes: number }) {
  const pct = Math.round(yes * 100)
  return (
    <div className="flex items-center gap-2 text-xs">
      <div className="flex-1 h-1.5 rounded-full overflow-hidden" style={{ backgroundColor: 'var(--color-border)' }}>
        <div
          className="h-full rounded-full transition-all"
          style={{
            width: `${pct}%`,
            backgroundColor: pct > 50 ? 'var(--color-accent)' : 'var(--color-danger)',
          }}
        />
      </div>
      <span
        className="w-10 text-right font-bold"
        style={{ color: pct > 50 ? 'var(--color-accent)' : 'var(--color-danger)' }}
      >
        {pct}%
      </span>
    </div>
  )
}

function formatVolume(vol?: number): string {
  if (!vol) return '—'
  if (vol >= 1_000_000) return `$${(vol / 1_000_000).toFixed(1)}M`
  if (vol >= 1_000) return `$${(vol / 1_000).toFixed(1)}K`
  return `$${vol.toFixed(0)}`
}

function genId() {
  return Math.random().toString(36).slice(2, 9)
}

// ── Orders & Positions types ──────────────────────────────────────────

interface PolyOrder {
  id: string
  asset_id?: string
  market?: string
  side?: string
  price?: number
  size?: number
  size_matched?: number
  status?: string
  created_at?: string
}

interface PolyPosition {
  asset_id?: string
  market?: string
  outcome?: string
  size?: number
  avg_price?: number
  value?: number
}

// ── Orders Tab ────────────────────────────────────────────────────────

function OrdersTab() {
  const qc = useQueryClient()
  const { data, isLoading, refetch } = useQuery<{ orders: PolyOrder[] }>({
    queryKey: ['poly-orders'],
    queryFn: () => apiFetch<{ orders: PolyOrder[] }>('/api/polymarket/orders').catch(() => ({ orders: [] })),
    refetchInterval: 15_000,
  })

  const cancelMutation = useMutation({
    mutationFn: (id: string) => apiDelete(`/api/polymarket/order/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['poly-orders'] }),
  })

  const orders = data?.orders ?? []

  return (
    <div>
      <div className="flex items-center justify-between mb-3">
        <h2 className="text-sm font-semibold">Open Orders</h2>
        <button onClick={() => refetch()} className="p-1.5 rounded hover:bg-white/5"
          style={{ color: 'var(--color-text-muted)' }}>
          <RefreshCw size={12} className={isLoading ? 'animate-spin' : ''} />
        </button>
      </div>
      {isLoading ? (
        <div className="p-6 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>Loading...</div>
      ) : orders.length === 0 ? (
        <div className="p-8 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>No open orders</div>
      ) : (
        <div className="space-y-2">
          {orders.map(o => (
            <div key={o.id} className="rounded-lg border p-3 flex items-center justify-between text-xs"
              style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)' }}>
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 mb-1">
                  <span className="font-mono truncate max-w-32">{o.id}</span>
                  <span className="px-1.5 py-0.5 rounded"
                    style={{ backgroundColor: o.side === 'buy' ? 'rgba(74,222,128,0.1)' : 'rgba(239,68,68,0.1)',
                      color: o.side === 'buy' ? 'var(--color-accent)' : 'var(--color-danger)' }}>
                    {o.side?.toUpperCase() ?? '—'}
                  </span>
                  {o.status && <span style={{ color: 'var(--color-text-muted)' }}>{o.status}</span>}
                </div>
                <div className="flex gap-3" style={{ color: 'var(--color-text-muted)' }}>
                  {o.price !== undefined && <span>Price: <b style={{ color: 'var(--color-text)' }}>{o.price}</b></span>}
                  {o.size !== undefined && <span>Size: <b style={{ color: 'var(--color-text)' }}>{o.size}</b></span>}
                  {o.size_matched !== undefined && <span>Filled: <b style={{ color: 'var(--color-text)' }}>{o.size_matched}</b></span>}
                </div>
              </div>
              <button onClick={() => {
                if (confirm(`Cancel order ${o.id}?`)) cancelMutation.mutate(o.id)
              }} className="p-1.5 rounded hover:bg-white/5 ml-2" style={{ color: 'var(--color-danger)' }}
                title="Cancel order">
                <Trash2 size={13} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ── Positions Tab ─────────────────────────────────────────────────────

function PositionsTab() {
  const { data, isLoading, refetch } = useQuery<{ positions: PolyPosition[] }>({
    queryKey: ['poly-positions'],
    queryFn: () => apiFetch<{ positions: PolyPosition[] }>('/api/polymarket/positions').catch(() => ({ positions: [] })),
    refetchInterval: 15_000,
  })

  const positions = data?.positions ?? []

  return (
    <div>
      <div className="flex items-center justify-between mb-3">
        <h2 className="text-sm font-semibold">Positions</h2>
        <button onClick={() => refetch()} className="p-1.5 rounded hover:bg-white/5"
          style={{ color: 'var(--color-text-muted)' }}>
          <RefreshCw size={12} className={isLoading ? 'animate-spin' : ''} />
        </button>
      </div>
      {isLoading ? (
        <div className="p-6 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>Loading...</div>
      ) : positions.length === 0 ? (
        <div className="p-8 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>No positions</div>
      ) : (
        <div className="space-y-2">
          {positions.map((p, i) => (
            <div key={i} className="rounded-lg border p-3 text-xs"
              style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)' }}>
              <div className="flex justify-between mb-1">
                <span className="font-semibold truncate max-w-64">{p.market ?? p.asset_id ?? '—'}</span>
                <span className="px-1.5 py-0.5 rounded" style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-accent)' }}>
                  {p.outcome ?? '—'}
                </span>
              </div>
              <div className="flex gap-3" style={{ color: 'var(--color-text-muted)' }}>
                {p.size !== undefined && <span>Size: <b style={{ color: 'var(--color-text)' }}>{p.size}</b></span>}
                {p.avg_price !== undefined && <span>Avg: <b style={{ color: 'var(--color-text)' }}>{p.avg_price}</b></span>}
                {p.value !== undefined && <span>Value: <b style={{ color: 'var(--color-accent)' }}>${p.value.toFixed(2)}</b></span>}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ── Place Order Tab ───────────────────────────────────────────────────

function PlaceOrderTab() {
  const [form, setForm] = useState({
    token_id: '',
    side: 'buy',
    price: '',
    size: '',
    order_type: 'limit',
  })
  const [result, setResult] = useState<{ status: string; order_id?: string; error?: string } | null>(null)

  const mutation = useMutation({
    mutationFn: () => apiPost<{ status: string; order_id?: string; error?: string }>('/api/polymarket/order', {
      token_id: form.token_id,
      side: form.side,
      price: form.order_type === 'market' ? undefined : parseFloat(form.price),
      size: parseFloat(form.size),
      order_type: form.order_type,
    }),
    onSuccess: (data) => setResult(data),
    onError: (e: Error) => setResult({ status: 'error', error: e.message }),
  })

  function set(k: keyof typeof form, v: string) {
    setForm(f => ({ ...f, [k]: v }))
  }

  const canSubmit = !!form.token_id && !!form.size && (form.order_type === 'market' || !!form.price)

  return (
    <div className="max-w-md">
      <h2 className="text-sm font-semibold mb-4">Place Order</h2>
      <div className="space-y-3">
        <div>
          <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Token ID (CLOB token)</label>
          <input className="w-full rounded px-3 py-2 text-sm font-mono" value={form.token_id}
            onChange={e => set('token_id', e.target.value)} placeholder="71321045679252212..." />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Side</label>
            <select className="w-full rounded px-3 py-2 text-sm" value={form.side}
              onChange={e => set('side', e.target.value)}>
              <option value="buy">Buy (YES)</option>
              <option value="sell">Sell (NO)</option>
            </select>
          </div>
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Order Type</label>
            <select className="w-full rounded px-3 py-2 text-sm" value={form.order_type}
              onChange={e => set('order_type', e.target.value)}>
              <option value="limit">Limit</option>
              <option value="market">Market</option>
            </select>
          </div>
        </div>
        <div className="grid grid-cols-2 gap-3">
          {form.order_type === 'limit' && (
            <div>
              <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Price (0–1)</label>
              <input type="number" step="0.01" min="0" max="1" className="w-full rounded px-3 py-2 text-sm"
                value={form.price} onChange={e => set('price', e.target.value)} placeholder="0.65" />
            </div>
          )}
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Size (USDC)</label>
            <input type="number" step="1" min="0" className="w-full rounded px-3 py-2 text-sm"
              value={form.size} onChange={e => set('size', e.target.value)} placeholder="10" />
          </div>
        </div>

        {result && (
          <div className="rounded px-3 py-2 text-xs"
            style={{
              backgroundColor: result.status === 'ok' ? 'rgba(74,222,128,0.1)' : 'rgba(239,68,68,0.1)',
              color: result.status === 'ok' ? 'var(--color-accent)' : 'var(--color-danger)',
              border: `1px solid ${result.status === 'ok' ? 'rgba(74,222,128,0.3)' : 'rgba(239,68,68,0.3)'}`,
            }}>
            {result.status === 'ok'
              ? `Order placed! ID: ${result.order_id ?? '—'}`
              : result.error ?? 'Unknown error'}
          </div>
        )}

        <button onClick={() => mutation.mutate()} disabled={!canSubmit || mutation.isPending}
          className="w-full py-2 rounded text-sm font-medium disabled:opacity-50"
          style={{ backgroundColor: form.side === 'buy' ? 'var(--color-accent)' : 'var(--color-danger)', color: '#000' }}>
          {mutation.isPending ? 'Placing...' : `${form.side === 'buy' ? 'Buy' : 'Sell'} ${form.order_type}`}
        </button>
      </div>
    </div>
  )
}

export default function Polymarket() {
  const [activeTab, setActiveTab] = useState<'markets' | 'orders' | 'positions'>('markets')
  const [walletAddress, setWalletAddress] = useState('')
  const [signatureType, setSignatureType] = useState('')
  const [apiKeys, setApiKeys] = useState<ApiKeyEntry[]>([
    { id: genId(), label: 'API Key 1', key: '', secret: '', passphrase: '', private_key: '', show: false, expanded: true },
  ])
  const [saveMsg, setSaveMsg] = useState('')
  const [saveErr, setSaveErr] = useState('')
  const [testStatus, setTestStatus] = useState<'idle' | 'ok' | 'error'>('idle')
  const [testMsg, setTestMsg] = useState('')
  const [testDetails, setTestDetails] = useState<{
    http_status?: number
    strategy?: string
    last_strategy?: string
    api_key_preview?: string
    api_key_length?: number
    secret_length?: number
    passphrase_length?: number
    wallet_address?: string
    response_preview?: string
  } | null>(null)
  const [showGuide, setShowGuide] = useState(false)

  const { data: marketsData, isLoading: marketsLoading, refetch, error: marketsError } = useQuery<MarketsResponse>({
    queryKey: ['polymarket-markets'],
    queryFn: () => apiFetch<MarketsResponse>('/api/polymarket/markets'),
    retry: 1,
  })

  const { data: savedConfig } = useQuery<PolyConfigData>({
    queryKey: ['polymarket-config'],
    queryFn: () => apiFetch('/api/polymarket/configure'),
  })

  const { data: balanceData, isLoading: balanceLoading, refetch: refetchBalance } = useQuery<{ balances?: { symbol: string; balance: string; chain: string }[] }>({
    queryKey: ['poly-balance', walletAddress],
    queryFn: () => walletAddress ? apiFetch(`/api/wallets/${encodeURIComponent(walletAddress)}/balance`) : Promise.resolve({}),
    enabled: !!walletAddress,
    refetchInterval: 30_000,
  })

  useEffect(() => {
    if (!savedConfig?.configured) return
    if (savedConfig.wallet_address) setWalletAddress(savedConfig.wallet_address)
    if ((savedConfig as any).signature_type) setSignatureType((savedConfig as any).signature_type)
    if (savedConfig.api_key_masked) {
      // Leave secret / passphrase / private_key blank. Backend preserves the
      // stored values when these fields come empty, so the user doesn't have
      // to re-type them. Previously we prefilled with "••••••••", which users
      // could accidentally save as the literal secret → 401 at request time.
      setApiKeys([{
        id: genId(),
        label: 'API Key 1',
        key: savedConfig.api_key_masked,
        secret: '',
        passphrase: '',
        private_key: '',
        show: false,
        expanded: false,
      }])
    }
  }, [savedConfig])

  const saveMutation = useMutation({
    mutationFn: () =>
      apiPost('/api/polymarket/configure', {
        wallet_address: walletAddress,
        api_key: apiKeys[0]?.key ?? '',
        secret: apiKeys[0]?.secret ?? '',
        passphrase: apiKeys[0]?.passphrase ?? '',
        private_key: apiKeys[0]?.private_key ?? '',
        signature_type: signatureType || undefined,
      }),
    onSuccess: () => {
      setSaveMsg('Saved!')
      setSaveErr('')
      setTimeout(() => setSaveMsg(''), 2000)
    },
    onError: (e: Error) => {
      setSaveErr(e.message)
      setSaveMsg('')
    },
  })

  const testMutation = useMutation({
    mutationFn: () =>
      apiPost<{
        status: string
        message?: string
        error?: string
        http_status?: number
        strategy?: string
        last_strategy?: string
        api_key_preview?: string
        api_key_length?: number
        secret_length?: number
        passphrase_length?: number
        wallet_address?: string
        response_preview?: string
      }>('/api/polymarket/test', {
        wallet_address: walletAddress,
        api_key: apiKeys[0]?.key ?? '',
        secret: apiKeys[0]?.secret ?? '',
        passphrase: apiKeys[0]?.passphrase ?? '',
        private_key: apiKeys[0]?.private_key ?? '',
      }),
    onSuccess: (data) => {
      if (data.status === 'ok') {
        setTestStatus('ok')
        setTestMsg(data.message ?? 'Connection OK')
      } else {
        setTestStatus('error')
        setTestMsg(data.error ?? 'Unknown error')
      }
      setTestDetails({
        http_status: data.http_status,
        strategy: data.strategy,
        last_strategy: data.last_strategy,
        api_key_preview: data.api_key_preview,
        api_key_length: data.api_key_length,
        secret_length: data.secret_length,
        passphrase_length: data.passphrase_length,
        wallet_address: data.wallet_address,
        response_preview: data.response_preview,
      })
      // Keep result visible — successful results clear after 8s, errors stay until the
      // user clicks Test again so they can read the diagnostics.
      if (data.status === 'ok') {
        setTimeout(() => {
          setTestStatus('idle')
          setTestDetails(null)
        }, 8000)
      }
    },
    onError: (e: Error) => {
      // Best-effort parse of the error body: apiPost throws Error(JSON.stringify(errBody))
      // or Error(plain text). Try to recover structured fields either way.
      let parsed: Record<string, unknown> | null = null
      try {
        const m = e.message.match(/\{.*\}/s)
        if (m) parsed = JSON.parse(m[0])
      } catch {
        /* ignore */
      }
      setTestStatus('error')
      setTestMsg(
        (parsed?.error as string | undefined) ??
          (parsed?.message as string | undefined) ??
          e.message,
      )
      setTestDetails(
        parsed
          ? {
              http_status: parsed.http_status as number | undefined,
              strategy: parsed.strategy as string | undefined,
              last_strategy: parsed.last_strategy as string | undefined,
              api_key_preview: parsed.api_key_preview as string | undefined,
              api_key_length: parsed.api_key_length as number | undefined,
              secret_length: parsed.secret_length as number | undefined,
              passphrase_length: parsed.passphrase_length as number | undefined,
              wallet_address: parsed.wallet_address as string | undefined,
              response_preview: parsed.response_preview as string | undefined,
            }
          : null,
      )
    },
  })

  // Regenerate API credentials via L1 EIP-712 auth using the saved private_key.
  // Autofills api_key/secret/passphrase in the first entry; user still has to click Save.
  const regenerateMutation = useMutation({
    mutationFn: () =>
      apiPost<{
        success?: boolean
        api_key?: string
        secret?: string
        passphrase?: string
        wallet_address?: string
        error?: string
      }>('/api/polymarket/refresh-credentials', {}),
    onSuccess: (data) => {
      if (!data.success || !data.api_key) {
        setSaveErr(data.error ?? 'Failed to regenerate credentials')
        return
      }
      setSaveErr('')
      setApiKeys((k) => {
        const first = k[0]
        const updated: ApiKeyEntry = {
          id: first?.id ?? genId(),
          label: first?.label ?? 'API Key 1',
          key: data.api_key ?? '',
          secret: data.secret ?? '',
          passphrase: data.passphrase ?? '',
          private_key: first?.private_key ?? '',
          show: true,
          expanded: true,
        }
        return [updated, ...k.slice(1)]
      })
      if (data.wallet_address) setWalletAddress(data.wallet_address)
      setSaveMsg('New credentials generated — click Save to store them.')
      setTimeout(() => setSaveMsg(''), 6000)
    },
    onError: (e: Error) => {
      setSaveErr(e.message)
    },
  })

  function addApiKey() {
    setApiKeys((k) => [
      ...k.map((e) => ({ ...e, expanded: false })),
      { id: genId(), label: `API Key ${k.length + 1}`, key: '', secret: '', passphrase: '', private_key: '', show: false, expanded: true },
    ])
  }

  function removeApiKey(id: string) {
    setApiKeys((k) => k.filter((entry) => entry.id !== id))
  }

  function updateApiKey(id: string, field: keyof ApiKeyEntry, value: string | boolean) {
    setApiKeys((k) => k.map((entry) =>
      entry.id === id ? { ...entry, [field]: value } : entry
    ))
  }

  function toggleExpand(id: string) {
    setApiKeys((k) => k.map((entry) =>
      entry.id === id ? { ...entry, expanded: !entry.expanded } : entry
    ))
  }

  const markets: Market[] = marketsData?.markets ?? []

  return (
    <div className="p-6 max-w-4xl mx-auto">
      <div className="flex items-center gap-2 mb-4">
        <BarChart2 size={18} style={{ color: 'var(--color-accent)' }} />
        <h1 className="text-lg font-bold">Polymarket</h1>
      </div>

      {/* Wallet Balance */}
      {walletAddress && (
        <div className="rounded-lg border p-4 mb-5 flex items-center justify-between"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
          <div>
            <p className="text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>Polygon Wallet Balance</p>
            <p className="text-xs font-mono truncate max-w-48" style={{ color: 'var(--color-text-muted)' }}>{walletAddress}</p>
          </div>
          <div className="flex items-center gap-3">
            {balanceLoading ? (
              <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>Loading...</span>
            ) : (
              <div className="flex gap-4">
                {(balanceData?.balances ?? []).filter(b => ['USDC', 'MATIC', 'POL'].includes(b.symbol)).map(b => (
                  <div key={b.symbol} className="text-right">
                    <p className="text-sm font-bold" style={{ color: 'var(--color-accent)' }}>{parseFloat(b.balance).toFixed(2)}</p>
                    <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>{b.symbol}</p>
                  </div>
                ))}
                {(!balanceData?.balances || balanceData.balances.length === 0) && (
                  <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>—</span>
                )}
              </div>
            )}
            <button onClick={() => refetchBalance()} className="p-1.5 rounded hover:bg-white/5"
              style={{ color: 'var(--color-text-muted)' }}>
              <RefreshCw size={12} className={balanceLoading ? 'animate-spin' : ''} />
            </button>
          </div>
        </div>
      )}

      {/* Tabs */}
      <div className="flex gap-1 mb-5 p-1 rounded" style={{ backgroundColor: 'var(--color-surface)' }}>
        {(['markets', 'orders', 'positions'] as const).map(tab => (
          <button key={tab} onClick={() => setActiveTab(tab)}
            className="flex-1 py-1.5 rounded text-sm font-medium transition-colors capitalize"
            style={activeTab === tab
              ? { backgroundColor: 'var(--color-accent)', color: '#000' }
              : { color: 'var(--color-text-muted)' }}>
            {tab.charAt(0).toUpperCase() + tab.slice(1)}
          </button>
        ))}
      </div>

      {/* Non-markets tabs */}
      {activeTab !== 'markets' && (
        <div className="rounded-lg border p-5 mb-6"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
          {activeTab === 'orders' && <OrdersTab />}
          {activeTab === 'positions' && <PositionsTab />}
        </div>
      )}

      {activeTab !== 'markets' ? null : <>


      {/* Polymarket setup wizard */}
      <PolymarketSetupWizard
        onComplete={() => window.location.reload()}
        onCancel={() => {}}
      />
      <div className="h-6" />

      {/* Markets tab content */}
      <div
        className="rounded-lg border"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div
          className="flex items-center justify-between px-5 py-3 border-b"
          style={{ borderColor: 'var(--color-border)' }}
        >
          <div className="flex items-center gap-2">
            <TrendingUp size={14} style={{ color: 'var(--color-accent)' }} />
            <h2 className="text-sm font-semibold">Top Markets</h2>
          </div>
          <button
            onClick={() => refetch()}
            className="p-1.5 rounded hover:bg-white/5 transition-colors"
            style={{ color: 'var(--color-text-muted)' }}
            title="Refresh"
          >
            <RefreshCw size={13} className={marketsLoading ? 'animate-spin' : ''} />
          </button>
        </div>

        {marketsLoading ? (
          <div className="p-8 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>
            Loading markets...
          </div>
        ) : marketsError || marketsData?.error ? (
          <div className="p-6 text-center">
            <AlertCircle size={28} className="mx-auto mb-2" style={{ color: 'var(--color-danger)' }} />
            <p className="text-sm" style={{ color: 'var(--color-danger)' }}>
              {marketsData?.error ?? String(marketsError)}
            </p>
            <button
              onClick={() => refetch()}
              className="mt-3 text-xs underline"
              style={{ color: 'var(--color-accent)' }}
            >
              Retry
            </button>
          </div>
        ) : markets.length === 0 ? (
          <div className="p-8 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>
            No markets available.
          </div>
        ) : (
          <div className="divide-y" style={{ borderColor: 'var(--color-border)' }}>
            {markets.map((m, i) => (
              <div key={m.id ?? i} className="px-5 py-4">
                <div className="flex items-start justify-between gap-4 mb-2">
                  <p className="text-sm leading-snug flex-1">{m.question ?? 'Unknown market'}</p>
                  <div className="flex items-center gap-3 flex-shrink-0">
                    {m.yes_token_id && <Sparkline tokenId={m.yes_token_id} />}
                    <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                      {formatVolume(m.volume)}
                    </span>
                  </div>
                </div>
                <PriceBar yes={m.yes_price ?? 0.5} />
              </div>
            ))}
          </div>
        )}
      </div>
      </>}
    </div>
  )
}
