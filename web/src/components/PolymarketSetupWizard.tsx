import { useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { apiPost } from '../hooks/useApi'
import { CheckCircle, AlertCircle, Loader2, KeyRound, Eye, EyeOff, ArrowRight, ArrowLeft, Check } from 'lucide-react'

interface VerifyWalletResp {
  eoa_address?: string
  account_type?: string
  is_smart_account?: boolean
  suggested_signature_type?: string
  error?: string
}

interface DetectProxyResp {
  proxy_address?: string
  proxy_type?: string
  suggested_signature_type?: string
  owner_embedded?: string
  owner_matches_eoa?: boolean
  is_contract?: boolean
  error?: string
}

interface GenerateCredsResp {
  success?: boolean
  method_used?: string
  api_key?: string
  api_key_masked?: string
  secret_masked?: string
  passphrase_masked?: string
  wallet_address?: string
  persisted?: boolean
  error?: string
}

type Step = 1 | 2 | 3 | 4

interface Props {
  onComplete?: () => void
  onCancel?: () => void
}

const ACCOUNT_TYPE_LABELS: Record<string, string> = {
  eoa: 'Externally Owned Account (EOA)',
  eip7702_smart_account: 'EIP-7702 Smart Account (MetaMask Smart Account / Coinbase Smart Wallet)',
  eip1167_proxy: 'EIP-1167 Minimal Proxy (Magic / email login)',
  polymarket_custom_proxy: 'Polymarket Custom Proxy',
  contract: 'Smart Contract Wallet',
}

export default function PolymarketSetupWizard({ onComplete, onCancel }: Props) {
  const [step, setStep] = useState<Step>(1)
  const [privateKey, setPrivateKey] = useState('')
  const [showPK, setShowPK] = useState(false)
  const [eoaAddress, setEoaAddress] = useState('')
  const [accountType, setAccountType] = useState('')
  const [proxyAddress, setProxyAddress] = useState('')
  const [proxyType, setProxyType] = useState('')
  const [signatureType, setSignatureType] = useState('')
  const [credsMode, setCredsMode] = useState<'auto' | 'create' | 'derive'>('auto')
  const [isBuilder, setIsBuilder] = useState(false)
  const [generatedCreds, setGeneratedCreds] = useState<GenerateCredsResp | null>(null)

  // ── Step 1: verify wallet ──────────────────────────────────────────────
  const verifyMutation = useMutation({
    mutationFn: () =>
      apiPost<VerifyWalletResp>('/api/polymarket/setup/verify-wallet', { private_key: privateKey.trim() }),
    onSuccess: (data) => {
      if (data.error) return
      setEoaAddress(data.eoa_address || '')
      setAccountType(data.account_type || '')
      // Default signature_type from server suggestion (may be overridden in step 2)
      setSignatureType(data.suggested_signature_type || '')
      setStep(2)
    },
  })

  // ── Step 2: detect proxy ──────────────────────────────────────────────
  const detectMutation = useMutation({
    mutationFn: () =>
      apiPost<DetectProxyResp>('/api/polymarket/setup/detect-proxy', {
        eoa_address: eoaAddress,
        proxy_address: proxyAddress.trim(),
      }),
    onSuccess: (data) => {
      if (data.error) return
      setProxyType(data.proxy_type || '')
      setSignatureType(data.suggested_signature_type || signatureType)
      setStep(3)
    },
  })

  // ── Step 3: generate creds ────────────────────────────────────────────
  const generateMutation = useMutation({
    mutationFn: () =>
      apiPost<GenerateCredsResp>('/api/polymarket/setup/generate-creds', {
        private_key: privateKey.trim(),
        wallet_address: eoaAddress,
        proxy_address: proxyAddress.trim() || undefined,
        signature_type: signatureType || undefined,
        is_builder: isBuilder,
        mode: credsMode,
        persist: true,
      }),
    onSuccess: (data) => {
      if (data.success) {
        setGeneratedCreds(data)
        setStep(4)
      }
    },
  })

  const stepHeaders: Record<Step, { title: string; subtitle: string }> = {
    1: { title: 'Step 1: Verify your wallet', subtitle: 'Paste your wallet private key. We verify it on Polygon and detect the wallet type.' },
    2: { title: 'Step 2: Polymarket trading address', subtitle: 'Paste the Builder/Trading address from your Polymarket dashboard. This is where your USDC sits.' },
    3: { title: 'Step 3: Generate API credentials', subtitle: 'We sign an EIP-712 message with your key and Polymarket issues your trading credentials.' },
    4: { title: 'Step 4: Setup complete', subtitle: 'Your wallet is ready to trade. The credentials have been saved to config.toml.' },
  }

  const canProceedStep1 = privateKey.trim().length === 64 || privateKey.trim().length === 66
  const canProceedStep2 = proxyAddress.trim().length === 42 && proxyAddress.trim().toLowerCase().startsWith('0x')

  return (
    <div className="bg-neutral-900 border border-neutral-700 rounded-lg p-6 max-w-3xl mx-auto">
      {/* Stepper */}
      <div className="flex items-center justify-between mb-6">
        {[1, 2, 3, 4].map((n) => (
          <div key={n} className="flex items-center flex-1">
            <div
              className={`flex items-center justify-center w-8 h-8 rounded-full text-sm font-semibold ${
                step === n
                  ? 'bg-blue-500 text-white'
                  : step > n
                  ? 'bg-green-600 text-white'
                  : 'bg-neutral-700 text-neutral-400'
              }`}
            >
              {step > n ? <Check className="w-4 h-4" /> : n}
            </div>
            {n < 4 && (
              <div className={`flex-1 h-0.5 mx-2 ${step > n ? 'bg-green-600' : 'bg-neutral-700'}`} />
            )}
          </div>
        ))}
      </div>

      <h2 className="text-xl font-bold text-white mb-1">{stepHeaders[step].title}</h2>
      <p className="text-sm text-neutral-400 mb-6">{stepHeaders[step].subtitle}</p>

      {/* Step 1: Private key input */}
      {step === 1 && (
        <div className="space-y-4">
          <div>
            <label className="block text-sm text-neutral-300 mb-2">
              <KeyRound className="inline w-4 h-4 mr-1" />
              Private key (hex, 64 chars)
            </label>
            <div className="relative">
              <input
                type={showPK ? 'text' : 'password'}
                value={privateKey}
                onChange={(e) => setPrivateKey(e.target.value)}
                placeholder="0x... or just the 64 hex chars"
                className="w-full bg-neutral-800 border border-neutral-700 rounded px-3 py-2 pr-10 font-mono text-sm text-white"
                autoComplete="off"
              />
              <button
                type="button"
                onClick={() => setShowPK(!showPK)}
                className="absolute right-2 top-2 text-neutral-400 hover:text-white"
              >
                {showPK ? <EyeOff className="w-5 h-5" /> : <Eye className="w-5 h-5" />}
              </button>
            </div>
            <p className="text-xs text-neutral-500 mt-2">
              MetaMask → Account menu → Account details → Show private key. Never share or commit this key.
            </p>
          </div>

          {verifyMutation.data?.error && (
            <div className="text-red-400 text-sm flex items-start gap-2">
              <AlertCircle className="w-4 h-4 mt-0.5 flex-shrink-0" />
              <span>{verifyMutation.data.error}</span>
            </div>
          )}

          <div className="flex justify-between">
            <button
              type="button"
              onClick={onCancel}
              className="text-neutral-400 hover:text-white text-sm"
            >
              Cancel
            </button>
            <button
              type="button"
              disabled={!canProceedStep1 || verifyMutation.isPending}
              onClick={() => verifyMutation.mutate()}
              className="bg-blue-500 hover:bg-blue-600 disabled:bg-neutral-700 disabled:text-neutral-500 text-white px-4 py-2 rounded text-sm flex items-center gap-2"
            >
              {verifyMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : null}
              Verify wallet
              <ArrowRight className="w-4 h-4" />
            </button>
          </div>
        </div>
      )}

      {/* Step 2: Detect proxy */}
      {step === 2 && (
        <div className="space-y-4">
          <div className="bg-neutral-800 border border-neutral-700 rounded p-3 text-sm">
            <div className="text-neutral-400">EOA detected</div>
            <div className="font-mono text-white text-xs break-all">{eoaAddress}</div>
            <div className="text-neutral-400 mt-2">Type</div>
            <div className="text-white">{ACCOUNT_TYPE_LABELS[accountType] || accountType || 'Unknown'}</div>
          </div>

          <div>
            <label className="block text-sm text-neutral-300 mb-2">
              Polymarket Builder/Trading address
            </label>
            <input
              type="text"
              value={proxyAddress}
              onChange={(e) => setProxyAddress(e.target.value)}
              placeholder="0x..."
              className="w-full bg-neutral-800 border border-neutral-700 rounded px-3 py-2 font-mono text-sm text-white"
              autoComplete="off"
            />
            <p className="text-xs text-neutral-500 mt-2">
              Polymarket dashboard → Profile → Settings → Builder Address (or Trading Address). This is the
              proxy that holds your USDC.e or pUSD balance.
            </p>
          </div>

          {detectMutation.data?.error && (
            <div className="text-red-400 text-sm flex items-start gap-2">
              <AlertCircle className="w-4 h-4 mt-0.5 flex-shrink-0" />
              <span>{detectMutation.data.error}</span>
            </div>
          )}

          <div className="flex justify-between">
            <button
              type="button"
              onClick={() => setStep(1)}
              className="text-neutral-400 hover:text-white text-sm flex items-center gap-1"
            >
              <ArrowLeft className="w-4 h-4" /> Back
            </button>
            <button
              type="button"
              disabled={!canProceedStep2 || detectMutation.isPending}
              onClick={() => detectMutation.mutate()}
              className="bg-blue-500 hover:bg-blue-600 disabled:bg-neutral-700 disabled:text-neutral-500 text-white px-4 py-2 rounded text-sm flex items-center gap-2"
            >
              {detectMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : null}
              Detect type
              <ArrowRight className="w-4 h-4" />
            </button>
          </div>
        </div>
      )}

      {/* Step 3: Generate credentials */}
      {step === 3 && (
        <div className="space-y-4">
          <div className="bg-neutral-800 border border-neutral-700 rounded p-3 text-sm space-y-2">
            <div>
              <div className="text-neutral-400 text-xs">Signing EOA</div>
              <div className="font-mono text-white text-xs break-all">{eoaAddress}</div>
            </div>
            <div>
              <div className="text-neutral-400 text-xs">Polymarket proxy</div>
              <div className="font-mono text-white text-xs break-all">{proxyAddress}</div>
              <div className="text-neutral-500 text-xs mt-1">{proxyType}</div>
            </div>
            <div>
              <div className="text-neutral-400 text-xs">Detected signature type</div>
              <div className="text-white">{signatureType || '(auto)'}</div>
            </div>
          </div>

          <div>
            <label className="block text-sm text-neutral-300 mb-2">
              Signature type override (optional)
            </label>
            <select
              value={signatureType}
              onChange={(e) => setSignatureType(e.target.value)}
              className="w-full bg-neutral-800 border border-neutral-700 rounded px-3 py-2 text-sm text-white"
            >
              <option value="">Auto-detect</option>
              <option value="eoa">EOA (pure externally-owned account)</option>
              <option value="proxy">Proxy (EIP-1167 minimal proxy / Magic email)</option>
              <option value="gnosis_safe">Gnosis Safe v1.3.0</option>
              <option value="poly1271">Poly1271 / EIP-1271 smart contract wallet</option>
            </select>
          </div>

          <div>
            <label className="block text-sm text-neutral-300 mb-2">
              Credentials mode
            </label>
            <div className="space-y-2">
              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="radio"
                  checked={credsMode === 'auto'}
                  onChange={() => setCredsMode('auto')}
                  className="mt-1"
                />
                <div>
                  <div className="text-sm text-white">Auto (recommended)</div>
                  <div className="text-xs text-neutral-500">Tries to derive existing credentials first; falls back to creating new ones if none exist.</div>
                </div>
              </label>
              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="radio"
                  checked={credsMode === 'derive'}
                  onChange={() => setCredsMode('derive')}
                  className="mt-1"
                />
                <div>
                  <div className="text-sm text-white">Recover existing</div>
                  <div className="text-xs text-neutral-500">Use if you've created Polymarket API credentials in the past for this wallet (any client / dashboard).</div>
                </div>
              </label>
              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="radio"
                  checked={credsMode === 'create'}
                  onChange={() => setCredsMode('create')}
                  className="mt-1"
                />
                <div>
                  <div className="text-sm text-white">Create fresh</div>
                  <div className="text-xs text-neutral-500">First time setup — only works if no credentials exist yet for this wallet.</div>
                </div>
              </label>
            </div>
          </div>

          <div>
            <label className="flex items-start gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={isBuilder}
                onChange={(e) => setIsBuilder(e.target.checked)}
                className="mt-1"
              />
              <div>
                <div className="text-sm text-white">Builder API credentials</div>
                <div className="text-xs text-neutral-500">
                  Check this only if you registered as a Polymarket Builder operator (revenue share program).
                  Leave unchecked for regular trading.
                </div>
              </div>
            </label>
          </div>

          {generateMutation.data?.error && (
            <div className="text-red-400 text-sm flex items-start gap-2">
              <AlertCircle className="w-4 h-4 mt-0.5 flex-shrink-0" />
              <span>{generateMutation.data.error}</span>
            </div>
          )}

          <div className="flex justify-between">
            <button
              type="button"
              onClick={() => setStep(2)}
              className="text-neutral-400 hover:text-white text-sm flex items-center gap-1"
            >
              <ArrowLeft className="w-4 h-4" /> Back
            </button>
            <button
              type="button"
              disabled={generateMutation.isPending}
              onClick={() => generateMutation.mutate()}
              className="bg-green-600 hover:bg-green-700 disabled:bg-neutral-700 disabled:text-neutral-500 text-white px-4 py-2 rounded text-sm flex items-center gap-2"
            >
              {generateMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin" /> : null}
              Generate & Save
              <ArrowRight className="w-4 h-4" />
            </button>
          </div>
        </div>
      )}

      {/* Step 4: Done */}
      {step === 4 && generatedCreds && (
        <div className="space-y-4">
          <div className="bg-green-950 border border-green-800 rounded p-4 flex items-start gap-3">
            <CheckCircle className="w-6 h-6 text-green-400 flex-shrink-0 mt-0.5" />
            <div>
              <div className="text-green-100 font-semibold">Credentials saved</div>
              <div className="text-sm text-green-200 mt-1">
                Method used: <span className="font-mono">{generatedCreds.method_used}</span>
              </div>
            </div>
          </div>

          <div className="bg-neutral-800 border border-neutral-700 rounded p-3 text-sm space-y-2">
            <Row label="EOA wallet" value={generatedCreds.wallet_address || eoaAddress} />
            <Row label="Polymarket proxy" value={proxyAddress} />
            <Row label="API key" value={generatedCreds.api_key || ''} mono />
            <Row label="Secret" value={generatedCreds.secret_masked || ''} />
            <Row label="Passphrase" value={generatedCreds.passphrase_masked || ''} />
            <Row label="Signature type" value={signatureType || 'auto'} />
          </div>

          <div className="bg-blue-950 border border-blue-800 rounded p-3 text-xs text-blue-200">
            Next: restart the gateway so it loads the new credentials, then place a small test order
            from the strategies page or via the order panel.
          </div>

          <div className="flex justify-end">
            <button
              type="button"
              onClick={onComplete}
              className="bg-blue-500 hover:bg-blue-600 text-white px-4 py-2 rounded text-sm"
            >
              Done
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

function Row({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div>
      <div className="text-neutral-400 text-xs">{label}</div>
      <div className={`text-white text-xs break-all ${mono ? 'font-mono' : ''}`}>{value || '—'}</div>
    </div>
  )
}
