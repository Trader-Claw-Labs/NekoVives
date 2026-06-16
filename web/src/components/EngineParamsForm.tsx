/**
 * Per-engine parameter form — rendered in both Backtesting and LiveStrategies
 * whenever the user selects a built-in engine kind.
 *
 * Each engine exposes only its most important tuning knobs; the rest keep Rust
 * defaults. All values are stored as a flat `Record<string,unknown>` and sent
 * as `engine_params` in the POST body.
 */

import { ChevronDown } from 'lucide-react'
import { useState } from 'react'

// ── Field definitions ──────────────────────────────────────────────────────

interface FieldDef {
  key: string
  label: string
  type: 'number' | 'text' | 'select'
  default: unknown
  min?: number
  max?: number
  step?: number
  options?: { value: string; label: string }[]
  hint?: string
}

export const ENGINE_PARAM_DEFS: Record<string, FieldDef[]> = {
  arb_binary: [
    { key: 'min_edge_pct', label: 'Min edge %', type: 'number', default: 0.05, min: 0, max: 0.5, step: 0.005, hint: 'YES+NO < 1 − edge to open arb' },
    { key: 'max_position_usd', label: 'Max position ($)', type: 'number', default: 500, min: 10, max: 100000, step: 10 },
    { key: 'liquidity_floor_usd', label: 'Liquidity floor ($)', type: 'number', default: 100, min: 0, max: 10000, step: 10 },
    { key: 'fee_pct', label: 'Fee %', type: 'number', default: 0.002, min: 0, max: 0.05, step: 0.0005 },
    { key: 'poll_secs', label: 'Poll interval (s)', type: 'number', default: 30, min: 5, max: 300, step: 5 },
  ],
  fair_value: [
    { key: 'edge_threshold', label: 'Edge threshold', type: 'number', default: 0.05, min: 0, max: 0.5, step: 0.005, hint: 'Min |FV − market| to trade' },
    { key: 'vwap_window', label: 'VWAP window (candles)', type: 'number', default: 20, min: 5, max: 100, step: 1 },
    { key: 'w_price', label: 'Price weight', type: 'number', default: 0.5, min: 0, max: 1, step: 0.05 },
    { key: 'w_volume', label: 'Volume weight', type: 'number', default: 0.25, min: 0, max: 1, step: 0.05 },
    { key: 'w_calibration', label: 'Calibration weight', type: 'number', default: 0.25, min: 0, max: 1, step: 0.05 },
    { key: 'kelly_cap', label: 'Kelly cap', type: 'number', default: 0.25, min: 0.01, max: 1, step: 0.01, hint: 'Max fraction of balance per bet' },
    { key: 'max_position_usd', label: 'Max position ($)', type: 'number', default: 300, min: 10, max: 100000, step: 10 },
  ],
  fv_momentum: [
    { key: 'edge_threshold', label: 'FV edge threshold', type: 'number', default: 0.05, min: 0, max: 0.5, step: 0.005 },
    { key: 'kelly_cap', label: 'Kelly cap', type: 'number', default: 0.25, min: 0.01, max: 1, step: 0.01 },
    { key: 'momentum_window', label: 'Momentum window (candles)', type: 'number', default: 5, min: 2, max: 50, step: 1 },
    { key: 'momentum_threshold', label: 'Momentum min Δ', type: 'number', default: 0.01, min: 0, max: 0.2, step: 0.005, hint: 'Min price change to confirm trend' },
    { key: 'convergence_pct', label: 'Convergence %', type: 'number', default: 0.02, min: 0, max: 0.2, step: 0.005, hint: 'Exit when FV gap closes this much' },
    { key: 'max_position_usd', label: 'Max position ($)', type: 'number', default: 300, min: 10, max: 100000, step: 10 },
  ],
  rotation_compounder: [
    { key: 'max_allocation_pct', label: 'Max allocation %', type: 'number', default: 0.6, min: 0.1, max: 1, step: 0.05 },
    { key: 'switch_threshold', label: 'Switch threshold', type: 'number', default: 0.05, min: 0, max: 0.3, step: 0.005, hint: 'Score gap needed to rotate' },
    { key: 'min_position_usd', label: 'Min position ($)', type: 'number', default: 10, min: 1, max: 1000, step: 1 },
    { key: 'stop_loss_pct', label: 'Stop-loss %', type: 'number', default: 0.4, min: 0, max: 1, step: 0.05 },
    { key: 'sim_days_to_close', label: 'Sim days to close', type: 'number', default: 15, min: 1, max: 90, step: 1, hint: 'Simulated resolution window for scoring' },
  ],
  arb_hedge: [
    { key: 'min_arb_edge', label: 'Min arb edge', type: 'number', default: 0.03, min: 0, max: 0.3, step: 0.005, hint: 'YES+NO < 1 − edge to open' },
    { key: 'hedge_trigger_pct', label: 'Hedge trigger %', type: 'number', default: 0.2, min: 0, max: 1, step: 0.05, hint: 'Position loss % that opens counter-hedge' },
    { key: 'max_position_usd', label: 'Max position ($)', type: 'number', default: 200, min: 10, max: 100000, step: 10 },
  ],
  rewards_maker: [
    { key: 'offset_cents', label: 'Offset (¢)', type: 'number', default: 1.0, min: 0.5, max: 5, step: 0.5, hint: 'Cents inside the mid each leg rests. Stay within the market max_spread to be eligible.' },
    { key: 'reprice_threshold', label: 'Reprice drift', type: 'number', default: 0.02, min: 0.005, max: 0.1, step: 0.005, hint: 'Re-center both quotes when the mid drifts this far (abs price). 0.02 = 2¢.' },
    { key: 'poll_secs', label: 'Poll interval (s)', type: 'number', default: 60, min: 5, max: 600, step: 5, hint: 'How often the maker checks the mid and re-posts filled legs.' },
  ],
  rewards_orchestrator: [
    { key: 'max_markets', label: 'Pool size (markets)', type: 'number', default: 3, min: 1, max: 10, step: 1, hint: 'How many safe reward markets to quote at once. Capital is split across all of them.' },
    { key: 'min_safety', label: 'Min safety', type: 'select', default: 'high', options: [
      { value: 'high', label: 'High (≥7d, ≥3¢ spread)' },
      { value: 'medium', label: 'Medium (≥1d, ≥1.5¢)' },
      { value: 'low', label: 'Low (any non-toxic)' },
    ], hint: 'Only quote markets at/above this safety. Toxic markets are always excluded.' },
    { key: 'offset_cents', label: 'Offset (¢)', type: 'number', default: 1.0, min: 0.5, max: 5, step: 0.5, hint: 'Cents inside the mid each leg rests.' },
    { key: 'reprice_threshold', label: 'Reprice drift', type: 'number', default: 0.02, min: 0.005, max: 0.1, step: 0.005, hint: 'Re-center when the mid drifts this far (abs price).' },
    { key: 'poll_secs', label: 'Poll interval (s)', type: 'number', default: 60, min: 10, max: 600, step: 5, hint: 'How often it re-scans markets + re-posts filled legs.' },
    { key: 'size_usd', label: 'Size/side ($, 0=auto)', type: 'number', default: 0, min: 0, max: 100000, step: 5, hint: '0 = auto-split capital across pool×2 legs. Override to fix the per-side size.' },
  ],
  minting_mm: [
    { key: 'premium_cents', label: 'Premium (cents)', type: 'number', default: 0.02, min: 0, max: 0.2, step: 0.005, hint: 'Min spread above CTF parity to mint' },
    { key: 'max_cycle_usd', label: 'Max cycle ($)', type: 'number', default: 200, min: 10, max: 100000, step: 10 },
    { key: 'cycle_hours', label: 'Cycle hours', type: 'number', default: 24, min: 1, max: 168, step: 1 },
    { key: 'target_apy', label: 'Target APY', type: 'number', default: 0.4, min: 0, max: 5, step: 0.05 },
    { key: 'min_spread', label: 'Min spread', type: 'number', default: 0.04, min: 0, max: 0.3, step: 0.005 },
    { key: 'collateral', label: 'Collateral token', type: 'select', default: '0xUSCD', options: [
      { value: '0xUSCD', label: 'USDC (Polygon)' },
      { value: '0xDAI', label: 'DAI' },
    ]},
  ],
}

