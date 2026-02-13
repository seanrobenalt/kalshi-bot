use chrono::{DateTime, Utc};
use regex::Regex;
use std::collections::HashMap;

use crate::cex::AssetReference;
use crate::config::Config;
use crate::log_err;
use crate::models::{Market, OrderRequest, Side};

#[derive(Debug, Clone)]
pub struct Decision {
    pub market: Market,
    pub orders: Vec<OrderRequest>,
    pub reason: String,
}

pub fn pick_opportunities(
    config: &Config,
    now: DateTime<Utc>,
    markets: Vec<Market>,
    cex_refs: Option<&HashMap<String, AssetReference>>,
) -> Vec<Decision> {
    let mut decisions = Vec::new();
    let interval_re = Regex::new(&config.interval_regex)
        .unwrap_or_else(|_| Regex::new("(?i)\\b15\\s?m(in(ute)?)?\\b").unwrap());

    for market in markets {
        let seconds_to_close = (market.close_time - now).num_seconds();
        if config.log_decisions {
            log_err!(
                "Evaluating market {} | title='{}' subtitle='{}' event='{}' close={} ttl={}s yes={} no={}",
                market.ticker,
                market.title,
                market.subtitle.clone().unwrap_or_default(),
                market.event_ticker.clone().unwrap_or_default(),
                market.close_time,
                seconds_to_close,
                market.yes_ask_dollars.clone().unwrap_or_default(),
                market.no_ask_dollars.clone().unwrap_or_default()
            );
        }
        if config.btc_only && !market.is_btc_related() {
            if config.log_decisions {
                log_err!("  -> skip: not BTC-related");
            }
            continue;
        }
        if config.crypto_only && !market.is_crypto_related(&config.crypto_assets) {
            if config.log_decisions {
                log_err!("  -> skip: not crypto-related");
            }
            continue;
        }
        if !matches_interval(&market, &interval_re) {
            if config.log_decisions {
                log_err!("  -> skip: not 15-minute interval");
            }
            continue;
        }

        if seconds_to_close < 0 {
            if config.log_decisions {
                log_err!("  -> skip: market already closed ({}s)", seconds_to_close);
            }
            continue;
        }

        let yes_price = market
            .yes_ask_dollars
            .as_ref()
            .and_then(|v| v.parse::<f64>().ok());
        let no_price = market
            .no_ask_dollars
            .as_ref()
            .and_then(|v| v.parse::<f64>().ok());

        let (yes_price, no_price) = match (yes_price, no_price) {
            (Some(yes), Some(no)) => (yes, no),
            _ => {
                if config.log_decisions {
                    log_err!("  -> skip: missing or invalid YES/NO ask");
                }
                continue;
            }
        };

        let combined = yes_price + no_price;
        let yes_in_band = (0.90..=0.97).contains(&yes_price);
        let no_in_band = (0.90..=0.97).contains(&no_price);
        let price_in_band = yes_in_band || no_in_band;
        let qualifies_fast = seconds_to_close < 60 && price_in_band;
        let lag_signal = compute_cex_lag_signal(config, &market, yes_price, cex_refs);

        if config.cex_lag_require_signal && config.enable_cex_lag_scan {
            let has_signal = lag_signal
                .as_ref()
                .map(|signal| signal.abs_lag >= config.cex_lag_threshold)
                .unwrap_or(false);
            if !has_signal {
                if config.log_decisions {
                    log_err!(
                        "  -> skip: cex lag signal below threshold {:.4}",
                        config.cex_lag_threshold
                    );
                }
                continue;
            }
        }

        if !qualifies_fast && combined >= config.combined_max_price {
            if config.log_decisions {
                log_err!(
                    "  -> skip: combined {:.4} >= threshold {:.4}",
                    combined,
                    config.combined_max_price
                );
            }
            continue;
        }

        let orders = if qualifies_fast {
            let mut fast_orders = Vec::new();
            if yes_in_band {
                fast_orders.push(OrderRequest {
                    ticker: market.ticker.clone(),
                    side: Side::Yes,
                    price_dollars: yes_price,
                    quantity: config.order_count,
                });
            }
            if no_in_band {
                fast_orders.push(OrderRequest {
                    ticker: market.ticker.clone(),
                    side: Side::No,
                    price_dollars: no_price,
                    quantity: config.order_count,
                });
            }
            fast_orders
        } else {
            vec![
                OrderRequest {
                    ticker: market.ticker.clone(),
                    side: Side::Yes,
                    price_dollars: yes_price,
                    quantity: config.order_count,
                },
                OrderRequest {
                    ticker: market.ticker.clone(),
                    side: Side::No,
                    price_dollars: no_price,
                    quantity: config.order_count,
                },
            ]
        };

        let mut reason = if qualifies_fast {
            format!(
                "TTL {}s with YES {:.4} / NO {:.4} in 0.90-0.97 band (single-side)",
                seconds_to_close, yes_price, no_price
            )
        } else {
            format!(
                "YES {:.4} + NO {:.4} = {:.4} within {}s of close",
                yes_price, no_price, combined, seconds_to_close
            )
        };
        if let Some(signal) = &lag_signal {
            reason.push_str(&format!(
                " | CEX lag {} {} strike {:.2}: model_yes {:.3} vs kalshi_yes {:.3} (lag {:.3})",
                signal.asset,
                signal.direction,
                signal.strike,
                signal.model_yes_prob,
                signal.kalshi_yes_prob,
                signal.lag
            ));
        }

        decisions.push(Decision {
            market,
            orders,
            reason,
        });

        if config.log_decisions {
            if qualifies_fast {
                log_err!(
                    "  -> QUALIFY: ttl {}s with YES {:.4} / NO {:.4} in 0.90-0.97 band",
                    seconds_to_close,
                    yes_price,
                    no_price
                );
            } else {
                log_err!(
                    "  -> QUALIFY: combined {:.4} < {:.4}, seconds_to_close={}",
                    combined,
                    config.combined_max_price,
                    seconds_to_close
                );
            }
            if let Some(signal) = &lag_signal {
                log_err!(
                    "  -> CEX LAG: {} {} strike {:.2} ref {:.2} model_yes {:.3} kalshi_yes {:.3} lag {:.3}",
                    signal.asset,
                    signal.direction,
                    signal.strike,
                    signal.reference_price,
                    signal.model_yes_prob,
                    signal.kalshi_yes_prob,
                    signal.lag
                );
            }
        }
    }

    decisions
}

