import { useState } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { Send, Eye, EyeOff, Save, RefreshCw, MessageCircle } from 'lucide-react'
import clsx from 'clsx'

interface StatusData {
  channels?: Record<string, boolean>
}

interface RecentMessage {
  from?: string
  text?: string
  date?: string
}

export default function Telegram() {
  const [botToken, setBotToken] = useState('')
  const [showToken, setShowToken] = useState(false)
  const [allowedUsers, setAllowedUsers] = useState('')
  const [saveMsg, setSaveMsg] = useState('')
  const [saveErr, setSaveErr] = useState('')
  const [testMsg, setTestMsg] = useState('')
  const [testErr, setTestErr] = useState('')

  const { data: status } = useQuery<StatusData>({
    queryKey: ['status'],
    queryFn: () => apiFetch('/api/status'),
    refetchInterval: 10_000,
  })

  const isOnline = !!(status?.channels?.['telegram'])

  const saveMutation = useMutation({
    mutationFn: () =>
      apiPost('/api/channels/telegram/configure', {
        bot_token: botToken,
        allowed_users: allowedUsers.split('\n').map((s) => s.trim()).filter(Boolean),
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
    mutationFn: () => apiPost('/api/channels/telegram/test', {}),
    onSuccess: (data: unknown) => {
      const d = data as Record<string, unknown>
      setTestMsg(String(d?.message ?? 'Connection successful!'))
      setTestErr('')
    },
    onError: (e: Error) => {
      setTestErr(e.message)
      setTestMsg('')
    },
  })

  const recentMessages: RecentMessage[] = []

  return (
    <div className="p-6 max-w-3xl mx-auto">
      <div className="flex items-center gap-2 mb-6">
        <Send size={18} style={{ color: 'var(--color-accent)' }} />
        <h1 className="text-lg font-bold">Telegram</h1>
        <span className="ml-auto flex items-center gap-2 text-xs">
          <span className={clsx('status-dot', isOnline ? 'online' : 'offline')} />
          <span style={{ color: isOnline ? 'var(--color-accent)' : 'var(--color-text-muted)' }}>
            {isOnline ? 'Connected' : 'Disconnected'}
          </span>
        </span>
      </div>

      {/* Config form */}
      <div
        className="rounded-lg border p-5 mb-5"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <h2 className="text-sm font-semibold mb-4">Bot Configuration</h2>

        <div className="space-y-4 mb-5">
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
              Bot Token
            </label>
            <div className="relative">
              <input
                type={showToken ? 'text' : 'password'}
                value={botToken}
                onChange={(e) => setBotToken(e.target.value)}
                className="w-full rounded px-3 py-2 text-sm pr-10 font-mono"
                placeholder="123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefgh"
              />
              <button
                className="absolute right-2 top-1/2 -translate-y-1/2"
                onClick={() => setShowToken((s) => !s)}
                style={{ color: 'var(--color-text-muted)' }}
                type="button"
              >
                {showToken ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
          </div>

          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
              Allowed Users (one username per line)
            </label>
            <textarea
              value={allowedUsers}
              onChange={(e) => setAllowedUsers(e.target.value)}
              className="w-full rounded px-3 py-2 text-sm font-mono resize-none"
              rows={4}
              placeholder="@username1&#10;@username2"
            />
            <p className="text-xs mt-1" style={{ color: 'var(--color-text-muted)' }}>
              Only these Telegram users will be allowed to interact with the bot
            </p>
          </div>
        </div>

        {saveErr && <p className="text-xs mb-3" style={{ color: 'var(--color-danger)' }}>{saveErr}</p>}
        {saveMsg && <p className="text-xs mb-3" style={{ color: 'var(--color-accent)' }}>{saveMsg}</p>}

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

        {testErr && <p className="text-xs mt-3" style={{ color: 'var(--color-danger)' }}>{testErr}</p>}
        {testMsg && <p className="text-xs mt-3" style={{ color: 'var(--color-accent)' }}>{testMsg}</p>}
      </div>

      {/* Recent messages preview */}
      <div
        className="rounded-lg border"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div
          className="flex items-center gap-2 px-5 py-3 border-b"
          style={{ borderColor: 'var(--color-border)' }}
        >
          <MessageCircle size={14} style={{ color: 'var(--color-accent)' }} />
          <h2 className="text-sm font-semibold">Recent Messages</h2>
        </div>
        {recentMessages.length === 0 ? (
          <div className="p-8 text-center text-sm" style={{ color: 'var(--color-text-muted)' }}>
            {isOnline
              ? 'No recent messages'
              : 'Connect your bot to see messages here'}
          </div>
        ) : (
          <div className="divide-y" style={{ borderColor: 'var(--color-border)' }}>
            {recentMessages.map((m, i) => (
              <div key={i} className="px-5 py-3">
                <div className="flex items-center justify-between text-xs mb-1">
                  <span style={{ color: 'var(--color-accent)' }}>{m.from ?? 'unknown'}</span>
                  <span style={{ color: 'var(--color-text-muted)' }}>{m.date}</span>
                </div>
                <p className="text-sm">{m.text}</p>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
