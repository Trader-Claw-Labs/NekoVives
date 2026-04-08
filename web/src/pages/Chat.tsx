import { useState, useRef, useEffect, useCallback } from 'react'
import {
  Plus, X, Wifi, WifiOff, Pencil, Terminal, Cpu,
  Zap, BarChart2, Search, Wallet, BookOpen, Settings2, ChevronRight,
} from 'lucide-react'
import { useWebSocket } from '../hooks/useWebSocket'
import { apiPost } from '../hooks/useApi'
import clsx from 'clsx'

// ── Slash commands ──────────────────────────────────────────────────

interface SlashCommand {
  cmd: string
  description: string
  prompt: string
  icon: React.ReactNode
  category: 'strategy' | 'analysis' | 'wallet' | 'system'
}

const SLASH_COMMANDS: SlashCommand[] = [
  {
    cmd: '/strategy',
    description: 'Generate a new Rhai trading strategy',
    prompt: 'Create a Rhai trading strategy that ',
    icon: <Zap size={13} />,
    category: 'strategy',
  },
  {
    cmd: '/backtest',
    description: 'Run a backtest on a strategy',
    prompt: 'Run a backtest on the strategy at scripts/',
    icon: <BarChart2 size={13} />,
    category: 'strategy',
  },
  {
    cmd: '/scan',
    description: 'Scan markets for opportunities',
    prompt: 'Scan the top crypto markets for trading opportunities using the TradingView screener',
    icon: <Search size={13} />,
    category: 'analysis',
  },
  {
    cmd: '/rsi',
    description: 'Check RSI for a symbol',
    prompt: 'What is the current RSI for ',
    icon: <BarChart2 size={13} />,
    category: 'analysis',
  },
  {
    cmd: '/buy',
    description: 'Execute a buy order',
    prompt: 'Buy ',
    icon: <Zap size={13} />,
    category: 'strategy',
  },
  {
    cmd: '/sell',
    description: 'Execute a sell order',
    prompt: 'Sell ',
    icon: <Zap size={13} />,
    category: 'strategy',
  },
  {
    cmd: '/wallets',
    description: 'List connected wallets and balances',
    prompt: 'Show me all connected wallets and their balances',
    icon: <Wallet size={13} />,
    category: 'wallet',
  },
  {
    cmd: '/polymarket',
    description: 'Browse Polymarket prediction markets',
    prompt: 'Show me the top Polymarket prediction markets by volume',
    icon: <BarChart2 size={13} />,
    category: 'analysis',
  },
  {
    cmd: '/memory',
    description: 'Show what the agent remembers',
    prompt: 'What do you remember from our previous conversations?',
    icon: <BookOpen size={13} />,
    category: 'system',
  },
  {
    cmd: '/status',
    description: 'Show system status and health',
    prompt: 'What is the current system status? Show provider, model, and component health.',
    icon: <Settings2 size={13} />,
    category: 'system',
  },
  {
    cmd: '/scripts',
    description: 'List available strategy scripts',
    prompt: 'List all available .rhai strategy scripts in the scripts directory',
    icon: <Terminal size={13} />,
    category: 'strategy',
  },
  {
    cmd: '/help',
    description: 'Show all available agent capabilities',
    prompt: 'What can you do? List all your capabilities and available tools.',
    icon: <BookOpen size={13} />,
    category: 'system',
  },
]

const CATEGORY_COLOR: Record<SlashCommand['category'], string> = {
  strategy: 'var(--color-accent)',
  analysis:  '#60a5fa',
  wallet:    '#f59e0b',
  system:    'var(--color-text-muted)',
}

// ── Quick-start prompt cards shown in empty state ────────────────────

const QUICK_PROMPTS = [
  { label: 'generate rsi strategy', prompt: 'Create a Rhai trading strategy that buys BTC when RSI < 30 and sells when RSI > 70. Save it to scripts/rsi_btc.rhai', icon: <Zap size={12} /> },
  { label: 'scan top markets', prompt: 'Scan the top 10 crypto markets for trading opportunities using RSI and MACD indicators', icon: <Search size={12} /> },
  { label: 'list wallets', prompt: 'Show me all connected wallets and their balances', icon: <Wallet size={12} /> },
  { label: 'top polymarket', prompt: 'Show me the top 5 Polymarket prediction markets by volume with current prices', icon: <BarChart2 size={12} /> },
]

