import { useState } from 'react'
import { apiPost } from '../hooks/useApi'
import {
  Brain, Eye, EyeOff, ChevronRight, ChevronLeft, CheckCircle,
  Zap, Globe, Cpu,
} from 'lucide-react'

// ── Provider / model options ──────────────────────────────────────────────

const PROVIDERS = [
  {
    id: 'openrouter',
    name: 'OpenRouter',
    description: 'Access 200+ models via one API key',
    icon: Globe,
    keyLabel: 'OpenRouter API Key',
    keyPlaceholder: 'sk-or-v1-...',
    keyLink: 'https://openrouter.ai/keys',
    models: [
      'anthropic/claude-sonnet-4',
      'anthropic/claude-opus-4',
      'openai/gpt-4o',
      'meta-llama/llama-3.1-70b-instruct',
    ],
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    description: 'Direct access to Claude models',
    icon: Brain,
    keyLabel: 'Anthropic API Key',
    keyPlaceholder: 'sk-ant-...',
    keyLink: 'https://console.anthropic.com/keys',
    models: [
      'claude-opus-4',
      'claude-sonnet-4',
      'claude-haiku-4',
    ],
  },
  {
    id: 'openai',
    name: 'OpenAI',
    description: 'GPT-4o and other OpenAI models',
    icon: Zap,
    keyLabel: 'OpenAI API Key',
    keyPlaceholder: 'sk-...',
    keyLink: 'https://platform.openai.com/api-keys',
    models: ['gpt-4o', 'gpt-4o-mini', 'gpt-4-turbo'],
  },
  {
    id: 'gemini',
    name: 'Google Gemini',
    description: 'Gemini 2.5 Flash / Pro',
    icon: Cpu,
    keyLabel: 'Gemini API Key',
    keyPlaceholder: 'AIza...',
    keyLink: 'https://aistudio.google.com/app/apikey',
    models: ['gemini-2.5-flash', 'gemini-2.5-pro', 'gemini-2.0-flash'],
  },
  {
    id: 'groq',
    name: 'Groq',
    description: 'Ultra-fast inference (free tier)',
    icon: Zap,
    keyLabel: 'Groq API Key',
    keyPlaceholder: 'gsk_...',
    keyLink: 'https://console.groq.com/keys',
    models: ['llama-3.1-70b-versatile', 'llama-3.1-8b-instant', 'mixtral-8x7b-32768'],
  },
  {
    id: 'ollama',
    name: 'Ollama (local)',
    description: 'Run models locally — no API key needed',
    icon: Cpu,
    keyLabel: null,
    keyPlaceholder: null,
    keyLink: null,
    models: ['llama3.2', 'mistral', 'qwen2.5', 'gemma3'],
    needsUrl: true,
  },
]

// ── Step indicator ────────────────────────────────────────────────────────

function StepDot({ active, done }: { active: boolean; done: boolean }) {
  return (
    <div
      className="w-2 h-2 rounded-full transition-all"
      style={{
        backgroundColor: done
          ? 'var(--color-accent)'
          : active
            ? 'var(--color-accent)'
            : 'var(--color-border)',
        transform: active ? 'scale(1.3)' : 'scale(1)',
        opacity: done ? 0.6 : 1,
      }}
    />
  )
}

// ── Main modal ────────────────────────────────────────────────────────────

interface Props {
  onDone: () => void
}

