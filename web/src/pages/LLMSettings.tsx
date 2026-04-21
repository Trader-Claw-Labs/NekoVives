import { useState, useEffect } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import { Brain, Save, Eye, EyeOff, DollarSign } from 'lucide-react'

interface CostSummary {
  session_cost_usd: number
  daily_cost_usd: number
  monthly_cost_usd: number
  total_tokens: number
  request_count: number
  by_model: Record<string, { tokens: number; cost_usd: number; requests: number }>
}

interface ConfigResponse {
  format?: string
  content?: string
}

const PROVIDERS = [
  'anthropic',
  'openai',
  'openrouter',
  'groq',
  'mistral',
  'gemini',
  'ollama',
  'custom',
  'anthropic-custom',
]

const MODELS_BY_PROVIDER: Record<string, string[]> = {
  anthropic: [
    'claude-opus-4',
    'claude-sonnet-4',
    'claude-haiku-4',
    'claude-3-5-sonnet-20241022',
  ],
  openai: [
    'gpt-4o',
    'gpt-4o-mini',
    'gpt-4-turbo',
    'gpt-3.5-turbo',
  ],
  openrouter: [
    'anthropic/claude-sonnet-4',
    'anthropic/claude-opus-4',
    'openai/gpt-4o',
    'mistralai/mistral-7b-instruct',
    'meta-llama/llama-3.1-70b-instruct',
  ],
  groq: [
    'llama-3.1-70b-versatile',
    'llama-3.1-8b-instant',
    'mixtral-8x7b-32768',
    'gemma2-9b-it',
  ],
  mistral: [
    'mistral-large-latest',
    'mistral-medium-latest',
    'mistral-small-latest',
  ],
  gemini: [
    'gemini-2.5-flash',
    'gemini-2.5-pro',
    'gemini-2.0-flash',
    'gemini-2.0-flash-lite',
  ],
  ollama: ['llama3.2', 'mistral', 'qwen2.5'],
  custom: [],
}

