import { useQueryClient, useQuery, useMutation } from '@tanstack/react-query'
import { apiPost, apiFetch } from './useApi'

// ── Types ─────────────────────────────────────────────────────

export type MarketType = 'crypto' | 'polymarket' | 'polymarket_binary' | 'clob_1hz' | 'archive_candles' | 'clob_events'

// A recurring Polymarket binary market series (loaded from /api/backtest/series)
export interface MarketSeries {
  id: string
  label: string
  slug_prefix: string
  data_source: 'binance' | 'open_meteo'
  symbol: string
  cadence: string
  resolution_logic: 'price_up' | 'threshold_above' | 'threshold_below'
  threshold: number | null
  unit: string | null
  description: string
  default_script: string | null
  builtin: boolean
}

// Legacy hardcoded presets — used as fallback if API not yet loaded
export interface PolyBinaryPreset {
  id: string
  label: string
  symbol: string
  defaultInterval: string
  description: string
}

export const POLY_BINARY_PRESETS: PolyBinaryPreset[] = [
  { id: 'btc_5m',   label: 'BTC UP/DOWN 5-min',  symbol: 'BTCUSDT',  defaultInterval: '5m',  description: 'Will BTC go up in the next 5 minutes?' },
  { id: 'btc_15m',  label: 'BTC UP/DOWN 15-min', symbol: 'BTCUSDT',  defaultInterval: '15m', description: 'Will BTC go up in the next 15 minutes?' },
  { id: 'btc_1h',   label: 'BTC UP/DOWN 1-hour', symbol: 'BTCUSDT',  defaultInterval: '1h',  description: 'Will BTC go up in the next hour?' },
  { id: 'eth_5m',   label: 'ETH UP/DOWN 5-min',  symbol: 'ETHUSDT',  defaultInterval: '5m',  description: 'Will ETH go up in the next 5 minutes?' },
  { id: 'eth_15m',  label: 'ETH UP/DOWN 15-min', symbol: 'ETHUSDT',  defaultInterval: '15m', description: 'Will ETH go up in the next 15 minutes?' },
  { id: 'eth_1h',   label: 'ETH UP/DOWN 1-hour', symbol: 'ETHUSDT',  defaultInterval: '1h',  description: 'Will ETH go up in the next hour?' },
  { id: 'sol_5m',   label: 'SOL UP/DOWN 5-min',  symbol: 'SOLUSDT',  defaultInterval: '5m',  description: 'Will SOL go up in the next 5 minutes?' },
  { id: 'sol_15m',  label: 'SOL UP/DOWN 15-min', symbol: 'SOLUSDT',  defaultInterval: '15m', description: 'Will SOL go up in the next 15 minutes?' },
  { id: 'sol_1h',   label: 'SOL UP/DOWN 1-hour', symbol: 'SOLUSDT',  defaultInterval: '1h',  description: 'Will SOL go up in the next hour?' },
  { id: 'xrp_5m',   label: 'XRP UP/DOWN 5-min',  symbol: 'XRPUSDT',  defaultInterval: '5m',  description: 'Will XRP go up in the next 5 minutes?' },
  { id: 'xrp_15m',  label: 'XRP UP/DOWN 15-min', symbol: 'XRPUSDT',  defaultInterval: '15m', description: 'Will XRP go up in the next 15 minutes?' },
  { id: 'doge_5m',  label: 'DOGE UP/DOWN 5-min', symbol: 'DOGEUSDT', defaultInterval: '5m',  description: 'Will DOGE go up in the next 5 minutes?' },
  { id: 'hype_5m',  label: 'HYPE UP/DOWN 5-min', symbol: 'HYPEUSDT', defaultInterval: '5m',  description: 'Will HYPE go up in the next 5 minutes?' },
  { id: 'bnb_5m',   label: 'BNB UP/DOWN 5-min',  symbol: 'BNBUSDT',  defaultInterval: '5m',  description: 'Will BNB go up in the next 5 minutes?' },
]

