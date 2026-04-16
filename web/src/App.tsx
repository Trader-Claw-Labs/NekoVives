import { Component, useState, useEffect } from 'react'
import type { ReactNode } from 'react'
import { Routes, Route } from 'react-router-dom'
import Sidebar from './components/Sidebar'
import PairingModal from './components/PairingModal'
import OnboardingModal from './components/OnboardingModal'
import Dashboard from './pages/Dashboard'
import Wallets from './pages/Wallets'
import Polymarket from './pages/Polymarket'
import Telegram from './pages/Telegram'
import Skills from './pages/Skills'
import ScheduledJobs from './pages/ScheduledJobs'
import Chat from './pages/Chat'
import { ChatProvider } from './context/ChatContext'
import LLMSettings from './pages/LLMSettings'
import Config from './pages/Config'
import TradingViewPage from './pages/TradingView'
import Backtesting from './pages/Backtesting'
import LiveStrategies from './pages/LiveStrategies'
import SystemHealth from './pages/SystemHealth'
import Memory from './pages/Memory'
import { apiFetch, getAuthToken } from './hooks/useApi'

// ── Error boundary ────────────────────────────────────────────────
class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state = { error: null }

  static getDerivedStateFromError(error: Error) {
    return { error }
  }

  render() {
    if (this.state.error) {
      return (
        <div
          className="flex h-screen items-center justify-center p-8"
          style={{ backgroundColor: 'var(--color-base)' }}
        >
          <div
            className="max-w-lg w-full rounded-xl border p-6 text-sm font-mono"
            style={{
              backgroundColor: 'var(--color-surface)',
              borderColor: 'var(--color-danger)',
              color: 'var(--color-danger)',
            }}
          >
            <p className="font-bold mb-2">Runtime error</p>
            <p style={{ color: 'var(--color-text-muted)' }}>
              {(this.state.error as Error).message}
            </p>
            <button
              className="mt-4 px-4 py-2 rounded text-xs font-semibold"
              style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
              onClick={() => window.location.reload()}
            >
              Reload
            </button>
          </div>
        </div>
      )
    }
    return this.props.children
  }
}

// ── Onboarding status check ────────────────────────────────────────

interface OnboardingStatus {
  onboarded: boolean
  api_key_set: boolean
  provider: string
  model: string
}

async function fetchOnboardingStatus(): Promise<OnboardingStatus> {
  return apiFetch<OnboardingStatus>('/api/onboarding')
}

// ── App ───────────────────────────────────────────────────────────
export default function App() {
  const [showPairing, setShowPairing] = useState(false)
  const [showOnboarding, setShowOnboarding] = useState(false)

  // On mount: check pairing, then check onboarding
  useEffect(() => {
    fetch('/health')
      .then((r) => r.json())
      .then((data: { require_pairing?: boolean }) => {
        if (data.require_pairing && !getAuthToken()) {
          setShowPairing(true)
        } else {
          // Already authenticated — check if onboarding is needed
          checkOnboarding()
        }
      })
      .catch(() => {
        if (!getAuthToken()) {
          setShowPairing(true)
        } else {
          checkOnboarding()
        }
      })
  }, [])

  async function checkOnboarding() {
    try {
      const status = await fetchOnboardingStatus()
      // Show onboarding if: not yet completed OR api_key not set
      if (!status.onboarded || !status.api_key_set) {
        setShowOnboarding(true)
      }
    } catch {
      // If the check fails (no auth, server error), skip onboarding silently
    }
  }

  function handlePaired() {
    setShowPairing(false)
    // After pairing, always check onboarding for first-time users
    checkOnboarding()
  }

  function handleOnboardingDone() {
    setShowOnboarding(false)
  }

  return (
    <ErrorBoundary>
      <div className="flex h-screen overflow-hidden" style={{ backgroundColor: 'var(--color-base)' }}>
        <Sidebar />
        <main className="flex-1 overflow-auto">
          <ChatProvider>
          <Routes>
            <Route path="/" element={<Dashboard />} />
            <Route path="/wallets" element={<Wallets />} />
            <Route path="/polymarket" element={<Polymarket />} />
            <Route path="/telegram" element={<Telegram />} />
            <Route path="/skills" element={<Skills />} />
            <Route path="/scheduled-jobs" element={<ScheduledJobs />} />
            <Route path="/chat" element={<Chat />} />
            <Route path="/tradingview" element={<TradingViewPage />} />
            <Route path="/backtesting" element={<Backtesting />} />
            <Route path="/live" element={<LiveStrategies />} />
            <Route path="/health" element={<SystemHealth />} />
            <Route path="/memory" element={<Memory />} />
            <Route path="/settings/llm" element={<LLMSettings />} />
            <Route path="/settings/config" element={<Config />} />
          </Routes>
          </ChatProvider>
        </main>

        {showPairing && <PairingModal onPaired={handlePaired} />}
        {!showPairing && showOnboarding && (
          <OnboardingModal onDone={handleOnboardingDone} />
        )}
      </div>
    </ErrorBoundary>
  )
}
