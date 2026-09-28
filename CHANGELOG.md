# Changelog

## 0.5.0 — 2026-09-28

- **1-second position engine** (Bounce → Position → *1-second engine*): entries and exits are simulated second by second on Binance aggTrades (downloaded once and kept as 1-second candles with buy and sell volume). It agrees with the bar engine when no management is set.
- **Position management:** breakeven after N R, trailing stop in ATR (from N R), partial exit (share and target), **flow exit** (aggressive volume against the position while price is through the level: the level is being eaten), **flip** into the breakout after the first stop or a flow exit.
- **Absorption entry** (Bounce → Absorption): wait at the level until aggressive volume hits it and price holds, then enter at market with the stop just behind the absorption extreme (small stop, higher leverage).
- **Risk:** daily loss limit in R; the backtest shows the result in money for a risk per trade (% of account) and the leverage each trade needs, cut to a max leverage.
- The genetic algorithm tunes the position settings when the 1-second engine is on; the sliders that have no effect with the current toggles are greyed out with the reason.
- **Research:** still **no edge**. The best rule (trailing 0.3 ATR on ATR ≥ $5) loses −0.09 R per trade and −0.02 R even with no costs; the flow exit halves the loss of a plain limit; the walk-forward GA loses out of sample (−0.215 R). Details: `docs/research/bounce.md`.
- Still no order placement.

## 0.4.0 — 2026-09-28

- **Correction: the v0.3.0 backtest results were wrong.** When a bar touched several levels, the scanner took the level nearest the bar's low/high, which uses where price turned (look-ahead). Fixed: the first level price reaches is the touched one. With the corrected engine the Bounce strategy has **no edge** on Binance XAUUSDT 5m: every touch loses about −0.15 R per trade after RoboForex costs, even before costs it loses. The "70%" presets are removed. Details: `docs/research/bounce.md`.
- **1m resolution:** the backtest downloads 1m candles and uses them to decide whether the stop or the target came first inside a 5m bar, and where a limit filled inside the touch bar.
- **Genetic algorithm** (Bounce → Genetic algorithm): walk-forward GA over every slider, toggle and metric filter (tournament, SBX, polynomial mutation, elitism, immigrants), train/validation split with early stopping and a robustness-weighted final pick, random search with the same budget as a baseline, convergence chart, "Apply to sliders".
- **Checks tab:** bootstrap intervals, Probabilistic and Deflated Sharpe, daily t-statistic, fill rules, random-level control, shuffled-metric permutation, PBO (CSCV), GA vs random search.
- **Languages:** English, Русский, Қазақша (Settings → Language).
- **More levels and metrics:** swings on 15m/4h/1d, previous month, equal highs/lows, fair value gaps; minutes to/since FOMC, CPI, NFP, PCE, PPI, GDP (calendar 2023–2026 from official sources), funding, spread; open interest, long/short and taker ratio from the Binance archive in the app.
- **Settings:** positions at once, spread for data without quotes, own presets (save/delete), a compare tab; sliders without effect under the current toggles are greyed out with the reason. Every slider was checked to change the backtest (80 of 83; the other three have no data in this history).
- Expected entries on RoboForex/Bybit charts are shifted by the current price difference to Binance.
- Still no order placement.

## 0.3.0 — 2026-09-27

- **Bounce strategy: settings and backtest.** The Bounce slot opens a panel with every setting as a slider or toggle: level kinds (5m/1h swings, previous day/week, sessions, round prices, previous day POC), sessions, direction, touch zone, stop, target, time exit, costs, and a min–max filter on each of 42 touch metrics (level, approach, market, candle, volume, tape, clusters, derivatives, time). Each metric shows its win rate by quintile from the research run.
- **Win-probability model.** A logistic model on 29 metrics, retrained every month on the previous 4 months, so every probability in the backtest is out of sample. *Min win probability* is a slider.
- **Run backtest** downloads Binance XAUUSDT 5m history since the listing from the public archive (no key, cached), and shows trades, win rate with its 95% lower bound, average and total R, profit factor, drawdown, equity curve, and results by month, session and level kind. Presets: *70% · Binance costs* and *70% · RoboForex costs*.
- **Chart:** *Show on chart* draws the backtest on 5m bars: entries with their probability, exits with R, stop and target of the selected trade. On the live Binance chart, expected entries (resting limit prices that pass the settings) are drawn with their probability and refreshed every 5 minutes.
- Research tooling: Binance archive downloader, 5m order-flow bars from aggTrades, research CLI (`examples/bounce.rs`) and report (`docs/research/bounce.md`).
- Still no order placement.

## 0.2.0 — 2026-09-27

- **Several brokers at once.** Binance, Bybit and RoboForex stay connected together. The header picks which one the chart shows; the chart never mixes venues. Header icons show each broker's state and open its settings.
- **Brokers panel** replaces the login dialog: each broker has its icon, its requirements (key type, permissions, IP restriction, UTA, MT5 terminal and Python package), its form and a connect checklist with a reason for every warning or failure. A failed step skips the steps that depend on it.
- **Saved credentials:** every field is stored encrypted (AES-256-GCM, key bound to this computer and user) in `settings.json`, with owner-only permissions. Secrets are never sent back to the window; an empty secret field keeps the stored one. *Connect on start* reconnects saved brokers; *Forget* removes them.
- The chart broker, timeframe and theme are remembered.
- **Vespera design:** the same glass styles and themes (Glass dark, light, blue), rail, dock with a log, and pickers.
- Still no order placement; the four strategy slots remain stubs.

## 0.1.0 — 2026-09-26

First skeleton.

- Tauri 2 window with the gold chart (candles + volume), timeframes 1m–1d, live last bar.
- Broker login for Binance (`XAUUSDT` USDⓈ-M futures), Bybit (`XAUUSDT` USDT perpetual) and RoboForex (MT5 `XAUUSD` through a Python bridge). The chart shows data only from the connected broker; keys are verified with a signed read-only request and kept in memory only.
- Strategy slots: Breakout, Bounce, Liquidity Sweep, DATA — stubs that never signal.
- No order placement.
