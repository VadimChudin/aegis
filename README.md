# AEGIS

Desktop terminal for automated gold (XAU) trading. **v0.1.0 is the skeleton:** a window with the gold chart from the broker you log in to, timeframe switching, and four empty strategy slots. It does not place orders.

## What works

- **Brokers:** Binance (USDⓈ-M futures `XAUUSDT`), Bybit (USDT perpetual `XAUUSDT`) and RoboForex (MetaTrader 5 `XAUUSD`).
- **One broker at a time.** The chart is empty until you log in, and it only ever shows data from the connected broker. Connecting another broker logs the previous one out first.
- **Timeframes:** 1m, 5m, 15m, 1h, 4h, 1d. 500 bars of history, then the last bar updates every second.
- **Strategies:** Breakout, Bounce, Liquidity Sweep, DATA. Registered and listed, but every slot is a stub that never signals.
- Credentials stay in memory for the session only. Keys are checked with a signed read-only request (Binance balance, Bybit wallet balance, MT5 login).

## Connecting

| Broker | What you need |
|---|---|
| Binance | API key + secret. Read-only is enough. Binance lists gold as a TradFi perpetual; it is not available in every region. |
| Bybit | API key + secret of a Unified Trading Account. Read-only is enough. |
| RoboForex | **Windows only.** The RoboForex MT5 terminal installed, Python 3, and `pip install MetaTrader5`. Enter the MT5 login, password and server (for example `RoboForex-ECN`); the terminal path and Python path are optional. |

MT5 has no network API, so AEGIS starts a small Python process (`python/aegis_lab/bridges/mt5_bridge.py`) that talks to the local terminal. MT5 bars carry the broker's server time; the bridge converts them to UTC.

## Layout

```
crates/aegis-core/     Rust: market types, Binance / Bybit / MT5 connectors, strategy registry
app/src-tauri/         Rust: Tauri 2 window, commands, live feed
app/ui/                window UI (plain HTML/JS, TradingView Lightweight Charts)
python/aegis_lab/      Python: MT5 bridge now; research, backtests and GA later
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
AEGIS_BINANCE_URL=http://127.0.0.1:8765/binance AEGIS_BYBIT_URL=http://127.0.0.1:8765/bybit cargo run -p aegis-app
```

## Release

Bump the version in `Cargo.toml`, `app/src-tauri/tauri.conf.json` and `app/package.json`, add a `CHANGELOG` entry, then push a tag:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

GitHub Actions builds `AEGIS_<ver>_x64-setup.exe` (Windows) and `AEGIS_<ver>_universal.dmg` (macOS, Apple Silicon + Intel, ad-hoc signed; first launch needs *System Settings → Privacy & Security → Open Anyway*).

Charts: [TradingView Lightweight Charts™](https://www.tradingview.com/lightweight-charts/), Apache-2.0.
