# RoboForex AI Observer — stage 2, v0.7.0-beta.2

## Scope

This is autonomous **observation and simulation**, restricted to a connected RoboForex
MT5 session whose selected symbol is exactly `XAUUSD`. It does not call order submission,
modify real positions, train model weights or consult OpenRouter. Earlier AI Paper and
backtest features remain available separately. The new observer uses the managed local
Qwen server on `127.0.0.1:11435`, not the legacy AI Paper endpoint.

## Start

1. On Windows, install the RoboForex MT5 terminal, Python and its `MetaTrader5` package.
   Connect the account through Brokers. The existing connect checklist reports prerequisites.
2. Open Local AI, approve installation, install Qwen and press Start server. Ping model
   verifies actual inference. Automatic GPU driver installation is not provided.
3. Open **AI Observer** or one of the four AI strategy chips. Choose enabled strategies,
   edit their prompts, select timeframes, configure simulation costs and save settings.
4. Press Start observation. Quotes, frame summaries, decisions, rejection reasons and the
   simulated position appear automatically. Download journal exports the local JSONL log.

No account credentials enter model prompts or the observer journal. No paid cloud requests
are made by stage 2. The global Stop AI button stops new analysis in both AI modes; it does
not invent a closing price. An existing simulated position remains marked against quotes
while the app is open and MT5 is connected. On restart, positions/equity are restored but
observation is always stopped. Protective simulation cannot run while the app is closed.

## Data and cadence

- MT5 is sampled once per second, independently of model HTTP requests. The terminal bridge
  serializes its read requests; terminal delays may slow sampling and are not a real-time
  guarantee. Last quote timestamps, not polling time, determine freshness.
  MT5 Python timestamps are preserved as UTC, as specified in the
  [official tick-history reference](https://www.mql5.com/en/docs/python_metatrader5/mt5copyticksfrom_py).
- Tick history is bounded to 256 records; missing tick history remains empty. CFD tick
  volume is not exchange volume, aggressive buy/sell flow or a reconstructable full tape.
- DOM is included only when MT5 returns a valid book. Otherwise `book_note` explains the
  fallback to quotes/ticks/candle price levels. A price-level proxy is never labelled real
  resting liquidity. MT5 DOM is a sampled broker book, not an exchange delta stream.
- Closed-candle summaries refresh about every 15 seconds for M1/M5/M15/H1/H4/D1. The
  forming bar is excluded; prior support/resistance excludes the decision bar. Missing
  periods are shown as unavailable and cannot be cited as evidence.
- Material price or top-of-book changes notify the analysis loop, throttled to at most one
  notification per five seconds; frame refreshes also notify it. There is one in-flight
  inference and latest-state coalescing, not a queue of stale market requests. Enabled
  strategies are analysed round-robin. Identical strategy/frame/price requests are deduped.
- Qwen gets bounded summaries and actual available data, not screenshots or fabricated
  order flow. Inference has a 28-second HTTP timeout and a 30-second decision deadline.
  The result must be valid JSON and is rechecked against current quotes and price drift.
  Timeout, malformed output, disconnected MT5 or excessive spread means no new entry.

## Strategies and adherence

The four strategies are density bounce, SMC/structural reversal, breakout and liquidity
sweep. Default prompts are editable starter definitions, not a claim of profitable edge or
a complete implementation of every interpretation of SMC/ICT.

- **100:** selected timeframes only, all canonical rule checks with evidence, plus necessary
  mechanical conditions on closed price data. Density bounce uses actual book levels when
  usable, or explicitly identified historical price proxies if DOM is unavailable.
- **0–99:** the slider influences the model's preference for the supplied strategy versus
  discretion. Other available timeframes may be considered. It is not a probability or a
  promise that a model follows an arbitrary natural-language prompt with mathematical precision.
- **All values:** fresh matching source data, spread bounds, a valid loss-side stop and
  profit-side target, controlled price drift, one simulated position and risk limits apply.

At 100 the mechanical gate checks the built-in canonical strategy, not every possible rule
in an edited free-text prompt. Additional instructions are followed by the model; formal
enforcement of arbitrary prose is not claimed. Prompts are bounded to fit local inference.

## Simulation and journal

Risk is percent of simulated equity, at most 1%, with notional capped at 10× equity and one
position total. Units are ounces, not MT5 lots. Entry uses ask for long/bid for short; exits
use the opposite quote side. Commission is configurable **per side**, USD/oz, and slippage
is configurable per fill. Defaults are zero commission/slippage: set actual assumptions
before interpreting results. Spread is taken from actual quotes.

Stops and targets are marked from sampled quotes, not guaranteed live fills. Gaps can cause
losses larger than planned risk; snapshots do not prove queue priority, fill availability,
broker slippage or profitability. Full discretionary position management remains a later
stage; this stage tests a fixed stop/target plan, not live trading.

Config lives in `observer/config.json` under the application config directory. The append-only
`observer/decisions.jsonl` stores source snapshots, exact prompts/configs, model identity,
decisions (including wait/rejections), simulation states and closed outcomes. These can feed
future dataset preparation; they are not automatically accepted as training targets. Future
outcomes must remain outside model inputs for training pre-entry decisions.

Journal size is bounded to 32 MiB. Recording errors or corrupted recovery data prevent new
entries rather than silently overwriting evidence. Keep/export the log before it reaches
this limit. The journal is local and contains market data/prompts, not account secrets.

## Verification boundary

Real local Ollama/Qwen inference and native desktop interactions can be exercised on Linux.
MT5 collection is covered by fake-terminal integration and Python tests, including unavailable
DOM, stale quotes and disconnection. Those checks are **not** a live RoboForex account test;
the actual terminal, clock, DOM availability and GPU performance require Windows validation.
