import { useState, useEffect } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { apiFetch, apiPost } from '../hooks/useApi'
import { Send, Eye, EyeOff, Save, RefreshCw, MessageCircle, Bot, ChevronDown, ChevronUp, ExternalLink, CheckCircle } from 'lucide-react'
import clsx from 'clsx'

interface StatusData {
  channels?: Record<string, boolean>
}

interface TelegramConfigData {
  configured: boolean
  bot_token_masked?: string
  allowed_users?: string[]
}

interface RecentMessage {
  from: string
  text: string
  date: string
}

interface RecentMessagesData {
  messages: RecentMessage[]
}

export default function Telegram() {
  const [botToken, setBotToken] = useState('')
  const [showToken, setShowToken] = useState(false)
  const [allowedUsers, setAllowedUsers] = useState('')
  const [saveMsg, setSaveMsg] = useState('')
  const [saveErr, setSaveErr] = useState('')
  const [testMsg, setTestMsg] = useState('')
  const [testErr, setTestErr] = useState('')
  const [showGuide, setShowGuide] = useState(false)

  const { data: status } = useQuery<StatusData>({
    queryKey: ['status'],
    queryFn: () => apiFetch('/api/status'),
    refetchInterval: 10_000,
  })

  // Load saved config on mount
  const { data: savedConfig } = useQuery<TelegramConfigData>({
    queryKey: ['telegram-config'],
    queryFn: () => apiFetch('/api/channels/telegram/configure'),
  })

  const { data: messagesData } = useQuery<RecentMessagesData>({
    queryKey: ['telegram-messages'],
    queryFn: () => apiFetch('/api/channels/telegram/messages'),
    refetchInterval: 5_000,
    enabled: savedConfig?.configured ?? false,
  })

  // Populate form fields once config is loaded
  useEffect(() => {
    if (savedConfig?.configured) {
      // Don't populate the token field with the masked version —
      // keep it empty so the user knows they must re-enter to change it.
      // But do populate allowed_users.
      if (savedConfig.allowed_users && savedConfig.allowed_users.length > 0) {
        setAllowedUsers(savedConfig.allowed_users.join('\n'))
      }
    }
  }, [savedConfig])

  const isOnline = !!(status?.channels?.['telegram'])
  const isConfigured = savedConfig?.configured ?? false

  const saveMutation = useMutation({
    mutationFn: () => {
      // If token field is blank and already configured, send a sentinel so the
      // backend knows to keep the existing token (we pass a special empty marker).
      const tokenToSend = botToken.trim() || (isConfigured ? '__keep__' : '')
      return apiPost('/api/channels/telegram/configure', {
        bot_token: tokenToSend,
        allowed_users: allowedUsers.split('\n').map((s) => s.trim()).filter(Boolean),
      })
    },
    onSuccess: () => {
      setSaveMsg('Saved! Restart the gateway for the Telegram bot to connect.')
      setSaveErr('')
      setBotToken('')
      setTimeout(() => setSaveMsg(''), 5000)
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

  const recentMessages: RecentMessage[] = messagesData?.messages ?? []

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
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-sm font-semibold">Bot Configuration</h2>
          {isConfigured && (
            <span className="flex items-center gap-1.5 text-xs" style={{ color: 'var(--color-accent)' }}>
              <CheckCircle size={12} />
              Configured
              {savedConfig?.bot_token_masked && (
                <span className="font-mono ml-1" style={{ color: 'var(--color-text-muted)' }}>
                  {savedConfig.bot_token_masked}
                </span>
              )}
            </span>
          )}
        </div>

        {/* BotFather guide */}
        <div
          className="rounded-lg border mb-4 overflow-hidden"
          style={{ borderColor: 'var(--color-border)' }}
        >
          <button
            type="button"
            onClick={() => setShowGuide((v) => !v)}
            className="w-full flex items-center gap-2 px-4 py-2.5 text-left text-xs hover:bg-white/5 transition-colors"
            style={{ backgroundColor: 'var(--color-surface-2)' }}
          >
            <Bot size={13} style={{ color: 'var(--color-accent)', flexShrink: 0 }} />
            <span className="font-medium" style={{ color: 'var(--color-text)' }}>
              How to get a Bot Token
            </span>
            <span className="ml-auto" style={{ color: 'var(--color-text-muted)' }}>
              {showGuide ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
            </span>
          </button>

          {showGuide && (
            <div
              className="px-4 py-3 border-t text-xs space-y-3"
              style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
            >
              <p>
                Telegram bots are created and managed through{' '}
                <a
                  href="https://t.me/BotFather"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1 hover:underline"
                  style={{ color: 'var(--color-accent)' }}
                >
                  @BotFather <ExternalLink size={10} />
                </a>
                {' '}— the official Telegram bot for creating bots.
              </p>

              <ol className="space-y-2 list-none">
                {[
                  { n: '1', text: <>Open Telegram and search for <span style={{ color: 'var(--color-accent)' }}>@BotFather</span>, or tap the link above.</> },
                  { n: '2', text: <>Send the command <code className="px-1 rounded" style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-accent)' }}>/newbot</code> to start the creation flow.</> },
                  { n: '3', text: <>Choose a display name for your bot (e.g. <em>Trader Claw</em>).</> },
                  { n: '4', text: <>Choose a unique username ending in <code className="px-1 rounded" style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-accent)' }}>bot</code> (e.g. <em>traderclaw_bot</em>).</> },
                  { n: '5', text: <>BotFather will reply with a token like <code className="px-1 rounded font-mono" style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-accent)' }}>123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZabcde</code>. Copy it and paste it below.</> },
                ].map(({ n, text }) => (
                  <li key={n} className="flex gap-2">
                    <span
                      className="flex-shrink-0 w-5 h-5 rounded-full flex items-center justify-center text-xs font-bold"
                      style={{ backgroundColor: 'rgba(0,255,136,0.12)', color: 'var(--color-accent)' }}
                    >
                      {n}
                    </span>
                    <span className="leading-relaxed">{text}</span>
                  </li>
                ))}
              </ol>

              <div
                className="rounded p-2.5 flex gap-2 text-xs"
                style={{ backgroundColor: 'rgba(255,170,0,0.08)', borderLeft: '2px solid var(--color-warning)', color: 'var(--color-warning)' }}
              >
                <span className="flex-shrink-0">⚠</span>
                <span>
                  Keep your token private — anyone with it can control your bot.
                  If compromised, regenerate it via <strong>/mybots</strong> → <em>API Token</em> → <em>Revoke current token</em> in BotFather.
                </span>
              </div>

              <p>
                Also add your Telegram username to <strong>Allowed Users</strong> below so only you can send commands to the bot.
              </p>
            </div>
          )}
        </div>

        <div className="space-y-4 mb-5">
          <div>
            <label className="text-xs block mb-1" style={{ color: 'var(--color-text-muted)' }}>
              Bot Token
              {isConfigured && (
                <span className="ml-2" style={{ color: 'var(--color-text-muted)', opacity: 0.6 }}>
                  (leave blank to keep current)
                </span>
              )}
            </label>
            <div className="relative">
              <input
                type={showToken ? 'text' : 'password'}
                value={botToken}
                onChange={(e) => setBotToken(e.target.value)}
                className="w-full rounded px-3 py-2 text-sm pr-10 font-mono"
                placeholder={isConfigured ? savedConfig?.bot_token_masked ?? '••••••••' : '123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefgh'}
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
                  <span style={{ color: 'var(--color-accent)' }}>@{m.from}</span>
                  <span style={{ color: 'var(--color-text-muted)' }}>
                    {m.date ? new Date(m.date).toLocaleString() : ''}
                  </span>
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
