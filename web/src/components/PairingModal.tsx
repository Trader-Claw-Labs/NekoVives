import { useState, useEffect } from 'react'
import { Lock, KeyRound, X } from 'lucide-react'
import { setAuthToken } from '../hooks/useApi'

interface Props {
  onPaired: () => void
}

export default function PairingModal({ onPaired }: Props) {
  const [code, setCode] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Auto-focus the input when modal appears
  useEffect(() => {
    const input = document.getElementById('pairing-code-input')
    input?.focus()
  }, [])

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    const trimmed = code.trim()
    if (!trimmed) return

    setLoading(true)
    setError(null)

    try {
      const res = await fetch('/pair', {
        method: 'POST',
        headers: { 'X-Pairing-Code': trimmed },
      })

      const data = await res.json().catch(() => ({}))

      if (!res.ok) {
        setError(data?.error ?? `Error ${res.status}`)
        return
      }

      if (!data?.token) {
        setError('No token in response')
        return
      }

      setAuthToken(data.token)
      onPaired()
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Connection failed')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ backgroundColor: 'rgba(0,0,0,0.7)', backdropFilter: 'blur(4px)' }}
    >
      <div
        className="w-full max-w-sm mx-4 rounded-xl border p-6"
        style={{
          backgroundColor: 'var(--color-surface)',
          borderColor: 'var(--color-border)',
          boxShadow: '0 0 40px rgba(0,255,136,0.08)',
        }}
      >
        {/* Header */}
        <div className="flex items-center gap-3 mb-5">
          <div
            className="flex items-center justify-center w-9 h-9 rounded-lg"
            style={{ backgroundColor: 'var(--color-accent-dim)', color: 'var(--color-accent)' }}
          >
            <Lock size={18} />
          </div>
          <div>
            <h2 className="text-sm font-bold" style={{ color: 'var(--color-text)' }}>
              Session Pairing
            </h2>
            <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              Enter the one-time code from the terminal
            </p>
          </div>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          {/* Code input */}
          <div>
            <label
              htmlFor="pairing-code-input"
              className="block text-xs mb-1.5 font-medium"
              style={{ color: 'var(--color-text-muted)' }}
            >
              Pairing Code
            </label>
            <div className="relative">
              <KeyRound
                size={14}
                className="absolute left-3 top-1/2 -translate-y-1/2 pointer-events-none"
                style={{ color: 'var(--color-text-muted)' }}
              />
              <input
                id="pairing-code-input"
                type="text"
                value={code}
                onChange={(e) => { setCode(e.target.value); setError(null) }}
                placeholder="e.g. a1b2c3d4"
                autoComplete="off"
                spellCheck={false}
                className="w-full rounded-lg pl-8 pr-3 py-2.5 text-sm font-mono"
                style={{
                  backgroundColor: 'var(--color-surface-2)',
                  borderColor: error ? 'var(--color-danger)' : 'var(--color-border)',
                  color: 'var(--color-text)',
                  border: '1px solid',
                }}
              />
            </div>
            {error && (
              <p className="text-xs mt-1.5" style={{ color: 'var(--color-danger)' }}>
                {error}
              </p>
            )}
          </div>

          {/* Submit */}
          <button
            type="submit"
            disabled={loading || !code.trim()}
            className="w-full py-2.5 rounded-lg text-sm font-semibold transition-opacity disabled:opacity-40"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            {loading ? 'Pairing…' : 'Pair Session'}
          </button>
        </form>

        {/* Hint */}
        <p className="text-xs text-center mt-4" style={{ color: 'var(--color-text-muted)' }}>
          Run{' '}
          <code
            className="px-1 py-0.5 rounded"
            style={{ backgroundColor: 'var(--color-surface-2)', color: 'var(--color-accent)' }}
          >
            trader-claw gateway
          </code>{' '}
          and look for the pairing code in the terminal output.
        </p>
      </div>
    </div>
  )
}
