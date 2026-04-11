import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { apiFetch } from '../hooks/useApi'
import {
  Zap, RefreshCw, Eye, X, ExternalLink, Package, Store
} from 'lucide-react'
import clsx from 'clsx'

interface AgentSkill {
  name: string
  description: string
  version: string
  author?: string
  tags: string[]
  location: string
  prompts?: string[]
}

interface SkillsResponse {
  skills: AgentSkill[]
}

interface SkillViewerModalProps {
  skill: AgentSkill
  onClose: () => void
}

function SkillViewerModal({ skill, onClose }: SkillViewerModalProps) {
  const { data, isLoading } = useQuery({
    queryKey: ['skill-content', skill.location],
    queryFn: async () => {
      const res = await apiFetch<{ content: string }>(`/api/skills/content?path=${encodeURIComponent(skill.location)}`)
      return res.content
    },
  })

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div
        className="rounded-lg border w-full max-w-3xl max-h-[80vh] flex flex-col"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center justify-between p-4 border-b" style={{ borderColor: 'var(--color-border)' }}>
          <div>
            <h2 className="font-semibold">{skill.name}</h2>
            <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              v{skill.version} {skill.author && `by ${skill.author}`}
            </p>
          </div>
          <button onClick={onClose} style={{ color: 'var(--color-text-muted)' }}>
            <X size={16} />
          </button>
        </div>

        <div className="flex-1 overflow-auto p-4">
          {isLoading ? (
            <div className="text-center py-8" style={{ color: 'var(--color-text-muted)' }}>
              Loading skill content...
            </div>
          ) : (
            <pre
              className="text-xs font-mono whitespace-pre-wrap p-4 rounded"
              style={{ backgroundColor: 'var(--color-base)' }}
            >
              {data || 'No content available'}
            </pre>
          )}
        </div>

        <div className="p-4 border-t flex justify-end" style={{ borderColor: 'var(--color-border)' }}>
          <button
            onClick={onClose}
            className="px-4 py-2 rounded text-sm font-medium"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            Close
          </button>
        </div>
      </div>
    </div>
  )
}

interface SkillCardProps {
  skill: AgentSkill
  onView: () => void
}

function SkillCard({ skill, onView }: SkillCardProps) {
  return (
    <div
      className="rounded-lg border p-4 card-hover transition-all"
      style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
    >
      <div className="flex items-start justify-between mb-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <Package size={14} style={{ color: 'var(--color-accent)' }} />
            <h3 className="text-sm font-semibold truncate">{skill.name}</h3>
          </div>
          <p
            className="text-xs line-clamp-2"
            style={{ color: 'var(--color-text-muted)' }}
          >
            {skill.description}
          </p>
        </div>
        <button
          onClick={onView}
          className="ml-2 p-1 rounded hover:bg-white/5 transition-colors"
          style={{ color: 'var(--color-text-muted)' }}
          title="View skill"
        >
          <Eye size={14} />
        </button>
      </div>

      <div className="flex items-center gap-2 mb-3 flex-wrap">
        <span
          className="text-xs px-2 py-0.5 rounded"
          style={{ backgroundColor: 'var(--color-accent-dim)', color: 'var(--color-accent)' }}
        >
          v{skill.version}
        </span>
        {skill.tags.map((tag) => (
          <span
            key={tag}
            className="text-xs px-2 py-0.5 rounded"
            style={{ backgroundColor: 'var(--color-base)', color: 'var(--color-text-muted)' }}
          >
            {tag}
          </span>
        ))}
      </div>

      {skill.author && (
        <div className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
          by {skill.author}
        </div>
      )}
    </div>
  )
}

export default function Skills() {
  const [viewingSkill, setViewingSkill] = useState<AgentSkill | null>(null)

  const { data, isLoading, refetch } = useQuery<SkillsResponse>({
    queryKey: ['agent-skills'],
    queryFn: (): Promise<SkillsResponse> =>
      apiFetch<SkillsResponse>('/api/skills').catch(() => ({ skills: [] })),
    refetchInterval: 30_000,
  })

  const skills = data?.skills ?? []

  return (
    <div className="p-6 max-w-5xl mx-auto">
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-2">
          <Zap size={18} style={{ color: 'var(--color-accent)' }} />
          <h1 className="text-lg font-bold">Agent Skills</h1>
          <span
            className="text-xs px-2 py-0.5 rounded"
            style={{ backgroundColor: 'var(--color-accent-dim)', color: 'var(--color-accent)' }}
          >
            {skills.length} installed
          </span>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => refetch()}
            className="p-2 rounded border hover:bg-white/5 transition-colors"
            style={{ borderColor: 'var(--color-border)', color: 'var(--color-text-muted)' }}
          >
            <RefreshCw size={13} className={isLoading ? 'animate-spin' : ''} />
          </button>
        </div>
      </div>

      {/* Marketplace teaser */}
      <div
        className="rounded-lg border p-4 mb-6"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <div className="flex items-center gap-3">
          <div
            className="p-2 rounded"
            style={{ backgroundColor: 'var(--color-accent-dim)' }}
          >
            <Store size={20} style={{ color: 'var(--color-accent)' }} />
          </div>
          <div className="flex-1">
            <h2 className="text-sm font-semibold">Skills Marketplace</h2>
            <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
              Coming soon: Browse and install community trading skills
            </p>
          </div>
          <button
            disabled
            className="flex items-center gap-2 px-3 py-1.5 rounded text-sm font-medium opacity-50 cursor-not-allowed"
            style={{ backgroundColor: 'var(--color-accent)', color: '#000' }}
          >
            <ExternalLink size={14} />
            Browse
          </button>
        </div>
      </div>

      {/* Skills info */}
      <div
        className="rounded-lg border p-4 mb-6"
        style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}
      >
        <h2 className="text-sm font-semibold mb-2">What are Agent Skills?</h2>
        <p className="text-xs mb-2" style={{ color: 'var(--color-text-muted)' }}>
          Skills are instructions and capabilities that extend the AI agent. They help the agent perform specialized tasks like generating trading strategies, analyzing markets, or interacting with specific protocols.
        </p>
        <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
          Skills are stored in <code className="px-1 rounded" style={{ backgroundColor: 'var(--color-base)' }}>~/.traderclaw/workspace/skills/</code>
        </p>
      </div>

      {/* Installed skills grid */}
      {isLoading ? (
        <div className="text-sm text-center py-8" style={{ color: 'var(--color-text-muted)' }}>
          Loading skills...
        </div>
      ) : skills.length === 0 ? (
        <div className="text-center py-12">
          <Zap size={40} className="mx-auto mb-3" style={{ color: 'var(--color-text-muted)' }} />
          <p className="text-sm mb-2" style={{ color: 'var(--color-text-muted)' }}>
            No skills installed
          </p>
          <p className="text-xs" style={{ color: 'var(--color-text-muted)' }}>
            Create a skill folder in ~/.traderclaw/workspace/skills/ with a SKILL.md file
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {skills.map((skill) => (
            <SkillCard
              key={skill.name}
              skill={skill}
              onView={() => setViewingSkill(skill)}
            />
          ))}
        </div>
      )}

      {viewingSkill && (
        <SkillViewerModal
          skill={viewingSkill}
          onClose={() => setViewingSkill(null)}
        />
      )}
    </div>
  )
}
