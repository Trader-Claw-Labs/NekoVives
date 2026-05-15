import { useState, useEffect } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { Copy, Eye, EyeOff, Plus, X, Wallet, Key, BookOpen, Tag, ArrowRightLeft, Loader2, CheckCircle, AlertCircle, Server, ChevronDown, ChevronUp, Save, Send, TriangleAlert } from 'lucide-react'
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

// ── Transfer Modal ────────────────────────────────────────────────────────

interface TransferModalProps {
  wallet: WalletEntry
  onClose: () => void
}

function TransferModal({ wallet, onClose }: TransferModalProps) {
  const [toAddress, setToAddress] = useState('')
  const [amount, setAmount] = useState('')
  const [password, setPassword] = useState('')
  const [showPw, setShowPw] = useState(false)
  const [error, setError] = useState('')
  const [txHash, setTxHash] = useState('')

  const mutation = useMutation({
    mutationFn: () =>
      apiPost<{ tx_hash: string }>('/api/wallets/transfer', {
        from_address: wallet.address,
        to_address: toAddress,
        amount: parseFloat(amount),
        chain: wallet.chain,
        password,
      }),
    onSuccess: (data) => setTxHash(data.tx_hash),
    onError: (e: Error) => setError(e.message),
  })

  const chainSymbol = wallet.chain === 'evm' ? 'ETH' : wallet.chain === 'solana' ? 'SOL' : 'TON'

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div className="rounded-lg border w-full max-w-md"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
        <div className="flex items-center justify-between px-6 py-4 border-b" style={{ borderColor: 'var(--color-border)' }}>
          <div className="flex items-center gap-2">
            <Send size={14} style={{ color: 'var(--color-accent)' }} />
            <h2 className="font-semibold text-sm">Send {chainSymbol}</h2>
          </div>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}><X size={16} /></button>
        </div>

        <div className="p-6 space-y-4">
          {txHash ? (
            <div className="space-y-3">
              <div className="flex items-center gap-2 text-sm" style={{ color: 'var(--color-accent)' }}>
                <CheckCircle size={15} />
                <span className="font-semibold">Transfer submitted!</span>
              </div>
              <div className="rounded p-3 text-xs font-mono break-all"
                style={{ backgroundColor: 'var(--color-base)', border: '1px solid var(--color-border)', color: 'var(--color-text-muted)' }}>
                TX: {txHash}
              </div>
              <button onClick={onClose} className="w-full py-2 rounded text-sm font-medium"
                style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
                Done
              </button>
            </div>
          ) : (
            <>
              <div className="rounded px-3 py-2.5 text-xs flex items-start gap-2"
                style={{ backgroundColor: 'rgba(245,158,11,0.1)', border: '1px solid rgba(245,158,11,0.3)', color: '#f59e0b' }}>
                <TriangleAlert size={12} className="mt-0.5 flex-shrink-0" />
                <span>Transfers are irreversible. Verify the recipient address carefully before sending.</span>
              </div>

              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>From</label>
                <div className="rounded px-3 py-2 text-xs font-mono truncate"
                  style={{ backgroundColor: 'var(--color-base)', border: '1px solid var(--color-border)', color: 'var(--color-text-muted)' }}>
                  {wallet.address}
                </div>
              </div>

              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>To Address</label>
                <input className="w-full rounded px-3 py-2 text-sm font-mono"
                  value={toAddress} onChange={e => setToAddress(e.target.value)}
                  placeholder={wallet.chain === 'solana' ? 'Solana address...' : wallet.chain === 'ton' ? 'TON address...' : '0x...'} />
              </div>

              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Amount ({chainSymbol})</label>
                <input type="number" className="w-full rounded px-3 py-2 text-sm"
                  value={amount} onChange={e => setAmount(e.target.value)}
                  placeholder="0.01" min="0" step="any" />
              </div>

              <div>
                <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Wallet Password</label>
                <div className="relative">
                  <input type={showPw ? 'text' : 'password'} className="w-full rounded px-3 py-2 text-sm pr-10"
                    value={password} onChange={e => setPassword(e.target.value)}
                    placeholder="Wallet encryption password..." />
                  <button type="button" className="absolute right-2 top-1/2 -translate-y-1/2"
                    onClick={() => setShowPw(s => !s)} style={{ color: 'var(--color-text-muted)' }}>
                    {showPw ? <EyeOff size={14} /> : <Eye size={14} />}
                  </button>
                </div>
              </div>

              {error && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{error}</p>}

              <div className="flex gap-2 pt-1">
                <button
                  onClick={() => { setError(''); mutation.mutate() }}
                  disabled={!toAddress || !amount || !password || mutation.isPending}
                  className="flex-1 py-2 rounded text-sm font-medium disabled:opacity-50 flex items-center justify-center gap-2"
                  style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
                  {mutation.isPending ? <Loader2 size={13} className="animate-spin" /> : <Send size={13} />}
                  {mutation.isPending ? 'Sending...' : 'Send'}
                </button>
                <button onClick={onClose} className="px-4 py-2 rounded text-sm border hover:bg-white/5"
                  style={{ borderColor: 'var(--color-border)' }}>Cancel</button>
              </div>
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
  const [showTransfer, setShowTransfer] = useState(false)

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
          <button
            onClick={() => setShowTransfer(true)}
            className="p-1.5 rounded hover:bg-white/5 transition-colors"
            style={{ color: 'var(--color-text-muted)' }}
            title="Send / Transfer"
          >
            <Send size={13} />
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

      {showTransfer && (
        <TransferModal
          wallet={wallet}
          onClose={() => setShowTransfer(false)}
        />
      )}
    </>
  )
}

