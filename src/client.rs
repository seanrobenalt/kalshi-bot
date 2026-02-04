use std::fs;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rand::thread_rng;
use reqwest::blocking::{Client as HttpClient, Response};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::pss::SigningKey;
use rsa::RsaPrivateKey;
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use serde::Deserialize;
use sha2::Sha256;
use base64::Engine;

use crate::config::Config;
use crate::log_err;
use crate::models::{Market, OrderRequest, OrderResponse, Side};

pub trait KalshiClient {
    fn now(&self) -> DateTime<Utc>;
    fn list_markets(&self) -> Result<Vec<Market>>;
    fn place_order(&self, order: &OrderRequest) -> Result<OrderResponse>;
    fn exchange_status(&self) -> Result<Option<ExchangeStatus>>;
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExchangeStatus {
    pub exchange_active: bool,
    pub trading_active: bool,
    pub exchange_estimated_resume_time: Option<DateTime<Utc>>,
}

pub struct MockClient {
    _config: Config,
}

impl MockClient {
    pub fn new(config: Config) -> Self {
        Self { _config: config }
    }
}

#[derive(Debug, Deserialize)]
struct MarketsResponse {
    markets: Vec<Market>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default, rename = "next_cursor")]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventsResponse {
    events: Vec<Event>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default, rename = "next_cursor")]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SeriesResponse {
    #[serde(default)]
    series: Option<Vec<Series>>,
    #[serde(default, rename = "market_series")]
    market_series: Option<Vec<Series>>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default, rename = "next_cursor")]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Event {
    event_ticker: String,
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    markets: Vec<Market>,
}

#[derive(Debug, Deserialize)]
struct Series {
    ticker: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    frequency: Option<String>,
}

impl KalshiClient for MockClient {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn list_markets(&self) -> Result<Vec<Market>> {
        Ok(Vec::new())
    }

    fn place_order(&self, order: &OrderRequest) -> Result<OrderResponse> {
        let order_id = format!("dry-{}-{:?}-{}", order.ticker, order.side, order.price_dollars);
        Ok(OrderResponse { order_id })
    }

    fn exchange_status(&self) -> Result<Option<ExchangeStatus>> {
        Ok(None)
    }
}

pub struct LiveClient {
    config: Config,
    http: HttpClient,
    private_key: RsaPrivateKey,
}

impl LiveClient {
    pub fn new(config: Config) -> Result<Self> {
        let private_key = load_private_key(&config)?;
        Ok(Self {
            config,
            http: HttpClient::new(),
            private_key,
        })
    }

    fn sign_headers(&self, method: &str, full_path: &str) -> Result<HeaderMap> {
        let timestamp = Utc::now().timestamp_millis().to_string();
        let path_without_query = full_path.split('?').next().unwrap_or(full_path);
        let message = format!("{}{}{}", timestamp, method, path_without_query);
        let mut rng = thread_rng();
        let signing_key = SigningKey::<Sha256>::new(self.private_key.clone());
        let signature = signing_key.sign_with_rng(&mut rng, message.as_bytes());
        let signature_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_vec());

        let mut headers = HeaderMap::new();
        headers.insert("KALSHI-ACCESS-KEY", HeaderValue::from_str(&self.config.api_key)?);
        headers.insert("KALSHI-ACCESS-TIMESTAMP", HeaderValue::from_str(&timestamp)?);
        headers.insert("KALSHI-ACCESS-SIGNATURE", HeaderValue::from_str(&signature_b64)?);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(headers)
    }

    fn send_signed(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> Result<Response> {
        let full_path = format!("{}{}", self.config.api_prefix, path);
        let url = format!("{}{}", self.config.base_url, full_path);
        let headers = self.sign_headers(method, &full_path)?;
        let request = match method {
            "GET" => self.http.get(&url).headers(headers),
            "POST" => {
                let mut req = self.http.post(&url).headers(headers);
                if let Some(body) = body {
                    req = req.json(&body);
                }
                req
            }
            _ => return Err(anyhow!("Unsupported method: {}", method)),
        };

        let response = request.send().context("request failed")?;
        Ok(response)
    }
}

impl KalshiClient for LiveClient {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn list_markets(&self) -> Result<Vec<Market>> {
        if self.config.discover_btc_events {
            return self.list_event_markets();
        }
        if self.config.discover_series {
            return self.list_series_markets();
        }
        self.list_all_markets()
    }