export interface BacktestConfig {
  kind?: string
  // Per-engine tunable parameters (only used when kind !== 'rhai_candle').
  engine_params?: Record<string, unknown>
  script: string
  market_type: MarketType
  symbol: string
  interval: string
  from_date: string
  to_date: string
  initial_balance: number
  fee_pct: number
  // Binary series identifier (replaces poly_binary_preset, drives symbol/interval/resolution)
  series_id?: string
  // Kept for backward compat with stored state
  poly_binary_preset?: string
  // Resolution override (set automatically from series)
  resolution_logic?: string
  threshold?: number
  // Polymarket position limit: max stake per trade in USD.
  // Reflects real market liquidity caps (~$500-$3,000 per 5-min window).
  max_position_usd?: number
  // Maximum entry price threshold. If the current price (crypto) or token
  // price (binary) exceeds this value, the trade/bet is skipped.
  max_entry_price?: number
  // Position sizing mode: fixed USD amount or percentage of balance.
  sizing_mode?: 'fixed' | 'percent'
  // Sizing value: USD amount for fixed mode, or max fraction (0-1) for percent mode.
  sizing_value?: number
  // Price mode for Polymarket binary entry: 'historical' = real scraped price,
  // 'mid' = average of buy/sell (mid-price).
  price_mode?: 'historical' | 'mid'
  // Hour gate: only trade during these UTC hours. Empty = no restriction.
  allowed_hours?: number[]
  // Spread guard: skip windows where CLOB spread > threshold. Default 0.03 (3%). archive_candles only.
  max_spread_pct?: number
  // RV floor: skip windows where BTC 1h realized-vol < this value. 0/undefined = disabled.
  rv_min_btc?: number
  // CLOB 1 HZ: tick slug to replay (e.g. "btc_5m"). Used as `symbol` in the backtest request.
  clob_slug?: string
  // Guardrail parameters — mirror the live runner's risk controls for live-parity backtesting.
  kelly_size_cap?: number
  min_entry_price?: number
  max_consecutive_losses?: number
  stop_loss_pct?: number
  // Latency simulation (clob_1hz / archive_candles / clob_events): order latency —
  // fill at signal_ts + latency_ms (clob_events: the order arrives this late).
  latency_ms?: number
  // clob_events only: feed latency — how late the strategy PERCEIVES each event
  // (ctx.ts_ms shifted forward). Separate from latency_ms (order arrival latency).
  feed_latency_ms?: number
  // Fee model: "pct" = flat fee_pct%, "crypto_taker" = 1.8%×p×(1-p) Polymarket formula.
  fee_model?: 'pct' | 'crypto_taker'
  // When true, /api/backtest/run also runs the 3-leg edge_validator on the
  // backtest's own trades and returns `edge_validation`. Passing the backtest is
  // NOT edge — this is the gate (bootstrap CI, random-null, shuffle-null).
  validate_edge?: boolean
}

