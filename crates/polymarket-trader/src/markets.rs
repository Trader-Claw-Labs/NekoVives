use serde::{Deserialize, Serialize};
use anyhow::{anyhow, Result};

/// Filter options for listing markets
#[derive(Default)]
pub struct MarketFilter {
    pub category: Option<String>,
    pub min_volume_usdc: Option<f64>,
    pub min_liquidity_usdc: Option<f64>,
    pub active_only: bool,
    /// Free-text search (passed as `question_mid_partial` to Gamma API)
    pub query: Option<String>,
    /// Max number of results to return (default 50)
    pub limit: Option<usize>,
}

/// Polymarket prediction market
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub condition_id: String,
    pub question: String,
    pub slug: String,
    pub yes_token_id: String,
    pub no_token_id: String,
    pub volume: f64,
    pub liquidity: f64,
    pub end_date_iso: Option<String>,
    pub category: Option<String>,
}

// --- Internal deserialization helpers ---

#[derive(Deserialize)]
struct GammaMarket {
    #[serde(rename = "conditionId", default)]
    condition_id: String,
    question: String,
    slug: String,
    #[serde(default)]
    tokens: Vec<GammaToken>,
    #[serde(default)]
    volume: serde_json::Value,
    #[serde(default)]
    liquidity: serde_json::Value,
    #[serde(rename = "endDateIso")]
    end_date_iso: Option<String>,
    category: Option<String>,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    closed: bool,
}

#[derive(Deserialize)]
struct GammaToken {
    token_id: String,
    outcome: String,
}

#[derive(Deserialize)]
struct ClobPriceResponse {
    price: String,
}

#[derive(Deserialize)]
struct ClobMarketResponse {
    condition_id: String,
    tokens: Vec<ClobToken>,
}

#[derive(Deserialize)]
struct ClobToken {
    token_id: String,
    outcome: String,
}

#[derive(Deserialize)]
struct GammaEvent {
    markets: Vec<GammaMarket>,
}

fn value_to_f64(v: &serde_json::Value) -> f64 {
    if let Some(n) = v.as_f64() {
        return n;
    }
    if let Some(s) = v.as_str() {
        return s.parse::<f64>().unwrap_or(0.0);
    }
    0.0
}

fn gamma_to_market(g: GammaMarket) -> Option<Market> {
    let yes_token = g.tokens.iter().find(|t|
        t.outcome.eq_ignore_ascii_case("Yes") || t.outcome.eq_ignore_ascii_case("Up")
    )?;
    let no_token = g.tokens.iter().find(|t|
        t.outcome.eq_ignore_ascii_case("No") || t.outcome.eq_ignore_ascii_case("Down")
    )?;

    Some(Market {
        condition_id: g.condition_id,
        question: g.question,
        slug: g.slug,
        yes_token_id: yes_token.token_id.clone(),
        no_token_id: no_token.token_id.clone(),
        volume: value_to_f64(&g.volume),
        liquidity: value_to_f64(&g.liquidity),
        end_date_iso: g.end_date_iso,
        category: g.category,
    })
}

fn apply_filter(markets: Vec<GammaMarket>, filter: &MarketFilter) -> Vec<Market> {
    markets
        .into_iter()
        .filter(|m| {
            if filter.active_only && (!m.active || m.closed) {
                return false;
            }
            if let Some(ref cat) = filter.category {
                if m.category.as_deref().unwrap_or("") != cat.as_str() {
                    return false;
                }
            }
            true
        })
        .filter_map(|m| {
            let vol = value_to_f64(&m.volume);
            let liq = value_to_f64(&m.liquidity);
            if let Some(min_vol) = filter.min_volume_usdc {
                if vol < min_vol {
                    return None;
                }
            }
            if let Some(min_liq) = filter.min_liquidity_usdc {
                if liq < min_liq {
                    return None;
                }
            }
            gamma_to_market(m)
        })
        .collect()
}

/// List markets from Gamma API.
/// Handles both flat `[...]` and paginated `{"data":[...]}` response shapes.
pub async fn list_markets(filter: MarketFilter) -> Result<Vec<Market>> {
    let client = reqwest::Client::new();
    let limit = filter.limit.unwrap_or(50);
    let mut url = format!("https://gamma-api.polymarket.com/markets?limit={limit}&order=volume&ascending=false");
    if filter.active_only {
        url.push_str("&active=true&closed=false");
    }
    if let Some(ref q) = filter.query {
        if !q.is_empty() {
            let encoded = q.replace(' ', "+");
            url.push_str(&format!("&question_mid_partial={encoded}"));
        }
    }

    let bytes = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    // Try flat array first, then paginated wrapper
    let raw: Vec<GammaMarket> = if let Ok(v) = serde_json::from_slice::<Vec<GammaMarket>>(&bytes) {
        v
    } else {
        #[derive(serde::Deserialize)]
        struct Paged { data: Vec<GammaMarket> }
        serde_json::from_slice::<Paged>(&bytes)
            .map(|p| p.data)
            .map_err(|e| anyhow::anyhow!("Gamma API parse error: {e}\nBody: {}", String::from_utf8_lossy(&bytes[..bytes.len().min(300)])))?
    };

    Ok(apply_filter(raw, &filter))
}

