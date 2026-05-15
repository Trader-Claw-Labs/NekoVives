import { Blocks, Play, FlaskConical, Bot, Send, BookOpen, AlertCircle } from 'lucide-react'

export default function Help() {
  return (
    <div className="p-6 max-w-4xl mx-auto space-y-8">
      <div className="flex items-center gap-3">
        <BookOpen size={24} style={{ color: 'var(--color-accent)' }} />
        <h1 className="text-xl font-bold">Neko Vives Help</h1>
      </div>

      <div className="rounded-lg border p-5" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
        <h2 className="text-sm font-bold mb-4">Quick Start</h2>
        <div className="space-y-4">
          <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
            Neko Vives is an autonomous agent-driven trading platform.
          </p>
          <ul className="list-disc list-inside text-sm space-y-2" style={{ color: 'var(--color-text-muted)' }}>
            <li><strong>Dashboard:</strong> View system status, health, and quick stats.</li>
            <li><strong>Strategy Builder:</strong> Create your trading scripts using templates.</li>
            <li><strong>Backtesting:</strong> Validate your strategies against historical data.</li>
            <li><strong>Live Strategies:</strong> Deploy and monitor active trading strategies.</li>
          </ul>
        </div>
      </div>

      <div className="rounded-lg border p-5" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
        <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
          <Blocks size={16} /> Creating a Strategy
        </h2>
        <div className="space-y-3">
          <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>1. Navigate to <strong>Strategy Builder</strong>.</p>
          <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>2. Choose a template (e.g., BTC Momentum).</p>
          <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>3. Adjust parameters (RSI, ATR multipliers, etc.).</p>
          <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>4. Click <strong>Save Strategy</strong> to generate the .rhai file.</p>
        </div>
      </div>

      <div className="rounded-lg border p-5" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
        <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
          <FlaskConical size={16} /> Testing
        </h2>
        <div className="space-y-3">
          <p className="text-sm" style={{ color: 'var(--color-text-muted)' }}>
            After saving, go to <strong>Backtesting</strong> to test against real candle data from Binance or Polymarket series.
          </p>
          <code className="text-xs p-2 rounded block" style={{ backgroundColor: 'var(--color-surface-2)' }}>
            Results show KPI: Win Rate, Sharpe Ratio, Max Drawdown
          </code>
        </div>
      </div>

      <div className="rounded-lg border p-5" style={{ backgroundColor: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
        <h2 className="text-sm font-bold mb-4 flex items-center gap-2">
          <Bot size={16} /> Production Launch
        </h2>
        <p className="text-sm mb-3" style={{ color: 'var(--color-text-muted)' }}>
          Once confident in backtest results:
        </p>
        <ol className="list-decimal list-inside text-sm space-y-2" style={{ color: 'var(--color-text-muted)' }}>
          <li>Go to <strong>Live Strategies</strong>.</li>
          <li>Click <strong>+ Add New Strategy</strong>.</li>
          <li>Select the strategy file, market, and wallet.</li>
          <li>Set execution frequency and confirm.</li>
        </ol>
      </div>

      <div className="rounded-lg border p-4 flex items-center gap-3" style={{ backgroundColor: 'var(--color-warning)10', borderColor: 'var(--color-warning)' }}>
        <AlertCircle size={20} style={{ color: 'var(--color-warning)' }} />
        <p className="text-sm font-medium" style={{ color: 'var(--color-warning)' }}>
          Always start with paper-money strategies or small test sizes before scaling.
        </p>
      </div>
    </div>
  )
}
