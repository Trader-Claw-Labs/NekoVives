import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { Copy, Eye, EyeOff, Plus, X, Wallet } from 'lucide-react'
import clsx from 'clsx'

type Chain = 'evm' | 'solana' | 'ton'

interface WalletEntry {
  address: string
  chain: Chain
  label?: string
}

interface WalletsResponse {
  wallets?: WalletEntry[]
}

function maskAddress(addr: string): string {
  if (addr.length <= 12) return addr
  return `${addr.slice(0, 6)}...${addr.slice(-4)}`
}

function ChainBadge({ chain }: { chain: Chain }) {
  const colors: Record<Chain, string> = {
    evm: '#627eea',
    solana: '#9945ff',
    ton: '#0098ea',
  }
  return (
    <span
      className="text-xs px-2 py-0.5 rounded font-bold uppercase"
      style={{ backgroundColor: `${colors[chain]}22`, color: colors[chain], border: `1px solid ${colors[chain]}44` }}
    >
      {chain}
    </span>
  )
}

interface CreateModalProps {
  chain: Chain
  onClose: () => void
  onCreated: () => void
}

function CreateModal({ chain, onClose, onCreated }: CreateModalProps) {
  const [password, setPassword] = useState('')
  const [showPw, setShowPw] = useState(false)
  const [error, setError] = useState('')

  const mutation = useMutation({
    mutationFn: () => apiPost('/api/wallets/create', { chain, password }),
    onSuccess: () => { onCreated(); onClose() },
    onError: (e: Error) => setError(e.message),
  })

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div
        className="rounded-lg border w-full max-w-sm p-6"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center justify-between mb-4">
          <h2 className="font-semibold">Create {chain.toUpperCase()} Wallet</h2>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>

        <div className="mb-4">
          <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
            Encryption Password
          </label>
          <div className="relative">
            <input
              type={showPw ? 'text' : 'password'}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm pr-10"
              placeholder="Strong password..."
            />
            <button
              className="absolute right-2 top-1/2 -translate-y-1/2"
              onClick={() => setShowPw((s) => !s)}
              style={{ color: 'var(--color-text-muted)' }}
            >
              {showPw ? <EyeOff size={14} /> : <Eye size={14} />}
            </button>
          </div>
        </div>

        {error && (
          <p className="text-xs mb-3" style={{ color: 'var(--color-danger)' }}>{error}</p>
        )}

        <button
          onClick={() => mutation.mutate()}
          disabled={!password || mutation.isPending}
          className="w-full py-2 rounded text-sm font-medium transition-opacity disabled:opacity-50"
          style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
        >
          {mutation.isPending ? 'Creating...' : 'Create Wallet'}
        </button>
      </div>
    </div>
  )
}

interface WalletRowProps {
  wallet: WalletEntry
}

function WalletRow({ wallet }: WalletRowProps) {
  const [revealed, setRevealed] = useState(false)
  const [copied, setCopied] = useState(false)

  const display = revealed ? wallet.address : maskAddress(wallet.address)

  function copy() {
    navigator.clipboard.writeText(wallet.address)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }

  return (
    <div
      className="flex items-center justify-between px-4 py-3 border-b last:border-0"
      style={{ borderColor: 'var(--color-border)' }}
    >
      <div className="flex items-center gap-3 min-w-0">
        <ChainBadge chain={wallet.chain} />
        <span
          className="font-mono text-sm truncate"
          style={{ color: 'var(--color-text)' }}
        >
          {display}
        </span>
      </div>
      <div className="flex items-center gap-2 ml-4 flex-shrink-0">
        <button
          onClick={() => setRevealed((r) => !r)}
          className="p-1 rounded hover:bg-white/5 transition-colors"
          style={{ color: 'var(--color-text-muted)' }}
          title={revealed ? 'Hide' : 'Reveal'}
        >
          {revealed ? <EyeOff size={14} /> : <Eye size={14} />}
        </button>
        <button
          onClick={copy}
          className="p-1 rounded hover:bg-white/5 transition-colors"
          style={{ color: copied ? 'var(--color-accent)' : 'var(--color-text-muted)' }}
          title="Copy address"
        >
          <Copy size={14} />
        </button>
      </div>
    </div>
  )
}

export default function Wallets() {
  const [activeTab, setActiveTab] = useState<Chain>('evm')
  const [showModal, setShowModal] = useState(false)
  const qc = useQueryClient()

  const { data, isLoading } = useQuery<WalletsResponse>({
    queryKey: ['wallets'],
    queryFn: (): Promise<WalletsResponse> =>
      apiFetch<WalletsResponse>('/api/wallets').catch(() => ({ wallets: [] })),
  })

  const wallets = data?.wallets ?? []
  const filtered = wallets.filter((w) => w.chain === activeTab)

  const tabs: readonly Chain[] = ['evm', 'solana', 'ton'] as const

  return (
    <div className="p-6 max-w-4xl mx-auto">
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-2">
          <Wallet size={18} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-lg font-bold">Wallets</h1>
        </div>
        <button
          onClick={() => setShowModal(true)}
          className="flex items-center gap-2 px-3 py-1.5 rounded text-sm font-medium"
          style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
        >
          <Plus size={14} />
          Create Wallet
        </button>
      </div>

      {/* Chain tabs */}
      <div
        className="flex gap-1 mb-4 p-1 rounded"
        style={{ backgroundColor: 'var(--color-surface)' }}
      >
        {tabs.map((tab) => {
          const count = wallets.filter((w) => w.chain === tab).length
          return (
            <button
              key={tab}
              onClick={() => setActiveTab(tab)}
              className={clsx(
                'flex-1 py-1.5 rounded text-sm font-medium transition-colors',
                activeTab === tab ? 'text-black' : 'hover:text-white'
              )}
              style={
                activeTab === tab
                  ? { backgroundColor: 'var(--color-accent)', color: '#000' }
                  : { color: 'var(--color-text-muted)' }
              }
            >
              {tab.toUpperCase()} {count > 0 && <span className="ml-1 opacity-70">({count})</span>}
            </button>
          )
        })}
      </div>

      {/* Wallet list */}
      <div
        className="rounded-lg border overflow-hidden"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        {isLoading ? (
          <div className="p-8 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>
            Loading...
          </div>
        ) : filtered.length === 0 ? (
          <div className="p-8 text-center">
            <Wallet size={32} className="mx-auto mb-3" style={{ color: 'var(--color-text-muted)' }} />
            <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
              No {activeTab.toUpperCase()} wallets yet
            </p>
            <button
              onClick={() => setShowModal(true)}
              className="mt-3 text-xs underline"
              style={{ color: 'var(--color-accent)' }}
            >
              Create one
            </button>
          </div>
        ) : (
          filtered.map((w, i) => <WalletRow key={`${w.chain}-${w.address}-${i}`} wallet={w} />)
        )}
      </div>

      {showModal && (
        <CreateModal
          chain={activeTab}
          onClose={() => setShowModal(false)}
          onCreated={() => qc.invalidateQueries({ queryKey: ['wallets'] })}
        />
      )}
    </div>
  )
}
