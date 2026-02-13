use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: String,
    pub api_prefix: String,
    pub api_key: String,
    pub private_key_path: Option<PathBuf>,
    pub private_key_pem: Option<String>,
    pub dry_run: bool,
    pub btc_only: bool,
    pub crypto_only: bool,
    pub crypto_assets: Vec<String>,
    pub event_ticker_prefixes: Vec<String>,
    pub event_series_tickers: Vec<String>,
    pub min_close_ts: Option<i64>,
    pub interval_regex: String,
    pub combined_max_price: f64,
    pub order_count: i64,
    pub check_exchange: bool,
    pub time_in_force: String,
    pub discover_btc_events: bool,
    pub discover_series: bool,
    pub series_category: String,
    pub series_frequency: String,
    pub events_limit: i64,
    pub log_decisions: bool,
    pub enable_cex_lag_scan: bool,
    pub cex_lag_threshold: f64,
    pub cex_lag_require_signal: bool,
    pub cex_lag_min_sources: usize,
}

impl Config {
    pub fn from_env() -> Self {
        let raw_base_url = env::var("KALSHI_BASE_URL")
            .unwrap_or_else(|_| "https://api.elections.kalshi.com/trade-api/v2".to_string());
        let (base_url, api_prefix) = split_base_url(&raw_base_url);
        let api_key = env::var("KALSHI_API_KEY").unwrap_or_default();
        let private_key_path = env::var("KALSHI_PRIVATE_KEY_PATH").ok().map(PathBuf::from);
        let private_key_pem = env::var("KALSHI_PRIVATE_KEY_PEM")
            .ok()
            .or_else(|| env::var("KALSHI_API_SECRET").ok());
        let dry_run = env::var("DRY_RUN")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);
        let btc_only = env::var("BTC_ONLY")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let crypto_only = env::var("CRYPTO_ONLY")
            .map(|v| v != "false")
            .unwrap_or(true);
        let crypto_assets = env::var("CRYPTO_ASSETS")
            .unwrap_or_else(|_| "BTC,ETH,SOL".to_string())
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        let event_ticker_prefixes = env::var("EVENT_TICKER_PREFIXES")
            .unwrap_or_else(|_| "KXBTC15M,KXETH15M,KXSOL15M".to_string())
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        let event_series_tickers = env::var("EVENT_SERIES_TICKERS")
            .unwrap_or_else(|_| "KXBTC15M,KXETH15M,KXSOL15M".to_string())
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        let min_close_ts = env::var("MIN_CLOSE_TS").ok().and_then(|v| v.parse().ok());
        let interval_regex = env::var("INTERVAL_REGEX")
            .unwrap_or_else(|_| "(?i)\\b15\\s?m(in(ute)?s?)?\\b".to_string());
        let combined_max_price = env::var("COMBINED_MAX_PRICE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        let order_count = env::var("ORDER_COUNT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let check_exchange = env::var("CHECK_EXCHANGE")
            .map(|v| v != "false")
            .unwrap_or(true);
        let time_in_force =
            env::var("TIME_IN_FORCE").unwrap_or_else(|_| "fill_or_kill".to_string());
        let discover_btc_events = env::var("DISCOVER_BTC_EVENTS")
            .map(|v| v != "false")
            .unwrap_or(true);
        let discover_series = env::var("DISCOVER_SERIES")
            .map(|v| v != "false")
            .unwrap_or(false);
        let series_category = env::var("SERIES_CATEGORY").unwrap_or_else(|_| "crypto".to_string());
        let series_frequency =
            env::var("SERIES_FREQUENCY").unwrap_or_else(|_| "fifteen_min".to_string());
        let events_limit = env::var("EVENTS_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);
        let log_decisions = env::var("LOG_DECISIONS")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let enable_cex_lag_scan = env::var("ENABLE_CEX_LAG_SCAN")
            .map(|v| v != "false")
            .unwrap_or(true);
        let cex_lag_threshold = env::var("CEX_LAG_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.08);
        let cex_lag_require_signal = env::var("CEX_LAG_REQUIRE_SIGNAL")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let cex_lag_min_sources = env::var("CEX_LAG_MIN_SOURCES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2usize);

        Self {
            base_url,
            api_prefix,
            api_key,
            private_key_path,
            private_key_pem,
            dry_run,
            btc_only,
            crypto_only,
            crypto_assets,
            event_ticker_prefixes,
            event_series_tickers,
            min_close_ts,
            interval_regex,
            combined_max_price,
            order_count,
            check_exchange,
            time_in_force,
            discover_btc_events,
            discover_series,
            series_category,
            series_frequency,
            events_limit,
            log_decisions,
            enable_cex_lag_scan,
            cex_lag_threshold,
            cex_lag_require_signal,
            cex_lag_min_sources,
        }
    }
}

fn split_base_url(raw: &str) -> (String, String) {
    if let Some(idx) = raw.find("/trade-api/") {
        let (base, suffix) = raw.split_at(idx);
        let prefix = suffix.to_string();
        return (base.trim_end_matches('/').to_string(), prefix);
    }

    (
        raw.trim_end_matches('/').to_string(),
        "/trade-api/v2".to_string(),
    )
}
