//! Structural-arb scanner for Polymarket events with multiple related markets.
//!
//! Detects two kinds of risk-free / structural inefficiencies in SLOW markets where HFT
//! does not compete (the bonereaper/wqewqa territory is crypto 5m only — these are months-
//! to-resolution events: SpaceX FDV strikes, "X out by date", elections):
//!
//!   1. SET ARB — disjoint buckets that must sum to exactly $1 at resolution. If the
//!      sum of YES asks is < $1, buying every YES locks in profit; if the sum of YES
//!      bids (i.e. NO asks via 1 - bid) is > $1, selling every YES (or buying every NO)
//!      locks profit. We surface both directions.
//!
//!   2. MONOTONICITY VIOLATIONS — strikes that should be ordered by inclusion.
//!      "By Dec 1" ⊆ "By Dec 31" ⇒ P(by Dec 1) ≤ P(by Dec 31). When a thin book
//!      violates this, you can go long the cheap leg + short the expensive one for
//!      a non-negative-payoff position. Pure structural (no view needed).
//!
//! Output is RANKED BY GROSS EDGE in cents. The user must verify each candidate
//! manually before posting (book depth changes, NegRisk fees, etc.) — this is a
//! discovery tool, not auto-execute.

use anyhow::Result;
use serde::{Deserialize, Serialize};

const GAMMA: &str = "https://gamma-api.polymarket.com";
const CLOB: &str = "https://clob.polymarket.com";

