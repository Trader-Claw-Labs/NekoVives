/**
 * Orderbook Archive — pmxt.dev v2
 *
 * Two modes:
 *   1. Remote Query  — DuckDB reads remote Parquet via HTTP (fast, filtered, no local storage).
 *   2. Full Download — Download hourly Parquet files locally for offline pandas analysis.
 *
 * Routes backed by:
 *   POST /api/orderbook/query
 *   POST /api/orderbook/download
 *   GET  /api/orderbook/download/status
 *   POST /api/orderbook/download/cancel
 *   GET  /api/orderbook/files
 */

import { useState, useEffect } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Database,
  Download,
  BarChart3,
  RefreshCw,
  XCircle,
  ChevronDown,
  ChevronUp,
  FileArchive,
  TrendingUp,
  Search,
  Activity,
  Clock,
} from "lucide-react";

// ── API helpers ────────────────────────────────────────────────────────────────

const API = "/api";

function authHeaders(): Record<string, string> {
  const token = localStorage.getItem("auth_token");
  return token ? { Authorization: `Bearer ${token}`, "Content-Type": "application/json" } : { "Content-Type": "application/json" };
}

async function apiGet<T>(path: string): Promise<T> {
  const r = await fetch(`${API}${path}`, { headers: authHeaders() });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
}

async function apiPost<T>(path: string, body: unknown): Promise<T> {
  const r = await fetch(`${API}${path}`, {
    method: "POST",
    headers: authHeaders(),
    body: JSON.stringify(body),
  });
  if (!r.ok) {
    const err = await r.json().catch(() => ({ error: r.statusText }));
    throw new Error(err.error || r.statusText);
  }
  return r.json();
}

// ── Types ──────────────────────────────────────────────────────────────────────

interface Market {
  market: string;
  trade_count?: number;
  total_volume?: number;
  avg_price?: number;
  first_seen?: string;
  last_seen?: string;
}

interface OhlcCandle {
  timestamp_received: string;
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  spread_mean_bps: number;
  mid: number;
}

interface SpreadStats {
  spread_mean_bps: number;
  spread_median_bps: number;
  spread_p95_bps: number;
  best_bid_mean: number;
  best_ask_mean: number;
  price_mean: number;
  price_std: number;
  total_events: number;
}

interface QueryResult {
  mode?: string;
  days?: number;
  markets?: Market[];
  market?: string;
  ohlc?: OhlcCandle[];
  spread_stats?: SpreadStats;
  candle_count?: number;
  row_count?: number;
  event_counts?: Record<string, number>;
  top_markets_by_volume?: Market[];
  error?: string;
}

interface DownloadProgress {
  running: boolean;
  done: number;
  total: number;
  current_hour: string;
  downloaded: number;
  skipped: number;
  errors: string[];
  out_dir: string;
  started_at?: string;
  finished_at?: string;
}

interface LocalFile {
  filename: string;
  hour: string;
  size_mb: number;
}

interface FilesResponse {
  file_count: number;
  total_mb: number;
  files: LocalFile[];
}

// ── Helpers ────────────────────────────────────────────────────────────────────

function shortenMarket(id: string): string {
  if (!id) return "-";
  return id.length > 18 ? `${id.slice(0, 8)}…${id.slice(-6)}` : id;
}

function fmt(n?: number, decimals = 2): string {
  if (n == null || isNaN(n)) return "-";
  return n.toFixed(decimals);
}

function fmtK(n?: number): string {
  if (n == null || isNaN(n)) return "-";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toFixed(0);
}

