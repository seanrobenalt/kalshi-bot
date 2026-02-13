use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub ticker: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub event_ticker: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    pub close_time: DateTime<Utc>,
    #[serde(default)]
    pub yes_ask_dollars: Option<String>,
    #[serde(default)]
    pub no_ask_dollars: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Side {
    Yes,
    No,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequest {
    pub ticker: String,
    pub side: Side,
    pub price_dollars: f64,
    pub quantity: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResponse {
    pub order_id: String,
}

impl Market {
    pub fn primary_asset(&self) -> Option<&'static str> {
        let haystack = self.haystack();
        if haystack.contains("btc") || haystack.contains("bitcoin") {
            return Some("BTC");
        }
        if haystack.contains("eth") || haystack.contains("ethereum") {
            return Some("ETH");
        }
        None
    }

    pub fn is_btc_related(&self) -> bool {
        let haystack = self.haystack();
        haystack.contains("btc") || haystack.contains("bitcoin")
    }

    pub fn is_crypto_related(&self, assets: &[String]) -> bool {
        let haystack = self.haystack();
        if assets.is_empty() {
            return false;
        }
        for asset in assets {
            if asset.is_empty() {
                continue;
            }
            if haystack.contains(asset) {
                return true;
            }
            if asset == "btc" && haystack.contains("bitcoin") {
                return true;
            }
            if asset == "eth" && haystack.contains("ethereum") {
                return true;
            }
            if asset == "sol" && haystack.contains("solana") {
                return true;
            }
        }
        false
    }

    fn haystack(&self) -> String {
        let mut haystack = String::new();
        haystack.push_str(&self.title);
        if let Some(subtitle) = &self.subtitle {
            haystack.push(' ');
            haystack.push_str(subtitle);
        }
        if let Some(event_ticker) = &self.event_ticker {
            haystack.push(' ');
            haystack.push_str(event_ticker);
        }
        haystack.to_lowercase()
    }
}
