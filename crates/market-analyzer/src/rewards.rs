//! Polymarket Liquidity-Rewards market scanner.
//!
//! Polymarket pays makers a daily USDC reward for posting two-sided resting orders
//! within `max_spread` of the midpoint in *designated incentivized* markets. The set
//! of incentivized markets is the CLOB `sampling-markets` endpoint (paginated).
//!
//! The edge here is execution/structural, NOT directional: you earn rewards for
//! providing liquidity regardless of fills. The dominant RISK is adverse selection —
//! informed flow picking off your stale quotes. That risk is highest in markets whose
//! fair value tracks a fast public reference (crypto UP/DOWN, hourly), and lowest in
//! slow-moving markets (politics, elections resolving far out). This scanner surfaces
//! reward-eligible markets and ranks them by a reward/adverse-selection-risk heuristic,
//! flagging the toxic (crypto/short-dated) ones so they can be excluded.

use anyhow::Result;
use serde::{Deserialize, Serialize};

const CLOB: &str = "https://clob.polymarket.com";

/// One reward-eligible market, enriched with a tradeability score.
#[derive(Debug, Clone, Serialize)]
pub struct RewardMarket {
    pub condition_id: String,
    pub question: String,
    pub market_slug: String,
    pub end_date_iso: Option<String>,
    /// Max `rewards_daily_rate` across the market's reward assets.
    pub daily_rate: f64,
    /// Eligible spread band in cents — quotes within this of the inside score rewards.
    pub max_spread: f64,
    /// Minimum order size to qualify for rewards.
    pub min_size: f64,
    pub neg_risk: bool,
    pub tags: Vec<String>,
    /// True when the market resolves on a fast public reference (crypto / UP-DOWN /
    /// hourly) — high adverse-selection toxicity, unsuitable for slow maker quoting.
    pub is_toxic: bool,
    pub days_to_end: Option<f64>,
    /// reward-per-risk heuristic (higher = better). 0 for toxic markets.
    pub score: f64,
    /// "high" | "medium" | "low" | "toxic" — qualitative safety of maker quoting here.
    pub safety: String,
    /// CLOB token ids for posting maker quotes (YES then NO). Empty if unavailable.
    pub yes_token_id: Option<String>,
    pub no_token_id: Option<String>,
    /// YES token mid price (0-1). ~0.5 = balanced book (maker fills both sides);
    /// near 0/1 = skewed longshot (quotes rarely cross). Drives the balance factor.
    pub yes_price: f64,
}

// ── CLOB sampling-markets deserialization ───────────────────────────────────────
#[derive(Deserialize)]
struct SamplingResponse {
    data: Vec<RawMarket>,
    #[serde(default)]
    next_cursor: String,
}

#[derive(Deserialize)]
struct RawMarket {
    #[serde(default)]
    condition_id: String,
    #[serde(default)]
    question: String,
    #[serde(default)]
    market_slug: String,
    #[serde(default)]
    end_date_iso: Option<String>,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    accepting_orders: bool,
    #[serde(default)]
    neg_risk: bool,
    #[serde(default)]
    rewards: Option<RawRewards>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    tokens: Option<Vec<RawToken>>,
}

#[derive(Deserialize)]
struct RawToken {
    #[serde(default)]
    token_id: String,
    #[serde(default)]
    outcome: String,
    /// Token mid/last price (0-1) from sampling-markets. Used to gauge how balanced
    /// the book is — maker fills come from balanced markets, not 0.1/0.9 longshots.
    #[serde(default)]
    price: f64,
}

#[derive(Deserialize)]
struct RawRewards {
    #[serde(default)]
    rates: Option<Vec<RawRate>>,
    #[serde(default)]
    min_size: f64,
    #[serde(default)]
    max_spread: f64,
}

#[derive(Deserialize)]
struct RawRate {
    #[serde(default)]
    rewards_daily_rate: f64,
}

/// A market is toxic for maker quoting when its fair value tracks a fast public price.
fn is_toxic(question: &str, tags: &[String]) -> bool {
    let q = question.to_lowercase();
    if q.contains("up or down") || q.contains("up/down") {
        return true;
    }
    tags.iter().any(|t| {
        let t = t.to_lowercase();
        t == "crypto" || t == "bitcoin" || t == "ethereum" || t == "hourly"
    })
}

/// Days from now until `end_date_iso`, or None if unparseable.
fn days_to_end(end_iso: &Option<String>) -> Option<f64> {
    let s = end_iso.as_ref()?;
    let end = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    let secs = end.timestamp() - chrono::Utc::now().timestamp();
    Some(secs as f64 / 86_400.0)
}

