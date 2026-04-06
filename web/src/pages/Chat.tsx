import { useState, useRef, useEffect } from 'react'
import { MessageSquare, Plus, X, Send, Wifi, WifiOff, Pencil } from 'lucide-react'
import { useWebSocket } from '../hooks/useWebSocket'
import { apiPost } from '../hooks/useApi'
import clsx from 'clsx'

interface Message {
  role: 'user' | 'assistant'
  content: string
  timestamp: number
  streaming?: boolean
}

interface Session {
  id: string
  label: string
  messages: Message[]
}

function generateId(): string {
  return Math.random().toString(36).slice(2, 10)
}

function TypewriterText({ text, active }: { text: string; active: boolean }) {
  return (
    <span className={active ? 'typewriter-cursor' : ''}>
      {text}
    </span>
  )
}

interface ChatWindowProps {
  session: Session
  onUpdate: (session: Session) => void
  visible: boolean
}

function ChatWindow({ session, onUpdate, visible }: ChatWindowProps) {
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)
  // Use a ref so onMessage always sees latest session
  const sessionRef = useRef(session)
  useEffect(() => { sessionRef.current = session }, [session])

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
      } else if (msg.type === 'done') {
        onUpdate({
          ...cur,
          messages: cur.messages.map((m) =>
            m.streaming ? { ...m, streaming: false } : m
          ),
        })
        setSending(false)
      }
    },
  })

  useEffect(() => {
    if (visible) bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [session.messages, visible])

  async function handleSend() {
    if (!input.trim() || sending) return

    const userMsg: Message = { role: 'user', content: input.trim(), timestamp: Date.now() }
    const assistantMsg: Message = { role: 'assistant', content: '', timestamp: Date.now(), streaming: true }

    const updated: Session = {
      ...session,
      messages: [...session.messages, userMsg, assistantMsg],
    }
    onUpdate(updated)
    setSending(true)
    setInput('')

    if (connected) {
      send({ type: 'chat', content: input.trim(), session_id: session.id })
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
              ? { ...m, content: `Error: ${String(e)}`, streaming: false }
              : m
          ),
        })
      } finally {
        setSending(false)
      }
    }
  }

  function handleKey(e: React.KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  return (
    <div className="flex flex-col h-full" style={{ display: visible ? 'flex' : 'none' }}>
      {/* Messages */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {session.messages.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full">
            <MessageSquare size={40} style={{ color: 'var(--color-text-muted)' }} className="mb-3" />
            <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
              Start a conversation with the agent
            </p>
            <p className="text-xs mt-1" style={{ color: 'var(--color-text-muted)' }}>
              Use this session to test a specific trading strategy
            </p>
          </div>
        ) : (
          session.messages.map((msg, i) => (
            <div
              key={i}
              className={clsx('flex', msg.role === 'user' ? 'justify-end' : 'justify-start')}
            >
              <div
                className={clsx(
                  'max-w-[80%] rounded-lg px-4 py-3 text-sm',
                  msg.role === 'user' ? 'text-black' : ''
                )}
                style={
                  msg.role === 'user'
                    ? { backgroundColor: 'var(--color-accent)', color: '#000' }
                    : { backgroundColor: 'var(--color-surface-2)', border: '1px solid var(--color-border)' }
                }
              >
                <TypewriterText text={msg.content || (msg.streaming ? ' ' : '')} active={!!msg.streaming} />
              </div>
            </div>
          ))
        )}
        <div ref={bottomRef} />
      </div>

      {/* Input */}
      <div
        className="flex items-end gap-2 p-4 border-t"
        style={{ borderColor: 'var(--color-border)' }}
      >
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKey}
          className="flex-1 rounded px-3 py-2 text-sm resize-none"
          rows={2}
          placeholder="Type a message... (Enter to send, Shift+Enter for newline)"
          disabled={sending}
        />
        <button
          onClick={handleSend}
          disabled={!input.trim() || sending}
          className="p-2.5 rounded disabled:opacity-50 transition-colors"
          style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
        >
          <Send size={16} />
        </button>
      </div>
    </div>
  )
}

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
        'group flex items-center gap-2 px-3 py-2.5 border-r text-sm cursor-pointer flex-shrink-0 transition-colors',
        !active && 'hover:bg-white/5'
      )}
      style={{
        borderColor: 'var(--color-border)',
        borderBottom: active ? '2px solid var(--color-accent)' : '2px solid transparent',
        color: active ? 'var(--color-accent)' : 'var(--color-text-muted)',
      }}
      onClick={onSelect}
    >
      <MessageSquare size={13} className={clsx(streaming && 'animate-pulse')} />
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
          className="w-20 bg-transparent border-b outline-none text-sm"
          style={{ borderColor: 'var(--color-accent)', color: 'var(--color-accent)' }}
        />
      ) : (
        <span>{session.label}</span>
      )}
      {streaming && (
        <span className="text-xs font-mono" style={{ color: 'var(--color-accent)' }}>▌</span>
      )}
      {active && !editing && (
        <button
          onClick={startEdit}
          className="opacity-0 group-hover:opacity-100 rounded hover:bg-white/10 p-0.5 transition-all"
          title="Rename"
        >
          <Pencil size={10} />
        </button>
      )}
      {canClose && (
        <button
          onClick={(e) => { e.stopPropagation(); onClose() }}
          className="opacity-0 group-hover:opacity-100 rounded hover:bg-white/10 p-0.5 transition-all ml-auto"
        >
          <X size={11} />
        </button>
      )}
    </div>
  )
}

export default function Chat() {
  const [sessions, setSessions] = useState<Session[]>([
    { id: generateId(), label: 'Strategy 1', messages: [] },
  ])
  const [activeId, setActiveId] = useState(sessions[0].id)

  const { connected } = useWebSocket('/ws/chat', { autoReconnect: true })

  function addSession() {
    const id = generateId()
    const label = `Strategy ${sessions.length + 1}`
    setSessions((s) => [...s, { id, label, messages: [] }])
    setActiveId(id)
  }

  function removeSession(id: string) {
    if (sessions.length === 1) return
    const next = sessions.filter((s) => s.id !== id)
    setSessions(next)
    if (activeId === id) setActiveId(next[next.length - 1].id)
  }

  function updateSession(updated: Session) {
    setSessions((s) => s.map((session) => session.id === updated.id ? updated : session))
  }

  function renameSession(id: string, label: string) {
    setSessions((s) => s.map((session) => session.id === id ? { ...session, label } : session))
  }

  return (
    <div className="flex flex-col h-full">
      {/* Tab bar */}
      <div
        className="flex items-center border-b overflow-x-auto"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
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
          className="px-3 py-2.5 flex-shrink-0 hover:bg-white/5 transition-colors"
          style={{ color: 'var(--color-text-muted)' }}
          title="New strategy chat"
        >
          <Plus size={15} />
        </button>
        <div className="ml-auto px-3 flex items-center gap-1.5">
          {connected
            ? <Wifi size={13} style={{ color: 'var(--color-accent)' }} />
            : <WifiOff size={13} style={{ color: 'var(--color-text-muted)' }} />
          }
          <span className="text-xs" style={{ color: connected ? 'var(--color-accent)' : 'var(--color-text-muted)' }}>
            {connected ? 'Live' : 'HTTP'}
          </span>
          <span className="text-xs ml-2" style={{ color: 'var(--color-text-muted)' }}>
            {sessions.length} session{sessions.length !== 1 ? 's' : ''}
          </span>
        </div>
      </div>

      {/* All chat windows — ALL mounted simultaneously so WebSocket stays alive per session */}
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