// ── Swap Section ──────────────────────────────────────────────────────────

const EVM_SUBNETS = [
  { key: 'ethereum',  label: 'Ethereum',  symbol: 'ETH' },
  { key: 'arbitrum',  label: 'Arbitrum',  symbol: 'ETH' },
  { key: 'optimism',  label: 'Optimism',  symbol: 'ETH' },
  { key: 'base',      label: 'Base',      symbol: 'ETH' },
  { key: 'polygon',   label: 'Polygon',   symbol: 'MATIC' },
  { key: 'bnb',       label: 'BNB Chain', symbol: 'BNB' },
  { key: 'unichain',  label: 'Unichain',  symbol: 'ETH' },
]

const CUSTOM_NET_KEY = '__custom__'

interface CustomNetwork {
  name: string
  chainId: string
  rpc: string
  symbol: string
  explorer: string
}

const EMPTY_CUSTOM: CustomNetwork = { name: '', chainId: '', rpc: '', symbol: '', explorer: '' }

// Persist custom networks in localStorage
const LS_CUSTOM_NETS = 'trader-claw:custom-networks'

function loadCustomNets(): CustomNetwork[] {
  try { return JSON.parse(localStorage.getItem(LS_CUSTOM_NETS) ?? '[]') } catch { return [] }
}

function saveCustomNets(nets: CustomNetwork[]) {
  localStorage.setItem(LS_CUSTOM_NETS, JSON.stringify(nets))
}

interface SwapQuote {
  in_amount: number
  out_amount: number
  out_amount_min: number
  price_impact_pct: number
  route: string
  fee_usd?: number
}

interface BalanceEntry {
  symbol: string
  amount: string
  contract?: string
}

interface BalanceResponse {
  address?: string
  chain?: string
  native?: BalanceEntry
  tokens?: BalanceEntry[]
}

// ── RPC Endpoints Section ─────────────────────────────────────────────────