    fn place_order(&self, order: &OrderRequest) -> Result<OrderResponse> {
        let side = match order.side {
            Side::Yes => "yes",
            Side::No => "no",
        };

        let mut body = serde_json::json!({
            "ticker": order.ticker,
            "side": side,
            "action": "buy",
            "count": order.quantity,
            "type": "limit",
            "time_in_force": self.config.time_in_force.clone(),
        });

        if side == "yes" {
            body["yes_price_dollars"] = serde_json::Value::String(format!("{:.4}", order.price_dollars));
        } else {
            body["no_price_dollars"] = serde_json::Value::String(format!("{:.4}", order.price_dollars));
        }

        let response = self.send_signed("POST", "/portfolio/orders", Some(body))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            return Err(anyhow!("create order failed: {} - {}", status, text));
        }

        #[derive(Debug, Deserialize)]
        struct CreateOrderResponse {
            order: Option<CreateOrder>,
            order_id: Option<String>,
        }

        #[derive(Debug, Deserialize)]
        struct CreateOrder {
            order_id: String,
        }

        let payload: CreateOrderResponse = response.json().context("failed to parse create order response")?;
        if let Some(order) = payload.order {
            return Ok(OrderResponse { order_id: order.order_id });
        }
        if let Some(order_id) = payload.order_id {
            return Ok(OrderResponse { order_id });
        }

        Err(anyhow!("missing order_id in response"))
    }

    fn exchange_status(&self) -> Result<Option<ExchangeStatus>> {
        log_err!("Checking exchange status...");
        let response = self.send_signed("GET", "/exchange/status", None)?;
        if !response.status().is_success() {
            return Err(anyhow!("exchange status failed: {}", response.status()));
        }
        let status: ExchangeStatus = response.json().context("failed to parse exchange status")?;
        Ok(Some(status))
    }
}

impl LiveClient {
    fn list_series_markets(&self) -> Result<Vec<Market>> {
        let mut markets = Vec::new();
        let category = self.config.series_category.trim();
        let frequency = canonical_frequency(self.config.series_frequency.trim());

        let series = self.list_series(category)?;
        if series.is_empty() {
            log_err!(
                "Series list empty for category='{}'. Falling back to full market list.",
                category
            );
            return self.list_all_markets();
        }
        let series_count = series.len();
        let mut matched = Vec::new();
        for entry in series {
            if entry.frequency.is_none() {
                continue;
            }
            if !frequency.is_empty()
                && canonical_frequency(entry.frequency.as_ref().unwrap()) != frequency
            {
                continue;
            }
            matched.push(entry);
        }

        if matched.is_empty() {
            log_err!(
                "No series matched category='{}' frequency='{}' ({} total series).",
                category,
                frequency,
                series_count
            );
            log_err!("Falling back to full market list.");
            return self.list_all_markets();
        }

        log_err!(
            "Matched {} series for category='{}' frequency='{}'",
            matched.len(),
            category,
            frequency
        );

        for entry in matched {
            let series_title = entry.title.clone().unwrap_or_else(|| "untitled".to_string());
            log_err!("Series {} [{}] {}", entry.ticker, entry.category.clone().unwrap_or_default(), series_title);
            markets.extend(self.list_markets_for_series(&entry.ticker)?);
        }

        log_err!("Fetched {} markets via series discovery.", markets.len());
        Ok(markets)
    }

    fn list_series(&self, category: &str) -> Result<Vec<Series>> {
        let mut series = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page = 0;

        loop {
            page += 1;
            let mut path = String::from("/series?limit=1000");
            if !category.is_empty() {
                path.push_str("&category=");
                path.push_str(&simple_query_escape(category));
            }
            if let Some(ref cursor_val) = cursor {
                path.push_str("&cursor=");
                path.push_str(cursor_val);
            }

            log_err!(
                "Fetching series list page {} for category='{}' (cursor={})",
                page,
                category,
                cursor.as_deref().unwrap_or("none")
            );
            let response = self.send_signed("GET", &path, None)?;
            let status = response.status();
            let body = response.text().unwrap_or_default();
            if !status.is_success() {
                return Err(anyhow!("get series failed: {} - {}", status, body));
            }

            let payload: SeriesResponse =
                serde_json::from_str(&body).context("failed to parse series response")?;
            let page_series = match payload.series.or(payload.market_series) {
                Some(items) => items,
                None => {
                    log_err!("Series response missing array; treating as empty page.");
                    Vec::new()
                }
            };
            series.extend(page_series);
            cursor = payload.cursor.or(payload.next_cursor);
            if cursor.as_deref().unwrap_or("").is_empty() {
                break;
            }
        }

        Ok(series)
    }

