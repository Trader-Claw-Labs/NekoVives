import { useState, useRef, useEffect } from 'react'
import {
  Plus, X, Wifi, WifiOff, Pencil, Send,
  Zap, BarChart2, Search, Wallet, BookOpen, Settings2, Terminal,
  ChevronDown, ChevronRight, AlertCircle, Paperclip, FlaskConical,
} from 'lucide-react'
import type { WsMessage } from '../hooks/useWebSocket'
import { apiPost } from '../hooks/useApi'
import clsx from 'clsx'
import { useChatContext } from '../context/ChatContext'
import type { Message, Session, ToolEvent } from '../context/ChatContext'
import { useBacktestState } from '../hooks/useBacktestState'
import type { BacktestResult } from '../hooks/useBacktestState'

// ── Slash commands ──────────────────────────────────────────────────

interface SlashCommand {
  cmd: string
  description: string
  prompt: string
  icon: React.ReactNode
  category: 'strategy' | 'analysis' | 'wallet' | 'system'
}

const SLASH_COMMANDS: SlashCommand[] = [
  { cmd: '/strategy', description: 'Generate a new Rhai trading strategy', prompt: 'Create a Rhai trading strategy that ', icon: <Zap size={13} />, category: 'strategy' },
  { cmd: '/backtest', description: 'Run a backtest on a strategy', prompt: 'Run a backtest on the strategy at scripts/', icon: <BarChart2 size={13} />, category: 'strategy' },
  { cmd: '/scan', description: 'Scan markets for opportunities', prompt: 'Scan the top crypto markets for trading opportunities using the TradingView screener', icon: <Search size={13} />, category: 'analysis' },
  { cmd: '/rsi', description: 'Check RSI for a symbol', prompt: 'What is the current RSI for ', icon: <BarChart2 size={13} />, category: 'analysis' },
  { cmd: '/buy', description: 'Execute a buy order', prompt: 'Buy ', icon: <Zap size={13} />, category: 'strategy' },
  { cmd: '/sell', description: 'Execute a sell order', prompt: 'Sell ', icon: <Zap size={13} />, category: 'strategy' },
  { cmd: '/wallets', description: 'List connected wallets and balances', prompt: 'Show me all connected wallets and their balances', icon: <Wallet size={13} />, category: 'wallet' },
  { cmd: '/polymarket', description: 'Browse Polymarket prediction markets', prompt: 'Show me the top Polymarket prediction markets by volume', icon: <BarChart2 size={13} />, category: 'analysis' },
  { cmd: '/memory', description: 'Show what the agent remembers', prompt: 'What do you remember from our previous conversations?', icon: <BookOpen size={13} />, category: 'system' },
  { cmd: '/status', description: 'Show system status and health', prompt: 'What is the current system status? Show provider, model, and component health.', icon: <Settings2 size={13} />, category: 'system' },
  { cmd: '/scripts', description: 'List available strategy scripts', prompt: 'List all available .rhai strategy scripts in the scripts directory', icon: <Terminal size={13} />, category: 'strategy' },
  { cmd: '/help', description: 'Show all available agent capabilities', prompt: 'What can you do? List all your capabilities and available tools.', icon: <BookOpen size={13} />, category: 'system' },
]

const CATEGORY_COLOR: Record<SlashCommand['category'], string> = {
  strategy: 'var(--color-accent)',
  analysis: '#60a5fa',
  wallet:   '#f59e0b',
  system:   'var(--color-text-muted)',
}

// ── Quick-start suggestions ──────────────────────────────────────────

const QUICK_PROMPTS = [
  { label: 'Generate RSI strategy', prompt: 'Create a Rhai trading strategy that buys BTC when RSI < 30 and sells when RSI > 70. Save it to scripts/rsi_btc.rhai', icon: <Zap size={15} /> },
  { label: 'Scan top markets', prompt: 'Scan the top 10 crypto markets for trading opportunities using RSI and MACD indicators', icon: <Search size={15} /> },
  { label: 'Check wallet balances', prompt: 'Show me all connected wallets and their balances', icon: <Wallet size={15} /> },
  { label: 'Browse Polymarket', prompt: 'Show me the top 5 Polymarket prediction markets by volume with current prices', icon: <BarChart2 size={15} /> },
]

// ── Local helpers ────────────────────────────────────────────────────

function generateId(): string {
  return Math.random().toString(36).slice(2, 10)
}

// ── Tool metadata ────────────────────────────────────────────────────

interface ToolMeta {
  label: string
  done: string
  icon: string
  friendly: (snippet: string) => string
}

