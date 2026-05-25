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
    con = duckdb.connect()
    # Allow more threads and memory for big queries
    con.execute("SET threads=4")
    con.execute("SET memory_limit='512MB'")
    # R2 requires a User-Agent header (403 otherwise)
    try:
        con.execute("SET http_user_agent='orderbook-parser/1.0'")
    except Exception:
        pass  # older DuckDB versions may not support this
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
    import urllib.request

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
                # Use DuckDB to filter and re-write as a smaller local Parquet
                con = get_con()
                con.execute(f"""
                COPY (
                    SELECT * FROM read_parquet('{url}', hive_partitioning=false)
                    WHERE CAST(market AS VARCHAR) = '{market_id}'
                ) TO '{out_path}' (FORMAT PARQUET, COMPRESSION 'zstd')
                """)
            else:
                # Full download
                urllib.request.urlretrieve(url, out_path)
            downloaded += 1
        except Exception as e:
            err_msg = f"{hour_str}: {e}"
            errors.append(err_msg)
            # Don't abort — some hours may be missing (archive skips empty hours)

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

    args = parser.parse_args()

    dispatch = {
        "summary": cmd_summary,
        "price-series": cmd_price_series,
        "top-markets": cmd_top_markets,
        "spread-stats": cmd_spread_stats,
        "drift": cmd_drift,
        "download": cmd_download,
        "analyze-local": cmd_analyze_local,
    }
    dispatch[args.cmd](args)


if __name__ == "__main__":
    main()