/// Get a single market by slug.
/// GET https://gamma-api.polymarket.com/markets?slug=<slug>
///
/// For recurring binary markets Gamma may return multiple entries (expired,
/// current, future). We scan all results, keep only active ones with valid
/// Yes/No tokens, and pick the best by liquidity + volume.
pub async fn get_market(slug: &str) -> Result<Market> {
    let client = reqwest::Client::new();
    let url = format!("https://gamma-api.polymarket.com/markets?slug={}", slug);

    let raw: Vec<GammaMarket> = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let mut best = raw
        .into_iter()
        .filter(|g| g.active && !g.closed)
        .filter_map(gamma_to_market)
        .filter(|m| !m.yes_token_id.trim().is_empty())
        .max_by(|a, b| {
            let av = a.liquidity + a.volume;
            let bv = b.liquidity + b.volume;
            av.partial_cmp(&bv).unwrap_or(std::cmp::Ordering::Equal)
        });

    // For newer market types (NegRisk / recurrent binary), Gamma token IDs
    // and CLOB token IDs differ. ALWAYS fetch CLOB tokens by condition_id
    // and replace Gamma's tokens when CLOB returns valid ones.
    if let Some(ref mut market) = best {
        if !market.condition_id.is_empty() {
            match fetch_clob_tokens(&client, &market.condition_id).await {
                Ok(clob_tokens) => {
                    if let Some(yes) = clob_tokens.iter().find(|t|
                        t.outcome.eq_ignore_ascii_case("Yes") || t.outcome.eq_ignore_ascii_case("Up")
                    ) {
                        market.yes_token_id = yes.token_id.clone();
                    }
                    if let Some(no) = clob_tokens.iter().find(|t|
                        t.outcome.eq_ignore_ascii_case("No") || t.outcome.eq_ignore_ascii_case("Down")
                    ) {
                        market.no_token_id = no.token_id.clone();
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[POLY-MARKETS] CLOB token fetch failed for condition_id {}: {}. Keeping Gamma tokens.",
                        market.condition_id, e
                    );
                }
            }
        }
    }

    // Fallback: if Gamma /markets returned nothing, try the /events endpoint.
    if best.is_none() {
        let event_url = format!("https://gamma-api.polymarket.com/events?slug={}", slug);
        let events: Vec<GammaEvent> = client.get(&event_url).send().await?.json().await.unwrap_or_default();

        if let Some(event) = events.into_iter().next() {
            if let Some(mut m) = event.markets.into_iter().find(|m| m.slug == slug) {
                if m.tokens.is_empty() && !m.condition_id.is_empty() {
                    match fetch_clob_tokens(&client, &m.condition_id).await {
                        Ok(tokens) => m.tokens = tokens,
                        Err(e) => {
                            tracing::warn!("[POLY-MARKETS] CLOB token fetch failed in events fallback: {}", e);
                        }
                    }
                }

                if let Some(market) = gamma_to_market(m) {
                    if !market.yes_token_id.trim().is_empty() {
                        best = Some(market);
                    }
                }
            }
        }
    }

    best.ok_or_else(|| anyhow!("No active market with valid tokens found for slug: {}", slug))
}

/// Fetch token IDs from the CLOB /markets/{condition_id} endpoint.
/// Returns a list of GammaToken-style structs with the CLOB token IDs.
async fn fetch_clob_tokens(client: &reqwest::Client, condition_id: &str) -> Result<Vec<GammaToken>> {
    let clob_url = format!("https://clob.polymarket.com/markets/{}", condition_id);
    tracing::info!("[POLY-MARKETS] CLOB token fetch: GET {}", clob_url);

    let resp = client
        .get(&clob_url)
        .send()
        .await
        .map_err(|e| anyhow!("CLOB request failed: {}", e))?;

    let status = resp.status();
    let raw_body = resp.text().await.unwrap_or_default();
    tracing::info!(
        "[POLY-MARKETS] CLOB token fetch response: {} | {}",
        status,
        &raw_body[..raw_body.len().min(500)]
    );

    if !status.is_success() {
        anyhow::bail!("CLOB returned non-success status: {}", status);
    }

    // Try the expected structured format first
    if let Ok(clob_market) = serde_json::from_str::<ClobMarketResponse>(&raw_body) {
        tracing::info!(
            "[POLY-MARKETS] Parsed ClobMarketResponse with {} tokens",
            clob_market.tokens.len()
        );
        return Ok(clob_market.tokens.into_iter().map(|t| GammaToken {
            token_id: t.token_id,
            outcome: t.outcome,
        }).collect());
    }

    // Fallback: try to extract token IDs from generic JSON
    let value: serde_json::Value = serde_json::from_str(&raw_body)
        .map_err(|e| anyhow!("CLOB response is not valid JSON: {}", e))?;

    if let Some(tokens) = value.get("tokens").and_then(|v| v.as_array()) {
        tracing::info!(
            "[POLY-MARKETS] Extracting {} tokens from generic JSON",
            tokens.len()
        );
        let extracted: Vec<GammaToken> = tokens.iter().filter_map(|t| {
            let token_id = t.get("token_id")
                .or_else(|| t.get("tokenId"))
                .or_else(|| t.get("asset_id"))
                .and_then(|v| v.as_str())?;
            let outcome = t.get("outcome")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(GammaToken {
                token_id: token_id.to_string(),
                outcome: outcome.to_string(),
            })
        }).collect();
        if !extracted.is_empty() {
            return Ok(extracted);
        }
    }

    anyhow::bail!("Could not extract tokens from CLOB response")
}

