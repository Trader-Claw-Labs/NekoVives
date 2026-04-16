import { useQueryClient, useQuery, useMutation } from '@tanstack/react-query'
import { apiPost } from './useApi'

// ── Types ─────────────────────────────────────────────────────────

export type MarketType = 'crypto' | 'polymarket' | 'polymarket_binary'

// Presets for binary markets (no condition token ID needed)
export interface PolyBinaryPreset {
  id: string
  label: string
  symbol: string          // Binance pair for underlying data
  defaultInterval: string // default window size
  description: string
}

export const POLY_BINARY_PRESETS: PolyBinaryPreset[] = [
  { id: 'btc_5m',  label: 'BTC — 5-min binary',  symbol: 'BTCUSDT', defaultInterval: '5m',  description: 'Will BTC go up in the next 5 minutes?' },
  { id: 'btc_4m',  label: 'BTC — 4-min binary',  symbol: 'BTCUSDT', defaultInterval: '4m',  description: 'Will BTC go up in the next 4 minutes?' },
  { id: 'btc_15m', label: 'BTC — 15-min binary', symbol: 'BTCUSDT', defaultInterval: '15m', description: 'Will BTC go up in the next 15 minutes?' },
  { id: 'btc_1h',  label: 'BTC — 1-hour binary', symbol: 'BTCUSDT', defaultInterval: '1h',  description: 'Will BTC go up in the next hour?' },
  { id: 'eth_5m',  label: 'ETH — 5-min binary',  symbol: 'ETHUSDT', defaultInterval: '5m',  description: 'Will ETH go up in the next 5 minutes?' },
  { id: 'eth_15m', label: 'ETH — 15-min binary', symbol: 'ETHUSDT', defaultInterval: '15m', description: 'Will ETH go up in the next 15 minutes?' },
]

export interface BacktestConfig {
  script: string
  market_type: MarketType
  symbol: string
  interval: string
  from_date: string
  to_date: string
  initial_balance: number
  fee_pct: number
  // Binary-only: which preset is selected (drives symbol + default interval)
  poly_binary_preset?: string
}

export interface TradeLog {
  timestamp: string
  side: string
  price: number
  size: number
  pnl: number
  balance?: number
}

export interface BacktestResult {
  script: string
  symbol: string
  total_return_pct: number
  sharpe_ratio: number | null
  max_drawdown_pct: number
  win_rate_pct: number
  total_trades: number
  worst_trades: TradeLog[]
  all_trades?: TradeLog[]
  analysis?: string
  initial_balance?: number
  // Binary-specific metrics (present only for polymarket_binary runs)
  avg_token_price?: number
  correct_direction_pct?: number
  break_even_win_rate?: number
}

export interface ProgressState {
  step: 'idle' | 'preparing' | 'fetching' | 'running' | 'analyzing' | 'done' | 'error'
  message: string
  progress?: number
  startTime?: number
}

export interface BacktestState {
  config: BacktestConfig
  result: BacktestResult | null
  progress: ProgressState
  isRunning: boolean
  runningScriptPath: string | null  // tracks which script is actually running
  error: string | null
  // per-script cached results, key = script path
  scriptResults: Record<string, BacktestResult>
}

// ── Defaults ─────────────────────────────────────────────────────────

const TODAY = new Date().toISOString().slice(0, 10)
const THREE_MONTHS_AGO = new Date(Date.now() - 90 * 86400 * 1000).toISOString().slice(0, 10)

const DEFAULT_CONFIG: BacktestConfig = {
  script: '',
  market_type: 'crypto',
  symbol: 'BTCUSDT',
  interval: '1m',
  from_date: THREE_MONTHS_AGO,
  to_date: TODAY,
  initial_balance: 10000,
  fee_pct: 0.1,
  poly_binary_preset: 'btc_5m',
}

const DEFAULT_STATE: BacktestState = {
  config: DEFAULT_CONFIG,
  result: null,
  progress: { step: 'idle', message: '' },
  isRunning: false,
  runningScriptPath: null,
  error: null,
  scriptResults: {},
}

// ── Persistence helpers ─────────────────────────────────────────────────────

const LS_KEY = 'trader-claw:backtest-state-v1'

function loadFromStorage(): Partial<BacktestState> {
  try {
    const raw = localStorage.getItem(LS_KEY)
    if (!raw) return {}
    const parsed = JSON.parse(raw)
    // Only restore non-running state — don't restore stale isRunning/progress
    return {
      config: parsed.config ?? DEFAULT_CONFIG,
      result: parsed.result ?? null,
      scriptResults: parsed.scriptResults ?? {},
    }
  } catch {
    return {}
  }
}

