/**
 * ChatContext — persists chat sessions and WebSocket connection across page
 * navigation. Lives above the router so it is never unmounted.
 */
import {
  createContext, useContext, useState, useRef, useEffect, useCallback,
  type ReactNode,
} from 'react'
import { useWebSocket, type WsMessage } from '../hooks/useWebSocket'

// ── Types ─────────────────────────────────────────────────────────────

export interface ToolEvent {
  type: 'tool_call' | 'tool_result' | 'thinking' | 'executing'
  name: string
  summary?: string
  args?: Record<string, unknown>
  outputSnippet?: string
  success?: boolean
  iteration?: number
  step?: number
  totalSteps?: number
  toolCount?: number
  tools?: string[]
  elapsedMs?: number
  replanning?: boolean
  toolsDone?: number
}

export interface Message {
  role: 'user' | 'assistant'
  content: string
  timestamp: number
  streaming?: boolean
  toolEvents?: ToolEvent[]
  agentStartedAt?: number
}

export interface Session {
  id: string
  label: string
  messages: Message[]
}

// ── Helpers ───────────────────────────────────────────────────────────

function generateId(): string {
  return Math.random().toString(36).slice(2, 10)
}

function truncate(s: string, max: number): string {
  return s.length <= max ? s : s.slice(0, max) + '…'
}

function summariseArgs(args: unknown): string {
  if (!args || typeof args !== 'object') return ''
  const obj = args as Record<string, unknown>
  const first = Object.values(obj)[0]
  if (typeof first === 'string') return truncate(first, 80)
  return ''
}

// ── Persistence ───────────────────────────────────────────────────────

const STORAGE_KEY = 'traderclaw_chat_sessions'
const ACTIVE_KEY  = 'traderclaw_chat_active'

function loadSessions(): Session[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as Session[]
      if (Array.isArray(parsed) && parsed.length > 0) {
        return parsed.map((s) => ({
          ...s,
          // Mark any in-flight message as interrupted on reload
          messages: s.messages.map((m) => ({
            ...m,
            streaming: false,
            content: m.streaming && !m.content
              ? 'error: Connection lost — please resend your message.'
              : m.content,
          })),
        }))
      }
    }
  } catch { /* ignore */ }
  return [{ id: generateId(), label: 'New chat', messages: [] }]
}

function loadActiveId(sessions: Session[]): string {
  try {
    const saved = localStorage.getItem(ACTIVE_KEY)
    if (saved && sessions.some((s) => s.id === saved)) return saved
  } catch { /* ignore */ }
  return sessions[0].id
}

// ── Context ───────────────────────────────────────────────────────────

interface ChatContextValue {
  sessions: Session[]
  activeId: string
  connected: boolean
  setActiveId: (id: string) => void
  updateSession: (session: Session) => void
  addSession: () => void
  removeSession: (id: string) => void
  renameSession: (id: string, label: string) => void
  send: (msg: WsMessage) => void
}

const ChatContext = createContext<ChatContextValue | null>(null)

export function useChatContext(): ChatContextValue {
  const ctx = useContext(ChatContext)
  if (!ctx) throw new Error('useChatContext must be used inside ChatProvider')
  return ctx
}

// ── Provider ──────────────────────────────────────────────────────────