// 3-leg edge validation result (mirrors edge_validator::ValidationResult).
export interface EdgeValidation {
  n: number
  wr_pct: number
  break_even_pct: number
  ev_per_trade_pct: number
  ci_lo: number
  ci_hi: number
  leg1_pass: boolean
  p_random: number
  leg2_pass: boolean
  p_shuffle: number
  leg3_pass: boolean
  // Leg 4 — calibration null: bettor wins with prob = entry price (fair value).
  // Works at constant price where Leg 3 (shuffle) is blind. Added after the quant review.
  p_calib?: number
  leg4_pass?: boolean
  verdict: 'EDGE' | 'NO_EDGE' | 'INSUFFICIENT'
  note: string
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
  final_balance?: number
  // Binary-specific metrics (present only for polymarket_binary runs)
  avg_token_price?: number
  correct_direction_pct?: number
  break_even_win_rate?: number
  markets_tested?: number
  // Historical data usage tracking
  windows_with_real_price?: number
  windows_with_estimated_price?: number
  historical_data_coverage_pct?: number
  recommended_max_stake_usd?: number
  // 3-leg edge validation (present only when config.validate_edge = true).
  edge_validation?: EdgeValidation
  // Maker engine stats (present only for rewards_maker / minting_mm on clob_events):
  // eligible_uptime_pct, adverse_selection_pct, yes_fills, no_fills.
  maker_stats?: Record<string, number>
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
  kind: 'rhai_candle',
  script: '',
  market_type: 'polymarket_binary',
  symbol: 'BTCUSDT',
  interval: '5m',
  from_date: THREE_MONTHS_AGO,
  to_date: TODAY,
  initial_balance: 10000,
  fee_pct: 1.5,
  series_id: 'btc_5m',
  poly_binary_preset: 'btc_5m',
  resolution_logic: 'price_up',
  // Default $500 per trade reflects real Polymarket 5-min binary window liquidity
  max_position_usd: 500,
  sizing_mode: 'percent',
  // Backend convention: percent mode uses 0-100 (e.g. 5 = 5%). Same as live runner.
  sizing_value: 5,
  price_mode: 'historical',
  allowed_hours: [],
  max_spread_pct: undefined, // undefined = use backend default (3%)
  rv_min_btc: undefined,
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
    const cfg: BacktestConfig = { ...DEFAULT_CONFIG, ...(parsed.config ?? {}) }
    // ── ONE-TIME MIGRATION: legacy fractional sizing_value → percent ───
    // Backend now expects 0-100 (5 = 5%). Old localStorage may have 0.05/0.25/1.0.
    if ((cfg.sizing_mode ?? 'percent') === 'percent'
        && cfg.sizing_value != null
        && cfg.sizing_value > 0
        && cfg.sizing_value <= 1.5) {
      console.log('[Backtest] One-time migration of localStorage sizing_value:', cfg.sizing_value, '→', cfg.sizing_value * 100)
      cfg.sizing_value = cfg.sizing_value * 100
    }
    return {
      config: cfg,
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

      await new Promise(r => setTimeout(r, 200))

      const fetchStart = Date.now()
      updateState({
        progress: {
          step: 'fetching',
          message: cfg.market_type === 'clob_events'
            ? `Replaying Orderbook Archive event stream '${cfg.clob_slug ?? cfg.symbol}' (${cfg.from_date} → ${cfg.to_date})…`
            : (cfg.market_type === 'clob_1hz' || cfg.market_type === 'archive_candles')
            ? `Loading Orderbook Archive tick data for '${cfg.clob_slug ?? cfg.symbol}' (${cfg.from_date} → ${cfg.to_date})…`
            : cfg.market_type === 'polymarket_binary'
            ? `Fetching ${cfg.symbol} 1m candles from Binance (${cfg.from_date} → ${cfg.to_date})…`
            : `Fetching ${cfg.symbol} ${cfg.interval} candles (${cfg.from_date} → ${cfg.to_date})…`,
          startTime: fetchStart,
        },
      })

      // After 8s advance to 'running', after 15s advance to 'analyzing'
      // These timers reflect what the backend is actually doing during the single API call.
      const engineLabel = cfg.kind && cfg.kind !== 'rhai_candle' ? cfg.kind : 'Rhai script'
      const phaseTimer = setTimeout(() => {
        updateState({
          progress: {
            step: 'running',
            message: `Executing ${engineLabel} engine against historical data…`,
            startTime: Date.now(),
          },
        })
      }, 8_000)
      const analyzeTimer = setTimeout(() => {
        updateState({
          progress: {
            step: 'analyzing',
            message: 'Computing metrics: Sharpe ratio, drawdown, win rate…',
            startTime: Date.now(),
          },
        })
      }, 15_000)

      // ── Normalize cfg before sending ────────────────────────────────────
      const cfgToSend: BacktestConfig = { ...cfg }

      // 1. sizing_value: legacy fraction → percent (backend uses 0-100).
      if ((cfg.sizing_mode ?? 'percent') === 'percent'
          && cfg.sizing_value != null
          && cfg.sizing_value > 0
          && cfg.sizing_value <= 1.5) {
        cfgToSend.sizing_value = cfg.sizing_value * 100
        console.log('[Backtest] Normalized legacy sizing_value', cfg.sizing_value, '→', cfgToSend.sizing_value)
      }

      // 2. archive modes use `symbol` as the slug directory name (e.g. "btc_5m" for
      // tick modes, "btc_5m_ev" for the event stream), NOT as a Binance ticker. If the
      // user switched market_type without re-selecting the slug, `symbol` may still be
      // "BTCUSDT" from polymarket_binary mode → backend looks for data/ticks/BTCUSDT/
      // (or data/events/BTCUSDT/) which doesn't exist → 0 trades. Fix here.
      if (cfg.market_type === 'clob_1hz' || cfg.market_type === 'archive_candles' || cfg.market_type === 'clob_events') {
        const slug = cfg.clob_slug ?? cfg.series_id
        if (slug && cfg.symbol !== slug) {
          cfgToSend.symbol = slug
          console.log('[Backtest] Archive mode: symbol "' + cfg.symbol + '" → "' + slug + '" (slug)')
        }
      }

      // ALWAYS log the EXACT payload that will be sent to the backend.
      // Helps diagnose 0-trade results when the UI sends unexpected values.
      console.log('[Backtest] Sending payload:', JSON.stringify(cfgToSend, null, 2))

      try {
        const response = await apiPost<BacktestResult>('/api/backtest/run', cfgToSend)
        clearTimeout(phaseTimer)
        clearTimeout(analyzeTimer)
        console.log('[Backtest] Complete: trades=', response.total_trades, 'return=', response.total_return_pct)
        return response
      } catch (err) {
        clearTimeout(phaseTimer)
        clearTimeout(analyzeTimer)
        throw err
      }
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
    runBacktestAsync: (cfg: BacktestConfig) => runBacktest.mutateAsync(cfg),

    // Mutation state
    mutation: runBacktest,
  }
}
