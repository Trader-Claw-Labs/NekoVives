import { useState, useEffect } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { BarChart2, Eye, EyeOff, Save, RefreshCw, TrendingUp, Plus, X, CheckCircle, AlertCircle, ChevronDown, ChevronUp, ExternalLink } from 'lucide-react'

interface Market {
  id?: string
  question?: string
  yes_price?: number
  volume?: number
  end_date?: string
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
}

interface ApiKeyEntry {
  id: string
  label: string
  key: string
  secret: string
  passphrase: string
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

export default function Polymarket() {
  const [walletAddress, setWalletAddress] = useState('')
  const [apiKeys, setApiKeys] = useState<ApiKeyEntry[]>([
    { id: genId(), label: 'API Key 1', key: '', secret: '', passphrase: '', show: false, expanded: true },
  ])
  const [saveMsg, setSaveMsg] = useState('')
  const [saveErr, setSaveErr] = useState('')
  const [testStatus, setTestStatus] = useState<'idle' | 'ok' | 'error'>('idle')
  const [testMsg, setTestMsg] = useState('')
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

  useEffect(() => {
    if (!savedConfig?.configured) return
    if (savedConfig.wallet_address) setWalletAddress(savedConfig.wallet_address)
    if (savedConfig.api_key_masked) {
      setApiKeys([{
        id: genId(),
        label: 'API Key 1',
        key: savedConfig.api_key_masked,
        secret: savedConfig.has_secret ? '••••••••' : '',
        passphrase: savedConfig.has_passphrase ? '••••••••' : '',
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
      apiPost<{ status: string; message?: string; error?: string }>('/api/polymarket/test', {
        wallet_address: walletAddress,
        api_key: apiKeys[0]?.key ?? '',
        secret: apiKeys[0]?.secret ?? '',
        passphrase: apiKeys[0]?.passphrase ?? '',
      }),
    onSuccess: (data) => {
      if (data.status === 'ok') {
        setTestStatus('ok')
        setTestMsg(data.message ?? 'Connection OK')
      } else {
        setTestStatus('error')
        setTestMsg(data.error ?? 'Unknown error')
      }
      setTimeout(() => setTestStatus('idle'), 4000)
    },
    onError: (e: Error) => {
      setTestStatus('error')
      setTestMsg(e.message)
      setTimeout(() => setTestStatus('idle'), 4000)
    },
  })

  function addApiKey() {
    setApiKeys((k) => [
      ...k.map((e) => ({ ...e, expanded: false })),
      { id: genId(), label: `API Key ${k.length + 1}`, key: '', secret: '', passphrase: '', show: false, expanded: true },
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
      <div className="flex items-center gap-2 mb-6">
        <BarChart2 size={18} style={{ color: 'var(--color-accent)' }} />
        <h1 className="text-lg font-bold">Polymarket</h1>
      </div>

      {/* Config form */}
      <div
        className="rounded-lg border p-5 mb-6"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-sm font-semibold">Builder API Credentials</h2>
          {savedConfig?.configured && (
            <span className="flex items-center gap-1.5 text-xs" style={{ color: 'var(--color-accent)' }}>
              <CheckCircle size={12} />
              Configured
              {savedConfig.api_key_masked && (
                <span className="font-mono ml-1" style={{ color: 'var(--color-text-muted)' }}>
                  {savedConfig.api_key_masked}
                </span>
              )}
            </span>
          )}
        </div>

        {/* How to get Builder API credentials guide */}
        <div
          className="rounded-lg border mb-5 overflow-hidden"
          style={{ borderColor: 'var(--color-border)' }}
        >
          <button
            type="button"
            onClick={() => setShowGuide((v) => !v)}
            className="w-full flex items-center gap-2 px-4 py-2.5 text-left text-xs hover:bg-white/5 transition-colors"
            style={{ backgroundColor: 'var(--color-surface-2)' }}
          >
            <span className="font-medium" style={{ color: 'var(--color-text)' }}>
              How to get Builder API credentials
            </span>
            <span className="ml-auto" style={{ color: 'var(--color-text-muted)' }}>
              {showGuide ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
            </span>
          </button>

          {showGuide && (
            <div
              className="px-4 py-3 border-t text-xs space-y-3"
              style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
            >
              <p>
                Polymarket uses a two-layer auth system. The <strong style={{ color: 'var(--color-text)' }}>Builder API</strong> (L2) requires
                an API key, secret, and passphrase tied to your Polygon wallet.
              </p>

              <ol className="space-y-2 list-none">
                {[
                  {
                    n: '1',
                    text: (
                      <>
                        Go to{' '}
                        <a
                          href="https://polymarket.com"
                          target="_blank"
                          rel="noopener noreferrer"
                          className="inline-flex items-center gap-1 hover:underline"
                          style={{ color: 'var(--color-accent)' }}
                        >
                          polymarket.com <ExternalLink size={10} />
                        </a>{' '}
                        and connect your Polygon wallet (MetaMask, WalletConnect, etc.).
                      </>
                    ),
                  },
                  {
                    n: '2',
                    text: (
                      <>
                        Open the{' '}
                        <a
                          href="https://docs.polymarket.com/#get-api-keys"
                          target="_blank"
                          rel="noopener noreferrer"
                          className="inline-flex items-center gap-1 hover:underline"
                          style={{ color: 'var(--color-accent)' }}
                        >
                          CLOB API docs <ExternalLink size={10} />
                        </a>{' '}
                        and call <code className="px-1 rounded" style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-accent)' }}>POST /auth/api-key</code> signed with your wallet to generate credentials.
                      </>
                    ),
                  },
                  {
                    n: '3',
                    text: <>The response returns <code className="px-1 rounded" style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-accent)' }}>apiKey</code>, <code className="px-1 rounded" style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-accent)' }}>secret</code>, and <code className="px-1 rounded" style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-accent)' }}>passphrase</code>. Copy all three — the secret and passphrase are shown only once.</>,
                  },
                  {
                    n: '4',
                    text: <>Paste them into the fields below along with your Polygon wallet address (<code className="px-1 rounded" style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-accent)' }}>0x…</code>) and click <strong>Save</strong>.</>,
                  },
                ].map(({ n, text }) => (
                  <li key={n} className="flex gap-2">
                    <span
                      className="flex-shrink-0 w-5 h-5 rounded-full flex items-center justify-center text-xs font-bold"
                      style={{ backgroundColor: 'rgba(0,255,136,0.12)', color: 'var(--color-accent)' }}
                    >
                      {n}
                    </span>
                    <span className="leading-relaxed">{text}</span>
                  </li>
                ))}
              </ol>

              <div
                className="rounded p-2.5 flex gap-2 text-xs"
                style={{ backgroundColor: 'rgba(255,170,0,0.08)', borderLeft: '2px solid var(--color-warning)', color: 'var(--color-warning)' }}
              >
                <span className="flex-shrink-0">⚠</span>
                <span>
                  Use a <strong>dedicated Polygon wallet</strong> for Polymarket — never your main hot wallet.
                  Keep your secret and passphrase private; anyone with them can place orders on your behalf.
                </span>
              </div>
            </div>
          )}
        </div>

        <div className="mb-4">
          <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
            Polygon Wallet Address
          </label>
          <input
            type="text"
            value={walletAddress}
            onChange={(e) => setWalletAddress(e.target.value)}
            className="w-full rounded px-3 py-2 text-sm font-mono"
            placeholder="0x..."
          />
        </div>

        {/* Multiple API Keys — each with its own secret + passphrase */}
        <div className="mb-4">
          <div className="flex items-center justify-between mb-2">
            <label className="text-xs font-medium" style={{ color: 'var(--color-text-muted)' }}>
              API Keys
            </label>
            <button
              onClick={addApiKey}
              className="flex items-center gap-1 text-xs px-2 py-1 rounded border"
              style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
            >
              <Plus size={11} />
              Add Key
            </button>
          </div>

          <div className="space-y-2">
            {apiKeys.map((entry) => (
              <div
                key={entry.id}
                className="rounded-lg border overflow-hidden"
                style={{ borderColor: 'var(--color-border)' }}
              >
                {/* Key header row */}
                <div
                  className="flex gap-2 items-center px-3 py-2"
                  style={{ backgroundColor: 'var(--color-surface-2)' }}
                >
                  <input
                    type="text"
                    value={entry.label}
                    onChange={(e) => updateApiKey(entry.id, 'label', e.target.value)}
                    className="rounded px-2 py-1 text-xs w-28 flex-shrink-0"
                    placeholder="Label"
                    style={{ backgroundColor: 'var(--color-surface)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                  />
                  <div className="relative flex-1">
                    <input
                      type={entry.show ? 'text' : 'password'}
                      value={entry.key}
                      onChange={(e) => updateApiKey(entry.id, 'key', e.target.value)}
                      className="w-full rounded px-3 py-1 text-xs pr-8 font-mono"
                      placeholder="API key..."
                      style={{ backgroundColor: 'var(--color-surface)', border: '1px solid var(--color-border)', color: 'var(--color-text)' }}
                    />
                    <button
                      type="button"
                      className="absolute right-2 top-1/2 -translate-y-1/2"
                      onClick={() => updateApiKey(entry.id, 'show', !entry.show)}
                      style={{ color: 'var(--color-text-muted)' }}
                    >
                      {entry.show ? <EyeOff size={11} /> : <Eye size={11} />}
                    </button>
                  </div>
                  <button
                    type="button"
                    onClick={() => toggleExpand(entry.id)}
                    className="p-1 rounded hover:bg-white/5 flex-shrink-0"
                    style={{ color: 'var(--color-text-muted)' }}
                    title={entry.expanded ? 'Hide secret & passphrase' : 'Set secret & passphrase'}
                  >
                    {entry.expanded ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
                  </button>
                  {apiKeys.length > 1 && (
                    <button
                      onClick={() => removeApiKey(entry.id)}
                      className="p-1 rounded hover:bg-white/5 flex-shrink-0"
                      style={{ color: 'var(--color-text-muted)' }}
                    >
                      <X size={13} />
                    </button>
                  )}
                </div>

                {/* Secret + Passphrase (expandable) */}
                {entry.expanded && (
                  <div
                    className="px-3 py-3 grid grid-cols-1 md:grid-cols-2 gap-3 border-t"
                    style={{ borderColor: 'var(--color-border)' }}
                  >
                    <MaskedInput
                      label="Secret"
                      value={entry.secret}
                      onChange={(v) => updateApiKey(entry.id, 'secret', v)}
                    />
                    <MaskedInput
                      label="Passphrase"
                      value={entry.passphrase}
                      onChange={(v) => updateApiKey(entry.id, 'passphrase', v)}
                    />
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>

        {saveErr && (
          <p className="text-xs mb-3" style={{ color: 'var(--color-danger)' }}>{saveErr}</p>
        )}
        {saveMsg && (
          <p className="text-xs mb-3" style={{ color: 'var(--color-accent)' }}>{saveMsg}</p>
        )}

        {/* Test connection status */}
        {testStatus !== 'idle' && (
          <div
            className="flex items-center gap-2 text-xs mb-3 px-3 py-2 rounded"
            style={{
              backgroundColor: testStatus === 'ok' ? 'rgba(0,255,136,0.08)' : 'rgba(255,68,68,0.08)',
              color: testStatus === 'ok' ? 'var(--color-accent)' : 'var(--color-danger)',
              border: `1px solid ${testStatus === 'ok' ? 'rgba(0,255,136,0.3)' : 'rgba(255,68,68,0.3)'}`,
            }}
          >
            {testStatus === 'ok' ? <CheckCircle size={13} /> : <AlertCircle size={13} />}
            {testMsg}
          </div>
        )}

        <div className="flex gap-2">
          <button
            onClick={() => saveMutation.mutate()}
            disabled={saveMutation.isPending}
            className="flex items-center gap-2 px-4 py-2 rounded text-sm font-medium disabled:opacity-50"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            <Save size={14} />
            Save
          </button>
          <button
            onClick={() => testMutation.mutate()}
            disabled={testMutation.isPending}
            className="flex items-center gap-2 px-4 py-2 rounded text-sm border disabled:opacity-50 hover:bg-white/5 transition-colors"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text)' }}
          >
            <RefreshCw size={14} className={testMutation.isPending ? 'animate-spin' : ''} />
            Test Connection
          </button>
        </div>
      </div>

      {/* Markets */}
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
                  <p className="text-sm leading-snug">{m.question ?? 'Unknown market'}</p>
                  <span
                    className="text-xs flex-shrink-0"
                    style={{ color: 'var(--color-text-muted)' }}
                  >
                    {formatVolume(m.volume)}
                  </span>
                </div>
                <PriceBar yes={m.yes_price ?? 0.5} />
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
