import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { Copy, Eye, EyeOff, Plus, X, Wallet, Key, BookOpen, Tag } from 'lucide-react'
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

// ── Create Modal ───────────────────────────────────────────────────────────

interface CreateResult {
  address: string
  chain: string
  label: string
  mnemonic: string
}

interface CreateModalProps {
  chain: Chain
  onClose: () => void
  onCreated: () => void
}

function CreateModal({ chain, onClose, onCreated }: CreateModalProps) {
  const [password, setPassword] = useState('')
  const [confirmPw, setConfirmPw] = useState('')
  const [label, setLabel] = useState('')
  const [showPw, setShowPw] = useState(false)
  const [error, setError] = useState('')
  const [result, setResult] = useState<CreateResult | null>(null)
  const [mnemonicCopied, setMnemonicCopied] = useState(false)

  const mutation = useMutation({
    mutationFn: () => {
      if (password !== confirmPw) throw new Error('Passwords do not match')
      if (password.length < 8) throw new Error('Password must be at least 8 characters')
      return apiPost<{ wallet: CreateResult }>('/api/wallets/create', { chain, password, label })
    },
    onSuccess: (data) => {
      setResult(data.wallet)
      onCreated()
    },
    onError: (e: Error) => setError(e.message),
  })

  function copyMnemonic() {
    if (!result) return
    navigator.clipboard.writeText(result.mnemonic)
    setMnemonicCopied(true)
    setTimeout(() => setMnemonicCopied(false), 2000)
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div
        className="rounded-lg border w-full max-w-lg"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center justify-between px-6 py-4 border-b" style={{ borderColor: 'var(--color-border)' }}>
          <h2 className="font-semibold">{result ? 'Wallet Created' : `Create ${chain.toUpperCase()} Wallet`}</h2>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>

        <div className="p-6">
          {result ? (
            // ── Success: show mnemonic ──
            <div className="space-y-4">
              <div
                className="rounded px-3 py-2 text-xs font-mono border"
                style={{ backgroundColor: 'var(--color-surface-2)', borderColor: 'var(--color-border)', color: 'var(--color-accent)' }}
              >
                {result.address}
              </div>

              <div>
                <div className="flex items-center justify-between mb-2">
                  <div className="flex items-center gap-1.5">
                    <BookOpen size={13} style={{ color: 'var(--color-accent)' }} />
                    <span className="text-xs font-semibold uppercase tracking-widest" style={{ color: 'var(--color-text-muted)' }}>
                      Secret Recovery Phrase
                    </span>
                  </div>
                  <button
                    onClick={copyMnemonic}
                    className="text-xs flex items-center gap-1"
                    style={{ color: mnemonicCopied ? 'var(--color-accent)' : 'var(--color-text-muted)' }}
                  >
                    <Copy size={11} />
                    {mnemonicCopied ? 'Copied!' : 'Copy'}
                  </button>
                </div>
                <div
                  className="rounded p-3 text-sm font-mono leading-loose border"
                  style={{
                    backgroundColor: 'rgba(255,68,68,0.06)',
                    borderColor: 'rgba(255,68,68,0.3)',
                    color: 'var(--color-text)',
                  }}
                >
                  {result.mnemonic}
                </div>
                <p className="text-xs mt-2" style={{ color: 'var(--color-danger)' }}>
                  ⚠ Write this down and store it safely. It will never be shown again.
                </p>
              </div>

              <button
                onClick={onClose}
                className="w-full py-2 rounded text-sm font-medium"
                style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
              >
                Done — I saved my phrase
              </button>
            </div>
          ) : (
            // ── Create form ──
            <div className="space-y-4">
              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
                  <Tag size={11} className="inline mr-1" />
                  Alias (optional)
                </label>
                <input
                  type="text"
                  value={label}
                  onChange={(e) => setLabel(e.target.value)}
                  className="w-full rounded px-3 py-2 text-sm"
                  placeholder="My Trading Wallet"
                />
              </div>

              <div>
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
                    type="button"
                  >
                    {showPw ? <EyeOff size={14} /> : <Eye size={14} />}
                  </button>
                </div>
              </div>

              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
                  Confirm Password
                </label>
                <input
                  type="password"
                  value={confirmPw}
                  onChange={(e) => setConfirmPw(e.target.value)}
                  className="w-full rounded px-3 py-2 text-sm"
                  placeholder="Repeat password..."
                />
              </div>

              {error && (
                <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{error}</p>
              )}

              <button
                onClick={() => mutation.mutate()}
                disabled={!password || !confirmPw || mutation.isPending}
                className="w-full py-2 rounded text-sm font-medium transition-opacity disabled:opacity-50"
                style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
              >
                {mutation.isPending ? 'Creating...' : 'Create Wallet'}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

// ── Export Modal (mnemonic or private key) ─────────────────────────────────

interface ExportModalProps {
  wallet: WalletEntry
  exportType: 'mnemonic' | 'private_key'
  onClose: () => void
}

function ExportModal({ wallet, exportType, onClose }: ExportModalProps) {
  const [password, setPassword] = useState('')
  const [value, setValue] = useState('')
  const [error, setError] = useState('')
  const [copied, setCopied] = useState(false)

  const mutation = useMutation({
    mutationFn: () =>
      apiPost<{ value: string }>('/api/wallets/export', {
        address: wallet.address,
        password,
        export_type: exportType,
      }),
    onSuccess: (data) => setValue(data.value),
    onError: (e: Error) => setError(e.message),
  })

  const title = exportType === 'mnemonic' ? 'Secret Recovery Phrase' : 'Private Key'
  const Icon = exportType === 'mnemonic' ? BookOpen : Key

  function copy() {
    navigator.clipboard.writeText(value)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div
        className="rounded-lg border w-full max-w-md"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center justify-between px-6 py-4 border-b" style={{ borderColor: 'var(--color-border)' }}>
          <div className="flex items-center gap-2">
            <Icon size={14} style={{ color: 'var(--color-accent)' }} />
            <h2 className="font-semibold text-sm">{title}</h2>
          </div>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>

        <div className="p-6 space-y-4">
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            Enter your wallet password to reveal the {title.toLowerCase()}.
          </p>

          {!value && (
            <>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && mutation.mutate()}
                className="w-full rounded px-3 py-2 text-sm"
                placeholder="Wallet password..."
              />
              {error && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{error}</p>}
              <button
                onClick={() => mutation.mutate()}
                disabled={!password || mutation.isPending}
                className="w-full py-2 rounded text-sm font-medium disabled:opacity-50"
                style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
              >
                {mutation.isPending ? 'Decrypting...' : 'Reveal'}
              </button>
            </>
          )}

          {value && (
            <>
              <div
                className="rounded p-3 text-xs font-mono leading-relaxed break-all border"
                style={{
                  backgroundColor: 'rgba(255,68,68,0.06)',
                  borderColor: 'rgba(255,68,68,0.3)',
                  color: 'var(--color-text)',
                }}
              >
                {value}
              </div>
              <div className="flex gap-2">
                <button
                  onClick={copy}
                  className="flex-1 py-2 rounded text-sm font-medium border flex items-center justify-center gap-2"
                  style={{ borderColor: 'var(--color-border)', color: copied ? 'var(--color-accent)' : 'var(--color-text)' }}
                >
                  <Copy size={13} />
                  {copied ? 'Copied!' : 'Copy'}
                </button>
                <button
                  onClick={onClose}
                  className="flex-1 py-2 rounded text-sm font-medium"
                  style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
                >
                  Done
                </button>
              </div>
              <p className="text-xs" style={{ color: 'var(--color-danger)' }}>
                ⚠ Never share this. Clear your clipboard after copying.
              </p>
            </>
          )}
        </div>
      </div>
    </div>
  )
}

// ── Wallet Row ────────────────────────────────────────────────────────────

interface WalletRowProps {
  wallet: WalletEntry
}

function WalletRow({ wallet }: WalletRowProps) {
  const [revealed, setRevealed] = useState(false)
  const [copied, setCopied] = useState(false)
  const [exportModal, setExportModal] = useState<'mnemonic' | 'private_key' | null>(null)

  const display = revealed ? wallet.address : maskAddress(wallet.address)

  function copy() {
    navigator.clipboard.writeText(wallet.address)
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }

  return (
    <>
      <div
        className="flex items-center justify-between px-4 py-3 border-b last:border-0"
        style={{ borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center gap-3 min-w-0">
          <ChainBadge chain={wallet.chain} />
          <div className="min-w-0">
            {wallet.label && (
              <p className="text-xs font-medium" style={{ color: 'var(--color-accent)' }}>
                {wallet.label}
              </p>
            )}
            <span
              className="font-mono text-sm truncate block"
              style={{ color: 'var(--color-text)' }}
            >
              {display}
            </span>
          </div>
        </div>
        <div className="flex items-center gap-1 ml-4 flex-shrink-0">
          <button
            onClick={() => setRevealed((r) => !r)}
            className="p-1.5 rounded hover:bg-white/5 transition-colors"
            style={{ color: 'var(--color-text-muted)' }}
            title={revealed ? 'Hide' : 'Reveal address'}
          >
            {revealed ? <EyeOff size={13} /> : <Eye size={13} />}
          </button>
          <button
            onClick={copy}
            className="p-1.5 rounded hover:bg-white/5 transition-colors"
            style={{ color: copied ? 'var(--color-accent)' : 'var(--color-text-muted)' }}
            title="Copy address"
          >
            <Copy size={13} />
          </button>
          <button
            onClick={() => setExportModal('mnemonic')}
            className="p-1.5 rounded hover:bg-white/5 transition-colors"
            style={{ color: 'var(--color-text-muted)' }}
            title="View recovery phrase"
          >
            <BookOpen size={13} />
          </button>
          <button
            onClick={() => setExportModal('private_key')}
            className="p-1.5 rounded hover:bg-white/5 transition-colors"
            style={{ color: 'var(--color-text-muted)' }}
            title="View private key"
          >
            <Key size={13} />
          </button>
        </div>
      </div>

      {exportModal && (
        <ExportModal
          wallet={wallet}
          exportType={exportModal}
          onClose={() => setExportModal(null)}
        />
      )}
    </>
  )
}

// ── Main Page ─────────────────────────────────────────────────────────────

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