const TOOL_META: Record<string, ToolMeta> = {
  shell:              { label: 'Running a command',            done: 'Command executed',         icon: '⚡', friendly: (s) => s || 'Done' },
  bash:               { label: 'Running a command',            done: 'Command executed',         icon: '⚡', friendly: (s) => s || 'Done' },
  file_read:          { label: 'Reading a file',               done: 'File read',                icon: '📄', friendly: (s) => s || 'Done' },
  read_file:          { label: 'Reading a file',               done: 'File read',                icon: '📄', friendly: (s) => s || 'Done' },
  file_write:         { label: 'Saving a file',                done: 'File saved',               icon: '💾', friendly: () => 'Saved successfully' },
  write_file:         { label: 'Saving a file',                done: 'File saved',               icon: '💾', friendly: () => 'Saved successfully' },
  file_edit:          { label: 'Editing a file',               done: 'File updated',             icon: '✏️',  friendly: () => 'Changes applied' },
  glob_search:        { label: 'Looking for files',            done: 'Files found',              icon: '🔍', friendly: (s) => s || 'Search complete' },
  content_search:     { label: 'Searching file contents',      done: 'Search complete',          icon: '🔍', friendly: (s) => s || 'Done' },
  web_fetch:          { label: 'Loading a web page',           done: 'Page loaded',              icon: '🌐', friendly: (s) => s || 'Done' },
  web_search:         { label: 'Searching the web',            done: 'Web search done',          icon: '🌐', friendly: (s) => s || 'Results retrieved' },
  wallet_balance:     { label: 'Checking your wallet',         done: 'Wallet checked',           icon: '💰', friendly: (s) => s || 'Balance retrieved' },
  trade_swap:         { label: 'Getting a swap quote',         done: 'Quote received',           icon: '🔄', friendly: (s) => s || 'Quote ready' },
  market_scan:        { label: 'Scanning markets',             done: 'Markets scanned',          icon: '📈', friendly: (s) => s || 'Scan complete' },
  tradingview_scan:   { label: 'Fetching market indicators',   done: 'Indicators fetched',       icon: '📈', friendly: (s) => s || 'Done' },
  backtest_run:       { label: 'Running your backtest',        done: 'Backtest complete',        icon: '🧪', friendly: (s) => s || 'Results ready' },
  backtest_list:      { label: 'Loading saved strategies',     done: 'Strategies loaded',        icon: '📋', friendly: (s) => s || 'Done' },
  polymarket_markets: { label: 'Browsing Polymarket',          done: 'Markets loaded',           icon: '🏛️', friendly: (s) => s || 'Markets retrieved' },
  polymarket_buy:     { label: 'Placing a buy order',          done: 'Order placed',             icon: '🟢', friendly: (s) => s || 'Done' },
  polymarket_sell:    { label: 'Placing a sell order',         done: 'Order placed',             icon: '🔴', friendly: (s) => s || 'Done' },
  memory_store:       { label: 'Saving to memory',             done: 'Remembered',               icon: '🧠', friendly: () => 'Saved for next time' },
  memory_recall:      { label: 'Searching memory',             done: 'Memory searched',          icon: '🧠', friendly: (s) => s || 'Nothing found' },
  memory_forget:      { label: 'Clearing memory',              done: 'Memory cleared',           icon: '🧠', friendly: () => 'Entry removed' },
  cron_add:           { label: 'Scheduling a strategy',        done: 'Strategy scheduled',       icon: '⏰', friendly: (s) => s || 'Added to schedule' },
  cron_list:          { label: 'Loading scheduled strategies', done: 'Schedule loaded',          icon: '⏰', friendly: (s) => s || 'Done' },
  cron_remove:        { label: 'Removing a scheduled task',    done: 'Task removed',             icon: '⏰', friendly: () => 'Removed from schedule' },
  screenshot:         { label: 'Taking a screenshot',          done: 'Screenshot taken',         icon: '🖼️', friendly: () => 'Screenshot captured' },
  image_info:         { label: 'Reading image data',           done: 'Image analyzed',           icon: '🖼️', friendly: (s) => s || 'Done' },
}

function toolMeta(name: string): ToolMeta {
  const key = name.toLowerCase().replace(/[^a-z_]/g, '_')
  for (const [k, v] of Object.entries(TOOL_META)) {
    if (key.includes(k)) return v
  }
  return { label: `Using ${name}`, done: name, icon: '⚙️', friendly: (s) => s || 'Done' }
}


// ── Timeline (Claude Code style) ─────────────────────────────────────
// Used both while streaming (live) and collapsed/expanded after done.

interface TimelineItem {
  kind: 'thinking' | 'tool'
  // thinking
  round?: number
  replanning?: boolean
  thinkingElapsed?: number   // seconds spent in this thinking phase (set when next event arrives)
  // tool
  toolName?: string
  args?: Record<string, unknown>
  outputSnippet?: string
  success?: boolean
  elapsedMs?: number
  active?: boolean           // currently running (no result yet)
}

