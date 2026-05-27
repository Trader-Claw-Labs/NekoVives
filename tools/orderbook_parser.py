#!/usr/bin/env python3
"""
Polymarket Orderbook Archive Parser  (pmxt.dev v2)
====================================================
Queries hourly Parquet files from the public archive using DuckDB.
DuckDB reads the remote Parquet via HTTP and pushes predicates down to
only fetch the relevant row-groups, keeping bandwidth manageable.

Usage (CLI):
    python tools/orderbook_parser.py summary --days 1
    python tools/orderbook_parser.py price-series --market 0xABC...  --days 3
    python tools/orderbook_parser.py top-markets --days 1 --limit 20
    python tools/orderbook_parser.py spread-stats --market 0xABC... --days 2
    python tools/orderbook_parser.py download --days 15 --out /path/to/data/orderbook
    python tools/orderbook_parser.py analyze-local --dir /path/to/data/orderbook

All commands output JSON (or save files) so they can be called from Rust
via std::process::Command and parsed easily.
"""

from __future__ import annotations
import sys
import json
import argparse
import os
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Optional
import traceback

try:
    import duckdb
    import pandas as pd
    import numpy as _np
    import urllib.request
    import urllib.error
except ImportError as e:
    print(json.dumps({"error": f"Missing dependency: {e}. Run: pip install duckdb pandas pyarrow"}))
    sys.exit(1)

# ── Constants ──────────────────────────────────────────────────────────────────

BASE_URL = "https://r2v2.pmxt.dev"
ARCHIVE_START = datetime(2026, 4, 13, 19, tzinfo=timezone.utc)

# ── URL helpers ────────────────────────────────────────────────────────────────

def parquet_url(dt: datetime) -> str:
    """Return the pmxt.dev v2 URL for the given UTC hour."""
    return f"{BASE_URL}/polymarket_orderbook_{dt.strftime('%Y-%m-%dT%H')}.parquet"


def urls_for_range(days: int, end: datetime | None = None) -> list[str]:
    """Return hourly URLs for the last `days` days (most recent first)."""
    if end is None:
        end = datetime.now(tz=timezone.utc).replace(minute=0, second=0, microsecond=0)
    start = end - timedelta(days=days)
    # Clamp to archive start
    if start < ARCHIVE_START:
        start = ARCHIVE_START
    urls: list[str] = []
    cur = start
    while cur < end:
        urls.append(parquet_url(cur))
        cur += timedelta(hours=1)
    return urls


def format_url_list(urls: list[str]) -> str:
    """Format list of URLs for DuckDB read_parquet(...)."""
    return "[" + ", ".join(f"'{u}'" for u in urls) + "]"


def filter_available_urls(urls: list[str], max_check: int = 48) -> list[str]:
    """
    Filter the URL list to only those that exist in the archive.
    Uses Range GET (bytes 0-0) since the archive doesn't support HEAD.
    For large lists we check only the first `max_check` URLs to bound latency.
    """
    import concurrent.futures

    HEADERS = {"User-Agent": "orderbook-parser/1.0", "Range": "bytes=0-0"}

    def check_url(url: str) -> bool:
        try:
            req = urllib.request.Request(url, headers=HEADERS)
            with urllib.request.urlopen(req, timeout=8) as r:
                return r.status in (200, 206)
        except Exception:
            return False

    to_check = urls[:max_check]
    with concurrent.futures.ThreadPoolExecutor(max_workers=12) as ex:
        results = list(ex.map(check_url, to_check))
    available_sample = [u for u, ok in zip(to_check, results) if ok]

    if max_check >= len(urls):
        return available_sample

    # Check the rest too
    rest = urls[max_check:]
    with concurrent.futures.ThreadPoolExecutor(max_workers=12) as ex:
        rest_results = list(ex.map(check_url, rest))
    available_rest = [u for u, ok in zip(rest, rest_results) if ok]
    return available_sample + available_rest


# ── DuckDB connection (singleton per process) ──────────────────────────────────

def get_con() -> duckdb.DuckDBPyConnection:
    # custom_user_agent must be set at connection time (cannot SET after open).
    # R2 returns 403 without a User-Agent header.
    try:
        con = duckdb.connect(config={"custom_user_agent": "orderbook-parser/1.0"})
    except Exception:
        con = duckdb.connect()  # older DuckDB — proceed without custom UA

    con.execute("SET threads=4")
    con.execute("SET memory_limit='1GB'")
    # Reliability settings for large remote files
    try:
        con.execute("SET http_retries=3")
        con.execute("SET http_timeout=120000")    # 120 s per HTTP request
        con.execute("SET http_retry_wait_ms=2000")
    except Exception:
        pass
    return con


# ── Query helpers ──────────────────────────────────────────────────────────────

def query_price_changes(
    urls: list[str],
    market_id: Optional[str] = None,
    asset_id: Optional[str] = None,
    limit: int = 500_000,
) -> pd.DataFrame:
    """
    Fetch price_change events from remote Parquet files.
    DuckDB pushes predicates to row-group level (files are sorted by market, asset_id).
    """
    if not urls:
        return pd.DataFrame()

    urls = filter_available_urls(urls)
    if not urls:
        return pd.DataFrame()

    con = get_con()
    url_expr = format_url_list(urls)
    where_parts = ["event_type = 'price_change'"]
    if market_id:
        # market column is fixed_size_binary[66] — compare as string
        where_parts.append(f"CAST(market AS VARCHAR) = '{market_id}'")
    if asset_id:
        where_parts.append(f"asset_id = '{asset_id}'")
    where = " AND ".join(where_parts)

    sql = f"""
    SELECT
        timestamp_received,
        timestamp,
        CAST(market AS VARCHAR) AS market,
        asset_id,
        CAST(price AS DOUBLE) AS price,
        CAST(size AS DOUBLE) AS size,
        side,
        CAST(best_bid AS DOUBLE) AS best_bid,
        CAST(best_ask AS DOUBLE) AS best_ask
    FROM read_parquet({url_expr}, hive_partitioning=false, union_by_name=true)
    WHERE {where}
    ORDER BY timestamp_received
    LIMIT {limit}
    """
    return con.execute(sql).df()


def query_trades(
    urls: list[str],
    market_id: Optional[str] = None,
    limit: int = 100_000,
) -> pd.DataFrame:
    """Fetch last_trade_price events (actual executed trades)."""
    if not urls:
        return pd.DataFrame()

    urls = filter_available_urls(urls)
    if not urls:
        return pd.DataFrame()

    con = get_con()
    url_expr = format_url_list(urls)
    where_parts = ["event_type = 'last_trade_price'"]
    if market_id:
        where_parts.append(f"CAST(market AS VARCHAR) = '{market_id}'")
    where = " AND ".join(where_parts)

    sql = f"""
    SELECT
        timestamp_received,
        timestamp,
        CAST(market AS VARCHAR) AS market,
        asset_id,
        CAST(price AS DOUBLE) AS price,
        CAST(size AS DOUBLE) AS size,
        side,
        transaction_hash,
        CAST(fee_rate_bps AS INTEGER) AS fee_rate_bps
    FROM read_parquet({url_expr}, hive_partitioning=false, union_by_name=true)
    WHERE {where}
    ORDER BY timestamp_received
    LIMIT {limit}
    """
    return con.execute(sql).df()


def query_top_markets(
    urls: list[str],
    limit: int = 30,
) -> pd.DataFrame:
    """Rank markets by trade volume (last_trade_price event count)."""
    if not urls:
        return pd.DataFrame()

    urls = filter_available_urls(urls)
    if not urls:
        return pd.DataFrame()

    con = get_con()
    url_expr = format_url_list(urls)

    sql = f"""
    SELECT
        CAST(market AS VARCHAR) AS market,
        COUNT(*) AS trade_count,
        SUM(CAST(size AS DOUBLE)) AS total_volume,
        AVG(CAST(price AS DOUBLE)) AS avg_price,
        MIN(timestamp_received) AS first_seen,
        MAX(timestamp_received) AS last_seen
    FROM read_parquet({url_expr}, hive_partitioning=false, union_by_name=true)
    WHERE event_type = 'last_trade_price'
    GROUP BY market
    ORDER BY total_volume DESC NULLS LAST
    LIMIT {limit}
    """
    return con.execute(sql).df()