export default function LLMSettings() {
  const [provider, setProvider] = useState('openrouter')
  const [model, setModel] = useState('anthropic/claude-sonnet-4')
  const [apiKey, setApiKey] = useState('')
  const [apiUrl, setApiUrl] = useState('')
  const [showApiKey, setShowApiKey] = useState(false)
  const [temperature, setTemperature] = useState(0.7)
  const [maxTokens, setMaxTokens] = useState(131072)
  const [saveMsg, setSaveMsg] = useState('')
  const [saveErr, setSaveErr] = useState('')

  const { data: configData } = useQuery<ConfigResponse>({
    queryKey: ['config'],
    queryFn: () => apiFetch('/api/config'),
  })

  // Parse TOML config to prefill fields
  useEffect(() => {
    if (!configData?.content) return
    const content = configData.content

    // Only parse from the top-level section (before first [header]) to avoid
    // picking up keys with the same name inside subsections (e.g. [transcription].api_url)
    const firstSection = content.search(/^\[/m)
    const topLevel = firstSection === -1 ? content : content.slice(0, firstSection)

    const providerMatch = topLevel.match(/default_provider\s*=\s*"([^"]+)"/)
    const modelMatch = topLevel.match(/default_model\s*=\s*"([^"]+)"/)
    const tempMatch = topLevel.match(/default_temperature\s*=\s*([\d.]+)/)
    const apiKeyMatch = topLevel.match(/^api_key\s*=\s*"([^"]*)"/m)
    const apiUrlMatch = topLevel.match(/^api_url\s*=\s*"([^"]*)"/m)

    if (providerMatch) {
      const raw = providerMatch[1]
      // "custom:https://..." → split into provider=custom + apiUrl
      if (raw.startsWith('custom:')) {
        setProvider('custom')
        setApiUrl(raw.slice('custom:'.length))
      } else if (raw.startsWith('anthropic-custom:')) {
        setProvider('anthropic-custom')
        setApiUrl(raw.slice('anthropic-custom:'.length))
      } else {
        setProvider(raw)
        // Recover URL from separate api_url field (legacy configs saved before url-embedding)
        if (apiUrlMatch && apiUrlMatch[1]) setApiUrl(apiUrlMatch[1])
      }
    }
    if (modelMatch) setModel(modelMatch[1])
    if (tempMatch) setTemperature(parseFloat(tempMatch[1]))
    if (apiKeyMatch && apiKeyMatch[1] && !apiKeyMatch[1].startsWith('•')) setApiKey(apiKeyMatch[1])
  }, [configData])

  const saveMutation = useMutation({
    mutationFn: async () => {
      if (!configData?.content) throw new Error('No config loaded')
      const toml = configData.content

      // Split TOML into top-level section (before first [header]) and the rest.
      // All LLM keys live at the top level — never touch keys inside [sections].
      const firstSection = toml.search(/^\[/m)
      let top = firstSection === -1 ? toml : toml.slice(0, firstSection)
      const rest = firstSection === -1 ? '' : toml.slice(firstSection)

      // For custom/anthropic-custom, embed the URL in the provider name itself.
      const needsUrlInName = provider === 'custom' || provider === 'anthropic-custom'
      const effectiveProvider = needsUrlInName && apiUrl.trim()
        ? `${provider}:${apiUrl.trim()}`
        : provider

      // Helper: set or append a key=value line in the top section only
      function setTopKey(src: string, key: string, value: string): string {
        const re = new RegExp(`^${key}\\s*=\\s*"[^"]*"`, 'm')
        if (re.test(src)) return src.replace(re, `${key} = "${value}"`)
        return src + `${key} = "${value}"\n`
      }
      function removeTopKey(src: string, key: string): string {
        return src.replace(new RegExp(`^${key}\\s*=\\s*"[^"]*"\\n?`, 'm'), '')
      }

      top = setTopKey(top, 'default_provider', effectiveProvider)
      top = setTopKey(top, 'default_model', model)
      // Temperature is a number, not a string
      top = /^default_temperature\s*=/m.test(top)
        ? top.replace(/^default_temperature\s*=\s*[\d.]+/m, `default_temperature = ${temperature}`)
        : top + `default_temperature = ${temperature}\n`

      if (apiKey) top = setTopKey(top, 'api_key', apiKey)

      // api_url: write at top level for ollama/openai/etc, and as fallback for custom types
      if (apiUrl.trim()) {
        top = setTopKey(top, 'api_url', apiUrl.trim())
      } else {
        top = removeTopKey(top, 'api_url')
      }

      const updatedToml = top + rest

      const res = await fetch('/api/config', {
        method: 'PUT',
        headers: { 'Content-Type': 'text/plain', Authorization: `Bearer ${localStorage.getItem('auth_token') ?? ''}` },
        body: updatedToml,
      })
      if (!res.ok) throw new Error(await res.text())
      return res.json()
    },
    onSuccess: () => {
      setSaveMsg('Saved! Model and temperature updated — no restart required.')
      setSaveErr('')
      setTimeout(() => setSaveMsg(''), 4000)
    },
    onError: (e: Error) => {
      setSaveErr(e.message)
      setSaveMsg('')
    },
  })

  const modelOptions = MODELS_BY_PROVIDER[provider] ?? []

  return (
    <div className="p-6 max-w-3xl mx-auto">
      <div className="flex items-center gap-2 mb-6">
        <Brain size={18} style={{ color: 'var(--color-accent)' }} />
        <h1 className="text-lg font-bold">LLM Settings</h1>
      </div>

      <div
        className="rounded-lg border p-5 space-y-5"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        {/* Provider */}
        <div>
          <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
            Provider
          </label>
          <select
            value={provider}
            onChange={(e) => setProvider(e.target.value)}
            className="w-full rounded px-3 py-2 text-sm"
          >
            {PROVIDERS.map((p) => (
              <option key={p} value={p}>{p}</option>
            ))}
          </select>
        </div>

        {/* Model */}
        <div>
          <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
            Model
          </label>
          <datalist id="model-suggestions">
            {modelOptions.map((m) => <option key={m} value={m} />)}
          </datalist>
          <input
            type="text"
            list="model-suggestions"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            className="w-full rounded px-3 py-2 text-sm font-mono"
            placeholder="Type or select a model..."
            autoComplete="off"
          />
          {modelOptions.length > 0 && (
            <p className="text-xs mt-1" style={{ color: 'var(--color-text-muted)' }}>
              Suggestions: {modelOptions.join(' · ')}
            </p>
          )}
        </div>

        {/* API Key */}
        <div>
          <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
            API Key
          </label>
          <div className="relative">
            <input
              type={showApiKey ? 'text' : 'password'}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm pr-10 font-mono"
              placeholder={`${provider} API key...`}
            />
            <button
              type="button"
              className="absolute right-2 top-1/2 -translate-y-1/2"
              onClick={() => setShowApiKey((s) => !s)}
              style={{ color: 'var(--color-text-muted)' }}
            >
              {showApiKey ? <EyeOff size={14} /> : <Eye size={14} />}
            </button>
          </div>
        </div>

        {/* Base URL (custom, anthropic-custom or ollama) */}
        {(provider === 'custom' || provider === 'anthropic-custom' || provider === 'ollama') && (
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
              Base URL
              {provider === 'ollama' && ' (e.g. http://localhost:11434)'}
              {provider === 'custom' && ' u2014 OpenAI-compatible endpoint'}
              {provider === 'anthropic-custom' && ' u2014 Anthropic-compatible endpoint'}
            </label>
            <input
              type="text"
              value={apiUrl}
              onChange={(e) => setApiUrl(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm font-mono"
              placeholder={
                provider === 'ollama' ? 'http://localhost:11434' :
                provider === 'anthropic-custom' ? 'http://localhost:20128' :
                'http://localhost:20128/v1'
              }
            />
            {(provider === 'custom' || provider === 'anthropic-custom') && apiUrl && (
              <p className="text-xs mt-1" style={{ color: 'var(--color-text-muted)' }}>
                Will save as: <span className="font-mono" style={{ color: 'var(--color-accent)' }}>{provider}:{apiUrl}</span>
              </p>
            )}
          </div>
        )}

        {/* Temperature */}
        <div>
          <div className="flex items-center justify-between mb-1">
            <label className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              Temperature
            </label>
            <span
              className="text-sm font-mono font-bold"
              style={{ color: 'var(--color-accent)' }}
            >
              {temperature.toFixed(2)}
            </span>
          </div>
          <input
            type="range"
            min={0}
            max={2}
            step={0.01}
            value={temperature}
            onChange={(e) => setTemperature(parseFloat(e.target.value))}
            className="w-full accent-[#00ff88]"
            style={{ accentColor: 'var(--color-accent)' }}
          />
          <div className="flex justify-between text-xs mt-1" style={{ color: 'var(--color-text-muted)' }}>
            <span>0.0 (deterministic)</span>
            <span>2.0 (creative)</span>
          </div>
        </div>

        {/* Max tokens */}
        <div>
          <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
            Max Tokens
          </label>
          <input
            type="number"
            value={maxTokens}
            onChange={(e) => setMaxTokens(parseInt(e.target.value, 10))}
            min={256}
            max={200000}
            step={1024}
            className="w-full rounded px-3 py-2 text-sm font-mono"
          />
          <p className="text-xs mt-1" style={{ color: 'var(--color-text-muted)' }}>
            128k = 131072 · 64k = 65536 · 32k = 32768
          </p>
        </div>

        {saveErr && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{saveErr}</p>}
        {saveMsg && <p className="text-xs" style={{ color: 'var(--color-accent)' }}>{saveMsg}</p>}

        <button
          onClick={() => saveMutation.mutate()}
          disabled={saveMutation.isPending}
          className="flex items-center gap-2 px-5 py-2 rounded text-sm font-medium disabled:opacity-50"
          style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
        >
          <Save size={14} />
          {saveMutation.isPending ? 'Saving...' : 'Save Settings'}
        </button>
      </div>

      <CostWidget />
    </div>
  )
}

// Estimate tokens from cost (rough: $3/M tokens for Claude Sonnet)
function fmtTok(usd: number): string {
  const tokens = Math.round(usd / 0.000003)
  if (tokens === 0) return '0'
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(0)}K`
  return tokens.toLocaleString()
}

function CostWidget() {
  const { data } = useQuery<{ cost: CostSummary }>({
    queryKey: ['cost'],
    queryFn: () => apiFetch<{ cost: CostSummary }>('/api/cost'),
    refetchInterval: 30_000,
  })
  const cost = data?.cost
  if (!cost) return null

  const byModel = Object.entries(cost.by_model ?? {})

  return (
    <div
      className="rounded-lg border p-5"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      <div className="flex items-center gap-2 mb-4">
        <DollarSign size={14} style={{ color: 'var(--color-accent)' }} />
        <h2 className="text-sm font-semibold">Usage & Cost</h2>
      </div>

      <div className="grid grid-cols-4 gap-3 mb-4">
        {[
          { label: 'Total Tokens', value: cost.total_tokens.toLocaleString() },
          { label: 'Session', value: fmtTok(cost.session_cost_usd) },
          { label: 'Today', value: fmtTok(cost.daily_cost_usd) },
          { label: 'Requests', value: cost.request_count.toLocaleString() },
        ].map(s => (
          <div key={s.label} className="rounded p-3 text-center"
            style={{ backgroundColor: 'var(--color-base)' }}>
            <div className="text-xs mb-1" style={{ color: 'var(--color-text-muted)' }}>{s.label}</div>
            <div className="text-sm font-bold" style={{ color: 'var(--color-accent)' }}>{s.value}</div>
          </div>
        ))}
      </div>

      {byModel.length > 0 && (
        <div className="space-y-1.5">
          {byModel.map(([model, stats]) => (
            <div key={model} className="flex items-center justify-between text-xs rounded px-3 py-1.5"
              style={{ backgroundColor: 'var(--color-base)' }}>
              <span className="font-mono truncate max-w-xs">{model}</span>
              <div className="flex gap-4 flex-shrink-0" style={{ color: 'var(--color-text-muted)' }}>
                <span>{stats.requests} req</span>
                <span style={{ color: 'var(--color-accent)' }}>{stats.tokens.toLocaleString()} tok</span>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
