use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use serde::Serialize;

#[derive(Serialize)]
struct SlackPayload<'a> {
    text: &'a str,
}

pub fn post_run_log(webhook_url: &str, header: &str, log: Option<&str>) -> Result<()> {
    if webhook_url.trim().is_empty() {
        return Ok(());
    }

    let mut text = String::new();
    text.push_str(header);
    if let Some(body) = log {
        text.push_str("\n\n```");
        text.push('\n');
        text.push_str(body);
        text.push_str("\n```");
    }

    let payload = SlackPayload { text: &text };
    let client = Client::new();
    let response = client.post(webhook_url).json(&payload).send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(anyhow!("slack webhook failed: {} - {}", status, body));
    }
    Ok(())
}