def query_archive_summary(urls: list[str]) -> dict:
    """High-level archive summary: event counts by type."""
    if not urls:
        return {}

    urls = filter_available_urls(urls)
    if not urls:
        return {}

    con = get_con()
    url_expr = format_url_list(urls)

    sql = f"""
    SELECT
        event_type,
        COUNT(*) AS count
    FROM read_parquet({url_expr}, hive_partitioning=false, union_by_name=true)
    GROUP BY event_type
    ORDER BY count DESC
    """
    df = con.execute(sql).df()
    return df.set_index("event_type")["count"].to_dict()


# ── Analytics (pandas) ─────────────────────────────────────────────────────────

def build_ohlc(df: pd.DataFrame, freq: str = "5min") -> pd.DataFrame:
    """
    Build OHLC candles from price_change events.
    Input: DataFrame with [timestamp_received, price, size, side, best_bid, best_ask].
    Output: OHLC with spread and volume columns.
    """
    if df.empty:
        return pd.DataFrame()

    df = df.copy()
    df["timestamp_received"] = pd.to_datetime(df["timestamp_received"], utc=True)
    df = df.set_index("timestamp_received").sort_index()

    # YES token price from best_bid/best_ask midpoint
    df["mid"] = (df["best_bid"].fillna(df["price"]) + df["best_ask"].fillna(df["price"])) / 2
    df["spread_bps"] = ((df["best_ask"] - df["best_bid"]) / df["best_ask"]).abs() * 10000

    ohlc = df["price"].resample(freq).ohlc()
    ohlc["volume"] = df["size"].resample(freq).sum()
    ohlc["spread_mean_bps"] = df["spread_bps"].resample(freq).mean()
    ohlc["mid"] = df["mid"].resample(freq).last()
    ohlc = ohlc.dropna(subset=["open"])
    ohlc = ohlc.reset_index()
    ohlc["timestamp_received"] = ohlc["timestamp_received"].dt.isoformat()
    return ohlc


def compute_spread_stats(df: pd.DataFrame) -> dict:
    """Compute spread statistics for a market."""
    if df.empty:
        return {}

    df = df.copy()
    df["spread"] = df["best_ask"] - df["best_bid"]
    df["spread_bps"] = (df["spread"] / df["best_ask"].replace(0, float("nan"))).abs() * 10000

    return {
        "spread_mean_bps": float(df["spread_bps"].mean()),
        "spread_median_bps": float(df["spread_bps"].median()),
        "spread_p95_bps": float(df["spread_bps"].quantile(0.95)),
        "spread_min": float(df["spread"].min()),
        "spread_max": float(df["spread"].max()),
        "best_bid_mean": float(df["best_bid"].mean()),
        "best_ask_mean": float(df["best_ask"].mean()),
        "price_mean": float(df["price"].mean()),
        "price_std": float(df["price"].std()),
        "total_events": len(df),
    }


def compute_volume_profile(trade_df: pd.DataFrame, bins: int = 20) -> list[dict]:
    """Price → volume distribution from trade events."""
    if trade_df.empty:
        return []

    df = trade_df.dropna(subset=["price", "size"])
    hist, edges = pd.cut(df["price"], bins=bins, retbins=True)
    vol = df.groupby(hist, observed=False)["size"].sum()
    result = []
    for i, (bucket, volume) in enumerate(vol.items()):
        result.append({
            "price_low": float(edges[i]),
            "price_high": float(edges[i + 1]),
            "volume": float(volume) if pd.notna(volume) else 0.0,
        })
    return result


def analyze_drift(df: pd.DataFrame, window_secs: int = 300) -> pd.DataFrame:
    """
    Compute rolling price drift (like ctx.token_drift in strategy scripts).
    Drift = current_price - price N seconds ago.
    Returns DataFrame with timestamp, price, drift columns.
    """
    if df.empty:
        return pd.DataFrame()

    df = df.copy()
    df["ts"] = pd.to_datetime(df["timestamp_received"], utc=True)
    df = df.set_index("ts").sort_index()
    df["price_lag"] = df["price"].shift(freq=pd.Timedelta(seconds=window_secs))
    df["drift"] = df["price"] - df["price_lag"]
    df = df.reset_index()
    df["ts"] = df["ts"].dt.isoformat()
    return df[["ts", "price", "best_bid", "best_ask", "drift"]].dropna(subset=["drift"])


# ── Download helpers ───────────────────────────────────────────────────────────

_DOWNLOAD_UA = "orderbook-parser/1.0"
_CHUNK = 1024 * 256  # 256 KB read chunks


def _http_download(url: str, dest: Path) -> None:
    """Stream-download a URL to `dest` using a proper User-Agent.
    Cloudflare R2 returns 403 without User-Agent."""
    req = urllib.request.Request(url, headers={"User-Agent": _DOWNLOAD_UA})
    tmp = dest.with_suffix(".tmp")
    try:
        with urllib.request.urlopen(req, timeout=60) as resp, open(tmp, "wb") as fh:
            while True:
                chunk = resp.read(_CHUNK)
                if not chunk:
                    break
                fh.write(chunk)
        tmp.rename(dest)
    except Exception:
        tmp.unlink(missing_ok=True)
        raise


def download_hourly_files(
    days: int,
    out_dir: Path,
    market_id: Optional[str] = None,
    progress_file: Optional[Path] = None,
) -> dict:
    """
    Download and optionally filter hourly Parquet files from pmxt.dev.
    Stores them in out_dir/YYYY-MM-DDTHH.parquet (full) or filtered .parquet.
    Returns a summary dict with file counts and sizes.
    """
    out_dir.mkdir(parents=True, exist_ok=True)
    urls = urls_for_range(days)
    total = len(urls)
    downloaded = 0
    skipped = 0
    errors = []

    for i, url in enumerate(urls):
        hour_str = url.split("polymarket_orderbook_")[1].replace(".parquet", "")
        out_path = out_dir / f"{hour_str}.parquet"

        # Update progress file for the Rust side to poll
        if progress_file:
            progress_file.write_text(json.dumps({
                "done": i,
                "total": total,
                "current": hour_str,
                "downloaded": downloaded,
                "skipped": skipped,
                "errors": errors[-5:],  # last 5 errors
            }))

        if out_path.exists():
            skipped += 1
            continue

        try:
            if market_id:
                # Use DuckDB to filter and re-write as a smaller local Parquet.
                # DuckDB's HTTP reader uses get_con() which sets http_user_agent.
                con = get_con()
                con.execute(f"""
                COPY (
                    SELECT * FROM read_parquet('{url}', hive_partitioning=false)
                    WHERE CAST(market AS VARCHAR) = '{market_id}'
                ) TO '{out_path}' (FORMAT PARQUET, COMPRESSION 'zstd')
                """)
            else:
                # Full download via streaming GET with User-Agent header.
                _http_download(url, out_path)
            downloaded += 1
        except urllib.error.HTTPError as e:
            if e.code == 404:
                # Archive skips empty hours — not an error
                pass
            else:
                errors.append(f"{hour_str}: HTTP {e.code} {e.reason}")
        except Exception as e:
            errors.append(f"{hour_str}: {e}")
            # Continue — don't abort the whole batch on one failure

    # Final progress update
    result = {
        "total_urls": total,
        "downloaded": downloaded,
        "skipped": skipped,
        "errors": errors,
        "out_dir": str(out_dir),
    }
    if progress_file:
        progress_file.write_text(json.dumps({**result, "done": total}))
    return result


def analyze_local_dir(data_dir: Path, market_id: Optional[str] = None) -> dict:
    """Load all local parquet files and return aggregated analytics."""
    files = sorted(data_dir.glob("*.parquet"))
    if not files:
        return {"error": "No parquet files found", "dir": str(data_dir)}

    con = get_con()
    file_list = "[" + ", ".join(f"'{f}'" for f in files) + "]"
    where = "event_type = 'price_change'"
    if market_id:
        where += f" AND CAST(market AS VARCHAR) = '{market_id}'"

    df = con.execute(f"""
    SELECT
        CAST(market AS VARCHAR) AS market,
        event_type,
        COUNT(*) as cnt,
        MIN(timestamp_received) as first_ts,
        MAX(timestamp_received) as last_ts
    FROM read_parquet({file_list}, hive_partitioning=false, union_by_name=true)
    GROUP BY market, event_type
    ORDER BY cnt DESC
    LIMIT 100
    """).df()

    return {
        "file_count": len(files),
        "markets": df.to_dict(orient="records"),
    }