interface ToolEvent {
  type: 'tool_call' | 'tool_result'
  name: string
  summary?: string
}

interface Message {
  role: 'user' | 'assistant'
  content: string
  timestamp: number
  streaming?: boolean
  toolEvents?: ToolEvent[]
}

interface Session {
  id: string
  label: string
  messages: Message[]
}

function generateId(): string {
  return Math.random().toString(36).slice(2, 10)
}

// ── Tool events log shown while agent is thinking ──────────────────

function AgentThinking({ toolEvents }: { toolEvents: ToolEvent[] }) {
  const latest = toolEvents.filter((e) => e.type === 'tool_call').slice(-1)[0]
  return (
    <div className="flex flex-col gap-0.5">
      {toolEvents.map((ev, i) => (
        <div key={i} className="tool-call-badge flex items-center gap-2 text-xs font-mono">
          {ev.type === 'tool_call' ? (
            <>
              <Terminal size={10} style={{ color: 'var(--color-accent)', flexShrink: 0 }} />
              <span style={{ color: 'var(--color-accent)' }}>→ {ev.name}</span>
            </>
          ) : (
            <>
              <Cpu size={10} style={{ color: 'var(--color-text-muted)', flexShrink: 0 }} />
              <span style={{ color: 'var(--color-text-muted)' }}>← {ev.name}</span>
            </>
          )}
          {ev.summary && (
            <span className="truncate max-w-xs" style={{ color: 'var(--color-text-muted)', opacity: 0.7 }}>
              {ev.summary}
            </span>
          )}
        </div>
      ))}
      <div className="flex items-center gap-2 text-xs font-mono mt-0.5">
        <span className="agent-spinner" />
        <span style={{ color: 'var(--color-text-muted)' }}>
          {latest ? `running ${latest.name}…` : 'thinking…'}
        </span>
      </div>
    </div>
  )
}

// ── Terminal message row ────────────────────────────────────────────