    fn list_markets_for_series(&self, series_ticker: &str) -> Result<Vec<Market>> {
        let mut markets = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page = 0;

        loop {
            page += 1;
            let mut path = format!(
                "/markets?status=open&series_ticker={}&limit=1000",
                simple_query_escape(series_ticker)
            );
            if let Some(ref cursor_val) = cursor {
                path.push_str("&cursor=");
                path.push_str(cursor_val);
            }

            log_err!(
                "Fetching markets for series {} page {} (cursor={})",
                series_ticker,
                page,
                cursor.as_deref().unwrap_or("none")
            );
            let response = self.send_signed("GET", &path, None)?;
            if !response.status().is_success() {
                return Err(anyhow!("get markets failed: {}", response.status()));
            }

            let payload: MarketsResponse = response.json().context("failed to parse markets response")?;
            markets.extend(payload.markets);
            cursor = payload.cursor.or(payload.next_cursor);
            if cursor.as_deref().unwrap_or("").is_empty() {
                break;
            }
        }

        Ok(markets)
    }

    fn list_all_markets(&self) -> Result<Vec<Market>> {
        let mut markets = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page = 0;

        loop {
            page += 1;
            let mut path = String::from("/markets?status=open&limit=1000");
            if let Some(ref cursor_val) = cursor {
                path.push_str("&cursor=");
                path.push_str(cursor_val);
            }

            log_err!("Fetching markets page {} (cursor={})", page, cursor.as_deref().unwrap_or("none"));
            let response = self.send_signed("GET", &path, None)?;
            if !response.status().is_success() {
                return Err(anyhow!("get markets failed: {}", response.status()));
            }

            let payload: MarketsResponse = response.json().context("failed to parse markets response")?;
            markets.extend(payload.markets);
            cursor = payload.cursor.or(payload.next_cursor);
            if cursor.as_deref().unwrap_or("").is_empty() {
                break;
            }
        }

        log_err!("Fetched {} markets total.", markets.len());
        Ok(markets)
    }

    fn list_event_markets(&self) -> Result<Vec<Market>> {
        let mut markets = Vec::new();
        let series_list = if self.config.event_series_tickers.is_empty() {
            vec![String::new()]
        } else {
            self.config.event_series_tickers.clone()
        };

        for series_ticker in series_list {
            let mut cursor: Option<String> = None;
            let mut page = 0;
            loop {
                page += 1;
                let mut path = format!(
                    "/events?status=open&with_nested_markets=true&limit={}",
                    self.config.events_limit
                );
                if !series_ticker.is_empty() {
                    path.push_str("&series_ticker=");
                    path.push_str(&simple_query_escape(&series_ticker));
                }
                if let Some(min_close_ts) = self.config.min_close_ts {
                    path.push_str("&min_close_ts=");
                    path.push_str(&min_close_ts.to_string());
                }
                if let Some(ref cursor_val) = cursor {
                    path.push_str("&cursor=");
                    path.push_str(cursor_val);
                }

                if series_ticker.is_empty() {
                    log_err!("Fetching events page {} (cursor={})", page, cursor.as_deref().unwrap_or("none"));
                } else {
                    log_err!(
                        "Fetching events for series {} page {} (cursor={})",
                        series_ticker,
                        page,
                        cursor.as_deref().unwrap_or("none")
                    );
                }
                let response = self.send_signed("GET", &path, None)?;
                if !response.status().is_success() {
                    return Err(anyhow!("get events failed: {}", response.status()));
                }

                let payload: EventsResponse = response.json().context("failed to parse events response")?;
                for event in payload.events {
                    if is_target_event(&event.event_ticker, &self.config.event_ticker_prefixes)
                        || is_crypto_text(&event.title, &self.config.crypto_assets)
                        || event
                            .subtitle
                            .as_ref()
                            .map(|s| is_crypto_text(s, &self.config.crypto_assets))
                            .unwrap_or(false)
                        || event
                            .category
                            .as_ref()
                            .map(|s| is_crypto_text(s, &self.config.crypto_assets))
                            .unwrap_or(false)
                        || is_crypto_text(&event.event_ticker, &self.config.crypto_assets)
                    {
                        log_err!(
                            "Crypto event: {} [{}] {}",
                            event.event_ticker,
                            event.category.clone().unwrap_or_else(|| "uncategorized".to_string()),
                            event.title
                        );
                        markets.extend(event.markets);
                    }
                }

                cursor = payload.cursor.or(payload.next_cursor);
                if cursor.as_deref().unwrap_or("").is_empty() {
                    break;
                }
            }
        }

        log_err!("Fetched {} markets via events.", markets.len());
        Ok(markets)
    }
}