# ── CLI entry-points ───────────────────────────────────────────────────────────

def cmd_summary(args: argparse.Namespace) -> None:
    # For remote queries, limit sample to avoid downloading hundreds of 400MB files
    max_hours = getattr(args, "hours", 3)
    urls = urls_for_range(args.days)
    sample_urls = urls[:max_hours]
    print(f"[orderbook_parser] Sampling {max_hours} hour(s) of {len(urls)} total for day range {args.days}...", file=sys.stderr)
    try:
        counts = query_archive_summary(sample_urls)
        top = query_top_markets(sample_urls, limit=10)
        result = {
            "sample_hours": max_hours,
            "total_hours": len(urls),
            "estimated_total_gb": round(len(urls) * 0.25, 1),  # ~250MB avg per file
            "event_counts": counts,
            "top_markets_by_volume": top.to_dict(orient="records"),
            "note": f"Sampled {max_hours} hour(s). For full coverage, download locally first.",
        }
        print(json.dumps(result, default=str))
    except Exception as e:
        print(json.dumps({"error": str(e), "trace": traceback.format_exc()}))


def cmd_price_series(args: argparse.Namespace) -> None:
    max_hours = getattr(args, "hours", None)
    urls = urls_for_range(args.days)
    if max_hours:
        urls = urls[:max_hours]
    print(f"[orderbook_parser] Fetching price changes for market={args.market} — {len(urls)} file(s)...", file=sys.stderr)
    try:
        df = query_price_changes(urls, market_id=args.market, limit=200_000)
        if df.empty:
            print(json.dumps({"error": "No data found for this market/period"}))
            return
        ohlc = build_ohlc(df, freq=args.freq)
        stats = compute_spread_stats(df)
        result = {
            "market": args.market,
            "days": args.days,
            "freq": args.freq,
            "row_count": len(df),
            "candle_count": len(ohlc),
            "spread_stats": stats,
            "ohlc": ohlc.to_dict(orient="records"),
        }
        print(json.dumps(result, default=str))
    except Exception as e:
        print(json.dumps({"error": str(e), "trace": traceback.format_exc()}))


def cmd_top_markets(args: argparse.Namespace) -> None:
    max_hours = getattr(args, "hours", 1)
    all_urls = urls_for_range(args.days)
    urls = all_urls[:max_hours]
    print(f"[orderbook_parser] Fetching top markets — sampling {max_hours} hour(s) of {len(all_urls)} total...", file=sys.stderr)
    try:
        df = query_top_markets(urls, limit=args.limit)
        print(json.dumps({
            "sample_hours": max_hours,
            "total_hours": len(all_urls),
            "markets": df.to_dict(orient="records"),
            "note": f"Sampled {max_hours} hour(s). Use --hours N for more coverage, or download locally for full data.",
        }, default=str))
    except Exception as e:
        print(json.dumps({"error": str(e), "trace": traceback.format_exc()}))


def cmd_spread_stats(args: argparse.Namespace) -> None:
    urls = urls_for_range(args.days)
    print(f"[orderbook_parser] Computing spread stats for {args.market}...", file=sys.stderr)
    try:
        df = query_price_changes(urls, market_id=args.market, limit=100_000)
        stats = compute_spread_stats(df)
        profile = []
        trades = query_trades(urls, market_id=args.market, limit=50_000)
        if not trades.empty:
            profile = compute_volume_profile(trades)
        print(json.dumps({"market": args.market, "spread_stats": stats, "volume_profile": profile}, default=str))
    except Exception as e:
        print(json.dumps({"error": str(e), "trace": traceback.format_exc()}))


def cmd_download(args: argparse.Namespace) -> None:
    out_dir = Path(args.out)
    market_id = getattr(args, "market", None)
    progress_file = Path(args.progress) if getattr(args, "progress", None) else None
    print(f"[orderbook_parser] Downloading {args.days}d of orderbook data to {out_dir}...", file=sys.stderr)
    try:
        result = download_hourly_files(args.days, out_dir, market_id=market_id, progress_file=progress_file)
        print(json.dumps(result))
    except Exception as e:
        print(json.dumps({"error": str(e), "trace": traceback.format_exc()}))


def cmd_analyze_local(args: argparse.Namespace) -> None:
    data_dir = Path(args.dir)
    market_id = getattr(args, "market", None)
    print(f"[orderbook_parser] Analyzing local files in {data_dir}...", file=sys.stderr)
    try:
        result = analyze_local_dir(data_dir, market_id=market_id)
        print(json.dumps(result, default=str))
    except Exception as e:
        print(json.dumps({"error": str(e), "trace": traceback.format_exc()}))


# ── Binance price helper ───────────────────────────────────────────────────────

def fetch_binance_prices(symbol: str, start_ts_ms: int, end_ts_ms: int) -> dict:
    """
    Fetch 1-minute Binance klines for `symbol` over the given range.
    Returns {ts_s: close_price} — every second within each minute gets the same close.
    Handles pagination (max 1000 candles per request ≈ ~16 hours).
    Falls back silently on errors (backtest will see binance_price=0 for those gaps).
    """
    url_base = "https://api.binance.com/api/v3/klines"
    prices: dict = {}
    cur = start_ts_ms
    while cur < end_ts_ms:
        end_chunk = min(cur + 999 * 60_000, end_ts_ms)
        req_url = (f"{url_base}?symbol={symbol}&interval=1m"
                   f"&startTime={cur}&endTime={end_chunk}&limit=1000")
        req = urllib.request.Request(req_url, headers={"User-Agent": "orderbook-parser/1.0"})
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                candles = json.loads(resp.read())
        except Exception as e:
            print(f"[binance] fetch error ({symbol}): {e}", file=sys.stderr)
            break
        if not candles:
            break
        for c in candles:
            open_ts_s = c[0] // 1000
            close_price = float(c[4])
            for s in range(60):
                prices[open_ts_s + s] = close_price
        last_open_ms = candles[-1][0]
        cur = last_open_ms + 60_000
        if len(candles) < 2:
            break
    return prices


# ── Historical JSONL helper ─────────────────────────────────────────────────────

def load_markets_from_historical_jsonl(
    slug: str,
    start_ts: int,
    end_ts: int,
    workspace_dir: "Path | None" = None,
) -> list[dict]:
    """
    Load condition IDs and YES token IDs for a series from the scraped
    historical JSONL at <workspace>/data/polymarket_historical/<slug>.jsonl.

    These files are produced by `trader-claw backtest-sync --series <slug>`
    and contain one JSON object per 5-minute window with fields:
      window_open_ts, window_close_ts, condition_id, yes_token_id, no_token_id, ...

    Returns list of {condition_id, yes_token_id, no_token_id, end_ts} dicts.
    """
    if workspace_dir is None:
        workspace_dir = Path.home() / ".traderclaw" / "workspace"

    jsonl_path = workspace_dir / "data" / "polymarket_historical" / f"{slug}.jsonl"

    if not jsonl_path.exists():
        print(f"[historical] {slug}: JSONL not found at {jsonl_path}", file=sys.stderr)
        return []

    markets: list[dict] = []
    seen_cids: set[str] = set()

    with open(jsonl_path) as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                m = json.loads(line)
                wts = m.get("window_open_ts", 0)
                # Only include windows within the parquet date range
                if not (start_ts <= wts <= end_ts):
                    continue
                cid  = m.get("condition_id", "")
                ytid = m.get("yes_token_id", "")
                ntid = m.get("no_token_id", "")
                end_ts_win = m.get("window_close_ts", wts + 300)
                if cid and cid not in seen_cids:
                    seen_cids.add(cid)
                    markets.append({
                        "condition_id": cid,
                        "yes_token_id": ytid,
                        "no_token_id":  ntid,
                        "end_ts":       end_ts_win,
                    })
            except Exception:
                continue

    print(f"[historical] {slug}: {len(markets)} unique condition IDs "
          f"from {jsonl_path.name}", file=sys.stderr)
    return markets


