use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client as HttpClient;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct VenueQuote {
    pub venue: String,
    pub mid: f64,
}

#[derive(Debug, Clone)]
pub struct AssetReference {
    pub asset: String,
    pub reference_price: f64,
    pub quotes: Vec<VenueQuote>,
}

pub fn scan_btc_eth_references(min_sources: usize) -> Result<HashMap<String, AssetReference>> {
    let http = HttpClient::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .context("failed to build cex http client")?;

    let mut out = HashMap::new();

    if let Some(reference) = build_reference(
        "BTC",
        vec![
            fetch_coinbase_mid(&http, "BTC-USD"),
            fetch_kraken_mid(&http, "XBTUSD"),
            fetch_binance_mid(&http, "BTCUSDT"),
        ],
        min_sources,
    ) {
        out.insert("BTC".to_string(), reference);
    }

    if let Some(reference) = build_reference(
        "ETH",
        vec![
            fetch_coinbase_mid(&http, "ETH-USD"),
            fetch_kraken_mid(&http, "ETHUSD"),
            fetch_binance_mid(&http, "ETHUSDT"),
        ],
        min_sources,
    ) {
        out.insert("ETH".to_string(), reference);
    }

    Ok(out)
}

fn build_reference(
    asset: &str,
    results: Vec<Result<VenueQuote>>,
    min_sources: usize,
) -> Option<AssetReference> {
    let quotes = results
        .into_iter()
        .filter_map(Result::ok)
        .filter(|q| q.mid.is_finite() && q.mid > 0.0)
        .collect::<Vec<_>>();

    if quotes.len() < min_sources {
        return None;
    }

    let mut mids = quotes.iter().map(|q| q.mid).collect::<Vec<_>>();
    mids.sort_by(|a, b| a.total_cmp(b));
    let median = if mids.len() % 2 == 0 {
        let right = mids.len() / 2;
        let left = right - 1;
        (mids[left] + mids[right]) / 2.0
    } else {
        mids[mids.len() / 2]
    };

    Some(AssetReference {
        asset: asset.to_string(),
        reference_price: median,
        quotes,
    })
}

fn fetch_coinbase_mid(http: &HttpClient, product: &str) -> Result<VenueQuote> {
    #[derive(Deserialize)]
    struct CoinbaseTicker {
        bid: String,
        ask: String,
    }

    let url = format!(
        "https://api.exchange.coinbase.com/products/{}/ticker",
        product
    );
    let payload: CoinbaseTicker = http
        .get(url)
        .send()
        .context("coinbase request failed")?
        .error_for_status()
        .context("coinbase non-success status")?
        .json()
        .context("coinbase parse failed")?;

    let bid = payload.bid.parse::<f64>().context("coinbase invalid bid")?;
    let ask = payload.ask.parse::<f64>().context("coinbase invalid ask")?;
    if ask <= 0.0 || bid <= 0.0 {
        return Err(anyhow!("coinbase invalid bid/ask"));
    }

    Ok(VenueQuote {
        venue: "coinbase".to_string(),
        mid: (bid + ask) / 2.0,
    })
}

fn fetch_kraken_mid(http: &HttpClient, pair: &str) -> Result<VenueQuote> {
    let url = format!("https://api.kraken.com/0/public/Ticker?pair={}", pair);
    let payload: Value = http
        .get(url)
        .send()
        .context("kraken request failed")?
        .error_for_status()
        .context("kraken non-success status")?
        .json()
        .context("kraken parse failed")?;

    let result = payload
        .get("result")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("kraken response missing result object"))?;
    let first = result
        .values()
        .next()
        .ok_or_else(|| anyhow!("kraken response empty result"))?;

    let ask = first
        .get("a")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("kraken ask missing"))?
        .parse::<f64>()
        .context("kraken invalid ask")?;

    let bid = first
        .get("b")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("kraken bid missing"))?
        .parse::<f64>()
        .context("kraken invalid bid")?;

    if ask <= 0.0 || bid <= 0.0 {
        return Err(anyhow!("kraken invalid bid/ask"));
    }

    Ok(VenueQuote {
        venue: "kraken".to_string(),
        mid: (bid + ask) / 2.0,
    })
}

fn fetch_binance_mid(http: &HttpClient, symbol: &str) -> Result<VenueQuote> {
    #[derive(Deserialize)]
    struct BinanceBookTicker {
        #[serde(rename = "bidPrice")]
        bid_price: String,
        #[serde(rename = "askPrice")]
        ask_price: String,
    }

    let url = format!(
        "https://api.binance.com/api/v3/ticker/bookTicker?symbol={}",
        symbol
    );
    let payload: BinanceBookTicker = http
        .get(url)
        .send()
        .context("binance request failed")?
        .error_for_status()
        .context("binance non-success status")?
        .json()
        .context("binance parse failed")?;

    let bid = payload
        .bid_price
        .parse::<f64>()
        .context("binance invalid bid")?;
    let ask = payload
        .ask_price
        .parse::<f64>()
        .context("binance invalid ask")?;
    if ask <= 0.0 || bid <= 0.0 {
        return Err(anyhow!("binance invalid bid/ask"));
    }

    Ok(VenueQuote {
        venue: "binance".to_string(),
        mid: (bid + ask) / 2.0,
    })
}