function RpcSection() {
  const [open, setOpen] = useState(false)
  const [rpcValues, setRpcValues] = useState<Record<string, string>>({})
  const [saveMsg, setSaveMsg] = useState('')
  const [saveErr, setSaveErr] = useState('')

  const { data: configData } = useQuery<{ content?: string }>({
    queryKey: ['config'],
    queryFn: () => apiFetch('/api/config'),
  })

  useEffect(() => {
    if (!configData?.content) return
    const content = configData.content
    const parsed: Record<string, string> = {}
    for (const subnet of EVM_SUBNETS) {
      const m = content.match(new RegExp(`${subnet.key}\\s*=\\s*"([^"]*)"`, 'i'))
      if (m) parsed[subnet.key] = m[1]
    }
    // also solana
    const solM = content.match(/solana\s*=\s*"([^"]*)"/i)
    if (solM) parsed['solana'] = solM[1]
    setRpcValues(parsed)
  }, [configData])

  const saveMutation = useMutation({
    mutationFn: async () => {
      if (!configData?.content) throw new Error('Config not loaded')
      let toml = configData.content

      // Ensure [chains_rpc] section exists
      if (!/\[chains_rpc\]/.test(toml)) {
        toml += '\n[chains_rpc]\n'
      }

      const allKeys = [...EVM_SUBNETS.map(s => s.key), 'solana']
      for (const key of allKeys) {
        const val = rpcValues[key] ?? ''
        const pat = new RegExp(`(\\[chains_rpc\\][\\s\\S]*?)${key}\\s*=\\s*"[^"]*"`)
        if (pat.test(toml)) {
          if (val) {
            toml = toml.replace(new RegExp(`${key}\\s*=\\s*"[^"]*"`), `${key} = "${val}"`)
          } else {
            toml = toml.replace(new RegExp(`${key}\\s*=\\s*"[^"]*"\\n?`), '')
          }
        } else if (val) {
          // Append under [chains_rpc]
          toml = toml.replace(/(\[chains_rpc\])/, `$1\n${key} = "${val}"`)
        }
      }

      const res = await fetch('/api/config', {
        method: 'PUT',
        headers: { 'Content-Type': 'text/plain', Authorization: `Bearer ${localStorage.getItem('auth_token') ?? ''}` },
        body: toml,
      })
      if (!res.ok) throw new Error(await res.text())
    },
    onSuccess: () => { setSaveMsg('Saved!'); setSaveErr(''); setTimeout(() => setSaveMsg(''), 3000) },
    onError: (e: Error) => { setSaveErr(e.message); setSaveMsg('') },
  })

  return (
    <div className="rounded-lg border mt-4"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
      <button
        className="flex items-center justify-between w-full px-4 py-3"
        onClick={() => setOpen(o => !o)}
      >
        <div className="flex items-center gap-2">
          <Server size={14} style={{ color: 'var(--color-accent)' }} />
          <span className="text-sm font-semibold">Premium RPC Endpoints</span>
          <span className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            (optional — overrides default public nodes)
          </span>
        </div>
        {open ? <ChevronUp size={14} style={{ color: 'var(--color-text-muted)' }} />
               : <ChevronDown size={14} style={{ color: 'var(--color-text-muted)' }} />}
      </button>

      {open && (
        <div className="px-4 pb-4 space-y-2 border-t" style={{ borderColor: 'var(--color-border)' }}>
          <p className="text-xs pt-3 pb-1" style={{ color: 'var(--color-text-muted)' }}>
            Enter your Alchemy, Infura, QuickNode, or other RPC URLs. Leave blank to use defaults.
          </p>
          {[...EVM_SUBNETS, { key: 'solana', label: 'Solana', symbol: 'SOL' }].map(subnet => (
            <div key={subnet.key} className="flex items-center gap-3">
              <span className="text-xs w-20 flex-shrink-0 font-medium" style={{ color: 'var(--color-text-muted)' }}>
                {subnet.label}
              </span>
              <input
                className="flex-1 rounded px-3 py-1.5 text-xs font-mono"
                value={rpcValues[subnet.key] ?? ''}
                onChange={e => setRpcValues(v => ({ ...v, [subnet.key]: e.target.value }))}
                placeholder={`https://... (${subnet.key} RPC)`}
              />
            </div>
          ))}
          {saveErr && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{saveErr}</p>}
          {saveMsg && <p className="text-xs" style={{ color: 'var(--color-accent)' }}>{saveMsg}</p>}
          <button
            onClick={() => saveMutation.mutate()}
            disabled={saveMutation.isPending}
            className="flex items-center gap-2 px-4 py-1.5 rounded text-sm font-medium disabled:opacity-50"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
            <Save size={12} />
            {saveMutation.isPending ? 'Saving…' : 'Save RPC Config'}
          </button>
        </div>
      )}
    </div>
  )
}

// ── Balance Panel ─────────────────────────────────────────────────────────