# ── Gamma API helpers ──────────────────────────────────────────────────────────

def fetch_gamma_markets_via_events(
    prefix: str,
    start_ts: int,
    end_ts: int,
    window_minutes: int = 5,
) -> list[dict]:
    """
    Fetch condition IDs and YES token IDs from Gamma /events?slug= endpoint.

    Unlike /markets?slug=, the /events endpoint serves closed/resolved markets
    so it works for recent historical timestamps (days to weeks old).

    Generates all 5-min boundary slugs between start_ts and end_ts,
    looks each one up concurrently, returns found markets.
    """
    import concurrent.futures

    window_secs = window_minutes * 60
    base = "https://gamma-api.polymarket.com"
    ua = {"User-Agent": "orderbook-parser/1.0"}

    # Generate all window timestamps in range
    cur = int(start_ts)
    cur = (cur // window_secs) * window_secs
    end_aligned = int(end_ts)
    slugs: list[str] = []
    while cur <= end_aligned:
        slugs.append(f"{prefix}-{cur}")
        cur += window_secs

    print(f"[gamma-events] {prefix}: checking {len(slugs)} slug timestamps via /events...",
          file=sys.stderr)

    def lookup(slug: str) -> "dict | None":
        url = f"{base}/events?slug={slug}"
        try:
            req = urllib.request.Request(url, headers=ua)
            with urllib.request.urlopen(req, timeout=10) as resp:
                events = json.loads(resp.read())
            for evt in events:
                for m in evt.get("markets", []):
                    cid = m.get("conditionId", "")
                    if not cid:
                        continue
                    raw_ids = m.get("clobTokenIds", [])
                    if isinstance(raw_ids, str):
                        try:
                            raw_ids = json.loads(raw_ids)
                        except Exception:
                            raw_ids = []
                    try:
                        ts = int(slug.split("-")[-1])
                        end_ts_win = ts + window_secs
                    except Exception:
                        end_ts_win = 0
                    return {
                        "condition_id": cid,
                        "yes_token_id": raw_ids[0] if raw_ids else "",
                        "no_token_id":  raw_ids[1] if len(raw_ids) > 1 else "",
                        "end_ts":       end_ts_win,
                    }
        except Exception:
            pass
        return None

    results: list[dict] = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=20) as ex:
        for r in ex.map(lookup, slugs):
            if r:
                results.append(r)

    print(f"[gamma-events] {prefix}: found {len(results)} markets", file=sys.stderr)
    return results


# Known recurring series: Gamma slug prefix → (tick slug, Binance symbol, window minutes)
MULTI_SERIES: list[dict] = [
    # ── 5-minute markets ──────────────────────────────────────────────────────
    {"prefix": "btc-updown-5m",  "slug": "btc_5m",  "binance": "BTCUSDT",  "window_minutes": 5},
    {"prefix": "eth-updown-5m",  "slug": "eth_5m",  "binance": "ETHUSDT",  "window_minutes": 5},
    {"prefix": "sol-updown-5m",  "slug": "sol_5m",  "binance": "SOLUSDT",  "window_minutes": 5},
    {"prefix": "xrp-updown-5m",  "slug": "xrp_5m",  "binance": "XRPUSDT",  "window_minutes": 5},
    {"prefix": "bnb-updown-5m",  "slug": "bnb_5m",  "binance": "BNBUSDT",  "window_minutes": 5},
    {"prefix": "doge-updown-5m", "slug": "doge_5m", "binance": "DOGEUSDT", "window_minutes": 5},
    {"prefix": "hype-updown-5m", "slug": "hype_5m", "binance": "HYPEUSDT", "window_minutes": 5},
    # ── 15-minute markets ─────────────────────────────────────────────────────
    {"prefix": "btc-updown-15m", "slug": "btc_15m", "binance": "BTCUSDT",  "window_minutes": 15},
    {"prefix": "eth-updown-15m", "slug": "eth_15m", "binance": "ETHUSDT",  "window_minutes": 15},
    {"prefix": "sol-updown-15m", "slug": "sol_15m", "binance": "SOLUSDT",  "window_minutes": 15},
    {"prefix": "xrp-updown-15m", "slug": "xrp_15m", "binance": "XRPUSDT",  "window_minutes": 15},
    # ── 1-hour markets ────────────────────────────────────────────────────────
    {"prefix": "btc-updown-1h",  "slug": "btc_1h",  "binance": "BTCUSDT",  "window_minutes": 60},
    {"prefix": "eth-updown-1h",  "slug": "eth_1h",  "binance": "ETHUSDT",  "window_minutes": 60},
    {"prefix": "sol-updown-1h",  "slug": "sol_1h",  "binance": "SOLUSDT",  "window_minutes": 60},
]


