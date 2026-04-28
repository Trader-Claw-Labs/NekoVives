import { useState, useEffect, useRef } from 'react'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import { FileText, RefreshCw, Download, Trash2 } from 'lucide-react'

interface LogsResponse {
  lines: string[]
  file: string
}

export default function Logs() {
  const [autoScroll, setAutoScroll] = useState(true)
  const scrollRef = useRef<HTMLDivElement>(null)

  const { data, isLoading, refetch } = useQuery<LogsResponse>({
    queryKey: ['logs'],
    queryFn: () => apiFetch('/api/logs'),
    refetchInterval: 3_000,
  })

  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [data?.lines, autoScroll])

  const lines = data?.lines ?? []

  function handleDownload() {
    const blob = new Blob([lines.join('\n')], { type: 'text/plain' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `gateway-log-${new Date().toISOString().slice(0, 10)}.txt`
    a.click()
    URL.revokeObjectURL(url)
  }

  function handleClear() {
    // Clearing is not supported server-side; just a client-side convenience
    if (confirm('Clear log view? (This only clears the display, not the file)')) {
      // No-op — will refresh on next poll
    }
  }

  return (
    <div className="p-6 max-w-5xl mx-auto h-full flex flex-col">
      <div className="flex items-center justify-between mb-4 flex-shrink-0">
        <div className="flex items-center gap-2">
          <FileText size={18} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-lg font-bold">Gateway Log</h1>
          {data?.file && (
            <span className="text-xs font-mono truncate max-w-[300px]" style={{ color: 'var(--color-text-muted)' }}>
              {data.file}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1.5 text-xs cursor-pointer select-none" style={{ color: 'var(--color-text-muted)' }}>
            <input
              type="checkbox"
              checked={autoScroll}
              onChange={(e) => setAutoScroll(e.target.checked)}
              className="rounded"
            />
            Auto-scroll
          </label>
          <button
            onClick={handleDownload}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs border hover:bg-white/5 transition-colors"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
            title="Download log"
          >
            <Download size={12} />
            Export
          </button>
          <button
            onClick={() => refetch()}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs border hover:bg-white/5 transition-colors"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
          >
            <RefreshCw size={12} className={isLoading ? 'animate-spin' : ''} />
            Refresh
          </button>
        </div>
      </div>

      <div
        ref={scrollRef}
        className="flex-1 min-h-0 rounded border overflow-auto font-mono text-xs p-3"
        style={{
          backgroundColor: 'var(--color-base)',
          borderColor: 'var(--color-border)',
          color: 'var(--color-text)',
          lineHeight: '1.6',
        }}
        onScroll={() => {
          if (!scrollRef.current) return
          const el = scrollRef.current
          const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 50
          setAutoScroll(nearBottom)
        }}
      >
        {lines.length === 0 ? (
          <p style={{ color: 'var(--color-text-muted)' }}>No log lines available yet.</p>
        ) : (
          lines.map((line, i) => {
            // Color-code log levels
            const levelColor = line.includes(' ERROR ') || line.includes(' error ')
              ? 'var(--color-danger)'
              : line.includes(' WARN ') || line.includes(' warn ')
                ? 'var(--color-warning)'
                : line.includes(' INFO ') || line.includes(' info ')
                  ? 'var(--color-accent)'
                  : 'var(--color-text-muted)'
            return (
              <div key={i} className="whitespace-pre-wrap break-all" style={{ color: levelColor }}>
                {line}
              </div>
            )
          })
        )}
      </div>

      <div className="mt-2 text-xs flex items-center justify-between" style={{ color: 'var(--color-text-muted)' }}>
        <span>{lines.length} lines · refreshes every 3s</span>
        <span>Last updated: {new Date().toLocaleTimeString()}</span>
      </div>
    </div>
  )
}
