//! Market series registry — defines recurring Polymarket binary market series.
//!
//! A "series" is a recurring Polymarket market that repeats on a fixed cadence,
//! e.g. BTC UP/DOWN every 5 minutes, or Munich max temperature every day.
//! Each series has:
//!   - A slug prefix (the deterministic part of the slug)
//!   - A data source (Binance price feed, Open-Meteo weather, etc.)
//!   - A resolution logic (did the price go up? did temp exceed threshold?)
//!   - A cadence (how often a new window opens)

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    /// Binance REST API — 1m OHLCV candles, used as Chainlink oracle proxy
    Binance,
    /// Open-Meteo archive API — daily weather observations
    OpenMeteo,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionLogic {
    /// YES if close price > window open price
    PriceUp,
    /// YES if value > threshold
    ThresholdAbove,
    /// YES if value < threshold
    ThresholdBelow,
}

/// A recurring Polymarket binary market series.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MarketSeries {
    /// Unique identifier, used as `series_id` in BacktestConfig
    pub id: String,
    /// Human-readable label shown in UI
    pub label: String,
    /// Polymarket slug prefix (e.g. "btc-updown-5m", "highest-temperature-in-munich")
    pub slug_prefix: String,
    /// Where to fetch historical data for backtesting
    pub data_source: DataSource,
    /// Instrument identifier within the data source
    ///   Binance: trading pair ("BTCUSDT")
    ///   OpenMeteo: city code ("munich", "london", "nyc")
    pub symbol: String,
    /// How often a new market window opens ("5m", "15m", "1d", ...)
    pub cadence: String,
    /// How the market resolves
    pub resolution_logic: ResolutionLogic,
    /// Threshold value for ThresholdAbove/Below resolution (e.g. 25.0 for °C)
    pub threshold: Option<f64>,
    /// Unit of the threshold / resolution value (e.g. "°C", "$", "%")
    pub unit: Option<String>,
    /// One-line description of the market question
    pub description: String,
    /// Suggested default strategy script filename
    pub default_script: Option<String>,
    /// Whether this series is built-in (true) or user-defined (false)
    pub builtin: bool,
}