fn matches_interval(market: &Market, interval_re: &Regex) -> bool {
    if interval_re.is_match(&market.title) {
        return true;
    }
    if let Some(subtitle) = &market.subtitle {
        if interval_re.is_match(subtitle) {
            return true;
        }
    }
    if let Some(event_ticker) = &market.event_ticker {
        if interval_re.is_match(event_ticker) {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone)]
struct LagSignal {
    asset: String,
    direction: Direction,
    strike: f64,
    reference_price: f64,
    model_yes_prob: f64,
    kalshi_yes_prob: f64,
    lag: f64,
    abs_lag: f64,
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Above,
    Below,
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Direction::Above => write!(f, "above"),
            Direction::Below => write!(f, "below"),
        }
    }
}

fn compute_cex_lag_signal(
    config: &Config,
    market: &Market,
    kalshi_yes_prob: f64,
    cex_refs: Option<&HashMap<String, AssetReference>>,
) -> Option<LagSignal> {
    if !config.enable_cex_lag_scan {
        return None;
    }
    let refs = cex_refs?;
    let asset = market.primary_asset()?;
    if asset != "BTC" && asset != "ETH" {
        return None;
    }
    let reference = refs.get(asset)?;
    if reference.quotes.len() < config.cex_lag_min_sources {
        return None;
    }

    let direction = parse_direction(&market.title, market.subtitle.as_deref())?;
    let strike = parse_strike(&market.title, market.subtitle.as_deref())?;
    if strike <= 0.0 {
        return None;
    }

    let model_yes_prob = model_yes_probability(asset, reference.reference_price, strike, direction);
    let lag = model_yes_prob - kalshi_yes_prob;
    Some(LagSignal {
        asset: asset.to_string(),
        direction,
        strike,
        reference_price: reference.reference_price,
        model_yes_prob,
        kalshi_yes_prob,
        lag,
        abs_lag: lag.abs(),
    })
}

fn parse_direction(title: &str, subtitle: Option<&str>) -> Option<Direction> {
    let mut text = title.to_lowercase();
    if let Some(sub) = subtitle {
        text.push(' ');
        text.push_str(&sub.to_lowercase());
    }

    let above_terms = [
        " at or above ",
        " above ",
        " over ",
        " greater than ",
        " higher than ",
    ];
    let below_terms = [
        " at or below ",
        " below ",
        " under ",
        " less than ",
        " lower than ",
    ];

    if above_terms.iter().any(|term| text.contains(term)) {
        return Some(Direction::Above);
    }
    if below_terms.iter().any(|term| text.contains(term)) {
        return Some(Direction::Below);
    }
    None
}

fn parse_strike(title: &str, subtitle: Option<&str>) -> Option<f64> {
    let number_re = Regex::new(r"\$?\d{1,3}(?:,\d{3})*(?:\.\d+)?").ok()?;
    let mut candidates = Vec::new();

    for cap in number_re.find_iter(title) {
        if let Ok(value) = parse_number_fragment(cap.as_str()) {
            candidates.push(value);
        }
    }

    if let Some(sub) = subtitle {
        for cap in number_re.find_iter(sub) {
            if let Ok(value) = parse_number_fragment(cap.as_str()) {
                candidates.push(value);
            }
        }
    }

    candidates
        .into_iter()
        .filter(|v| *v >= 100.0)
        .max_by(|a, b| a.total_cmp(b))
}

fn parse_number_fragment(fragment: &str) -> Result<f64, std::num::ParseFloatError> {
    fragment.replace(['$', ','], "").parse::<f64>()
}

fn model_yes_probability(
    asset: &str,
    reference_price: f64,
    strike: f64,
    direction: Direction,
) -> f64 {
    let scale_bps = match asset {
        "BTC" => 45.0,
        "ETH" => 65.0,
        _ => 55.0,
    };
    let dist_bps = ((reference_price - strike) / strike) * 10_000.0;
    let above_prob = sigmoid(dist_bps / scale_bps);
    match direction {
        Direction::Above => above_prob,
        Direction::Below => 1.0 - above_prob,
    }
}

fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        let z = (-x).exp();
        1.0 / (1.0 + z)
    } else {
        let z = x.exp();
        z / (1.0 + z)
    }
}
