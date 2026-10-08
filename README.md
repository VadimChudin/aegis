# AEGIS

**v0.7.0-beta.4 reliability update:** fixes Stop/Brokers overlap, repeated-close UI races,
stale/gapped strategy history and MT5 reply/filling/error handling. Same execution-preview
scope and demo acceptance requirements as beta.3; see the changelog for verified changes.

**v0.7.0-beta.3 execution preview:** one Settings → AI tab, JSON SPA for five strategies,
Paper/Money modes, OpenRouter/Fable5 agreement, MT5 execution and scoped market close controls,
dynamic account data and position lines. **Windows demo acceptance is required before client
or real-money use.** See [`docs/ai-execution-preview.md`](docs/ai-execution-preview.md).

**v0.7.0-beta.2:** RoboForex MT5/XAUUSD autonomous AI observer, four configurable strategy
prompts, 0–100 adherence, optional DOM fallback and automatic simulated decision/outcome
journaling. **No real orders or model-weight training.** Setup and exact limits:
[`docs/roboforex-observer.md`](docs/roboforex-observer.md).

Desktop terminal for automated gold (XAU) trading. **v0.6.0:** a separate dockable gold-density screener with continuous broker-specific recording, the gold chart from any connected broker, a Brokers panel with per-broker requirements and connect checks, saved encrypted credentials, three languages (English, Русский, Қазақша), and the **Bounce** strategy with sliders, a win-probability model, a backtest, a walk-forward genetic algorithm and statistical checks. It does not place orders. The Bounce strategy currently has **no edge** in backtests (see the research report).

## What works

**v0.7.0-beta.1 adds experimental AI Paper, not live trading.** The AI panel can
install a user-local Ollama/Qwen3-8B runtime on Linux x86-64, save an encrypted
OpenRouter key, run broker-specific numeric decisions, and manage simulated
positions within hard risk and API budgets. See [AI Paper setup and limits](docs/ai-paper.md).
Real-money execution, Windows/macOS automatic model installation and email
reports are not part of this beta. The existing strategy backtests remain separate.

- **Structural reversal (experimental):** independent slider/toggle panel on six-month Bybit trade archives, cached native downloads, custom presets and a red **Setup 1 + · +0.85R / 5 trades** retrospective preset. No live orders; five trades do not prove profitability. Rules and limitations: [`docs/structural-reversal.md`](docs/structural-reversal.md).
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

MT5 has no network API, so AEGIS starts a small Python process (`python/aegis_lab/bridges/mt5_bridge.py`) that talks to the local terminal. The MT5 Python API returns UTC timestamps; AEGIS preserves them without inferring a clock offset from stale quotes.

## Local AI

Rail → **Local AI** installs a private Ollama runtime and **Qwen3 8B Q4_K_M** (`qwen3:8b`).
Confirm the download, then press **Install locally**. AEGIS downloads the official Ollama
v0.35.1 archive over HTTPS, verifies its published SHA-256 checksum, extracts it, starts a
loopback-only server at `127.0.0.1:11435`, downloads the model with progress and tests inference.
Model downloads can be resumed by retrying. Existing model files are reused, including offline.
No administrator access, Python, cloud account or API key is needed for inference.

- Allow roughly 5.2 GB for model weights, plus several GB for Ollama libraries and temporary
  extraction space. Installation is offered on Windows x64/ARM64, Linux x64/ARM64 and macOS.
- The 4096-token context, one parallel request and non-thinking mode target a computer with
  8 GB VRAM and 16 GB RAM. They are defaults, not a guarantee of GPU compatibility or speed.
  The panel reports loaded CPU/GPU memory. Existing GPU drivers are required; AEGIS does not
  install drivers or promise a once-per-second inference cycle. Linux AMD ROCm-specific
  packages are not installed automatically by this first version.
- **Start server** reloads the model; **Ping model** runs actual inference, not just an HTTP ping.
  **Stop** and closing AEGIS stop only the server AEGIS started. A separately started server
  is never killed. Models remain on disk. The server is not automatically started on launch.