export function ChatProvider({ children }: { children: ReactNode }) {
  const [sessions, setSessions] = useState<Session[]>(() => loadSessions())
  const [activeId, setActiveIdState] = useState<string>(() => loadActiveId(loadSessions()))

  // Keep stable refs so the WS callback never goes stale
  const sessionsRef = useRef(sessions)
  const activeIdRef = useRef(activeId)
  useEffect(() => { sessionsRef.current = sessions }, [sessions])
  useEffect(() => { activeIdRef.current = activeId }, [activeId])

  // ── Persist to localStorage ───────────────────────────────────────
  useEffect(() => {
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify(sessions)) } catch { /* quota */ }
  }, [sessions])
  useEffect(() => {
    try { localStorage.setItem(ACTIVE_KEY, activeId) } catch { /* quota */ }
  }, [activeId])

  // ── Session helpers ───────────────────────────────────────────────
  const updateSession = useCallback((session: Session) => {
    setSessions((prev) => prev.map((s) => s.id === session.id ? session : s))
  }, [])

  const setActiveId = useCallback((id: string) => {
    setActiveIdState(id)
  }, [])

  const addSession = useCallback(() => {
    const id = generateId()
    setSessions((prev) => {
      const n = prev.length + 1
      return [...prev, { id, label: `Chat ${n}`, messages: [] }]
    })
    setActiveIdState(id)
  }, [])

  const removeSession = useCallback((id: string) => {
    setSessions((prev) => {
      const next = prev.filter((s) => s.id !== id)
      if (next.length === 0) {
        const newId = generateId()
        setActiveIdState(newId)
        return [{ id: newId, label: 'New chat', messages: [] }]
      }
      setActiveIdState((cur) => {
        if (cur === id) return next[next.length - 1].id
        return cur
      })
      return next
    })
  }, [])

  const renameSession = useCallback((id: string, label: string) => {
    setSessions((prev) => prev.map((s) => s.id === id ? { ...s, label } : s))
  }, [])

  // ── WebSocket message handler ─────────────────────────────────────
  const onWsMessage = useCallback((msg: WsMessage) => {
    setSessions((prev) => {
      const targetId = activeIdRef.current
      return prev.map((s) => {
        if (s.id !== targetId) return s
        const msgs = s.messages
        const lastIdx = msgs.length - 1
        const last = msgs[lastIdx]
        if (!last || last.role !== 'assistant' || !last.streaming) return s

        if (msg.type === 'thinking') {
          const updated: Message = {
            ...last,
            toolEvents: [...(last.toolEvents ?? []), {
              type: 'thinking' as const,
              name: 'thinking',
              iteration: Number(msg.iteration ?? 1),
              replanning: Boolean(msg.replanning),
              toolsDone: Number(msg.tools_done ?? 0),
            }],
          }
          return { ...s, messages: [...msgs.slice(0, lastIdx), updated] }
        }

        if (msg.type === 'executing') {
          const updated: Message = {
            ...last,
            toolEvents: [...(last.toolEvents ?? []), {
              type: 'executing' as const,
              name: 'executing',
              iteration: Number(msg.iteration ?? 1),
              toolCount: Number(msg.tool_count ?? 0),
              tools: Array.isArray(msg.tools) ? (msg.tools as string[]) : [],
            }],
          }
          return { ...s, messages: [...msgs.slice(0, lastIdx), updated] }
        }

        if (msg.type === 'chunk' && typeof msg.content === 'string') {
          const updated: Message = { ...last, content: last.content + msg.content }
          return { ...s, messages: [...msgs.slice(0, lastIdx), updated] }
        }

        if (msg.type === 'tool_call') {
          const rawArgs = msg.args && typeof msg.args === 'object'
            ? msg.args as Record<string, unknown>
            : undefined
          const updated: Message = {
            ...last,
            toolEvents: [...(last.toolEvents ?? []), {
              type: 'tool_call' as const,
              name: String(msg.name ?? ''),
              summary: summariseArgs(msg.args),
              args: rawArgs,
              iteration: Number(msg.iteration ?? 1),
              step: Number(msg.step ?? 1),
              totalSteps: Number(msg.total_steps ?? 1),
            }],
          }
          return { ...s, messages: [...msgs.slice(0, lastIdx), updated] }
        }

        if (msg.type === 'tool_result') {
          const updated: Message = {
            ...last,
            toolEvents: [...(last.toolEvents ?? []), {
              type: 'tool_result' as const,
              name: String(msg.name ?? ''),
              outputSnippet: String(msg.output_snippet ?? ''),
              success: Boolean(msg.success ?? true),
              elapsedMs: Number(msg.elapsed_ms ?? 0),
              iteration: Number(msg.iteration ?? 1),
            }],
          }
          return { ...s, messages: [...msgs.slice(0, lastIdx), updated] }
        }

        if (msg.type === 'done') {
          const full = typeof msg.full_response === 'string' ? msg.full_response : undefined
          return {
            ...s,
            messages: msgs.map((m) =>
              m.streaming ? { ...m, content: full ?? m.content, streaming: false } : m
            ),
          }
        }

        if (msg.type === 'error') {
          return {
            ...s,
            messages: msgs.map((m) =>
              m.streaming
                ? { ...m, content: `error: ${msg.message ?? 'unknown error'}`, streaming: false }
                : m
            ),
          }
        }

        return s
      })
    })
  }, [])

  const { connected, send } = useWebSocket('/ws/chat', {
    autoReconnect: true,
    onMessage: onWsMessage,
  })

  const value: ChatContextValue = {
    sessions,
    activeId,
    connected,
    setActiveId,
    updateSession,
    addSession,
    removeSession,
    renameSession,
    send,
  }

  return <ChatContext.Provider value={value}>{children}</ChatContext.Provider>
}