function BalancePanel({ chain, subnet, address }: { chain: Chain; subnet: string; address: string }) {
  const { data, isLoading } = useQuery<BalanceResponse>({
    queryKey: ['balance', address, chain, subnet],
    queryFn: () => apiFetch(`/api/wallets/${encodeURIComponent(address)}/balance?chain=${chain}&subnet=${subnet}`),
    enabled: !!address,
    staleTime: 30_000,
  })

  if (!address) return null

  return (
    <div className="rounded p-3 text-xs space-y-1.5"
      style={{ backgroundColor: 'var(--color-base)', border: '1px solid var(--color-border)' }}>
      <div className="flex items-center justify-between mb-1">
        <span className="font-semibold" style={{ color: 'var(--color-text-muted)' }}>Wallet Balance</span>
        {isLoading && <Loader2 size={11} className="animate-spin" style={{ color: 'var(--color-text-muted)' }} />}
      </div>
      {data?.native && (
        <div className="flex justify-between">
          <span style={{ color: 'var(--color-text-muted)' }}>{data.native.symbol}</span>
          <span className="font-mono font-semibold" style={{ color: 'var(--color-accent)' }}>{data.native.amount}</span>
        </div>
      )}
      {(data?.tokens ?? []).map((t, i) => (
        <div key={i} className="flex justify-between">
          <span style={{ color: 'var(--color-text-muted)' }}>{t.symbol}</span>
          <span className="font-mono">{t.amount}</span>
        </div>
      ))}
      {!isLoading && !data?.native && (
        <p style={{ color: 'var(--color-text-muted)' }}>No balance data</p>
      )}
    </div>
  )
}