/// All built-in market series.
pub fn builtin_series() -> Vec<MarketSeries> {
    vec![
        // ── BTC UP/DOWN (Binance / price_up) ────────────────────────────────
        MarketSeries {
            id: "btc_5m".into(),
            label: "BTC UP/DOWN 5-min".into(),
            slug_prefix: "btc-updown-5m".into(),
            data_source: DataSource::Binance,
            symbol: "BTCUSDT".into(),
            cadence: "5m".into(),
            resolution_logic: ResolutionLogic::PriceUp,
            threshold: None,
            unit: Some("$".into()),
            description: "Will BTC/USD be higher 5 minutes from now?".into(),
            default_script: Some("polymarket_btc_binary.rhai".into()),
            builtin: true,
        },
        MarketSeries {
            id: "btc_15m".into(),
            label: "BTC UP/DOWN 15-min".into(),
            slug_prefix: "btc-updown-15m".into(),
            data_source: DataSource::Binance,
            symbol: "BTCUSDT".into(),
            cadence: "15m".into(),
            resolution_logic: ResolutionLogic::PriceUp,
            threshold: None,
            unit: Some("$".into()),
            description: "Will BTC/USD be higher 15 minutes from now?".into(),
            default_script: Some("polymarket_btc_binary.rhai".into()),
            builtin: true,
        },
        MarketSeries {
            id: "btc_1h".into(),
            label: "BTC UP/DOWN 1-hour".into(),
            slug_prefix: "btc-updown-1h".into(),
            data_source: DataSource::Binance,
            symbol: "BTCUSDT".into(),
            cadence: "1h".into(),
            resolution_logic: ResolutionLogic::PriceUp,
            threshold: None,
            unit: Some("$".into()),
            description: "Will BTC/USD be higher in 1 hour?".into(),
            default_script: Some("polymarket_btc_binary.rhai".into()),
            builtin: true,
        },
        // ── SOL UP/DOWN (Binance / price_up) ────────────────────────────────
        MarketSeries {
            id: "sol_5m".into(),
            label: "SOL UP/DOWN 5-min".into(),
            slug_prefix: "sol-updown-5m".into(),
            data_source: DataSource::Binance,
            symbol: "SOLUSDT".into(),
            cadence: "5m".into(),
            resolution_logic: ResolutionLogic::PriceUp,
            threshold: None,
            unit: Some("$".into()),
            description: "Will SOL/USD be higher 5 minutes from now?".into(),
            default_script: Some("polymarket_btc_binary.rhai".into()),
            builtin: true,
        },
        // ── XRP UP/DOWN (Binance / price_up) ────────────────────────────────
        MarketSeries {
            id: "xrp_5m".into(),
            label: "XRP UP/DOWN 5-min".into(),
            slug_prefix: "xrp-updown-5m".into(),
            data_source: DataSource::Binance,
            symbol: "XRPUSDT".into(),
            cadence: "5m".into(),
            resolution_logic: ResolutionLogic::PriceUp,
            threshold: None,
            unit: Some("$".into()),
            description: "Will XRP/USD be higher 5 minutes from now?".into(),
            default_script: Some("polymarket_btc_binary.rhai".into()),
            builtin: true,
        },
        // ── DOGE UP/DOWN (Binance / price_up) ───────────────────────────────
        MarketSeries {
            id: "doge_5m".into(),
            label: "DOGE UP/DOWN 5-min".into(),
            slug_prefix: "doge-updown-5m".into(),
            data_source: DataSource::Binance,
            symbol: "DOGEUSDT".into(),
            cadence: "5m".into(),
            resolution_logic: ResolutionLogic::PriceUp,
            threshold: None,
            unit: Some("$".into()),
            description: "Will DOGE/USD be higher 5 minutes from now?".into(),
            default_script: Some("polymarket_btc_binary.rhai".into()),
            builtin: true,
        },
        // ── HYPE UP/DOWN (Binance / price_up) ───────────────────────────────
        MarketSeries {
            id: "hype_5m".into(),
            label: "HYPE UP/DOWN 5-min".into(),
            slug_prefix: "hype-updown-5m".into(),
            data_source: DataSource::Binance,
            symbol: "HYPEUSDT".into(),
            cadence: "5m".into(),
            resolution_logic: ResolutionLogic::PriceUp,
            threshold: None,
            unit: Some("$".into()),
            description: "Will HYPE/USD be higher 5 minutes from now?".into(),
            default_script: Some("polymarket_btc_binary.rhai".into()),
            builtin: true,
        },
        // ── BNB UP/DOWN (Binance / price_up) ────────────────────────────────
        MarketSeries {
            id: "bnb_5m".into(),
            label: "BNB UP/DOWN 5-min".into(),
            slug_prefix: "bnb-updown-5m".into(),
            data_source: DataSource::Binance,
            symbol: "BNBUSDT".into(),
            cadence: "5m".into(),
            resolution_logic: ResolutionLogic::PriceUp,
            threshold: None,
            unit: Some("$".into()),
            description: "Will BNB/USD be higher 5 minutes from now?".into(),
            default_script: Some("polymarket_btc_binary.rhai".into()),
            builtin: true,
        },
        // ── ETH UP/DOWN (Binance / price_up) ────────────────────────────────
        MarketSeries {
            id: "eth_5m".into(),
            label: "ETH UP/DOWN 5-min".into(),
            slug_prefix: "eth-updown-5m".into(),
            data_source: DataSource::Binance,
            symbol: "ETHUSDT".into(),
            cadence: "5m".into(),
            resolution_logic: ResolutionLogic::PriceUp,
            threshold: None,
            unit: Some("$".into()),
            description: "Will ETH/USD be higher 5 minutes from now?".into(),
            default_script: Some("polymarket_btc_binary.rhai".into()),
            builtin: true,
        },
        MarketSeries {
            id: "eth_15m".into(),
            label: "ETH UP/DOWN 15-min".into(),
            slug_prefix: "eth-updown-15m".into(),
            data_source: DataSource::Binance,
            symbol: "ETHUSDT".into(),
            cadence: "15m".into(),
            resolution_logic: ResolutionLogic::PriceUp,
            threshold: None,
            unit: Some("$".into()),
            description: "Will ETH/USD be higher 15 minutes from now?".into(),
            default_script: Some("polymarket_btc_binary.rhai".into()),
            builtin: true,
        },
        // ── Weather — daily temperature (Open-Meteo / threshold_above) ──────
        MarketSeries {
            id: "munich_temp_daily".into(),
            label: "Munich Max Temp (daily)".into(),
            slug_prefix: "highest-temperature-in-munich".into(),
            data_source: DataSource::OpenMeteo,
            symbol: "munich".into(),
            cadence: "1d".into(),
            resolution_logic: ResolutionLogic::ThresholdAbove,
            threshold: Some(25.0),
            unit: Some("°C".into()),
            description: "Will Munich's daily max temperature exceed the threshold?".into(),
            default_script: Some("weather_binary.rhai".into()),
            builtin: true,
        },
        MarketSeries {
            id: "london_temp_daily".into(),
            label: "London Max Temp (daily)".into(),
            slug_prefix: "highest-temperature-in-london".into(),
            data_source: DataSource::OpenMeteo,
            symbol: "london".into(),
            cadence: "1d".into(),
            resolution_logic: ResolutionLogic::ThresholdAbove,
            threshold: Some(20.0),
            unit: Some("°C".into()),
            description: "Will London's daily max temperature exceed the threshold?".into(),
            default_script: Some("weather_binary.rhai".into()),
            builtin: true,
        },
        MarketSeries {
            id: "nyc_temp_daily".into(),
            label: "NYC Max Temp (daily)".into(),
            slug_prefix: "highest-temperature-in-new-york".into(),
            data_source: DataSource::OpenMeteo,
            symbol: "nyc".into(),
            cadence: "1d".into(),
            resolution_logic: ResolutionLogic::ThresholdAbove,
            threshold: Some(30.0),
            unit: Some("°C".into()),
            description: "Will NYC's daily max temperature exceed the threshold?".into(),
            default_script: Some("weather_binary.rhai".into()),
            builtin: true,
        },
    ]
}

/// Geo coordinates for Open-Meteo weather cities.
pub fn city_coords(city: &str) -> Option<(f64, f64)> {
    match city.to_lowercase().as_str() {
        "munich" | "münchen"  => Some((48.1351, 11.5820)),
        "london"              => Some((51.5074, -0.1278)),
        "nyc" | "new_york" | "new-york" => Some((40.7128, -74.0060)),
        "paris"               => Some((48.8566, 2.3522)),
        "berlin"              => Some((52.5200, 13.4050)),
        "madrid"              => Some((40.4168, -3.7038)),
        "tokyo"               => Some((35.6762, 139.6503)),
        "chicago"             => Some((41.8781, -87.6298)),
        "sydney"              => Some((33.8688, 151.2093)),
        _                     => None,
    }
}