/// Reward-per-risk heuristic. Toxic markets score 0 (excluded). Otherwise reward grows
/// with the daily rate and the eligible spread (wider band = quote further from mid =
/// less adverse selection), and with longer time-to-resolution (slower fair value).
///
/// `yes_price` (0-1) gauges how BALANCED the book is. A balanced market (~0.50) gives
/// maker fills on BOTH sides and the best reward-efficiency; a skewed longshot (~0.1 /
/// ~0.9) almost never crosses your quotes (the World-Cup-winner markets the pilot kept
/// quoting with 0 fills). We multiply the score by a balance factor and DOWNGRADE the
/// safety label of very skewed markets so a `min_safety = high` orchestrator skips them.
fn score_market(daily_rate: f64, max_spread: f64, days: Option<f64>, toxic: bool, yes_price: f64) -> (f64, String) {
    if toxic {
        return (0.0, "toxic".to_string());
    }
    // Already-resolving / expired markets cannot be farmed — drop them to the bottom.
    if let Some(d) = days {
        if d < 0.5 {
            return (0.0, "expiring".to_string());
        }
    }
    let horizon = days.unwrap_or(1.0).max(0.05);
    // Wider eligible spread and longer horizon both reduce adverse-selection risk.
    let safety_mult = (max_spread / 2.0).min(3.0) * (1.0 + horizon.min(30.0) / 30.0);
    // Balance factor: 1.0 at mid=0.50, decaying to ~0 at the 0/1 extremes. Skew = how
    // far the YES price is from a coin-flip. When the price is unknown (0), assume
    // mildly balanced (0.5) rather than penalising it to zero.
    let skew = if yes_price > 0.0 { (yes_price - 0.5).abs() } else { 0.25 };
    let balance = (1.0 - 2.0 * skew).max(0.0); // 0.50→1.0, 0.25/0.75→0.5, ≤0.05/≥0.95→~0
    let score = daily_rate * safety_mult * (0.15 + 0.85 * balance);
    // Base label from horizon + spread, then cap by balance: a heavily skewed book can
    // be at most "medium" (≤0.15 / ≥0.85 → "low"), so it won't pass a high-only pool.
    let base = if horizon >= 7.0 && max_spread >= 3.0 {
        "high"
    } else if horizon >= 1.0 && max_spread >= 1.5 {
        "medium"
    } else {
        "low"
    };
    let label = if balance < 0.30 {
        "low"           // very skewed (≤0.35 / ≥0.65 in YES price): maker rarely fills
    } else if balance < 0.60 && base == "high" {
        "medium"        // moderately skewed: demote from high
    } else {
        base
    };
    (score, label.to_string())
}

/// Fetch all incentivized markets (paginated) and rank them. `max_pages` caps the
/// CLOB round-trips (each page = up to 1000 markets).
pub async fn scan_reward_markets(max_pages: usize) -> Result<Vec<RewardMarket>> {
    let client = reqwest::Client::builder()
        .user_agent("trader-claw/rewards-scanner")
        .build()?;

    let mut out: Vec<RewardMarket> = Vec::new();
    let mut cursor = String::new();
    for _ in 0..max_pages.max(1) {
        let url = if cursor.is_empty() {
            format!("{CLOB}/sampling-markets")
        } else {
            format!("{CLOB}/sampling-markets?next_cursor={cursor}")
        };
        let resp: SamplingResponse = client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        for m in resp.data {
            if m.closed || !m.active || !m.accepting_orders {
                continue;
            }
            let rewards = match m.rewards {
                Some(r) => r,
                None => continue,
            };
            let daily_rate = rewards
                .rates
                .unwrap_or_default()
                .iter()
                .map(|r| r.rewards_daily_rate)
                .fold(0.0_f64, f64::max);
            if daily_rate <= 0.0 {
                continue;
            }
            let tags = m.tags.unwrap_or_default();
            let toxic = is_toxic(&m.question, &tags);
            let days = days_to_end(&m.end_date_iso);
            let toks = m.tokens.unwrap_or_default();
            let yes_tok = toks.iter().find(|t| t.outcome.eq_ignore_ascii_case("yes"));
            let no_tok = toks.iter().find(|t| t.outcome.eq_ignore_ascii_case("no"));
            // YES mid: prefer the YES token's price; else derive from NO (1 - no_price).
            let yes_price = match (yes_tok.map(|t| t.price), no_tok.map(|t| t.price)) {
                (Some(p), _) if p > 0.0 => p,
                (_, Some(np)) if np > 0.0 => 1.0 - np,
                _ => 0.0,
            };
            let (score, safety) = score_market(daily_rate, rewards.max_spread, days, toxic, yes_price);
            let yes_token_id = yes_tok.map(|t| t.token_id.clone());
            let no_token_id = no_tok.map(|t| t.token_id.clone());
            out.push(RewardMarket {
                condition_id: m.condition_id,
                question: m.question,
                market_slug: m.market_slug,
                end_date_iso: m.end_date_iso,
                daily_rate,
                max_spread: rewards.max_spread,
                min_size: rewards.min_size,
                neg_risk: m.neg_risk,
                tags,
                is_toxic: toxic,
                days_to_end: days,
                score,
                safety,
                yes_token_id,
                no_token_id,
                yes_price,
            });
        }

        // CLOB signals end-of-pages with cursor "LTE=" (base64 "-1") or empty.
        if resp.next_cursor.is_empty() || resp.next_cursor == "LTE=" {
            break;
        }
        cursor = resp.next_cursor;
    }

    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}
