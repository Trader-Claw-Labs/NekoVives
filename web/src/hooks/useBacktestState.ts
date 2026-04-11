import { useQueryClient, useQuery, useMutation } from '@tanstack/react-query'
import { apiPost } from './useApi'

// ── Types ─────────────────────────────────────────────────────────

export type MarketType = 'crypto' | 'polymarket'

export interface BacktestConfig {
  script: string
  market_type: MarketType
  symbol: string
  interval: string
  from_date: string
  to_date: string
  initial_balance: number
  fee_pct: number
}

export interface TradeLog {
  timestamp: string
  side: string
  price: number
  size: number
  pnl: number
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
  analysis?: string
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
  error: string | null
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
}

const DEFAULT_STATE: BacktestState = {
  config: DEFAULT_CONFIG,
  result: null,
  progress: { step: 'idle', message: '' },
  isRunning: false,
  error: null,
}

// ── Query Keys ─────────────────────────────────────────────────────────

const BACKTEST_STATE_KEY = ['backtest-state']

// ── Hook ─────────────────────────────────────────────────────────

export function useBacktestState() {
  const queryClient = useQueryClient()

  // Get persisted state from cache
  const { data: state } = useQuery<BacktestState>({
    queryKey: BACKTEST_STATE_KEY,
    queryFn: () => {
      // Return cached data or default
      const cached = queryClient.getQueryData<BacktestState>(BACKTEST_STATE_KEY)
      return cached ?? DEFAULT_STATE
    },
    staleTime: Infinity, // Never refetch automatically
    gcTime: Infinity, // Keep in cache forever during session
  })

  const currentState = state ?? DEFAULT_STATE

  // Update state helper
  const updateState = (updates: Partial<BacktestState>) => {
    queryClient.setQueryData<BacktestState>(BACKTEST_STATE_KEY, (old) => ({
      ...(old ?? DEFAULT_STATE),
      ...updates,
    }))
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
        error: null,
        progress: { step: 'preparing', message: 'Validating configuration...', startTime: Date.now() },
      })

      await new Promise(r => setTimeout(r, 300))

      updateState({
        progress: {
          step: 'fetching',
          message: `Fetching ${cfg.symbol} ${cfg.interval} candles (${cfg.from_date} to ${cfg.to_date})...`,
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
      updateState({
        result: data,
        isRunning: false,
        progress: { step: 'done', message: 'Backtest complete!' },
      })
    },
    onError: (err) => {
      console.error('[Backtest] Error:', err)
      updateState({
        isRunning: false,
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
    error: currentState.error,

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