// Simple inline sparkline from OHLC close prices
function Sparkline({ data }: { data: OhlcCandle[] }) {
  if (!data || data.length < 2) return null;
  const closes = data.map((c) => c.close);
  const min = Math.min(...closes);
  const max = Math.max(...closes);
  const range = max - min || 0.01;
  const w = 200;
  const h = 48;
  const pts = closes.map((c, i) => {
    const x = (i / (closes.length - 1)) * w;
    const y = h - ((c - min) / range) * h;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  const color = closes[closes.length - 1] >= closes[0] ? "#22c55e" : "#ef4444";
  return (
    <svg width={w} height={h} className="block">
      <polyline points={pts.join(" ")} fill="none" stroke={color} strokeWidth="1.5" />
    </svg>
  );
}

// ── Mode tabs ──────────────────────────────────────────────────────────────────

type QueryMode = "top-markets" | "price-series" | "spread-stats" | "summary";
type PageTab = "query" | "download" | "files";

// ── Main component ─────────────────────────────────────────────────────────────

export default function Orderbook() {
  const queryClient = useQueryClient();

  // ── Tab state ──
  const [tab, setTab] = useState<PageTab>("query");

  // ── Query form state ──
  const [queryMode, setQueryMode] = useState<QueryMode>("top-markets");
  const [days, setDays] = useState(1);
  const [marketId, setMarketId] = useState("");
  const [candleFreq, setCandleFreq] = useState("5min");
  const [queryResult, setQueryResult] = useState<QueryResult | null>(null);
  const [queryError, setQueryError] = useState("");
  const [isQuerying, setIsQuerying] = useState(false);

  // ── Download form state ──
  const [dlDays, setDlDays] = useState(15);
  const [dlMarket, setDlMarket] = useState("");

  // ── Progress polling ──
  const { data: progress, refetch: refetchProgress } = useQuery<DownloadProgress>({
    queryKey: ["orderbook-download-status"],
    queryFn: () => apiGet("/orderbook/download/status"),
    refetchInterval: (q) => (q.state.data?.running ? 3000 : false),
    staleTime: 0,
  });

  // ── Local files ──
  const { data: filesData, refetch: refetchFiles } = useQuery<FilesResponse>({
    queryKey: ["orderbook-files"],
    queryFn: () => apiGet("/orderbook/files"),
    enabled: tab === "files",
    staleTime: 30_000,
  });

  // ── Mutations ──
  const downloadMut = useMutation({
    mutationFn: (body: { days: number; market?: string }) =>
      apiPost("/orderbook/download", body),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["orderbook-download-status"] });
    },
  });

  const cancelMut = useMutation({
    mutationFn: () => apiPost("/orderbook/download/cancel", {}),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["orderbook-download-status"] });
    },
  });

  // ── Query runner ──
  async function runQuery() {
    setIsQuerying(true);
    setQueryError("");
    setQueryResult(null);
    try {
      const body: Record<string, unknown> = { days, mode: queryMode };
      if (marketId.trim()) body.market = marketId.trim();
      if (queryMode === "price-series") body.freq = candleFreq;
      const res = await apiPost<QueryResult>("/orderbook/query", body);
      if (res.error) {
        setQueryError(res.error);
      } else {
        setQueryResult(res);
      }
    } catch (e: unknown) {
      setQueryError(e instanceof Error ? e.message : "Query failed");
    } finally {
      setIsQuerying(false);
    }
  }

  // ── Progress percentage ──
  const pct =
    progress && progress.total > 0
      ? Math.round((progress.done / progress.total) * 100)
      : 0;

  return (
    <div className="p-6 space-y-6 max-w-6xl mx-auto">
      {/* Header */}
      <div className="flex items-center gap-3">
        <Database className="w-7 h-7 text-accent" />
        <div>
          <h1 className="text-2xl font-bold">Orderbook Archive</h1>
          <p className="text-sm text-muted-foreground">
            pmxt.dev v2 — Hourly Parquet snapshots of the Polymarket CLOB event stream
          </p>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 border-b border-border">
        {(["query", "download", "files"] as PageTab[]).map((t) => (
          <button
            key={t}
            onClick={() => { setTab(t); if (t === "files") refetchFiles(); }}
            className={`px-4 py-2 text-sm font-medium capitalize rounded-t transition-colors ${
              tab === t
                ? "border-b-2 border-accent text-accent bg-accent/5"
                : "text-muted-foreground hover:text-foreground"
            }`}
          >
            {t === "query" && <span className="flex items-center gap-1"><Search className="w-3.5 h-3.5" />{" "}Remote Query</span>}
            {t === "download" && <span className="flex items-center gap-1"><Download className="w-3.5 h-3.5" />{" "}Download</span>}
            {t === "files" && <span className="flex items-center gap-1"><FileArchive className="w-3.5 h-3.5" />{" "}Local Files</span>}
          </button>
        ))}
      </div>

      {/* ── REMOTE QUERY TAB ── */}
      {tab === "query" && (
        <div className="space-y-6">
          {/* Info banner */}
          <div className="rounded-lg bg-blue-950/30 border border-blue-800/40 p-3 text-sm text-blue-200 flex gap-2">
            <Activity className="w-4 h-4 mt-0.5 shrink-0 text-blue-400" />
            <span>
              DuckDB reads remote Parquet via HTTP with predicate pushdown — no local storage
              needed. Files are 100–400 MB/hr; filtering by market reduces bandwidth significantly.
              Archive starts <strong>2026-04-13</strong>.
            </span>
          </div>

          {/* Form */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 bg-card border border-border rounded-xl p-5">
            {/* Mode */}
            <div>
              <label className="text-xs font-medium text-muted-foreground block mb-1">Analysis Mode</label>
              <select
                className="w-full bg-background border border-border rounded-lg px-3 py-2 text-sm"
                value={queryMode}
                onChange={(e) => setQueryMode(e.target.value as QueryMode)}
              >
                <option value="top-markets">Top Markets by Volume</option>
                <option value="price-series">Price Series (OHLC)</option>
                <option value="spread-stats">Spread Statistics</option>
                <option value="summary">Archive Summary</option>
              </select>
            </div>

            {/* Days */}
            <div>
              <label className="text-xs font-medium text-muted-foreground block mb-1">
                Past Days to Query <span className="text-yellow-400">(1 day ≈ 24 files × 200 MB)</span>
              </label>
              <input
                type="number"
                min={1}
                max={30}
                value={days}
                onChange={(e) => setDays(Number(e.target.value))}
                className="w-full bg-background border border-border rounded-lg px-3 py-2 text-sm"
              />
            </div>

            {/* Market ID (conditional) */}
            {(queryMode === "price-series" || queryMode === "spread-stats") && (
              <div className="md:col-span-2">
                <label className="text-xs font-medium text-muted-foreground block mb-1">
                  Market Condition ID (0x…) — required
                </label>
                <input
                  type="text"
                  placeholder="0x1234abcd…"
                  value={marketId}
                  onChange={(e) => setMarketId(e.target.value)}
                  className="w-full bg-background border border-border rounded-lg px-3 py-2 text-sm font-mono"
                />
                <p className="text-xs text-muted-foreground mt-1">
                  Find IDs via "Top Markets" mode first, then copy the condition ID.
                </p>
              </div>
            )}

            {/* Candle freq */}
            {queryMode === "price-series" && (
              <div>
                <label className="text-xs font-medium text-muted-foreground block mb-1">Candle Frequency</label>
                <select
                  className="w-full bg-background border border-border rounded-lg px-3 py-2 text-sm"
                  value={candleFreq}
                  onChange={(e) => setCandleFreq(e.target.value)}
                >
                  <option value="1min">1 minute</option>
                  <option value="5min">5 minutes</option>
                  <option value="15min">15 minutes</option>
                  <option value="1h">1 hour</option>
                </select>
              </div>
            )}

            {/* Run button */}
            <div className="md:col-span-2 flex items-center gap-3">
              <button
                onClick={runQuery}
                disabled={isQuerying}
                className="flex items-center gap-2 px-5 py-2.5 bg-accent text-accent-foreground rounded-lg text-sm font-medium hover:bg-accent/80 disabled:opacity-50 transition-colors"
              >
                {isQuerying ? (
                  <RefreshCw className="w-4 h-4 animate-spin" />
                ) : (
                  <BarChart3 className="w-4 h-4" />
                )}
                {isQuerying ? "Querying…" : "Run Query"}
              </button>
              {isQuerying && (
                <span className="text-xs text-muted-foreground">
                  DuckDB is reading remote Parquet files — may take 30–90 seconds for multi-day queries.
                </span>
              )}
            </div>
          </div>

          {/* Error */}
          {queryError && (
            <div className="rounded-lg bg-red-950/30 border border-red-800 p-4 text-sm text-red-300">
              <strong>Error:</strong> {queryError}
            </div>
          )}

          {/* Results */}
          {queryResult && !queryResult.error && (
            <QueryResults result={queryResult} onSelectMarket={(m) => { setMarketId(m); setQueryMode("price-series"); }} />
          )}
        </div>
      )}

      {/* ── DOWNLOAD TAB ── */}
      {tab === "download" && (
        <div className="space-y-6">
          <div className="rounded-lg bg-yellow-950/30 border border-yellow-700/40 p-3 text-sm text-yellow-200 flex gap-2">
            <Download className="w-4 h-4 mt-0.5 shrink-0 text-yellow-400" />
            <span>
              Downloads hourly Parquet files locally for offline pandas analysis.{" "}
              <strong>15 days ≈ 360 files × 100–400 MB = 36–144 GB.</strong> Filter by market
              to only store data for specific condition IDs (much smaller).
            </span>
          </div>

          {/* Form */}
          <div className="bg-card border border-border rounded-xl p-5 space-y-4">
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="text-xs font-medium text-muted-foreground block mb-1">Days to Download</label>
                <input
                  type="number"
                  min={1}
                  max={30}
                  value={dlDays}
                  onChange={(e) => setDlDays(Number(e.target.value))}
                  className="w-full bg-background border border-border rounded-lg px-3 py-2 text-sm"
                />
              </div>
              <div>
                <label className="text-xs font-medium text-muted-foreground block mb-1">
                  Market Filter (optional — saves bandwidth)
                </label>
                <input
                  type="text"
                  placeholder="0x… condition ID or leave blank for all markets"
                  value={dlMarket}
                  onChange={(e) => setDlMarket(e.target.value)}
                  className="w-full bg-background border border-border rounded-lg px-3 py-2 text-sm font-mono"
                />
              </div>
            </div>

            <div className="flex gap-3">
              <button
                onClick={() =>
                  downloadMut.mutate({ days: dlDays, market: dlMarket.trim() || undefined })
                }
                disabled={downloadMut.isPending || progress?.running}
                className="flex items-center gap-2 px-5 py-2.5 bg-accent text-accent-foreground rounded-lg text-sm font-medium hover:bg-accent/80 disabled:opacity-50 transition-colors"
              >
                <Download className="w-4 h-4" />
                {progress?.running ? "Running…" : "Start Download"}
              </button>

              {progress?.running && (
                <button
                  onClick={() => cancelMut.mutate()}
                  className="flex items-center gap-2 px-4 py-2.5 border border-red-700 text-red-400 rounded-lg text-sm hover:bg-red-950/30 transition-colors"
                >
                  <XCircle className="w-4 h-4" />
                  Cancel
                </button>
              )}
            </div>

            {downloadMut.isError && (
              <div className="text-sm text-red-400">{(downloadMut.error as Error).message}</div>
            )}
          </div>

          {/* Progress */}
          {progress && (progress.running || progress.downloaded > 0 || progress.skipped > 0) && (
            <DownloadProgressCard progress={progress} pct={pct} onRefresh={refetchProgress} />
          )}

          {/* Usage hint */}
          <div className="bg-card border border-border rounded-xl p-5 space-y-2">
            <h3 className="text-sm font-semibold flex items-center gap-2">
              <TrendingUp className="w-4 h-4 text-accent" />
              Analyze with Python
            </h3>
            <pre className="text-xs bg-background rounded-lg p-3 overflow-x-auto text-green-300">
{`# After download, use the bundled parser for pandas analysis:
python tools/orderbook_parser.py analyze-local \\
  --dir ~/.traderclaw/workspace/data/orderbook

# Or query a specific market:
python tools/orderbook_parser.py price-series \\
  --market 0xABC... --days 7

# Export OHLC to CSV:
python tools/orderbook_parser.py price-series \\
  --market 0xABC... --days 7 \\
  | python -c "import sys,json,csv; d=json.load(sys.stdin); w=csv.DictWriter(sys.stdout, d['ohlc'][0].keys()); w.writeheader(); w.writerows(d['ohlc'])"`}
            </pre>
          </div>
        </div>
      )}

      {/* ── FILES TAB ── */}
      {tab === "files" && (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold text-muted-foreground">
              {filesData
                ? `${filesData.file_count} file(s) — ${filesData.total_mb.toFixed(1)} MB total`
                : "Loading…"}
            </h2>
            <button
              onClick={() => refetchFiles()}
              className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            >
              <RefreshCw className="w-3 h-3" /> Refresh
            </button>
          </div>

          {filesData && filesData.file_count === 0 && (
            <div className="text-center py-12 text-muted-foreground text-sm">
              No local Parquet files yet. Go to the <strong>Download</strong> tab to fetch data.
            </div>
          )}

          {filesData && filesData.files.length > 0 && (
            <div className="bg-card border border-border rounded-xl overflow-hidden">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-border text-xs text-muted-foreground">
                    <th className="text-left px-4 py-2.5">Hour (UTC)</th>
                    <th className="text-right px-4 py-2.5">Size (MB)</th>
                    <th className="text-left px-4 py-2.5">Filename</th>
                  </tr>
                </thead>
                <tbody>
                  {filesData.files.map((f) => (
                    <tr key={f.filename} className="border-b border-border/50 hover:bg-accent/5">
                      <td className="px-4 py-2 font-mono text-xs">{f.hour}</td>
                      <td className="px-4 py-2 text-right tabular-nums">{f.size_mb.toFixed(1)}</td>
                      <td className="px-4 py-2 text-muted-foreground text-xs">{f.filename}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ── Sub-components ─────────────────────────────────────────────────────────────

function QueryResults({
  result,
  onSelectMarket,
}: {
  result: QueryResult;
  onSelectMarket: (id: string) => void;
}) {
  const [showRaw, setShowRaw] = useState(false);

  // Top markets
  const markets = result.markets || result.top_markets_by_volume || [];

  return (
    <div className="space-y-4">
      {/* Summary stats */}
      {result.event_counts && (
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          {Object.entries(result.event_counts).map(([type, count]) => (
            <div key={type} className="bg-card border border-border rounded-xl p-3">
              <div className="text-xs text-muted-foreground capitalize">{type.replace(/_/g, " ")}</div>
              <div className="text-lg font-bold tabular-nums">{fmtK(count)}</div>
            </div>
          ))}
        </div>
      )}

      {/* Spread stats */}
      {result.spread_stats && (
        <div className="bg-card border border-border rounded-xl p-5 space-y-3">
          <h3 className="text-sm font-semibold">Spread Statistics</h3>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-3 text-sm">
            <Stat label="Mean Spread (bps)" value={fmt(result.spread_stats.spread_mean_bps)} />
            <Stat label="Median Spread (bps)" value={fmt(result.spread_stats.spread_median_bps)} />
            <Stat label="P95 Spread (bps)" value={fmt(result.spread_stats.spread_p95_bps)} />
            <Stat label="Events" value={fmtK(result.spread_stats.total_events)} />
            <Stat label="Avg Best Bid" value={fmt(result.spread_stats.best_bid_mean, 4)} />
            <Stat label="Avg Best Ask" value={fmt(result.spread_stats.best_ask_mean, 4)} />
            <Stat label="Avg Price" value={fmt(result.spread_stats.price_mean, 4)} />
            <Stat label="Price Std Dev" value={fmt(result.spread_stats.price_std, 4)} />
          </div>
        </div>
      )}

      {/* OHLC Chart */}
      {result.ohlc && result.ohlc.length > 0 && (
        <div className="bg-card border border-border rounded-xl p-5 space-y-3">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold">
              Price Series — {result.candle_count} candles ({result.row_count?.toLocaleString()} events)
            </h3>
          </div>
          <Sparkline data={result.ohlc} />
          <OhlcTable candles={result.ohlc.slice(-20)} />
        </div>
      )}

      {/* Top markets table */}
      {markets.length > 0 && (
        <div className="bg-card border border-border rounded-xl overflow-hidden">
          <div className="px-4 py-3 border-b border-border">
            <h3 className="text-sm font-semibold">Top Markets by Volume ({markets.length})</h3>
            <p className="text-xs text-muted-foreground mt-0.5">Click a row to analyze its price series</p>
          </div>
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-border text-xs text-muted-foreground">
                <th className="text-left px-4 py-2">Market ID</th>
                <th className="text-right px-4 py-2">Trades</th>
                <th className="text-right px-4 py-2">Volume</th>
                <th className="text-right px-4 py-2">Avg Price</th>
                <th className="text-left px-4 py-2">Last Seen</th>
              </tr>
            </thead>
            <tbody>
              {markets.map((m) => (
                <tr
                  key={m.market}
                  className="border-b border-border/50 hover:bg-accent/5 cursor-pointer"
                  onClick={() => onSelectMarket(m.market)}
                  title="Click to analyze price series for this market"
                >
                  <td className="px-4 py-2 font-mono text-xs text-accent">{shortenMarket(m.market)}</td>
                  <td className="px-4 py-2 text-right tabular-nums">{fmtK(m.trade_count)}</td>
                  <td className="px-4 py-2 text-right tabular-nums">{fmtK(m.total_volume)}</td>
                  <td className="px-4 py-2 text-right tabular-nums">{fmt(m.avg_price, 4)}</td>
                  <td className="px-4 py-2 text-xs text-muted-foreground">
                    {m.last_seen ? new Date(m.last_seen).toLocaleString() : "-"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Raw JSON toggle */}
      <div>
        <button
          onClick={() => setShowRaw((v) => !v)}
          className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
        >
          {showRaw ? <ChevronUp className="w-3 h-3" /> : <ChevronDown className="w-3 h-3" />}
          {showRaw ? "Hide" : "Show"} raw JSON
        </button>
        {showRaw && (
          <pre className="mt-2 text-xs bg-background rounded-lg p-3 overflow-x-auto max-h-64 text-green-300">
            {JSON.stringify(result, null, 2)}
          </pre>
        )}
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="font-semibold tabular-nums">{value}</div>
    </div>
  );
}

function OhlcTable({ candles }: { candles: OhlcCandle[] }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-xs">
        <thead>
          <tr className="text-muted-foreground border-b border-border">
            <th className="text-left py-1 pr-3">Time</th>
            <th className="text-right pr-3">Open</th>
            <th className="text-right pr-3">High</th>
            <th className="text-right pr-3">Low</th>
            <th className="text-right pr-3">Close</th>
            <th className="text-right pr-3">Volume</th>
            <th className="text-right">Spread bps</th>
          </tr>
        </thead>
        <tbody>
          {candles.map((c, i) => {
            const up = c.close >= c.open;
            return (
              <tr key={i} className="border-b border-border/30">
                <td className="py-0.5 pr-3 font-mono text-muted-foreground">
                  {new Date(c.timestamp_received).toLocaleTimeString()}
                </td>
                <td className="text-right pr-3 tabular-nums">{c.open?.toFixed(4)}</td>
                <td className="text-right pr-3 tabular-nums text-green-400">{c.high?.toFixed(4)}</td>
                <td className="text-right pr-3 tabular-nums text-red-400">{c.low?.toFixed(4)}</td>
                <td className={`text-right pr-3 tabular-nums font-medium ${up ? "text-green-400" : "text-red-400"}`}>
                  {c.close?.toFixed(4)}
                </td>
                <td className="text-right pr-3 tabular-nums">{fmtK(c.volume)}</td>
                <td className="text-right tabular-nums text-muted-foreground">{fmt(c.spread_mean_bps)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function DownloadProgressCard({
  progress,
  pct,
  onRefresh,
}: {
  progress: DownloadProgress;
  pct: number;
  onRefresh: () => void;
}) {
  return (
    <div className="bg-card border border-border rounded-xl p-5 space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold flex items-center gap-2">
          <Clock className="w-4 h-4 text-accent" />
          Download Progress
          {progress.running && <span className="text-xs text-yellow-400 animate-pulse">● Running</span>}
          {!progress.running && progress.finished_at && (
            <span className="text-xs text-green-400">✓ Finished</span>
          )}
        </h3>
        <button onClick={onRefresh} className="text-xs text-muted-foreground hover:text-foreground flex items-center gap-1">
          <RefreshCw className="w-3 h-3" /> Refresh
        </button>
      </div>

      {/* Progress bar */}
      <div className="space-y-1">
        <div className="flex justify-between text-xs text-muted-foreground">
          <span>{progress.done} / {progress.total} hours</span>
          <span>{pct}%</span>
        </div>
        <div className="h-2 bg-background rounded-full overflow-hidden">
          <div
            className="h-full bg-accent transition-all duration-500"
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>

      {/* Stats row */}
      <div className="grid grid-cols-3 gap-3 text-sm">
        <div>
          <div className="text-xs text-muted-foreground">Downloaded</div>
          <div className="font-semibold text-green-400">{progress.downloaded}</div>
        </div>
        <div>
          <div className="text-xs text-muted-foreground">Skipped (cached)</div>
          <div className="font-semibold text-blue-400">{progress.skipped}</div>
        </div>
        <div>
          <div className="text-xs text-muted-foreground">Errors</div>
          <div className={`font-semibold ${progress.errors.length > 0 ? "text-red-400" : "text-muted-foreground"}`}>
            {progress.errors.length}
          </div>
        </div>
      </div>

      {progress.current_hour && (
        <div className="text-xs text-muted-foreground font-mono">
          Current: {progress.current_hour}
        </div>
      )}

      {progress.errors.length > 0 && (
        <div className="text-xs text-red-400 bg-red-950/30 rounded p-2 space-y-0.5">
          {progress.errors.map((e, i) => <div key={i}>{e}</div>)}
        </div>
      )}

      {progress.out_dir && (
        <div className="text-xs text-muted-foreground">
          Output: <span className="font-mono">{progress.out_dir}</span>
        </div>
      )}
    </div>
  );
}
