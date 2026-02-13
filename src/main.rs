mod cex;
mod client;
mod config;
mod logger;
mod models;
mod slack;
mod strategy;

use anyhow::{anyhow, Result};
use cex::scan_btc_eth_references;
use client::{KalshiClient, LiveClient, MockClient};
use config::Config;
use logger::collected_log;
use logger::init_logger;

fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_logger();
    let config = Config::from_env();

    let result = run_with_config(&config);
    if let Err(err) = &result {
        log_err!("Error: {}", err);
        for (idx, cause) in err.chain().skip(1).enumerate() {
            log_err!("  {}: {}", idx, cause);
        }
    }

    if let Ok(webhook) = std::env::var("SLACK_WEBHOOK_URL") {
        let mode = if config.dry_run { "DRY_RUN" } else { "LIVE" };
        let now = chrono::Utc::now().to_rfc3339();
        let log = collected_log();
        let mut header = format!("*Kalshi 15m bot run* `{}` `{}`", mode, now);
        if let Some(opps) = extract_opportunities(&log) {
            header.push_str(&format!("\nOpportunities: {}", opps));
        }
        if log.contains("Error:") {
            header.push_str("\nResult: ERROR");
            let error_lines = extract_error_lines(&log, 6);
            if !error_lines.is_empty() {
                header.push_str("\n\n*Error Details*");
                for line in error_lines {
                    header.push_str("\n- ");
                    header.push_str(&line);
                }
            }
        } else {
            header.push_str("\nResult: OK");
        }
        let highlights = format_highlights(&log, 6);
        if !highlights.is_empty() {
            header.push_str("\n\n*Highlights*");
            header.push_str(&highlights);
        }
        if let Err(err) = slack::post_run_log(&webhook, &header, None) {
            log_err!("Slack post failed: {}", err);
        }
    }

    if let Err(err) = result {
        return Err(err);
    }
    Ok(())
}

fn run_with_config(config: &Config) -> Result<()> {
    if config.dry_run {
        log_out!("Running in DRY_RUN mode.");
        if !config.api_key.is_empty()
            && (config.private_key_pem.is_some() || config.private_key_path.is_some())
        {
            let client = LiveClient::new(config.clone())?;
            run(client, config)?;
            return Ok(());
        }

        let client = MockClient::new(config.clone());
        run(client, config)?;
        return Ok(());
    }

    if config.api_key.is_empty() {
        return Err(anyhow!("KALSHI_API_KEY not set"));
    }

    let client = LiveClient::new(config.clone())?;

    if config.check_exchange {
        if let Some(status) = client.exchange_status()? {
            if !status.exchange_active || !status.trading_active {
                let resume = status
                    .exchange_estimated_resume_time
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_else(|| "unknown".to_string());
                return Err(anyhow!(
                    "Exchange not active (exchange_active={}, trading_active={}). Resume: {}",
                    status.exchange_active,
                    status.trading_active,
                    resume
                ));
            }
        }
    }

    run(client, config)?;
    Ok(())
}