fn is_crypto_text(value: &str, assets: &[String]) -> bool {
    let v = value.to_lowercase();
    for asset in assets {
        if asset.is_empty() {
            continue;
        }
        if v.contains(asset) {
            return true;
        }
        if asset == "btc" && v.contains("bitcoin") {
            return true;
        }
        if asset == "eth" && v.contains("ethereum") {
            return true;
        }
        if asset == "sol" && v.contains("solana") {
            return true;
        }
    }
    false
}

fn is_target_event(event_ticker: &str, prefixes: &[String]) -> bool {
    if prefixes.is_empty() {
        return false;
    }
    let ticker = event_ticker.to_uppercase();
    for prefix in prefixes {
        if !prefix.is_empty() && ticker.starts_with(prefix) {
            return true;
        }
    }
    false
}

fn canonical_frequency(value: &str) -> String {
    let v = value.trim().to_lowercase();
    if v.is_empty() {
        return String::new();
    }
    let v = v.replace('-', "_").replace(' ', "_");
    match v.as_str() {
        "15m" | "15min" | "15mins" | "15_min" | "15_mins" | "15minutes" | "15_minutes" => "fifteen_min".to_string(),
        "fifteenmin" | "fifteen_mins" => "fifteen_min".to_string(),
        _ => v,
    }
}

fn simple_query_escape(value: &str) -> String {
    value.replace(' ', "%20")
}

fn load_private_key(config: &Config) -> Result<RsaPrivateKey> {
    if let Some(pem) = &config.private_key_pem {
        let normalized = normalize_pem(pem);
        if let Ok(key) = RsaPrivateKey::from_pkcs8_pem(&normalized) {
            return Ok(key);
        }
        let key = RsaPrivateKey::from_pkcs1_pem(&normalized)
            .context("failed to parse KALSHI_PRIVATE_KEY_PEM (PKCS#1 or PKCS#8)")?;
        return Ok(key);
    }

    if let Some(path) = &config.private_key_path {
        let pem = fs::read_to_string(path).with_context(|| format!("failed to read private key at {:?}", path))?;
        let normalized = normalize_pem(&pem);
        if let Ok(key) = RsaPrivateKey::from_pkcs8_pem(&normalized) {
            return Ok(key);
        }
        let key = RsaPrivateKey::from_pkcs1_pem(&normalized)
            .context("failed to parse KALSHI_PRIVATE_KEY_PATH (PKCS#1 or PKCS#8)")?;
        return Ok(key);
    }

    Err(anyhow!("missing KALSHI_PRIVATE_KEY_PEM or KALSHI_PRIVATE_KEY_PATH"))
}

fn normalize_pem(raw: &str) -> String {
    let pem = raw.trim().replace("\\n", "\n").replace('\r', "");
    if let Some(extracted) = extract_pem_block(&pem, "RSA PRIVATE KEY") {
        return sanitize_pem_block(&extracted, "RSA PRIVATE KEY");
    }
    if let Some(extracted) = extract_pem_block(&pem, "PRIVATE KEY") {
        return sanitize_pem_block(&extracted, "PRIVATE KEY");
    }
    pem
}

fn sanitize_pem_block(pem_block: &str, label: &str) -> String {
    let begin = format!("-----BEGIN {}-----", label);
    let end = format!("-----END {}-----", label);
    let mut base64_data = String::new();
    let mut in_body = false;

    for line in pem_block.lines() {
        if line.contains(&begin) {
            in_body = true;
            continue;
        }
        if line.contains(&end) {
            in_body = false;
            continue;
        }
        if in_body {
            for ch in line.chars() {
                if is_base64_char(ch) {
                    base64_data.push(ch);
                }
            }
        }
    }

    let mut rebuilt = String::new();
    rebuilt.push_str(&begin);
    rebuilt.push('\n');
    for chunk in base64_data.as_bytes().chunks(64) {
        if let Ok(s) = std::str::from_utf8(chunk) {
            rebuilt.push_str(s);
            rebuilt.push('\n');
        }
    }
    rebuilt.push_str(&end);
    rebuilt
}

fn is_base64_char(ch: char) -> bool {
    matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' | '=')
}

fn extract_pem_block(raw: &str, label: &str) -> Option<String> {
    let begin = format!("-----BEGIN {}-----", label);
    let end = format!("-----END {}-----", label);
    let start = raw.find(&begin)?;
    let stop = raw.find(&end)?;
    if stop < start {
        return None;
    }
    let mut block = raw[start..stop + end.len()].to_string();
    block = block.replace('\r', "");
    if !block.contains("\n") {
        block = block.replace(&begin, &format!("{}\n", begin));
        block = block.replace(&end, &format!("\n{}", end));
    }
    Some(block)
}
