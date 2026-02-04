use chrono::{DateTime, Utc};
use regex::Regex;

use crate::config::Config;
use crate::log_err;
use crate::models::{Market, OrderRequest, Side};

#[derive(Debug, Clone)]
pub struct Decision {
    pub market: Market,
    pub orders: Vec<OrderRequest>,
    pub reason: String,
}

pub fn pick_opportunities(config: &Config, now: DateTime<Utc>, markets: Vec<Market>) -> Vec<Decision> {
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

        let yes_price = market.yes_ask_dollars.as_ref().and_then(|v| v.parse::<f64>().ok());
        let no_price = market.no_ask_dollars.as_ref().and_then(|v| v.parse::<f64>().ok());

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
        let price_in_band = (0.90..=0.97).contains(&yes_price) || (0.90..=0.97).contains(&no_price);
        let qualifies_fast = seconds_to_close < 60 && price_in_band;

        if !qualifies_fast && combined >= config.combined_max_price {
            if config.log_decisions {
                log_err!(
                    "  -> skip: combined {:.4} >= threshold {:.4}",
                    combined, config.combined_max_price
                );
            }
            continue;
        }

        let orders = vec![
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
        ];

        let reason = if qualifies_fast {
            format!(
                "TTL {}s with YES {:.4} / NO {:.4} in 0.90-0.97 band",
                seconds_to_close, yes_price, no_price
            )
        } else {
            format!(
                "YES {:.4} + NO {:.4} = {:.4} within {}s of close",
                yes_price, no_price, combined, seconds_to_close
            )
        };

        decisions.push(Decision {
            market,
            orders,
            reason,
        });

        if config.log_decisions {
            if qualifies_fast {
                log_err!(
                    "  -> QUALIFY: ttl {}s with YES {:.4} / NO {:.4} in 0.90-0.97 band",
                    seconds_to_close, yes_price, no_price
                );
            } else {
                log_err!(
                    "  -> QUALIFY: combined {:.4} < {:.4}, seconds_to_close={}",
                    combined, config.combined_max_price, seconds_to_close
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
