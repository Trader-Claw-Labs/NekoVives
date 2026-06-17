import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { apiFetch, apiDelete } from '../hooks/useApi'
import { RefreshCw, X, TrendingUp } from 'lucide-react'

interface Order {
  id: string; price: string; side: string; size: string; status: string; token_id: string
}
interface Balance { balance: number; positions_value: number | null; total_equity: number; currency: string }

export default function RewardsPositions() {
  const qc = useQueryClient()

  const { data: ordersData, isFetching: loadingOrders, refetch } = useQuery<{ orders: Order[] }>({
    queryKey: ['poly-orders-rewards'],
    queryFn: () => apiFetch('/api/polymarket/orders'),
    staleTime: 30_000,
    refetchInterval: 60_000,
  })

  const { data: balData } = useQuery<Balance>({
    queryKey: ['poly-balance-rewards'],
    queryFn: () => apiFetch('/api/polymarket/balance'),
    staleTime: 30_000,
    refetchInterval: 60_000,
  })

  const { data: historyData } = useQuery<{ status: string; history?: { date: string; delta: number; balance_end: number }[] }>({
    queryKey: ['rewards-history'],
    queryFn: () => apiFetch('/api/rewards/history'),
    staleTime: 5 * 60_000,
  })

  const cancelMutation = useMutation({
    mutationFn: (id: string) => apiDelete(`/api/polymarket/order/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['poly-orders-rewards'] }),
  })

  const orders = ordersData?.orders ?? []
  const live = orders.filter(o => o.status === 'LIVE')

  return (
    <div className="rounded-lg border p-4 mb-4"
      style={{ background: 'var(--color-surface)', borderColor: 'var(--color-border)' }}>
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-2">
          <TrendingUp size={15} style={{ color: 'var(--color-accent)' }} />
          <span className="text-sm font-semibold" style={{ color: 'var(--color-text)' }}>
            Active Maker Quotes
          </span>
        </div>
        <div className="flex items-center gap-3">
          {balData && (
            <span className="text-xs font-mono" style={{ color: 'var(--color-accent)' }}>
              {balData.positions_value != null && balData.positions_value > 0.01 ? (
                <span title="liquid USDC + open-position value">
                  ${balData.total_equity.toFixed(2)} equity
                  <span style={{ color: 'var(--color-text-muted)' }}>
                    {' '}(${balData.balance.toFixed(2)} free + ${balData.positions_value.toFixed(2)} in positions)
                  </span>
                </span>
              ) : (
                <>${balData.balance.toFixed(2)} USDC free</>
              )}
            </span>
          )}
          <button onClick={() => refetch()}
            className="p-1 rounded hover:bg-white/10"
            style={{ color: 'var(--color-text-muted)' }}>
            <RefreshCw size={13} className={loadingOrders ? 'animate-spin' : ''} />
          </button>
        </div>
      </div>

      {live.length === 0 ? (
        <p className="text-xs text-center py-3" style={{ color: 'var(--color-text-muted)' }}>
          No resting quotes. Post two-sided limit orders using the market scanner above.
        </p>
      ) : (
        <div className="space-y-1.5">
          {live.map(o => (
            <div key={o.id} className="flex items-center gap-3 text-xs px-2 py-1.5 rounded"
              style={{ background: 'var(--color-surface-2)' }}>
              <span className="font-mono font-semibold w-10"
                style={{ color: o.side === 'BUY' ? 'var(--color-accent)' : '#f87171' }}>
                {o.side}
              </span>
              <span className="font-mono text-[11px]" style={{ color: 'var(--color-text)' }}>
                {parseFloat(o.price).toFixed(3)}
              </span>
              <span style={{ color: 'var(--color-text-muted)' }}>
                {o.size} shares
              </span>
              <span className="ml-auto text-[10px] px-1.5 py-0.5 rounded"
                style={{ background: 'rgba(74,222,128,0.1)', color: '#4ade80' }}>
                {o.status}
              </span>
              <button
                onClick={() => cancelMutation.mutate(o.id)}
                disabled={cancelMutation.isPending}
                className="p-1 rounded hover:bg-white/10"
                style={{ color: 'var(--color-text-muted)' }}
                title="Cancel order">
                <X size={12} />
              </button>
            </div>
          ))}
        </div>
      )}

      {/* Daily reward proxy: USDC balance delta across UTC midnight */}
      {historyData?.history && historyData.history.length > 0 && (
        <div className="mt-3 pt-3 border-t" style={{ borderColor: 'var(--color-border)' }}>
          <div className="text-[11px] font-semibold mb-1.5" style={{ color: 'var(--color-text)' }}>
            Daily balance delta (reward proxy)
          </div>
          <div className="space-y-0.5">
            {historyData.history.slice(-7).reverse().map(h => (
              <div key={h.date} className="flex items-center justify-between text-[11px]">
                <span style={{ color: 'var(--color-text-muted)' }}>{h.date}</span>
                <span className="font-mono" style={{ color: h.delta >= 0 ? 'var(--color-accent)' : '#f87171' }}>
                  {h.delta >= 0 ? '+' : ''}{h.delta.toFixed(2)} USDC
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      <p className="text-[10px] mt-2" style={{ color: 'var(--color-text-muted)' }}>
        Reward payout: daily at midnight UTC to your proxy wallet. The daily delta above is
        the balance change (≈ rewards minus fill P&L). No public rewards API exists — this is
        the onchain proxy.
      </p>
    </div>
  )
}