function SwapSection({ chain, wallets }: { chain: Chain; wallets: WalletEntry[] }) {
  const evmWallets = wallets.filter(w => w.chain === chain)
  const [subnet, setSubnet] = useState('ethereum')
  const [customNets, setCustomNets] = useState<CustomNetwork[]>(loadCustomNets)
  const [showCustomForm, setShowCustomForm] = useState(false)
  const [editingNet, setEditingNet] = useState<CustomNetwork>(EMPTY_CUSTOM)
  const [customFormErr, setCustomFormErr] = useState('')
  const [fromToken, setFromToken] = useState('')
  const [toToken, setToToken] = useState('')
  const [amount, setAmount] = useState('')
  const [walletAddr, setWalletAddr] = useState(evmWallets[0]?.address ?? '')
  const [quote, setQuote] = useState<SwapQuote | null>(null)
  const [swapResult, setSwapResult] = useState<{ status: string; tx_hash?: string; error?: string } | null>(null)

  // When chain tab changes reset wallet addr
  useEffect(() => {
    const match = wallets.filter(w => w.chain === chain)
    setWalletAddr(match[0]?.address ?? '')
    setQuote(null)
    setSwapResult(null)
  }, [chain, wallets])

  function saveCustomNet() {
    if (!editingNet.name.trim()) { setCustomFormErr('Name is required'); return }
    if (!editingNet.chainId.trim() || isNaN(Number(editingNet.chainId))) { setCustomFormErr('Valid Chain ID is required'); return }
    if (!editingNet.rpc.trim() || !editingNet.rpc.startsWith('http')) { setCustomFormErr('Valid RPC URL is required'); return }
    if (!editingNet.symbol.trim()) { setCustomFormErr('Native currency symbol is required'); return }
    const key = `custom_${editingNet.chainId}`
    const updated = [...customNets.filter(n => `custom_${n.chainId}` !== key), editingNet]
    setCustomNets(updated)
    saveCustomNets(updated)
    setSubnet(key)
    setShowCustomForm(false)
    setEditingNet(EMPTY_CUSTOM)
    setCustomFormErr('')
  }

  function removeCustomNet(chainId: string) {
    const updated = customNets.filter(n => n.chainId !== chainId)
    setCustomNets(updated)
    saveCustomNets(updated)
    if (subnet === `custom_${chainId}`) setSubnet('ethereum')
  }

  const activeCustomNet = customNets.find(n => `custom_${n.chainId}` === subnet)

  const quoteMutation = useMutation({
    mutationFn: () => apiPost<{ quote: SwapQuote }>('/api/wallets/quote', {
      chain,
      subnet: chain === 'evm' ? subnet : undefined,
      from_token: fromToken,
      to_token: toToken,
      amount: parseFloat(amount),
    }),
    onSuccess: (data) => setQuote(data.quote),
    onError: () => setQuote(null),
  })

  const swapMutation = useMutation({
    mutationFn: () => apiPost<{ status: string; tx_hash?: string; error?: string }>('/api/wallets/swap', {
      chain,
      subnet: chain === 'evm' ? subnet : undefined,
      from_token: fromToken,
      to_token: toToken,
      amount: parseFloat(amount),
      wallet_address: walletAddr,
    }),
    onSuccess: (data) => setSwapResult(data),
    onError: (e: Error) => setSwapResult({ status: 'error', error: e.message }),
  })

  const canQuote = !!fromToken && !!toToken && !!amount && parseFloat(amount) > 0
  const dex = chain === 'evm' ? 'Uniswap V3' : chain === 'solana' ? 'Jupiter' : 'STON.fi'

  return (
    <div className="rounded-lg border mt-6"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
      <div className="flex items-center gap-2 px-4 py-3 border-b" style={{ borderColor: 'var(--color-border)' }}>
        <ArrowRightLeft size={14} style={{ color: 'var(--color-accent)' }} />
        <h2 className="text-sm font-semibold">Swap / Trade</h2>
        <span className="text-xs px-1.5 py-0.5 rounded"
          style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)' }}>
          {dex}
        </span>
      </div>

      <div className="p-4 space-y-3">
        {/* EVM subnet selector */}
        {chain === 'evm' && (
          <div>
            <label className="text-xs block mb-1.5" style={{ color: 'var(--color-text-muted)' }}>Network / Subnet</label>
            <div className="flex flex-wrap gap-1.5 items-center">
              {EVM_SUBNETS.map(s => (
                <button key={s.key}
                  onClick={() => setSubnet(s.key)}
                  className={clsx('px-2.5 py-1 rounded text-xs font-medium border transition-colors',
                    subnet === s.key ? 'text-black' : 'hover:border-white/30')}
                  style={subnet === s.key
                    ? { backgroundColor: 'var(--color-accent)', borderColor: 'var(--color-accent)', color: '#000' }
                    : { borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
                  {s.label}
                </button>
              ))}

              {/* Saved custom networks */}
              {customNets.map(n => {
                const key = `custom_${n.chainId}`
                return (
                  <div key={key} className="flex items-center gap-0 rounded border overflow-hidden"
                    style={subnet === key
                      ? { borderColor: 'var(--color-accent)' }
                      : { borderColor: 'var(--color-border)' }}>
                    <button
                      onClick={() => setSubnet(key)}
                      className="px-2.5 py-1 text-xs font-medium transition-colors"
                      style={subnet === key
                        ? { backgroundColor: 'var(--color-accent)', color: '#000' }
                        : { color: 'var(--color-text-muted)' }}>
                      {n.name} <span className="opacity-60">(#{n.chainId})</span>
                    </button>
                    <button
                      onClick={() => removeCustomNet(n.chainId)}
                      className="px-1.5 py-1 hover:bg-white/10 transition-colors"
                      style={{ color: subnet === key ? '#000' : 'var(--color-text-muted)' }}
                      title="Remove network">
                      <X size={10} />
                    </button>
                  </div>
                )
              })}

              {/* Add custom network button */}
              <button
                onClick={() => { setShowCustomForm(f => !f); setEditingNet(EMPTY_CUSTOM); setCustomFormErr('') }}
                className="px-2.5 py-1 rounded text-xs font-medium border flex items-center gap-1 transition-colors hover:border-white/30"
                style={subnet === CUSTOM_NET_KEY
                  ? { backgroundColor: 'var(--color-accent)', borderColor: 'var(--color-accent)', color: '#000' }
                  : { borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
                <Plus size={11} />
                Custom
              </button>
            </div>

            {/* Custom network form */}
            {showCustomForm && (
              <div className="mt-3 rounded-lg border p-4 space-y-3"
                style={{ backgroundColor: 'var(--color-base)', borderColor: 'var(--color-border)' }}>
                <div className="flex items-center justify-between mb-1">
                  <span className="text-xs font-semibold" style={{ color: 'var(--color-text)' }}>Add Custom Network</span>
                  <button onClick={() => setShowCustomForm(false)} style={{ color: 'var(--color-text-muted)' }}>
                    <X size={13} />
                  </button>
                </div>

                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Network Name *</label>
                    <input className="w-full rounded px-3 py-1.5 text-xs"
                      value={editingNet.name}
                      onChange={e => setEditingNet(v => ({ ...v, name: e.target.value }))}
                      placeholder="e.g. Avalanche C-Chain" />
                  </div>
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Chain ID *</label>
                    <input className="w-full rounded px-3 py-1.5 text-xs font-mono"
                      value={editingNet.chainId}
                      onChange={e => setEditingNet(v => ({ ...v, chainId: e.target.value }))}
                      placeholder="43114" type="number" min="1" />
                  </div>
                </div>

                <div>
                  <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>RPC URL *</label>
                  <input className="w-full rounded px-3 py-1.5 text-xs font-mono"
                    value={editingNet.rpc}
                    onChange={e => setEditingNet(v => ({ ...v, rpc: e.target.value }))}
                    placeholder="https://api.avax.network/ext/bc/C/rpc" />
                </div>

                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Native Symbol *</label>
                    <input className="w-full rounded px-3 py-1.5 text-xs font-mono"
                      value={editingNet.symbol}
                      onChange={e => setEditingNet(v => ({ ...v, symbol: e.target.value.toUpperCase() }))}
                      placeholder="AVAX" />
                  </div>
                  <div>
                    <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Block Explorer</label>
                    <input className="w-full rounded px-3 py-1.5 text-xs font-mono"
                      value={editingNet.explorer}
                      onChange={e => setEditingNet(v => ({ ...v, explorer: e.target.value }))}
                      placeholder="https://snowtrace.io" />
                  </div>
                </div>

                {customFormErr && (
                  <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{customFormErr}</p>
                )}

                <div className="flex gap-2 pt-1">
                  <button
                    onClick={saveCustomNet}
                    className="flex items-center gap-2 px-4 py-1.5 rounded text-xs font-medium"
                    style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
                    <CheckCircle size={12} />
                    Add Network
                  </button>
                  <button
                    onClick={() => { setShowCustomForm(false); setEditingNet(EMPTY_CUSTOM); setCustomFormErr('') }}
                    className="px-4 py-1.5 rounded text-xs font-medium border"
                    style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
                    Cancel
                  </button>
                </div>
              </div>
            )}

            {/* Active custom network info badge */}
            {activeCustomNet && (
              <div className="mt-2 flex flex-wrap gap-3 text-xs px-3 py-2 rounded"
                style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)' }}>
                <span>Chain ID: <span className="font-mono" style={{ color: 'var(--color-text)' }}>{activeCustomNet.chainId}</span></span>
                <span>Symbol: <span className="font-mono" style={{ color: 'var(--color-text)' }}>{activeCustomNet.symbol}</span></span>
                <span className="truncate max-w-xs">RPC: <span className="font-mono" style={{ color: 'var(--color-text)' }}>{activeCustomNet.rpc}</span></span>
                {activeCustomNet.explorer && (
                  <a href={activeCustomNet.explorer} target="_blank" rel="noopener noreferrer"
                    style={{ color: 'var(--color-accent)' }} className="underline">Explorer ↗</a>
                )}
              </div>
            )}
          </div>
        )}

        {/* Wallet selector */}
        <div>
          <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Wallet</label>
          {evmWallets.length > 0 ? (
            <select
              value={walletAddr}
              onChange={e => setWalletAddr(e.target.value)}
              className="w-full rounded px-3 py-2 text-xs font-mono">
              {evmWallets.map(w => (
                <option key={w.address} value={w.address}>
                  {w.label ? `${w.label} — ` : ''}{w.address.slice(0, 10)}…{w.address.slice(-6)}
                </option>
              ))}
            </select>
          ) : (
            <input className="w-full rounded px-3 py-2 text-xs font-mono"
              value={walletAddr} onChange={e => setWalletAddr(e.target.value)}
              placeholder="Paste wallet address…" />
          )}
        </div>

        {/* Balance panel */}
        {walletAddr && (
          <BalancePanel chain={chain} subnet={subnet} address={walletAddr} />
        )}

        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
              From Token
            </label>
            <input className="w-full rounded px-3 py-2 text-sm font-mono" value={fromToken}
              onChange={e => setFromToken(e.target.value)}
              placeholder={chain === 'solana' ? 'SOL' : chain === 'evm' ? 'ETH' : 'TON'} />
          </div>
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>To Token</label>
            <input className="w-full rounded px-3 py-2 text-sm font-mono" value={toToken}
              onChange={e => setToToken(e.target.value)}
              placeholder={chain === 'solana' ? 'USDC' : chain === 'evm' ? 'USDC' : 'USDT'} />
          </div>
        </div>

        <div>
          <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>Amount</label>
          <input type="number" className="w-full rounded px-3 py-2 text-sm" value={amount}
            onChange={e => setAmount(e.target.value)} placeholder="0.1" min="0" step="any" />
        </div>

        {/* Quote result */}
        {quote && (
          <div className="rounded p-3 text-xs space-y-1"
            style={{ backgroundColor: 'var(--color-base)', border: '1px solid var(--color-border)' }}>
            <div className="flex justify-between">
              <span style={{ color: 'var(--color-text-muted)' }}>You receive (est.)</span>
              <span className="font-semibold" style={{ color: 'var(--color-accent)' }}>
                {quote.out_amount.toFixed(6)} {toToken}
              </span>
            </div>
            <div className="flex justify-between">
              <span style={{ color: 'var(--color-text-muted)' }}>Min received</span>
              <span>{quote.out_amount_min.toFixed(6)}</span>
            </div>
            <div className="flex justify-between">
              <span style={{ color: 'var(--color-text-muted)' }}>Price impact</span>
              <span style={{ color: quote.price_impact_pct > 1 ? 'var(--color-danger)' : 'var(--color-text)' }}>
                {quote.price_impact_pct.toFixed(2)}%
              </span>
            </div>
            {quote.route && (
              <div className="flex justify-between">
                <span style={{ color: 'var(--color-text-muted)' }}>Route</span>
                <span className="font-mono">{quote.route}</span>
              </div>
            )}
          </div>
        )}

        {/* Swap result */}
        {swapResult && (
          <div className="rounded px-3 py-2 text-xs flex items-center gap-2"
            style={{
              backgroundColor: swapResult.status === 'ok' ? 'rgba(74,222,128,0.1)' : 'rgba(239,68,68,0.1)',
              color: swapResult.status === 'ok' ? 'var(--color-accent)' : 'var(--color-danger)',
              border: `1px solid ${swapResult.status === 'ok' ? 'rgba(74,222,128,0.3)' : 'rgba(239,68,68,0.3)'}`,
            }}>
            {swapResult.status === 'ok'
              ? <><CheckCircle size={12} /> Swap submitted! TX: {swapResult.tx_hash}</>
              : <><AlertCircle size={12} /> {swapResult.error}</>}
          </div>
        )}

        <div className="flex gap-2">
          <button onClick={() => { setQuote(null); setSwapResult(null); quoteMutation.mutate() }}
            disabled={!canQuote || quoteMutation.isPending}
            className="flex-1 py-2 rounded text-sm font-medium border disabled:opacity-50 hover:bg-white/5 flex items-center justify-center gap-2"
            style={{ borderColor: 'var(--color-border)' }}>
            {quoteMutation.isPending ? <Loader2 size={13} className="animate-spin" /> : null}
            Get Quote
          </button>
          <button onClick={() => swapMutation.mutate()}
            disabled={!quote || !walletAddr || swapMutation.isPending}
            className="flex-1 py-2 rounded text-sm font-medium disabled:opacity-50 flex items-center justify-center gap-2"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}>
            {swapMutation.isPending ? <Loader2 size={13} className="animate-spin" /> : <ArrowRightLeft size={13} />}
            Execute Swap
          </button>
        </div>

        <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
          Swaps are executed on-chain. Double-check the token addresses and amounts before confirming.
        </p>
      </div>
    </div>
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

      {/* Warning banner */}
      <div className="flex items-start gap-3 rounded-lg px-4 py-3 mb-5"
        style={{ backgroundColor: 'rgba(245,158,11,0.08)', border: '1px solid rgba(245,158,11,0.25)' }}>
        <TriangleAlert size={15} className="flex-shrink-0 mt-0.5" style={{ color: '#f59e0b' }} />
        <p className="text-xs leading-relaxed" style={{ color: '#f59e0b' }}>
          <span className="font-semibold">Trading wallets only.</span> These wallets are designed for active
          trading operations (swaps, Polymarket, DeFi). Do not use them as long-term vaults for storing
          significant funds. Always keep large holdings in a hardware wallet or dedicated cold storage.
        </p>
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

      <SwapSection chain={activeTab} wallets={wallets} />
      <RpcSection />

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
