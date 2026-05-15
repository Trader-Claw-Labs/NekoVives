import { useState, useEffect, useRef, useCallback } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import { Settings, Save, RefreshCw, Copy, Check, Download, Upload } from 'lucide-react'
import { getAuthToken } from '../hooks/useApi'

interface ConfigResponse {
  format?: string
  content?: string
}

// ── TOML syntax highlighter ───────────────────────────────────────────────────

function highlightTomlValue(value: string): React.ReactNode {
  const trimmed = value.trimStart()

  // Inline comment at end (after value)
  const commentIdx = (() => {
    let inStr = false
    let strChar = ''
    for (let i = 0; i < trimmed.length; i++) {
      const c = trimmed[i]
      if (!inStr && (c === '"' || c === "'")) { inStr = true; strChar = c }
      else if (inStr && c === strChar && trimmed[i - 1] !== '\\') inStr = false
      else if (!inStr && c === '#') return i
    }
    return -1
  })()

  const raw = commentIdx >= 0 ? trimmed.slice(0, commentIdx).trimEnd() : trimmed
  const comment = commentIdx >= 0 ? trimmed.slice(commentIdx) : ''
  const leading = value.slice(0, value.length - trimmed.length)

  let valueNode: React.ReactNode
  // Quoted string (single or multi-line start)
  if (/^("""|'''|"|')/.test(raw)) {
    valueNode = <span style={{ color: '#86efac' }}>{raw}</span>
  // Array or inline table
  } else if (/^\[/.test(raw) || /^\{/.test(raw)) {
    valueNode = <span style={{ color: '#e2e8f0' }}>{raw}</span>
  // Boolean
  } else if (raw === 'true' || raw === 'false') {
    valueNode = <span style={{ color: '#f9a8d4' }}>{raw}</span>
  // Number (int, float, hex, special)
  } else if (/^-?(0x[\da-fA-F_]+|\d[\d_]*(\.\d+)?([eE][+-]?\d+)?|nan|inf)$/.test(raw)) {
    valueNode = <span style={{ color: '#fcd34d' }}>{raw}</span>
  } else {
    valueNode = <span>{raw}</span>
  }

  return (
    <>
      {leading}
      {valueNode}
      {comment && <span style={{ color: 'var(--color-text-muted)', opacity: 0.6 }}>{' ' + comment}</span>}
    </>
  )
}

function highlightTomlLine(line: string, idx: number): React.ReactNode {
  // Blank line
  if (!line.trim()) return <span key={idx}>{'\n'}</span>

  // Full-line comment
  if (/^\s*#/.test(line)) {
    return (
      <span key={idx} style={{ color: 'var(--color-text-muted)', opacity: 0.55 }}>
        {line}{'\n'}
      </span>
    )
  }

  // Section header [section] or [[array-of-tables]]
  const sectionMatch = line.match(/^(\s*)(\[{1,2})([^\]]+)(\]{1,2})(.*)$/)
  if (sectionMatch) {
    const [, indent, open, name, close, rest] = sectionMatch
    return (
      <span key={idx}>
        {indent}
        <span style={{ color: 'var(--color-text-muted)' }}>{open}</span>
        <span style={{ color: 'var(--color-accent)', fontWeight: 600 }}>{name}</span>
        <span style={{ color: 'var(--color-text-muted)' }}>{close}</span>
        {rest && <span style={{ color: 'var(--color-text-muted)', opacity: 0.55 }}>{rest}</span>}
        {'\n'}
      </span>
    )
  }

  // Key = value  (handles dotted keys and quoted keys)
  const kvMatch = line.match(/^(\s*)((?:"[^"]*"|'[^']*'|[\w.-]+)(?:\s*\.\s*(?:"[^"]*"|'[^']*'|[\w.-]+))*)(\s*=\s*)(.*)$/)
  if (kvMatch) {
    const [, indent, key, eq, value] = kvMatch
    return (
      <span key={idx}>
        {indent}
        <span style={{ color: '#7dd3fc' }}>{key}</span>
        <span style={{ color: 'var(--color-text-muted)' }}>{eq}</span>
        {highlightTomlValue(value)}
        {'\n'}
      </span>
    )
  }

  // Continuation / anything else
  return <span key={idx}>{line}{'\n'}</span>
}

function TomlHighlight({ content }: { content: string }) {
  const lines = content.split('\n')
  // Remove trailing empty line added by split
  if (lines[lines.length - 1] === '') lines.pop()
  return (
    <>
      {lines.map((line, i) => highlightTomlLine(line, i))}
    </>
  )
}

// ── Page ─────────────────────────────────────────────────────────────────────

export default function Config() {
  const [content, setContent] = useState('')
  const [saveMsg, setSaveMsg] = useState('')
  const [saveErr, setSaveErr] = useState('')
  const [copied, setCopied] = useState(false)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const preRef = useRef<HTMLPreElement>(null)

  const { data, isLoading, refetch } = useQuery<ConfigResponse>({
    queryKey: ['config'],
    queryFn: () => apiFetch('/api/config'),
  })

  useEffect(() => {
    if (data?.content) setContent(data.content)
  }, [data])

  // Keep pre and textarea scroll in sync
  const syncScroll = useCallback(() => {
    if (textareaRef.current && preRef.current) {
      preRef.current.scrollTop = textareaRef.current.scrollTop
      preRef.current.scrollLeft = textareaRef.current.scrollLeft
    }
  }, [])

  const saveMutation = useMutation({
    mutationFn: async () => {
      const res = await fetch('/api/config', {
        method: 'PUT',
        headers: {
          'Content-Type': 'text/plain',
          Authorization: `Bearer ${localStorage.getItem('auth_token') ?? ''}`,
        },
        body: content,
      })
      if (!res.ok) throw new Error(await res.text())
      return res.json()
    },
    onSuccess: () => {
      setSaveMsg('Config saved successfully!')
      setSaveErr('')
      setTimeout(() => setSaveMsg(''), 3000)
    },
    onError: (e: Error) => {
      setSaveErr(e.message)
      setSaveMsg('')
    },
  })

  function copy() {
    navigator.clipboard.writeText(content)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  const lineCount = content.split('\n').length

  const editorFontStyle: React.CSSProperties = {
    fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
    fontSize: '12px',
    lineHeight: '20px',
    tabSize: 2,
    whiteSpace: 'pre',
    overflowWrap: 'normal',
    wordBreak: 'normal',
  }

  return (
    <div className="p-6 max-w-4xl mx-auto">
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-2">
          <Settings size={18} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-lg font-bold">Config</h1>
          <span
            className="text-xs px-2 py-0.5 rounded"
            style={{ backgroundColor: 'var(--color-surface)', color: 'var(--color-text-muted)', border: '1px solid var(--color-border)' }}
          >
            TOML
          </span>
        </div>
        <div className="flex gap-2">
          <button
            onClick={copy}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs border hover:bg-white/5 transition-colors"
            style={{ borderColor: 'var(--color-border)', color: copied ? 'var(--color-accent)' : 'var(--color-text-muted)' }}
          >
            {copied ? <Check size={12} /> : <Copy size={12} />}
            Copy
          </button>
          <button
            onClick={() => refetch()}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs border hover:bg-white/5 transition-colors"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
          >
            <RefreshCw size={12} className={isLoading ? 'animate-spin' : ''} />
            Reload
          </button>
        </div>
      </div>

      <div
        className="rounded-lg border overflow-hidden"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        {/* Editor header */}
        <div
          className="flex items-center justify-between px-4 py-2 border-b text-xs"
          style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)' }}
        >
          <span>config.toml</span>
          <span>{lineCount} lines</span>
        </div>

        {/* Editor body */}
        <div className="relative flex" style={{ backgroundColor: 'var(--color-base)' }}>
          {/* Line numbers */}
          <div
            className="select-none text-right py-3 px-3 leading-5 min-w-[3rem] flex-shrink-0 overflow-hidden"
            style={{
              ...editorFontStyle,
              backgroundColor: 'var(--color-base)',
              color: 'var(--color-text-muted)',
              opacity: 0.45,
              borderRight: '1px solid var(--color-border)',
              height: '500px',
            }}
          >
            {Array.from({ length: lineCount }, (_, i) => (
              <div key={i}>{i + 1}</div>
            ))}
          </div>

          {/* Highlighted pre + transparent textarea overlay */}
          <div className="relative flex-1 overflow-hidden">
            {/* Highlighted layer */}
            <pre
              ref={preRef}
              aria-hidden
              className="absolute inset-0 py-3 px-4 m-0 overflow-auto pointer-events-none"
              style={{
                ...editorFontStyle,
                color: 'var(--color-text)',
                backgroundColor: 'transparent',
                height: '500px',
              }}
            >
              <TomlHighlight content={content || (isLoading ? '' : '# config.toml content here...')} />
            </pre>

            {/* Editable overlay */}
            <textarea
              ref={textareaRef}
              value={content}
              onChange={(e) => setContent(e.target.value)}
              onScroll={syncScroll}
              className="absolute inset-0 py-3 px-4 resize-none"
              style={{
                ...editorFontStyle,
                color: 'transparent',
                caretColor: 'var(--color-accent)',
                backgroundColor: 'transparent',
                border: 'none',
                outline: 'none',
                height: '500px',
                width: '100%',
              }}
              spellCheck={false}
              placeholder=""
            />
          </div>
        </div>

        {/* Footer */}
        <div
          className="flex items-center justify-between px-4 py-3 border-t"
          style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-base)' }}
        >
          <div>
            {saveErr && <p className="text-xs" style={{ color: 'var(--color-danger)' }}>{saveErr}</p>}
            {saveMsg && <p className="text-xs" style={{ color: 'var(--color-accent)' }}>{saveMsg}</p>}
          </div>
          <button
            onClick={() => saveMutation.mutate()}
            disabled={saveMutation.isPending || !content}
            className="flex items-center gap-2 px-4 py-1.5 rounded text-sm font-medium disabled:opacity-50"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            <Save size={13} />
            {saveMutation.isPending ? 'Saving...' : 'Save Config'}
          </button>
        </div>
      </div>

      <div
        className="mt-4 px-4 py-3 rounded text-xs border"
        style={{ backgroundColor: 'rgba(255,170,0,0.05)', borderColor: 'rgba(255,170,0,0.2)', color: 'var(--color-warning)' }}
      >
        Warning: Editing config directly. Sensitive values (API keys, tokens) are masked with ***MASKED*** — saving will preserve existing secrets.
      </div>

      {/* Export / Import */}
      <ExportImportPanel />
    </div>
  )
}

function ExportImportPanel() {
  const [importMsg, setImportMsg] = useState('')
  const [importErr, setImportErr] = useState('')
  const [importing, setImporting] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)

  function handleExport() {
    const token = getAuthToken()
    const a = document.createElement('a')
    a.href = '/api/export'
    // Pass auth token via URL is not ideal; use fetch instead
    fetch('/api/export', { headers: token ? { Authorization: `Bearer ${token}` } : {} })
      .then(r => r.blob())
      .then(blob => {
        const url = URL.createObjectURL(blob)
        a.href = url
        a.download = 'traderclaw-export.zip'
        a.click()
        URL.revokeObjectURL(url)
      })
  }

  async function handleImport(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0]
    if (!file) return
    setImporting(true)
    setImportMsg('')
    setImportErr('')
    try {
      const token = getAuthToken()
      const arrayBuf = await file.arrayBuffer()
      const b64 = btoa(String.fromCharCode(...new Uint8Array(arrayBuf)))
      const res = await fetch('/api/import', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        body: JSON.stringify({ data: b64 }),
      })
      const json = await res.json()
      if (res.ok) {
        const imported: string[] = json.imported ?? []
        setImportMsg(`Imported: ${imported.length ? imported.join(', ') : 'nothing'}`)
      } else {
        setImportErr(json.error ?? 'Import failed')
      }
    } catch (err: unknown) {
      setImportErr(err instanceof Error ? err.message : 'Import failed')
    } finally {
      setImporting(false)
      if (fileRef.current) fileRef.current.value = ''
    }
  }

  return (
    <div className="mt-4 rounded-lg border p-4" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
      <h2 className="text-sm font-semibold mb-3">Export / Import</h2>
      <p className="text-xs mb-4" style={{ color: 'var(--color-text-muted)' }}>
        Export wallets and strategies as a ZIP archive. Import to restore them on another instance.
        Sensitive config values (API keys) are masked in exports.
      </p>
      <div className="flex gap-3 flex-wrap">
        <button
          onClick={handleExport}
          className="flex items-center gap-2 px-4 py-1.5 rounded text-sm font-medium border"
          style={{ borderColor: 'var(--color-accent)', color: 'var(--color-accent)' }}
        >
          <Download size={13} />
          Export ZIP
        </button>
        <label className="flex items-center gap-2 px-4 py-1.5 rounded text-sm font-medium border cursor-pointer"
          style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}>
          <Upload size={13} />
          {importing ? 'Importing...' : 'Import ZIP'}
          <input ref={fileRef} type="file" accept=".zip" className="hidden" onChange={handleImport} disabled={importing} />
        </label>
      </div>
      {importMsg && <p className="mt-2 text-xs" style={{ color: 'var(--color-accent)' }}>{importMsg}</p>}
      {importErr && <p className="mt-2 text-xs" style={{ color: 'var(--color-danger)' }}>{importErr}</p>}
    </div>
  )
}