function buildTimeline(toolEvents: ToolEvent[], streaming: boolean): TimelineItem[] {
  const items: TimelineItem[] = []
  const thinkingEvents = toolEvents.filter(e => e.type === 'thinking')
  const calls          = toolEvents.filter(e => e.type === 'tool_call')
  const results        = toolEvents.filter(e => e.type === 'tool_result')

  // Interleave thinking rounds and tool calls in chronological order.
  // Each "thinking" event starts a new round; tool calls follow.
  let callIdx = 0
  let resultIdx = 0

  for (let ti = 0; ti < thinkingEvents.length; ti++) {
    const t = thinkingEvents[ti]
    const round = t.iteration ?? 1

    // Find tool calls that belong to this round
    const roundCalls: ToolEvent[] = []
    while (callIdx < calls.length && (calls[callIdx].iteration ?? 1) === round) {
      roundCalls.push(calls[callIdx++])
    }

    // Compute elapsed in this thinking phase (time until first tool call of this round, or still running)
    items.push({ kind: 'thinking', round, replanning: t.replanning })

    for (const call of roundCalls) {
      const result = results[resultIdx]
      const hasResult = result !== undefined
      if (hasResult) resultIdx++
      items.push({
        kind: 'tool',
        toolName: call.name,
        args: call.args,
        outputSnippet: hasResult ? result.outputSnippet : undefined,
        success: hasResult ? result.success : undefined,
        elapsedMs: hasResult ? result.elapsedMs : undefined,
        active: !hasResult && streaming,
      })
    }
  }

  // Any tool calls that arrived before the first thinking event (shouldn't happen but defensive)
  while (callIdx < calls.length) {
    const call = calls[callIdx++]
    const result = results[resultIdx]
    const hasResult = result !== undefined
    if (hasResult) resultIdx++
    items.push({
      kind: 'tool',
      toolName: call.name,
      args: call.args,
      outputSnippet: hasResult ? result.outputSnippet : undefined,
      success: hasResult ? result.success : undefined,
      elapsedMs: hasResult ? result.elapsedMs : undefined,
      active: !hasResult && streaming,
    })
  }

  return items
}

function ThinkingRow({ round, replanning, active }: { round: number; replanning?: boolean; active: boolean }) {
  const [secs, setSecs] = useState(0)
  useEffect(() => {
    if (!active) return
    const t = setInterval(() => setSecs(s => s + 1), 1000)
    return () => clearInterval(t)
  }, [active])

  const label = replanning
    ? round === 2 ? 'Reviewing results' : `Re-planning (round ${round})`
    : 'Thinking'
  const timeStr = secs > 0 ? ` for ${secs}s` : ''

  return (
    <div className="flex items-center gap-3" style={{ minHeight: 24 }}>
      {/* Dot / spinner */}
      <div className="flex-shrink-0 flex items-center justify-center" style={{ width: 18 }}>
        {active
          ? <span className="agent-spinner" style={{ width: 8, height: 8 }} />
          : <span style={{ width: 8, height: 8, borderRadius: '50%', background: 'rgba(255,255,255,0.2)', display: 'inline-block' }} />
        }
      </div>
      <span className="text-sm" style={{ color: active ? 'var(--color-text)' : 'var(--color-text-muted)', opacity: active ? 1 : 0.5 }}>
        {label}{active ? timeStr : ''}
        {!active && <span style={{ opacity: 0.35 }}>{timeStr}</span>}
      </span>
    </div>
  )
}