// ── Defaults helpers ───────────────────────────────────────────────────────

export function defaultEngineParams(kind: string): Record<string, unknown> {
  const defs = ENGINE_PARAM_DEFS[kind]
  if (!defs) return {}
  return Object.fromEntries(defs.map((f) => [f.key, f.default]))
}

// ── Component ──────────────────────────────────────────────────────────────

interface EngineParamsFormProps {
  kind: string
  params: Record<string, unknown>
  onChange: (params: Record<string, unknown>) => void
}

export default function EngineParamsForm({ kind, params, onChange }: EngineParamsFormProps) {
  const [open, setOpen] = useState(true)
  const defs = ENGINE_PARAM_DEFS[kind]
  if (!defs || defs.length === 0) return null

  function set(key: string, value: unknown) {
    onChange({ ...params, [key]: value })
  }

  return (
    <div
      className="rounded-lg border mt-3"
      style={{ borderColor: 'var(--color-border)', backgroundColor: 'var(--color-surface)' }}
    >
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="w-full flex items-center justify-between px-3 py-2 text-xs font-semibold"
        style={{ color: 'var(--color-text-muted)' }}
      >
        <span>Engine Parameters</span>
        <ChevronDown
          size={12}
          style={{ transform: open ? 'rotate(180deg)' : 'none', transition: 'transform 150ms' }}
        />
      </button>

      {open && (
        <div className="px-3 pb-3 grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-3">
          {defs.map((field) => (
            <div key={field.key}>
              <label
                className="block text-[10px] mb-1 leading-tight"
                style={{ color: 'var(--color-text-muted)' }}
                title={field.hint}
              >
                {field.label}
                {field.hint && (
                  <span className="ml-1 opacity-50" title={field.hint}>?</span>
                )}
              </label>
              {field.type === 'select' ? (
                <select
                  value={String(params[field.key] ?? field.default)}
                  onChange={(e) => set(field.key, e.target.value)}
                  className="w-full rounded px-2 py-1 text-xs"
                  style={{
                    backgroundColor: 'var(--color-base)',
                    border: '1px solid var(--color-border)',
                    color: 'var(--color-text)',
                  }}
                >
                  {field.options!.map((o) => (
                    <option key={o.value} value={o.value}>{o.label}</option>
                  ))}
                </select>
              ) : (
                <input
                  type="number"
                  min={field.min}
                  max={field.max}
                  step={field.step}
                  value={params[field.key] !== undefined ? String(params[field.key]) : String(field.default)}
                  onChange={(e) =>
                    set(field.key, e.target.value === '' ? field.default : Number(e.target.value))
                  }
                  className="w-full rounded px-2 py-1 text-xs font-mono"
                  style={{
                    backgroundColor: 'var(--color-base)',
                    border: '1px solid var(--color-border)',
                    color: 'var(--color-text)',
                  }}
                />
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
