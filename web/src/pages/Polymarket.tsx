import { useState } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { BarChart2, Eye, EyeOff, Save, RefreshCw, TrendingUp } from 'lucide-react'

interface Market {
  id?: string
  question?: string
  yes_price?: number
  volume?: number
  end_date?: string
}

interface MarketsResponse {
  markets?: Market[]
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

export default function Polymarket() {
  const [walletAddress, setWalletAddress] = useState('')
  const [apiKey, setApiKey] = useState('')
  const [secret, setSecret] = useState('')
  const [passphrase, setPassphrase] = useState('')
  const [saveMsg, setSaveMsg] = useState('')
  const [saveErr, setSaveErr] = useState('')

  const { data: marketsData, isLoading: marketsLoading, refetch } = useQuery<MarketsResponse>({
    queryKey: ['polymarket-markets'],
    queryFn: (): Promise<MarketsResponse> =>
      apiFetch<MarketsResponse>('/api/polymarket/markets').catch(() => ({ markets: [] })),
  })

  const saveMutation = useMutation({
    mutationFn: () =>
      apiPost('/api/polymarket/configure', {
        wallet_address: walletAddress,
        api_key: apiKey,
        secret,
        passphrase,
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
      apiPost('/api/polymarket/test', {
        wallet_address: walletAddress,
        api_key: apiKey,
        secret,
        passphrase,
      }),
    onSuccess: () => {
      setSaveMsg('Connection OK!')
      setTimeout(() => setSaveMsg(''), 2000)
    },
    onError: (e: Error) => setSaveErr(e.message),
  })

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
        <h2 className="text-sm font-semibold mb-4">API Credentials</h2>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4">
          <div>
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
          <MaskedInput label="API Key" value={apiKey} onChange={setApiKey} />
          <MaskedInput label="Secret" value={secret} onChange={setSecret} />
          <MaskedInput label="Passphrase" value={passphrase} onChange={setPassphrase} />
        </div>

        {saveErr && (
          <p className="text-xs mb-3" style={{ color: 'var(--color-danger)' }}>{saveErr}</p>
        )}
        {saveMsg && (
          <p className="text-xs mb-3" style={{ color: 'var(--color-accent)' }}>{saveMsg}</p>
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
          >
            <RefreshCw size={13} className={marketsLoading ? 'animate-spin' : ''} />
          </button>
        </div>

        {marketsLoading ? (
          <div className="p-8 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>
            Loading markets...
          </div>
        ) : markets.length === 0 ? (
          <div className="p-8 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>
            No markets available. Configure credentials and test connection.
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