def fetch_gamma_markets_for_prefix(prefix: str, start_date_str: str, end_date_str: str) -> list[dict]:
    """
    Query the Gamma API /markets endpoint to find all condition IDs + YES token IDs
    for a given slug prefix within a date range.

    Returns list of {condition_id, yes_token_id, end_ts} dicts.
    Falls back to slug-generation if the API filter isn't supported.
    """
    import concurrent.futures

    base = "https://gamma-api.polymarket.com"
    all_markets: list[dict] = []
    offset = 0
    limit = 500
    ua = {"User-Agent": "orderbook-parser/1.0"}

    # Try paginated /markets?slug_prefix=... query
    while True:
        url = (f"{base}/markets?limit={limit}&offset={offset}"
               f"&slug_prefix={prefix}"
               f"&start_date_min={start_date_str}&end_date_max={end_date_str}")
        try:
            req = urllib.request.Request(url, headers=ua)
            with urllib.request.urlopen(req, timeout=20) as resp:
                data = json.loads(resp.read())
        except Exception as e:
            print(f"[gamma] {prefix} page {offset // limit}: {e}", file=sys.stderr)
            break

        if not data:
            break

        for m in data:
            cid = m.get("conditionId", "")
            if not cid:
                continue
            # clobTokenIds may be JSON string or list
            raw_ids = m.get("clobTokenIds", [])
            if isinstance(raw_ids, str):
                try:
                    raw_ids = json.loads(raw_ids)
                except Exception:
                    raw_ids = []
            end_iso = m.get("endDateIso") or m.get("endDate") or ""
            end_ts = 0
            if end_iso:
                try:
                    from datetime import timezone as _tz
                    dt = datetime.fromisoformat(end_iso.replace("Z", "+00:00"))
                    end_ts = int(dt.timestamp())
                except Exception:
                    pass
            all_markets.append({
                "condition_id":  cid,
                "yes_token_id":  raw_ids[0] if raw_ids else "",
                "no_token_id":   raw_ids[1] if len(raw_ids) > 1 else "",
                "end_ts":        end_ts,
            })

        if len(data) < limit:
            break
        offset += limit
        time.sleep(0.05)

    if all_markets:
        print(f"[gamma] {prefix}: {len(all_markets)} markets via API", file=sys.stderr)
        return all_markets

    # ── Fallback: generate expected slugs from timestamps ──────────────────────
    # If the API doesn't support slug_prefix, generate all 5-min boundary slugs
    # and look up each condition ID individually via /markets?slug=...
    print(f"[gamma] {prefix}: API filter not supported, generating slugs...", file=sys.stderr)

    try:
        start_dt = datetime.fromisoformat(start_date_str).replace(tzinfo=timezone.utc)
        end_dt   = datetime.fromisoformat(end_date_str).replace(tzinfo=timezone.utc)
    except Exception:
        return []

    # Generate all 5-min boundary timestamps in range
    window_secs = 300
    cur_ts = int(start_dt.timestamp())
    cur_ts = (cur_ts // window_secs) * window_secs  # align to 5-min boundary
    end_ts_limit = int(end_dt.timestamp())
    slugs_to_check: list[str] = []
    while cur_ts <= end_ts_limit:
        slugs_to_check.append(f"{prefix}-{cur_ts}")
        cur_ts += window_secs

    print(f"[gamma] {prefix}: checking {len(slugs_to_check)} slug timestamps...", file=sys.stderr)

    def lookup_slug(slug: str) -> dict | None:
        url = f"{base}/markets?slug={slug}"
        try:
            req = urllib.request.Request(url, headers=ua)
            with urllib.request.urlopen(req, timeout=10) as resp:
                data = json.loads(resp.read())
            if data:
                m = data[0]
                cid = m.get("conditionId", "")
                raw_ids = m.get("clobTokenIds", [])
                if isinstance(raw_ids, str):
                    try:
                        raw_ids = json.loads(raw_ids)
                    except Exception:
                        raw_ids = []
                # end_ts from slug name (last part is the unix timestamp of window START)
                try:
                    end_ts = int(slug.split("-")[-1]) + window_secs
                except Exception:
                    end_ts = 0
                if cid:
                    return {"condition_id": cid, "yes_token_id": raw_ids[0] if raw_ids else "",
                            "no_token_id": raw_ids[1] if len(raw_ids) > 1 else "", "end_ts": end_ts}
        except Exception:
            pass
        return None

    with concurrent.futures.ThreadPoolExecutor(max_workers=20) as ex:
        futures = list(ex.map(lookup_slug, slugs_to_check))
    results = [r for r in futures if r]
    print(f"[gamma] {prefix}: found {len(results)} markets via slug lookup", file=sys.stderr)
    return results


# ── 1Hz tick builder (shared by to-ticks and to-ticks-multi) ──────────────────

def build_ticks_from_df(
    df: pd.DataFrame,
    yes_token_ids: set,
    window_minutes: int,
    binance_prices: dict,
) -> pd.DataFrame:
    """
    Given a DataFrame of price_change events for one or many markets,
    build a 1-Hz tick table.

    `yes_token_ids`: set of asset_ids that are the YES token for their respective market.
    `binance_prices`: {ts_s: float} lookup for Binance spot price.
    Returns DataFrame with all tick fields ready to serialize as JSONL.
    """
    window_secs = window_minutes * 60

    if yes_token_ids:
        yes_df = df[df["asset_id"].isin(yes_token_ids)].copy()
    else:
        # Fallback: pick most-active asset_id globally
        counts = df.groupby("asset_id").size()
        yes_df = df[df["asset_id"] == counts.idxmax()].copy()

    if yes_df.empty:
        return pd.DataFrame()

    yes_df["ts_s"] = yes_df["ts_ms"] // 1000
    yes_1hz = yes_df.groupby("ts_s").last().reset_index()

    t_min = int(yes_1hz["ts_s"].min())
    t_max = int(yes_1hz["ts_s"].max())
    all_secs = pd.DataFrame({"ts_s": range(t_min, t_max + 1)})
    yes_1hz = all_secs.merge(yes_1hz, on="ts_s", how="left").ffill()

    yes_1hz["yes_bid"] = yes_1hz["yes_bid"].fillna(0.0)
    yes_1hz["yes_ask"] = yes_1hz["yes_ask"].fillna(0.0)
    yes_1hz["yes_mid"] = (yes_1hz["yes_bid"] + yes_1hz["yes_ask"]) / 2
    yes_1hz["no_bid"]  = (1.0 - yes_1hz["yes_ask"]).clip(0, 1)
    yes_1hz["no_ask"]  = (1.0 - yes_1hz["yes_bid"]).clip(0, 1)
    yes_1hz["ts_ms_out"] = yes_1hz["ts_s"] * 1000

    # Binance price lookup
    yes_1hz["binance_price"] = yes_1hz["ts_s"].map(
        lambda s: binance_prices.get(int(s), 0.0)
    )

    # Window fields.
    # At exact 5-min boundaries (ts_s % window_secs == 0) the second belongs to the
    # PREVIOUS window as its close tick (secs_left = 0).  All other seconds count
    # down from window_secs-1 to 1 within their window.
    # This ensures `window_secs_left == 0` ticks exist for the clob_1hz backtester
    # to resolve open positions.
    _rem = (yes_1hz["ts_s"] % window_secs).astype(int)
    _is_boundary = _rem == 0
    yes_1hz["window_ts"] = _np.where(
        _is_boundary,
        yes_1hz["ts_s"].astype(int) - window_secs,
        yes_1hz["ts_s"].astype(int) - _rem,
    ).astype(int)
    yes_1hz["window_secs_left"] = _np.where(_is_boundary, 0, window_secs - _rem).astype(int)

    yes_1hz["date"] = pd.to_datetime(yes_1hz["ts_s"], unit="s", utc=True).dt.date
    return yes_1hz


def write_ticks_jsonl(yes_1hz: pd.DataFrame, out_dir: Path) -> int:
    """Write a 1Hz tick DataFrame to per-day JSONL files. Returns total row count."""
    out_dir.mkdir(parents=True, exist_ok=True)
    total = 0
    for date in sorted(yes_1hz["date"].unique()):
        day_df = yes_1hz[yes_1hz["date"] == date]
        date_str = str(date)
        out_file = out_dir / f"{date_str}.jsonl"
        rows = []
        for _, row in day_df.iterrows():
            rows.append(json.dumps({
                "ts_ms":            int(row["ts_ms_out"]),
                "yes_bid":          round(float(row["yes_bid"]), 6),
                "yes_ask":          round(float(row["yes_ask"]), 6),
                "no_bid":           round(float(row["no_bid"]),  6),
                "no_ask":           round(float(row["no_ask"]),  6),
                "yes_mid":          round(float(row["yes_mid"]), 6),
                "binance_price":    round(float(row["binance_price"]), 4),
                "chainlink_price":  0.0,
                "oracle_lag_ms":    0,
                "window_ts":        int(row["window_ts"]),
                "window_secs_left": int(row["window_secs_left"]),
            }))
        out_file.write_text("\n".join(rows) + "\n")
        total += len(rows)
        print(f"  {date_str} → {len(rows):,} rows", file=sys.stderr)
    return total


def cmd_to_ticks(args: argparse.Namespace) -> None:
    """
    Convert local Parquet files into CLOB 1Hz JSONL ticks compatible with the
    existing tick-recorder backtester (on_tick(ctx) Rhai scripts).

    Output format (one JSON object per line, 1 row per second):
      ts_ms, yes_bid, yes_ask, no_bid, no_ask, yes_mid,
      binance_price, chainlink_price, oracle_lag_ms,
      window_ts, window_secs_left
    """
    # Support both --in (new) and --dir (legacy)
    input_dir_str = getattr(args, "input_dir", None) or getattr(args, "dir", None)
    data_dir = Path(input_dir_str)
    out_dir = Path(args.out)
    market_id = args.market
    window_minutes = getattr(args, "window_minutes", 5)
    window_secs = window_minutes * 60
    slug = args.slug
    binance_symbol = getattr(args, "binance_symbol", None)

    out_dir.mkdir(parents=True, exist_ok=True)

    files = sorted(data_dir.glob("*.parquet"))
    if not files:
        print(json.dumps({"error": f"No .parquet files in {data_dir}"}))
        return

    print(f"[to-ticks] Loading {len(files)} parquet file(s) for market={market_id}...", file=sys.stderr)

    con = get_con()
    file_list = "[" + ", ".join(f"'{f}'" for f in files) + "]"
    where = f"event_type = 'price_change' AND CAST(market AS VARCHAR) = '{market_id}'"

    df = con.execute(f"""
    SELECT
        CAST(epoch_ms(timestamp_received) AS BIGINT) AS ts_ms,
        CAST(best_bid  AS DOUBLE)  AS yes_bid,
        CAST(best_ask  AS DOUBLE)  AS yes_ask,
        CAST(price     AS DOUBLE)  AS price,
        asset_id
    FROM read_parquet({file_list}, hive_partitioning=false, union_by_name=true)
    WHERE {where}
    ORDER BY timestamp_received
    """).df()

    if df.empty:
        print(json.dumps({"error": f"No price_change events found for market {market_id}"}))
        return

    print(f"[to-ticks] {len(df):,} price_change events. Resampling to 1 Hz...", file=sys.stderr)

    # YES asset = most events
    asset_counts = df.groupby("asset_id").size()
    yes_asset = asset_counts.idxmax()
    print(f"[to-ticks] YES asset_id={yes_asset[:20]}... ({asset_counts[yes_asset]:,} events)", file=sys.stderr)

    # Binance prices (optional)
    binance_prices: dict = {}
    if binance_symbol:
        ts_min = int(df["ts_ms"].min())
        ts_max = int(df["ts_ms"].max())
        print(f"[to-ticks] Fetching {binance_symbol} prices from Binance...", file=sys.stderr)
        binance_prices = fetch_binance_prices(binance_symbol, ts_min, ts_max + 60_000)
        print(f"[to-ticks] {len(binance_prices):,} Binance price points", file=sys.stderr)

    yes_1hz = build_ticks_from_df(df, {yes_asset}, window_minutes, binance_prices)
    if yes_1hz.empty:
        print(json.dumps({"error": "Failed to build tick table"}))
        return

    total_rows = write_ticks_jsonl(yes_1hz, out_dir)

    print(json.dumps({
        "ok": True,
        "slug": slug,
        "market": market_id,
        "yes_asset_id": yes_asset,
        "total_rows": total_rows,
        "days": int(yes_1hz["date"].nunique()),
        "out_dir": str(out_dir),
        "next_step": (
            f"Files written to {out_dir}. "
            f"In Backtesting, select 'Orderbook Archive (on_tick)' or "
            f"'Orderbook Archive (on_candle)' and slug='{slug}'."
        ),
    }))


def cmd_to_ticks_multi(args: argparse.Namespace) -> None:
    """
    Convert all recurring UP/DOWN markets (5m, 15m, 1h) from local Parquet files
    to 1-Hz JSONL ticks with real Binance prices.

    Supported series (use --slugs to restrict):
      5m:  btc_5m, eth_5m, sol_5m, xrp_5m, bnb_5m, doge_5m, hype_5m
      15m: btc_15m, eth_15m, sol_15m, xrp_15m
      1h:  btc_1h, eth_1h, sol_1h

    Condition IDs are sourced from scraped historical JSONL files at:
      <workspace>/data/polymarket_historical/<slug>.jsonl
    Falls back to the Gamma API for any series missing a local JSONL file.

    Writes:
      <out>/btc_5m/YYYY-MM-DD.jsonl
      <out>/btc_15m/YYYY-MM-DD.jsonl
      <out>/btc_1h/YYYY-MM-DD.jsonl
      ...
    """
    data_dir  = Path(args.input_dir)
    out_base  = Path(args.out)
    slugs_arg = args.slugs  # e.g. "btc_5m,eth_5m" or None (= all)
    workspace_dir = Path(args.workspace) if getattr(args, "workspace", None) else None
    window_minutes = 5

    files = sorted(data_dir.glob("*.parquet"))
    if not files:
        print(json.dumps({"error": f"No .parquet files in {data_dir}"}))
        return

    # Determine date range from file names (YYYY-MM-DDTHH.parquet)
    def parse_file_ts(f: Path) -> "datetime | None":
        try:
            return datetime.strptime(f.stem, "%Y-%m-%dT%H").replace(tzinfo=timezone.utc)
        except ValueError:
            return None

    file_dts = [dt for f in files if (dt := parse_file_ts(f))]
    if not file_dts:
        print(json.dumps({"error": "Cannot parse timestamps from filenames"}))
        return

    range_start = min(file_dts)
    range_end   = max(file_dts) + timedelta(hours=1)
    start_ts    = int(range_start.timestamp())
    end_ts      = int(range_end.timestamp())
    start_date_str = range_start.strftime("%Y-%m-%dT%H:%M:%SZ")
    end_date_str   = range_end.strftime("%Y-%m-%dT%H:%M:%SZ")

    print(f"[to-ticks-multi] Parquet date range: {range_start} → {range_end}", file=sys.stderr)

    # Filter series
    wanted_slugs = set(slugs_arg.split(",")) if slugs_arg else None
    series_list = [
        s for s in MULTI_SERIES
        if wanted_slugs is None or s["slug"] in wanted_slugs
    ]

    print(f"[to-ticks-multi] Processing {len(series_list)} series: "
          f"{[s['slug'] for s in series_list]}", file=sys.stderr)

    con = get_con()
    window_minutes = 5  # default; overridden per-series below

    # Build a lookup: stem → file path (e.g. "2026-05-10T17" → Path)
    file_by_stem: dict[str, Path] = {}
    for f in files:
        dt = parse_file_ts(f)
        if dt:
            file_by_stem[f.stem] = f

    results = []

    for series in series_list:
        prefix         = series["prefix"]
        slug           = series["slug"]
        binance        = series["binance"]
        window_minutes = series.get("window_minutes", 5)

        print(f"\n{'='*60}", file=sys.stderr)
        print(f"[to-ticks-multi] Series: {slug} (prefix={prefix}, binance={binance}, window={window_minutes}m)", file=sys.stderr)

        # ── Step 1: Load condition IDs from local historical JSONL ─────────────
        # Primary: scraped historical data (no network needed, covers full history)
        markets_info = load_markets_from_historical_jsonl(
            slug, start_ts, end_ts, workspace_dir
        )

        # Supplement: if the JSONL doesn't cover the full range, fill the gap
        # using the Gamma /events?slug= endpoint (works for recent closed markets).
        if markets_info:
            covered_end_ts = max(m["end_ts"] for m in markets_info)
            # Leave a 1-window grace margin before calling the events API
            gap_start = covered_end_ts - window_minutes * 60
            if gap_start < end_ts - window_minutes * 60:
                print(f"[to-ticks-multi] {slug}: JSONL covers through "
                      f"{datetime.fromtimestamp(covered_end_ts, tz=timezone.utc).date()}, "
                      f"filling gap to {range_end.date()} via Gamma events API...",
                      file=sys.stderr)
                gap_markets = fetch_gamma_markets_via_events(
                    prefix, gap_start, end_ts, window_minutes
                )
                if gap_markets:
                    # Merge, deduplicating by condition_id
                    existing_cids = {m["condition_id"] for m in markets_info}
                    new_markets = [m for m in gap_markets if m["condition_id"] not in existing_cids]
                    markets_info.extend(new_markets)
                    print(f"[to-ticks-multi] {slug}: added {len(new_markets)} markets "
                          f"from gap fill", file=sys.stderr)

        # Full fallback: if no historical data at all, try Gamma events API
        if not markets_info:
            print(f"[to-ticks-multi] {slug}: no local historical data — "
                  f"falling back to Gamma events API...", file=sys.stderr)
            markets_info = fetch_gamma_markets_via_events(
                prefix, start_ts, end_ts, window_minutes
            )

        if not markets_info:
            print(f"[to-ticks-multi] {slug}: no markets found — skipping", file=sys.stderr)
            results.append({
                "slug": slug, "ok": False,
                "error": (
                    "No condition IDs found. Run: "
                    f"trader-claw backtest-sync --series {slug} "
                    f"--from {range_start.strftime('%Y-%m-%d')} "
                    f"--to {range_end.strftime('%Y-%m-%d')}"
                ),
            })
            continue

        # ── Step 2: Day-by-day batch query with SQL 1Hz aggregation ─────────────
        # Build per-day lookup tables for condition IDs and YES token IDs.
        # For each calendar day we query only that day's ~24 parquet files with
        # that day's ~288 condition IDs, AND aggregate to 1 second in DuckDB SQL.
        # This keeps Python-side memory to ~86,400 rows/day (1 row per second)
        # instead of 74M+ raw events.
        from collections import defaultdict as _defaultdict

        yes_token_ids = {m["yes_token_id"] for m in markets_info if m["yes_token_id"]}
        print(f"[to-ticks-multi] {slug}: {len(markets_info)} condition IDs, "
              f"{len(yes_token_ids)} YES token IDs", file=sys.stderr)

        cids_by_date:  dict[str, set] = _defaultdict(set)
        ytids_by_date: dict[str, set] = _defaultdict(set)
        for m in markets_info:
            win_open = m["end_ts"] - window_minutes * 60
            for delta in (0, window_minutes * 60 - 1):
                day = datetime.fromtimestamp(win_open + delta, tz=timezone.utc).strftime("%Y-%m-%d")
                cids_by_date[day].add(m["condition_id"])
                if m["yes_token_id"]:
                    ytids_by_date[day].add(m["yes_token_id"])

        all_day_dfs: list[pd.DataFrame] = []
        ts_min_global = int(range_start.timestamp())
        ts_max_global = int(range_end.timestamp())

        for day_str in sorted(cids_by_date.keys()):
            day_cids  = list(cids_by_date[day_str])
            day_ytids = list(ytids_by_date.get(day_str, set()))
            day_files = [
                f for stem, f in file_by_stem.items()
                if stem.startswith(day_str)
            ]
            if not day_files:
                continue

            cids_sql  = ", ".join(f"'{c}'" for c in day_cids)
            day_file_list = "[" + ", ".join(f"'{f}'" for f in sorted(day_files)) + "]"

            # YES token filter: massive reduction in scanned rows
            if day_ytids:
                ytids_sql   = ", ".join(f"'{t}'" for t in day_ytids)
                asset_filter = f"AND CAST(asset_id AS VARCHAR) IN ({ytids_sql})"
            else:
                asset_filter = ""

            # Aggregate to 1Hz in SQL using max_by (last event per second)
            try:
                day_df = con.execute(f"""
                SELECT
                    CAST(epoch(timestamp_received) AS BIGINT) AS ts_s,
                    max_by(CAST(best_bid AS DOUBLE), timestamp_received) AS yes_bid,
                    max_by(CAST(best_ask AS DOUBLE), timestamp_received) AS yes_ask
                FROM read_parquet({day_file_list}, hive_partitioning=false, union_by_name=true)
                WHERE event_type = 'price_change'
                  AND CAST(market AS VARCHAR) IN ({cids_sql})
                  {asset_filter}
                GROUP BY ts_s
                ORDER BY ts_s
                """).df()
            except Exception as e:
                print(f"[to-ticks-multi] {slug} {day_str}: DuckDB error: {e}", file=sys.stderr)
                continue

            if not day_df.empty:
                ts_min_global = min(ts_min_global, int(day_df["ts_s"].min()))
                ts_max_global = max(ts_max_global, int(day_df["ts_s"].max()))
                all_day_dfs.append(day_df)
                print(f"  {day_str}: {len(day_df):,} 1Hz rows "
                      f"({len(day_cids)} cids, {len(day_files)} files)",
                      file=sys.stderr)

        if not all_day_dfs:
            print(f"[to-ticks-multi] {slug}: no data in parquets — skipping", file=sys.stderr)
            results.append({"slug": slug, "ok": False, "error": "No events in local parquets for these condition IDs"})
            continue

        # ── Step 3: Fetch Binance prices ───────────────────────────────────────
        print(f"[to-ticks-multi] {slug}: fetching {binance} prices...", file=sys.stderr)
        binance_prices = fetch_binance_prices(
            binance, ts_min_global * 1000, ts_max_global * 1000 + 60_000
        )
        print(f"[to-ticks-multi] {slug}: {len(binance_prices):,} Binance price points", file=sys.stderr)

        # ── Step 4: Build 1Hz tick table ──────────────────────────────────────
        # Data is already aggregated to 1Hz by DuckDB; just forward-fill gaps
        # and add window / Binance fields.
        window_secs = window_minutes * 60
        df_1hz = (
            pd.concat(all_day_dfs, ignore_index=True)
            .sort_values("ts_s")
            .drop_duplicates("ts_s", keep="last")  # guard against day-boundary overlaps
            .reset_index(drop=True)
        )

        t_min = int(df_1hz["ts_s"].min())
        t_max = int(df_1hz["ts_s"].max())
        all_secs = pd.DataFrame({"ts_s": range(t_min, t_max + 1)})
        yes_1hz = all_secs.merge(df_1hz, on="ts_s", how="left").ffill()

        yes_1hz["yes_bid"]   = yes_1hz["yes_bid"].fillna(0.0)
        yes_1hz["yes_ask"]   = yes_1hz["yes_ask"].fillna(0.0)
        yes_1hz["yes_mid"]   = (yes_1hz["yes_bid"] + yes_1hz["yes_ask"]) / 2
        yes_1hz["no_bid"]    = (1.0 - yes_1hz["yes_ask"]).clip(0, 1)
        yes_1hz["no_ask"]    = (1.0 - yes_1hz["yes_bid"]).clip(0, 1)
        yes_1hz["ts_ms_out"] = yes_1hz["ts_s"].astype(int) * 1000

        yes_1hz["binance_price"] = yes_1hz["ts_s"].map(
            lambda s: binance_prices.get(int(s), 0.0)
        )
        _rem2 = (yes_1hz["ts_s"] % window_secs).astype(int)
        _is_boundary2 = _rem2 == 0
        yes_1hz["window_ts"] = _np.where(
            _is_boundary2,
            yes_1hz["ts_s"].astype(int) - window_secs,
            yes_1hz["ts_s"].astype(int) - _rem2,
        ).astype(int)
        yes_1hz["window_secs_left"] = _np.where(
            _is_boundary2, 0, window_secs - _rem2
        ).astype(int)
        yes_1hz["date"] = pd.to_datetime(
            yes_1hz["ts_s"], unit="s", utc=True
        ).dt.date

        if yes_1hz.empty:
            results.append({"slug": slug, "ok": False, "error": "Empty tick table after resampling"})
            continue

        # ── Step 5: Write JSONL ────────────────────────────────────────────────
        out_dir = out_base / slug
        print(f"[to-ticks-multi] {slug}: writing ticks to {out_dir}...", file=sys.stderr)
        total_rows = write_ticks_jsonl(yes_1hz, out_dir)
        n_days = int(yes_1hz["date"].nunique())

        print(f"[to-ticks-multi] {slug}: done — {total_rows:,} ticks over {n_days} day(s)", file=sys.stderr)
        results.append({
            "slug": slug,
            "ok": True,
            "condition_ids": len(markets_info),
            "total_rows": total_rows,
            "days": n_days,
            "out_dir": str(out_dir),
            "binance_symbol": binance,
        })

    # Summary
    ok_count  = sum(1 for r in results if r.get("ok"))
    err_count = len(results) - ok_count
    print(f"\n[to-ticks-multi] Completed: {ok_count} OK, {err_count} failed", file=sys.stderr)

    print(json.dumps({
        "ok": ok_count > 0,
        "series_processed": len(results),
        "series_ok": ok_count,
        "series_failed": err_count,
        "results": results,
        "next_step": (
            "In Backtesting → Market = 'Orderbook Archive (on_candle)' or "
            "'Orderbook Archive (on_tick)', then pick a slug from the dropdown."
        ),
    }, indent=2))


def cmd_to_candles(args: argparse.Namespace) -> None:
    """
    Convert local Parquet files into OHLC candle JSON compatible with the
    existing candle-based backtester (on_candle(ctx) Rhai scripts).

    Output: ~/.traderclaw/workspace/data/<slug>_<freq>.json
    Format: list of {time, open, high, low, close, volume}
    (same format as Binance klines used by the existing engine)
    """
    data_dir = Path(args.dir)
    market_id = args.market
    freq = args.freq  # e.g. "5min", "1min", "15min"
    slug = args.slug
    out_dir = Path(args.out) if args.out else data_dir

    out_dir.mkdir(parents=True, exist_ok=True)

    files = sorted(data_dir.glob("*.parquet"))
    if not files:
        print(json.dumps({"error": f"No .parquet files in {data_dir}"}))
        return

    print(f"[to-candles] Loading {len(files)} file(s) for market={market_id}...", file=sys.stderr)

    con = get_con()
    file_list = "[" + ", ".join(f"'{f}'" for f in files) + "]"
    where = f"event_type = 'price_change' AND CAST(market AS VARCHAR) = '{market_id}'"

    df = con.execute(f"""
    SELECT
        timestamp_received,
        CAST(best_bid AS DOUBLE)  AS yes_bid,
        CAST(best_ask AS DOUBLE)  AS yes_ask,
        CAST(price    AS DOUBLE)  AS price,
        CAST(size     AS DOUBLE)  AS size,
        asset_id
    FROM read_parquet({file_list}, hive_partitioning=false, union_by_name=true)
    WHERE {where}
    ORDER BY timestamp_received
    """).df()

    if df.empty:
        print(json.dumps({"error": f"No price_change events for market {market_id}"}))
        return

    print(f"[to-candles] {len(df):,} events → building {freq} OHLC...", file=sys.stderr)

    # Use the YES asset (most events)
    asset_counts = df.groupby("asset_id").size()
    yes_asset = asset_counts.idxmax()
    df = df[df["asset_id"] == yes_asset].copy()

    df["timestamp_received"] = pd.to_datetime(df["timestamp_received"], utc=True)
    df = df.set_index("timestamp_received").sort_index()
    df["mid"] = (df["yes_bid"] + df["yes_ask"]) / 2

    ohlc = df["mid"].resample(freq).ohlc()
    ohlc["volume"] = df["size"].resample(freq).sum()
    ohlc = ohlc.dropna(subset=["open"]).reset_index()

    # Format as list of Binance-compatible candle dicts
    candles = []
    for _, row in ohlc.iterrows():
        candles.append({
            "time":   int(row["timestamp_received"].timestamp() * 1000),
            "open":   round(float(row["open"]),   6),
            "high":   round(float(row["high"]),   6),
            "low":    round(float(row["low"]),    6),
            "close":  round(float(row["close"]),  6),
            "volume": round(float(row["volume"]), 4),
        })

    out_name = f"{slug}_{freq.replace('min','m').replace('h','H')}.json"
    out_file = out_dir / out_name
    out_file.write_text(json.dumps(candles))
    print(f"[to-candles] Wrote {len(candles):,} candles to {out_file}", file=sys.stderr)

    print(json.dumps({
        "ok": True,
        "slug": slug,
        "freq": freq,
        "market": market_id,
        "yes_asset_id": yes_asset,
        "candle_count": len(candles),
        "out_file": str(out_file),
        "date_range": {
            "from": candles[0]["time"] if candles else None,
            "to":   candles[-1]["time"] if candles else None,
        },
        "next_step": (
            f"File written: {out_file}. "
            f"Copy to ~/.traderclaw/workspace/data/ then use "
            f"market_type='polymarket' with slug='{slug}' in the Backtesting page."
        ),
    }))


def cmd_drift(args: argparse.Namespace) -> None:
    urls = urls_for_range(args.days)
    print(f"[orderbook_parser] Computing drift for {args.market} window={args.window}s...", file=sys.stderr)
    try:
        df = query_price_changes(urls, market_id=args.market, limit=200_000)
        drift_df = analyze_drift(df, window_secs=args.window)
        print(json.dumps({
            "market": args.market,
            "window_secs": args.window,
            "rows": len(drift_df),
            "data": drift_df.head(1000).to_dict(orient="records"),
        }, default=str))
    except Exception as e:
        print(json.dumps({"error": str(e), "trace": traceback.format_exc()}))


# ── Main ───────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="Polymarket Orderbook Archive Parser (pmxt.dev v2)")
    sub = parser.add_subparsers(dest="cmd", required=True)

    # summary
    p = sub.add_parser("summary", help="Archive summary: event counts + top markets")
    p.add_argument("--days", type=int, default=1, help="Number of past days to query")
    p.add_argument("--hours", type=int, default=1, help="Sample hours (1-6); each file ≈200MB")

    # price-series
    p = sub.add_parser("price-series", help="OHLC price series for a specific market")
    p.add_argument("--market", required=True, help="Condition ID (0x...)")
    p.add_argument("--days", type=int, default=1)
    p.add_argument("--hours", type=int, default=None, help="Limit remote query to N hours")
    p.add_argument("--freq", default="5min", help="Candle frequency (pandas offset string)")

    # top-markets
    p = sub.add_parser("top-markets", help="Top markets by trade volume")
    p.add_argument("--days", type=int, default=1)
    p.add_argument("--hours", type=int, default=1, help="Sample hours (1-6); each file ≈200MB")
    p.add_argument("--limit", type=int, default=20)

    # spread-stats
    p = sub.add_parser("spread-stats", help="Spread + volume profile for a market")
    p.add_argument("--market", required=True)
    p.add_argument("--days", type=int, default=1)

    # drift
    p = sub.add_parser("drift", help="Rolling price drift analysis")
    p.add_argument("--market", required=True)
    p.add_argument("--days", type=int, default=1)
    p.add_argument("--window", type=int, default=300, help="Drift window in seconds")

    # download
    p = sub.add_parser("download", help="Download hourly Parquet files locally")
    p.add_argument("--days", type=int, required=True)
    p.add_argument("--out", required=True, help="Output directory")
    p.add_argument("--market", default=None, help="Filter to a specific market (reduces file size)")
    p.add_argument("--progress", default=None, help="Path to progress JSON file (polled by Rust)")

    # analyze-local
    p = sub.add_parser("analyze-local", help="Analyze already-downloaded local Parquet files")
    p.add_argument("--dir", required=True, help="Directory with *.parquet files")
    p.add_argument("--market", default=None)

    # to-ticks — convert parquet → CLOB 1Hz JSONL (for on_tick(ctx) backtester)
    p = sub.add_parser(
        "to-ticks",
        help="Convert local Parquet to 1-Hz JSONL ticks for the CLOB backtester",
    )
    p.add_argument("--in",    dest="input_dir", required=True, help="Directory with local *.parquet files")
    p.add_argument("--market", required=True,  help="Condition ID (0x…)")
    p.add_argument("--slug",   required=True,  help="Slug name (e.g. btc_5m) — used as folder name")
    p.add_argument("--out",    required=True,  help="Output directory (e.g. ~/.traderclaw/workspace/data/ticks/<slug>)")
    p.add_argument("--binance-symbol", dest="binance_symbol", default=None,
                   help="Binance symbol for price feed (e.g. BTCUSDT). Fetches 1m klines to fill binance_price.")
    p.add_argument("--window-minutes", type=int, default=5, help="Window duration in minutes (default 5)")

    # to-ticks-multi — auto-detect all 5-min series and convert in one shot
    p = sub.add_parser(
        "to-ticks-multi",
        help="Auto-detect 5m/15m/1h UP/DOWN markets via Gamma API and convert all to JSONL ticks",
    )
    p.add_argument("--in",  dest="input_dir", required=True, help="Directory with local *.parquet files")
    p.add_argument("--out", required=True, help="Base output directory (slug subdirs created automatically)")
    p.add_argument("--slugs", default=None,
                   help="Comma-separated slugs to process (default: all). E.g. btc_5m,eth_5m,sol_5m")
    p.add_argument("--workspace", default=None,
                   help="Trader-Claw workspace dir (default: ~/.traderclaw/workspace). "
                        "Used to locate polymarket_historical/*.jsonl for condition ID lookup.")

    # to-candles — convert parquet → OHLC JSON (for on_candle(ctx) backtester)
    p = sub.add_parser(
        "to-candles",
        help="Convert local Parquet to OHLC candle JSON for the candle backtester",
    )
    p.add_argument("--dir",    required=True,  help="Directory with local *.parquet files")
    p.add_argument("--market", required=True,  help="Condition ID (0x…)")
    p.add_argument("--slug",   required=True,  help="Slug name (e.g. btc_ob)")
    p.add_argument("--freq",   default="5min", help="Candle frequency: 1min, 5min, 15min, 1h")
    p.add_argument("--out",    default=None,   help="Output dir (default: same as --dir)")

    args = parser.parse_args()

    dispatch = {
        "summary":        cmd_summary,
        "price-series":   cmd_price_series,
        "top-markets":    cmd_top_markets,
        "spread-stats":   cmd_spread_stats,
        "drift":          cmd_drift,
        "download":       cmd_download,
        "analyze-local":  cmd_analyze_local,
        "to-ticks":       cmd_to_ticks,
        "to-ticks-multi": cmd_to_ticks_multi,
        "to-candles":     cmd_to_candles,
    }
    dispatch[args.cmd](args)


if __name__ == "__main__":
    main()
