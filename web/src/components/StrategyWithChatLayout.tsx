import { useState } from 'react'
import type { ReactNode } from 'react'
import { MessageSquare, PanelRightClose, PanelRightOpen } from 'lucide-react'
import Chat from '../pages/Chat'

type ChatPanelSize = '30' | '40' | 'hidden'

interface StrategyWithChatLayoutProps {
  children: ReactNode
}

export default function StrategyWithChatLayout({ children }: StrategyWithChatLayoutProps) {
  const [size, setSize] = useState<ChatPanelSize>('30')
  const isHidden = size === 'hidden'

  const widthStyle =
    size === '40'
      ? { width: '40vw', maxWidth: 700, minWidth: 420 }
      : { width: '30vw', maxWidth: 560, minWidth: 360 }

  return (
    <div className="h-full flex flex-col lg:flex-row">
      <section className="flex-1 min-w-0 overflow-auto">
        {children}
      </section>

      <aside
        className="border-t lg:border-t-0 lg:border-l"
        style={{
          borderColor: 'var(--color-border)',
          ...(isHidden ? { width: 46, minWidth: 46 } : widthStyle),
        }}
      >
        <div className="h-full flex flex-col" style={{ backgroundColor: 'var(--color-surface)' }}>
          <div className="h-10 border-b flex items-center justify-end gap-1 px-2" style={{ borderColor: 'var(--color-border)' }}>
            <button
              type="button"
              onClick={() => setSize('30')}
              className="px-2 py-1 rounded text-xs"
              style={{
                backgroundColor: size === '30' ? 'var(--color-accent)' : 'transparent',
                color: size === '30' ? '#000' : 'var(--color-text-muted)',
              }}
              title="Chat 30%"
            >
              30%
            </button>
            <button
              type="button"
              onClick={() => setSize('40')}
              className="px-2 py-1 rounded text-xs"
              style={{
                backgroundColor: size === '40' ? 'var(--color-accent)' : 'transparent',
                color: size === '40' ? '#000' : 'var(--color-text-muted)',
              }}
              title="Chat 40%"
            >
              40%
            </button>
            <button
              type="button"
              onClick={() => setSize(isHidden ? '30' : 'hidden')}
              className="p-1.5 rounded hover:bg-white/5"
              style={{ color: 'var(--color-text-muted)' }}
              title={isHidden ? 'Show chat panel' : 'Hide chat panel'}
            >
              {isHidden ? <PanelRightOpen size={14} /> : <PanelRightClose size={14} />}
            </button>
          </div>

          {isHidden ? (
            <div className="flex-1 flex items-center justify-center" style={{ color: 'var(--color-text-muted)' }}>
              <MessageSquare size={16} />
            </div>
          ) : (
            <div className="flex-1 min-h-0">
              <Chat />
            </div>
          )}
        </div>
      </aside>
    </div>
  )
}
