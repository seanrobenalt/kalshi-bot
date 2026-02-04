# Kalshi 15m Crypto Arbitrage Bot (Rust)

Minimal Rust bot that:

- Trades **crypto 15-minute** markets (BTC/ETH/SOL by default)
- Filters for **15-minute intervals** (regex-based)
- Buys both YES and NO when the **combined ask price < $1**
- Additional fast-close buy: if **time to close < 60s** and **either YES or NO is in the 0.90–0.97 band**, it buys even if the combined price is >= `COMBINED_MAX_PRICE`

This is live-ready (Kalshi Trade API v2) and supports dry runs against live markets.

## Live setup

Set these env vars:

- `KALSHI_API_KEY` (key id)
- `KALSHI_PRIVATE_KEY_PATH` (path to the RSA private key PEM)
- Optional: `KALSHI_BASE_URL` (default: `https://api.elections.kalshi.com/trade-api/v2`)
- `DRY_RUN=false`

Run:

```bash
cd kalshi-15m-bot
DRY_RUN=false cargo run
```

## Dry-run

```bash
cd kalshi-15m-bot
DRY_RUN=true cargo run
```

If you have credentials set, `DRY_RUN` will still fetch live markets and simulate orders.
Without credentials, the mock client runs and no markets are loaded.

## Config

- `KALSHI_BASE_URL` (default: `https://api.elections.kalshi.com/trade-api/v2`)
- `KALSHI_API_KEY`
- `KALSHI_PRIVATE_KEY_PATH` or `KALSHI_PRIVATE_KEY_PEM` (or `KALSHI_API_SECRET` as a PEM string)
- `DRY_RUN` (default: `true`)
- `BTC_ONLY` (default: `false`) set to true to restrict to BTC-only titles/tickers
- `CRYPTO_ONLY` (default: `true`) restricts to titles/tickers containing `CRYPTO_ASSETS`
- `CRYPTO_ASSETS` (default: `BTC,ETH,SOL`) comma-separated list used by `CRYPTO_ONLY`
- `EVENT_TICKER_PREFIXES` (default: `KXBTC15M,KXETH15M,KXSOL15M`) prioritized event ticker prefixes to narrow `/events` discovery
- `EVENT_SERIES_TICKERS` (default: `KXBTC15M,KXETH15M,KXSOL15M`) series tickers used to query `/events?series_ticker=...`
- `MIN_CLOSE_TS` (optional) filters events to those with close times >= this unix timestamp (seconds)
- `INTERVAL_REGEX` (default: `(?i)\b15\s?m(in(ute)?)?\b`)
- `COMBINED_MAX_PRICE` (default: `1.0`)
- `ORDER_COUNT` (default: `1`)
- `CHECK_EXCHANGE` (default: `true`)
- `TIME_IN_FORCE` (default: `fill_or_kill`)
- `DISCOVER_BTC_EVENTS` (default: `true`) uses `/events` with nested markets and filters by `CRYPTO_ASSETS`
- `DISCOVER_SERIES` (default: `false`) uses `/series` + `/markets` to find markets by category/frequency
- `SERIES_CATEGORY` (default: `crypto`)
- `SERIES_FREQUENCY` (default: `fifteen_min`)
- `EVENTS_LIMIT` (default: `200`) page size for events discovery
- `LOG_DECISIONS` (default: `false`) prints per-market qualification metrics and skip reasons
- `SLACK_WEBHOOK_URL` (optional) posts a formatted run summary to Slack

## Notes

- This bot only places **buy** orders.
- Ensure your `COMBINED_MAX_PRICE` leaves room for fees.
- Fast-close band logic: if a market is within 60 seconds of close and either side is priced in the 0.90–0.97 band, it will place the buys regardless of `COMBINED_MAX_PRICE`.
- Start with `DRY_RUN=true` to validate selection logic.

## Simple Deployment (GitHub Actions)

Cheapest and simplest way to run this on a schedule is GitHub Actions. It runs on GitHub’s hosted runners (no server to manage), and the minimum schedule is every 5 minutes.

### 1) Create repo secrets

In GitHub: `Settings → Secrets and variables → Actions → New repository secret`.

Add:

- `KALSHI_API_KEY`
- `KALSHI_PRIVATE_KEY_PEM` (paste the full PEM contents)
- `SLACK_WEBHOOK_URL` (optional; for Slack output)

### 2) Add the workflow

Example `.github/workflows/kalshi-bot.yml`:

```yaml
name: Kalshi 15m Bot

on:
  schedule:
    - cron: "*/5 * * * *"
  workflow_dispatch: {}

jobs:
  run:
    runs-on: ubuntu-latest
    env:
      KALSHI_API_KEY: ${{ secrets.KALSHI_API_KEY }}
      KALSHI_PRIVATE_KEY_PEM: ${{ secrets.KALSHI_PRIVATE_KEY_PEM }}
      SLACK_WEBHOOK_URL: ${{ secrets.SLACK_WEBHOOK_URL }}
      DRY_RUN: "false"
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo run --release
```

### 3) Adjust run settings (optional)

Edit `.github/workflows/kalshi-bot.yml` to change:

- `schedule` (cron)
- `DRY_RUN` (set to `"true"` to dry-run)
- `COMBINED_MAX_PRICE`
- `EVENT_SERIES_TICKERS` / `CRYPTO_ASSETS`

### 4) Trigger a manual run

Go to `Actions → Kalshi 15m Bot → Run workflow`.

## Faster Cadence (Cheapest VPS)

If you need **every N seconds**, GitHub Actions cannot do that (its minimum scheduled interval is 5 minutes). Use a small VPS and run the bot in a loop.

### 1) Provision a cheap VPS

Any $5/mo instance works (Ubuntu recommended).

### 2) Install dependencies

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libssl-dev curl
curl https://sh.rustup.rs -sSf | sh -s -- -y
source $HOME/.cargo/env
```

### 3) Build the binary

```bash
git clone <your-repo-url>
cd kalshi-15m-bot
cargo build --release
```

### 4) Create env file

```bash
cat > .env << 'EOF'
KALSHI_API_KEY=...
KALSHI_PRIVATE_KEY_PEM=...
SLACK_WEBHOOK_URL=...
DRY_RUN=false
LOG_DECISIONS=true
EVENT_SERIES_TICKERS=KXBTC15M,KXETH15M,KXSOL15M
CRYPTO_ASSETS=BTC,ETH,SOL
EOF
```

### 5) Run every 20 seconds with systemd

Create a systemd service that runs the bot in a loop:

```bash
sudo tee /etc/systemd/system/kalshi-15m-bot.service > /dev/null << 'EOF'
[Unit]
Description=Kalshi 15m bot (20s loop)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/home/ubuntu/kalshi-15m-bot
EnvironmentFile=/home/ubuntu/kalshi-15m-bot/.env
ExecStart=/bin/bash -lc 'while true; do ./target/release/kalshi-15m-bot; sleep 20; done'
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable kalshi-15m-bot
sudo systemctl start kalshi-15m-bot
```

Tail logs:

```bash
journalctl -u kalshi-15m-bot -f
```

### Low-Memory VPS (Build Locally, Deploy Binary)

If you use a 512 MB VPS, build locally to avoid memory issues, then copy the binary up.

Local build:

```bash
cargo build --release
```

Copy to server:

```bash
scp ./target/release/kalshi-15m-bot ubuntu@<server-ip>:/home/ubuntu/kalshi-15m-bot/
```

On the server, update the systemd `ExecStart` to point at the copied binary:

```
ExecStart=/bin/bash -lc 'while true; do ./kalshi-15m-bot; sleep 20; done'
```