export default function OnboardingModal({ onDone }: Props) {
  const [step, setStep] = useState(0) // 0=welcome 1=provider 2=apikey 3=model 4=done
  const [selectedProvider, setSelectedProvider] = useState(PROVIDERS[0])
  const [apiKey, setApiKey] = useState('')
  const [apiUrl, setApiUrl] = useState('http://localhost:11434')
  const [showKey, setShowKey] = useState(false)
  const [selectedModel, setSelectedModel] = useState(PROVIDERS[0].models[0])
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')

  const TOTAL_STEPS = 4 // welcome(0) provider(1) key(2) model(3) done(4)

  function selectProvider(p: typeof PROVIDERS[0]) {
    setSelectedProvider(p)
    setSelectedModel(p.models[0])
    setApiKey('')
    setApiUrl('http://localhost:11434')
  }

  async function finish() {
    setSaving(true)
    setError('')
    try {
      await apiPost('/api/onboarding/complete', {
        provider: selectedProvider.id,
        model: selectedModel,
        api_key: apiKey,
        api_url: selectedProvider.needsUrl ? apiUrl : '',
      })
      setStep(4)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to save')
    } finally {
      setSaving(false)
    }
  }

  function canAdvanceFromKey() {
    if (selectedProvider.needsUrl) return true // ollama — no key required
    if (!selectedProvider.keyLabel) return true // no key needed
    return apiKey.trim().length > 0
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ backgroundColor: 'rgba(0,0,0,0.8)', backdropFilter: 'blur(6px)' }}
    >
      <div
        className="w-full max-w-lg rounded-2xl border overflow-hidden"
        style={{
          backgroundColor: 'var(--color-surface)',
          borderColor: 'var(--color-border)',
          boxShadow: '0 0 60px rgba(0,255,136,0.1)',
        }}
      >
        {/* Header bar */}
        <div
          className="px-6 py-4 border-b flex items-center justify-between"
          style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-surface-2)' }}
        >
          <div className="flex items-center gap-2">
            <Brain size={16} style={{ color: 'var(--color-accent)' }} />
            <span className="text-sm font-bold">Setup Trader Claw</span>
          </div>
          {/* Step dots */}
          <div className="flex items-center gap-1.5">
            {[0, 1, 2, 3].map((i) => (
              <StepDot key={i} active={step === i} done={step > i} />
            ))}
          </div>
        </div>

        {/* Content */}
        <div className="p-6 min-h-[340px] flex flex-col">
          {/* ── Step 0: Welcome ── */}
          {step === 0 && (
            <div className="flex flex-col items-center justify-center flex-1 text-center">
              <div
                className="w-16 h-16 rounded-2xl flex items-center justify-center mb-5"
                style={{ backgroundColor: 'rgba(0,255,136,0.12)', border: '1px solid rgba(0,255,136,0.3)' }}
              >
                <Brain size={32} style={{ color: 'var(--color-accent)' }} />
              </div>
              <h2 className="text-xl font-bold mb-2">Welcome to Trader Claw</h2>
              <p className="text-sm max-w-sm" style={{ color: 'var(--color-text-muted)' }}>
                Let's get you set up in a few steps. You'll choose your AI provider and configure your API key so the agent can start working.
              </p>
              <div
                className="mt-5 px-4 py-3 rounded-lg text-xs text-left w-full max-w-sm"
                style={{ backgroundColor: 'rgba(0,255,136,0.06)', border: '1px solid rgba(0,255,136,0.2)', color: 'var(--color-text-muted)' }}
              >
                <p className="font-semibold mb-1" style={{ color: 'var(--color-accent)' }}>What you'll configure:</p>
                <ul className="space-y-0.5">
                  <li>• AI provider &amp; model (GPT-4o, Claude, Gemini…)</li>
                  <li>• API key for your chosen provider</li>
                  <li>• Optional: Polymarket, Telegram, Wallets</li>
                </ul>
              </div>
            </div>
          )}

          {/* ── Step 1: Choose provider ── */}
          {step === 1 && (
            <div className="flex flex-col flex-1">
              <h2 className="text-base font-bold mb-1">Choose your AI provider</h2>
              <p className="text-xs mb-4" style={{ color: 'var(--color-text-muted)' }}>
                The agent uses this to analyze markets and generate strategies.
              </p>
              <div className="grid grid-cols-2 gap-2 flex-1 content-start">
                {PROVIDERS.map((p) => {
                  const Icon = p.icon
                  const selected = selectedProvider.id === p.id
                  return (
                    <button
                      key={p.id}
                      onClick={() => selectProvider(p)}
                      className="flex items-start gap-3 p-3 rounded-xl border text-left transition-all"
                      style={{
                        borderColor: selected ? 'var(--color-accent)' : 'var(--color-border)',
                        backgroundColor: selected ? 'rgba(0,255,136,0.07)' : 'var(--color-surface-2)',
                      }}
                    >
                      <div
                        className="w-8 h-8 rounded-lg flex items-center justify-center flex-shrink-0"
                        style={{
                          backgroundColor: selected ? 'rgba(0,255,136,0.15)' : 'var(--color-border)',
                          color: selected ? 'var(--color-accent)' : 'var(--color-text-muted)',
                        }}
                      >
                        <Icon size={15} />
                      </div>
                      <div className="min-w-0">
                        <p className="text-sm font-semibold leading-none mb-0.5" style={{ color: selected ? 'var(--color-accent)' : 'var(--color-text)' }}>
                          {p.name}
                        </p>
                        <p className="text-xs leading-snug" style={{ color: 'var(--color-text-muted)' }}>
                          {p.description}
                        </p>
                      </div>
                    </button>
                  )
                })}
              </div>
            </div>
          )}

          {/* ── Step 2: API key ── */}
          {step === 2 && (
            <div className="flex flex-col flex-1">
              <h2 className="text-base font-bold mb-1">
                {selectedProvider.needsUrl ? 'Local server URL' : `Enter your ${selectedProvider.name} API key`}
              </h2>
              <p className="text-xs mb-5" style={{ color: 'var(--color-text-muted)' }}>
                {selectedProvider.needsUrl
                  ? 'Point to your local Ollama instance.'
                  : 'Your key is encrypted in config.toml and never sent anywhere other than the provider.'}
              </p>

              {selectedProvider.needsUrl ? (
                <div>
                  <label className="text-xs block mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
                    Ollama Base URL
                  </label>
                  <input
                    type="text"
                    value={apiUrl}
                    onChange={(e) => setApiUrl(e.target.value)}
                    className="w-full rounded-lg px-3 py-2.5 text-sm font-mono"
                    style={{
                      backgroundColor: 'var(--color-surface-2)',
                      border: '1px solid var(--color-border)',
                      color: 'var(--color-text)',
                    }}
                    placeholder="http://localhost:11434"
                  />
                </div>
              ) : selectedProvider.keyLabel ? (
                <div>
                  <label className="text-xs block mb-1.5" style={{ color: 'var(--color-text-muted)' }}>
                    {selectedProvider.keyLabel}
                  </label>
                  <div className="relative">
                    <input
                      type={showKey ? 'text' : 'password'}
                      value={apiKey}
                      onChange={(e) => setApiKey(e.target.value)}
                      className="w-full rounded-lg px-3 py-2.5 text-sm font-mono pr-10"
                      style={{
                        backgroundColor: 'var(--color-surface-2)',
                        border: `1px solid ${apiKey ? 'var(--color-accent)' : 'var(--color-border)'}`,
                        color: 'var(--color-text)',
                      }}
                      placeholder={selectedProvider.keyPlaceholder ?? ''}
                      autoComplete="off"
                    />
                    <button
                      type="button"
                      onClick={() => setShowKey((s) => !s)}
                      className="absolute right-3 top-1/2 -translate-y-1/2"
                      style={{ color: 'var(--color-text-muted)' }}
                    >
                      {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
                    </button>
                  </div>
                  {selectedProvider.keyLink && (
                    <p className="text-xs mt-2" style={{ color: 'var(--color-text-muted)' }}>
                      Don't have a key?{' '}
                      <a
                        href={selectedProvider.keyLink}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="underline"
                        style={{ color: 'var(--color-accent)' }}
                      >
                        Get one here
                      </a>
                    </p>
                  )}
                </div>
              ) : null}

              {error && (
                <p className="text-xs mt-3" style={{ color: 'var(--color-danger)' }}>{error}</p>
              )}
            </div>
          )}

          {/* ── Step 3: Model ── */}
          {step === 3 && (
            <div className="flex flex-col flex-1">
              <h2 className="text-base font-bold mb-1">Choose a model</h2>
              <p className="text-xs mb-4" style={{ color: 'var(--color-text-muted)' }}>
                This is the default model the trading agent will use. You can change it anytime in LLM Settings.
              </p>
              <div className="space-y-2">
                {selectedProvider.models.map((m) => (
                  <button
                    key={m}
                    onClick={() => setSelectedModel(m)}
                    className="w-full flex items-center justify-between px-4 py-3 rounded-xl border text-left transition-all"
                    style={{
                      borderColor: selectedModel === m ? 'var(--color-accent)' : 'var(--color-border)',
                      backgroundColor: selectedModel === m ? 'rgba(0,255,136,0.07)' : 'var(--color-surface-2)',
                    }}
                  >
                    <span
                      className="text-sm font-mono"
                      style={{ color: selectedModel === m ? 'var(--color-accent)' : 'var(--color-text)' }}
                    >
                      {m}
                    </span>
                    {selectedModel === m && (
                      <CheckCircle size={14} style={{ color: 'var(--color-accent)' }} />
                    )}
                  </button>
                ))}
              </div>
            </div>
          )}

          {/* ── Step 4: Done ── */}
          {step === 4 && (
            <div className="flex flex-col items-center justify-center flex-1 text-center">
              <div
                className="w-16 h-16 rounded-full flex items-center justify-center mb-5"
                style={{ backgroundColor: 'rgba(0,255,136,0.12)', border: '2px solid rgba(0,255,136,0.4)' }}
              >
                <CheckCircle size={32} style={{ color: 'var(--color-accent)' }} />
              </div>
              <h2 className="text-xl font-bold mb-2">You're all set!</h2>
              <p className="text-sm mb-1" style={{ color: 'var(--color-text-muted)' }}>
                Provider: <span style={{ color: 'var(--color-accent)' }}>{selectedProvider.name}</span>
              </p>
              <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
                Model: <span style={{ color: 'var(--color-accent)' }}>{selectedModel}</span>
              </p>
              <p className="text-xs mt-4 max-w-xs" style={{ color: 'var(--color-text-muted)' }}>
                You can update your API key, provider, and model anytime from <strong>LLM Settings</strong>.
              </p>
            </div>
          )}
        </div>

        {/* Footer actions */}
        <div
          className="flex items-center justify-between px-6 py-4 border-t"
          style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-surface-2)' }}
        >
          {/* Back */}
          <button
            onClick={() => setStep((s) => Math.max(0, s - 1))}
            className="flex items-center gap-1 text-sm px-3 py-1.5 rounded-lg transition-opacity"
            style={{
              color: 'var(--color-text-muted)',
              visibility: step > 0 && step < 4 ? 'visible' : 'hidden',
            }}
          >
            <ChevronLeft size={15} />
            Back
          </button>

          {/* Next / Save / Enter */}
          {step < 3 && (
            <button
              onClick={() => setStep((s) => s + 1)}
              disabled={step === 2 && !canAdvanceFromKey()}
              className="flex items-center gap-1.5 px-5 py-2 rounded-lg text-sm font-semibold transition-opacity disabled:opacity-40"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              {step === 0 ? 'Get Started' : 'Continue'}
              <ChevronRight size={15} />
            </button>
          )}

          {step === 3 && (
            <button
              onClick={finish}
              disabled={saving}
              className="flex items-center gap-1.5 px-5 py-2 rounded-lg text-sm font-semibold disabled:opacity-50"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              {saving ? 'Saving…' : 'Save & Finish'}
              <CheckCircle size={14} />
            </button>
          )}

          {step === 4 && (
            <button
              onClick={onDone}
              className="flex items-center gap-1.5 px-5 py-2 rounded-lg text-sm font-semibold"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
            >
              Open Dashboard
              <ChevronRight size={15} />
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