function saveToStorage(s: BacktestState) {
  try {
    localStorage.setItem(LS_KEY, JSON.stringify({
      config: s.config,
      result: s.result,
      scriptResults: s.scriptResults,
    }))
  } catch {
    // ignore quota errors
  }
}

// ── Query Keys ─────────────────────────────────────────────────────────

const BACKTEST_STATE_KEY = ['backtest-state']

// ── Hook ─────────────────────────────────────────────────────────

export function useBacktestState() {
  const queryClient = useQueryClient()

  // Get persisted state from cache, seeding from localStorage on first load
  const { data: state } = useQuery<BacktestState>({
    queryKey: BACKTEST_STATE_KEY,
    queryFn: () => {
      const cached = queryClient.getQueryData<BacktestState>(BACKTEST_STATE_KEY)
      if (cached) return cached
      // First load: merge localStorage into defaults
      const stored = loadFromStorage()
      return { ...DEFAULT_STATE, ...stored }
    },
    staleTime: Infinity,
    gcTime: Infinity,
  })

  const currentState = state ?? DEFAULT_STATE

  // Update state helper — also persists to localStorage
  const updateState = (updates: Partial<BacktestState>) => {
    queryClient.setQueryData<BacktestState>(BACKTEST_STATE_KEY, (old) => {
      const next = { ...(old ?? DEFAULT_STATE), ...updates }
      saveToStorage(next)
      return next
    })
  }

  // Update config
  const setConfig = <K extends keyof BacktestConfig>(key: K, value: BacktestConfig[K]) => {
    updateState({
      config: { ...currentState.config, [key]: value },
    })
  }

  // Set full config
  const setFullConfig = (config: BacktestConfig) => {
    updateState({ config })
  }

  // Set progress
  const setProgress = (progress: ProgressState) => {
    updateState({ progress })
  }

  // Set result
  const setResult = (result: BacktestResult | null) => {
    updateState({ result, isRunning: false })
  }

  // Clear result
  const clearResult = () => {
    updateState({
      result: null,
      progress: { step: 'idle', message: '' },
      error: null,
    })
  }

  // Run backtest mutation
  const runBacktest = useMutation({
    mutationFn: async (cfg: BacktestConfig) => {
      console.log('[Backtest] Starting with config:', cfg)

      updateState({
        isRunning: true,
        runningScriptPath: cfg.script,
        error: null,
        progress: { step: 'preparing', message: 'Validating configuration...', startTime: Date.now() },
      })

      await new Promise(r => setTimeout(r, 300))

      updateState({
        progress: {
          step: 'fetching',
          message: cfg.market_type === 'polymarket_binary'
            ? `Fetching ${cfg.symbol} 1m candles from Binance for ${cfg.interval} binary windows (${cfg.from_date} → ${cfg.to_date})...`
            : `Fetching ${cfg.symbol} ${cfg.interval} candles (${cfg.from_date} to ${cfg.to_date})...`,
          startTime: Date.now(),
        },
      })

      const response = await apiPost<BacktestResult>('/api/backtest/run', cfg)

      updateState({
        progress: { step: 'running', message: 'Executing Rhai strategy engine...', startTime: Date.now() },
      })
      await new Promise(r => setTimeout(r, 200))

      updateState({
        progress: { step: 'analyzing', message: 'Computing metrics and analysis...', startTime: Date.now() },
      })
      await new Promise(r => setTimeout(r, 200))

      console.log('[Backtest] Complete:', response)
      return response
    },
    onSuccess: (data) => {
      queryClient.setQueryData<BacktestState>(BACKTEST_STATE_KEY, (old) => {
        const base = old ?? DEFAULT_STATE
        const next = {
          ...base,
          result: data,
          isRunning: false,
          runningScriptPath: null,
          progress: { step: 'done' as const, message: 'Backtest complete!' },
          scriptResults: { ...base.scriptResults, [data.script]: data },
        }
        saveToStorage(next)
        return next
      })
    },
    onError: (err) => {
      console.error('[Backtest] Error:', err)
      updateState({
        isRunning: false,
        runningScriptPath: null,
        error: (err as Error)?.message ?? String(err),
        progress: { step: 'error', message: `Error: ${(err as Error)?.message ?? String(err)}` },
      })
    },
  })

  return {
    // State
    config: currentState.config,
    result: currentState.result,
    progress: currentState.progress,
    isRunning: currentState.isRunning || runBacktest.isPending,
    runningScriptPath: currentState.runningScriptPath,
    error: currentState.error,
    scriptResults: currentState.scriptResults,

    // Actions
    setConfig,
    setFullConfig,
    setProgress,
    setResult,
    clearResult,
    runBacktest: (cfg?: BacktestConfig) => runBacktest.mutate(cfg ?? currentState.config),

    // Mutation state
    mutation: runBacktest,
  }
}