function TerminalLine({ msg }: { msg: Message }) {
  const isUser = msg.role === 'user'
  const isEmpty = !msg.content && msg.streaming

  if (isUser) {
    return (
      <div className="flex gap-3 py-1.5 group">
        <span
          className="select-none flex-shrink-0 text-base leading-5"
          style={{ color: 'var(--color-accent)', fontWeight: 700 }}
        >
          ❯
        </span>
        <span
          className="text-sm whitespace-pre-wrap leading-relaxed flex-1"
          style={{ color: 'var(--color-text)' }}
        >
          {msg.content}
        </span>
        <span
          className="text-xs self-start opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0"
          style={{ color: 'var(--color-text-muted)' }}
        >
          {new Date(msg.timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
        </span>
      </div>
    )
  }

  return (
    <div className="pl-6 py-1 group">
      {isEmpty ? (
        <AgentThinking toolEvents={msg.toolEvents ?? []} />
      ) : (
        <>
          {/* Completed tool events */}
          {msg.toolEvents && msg.toolEvents.length > 0 && (
            <div className="flex flex-col gap-0.5 mb-1.5">
              {msg.toolEvents.map((ev, i) => (
                <div key={i} className="flex items-center gap-2 text-xs font-mono">
                  {ev.type === 'tool_call' ? (
                    <>
                      <Terminal size={10} style={{ color: 'var(--color-accent)', flexShrink: 0 }} />
                      <span style={{ color: 'var(--color-accent)' }}>→ {ev.name}</span>
                    </>
                  ) : (
                    <>
                      <Cpu size={10} style={{ color: 'var(--color-text-muted)', flexShrink: 0 }} />
                      <span style={{ color: 'var(--color-text-muted)' }}>← {ev.name}</span>
                    </>
                  )}
                  {ev.summary && (
                    <span className="truncate max-w-xs" style={{ color: 'var(--color-text-muted)', opacity: 0.6 }}>
                      {ev.summary}
                    </span>
                  )}
                </div>
              ))}
            </div>
          )}
          <span
            className={clsx('text-sm whitespace-pre-wrap leading-relaxed', msg.streaming && 'typewriter-cursor')}
            style={{ color: 'var(--color-text)' }}
          >
            {msg.content}
          </span>
        </>
      )}
    </div>
  )
}

// ── Chat window ─────────────────────────────────────────────────────

interface ChatWindowProps {
  session: Session
  onUpdate: (session: Session) => void
  visible: boolean
}

function ChatWindow({ session, onUpdate, visible }: ChatWindowProps) {
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const [cmdIndex, setCmdIndex] = useState(0)
  const bottomRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)
  const sessionRef = useRef(session)
  useEffect(() => { sessionRef.current = session }, [session])

  // Slash-command filtering
  const slashMatch = input.match(/^(\/\S*)/)
  const slashQuery = slashMatch ? slashMatch[1].toLowerCase() : null
  const filteredCmds = slashQuery
    ? SLASH_COMMANDS.filter(
        (c) => c.cmd.startsWith(slashQuery) || c.description.toLowerCase().includes(slashQuery.slice(1))
      )
    : []
  const showPalette = slashQuery !== null && filteredCmds.length > 0

  useEffect(() => { setCmdIndex(0) }, [slashQuery])

  const { connected, send } = useWebSocket('/ws/chat', {
    onMessage: (msg) => {
      if (msg.session_id !== sessionRef.current.id) return
      const cur = sessionRef.current

      if (msg.type === 'chunk' && typeof msg.content === 'string') {
        onUpdate({
          ...cur,
          messages: cur.messages.map((m, i) =>
            i === cur.messages.length - 1 && m.role === 'assistant' && m.streaming
              ? { ...m, content: m.content + msg.content }
              : m
          ),
        })
      } else if (msg.type === 'tool_call') {
        onUpdate({
          ...cur,
          messages: cur.messages.map((m, i) =>
            i === cur.messages.length - 1 && m.role === 'assistant' && m.streaming
              ? {
                  ...m,
                  toolEvents: [
                    ...(m.toolEvents ?? []),
                    { type: 'tool_call', name: String(msg.name ?? ''), summary: summariseArgs(msg.args) },
                  ],
                }
              : m
          ),
        })
      } else if (msg.type === 'tool_result') {
        onUpdate({
          ...cur,
          messages: cur.messages.map((m, i) =>
            i === cur.messages.length - 1 && m.role === 'assistant' && m.streaming
              ? {
                  ...m,
                  toolEvents: [
                    ...(m.toolEvents ?? []),
                    { type: 'tool_result', name: String(msg.name ?? ''), summary: truncate(String(msg.output ?? ''), 60) },
                  ],
                }
              : m
          ),
        })
      } else if (msg.type === 'done') {
        const fullResponse = typeof msg.full_response === 'string' ? msg.full_response : undefined
        onUpdate({
          ...cur,
          messages: cur.messages.map((m) =>
            m.streaming
              ? { ...m, content: fullResponse ?? m.content, streaming: false }
              : m
          ),
        })
        setSending(false)
      } else if (msg.type === 'error') {
        onUpdate({
          ...cur,
          messages: cur.messages.map((m) =>
            m.streaming
              ? { ...m, content: `error: ${msg.message ?? 'unknown error'}`, streaming: false }
              : m
          ),
        })
        setSending(false)
      }
    },
  })

  useEffect(() => {
    if (visible) bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [session.messages, visible])

  // Auto-resize textarea
  useEffect(() => {
    const el = inputRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`
  }, [input])

  async function handleSend() {
    if (!input.trim() || sending) return

    const userMsg: Message = { role: 'user', content: input.trim(), timestamp: Date.now() }
    const assistantMsg: Message = {
      role: 'assistant',
      content: '',
      timestamp: Date.now(),
      streaming: true,
      toolEvents: [],
    }

    const updated: Session = {
      ...session,
      messages: [...session.messages, userMsg, assistantMsg],
    }
    onUpdate(updated)
    setSending(true)
    setInput('')

    if (connected) {
      send({ type: 'message', content: input.trim(), session_id: session.id })
    } else {
      try {
        const res = await apiPost<{ response?: string; content?: string }>(
          '/api/chat',
          { session_id: session.id, message: input.trim() }
        )
        const text = res.response ?? res.content ?? 'No response'
        onUpdate({
          ...updated,
          messages: updated.messages.map((m) =>
            m.streaming ? { ...m, content: text, streaming: false } : m
          ),
        })
      } catch (e) {
        onUpdate({
          ...updated,
          messages: updated.messages.map((m) =>
            m.streaming
              ? { ...m, content: `error: ${String(e)}`, streaming: false }
              : m
          ),
        })
      } finally {
        setSending(false)
      }
    }
  }

  function applyCommand(cmd: SlashCommand) {
    setInput(cmd.prompt)
    setCmdIndex(0)
    setTimeout(() => {
      inputRef.current?.focus()
      const len = cmd.prompt.length
      inputRef.current?.setSelectionRange(len, len)
    }, 0)
  }

  function handleKey(e: React.KeyboardEvent) {
    if (showPalette) {
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setCmdIndex((i) => Math.min(i + 1, filteredCmds.length - 1))
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setCmdIndex((i) => Math.max(i - 1, 0))
        return
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && filteredCmds.length > 0)) {
        e.preventDefault()
        applyCommand(filteredCmds[cmdIndex])
        return
      }
      if (e.key === 'Escape') {
        setInput('')
        return
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  return (
    <div className="flex flex-col h-full font-mono" style={{ display: visible ? 'flex' : 'none' }}>
      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-4 py-3">
        {session.messages.length === 0 ? (
          <div className="flex flex-col gap-4 pt-8 px-1">
            {/* Banner */}
            <div>
              <div className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
                trader-claw agent — shell · file i/o · strategies · markets
              </div>
              <div className="text-xs mt-1" style={{ color: 'var(--color-text-muted)', opacity: 0.6 }}>
                type <span style={{ color: 'var(--color-accent)' }}>/</span> for commands · <span style={{ color: 'var(--color-accent)' }}>↵</span> to send · <span style={{ color: 'var(--color-accent)' }}>shift+↵</span> for newline
              </div>
            </div>

            {/* Quick-start suggestions */}
            <div className="flex flex-col gap-1">
              {QUICK_PROMPTS.map((p) => (
                <button
                  key={p.label}
                  onClick={() => { setInput(p.prompt); inputRef.current?.focus() }}
                  className="flex items-center gap-2 text-left text-xs py-1 transition-colors hover:opacity-100"
                  style={{ color: 'var(--color-text-muted)', opacity: 0.7 }}
                >
                  <span style={{ color: 'var(--color-accent)', flexShrink: 0 }}>{p.icon}</span>
                  <span className="hover:underline">{p.label}</span>
                </button>
              ))}
            </div>
          </div>
        ) : (
          <div className="flex flex-col gap-0.5">
            {session.messages.map((msg, i) => (
              <TerminalLine key={i} msg={msg} />
            ))}
          </div>
        )}
        <div ref={bottomRef} />
      </div>

      {/* Slash-command palette */}
      {showPalette && (
        <div
          className="border-t border-b"
          style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
        >
          <div className="px-3 py-1.5 flex items-center gap-1.5 text-xs" style={{ color: 'var(--color-text-muted)' }}>
            <Terminal size={10} />
            <span>
              <kbd className="font-mono">↑↓</kbd> navigate ·{' '}
              <kbd className="font-mono">tab/↵</kbd> select ·{' '}
              <kbd className="font-mono">esc</kbd> dismiss
            </span>
          </div>
          <div className="max-h-48 overflow-y-auto">
            {filteredCmds.map((c, i) => (
              <button
                key={c.cmd}
                onMouseDown={(e) => { e.preventDefault(); applyCommand(c) }}
                className={clsx(
                  'w-full flex items-center gap-3 px-3 py-1.5 text-left text-xs transition-colors',
                  i === cmdIndex ? 'bg-[rgba(0,255,136,0.08)]' : 'hover:bg-white/5'
                )}
              >
                <span style={{ color: CATEGORY_COLOR[c.category], flexShrink: 0 }}>{c.icon}</span>
                <span className="font-mono font-semibold w-24 flex-shrink-0" style={{ color: 'var(--color-accent)' }}>
                  {c.cmd}
                </span>
                <span style={{ color: 'var(--color-text-muted)' }}>{c.description}</span>
                {i === cmdIndex && (
                  <ChevronRight size={10} className="ml-auto flex-shrink-0" style={{ color: 'var(--color-accent)' }} />
                )}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* Terminal prompt input */}
      <div
        className="flex items-start gap-2 px-4 py-3 border-t"
        style={{ borderColor: 'var(--color-border)' }}
      >
        <span
          className="text-base leading-5 pt-0.5 flex-shrink-0 select-none"
          style={{ color: sending ? 'var(--color-text-muted)' : 'var(--color-accent)', fontWeight: 700 }}
        >
          {sending ? <span className="agent-spinner" /> : '❯'}
        </span>
        <textarea
          ref={inputRef}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKey}
          className="flex-1 bg-transparent border-0 outline-none resize-none text-sm leading-relaxed"
          style={{
            color: 'var(--color-text)',
            fontFamily: 'inherit',
            minHeight: '1.4rem',
            overflow: 'hidden',
          }}
          rows={1}
          placeholder={sending ? '' : 'type a command or ask the agent… (/ for commands)'}
          disabled={sending}
          autoFocus
        />
      </div>
    </div>
  )
}

// ── Session tab ─────────────────────────────────────────────────────

function SessionTab({
  session,
  active,
  canClose,
  onSelect,
  onClose,
  onRename,
}: {
  session: Session
  active: boolean
  canClose: boolean
  onSelect: () => void
  onClose: () => void
  onRename: (label: string) => void
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(session.label)
  const inputRef = useRef<HTMLInputElement>(null)

  function startEdit(e: React.MouseEvent) {
    e.stopPropagation()
    setDraft(session.label)
    setEditing(true)
    setTimeout(() => inputRef.current?.select(), 0)
  }

  function commitRename() {
    const trimmed = draft.trim()
    if (trimmed) onRename(trimmed)
    setEditing(false)
  }

  const streaming = session.messages.some((m) => m.streaming)

  return (
    <div
      className={clsx(
        'group flex items-center gap-1.5 px-3 py-2 border-r text-xs cursor-pointer flex-shrink-0 transition-colors font-mono',
        !active && 'hover:bg-white/5'
      )}
      style={{
        borderColor: 'var(--color-border)',
        borderBottom: active ? '1px solid var(--color-accent)' : '1px solid transparent',
        color: active ? 'var(--color-accent)' : 'var(--color-text-muted)',
      }}
      onClick={onSelect}
    >
      {streaming ? (
        <span className="agent-spinner" style={{ fontSize: '0.7rem' }} />
      ) : (
        <span style={{ opacity: 0.5 }}>~</span>
      )}
      {editing ? (
        <input
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commitRename}
          onKeyDown={(e) => {
            if (e.key === 'Enter') commitRename()
            if (e.key === 'Escape') setEditing(false)
            e.stopPropagation()
          }}
          onClick={(e) => e.stopPropagation()}
          className="w-20 bg-transparent border-b outline-none text-xs"
          style={{ borderColor: 'var(--color-accent)', color: 'var(--color-accent)' }}
        />
      ) : (
        <span>{session.label}</span>
      )}
      {active && !editing && (
        <button
          onClick={startEdit}
          className="opacity-0 group-hover:opacity-100 p-0.5 transition-all"
          title="Rename"
        >
          <Pencil size={9} />
        </button>
      )}
      {canClose && (
        <button
          onClick={(e) => { e.stopPropagation(); onClose() }}
          className="opacity-0 group-hover:opacity-100 p-0.5 transition-all ml-1"
        >
          <X size={10} />
        </button>
      )}
    </div>
  )
}

// ── Helpers ─────────────────────────────────────────────────────────

function truncate(s: string, max: number): string {
  return s.length <= max ? s : s.slice(0, max) + '…'
}

function summariseArgs(args: unknown): string {
  if (!args || typeof args !== 'object') return ''
  const obj = args as Record<string, unknown>
  const first = Object.values(obj)[0]
  if (typeof first === 'string') return truncate(first, 60)
  return ''
}

// ── Persistence ─────────────────────────────────────────────────────

const STORAGE_KEY = 'traderclaw_chat_sessions'
const ACTIVE_KEY = 'traderclaw_chat_active'

function loadSessions(): Session[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as Session[]
      if (Array.isArray(parsed) && parsed.length > 0) {
        return parsed.map((s) => ({
          ...s,
          messages: s.messages.map((m) => ({ ...m, streaming: false })),
        }))
      }
    }
  } catch {
    // ignore
  }
  return [{ id: generateId(), label: 'session-1', messages: [] }]
}

function loadActiveId(sessions: Session[]): string {
  try {
    const saved = localStorage.getItem(ACTIVE_KEY)
    if (saved && sessions.some((s) => s.id === saved)) return saved
  } catch {
    // ignore
  }
  return sessions[0].id
}

// ── Root component ───────────────────────────────────────────────────

export default function Chat() {
  const initialSessions = loadSessions()
  const [sessions, setSessions] = useState<Session[]>(initialSessions)
  const [activeId, setActiveId] = useState<string>(() => loadActiveId(initialSessions))

  const { connected } = useWebSocket('/ws/chat', { autoReconnect: true })

  useEffect(() => {
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify(sessions)) } catch { /* quota */ }
  }, [sessions])

  useEffect(() => {
    try { localStorage.setItem(ACTIVE_KEY, activeId) } catch { /* quota */ }
  }, [activeId])

  function addSession() {
    const id = generateId()
    const n = sessions.length + 1
    setSessions((s) => [...s, { id, label: `session-${n}`, messages: [] }])
    setActiveId(id)
  }

  function removeSession(id: string) {
    if (sessions.length === 1) return
    const next = sessions.filter((s) => s.id !== id)
    setSessions(next)
    if (activeId === id) setActiveId(next[next.length - 1].id)
  }

  const updateSession = useCallback((updated: Session) => {
    setSessions((s) => s.map((session) => session.id === updated.id ? updated : session))
  }, [])

  function renameSession(id: string, label: string) {
    setSessions((s) => s.map((session) => session.id === id ? { ...session, label } : session))
  }

  return (
    <div className="flex flex-col h-full font-mono">
      {/* Terminal title bar */}
      <div
        className="flex items-center border-b overflow-x-auto flex-shrink-0"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        {/* Session tabs */}
        <div className="flex items-center flex-1 overflow-x-auto">
          {sessions.map((session) => (
            <SessionTab
              key={session.id}
              session={session}
              active={session.id === activeId}
              canClose={sessions.length > 1}
              onSelect={() => setActiveId(session.id)}
              onClose={() => removeSession(session.id)}
              onRename={(label) => renameSession(session.id, label)}
            />
          ))}
          <button
            onClick={addSession}
            className="px-2.5 py-2 flex-shrink-0 hover:bg-white/5 transition-colors"
            style={{ color: 'var(--color-text-muted)' }}
            title="New session"
          >
            <Plus size={12} />
          </button>
        </div>

        {/* Status right side */}
        <div className="px-3 flex items-center gap-2 flex-shrink-0 text-xs" style={{ color: 'var(--color-text-muted)' }}>
          <span className="flex items-center gap-1">
            {connected
              ? <Wifi size={11} style={{ color: 'var(--color-accent)' }} />
              : <WifiOff size={11} />
            }
            <span style={{ color: connected ? 'var(--color-accent)' : 'var(--color-text-muted)' }}>
              {connected ? 'ws' : 'http'}
            </span>
          </span>
        </div>
      </div>

      {/* All chat windows — ALL mounted so WebSocket stays alive */}
      <div className="flex-1 overflow-hidden relative">
        {sessions.map((session) => (
          <div key={session.id} className="absolute inset-0">
            <ChatWindow
              session={session}
              onUpdate={updateSession}
              visible={session.id === activeId}
            />
          </div>
        ))}
      </div>
    </div>
  )
}