#[derive(Debug, Clone, Serialize)]
pub struct ArbCandidate {
    pub kind: String,           // "set_arb_long" | "set_arb_short" | "monotonicity"
    pub event_title: String,
    pub event_slug: String,
    pub n_markets: usize,
    pub gross_edge_c: f64,      // ¢ per $1 set / per pair
    pub legs: Vec<ArbLeg>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArbLeg {
    pub market_slug: String,
    pub action: String,         // "BUY YES" | "BUY NO" | "SELL YES"
    pub price: f64,
    pub token_id: String,
}

// ── Gamma deserialization ────────────────────────────────────────────────────
#[derive(Deserialize)]
struct GammaEvent {
    #[serde(default)]
    title: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    markets: Vec<GammaMarket>,
}

#[derive(Deserialize)]
struct GammaMarket {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    closed: bool,
    #[serde(rename = "clobTokenIds", default)]
    clob_token_ids: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ClobBook {
    #[serde(default)]
    bids: Vec<ClobLevel>,
    #[serde(default)]
    asks: Vec<ClobLevel>,
}

#[derive(Deserialize)]
struct ClobLevel {
    #[serde(default)]
    price: String,
}

fn parse_token_ids(v: &serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::String(s) => serde_json::from_str::<Vec<String>>(s).unwrap_or_default(),
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => vec![],
    }
}

async fn fetch_book(client: &reqwest::Client, token_id: &str) -> Option<(f64, f64)> {
    let url = format!("{CLOB}/book?token_id={token_id}");
    let b: ClobBook = client.get(&url).send().await.ok()?.json().await.ok()?;
    let bid = b.bids.last().and_then(|l| l.price.parse::<f64>().ok())?;
    let ask = b.asks.last().and_then(|l| l.price.parse::<f64>().ok())?;
    if bid > 0.0 && ask > 0.0 && ask >= bid {
        Some((bid, ask))
    } else {
        None
    }
}

/// Top-of-book for the YES token of every (open) market in an event. Runs the book
/// fetches in parallel — sequential is too slow (~10 markets × 200ms = 2s per event;
/// 50 events would exceed the default curl timeout).
async fn event_yes_quotes(
    client: &reqwest::Client,
    ev: &GammaEvent,
) -> Vec<(String, String, f64, f64)> {
    let pending: Vec<_> = ev
        .markets
        .iter()
        .filter(|m| !m.closed)
        .filter_map(|m| {
            let toks = m.clob_token_ids.as_ref().map(parse_token_ids).unwrap_or_default();
            let yes_tok = toks.into_iter().next()?;
            let slug = m.slug.clone();
            let cli = client.clone();
            Some(async move {
                let res = fetch_book(&cli, &yes_tok).await;
                (slug, yes_tok, res)
            })
        })
        .collect();
    futures::future::join_all(pending)
        .await
        .into_iter()
        .filter_map(|(slug, tok, res)| res.map(|(b, a)| (slug, tok, b, a)))
        .collect()
}

/// Heuristic: only events whose markets look like a DISJOINT bucket cover should be
/// checked for set arb. Cumulative ("by date") and gating events ("X out by Y") are
/// NOT covers — their probabilities should NOT sum to 1, so set-arb math gives huge
/// false positives there. We accept buckets like "between Xt and Yt", "less than Xt",
/// "at least Xt" (price/value strikes), and reject anything containing "by" + a
/// month/date token (those go to the monotonicity check instead).
fn looks_like_disjoint_cover(quotes: &[(String, String, f64, f64)]) -> bool {
    let mut bucket_like = 0usize;
    let mut date_like = 0usize;
    for (slug, _, _, _) in quotes {
        let s = slug.to_lowercase();
        let has_bucket = s.contains("between-") || s.contains("less-than-")
            || s.contains("at-least-") || s.contains("greater-than-")
            || s.contains("under-") || s.contains("over-");
        if has_bucket { bucket_like += 1; }
        // any month name = date strike (cumulative or gating, not a cover)
        let date_words = ["january","february","march","april","may","june","july",
            "august","september","october","november","december",
            "-jan-","-feb-","-mar-","-apr-","-jun-","-jul-","-aug-","-sep-","-oct-","-nov-","-dec-"];
        if date_words.iter().any(|w| s.contains(w)) { date_like += 1; }
    }
    // Cover: ≥80% of legs match bucket pattern AND none look date-strike.
    bucket_like as f64 / quotes.len() as f64 >= 0.8 && date_like == 0
}

/// SET ARB: a disjoint cover (every outcome in exactly one bucket) sums to 1.0.
/// long = buy every YES at ask; profit = $1 - sum(asks).
/// short = sell every YES at bid (≡ buy NO at 1 - bid); profit = sum(bids) - $1.
fn try_set_arb(
    ev_title: &str,
    ev_slug: &str,
    quotes: &[(String, String, f64, f64)],
    threshold_c: f64,
) -> Vec<ArbCandidate> {
    let mut out = Vec::new();
    if quotes.len() < 2 {
        return out;
    }
    // Critical filter — without this, "X out by date" events show as $1.40 set-arb
    // (false positive: they aren't a disjoint cover).
    if !looks_like_disjoint_cover(quotes) {
        return out;
    }
    let sum_ask: f64 = quotes.iter().map(|q| q.3).sum();
    let sum_bid: f64 = quotes.iter().map(|q| q.2).sum();

    // LONG side: buy every YES if total < $1 (gross profit before fees)
    let long_edge = (1.0 - sum_ask) * 100.0;
    if long_edge >= threshold_c {
        out.push(ArbCandidate {
            kind: "set_arb_long".into(),
            event_title: ev_title.into(),
            event_slug: ev_slug.into(),
            n_markets: quotes.len(),
            gross_edge_c: long_edge,
            legs: quotes
                .iter()
                .map(|(slug, tok, _, ask)| ArbLeg {
                    market_slug: slug.clone(),
                    action: "BUY YES".into(),
                    price: *ask,
                    token_id: tok.clone(),
                })
                .collect(),
            note: format!(
                "Sum of YES asks = ${sum_ask:.3} (<$1). Buying every YES costs ${sum_ask:.3} and pays $1 at resolution."
            ),
        });
    }

    // SHORT side: sum of YES bids > $1 ⇒ sum of NO asks < $1 ⇒ same arb on the NO side.
    let short_edge = (sum_bid - 1.0) * 100.0;
    if short_edge >= threshold_c {
        out.push(ArbCandidate {
            kind: "set_arb_short".into(),
            event_title: ev_title.into(),
            event_slug: ev_slug.into(),
            n_markets: quotes.len(),
            gross_edge_c: short_edge,
            legs: quotes
                .iter()
                .map(|(slug, tok, bid, _)| ArbLeg {
                    market_slug: slug.clone(),
                    action: "BUY NO".into(),
                    price: 1.0 - *bid,
                    token_id: tok.clone(),
                })
                .collect(),
            note: format!(
                "Sum of YES bids = ${sum_bid:.3} (>$1). Equivalently, sum of NO asks = ${:.3} — buy every NO and one of them pays $1.",
                quotes.len() as f64 - sum_bid
            ),
        });
    }
    out
}

/// MONOTONICITY: heuristic on slugs that look like ordered cumulative strikes
/// ("by-december-1", "by-december-31"). When two strikes A ⊆ B but P(A) > P(B), surface it.
/// Conservative: only checks "by-<MMM>-<DD>" patterns where we can parse a date from the slug.
fn try_monotonicity(
    ev_title: &str,
    ev_slug: &str,
    quotes: &[(String, String, f64, f64)],
    threshold_c: f64,
) -> Vec<ArbCandidate> {
    let mut out = Vec::new();
    if quotes.len() < 2 {
        return out;
    }
    // Try to extract a date from each slug; skip if any can't be parsed.
    let dated: Vec<(chrono::NaiveDate, &str, &str, f64, f64)> = quotes
        .iter()
        .filter_map(|(slug, tok, bid, ask)| {
            slug_to_date(slug).map(|d| (d, slug.as_str(), tok.as_str(), *bid, *ask))
        })
        .collect();
    if dated.len() < 2 {
        return out;
    }
    // For every pair earlier-date < later-date, P(earlier) must be ≤ P(later).
    // Violation: we can BUY YES on the cheaper later-strike and SELL YES (= BUY NO) on the
    // more expensive earlier-strike. Net cost ≤ 0 in some payoff scenarios → arb.
    for i in 0..dated.len() {
        for j in 0..dated.len() {
            if i == j {
                continue;
            }
            let (de, _, _, _, ask_e) = &dated[i];
            let (dl, _, _, bid_l, _) = &dated[j];
            if de >= dl {
                continue;
            }
            // P(earlier_strike) priced at ask_e ; P(later) priced at bid_l.
            // Violation if ask_e > bid_l (the cheaper-to-sell earlier > the cheap-to-buy later)
            let edge_c = (ask_e - bid_l) * 100.0;
            // Stale-book guard: an edge > 20¢ on a date-ordered pair almost always means
            // one leg's quoted bid/ask is a stale residual, not real liquidity. Real
            // monotonicity violations are 1-5¢ when they happen at all. Skip the giants
            // to keep the surface honest.
            if edge_c > 20.0 {
                continue;
            }
            if edge_c >= threshold_c {
                let leg_sell = ArbLeg {
                    market_slug: dated[i].1.into(),
                    action: "SELL YES (or BUY NO)".into(),
                    price: 1.0 - dated[i].3, // we'd post NO ask = 1 - YES bid (or sell YES at YES bid)
                    token_id: dated[i].2.into(),
                };
                let leg_buy = ArbLeg {
                    market_slug: dated[j].1.into(),
                    action: "BUY YES".into(),
                    price: dated[j].4,
                    token_id: dated[j].2.into(),
                };
                out.push(ArbCandidate {
                    kind: "monotonicity".into(),
                    event_title: ev_title.into(),
                    event_slug: ev_slug.into(),
                    n_markets: 2,
                    gross_edge_c: edge_c,
                    legs: vec![leg_sell, leg_buy],
                    note: format!(
                        "P(earlier {de}) ask={ask_e:.3} > P(later {dl}) bid={bid_l:.3} — order violated."
                    ),
                });
            }
        }
    }
    out
}

/// Best-effort: pull a date out of slugs like "...-by-december-31-2026..." or
/// "...-by-jan-15...". Returns None if we can't confidently parse one.
fn slug_to_date(slug: &str) -> Option<chrono::NaiveDate> {
    let s = slug.to_lowercase();
    let months = [
        ("january", 1), ("jan", 1), ("february", 2), ("feb", 2),
        ("march", 3), ("mar", 3), ("april", 4), ("apr", 4),
        ("may", 5), ("june", 6), ("jun", 6), ("july", 7), ("jul", 7),
        ("august", 8), ("aug", 8), ("september", 9), ("sep", 9), ("sept", 9),
        ("october", 10), ("oct", 10), ("november", 11), ("nov", 11),
        ("december", 12), ("dec", 12),
    ];
    for (name, m) in months {
        if let Some(idx) = s.find(name) {
            let tail = &s[idx + name.len()..];
            // expect "-<DD>-..." or "-<DD>"
            let parts: Vec<&str> = tail.split('-').filter(|p| !p.is_empty()).collect();
            if let Some(day_str) = parts.first() {
                if let Ok(day) = day_str.parse::<u32>() {
                    // year: prefer the next numeric token if present, else current year+1 if month already passed.
                    let year = parts
                        .iter()
                        .skip(1)
                        .find_map(|p| p.parse::<i32>().ok().filter(|y| (2024..2030).contains(y)))
                        .unwrap_or_else(|| chrono::Utc::now().date_naive().year_ce().1 as i32);
                    if let Some(d) = chrono::NaiveDate::from_ymd_opt(year, m, day) {
                        return Some(d);
                    }
                }
            }
        }
    }
    None
}

/// Scan all open events with ≥2 markets for set-arb and monotonicity violations.
/// `threshold_c` filters out edges below N cents of gross profit.
pub async fn scan_arb_opportunities(
    max_events: usize,
    threshold_c: f64,
) -> Result<Vec<ArbCandidate>> {
    let client = reqwest::Client::builder()
        .user_agent("trader-claw/arb-scanner")
        .build()?;
    // Pull active, not-closed events with embedded markets. Pagination via offset.
    let mut events: Vec<GammaEvent> = Vec::new();
    let mut offset = 0usize;
    while events.len() < max_events {
        let url = format!(
            "{GAMMA}/events?limit=100&offset={offset}&active=true&closed=false"
        );
        let batch: Vec<GammaEvent> = match client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
        {
            Ok(v) => v,
            Err(_) => break,
        };
        if batch.is_empty() {
            break;
        }
        let len = batch.len();
        events.extend(batch);
        offset += len;
        if len < 100 {
            break;
        }
    }
    events.truncate(max_events);

    // Process events in parallel chunks (cap concurrency to avoid overwhelming the CLOB).
    use futures::stream::StreamExt;
    const EVENT_CONCURRENCY: usize = 8;
    let candidates_iter = futures::stream::iter(
        events
            .into_iter()
            .filter(|e| e.active && !e.closed && e.markets.len() >= 2)
            .map(|ev| {
                let cli = client.clone();
                async move {
                    let quotes = event_yes_quotes(&cli, &ev).await;
                    if quotes.len() < 2 {
                        return Vec::new();
                    }
                    let mut v = try_set_arb(&ev.title, &ev.slug, &quotes, threshold_c);
                    v.extend(try_monotonicity(&ev.title, &ev.slug, &quotes, threshold_c));
                    v
                }
            }),
    )
    .buffer_unordered(EVENT_CONCURRENCY);
    let chunks: Vec<Vec<ArbCandidate>> = candidates_iter.collect().await;
    let mut out: Vec<ArbCandidate> = chunks.into_iter().flatten().collect();
    out.sort_by(|a, b| b.gross_edge_c.partial_cmp(&a.gross_edge_c).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

#[doc(hidden)]
use chrono::Datelike;
