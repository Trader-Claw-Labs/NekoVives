import { useState, useEffect } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import { Settings, Save, RefreshCw, Copy, Check } from 'lucide-react'

interface ConfigResponse {
  format?: string
  content?: string
}

export default function Config() {
  const [content, setContent] = useState('')
  const [saveMsg, setSaveMsg] = useState('')
  const [saveErr, setSaveErr] = useState('')
  const [copied, setCopied] = useState(false)

  const { data, isLoading, refetch } = useQuery<ConfigResponse>({
    queryKey: ['config'],
    queryFn: () => apiFetch('/api/config'),
  })

  useEffect(() => {
    if (data?.content) {
      setContent(data.content)
    }
  }, [data])

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

        {/* Editor */}
        <div className="relative flex">
          {/* Line numbers */}
          <div
            className="select-none text-right py-3 px-3 text-xs leading-5 min-w-[3rem]"
            style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)', borderRight: '1px solid var(--color-border)' }}
          >
            {Array.from({ length: lineCount }, (_, i) => (
              <div key={i}>{i + 1}</div>
            ))}
          </div>

          {/* Textarea */}
          <textarea
            value={content}
            onChange={(e) => setContent(e.target.value)}
            className="flex-1 py-3 px-4 text-xs leading-5 resize-none font-mono min-h-[500px]"
            style={{
              backgroundColor: 'transparent',
              color: 'var(--color-text)',
              border: 'none',
              outline: 'none',
            }}
            spellCheck={false}
            placeholder={isLoading ? 'Loading config...' : '# config.toml content here...'}
          />
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
    </div>
  )
}