function ToolRow({ item }: { item: TimelineItem }) {
  const [open, setOpen] = useState(false)
  const meta    = toolMeta(item.toolName ?? '')
  const label   = argsLabel(item.toolName ?? '', item.args)
  const failed  = item.success === false
  const elapsed = item.elapsedMs
  const elapsedStr = elapsed !== undefined && elapsed > 0
    ? elapsed < 1000 ? `${elapsed}ms` : `${(elapsed / 1000).toFixed(1)}s`
    : null
  const hasOutput = Boolean(item.outputSnippet)
  const canExpand = hasOutput || Boolean(label)

  return (
    <div>
      <button
        onClick={() => canExpand && setOpen(v => !v)}
        style={{
          display: 'flex', alignItems: 'center', gap: 12, width: '100%',
          background: 'none', border: 'none', padding: '3px 0',
          cursor: canExpand ? 'pointer' : 'default', textAlign: 'left',
          minHeight: 24,
        }}
      >
        {/* Dot / spinner */}
        <div className="flex-shrink-0 flex items-center justify-center" style={{ width: 18 }}>
          {item.active
            ? <span className="agent-spinner" style={{ width: 8, height: 8 }} />
            : <span style={{
                width: 8, height: 8, borderRadius: '50%', display: 'inline-block',
                background: failed ? 'var(--color-danger)' : 'var(--color-accent)',
                opacity: failed ? 0.8 : 0.6,
              }} />
          }
        </div>

        {/* Tool name — bold, like "Bash" or "Read" in Claude Code */}
        <span className="text-sm font-semibold flex-shrink-0"
          style={{ color: item.active ? 'var(--color-accent)' : failed ? 'var(--color-danger)' : 'var(--color-text)', minWidth: 80 }}>
          {meta.icon} {item.toolName}
        </span>

        {/* Key arg — description column */}
        {label && (
          <span className="text-sm truncate flex-1"
            style={{ color: 'var(--color-text-muted)', opacity: 0.65, fontFamily: 'inherit' }}>
            {label}
          </span>
        )}

        {/* Timing + chevron on the right */}
        <span className="flex items-center gap-1.5 ml-auto flex-shrink-0">
          {elapsedStr && (
            <span className="text-xs tabular-nums" style={{ color: 'var(--color-text-muted)', opacity: 0.35 }}>
              {elapsedStr}
            </span>
          )}
          {canExpand && !item.active && (
            open
              ? <ChevronDown size={11} style={{ color: 'var(--color-text-muted)', opacity: 0.4 }} />
              : <ChevronRight size={11} style={{ color: 'var(--color-text-muted)', opacity: 0.4 }} />
          )}
        </span>
      </button>

      {/* Expanded IN/OUT panel */}
      {open && (
        <div className="flex flex-col gap-1.5 mt-1 mb-1 ml-8">
          {label && (
            <div>
              <div className="text-xs mb-0.5" style={{ color: 'var(--color-text-muted)', opacity: 0.4, fontWeight: 600, letterSpacing: '0.05em' }}>IN</div>
              <div className="text-xs font-mono px-2 py-1.5 rounded"
                style={{
                  background: 'rgba(255,255,255,0.04)',
                  color: 'var(--color-text)',
                  border: '1px solid rgba(255,255,255,0.06)',
                  overflowX: 'auto', whiteSpace: 'pre',
                }}>
                {label}
              </div>
            </div>
          )}
          {hasOutput && (
            <div>
              <div className="text-xs mb-0.5" style={{ color: 'var(--color-text-muted)', opacity: 0.4, fontWeight: 600, letterSpacing: '0.05em' }}>OUT</div>
              <div className="text-xs font-mono px-2 py-1.5 rounded"
                style={{
                  background: failed ? 'rgba(239,68,68,0.06)' : 'rgba(255,255,255,0.025)',
                  color: failed ? 'var(--color-danger)' : 'var(--color-text-muted)',
                  border: failed ? '1px solid rgba(239,68,68,0.15)' : '1px solid rgba(255,255,255,0.04)',
                  whiteSpace: 'pre-wrap', wordBreak: 'break-all',
                  maxHeight: 160, overflowY: 'auto',
                }}>
                {item.outputSnippet}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

function Timeline({ toolEvents, streaming }: { toolEvents: ToolEvent[]; streaming: boolean }) {
  const items = buildTimeline(toolEvents, streaming)
  if (items.length === 0) {
    // Nothing yet — show a single thinking spinner
    return (
      <div className="flex items-center gap-3" style={{ minHeight: 24 }}>
        <div className="flex-shrink-0 flex items-center justify-center" style={{ width: 18 }}>
          <span className="agent-spinner" style={{ width: 8, height: 8 }} />
        </div>
        <ThinkingRow round={1} active={true} />
      </div>
    )
  }

  const isLastActive = streaming
  const lastThinkingIdx = items.reduce((acc, item, i) => item.kind === 'thinking' ? i : acc, -1)

  return (
    <div className="flex flex-col" style={{ gap: 2 }}>
      {items.map((item, i) => {
        if (item.kind === 'thinking') {
          // This thinking row is "active" if it's the last one and we're still streaming with no active tool
          const hasActiveTool = items.slice(i + 1).some(x => x.kind === 'tool' && x.active)
          const isActiveThinking = isLastActive && i === lastThinkingIdx && !hasActiveTool
          return (
            <ThinkingRow
              key={i}
              round={item.round ?? 1}
              replanning={item.replanning}
              active={isActiveThinking}
            />
          )
        }
        return <ToolRow key={i} item={item} />
      })}
    </div>
  )
}

// Keep backward-compat alias used in MessageBubble for completed messages
function ActionLog({ toolEvents }: { toolEvents: ToolEvent[] }) {
  const [open, setOpen] = useState(false)
  const calls = toolEvents.filter(e => e.type === 'tool_call')
  if (calls.length === 0) return null

  const rounds   = Math.max(...toolEvents.filter(e => e.type === 'thinking').map(e => e.iteration ?? 1), 1)
  const failures = toolEvents.filter(e => e.type === 'tool_result' && e.success === false).length
  const summary  = [
    `${calls.length} action${calls.length !== 1 ? 's' : ''}`,
    rounds > 1 ? `${rounds} rounds` : null,
    failures > 0 ? `${failures} failed` : null,
  ].filter(Boolean).join(' · ')

  return (
    <div className="mb-2">
      <button
        onClick={() => setOpen(v => !v)}
        className="flex items-center gap-1.5 text-xs hover:opacity-80 transition-opacity"
        style={{ background: 'none', border: 'none', padding: '2px 0', cursor: 'pointer',
          color: 'var(--color-text-muted)', opacity: 0.45 }}
      >
        {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
        <span>{summary}</span>
      </button>
      {open && (
        <div className="mt-2 pl-1">
          <Timeline toolEvents={toolEvents} streaming={false} />
        </div>
      )}
    </div>
  )
}

// ── Message bubble ───────────────────────────────────────────────────

function MessageBubble({ msg }: { msg: Message }) {
  const isUser   = msg.role === 'user'
  const isActive = msg.streaming && !msg.content   // still thinking, no text yet
  const hasError = !msg.streaming && msg.content.startsWith('error:')

  if (isUser) {
    return (
      <div className="flex justify-end px-4 py-1">
        <div
          className="max-w-[75%] rounded-2xl px-4 py-2.5 text-sm leading-relaxed whitespace-pre-wrap"
          style={{
            background: 'var(--color-accent)',
            color: '#000',
            fontWeight: 450,
            borderBottomRightRadius: 4,
          }}
        >
          {msg.content}
        </div>
      </div>
    )
  }

  // Assistant bubble
  return (
    <div className="flex gap-3 px-4 py-1">
      {/* Avatar */}
      <div
        className="flex-shrink-0 flex items-center justify-center rounded-full text-xs font-bold mt-0.5"
        style={{
          width: 28, height: 28,
          background: 'linear-gradient(135deg, var(--color-accent) 0%, #00b8ff 100%)',
          color: '#000',
          fontSize: '0.6rem',
          letterSpacing: '0.05em',
        }}
      >
        TC
      </div>

      {/* Content */}
      <div className="flex-1 min-w-0 pt-0.5">
        {isActive ? (
          // Agent working — show live timeline
          <Timeline toolEvents={msg.toolEvents ?? []} streaming={true} />
        ) : hasError ? (
          // Error state
          <div className="flex flex-col gap-2">
            {(msg.toolEvents?.filter(e => e.type === 'tool_call').length ?? 0) > 0 && (
              <ActionLog toolEvents={msg.toolEvents!} />
            )}
            <div className="flex items-start gap-2 rounded-xl px-3.5 py-2.5 text-sm"
              style={{ background: 'rgba(239,68,68,0.1)', border: '1px solid rgba(239,68,68,0.25)' }}>
              <AlertCircle size={15} style={{ color: 'var(--color-danger)', flexShrink: 0, marginTop: 1 }} />
              <span style={{ color: 'var(--color-danger)' }}>
                {msg.content.replace(/^error:\s*/i, '')}
              </span>
            </div>
          </div>
        ) : (
          // Normal response
          <div className="flex flex-col">
            {(msg.toolEvents?.filter(e => e.type === 'tool_call').length ?? 0) > 0 && (
              <ActionLog toolEvents={msg.toolEvents!} />
            )}
            <p className={clsx('text-sm leading-relaxed whitespace-pre-wrap mt-1',
              msg.streaming && 'typewriter-cursor')}
              style={{ color: 'var(--color-text)' }}>
              {msg.content}
            </p>
          </div>
        )}
      </div>
    </div>
  )
}

// ── Chat window ──────────────────────────────────────────────────────

interface ChatWindowProps {
  session: Session
  onUpdate: (session: Session) => void
  connected: boolean
  send: (msg: WsMessage) => void
}

// ── Backtest attachment helpers ──────────────────────────────────────

function formatBacktestAttachment(r: BacktestResult): string {
  return `\n\n---\n**Attached Backtest Result** — ${r.script} (${r.symbol})\n` +
    `- Return: ${r.total_return_pct >= 0 ? '+' : ''}${r.total_return_pct.toFixed(2)}%\n` +
    `- Sharpe: ${r.sharpe_ratio?.toFixed(2) ?? 'N/A'}\n` +
    `- Max Drawdown: ${r.max_drawdown_pct.toFixed(2)}%\n` +
    `- Win Rate: ${r.win_rate_pct.toFixed(1)}%\n` +
    `- Trades: ${r.total_trades}\n` +
    (r.analysis ? `- Analysis: ${r.analysis}\n` : '') +
    `---`
}

function ChatWindow({ session, onUpdate, connected, send }: ChatWindowProps) {
  const [input, setInput]       = useState('')
  const [cmdIndex, setCmdIndex] = useState(0)
  const [showAttachMenu, setShowAttachMenu] = useState(false)
  const [attachedResult, setAttachedResult] = useState<BacktestResult | null>(null)
  const bottomRef  = useRef<HTMLDivElement>(null)
  const inputRef   = useRef<HTMLTextAreaElement>(null)
  const { scriptResults, result: latestResult } = useBacktestState()

  // Derive sending from whether the last assistant message is still streaming
  const sending = session.messages.some((m) => m.streaming)

  const slashMatch  = input.match(/^(\/\S*)/)
  const slashQuery  = slashMatch ? slashMatch[1].toLowerCase() : null
  const filteredCmds = slashQuery
    ? SLASH_COMMANDS.filter(
        (c) => c.cmd.startsWith(slashQuery) || c.description.toLowerCase().includes(slashQuery.slice(1))
      )
    : []
  const showPalette = slashQuery !== null && filteredCmds.length > 0
  useEffect(() => { setCmdIndex(0) }, [slashQuery])

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [session.messages])

  useEffect(() => {
    const el = inputRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, 180)}px`
  }, [input])

  async function handleSend() {
    if (!input.trim() || sending) return
    const fullContent = input.trim() + (attachedResult ? formatBacktestAttachment(attachedResult) : '')
    const userMsg: Message      = { role: 'user',      content: fullContent, timestamp: Date.now() }
    const assistantMsg: Message = { role: 'assistant', content: '', timestamp: Date.now(),
      streaming: true, toolEvents: [], agentStartedAt: Date.now() }
    const updated: Session = { ...session, messages: [...session.messages, userMsg, assistantMsg] }
    onUpdate(updated)
    setInput('')
    setAttachedResult(null)

    if (connected) {
      send({ type: 'message', content: fullContent, session_id: session.id })
    } else {
      try {
        const res = await apiPost<{ response?: string; content?: string }>(
          '/api/chat', { session_id: session.id, message: fullContent }
        )
        const text = res.response ?? res.content ?? 'No response'
        onUpdate({ ...updated, messages: updated.messages.map((m) =>
          m.streaming ? { ...m, content: text, streaming: false } : m
        )})
      } catch (e) {
        onUpdate({ ...updated, messages: updated.messages.map((m) =>
          m.streaming ? { ...m, content: `error: ${String(e)}`, streaming: false } : m
        )})
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
      if (e.key === 'ArrowDown')  { e.preventDefault(); setCmdIndex((i) => Math.min(i + 1, filteredCmds.length - 1)); return }
      if (e.key === 'ArrowUp')    { e.preventDefault(); setCmdIndex((i) => Math.max(i - 1, 0)); return }
      if (e.key === 'Tab' || (e.key === 'Enter' && filteredCmds.length > 0)) { e.preventDefault(); applyCommand(filteredCmds[cmdIndex]); return }
      if (e.key === 'Escape')     { setInput(''); return }
    }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend() }
  }

  const isEmpty = session.messages.length === 0

  return (
    <div className="flex flex-col h-full" style={{ fontFamily: 'var(--font-sans, system-ui, sans-serif)' }}>

      {/* ── Message list ── */}
      <div className="flex-1 overflow-y-auto">
        {isEmpty ? (
          /* Empty state */
          <div className="flex flex-col items-center justify-center h-full gap-6 px-6 pb-16">
            <div className="text-center">
              <div
                className="inline-flex items-center justify-center rounded-2xl mb-4"
                style={{
                  width: 48, height: 48,
                  background: 'linear-gradient(135deg, var(--color-accent) 0%, #00b8ff 100%)',
                }}
              >
                <span style={{ fontSize: 22 }}>🤖</span>
              </div>
              <h2 className="text-lg font-semibold mb-1" style={{ color: 'var(--color-text)' }}>
                Trader Claw Agent
              </h2>
              <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
                Markets · Strategies · Wallets · Backtesting
              </p>
            </div>

            {/* Quick prompt grid */}
            <div className="grid grid-cols-2 gap-2 w-full" style={{ maxWidth: 480 }}>
              {QUICK_PROMPTS.map((p) => (
                <button
                  key={p.label}
                  onClick={() => { setInput(p.prompt); inputRef.current?.focus() }}
                  className="flex flex-col gap-1.5 text-left p-3 rounded-xl transition-colors hover:bg-white/10"
                  style={{
                    border: '1px solid rgba(255,255,255,0.08)',
                    background: 'rgba(255,255,255,0.03)',
                    color: 'var(--color-text)',
                  }}
                >
                  <span style={{ color: 'var(--color-accent)' }}>{p.icon}</span>
                  <span className="text-sm font-medium">{p.label}</span>
                </button>
              ))}
            </div>

            <p className="text-xs" style={{ color: 'var(--color-text-muted)', opacity: 0.5 }}>
              Type <kbd className="font-mono px-1 rounded" style={{ background: 'rgba(255,255,255,0.08)' }}>/</kbd> for commands
            </p>
          </div>
        ) : (
          /* Messages */
          <div className="flex flex-col gap-1 py-4 pb-2">
            {session.messages.map((msg, i) => (
              <MessageBubble key={i} msg={msg} />
            ))}
            <div ref={bottomRef} />
          </div>
        )}
      </div>

      {/* ── Slash-command palette ── */}
      {showPalette && (
        <div className="border-t" style={{ borderColor: 'var(--color-border)', background: 'var(--color-surface)' }}>
          <div className="px-4 py-1.5 flex items-center gap-2 text-xs" style={{ color: 'var(--color-text-muted)', opacity: 0.6 }}>
            <kbd className="font-mono">↑↓</kbd> navigate ·{' '}
            <kbd className="font-mono">tab/↵</kbd> select ·{' '}
            <kbd className="font-mono">esc</kbd> dismiss
          </div>
          <div className="max-h-52 overflow-y-auto">
            {filteredCmds.map((c, i) => (
              <button
                key={c.cmd}
                onMouseDown={(e) => { e.preventDefault(); applyCommand(c) }}
                className={clsx(
                  'w-full flex items-center gap-3 px-4 py-2 text-left text-sm transition-colors',
                  i === cmdIndex ? 'bg-white/8' : 'hover:bg-white/5'
                )}
              >
                <span style={{ color: CATEGORY_COLOR[c.category], flexShrink: 0 }}>{c.icon}</span>
                <span className="font-mono font-semibold w-24 flex-shrink-0 text-xs" style={{ color: 'var(--color-accent)' }}>
                  {c.cmd}
                </span>
                <span className="text-sm" style={{ color: 'var(--color-text-muted)' }}>{c.description}</span>
                {i === cmdIndex && (
                  <ChevronRight size={12} className="ml-auto flex-shrink-0" style={{ color: 'var(--color-accent)' }} />
                )}
              </button>
            ))}
          </div>
        </div>
      )}

      {/* ── Input bar ── */}
      <div className="px-4 pb-4 pt-2">
        {/* Attached backtest pill */}
        {attachedResult && (
          <div className="flex items-center gap-2 mb-2 px-1">
            <div className="flex items-center gap-1.5 text-xs px-2.5 py-1 rounded-full"
              style={{ backgroundColor: 'rgba(74,222,128,0.12)', color: 'var(--color-accent)', border: '1px solid rgba(74,222,128,0.25)' }}>
              <FlaskConical size={11} />
              <span>{attachedResult.script} · {attachedResult.total_return_pct >= 0 ? '+' : ''}{attachedResult.total_return_pct.toFixed(2)}%</span>
              <button onClick={() => setAttachedResult(null)} className="ml-1 hover:opacity-70">
                <X size={10} />
              </button>
            </div>
          </div>
        )}

        {/* Attach menu (backtest results picker) */}
        {showAttachMenu && (
          <div className="mb-2 rounded-xl border overflow-hidden"
            style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
            <div className="px-3 py-2 text-xs font-semibold border-b flex items-center gap-2"
              style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
              <FlaskConical size={11} style={{ color: 'var(--color-accent)' }} />
              Attach a backtest result
            </div>
            {latestResult || Object.keys(scriptResults).length > 0 ? (
              <div className="max-h-40 overflow-y-auto">
                {[...(latestResult ? [latestResult] : []),
                  ...Object.values(scriptResults).filter(r => r.script !== latestResult?.script)
                ].map(r => (
                  <button key={r.script} className="w-full flex items-center justify-between px-3 py-2 text-xs hover:bg-white/5 text-left"
                    onClick={() => { setAttachedResult(r); setShowAttachMenu(false) }}>
                    <span className="font-mono truncate max-w-48">{r.script}</span>
                    <span style={{ color: r.total_return_pct >= 0 ? 'var(--color-accent)' : 'var(--color-danger)', flexShrink: 0 }}>
                      {r.total_return_pct >= 0 ? '+' : ''}{r.total_return_pct.toFixed(2)}%
                    </span>
                  </button>
                ))}
              </div>
            ) : (
              <div className="px-3 py-3 text-xs" style={{ color: 'var(--color-text-muted)' }}>
                No backtest results yet — run a backtest first
              </div>
            )}
          </div>
        )}

        <div
          className="flex items-end gap-2 rounded-2xl px-4 py-3 transition-shadow"
          style={{
            background: 'var(--color-surface)',
            border: '1px solid rgba(255,255,255,0.1)',
            boxShadow: '0 0 0 0 transparent',
          }}
          onFocus={(e) => {
            const el = e.currentTarget as HTMLDivElement
            el.style.border = '1px solid rgba(255,255,255,0.22)'
          }}
          onBlur={(e) => {
            const el = e.currentTarget as HTMLDivElement
            el.style.border = '1px solid rgba(255,255,255,0.1)'
          }}
        >
          <button
            onClick={() => setShowAttachMenu(v => !v)}
            className="flex-shrink-0 flex items-center justify-center rounded-lg transition-colors hover:bg-white/10 mb-0.5"
            style={{
              width: 28, height: 28,
              color: showAttachMenu || attachedResult ? 'var(--color-accent)' : 'rgba(255,255,255,0.3)',
            }}
            title="Attach backtest result"
            type="button"
          >
            <Paperclip size={14} />
          </button>
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKey}
            className="flex-1 bg-transparent border-0 outline-none resize-none text-sm leading-relaxed"
            style={{
              color: 'var(--color-text)',
              minHeight: '1.4rem',
              overflow: 'hidden',
              fontFamily: 'inherit',
            }}
            rows={1}
            placeholder={sending ? 'Agent is working…' : 'Message the agent… (/ for commands)'}
            disabled={sending}
            autoFocus
          />
          <button
            onClick={handleSend}
            disabled={!input.trim() || sending}
            className="flex-shrink-0 flex items-center justify-center rounded-xl transition-all"
            style={{
              width: 32, height: 32,
              background: input.trim() && !sending ? 'var(--color-accent)' : 'rgba(255,255,255,0.08)',
              color: input.trim() && !sending ? '#000' : 'rgba(255,255,255,0.25)',
              cursor: input.trim() && !sending ? 'pointer' : 'not-allowed',
              transition: 'background 0.15s, color 0.15s',
            }}
            title="Send (Enter)"
          >
            {sending ? (
              <span className="agent-spinner" style={{ width: 12, height: 12 }} />
            ) : (
              <Send size={14} />
            )}
          </button>
        </div>
        <p className="text-center text-xs mt-1.5" style={{ color: 'var(--color-text-muted)', opacity: 0.3 }}>
          Enter to send · Shift+Enter for newline
        </p>
      </div>
    </div>
  )
}

// ── Session tab ──────────────────────────────────────────────────────

function SessionTab({
  session, active, canClose, onSelect, onClose, onRename,
}: {
  session: Session; active: boolean; canClose: boolean
  onSelect: () => void; onClose: () => void; onRename: (label: string) => void
}) {
  const [editing, setEditing]   = useState(false)
  const [draft, setDraft]       = useState(session.label)
  const inputRef = useRef<HTMLInputElement>(null)

  function startEdit(e: React.MouseEvent) {
    e.stopPropagation()
    setDraft(session.label)
    setEditing(true)
    setTimeout(() => inputRef.current?.select(), 0)
  }

  function commitRename() {
    const t = draft.trim()
    if (t) onRename(t)
    setEditing(false)
  }

  const streaming = session.messages.some((m) => m.streaming)

  return (
    <div
      className={clsx(
        'group flex items-center gap-1.5 px-3 py-2 border-r text-sm cursor-pointer flex-shrink-0 transition-colors',
        !active && 'hover:bg-white/5'
      )}
      style={{
        borderColor: 'var(--color-border)',
        borderBottom: active ? '1.5px solid var(--color-accent)' : '1.5px solid transparent',
        color: active ? 'var(--color-text)' : 'var(--color-text-muted)',
        fontWeight: active ? 500 : 400,
      }}
      onClick={onSelect}
    >
      {streaming ? (
        <span className="agent-spinner flex-shrink-0" style={{ width: 8, height: 8 }} />
      ) : (
        <span style={{ opacity: 0.3, fontSize: '0.6rem' }}>●</span>
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
          className="w-24 bg-transparent border-b outline-none text-sm"
          style={{ borderColor: 'var(--color-accent)', color: 'var(--color-accent)' }}
        />
      ) : (
        <span className="max-w-[96px] truncate">{session.label}</span>
      )}
      {active && !editing && (
        <button onClick={startEdit} className="opacity-0 group-hover:opacity-60 p-0.5 transition-all" title="Rename">
          <Pencil size={10} />
        </button>
      )}
      {canClose && (
        <button
          onClick={(e) => { e.stopPropagation(); onClose() }}
          className="opacity-0 group-hover:opacity-60 p-0.5 transition-all ml-0.5"
        >
          <X size={11} />
        </button>
      )}
    </div>
  )
}

// ── Helpers ──────────────────────────────────────────────────────────

function truncate(s: string, max: number): string {
  return s.length <= max ? s : s.slice(0, max) + '…'
}

/** Extract the most meaningful single argument for a tool call — the thing the user cares about. */
function argsLabel(toolName: string, args?: Record<string, unknown>): string | null {
  if (!args) return null
  const s = (key: string) => {
    const v = args[key]
    return typeof v === 'string' && v.trim() ? v.trim() : null
  }
  const name = toolName.toLowerCase()
  // Shell: show the command
  if (name === 'shell' || name === 'bash') return s('command') ?? s('cmd')
  // File ops: show path
  if (name.startsWith('file_') || name === 'pdf_read') return s('path') ?? s('file')
  // Search: show pattern or query
  if (name === 'glob_search') return s('pattern') ?? s('glob')
  if (name === 'content_search') return s('query') ?? s('pattern') ?? s('text')
  if (name === 'web_search') return s('query') ?? s('q')
  if (name === 'web_fetch') return s('url') ?? s('uri')
  if (name === 'http_request') return `${s('method') ?? 'GET'} ${s('url') ?? ''}`
  // Backtest
  if (name === 'backtest_run') return s('script') ?? s('path') ?? s('strategy')
  // Memory
  if (name === 'memory_store') return s('content') ? truncate(s('content')!, 60) : null
  if (name === 'memory_recall') return s('query')
  // Cron
  if (name === 'cron_add') return s('name') ?? s('command') ?? s('schedule')
  if (name === 'cron_remove') return s('name') ?? s('id')
  // Market
  if (name === 'market_scan') return s('symbol') ?? s('symbols') ?? s('market')
  // Wallet
  if (name === 'wallet_balance') return s('address') ?? s('chain') ?? s('network')
  if (name === 'trade_swap') return s('from_token') && s('to_token')
    ? `${s('from_token')} → ${s('to_token')}`
    : s('symbol')
  // Fallback: first string value
  const first = Object.values(args).find((v) => typeof v === 'string' && (v as string).trim())
  return first ? truncate(first as string, 80) : null
}

// ── Root ─────────────────────────────────────────────────────────────

export default function Chat() {
  const {
    sessions, activeId, connected,
    setActiveId, updateSession, addSession, removeSession, renameSession, send,
  } = useChatContext()

  const activeSession = sessions.find((s) => s.id === activeId) ?? sessions[0]

  return (
    <div className="flex flex-col h-full">
      {/* ── Tab bar ── */}
      <div
        className="flex items-center border-b flex-shrink-0 overflow-x-auto"
        style={{ background: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
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
            className="px-3 py-2 flex-shrink-0 hover:bg-white/5 transition-colors"
            style={{ color: 'var(--color-text-muted)' }}
            title="New chat"
          >
            <Plus size={13} />
          </button>
        </div>

        {/* Connection status */}
        <div className="px-3 flex items-center gap-1.5 flex-shrink-0 text-xs"
          style={{ color: connected ? 'var(--color-accent)' : 'var(--color-text-muted)' }}>
          {connected
            ? <Wifi size={11} style={{ color: 'var(--color-accent)' }} />
            : <WifiOff size={11} />}
          <span style={{ opacity: 0.6 }}>{connected ? 'live' : 'offline'}</span>
        </div>
      </div>

      {/* ── Active chat window ── */}
      <div className="flex-1 overflow-hidden">
        <ChatWindow
          key={activeSession.id}
          session={activeSession}
          onUpdate={updateSession}
          connected={connected}
          send={send}
        />
      </div>
    </div>
  )
}