/// Get YES token price (0.0 to 1.0).
/// GET https://clob.polymarket.com/price?token_id=<token_id>&side=buy
pub async fn get_market_price(token_id: &str) -> Result<f64> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://clob.polymarket.com/price?token_id={}&side=buy",
        token_id
    );

    let resp: ClobPriceResponse = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let price = resp
        .price
        .parse::<f64>()
        .map_err(|e| anyhow!("Failed to parse price: {}", e))?;

    Ok(price)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_gamma_response() -> &'static str {
        r#"[{
            "conditionId": "0xabc",
            "question": "Will BTC reach 100k?",
            "slug": "will-btc-reach-100k",
            "tokens": [
                {"token_id": "123", "outcome": "Yes"},
                {"token_id": "456", "outcome": "No"}
            ],
            "volume": "50000.00",
            "liquidity": "1000.00",
            "endDateIso": "2025-12-31T00:00:00Z",
            "category": "crypto",
            "active": true,
            "closed": false
        }]"#
    }

    #[test]
    fn test_parse_gamma_market() {
        let raw: Vec<GammaMarket> =
            serde_json::from_str(sample_gamma_response()).expect("parse failed");
        assert_eq!(raw.len(), 1);

        let market = gamma_to_market(raw.into_iter().next().unwrap())
            .expect("conversion failed");

        assert_eq!(market.condition_id, "0xabc");
        assert_eq!(market.question, "Will BTC reach 100k?");
        assert_eq!(market.slug, "will-btc-reach-100k");
        assert_eq!(market.yes_token_id, "123");
        assert_eq!(market.no_token_id, "456");
        assert!((market.volume - 50000.0).abs() < 0.01);
        assert!((market.liquidity - 1000.0).abs() < 0.01);
        assert_eq!(
            market.end_date_iso.as_deref(),
            Some("2025-12-31T00:00:00Z")
        );
        assert_eq!(market.category.as_deref(), Some("crypto"));
    }

    #[test]
    fn test_filter_active_only() {
        // Build a response with one active and one closed market
        let json = r#"[
            {
                "conditionId": "0x1",
                "question": "Active market?",
                "slug": "active-market",
                "tokens": [
                    {"token_id": "1", "outcome": "Yes"},
                    {"token_id": "2", "outcome": "No"}
                ],
                "volume": "100.0",
                "liquidity": "50.0",
                "active": true,
                "closed": false
            },
            {
                "conditionId": "0x2",
                "question": "Closed market?",
                "slug": "closed-market",
                "tokens": [
                    {"token_id": "3", "outcome": "Yes"},
                    {"token_id": "4", "outcome": "No"}
                ],
                "volume": "200.0",
                "liquidity": "80.0",
                "active": false,
                "closed": true
            }
        ]"#;

        let raw: Vec<GammaMarket> = serde_json::from_str(json).expect("parse failed");
        let filter = MarketFilter {
            active_only: true,
            ..Default::default()
        };
        let markets = apply_filter(raw, &filter);
        assert_eq!(markets.len(), 1);
        assert_eq!(markets[0].slug, "active-market");
    }

    #[test]
    fn test_market_filter_default() {
        let filter = MarketFilter::default();
        assert!(filter.category.is_none());
        assert!(filter.min_volume_usdc.is_none());
        assert!(filter.min_liquidity_usdc.is_none());
        assert!(!filter.active_only);

        // With default filter, all valid markets pass through
        let raw: Vec<GammaMarket> =
            serde_json::from_str(sample_gamma_response()).expect("parse failed");
        let markets = apply_filter(raw, &filter);
        assert_eq!(markets.len(), 1);
    }

    #[tokio::test]
    #[ignore]
    async fn test_list_markets_network() {
        let filter = MarketFilter {
            active_only: true,
            ..Default::default()
        };
        let markets = list_markets(filter).await.expect("network call failed");
        assert!(!markets.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_market_network() {
        // Use a real slug from Polymarket
        let market = get_market("will-btc-reach-100k-in-2024")
            .await
            .expect("network call failed");
        assert!(!market.slug.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_price_network() {
        // token_id would come from a real market query
        let price = get_market_price("123").await;
        // Just ensure no panic; network result may vary
        println!("price result: {:?}", price);
    }
}