- Files live under the app's config folder, in `local-ai/` (`AEGIS_CONFIG_DIR` also applies):
  `runtime-v0.35.1/`, `models/`, `server.log` and `experience.json`.

### Learning from experience

Save a confirmed observation and a lesson, tagged with the strategy, optionally with a realised
result in R. AEGIS retains up to 2000 experiences, retrieves relevant lessons for subsequent
local questions and persists them across restarts. Each record can be deleted or exported in
JSONL dataset form. Model answers are **not** automatically accepted as lessons. Do not put API
keys or customer information into prompts or experience records.

This is **retrieval-based experiential memory**, not LoRA training or a change to Qwen's weights.
Dataset export does not train a model. Continuous weight fine-tuning, market observation,
OpenRouter consultation and order/position management are not implemented by this step. The
local question box cannot see current markets or place orders.

Next stages and the future training-data contract are described in
[`docs/local_ai_roadmap.md`](docs/local_ai_roadmap.md). Stage 2 is implemented for
RoboForex MT5/XAUUSD: autonomous observation, strategy prompts and an automatic
decision/outcome simulation journal. Dataset import and weight fine-tuning remain planned.

For an explicit runtime smoke test (not part of normal tests):

```bash
AEGIS_AI_DIR=/path/to/test/local-ai cargo run -p aegis-core --example local_ai -- install
AEGIS_AI_DIR=/path/to/test/local-ai cargo run -p aegis-core --example local_ai -- ping
```

Installer references: [Ollama Windows](https://docs.ollama.com/windows),
[Ollama Linux](https://docs.ollama.com/linux), [model pull API](https://docs.ollama.com/api/pull).

## Layout

```
crates/aegis-core/     Rust: market types, Binance / Bybit / MT5 connectors and checks, settings, strategy registry,
                       bounce/ (levels, touch metrics, probability model, backtest)
app/src-tauri/         Rust: Tauri 2 window, commands, live feed
app/ui/                window UI (plain HTML/JS, Vespera styles, TradingView Lightweight Charts)
python/aegis_lab/      Python: MT5 bridge, Binance and Bybit archive downloaders, research scripts
scripts/mock_venues.py local Binance/Bybit stand-in for development and tests
```

## Density screener

The terminal opens a separate, dockable gold order-book density window at startup. It monitors the
selected connected broker even while the window is hidden, and records sampled densities and
observations as rotating JSONL files in the application data folder (`densities/`). Binance and
Bybit provide exchange books; RoboForex requires the MT5 symbol to expose a DOM. Missing DOM is
shown as unavailable, never substituted with candle or another venue's data. Scores describe
observations, not calibrated probabilities; no live orders are placed.

Full Bounce setting inventory and screener rules: [Русское описание](docs/density-screener.md).

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

Linux x86-64 packages are built on Ubuntu 24.04 as `.deb` files. Install with
`sudo apt install ./AEGIS_0.6.1_amd64.deb` (use the filename of your download),
then launch `aegis`. A graphical desktop and WebKitGTK 4.1 are required; apt
resolves the declared runtime dependencies. Older distributions are not verified.
RoboForex's official MetaTrader5 Python integration remains Windows-only; the
Linux package does not make real MT5 connectivity available on Linux.

Linux package verification and limitations: [Linux release checks](docs/linux-release.md).

Bump the version in `Cargo.toml`, `app/src-tauri/tauri.conf.json` and `app/package.json`, add a `CHANGELOG` entry, then push a tag:

```bash
git tag v0.6.0 && git push origin v0.6.0
```

GitHub Actions builds `AEGIS_<ver>_x64-setup.exe` (Windows) and `AEGIS_<ver>_universal.dmg` (macOS, Apple Silicon + Intel, ad-hoc signed; first launch needs *System Settings → Privacy & Security → Open Anyway*).

Charts: [TradingView Lightweight Charts™](https://www.tradingview.com/lightweight-charts/), Apache-2.0.
