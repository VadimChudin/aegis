# AEGIS

Desktop terminal for automated gold (XAU) trading. **v0.5.0:** the gold chart from any connected broker, a Brokers panel with per-broker requirements and connect checks, saved encrypted credentials, three languages (English, Русский, Қазақша), and the **Bounce** strategy with sliders, a win-probability model, a backtest, a walk-forward genetic algorithm and statistical checks. It does not place orders. The Bounce strategy currently has **no edge** in backtests (see the research report).

## What works

- **Brokers:** Binance (USDⓈ-M futures `XAUUSDT`), Bybit (USDT perpetual `XAUUSDT`) and RoboForex (MetaTrader 5 `XAUUSD`). **All three can be connected at the same time.**
- **Chart source:** pick any connected broker in the header. The chart only ever shows that broker's data; it never mixes venues.
- **Timeframes:** 1m, 5m, 15m, 1h, 4h, 1d. 500 bars of history, then the last bar updates every second.
- **Brokers panel** (header icons, rail or the ⋯ menu): each broker has its icon, its own requirements, its form and a **connect checklist** in the style of Vespera:
  - Binance: reachable / region, clock, XAUUSDT listed, key accepted, key permissions (reading, futures, withdrawals, IP restriction), position mode, USDT balance.
  - Bybit: reachable / region, clock, XAUUSDT listed, key accepted, Unified Trading Account, permissions (read-only, contract trading, withdrawals), IP binding, equity.
  - RoboForex: Python, MetaTrader5 package, MT5 login, terminal connected, Algo Trading button, trading allowed, gold symbol and spread, balance.
- **Saved credentials:** stored in `settings.json` in the app config folder, every field encrypted (AES-256-GCM) with a key bound to this computer and user. Secrets are never sent back to the window. *Connect on start* reconnects saved brokers when AEGIS opens; *Forget* removes them.
- **Design and themes** from Vespera: Glass dark, Glass light, Glass blue.
- **Bounce strategy** (rail → Strategies → Bounce): every setting is a slider or toggle, including a min–max filter on each of 42 touch metrics; *Run backtest* downloads Binance XAUUSDT 5m and 1m history (public archive, no key) and shows the result; *Genetic algorithm* tunes the settings walk-forward; *Checks* tests whether a result is real; *Show on chart* draws entries with probability and exits with R. On the live Binance chart the expected entries are drawn with their probability.
- **1-second position engine** (Bounce → Position): entries and exits are simulated second by second on Binance aggTrades (downloaded once, about 1 GB, kept as 1-second candles). It adds an *absorption* entry (aggressive volume into the level while price holds, stop just behind the absorption extreme), breakeven, trailing stop, partial exit, a *flow exit* when the level is being eaten, a *flip* into the breakout when the bounce fails, a daily loss limit, and a money view with risk per trade and the leverage it needs. Research: [`docs/research/bounce.md`](docs/research/bounce.md).
- **Order-book densities (research):** Bybit XAUUSDT book (200 levels) and tape since 2026-03-09 are replayed to test the bounce off a large resting order with a cascade of limits. Gross edge exists, Bybit fees erase it. Research: [`docs/research/density.md`](docs/research/density.md).
- **Strategies:** Breakout, Liquidity Sweep and DATA are still stubs.

## Connecting

| Broker | What you need |
|---|---|
| Binance | System-generated API key + secret with *Enable Reading*. Futures trading and an IP restriction will be needed for trading later; never enable withdrawals. Gold is a TradFi perpetual and is not available in every region. |
| Bybit | System-generated API key + secret of a Unified Trading Account. Read-only is enough now. Bind the key to your IP or Bybit expires it after 90 days. |
| RoboForex | **Windows only.** The RoboForex MT5 terminal, Python 3.9+, and `pip install MetaTrader5`. MT5 login, password and server (for example `RoboForex-ECN`); terminal and Python paths are optional. MT4 accounts cannot connect. |

MT5 has no network API, so AEGIS starts a small Python process (`python/aegis_lab/bridges/mt5_bridge.py`) that talks to the local terminal. MT5 bars carry the broker's server time; the bridge converts them to UTC.

## Layout

```
crates/aegis-core/     Rust: market types, Binance / Bybit / MT5 connectors and checks, settings, strategy registry,
                       bounce/ (levels, touch metrics, probability model, backtest)
app/src-tauri/         Rust: Tauri 2 window, commands, live feed
app/ui/                window UI (plain HTML/JS, Vespera styles, TradingView Lightweight Charts)
python/aegis_lab/      Python: MT5 bridge, Binance and Bybit archive downloaders, research scripts
scripts/mock_venues.py local Binance/Bybit stand-in for development and tests
```

## Develop

Needs Rust (stable), Node 18+ and Python 3.9+. On Linux also the Tauri system libraries (`libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libxdo-dev`, `libssl-dev`).

```bash
cargo test --workspace                                  # Rust tests (starts the mock and the MT5 bridge)
(cd python && python -m unittest discover -s tests)     # Python tests
cargo run -p aegis-app                                  # open the window
```

Without access to the real APIs, run the mock and log in with key `test-key`, secret `test-secret`:

```bash
python scripts/mock_venues.py &
AEGIS_BINANCE_URL=http://127.0.0.1:8765/binance AEGIS_BINANCE_SPOT_URL=http://127.0.0.1:8765/binance \
AEGIS_BYBIT_URL=http://127.0.0.1:8765/bybit AEGIS_CONFIG_DIR=/tmp/aegis-dev cargo run -p aegis-app
```

## Release

Bump the version in `Cargo.toml`, `app/src-tauri/tauri.conf.json` and `app/package.json`, add a `CHANGELOG` entry, then push a tag:

```bash
git tag v0.5.0 && git push origin v0.5.0
```

GitHub Actions builds `AEGIS_<ver>_x64-setup.exe` (Windows) and `AEGIS_<ver>_universal.dmg` (macOS, Apple Silicon + Intel, ad-hoc signed; first launch needs *System Settings → Privacy & Security → Open Anyway*).

Charts: [TradingView Lightweight Charts™](https://www.tradingview.com/lightweight-charts/), Apache-2.0.