fn extract_opportunities(log: &str) -> Option<String> {
    for line in log.lines() {
        if let Some(rest) = line.strip_prefix("Opportunities found: ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn extract_error_lines(log: &str, max_lines: usize) -> Vec<String> {
    let lines: Vec<&str> = log.lines().collect();
    let mut start_idx: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.starts_with("Error:") {
            start_idx = Some(idx);
        }
    }

    let Some(start) = start_idx else {
        return Vec::new();
    };
    let mut collected = Vec::new();
    for line in lines.iter().skip(start) {
        if collected.is_empty() {
            collected.push((*line).to_string());
            continue;
        }

        if line.starts_with("  ") {
            collected.push((*line).to_string());
        } else {
            break;
        }
    }

    if collected.len() > max_lines {
        collected.truncate(max_lines);
        collected.push("...".to_string());
    }

    collected
}

fn format_highlights(log: &str, max_items: usize) -> String {
    let mut highlights = String::new();
    let mut pending_title: Option<String> = None;
    let mut pending_ticker: Option<String> = None;
    let mut pending_yes: Option<String> = None;
    let mut pending_no: Option<String> = None;
    let mut pending_ttl: Option<i64> = None;
    let mut count = 0;

    for line in log.lines() {
        if let Some(rest) = line.strip_prefix("Evaluating market ") {
            let mut parts = rest.split(" | ");
            if let Some(ticker) = parts.next() {
                pending_ticker = Some(ticker.trim().to_string());
            }
            if let Some(details) = parts.next() {
                if let Some(title) = extract_between(details, "title='", "'") {
                    pending_title = Some(title.to_string());
                }
                if let Some(ttl) = extract_value(details, "ttl=") {
                    let cleaned = ttl.trim_end_matches('s');
                    if let Ok(value) = cleaned.parse::<i64>() {
                        pending_ttl = Some(value);
                    }
                }
                if let Some(yes) = extract_value(details, "yes=") {
                    pending_yes = Some(yes.to_string());
                }
                if let Some(no) = extract_value(details, "no=") {
                    pending_no = Some(no.to_string());
                }
            }
            continue;
        }

        if let Some(skip) = line.strip_prefix("  -> skip: ") {
            if let (Some(title), Some(ticker)) = (pending_title.take(), pending_ticker.take()) {
                let reason = if let Some(combined) = extract_between(skip, "combined ", " >=") {
                    format!("combined {}", combined.trim())
                } else {
                    skip.trim().to_string()
                };
                let price_part = match (pending_yes.take(), pending_no.take()) {
                    (Some(yes), Some(no)) => format!("YES {} / NO {}", yes, no),
                    _ => String::new(),
                };
                let ttl_part = pending_ttl.take().map(format_ttl);
                let mut parts = Vec::new();
                if !price_part.is_empty() {
                    parts.push(price_part);
                }
                if let Some(ttl) = ttl_part {
                    parts.push(ttl);
                }
                let info = if parts.is_empty() {
                    String::new()
                } else {
                    parts.join(" — ")
                };
                if info.is_empty() {
                    highlights.push_str(&format!("\n- *{}* ({}) — *{}*", title, ticker, reason));
                } else {
                    highlights.push_str(&format!(
                        "\n- *{}* ({}) — {} — *{}*",
                        title, ticker, info, reason
                    ));
                }
                count += 1;
                if count >= max_items {
                    break;
                }
            }
            continue;
        }

        if let Some(ok) = line.strip_prefix("  -> QUALIFY: ") {
            if let (Some(title), Some(ticker)) = (pending_title.take(), pending_ticker.take()) {
                let reason = ok.trim();
                let price_part = match (pending_yes.take(), pending_no.take()) {
                    (Some(yes), Some(no)) => format!("YES {} / NO {}", yes, no),
                    _ => String::new(),
                };
                let ttl_part = pending_ttl.take().map(format_ttl);
                let mut parts = Vec::new();
                if !price_part.is_empty() {
                    parts.push(price_part);
                }
                if let Some(ttl) = ttl_part {
                    parts.push(ttl);
                }
                let info = if parts.is_empty() {
                    String::new()
                } else {
                    parts.join(" — ")
                };
                if info.is_empty() {
                    highlights.push_str(&format!("\n- *{}* ({}) — *{}*", title, ticker, reason));
                } else {
                    highlights.push_str(&format!(
                        "\n- *{}* ({}) — {} — *{}*",
                        title, ticker, info, reason
                    ));
                }
                count += 1;
                if count >= max_items {
                    break;
                }
            }
        }
    }

    highlights
}

fn extract_between<'a>(value: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let start_idx = value.find(start)? + start.len();
    let rest = &value[start_idx..];
    let end_idx = rest.find(end)?;
    Some(&rest[..end_idx])
}

fn extract_value<'a>(value: &'a str, label: &str) -> Option<&'a str> {
    let start_idx = value.find(label)? + label.len();
    let rest = &value[start_idx..];
    rest.split_whitespace().next()
}

fn format_ttl(seconds: i64) -> String {
    let mut value = seconds;
    if value < 0 {
        value = 0;
    }
    let minutes = value / 60;
    let secs = value % 60;
    format!("TTL {}m{:02}s", minutes, secs)
}

fn run<C: KalshiClient>(client: C, config: &Config) -> Result<()> {
    let now = client.now();
    let cex_refs = if config.enable_cex_lag_scan {
        match scan_btc_eth_references(config.cex_lag_min_sources) {
            Ok(map) => {
                for reference in map.values() {
                    let venues = reference
                        .quotes
                        .iter()
                        .map(|q| format!("{}:{:.2}", q.venue, q.mid))
                        .collect::<Vec<_>>()
                        .join(", ");
                    log_err!(
                        "CEX ref {} {:.2} from {} venues [{}]",
                        reference.asset,
                        reference.reference_price,
                        reference.quotes.len(),
                        venues
                    );
                }
                Some(map)
            }
            Err(err) => {
                log_err!("CEX price scan failed: {}", err);
                None
            }
        }
    } else {
        None
    };

    log_err!("Fetching markets...");
    let markets = client.list_markets()?;

    if markets.is_empty() {
        log_err!("No markets loaded.");
        return Ok(());
    }

    let decisions = strategy::pick_opportunities(config, now, markets, cex_refs.as_ref());
    log_err!("Opportunities found: {}", decisions.len());

    if decisions.is_empty() {
        log_out!("No qualifying opportunities.");
        return Ok(());
    }

    for decision in decisions {
        if config.dry_run {
            log_out!(
                "DRY_RUN: {} -> {} orders ({})",
                decision.market.ticker,
                decision.orders.len(),
                decision.reason
            );
            continue;
        }

        for order in decision.orders {
            let response = client.place_order(&order)?;
            log_out!("ORDER: {} -> {}", order.ticker, response.order_id);
        }
    }

    Ok(())
}
