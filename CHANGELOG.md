# Changelog

## Unreleased

- **Research: gold parts bin and FOMC tiles** (`gold_parts.py`, `docs/research/gold_lab.md`). 17 parts of the tested systems (trend, chop, volatility, RSI(2), weekday, turn of month, FOMC days) as filters: none improves the Asian drift or Power of 3 in both 2008-14 and 2015-25. New live candidate: gold's pre-FOMC drift. Long 02:00 → 13:55 on the statement day earns +14.5 bp net per trade over 136 meetings (t 2.1; 2.7 against other days). It is positive in 4 of 5 periods and disappears on placebo dates.
- **Research: gold component library** (`gold_lab.py`, `gold_sessions.py`, `docs/research/gold_lab.md`). 12 public gold systems (TradingView, GitHub/MQL5 EAs, classic rules) re-implemented and tested on 2004-2025 with gold's drift removed: none has a timing edge. Nearly all of gold's return is earned 18:00-02:00 New York; a long 18:05 → 02:00 on weekday evenings is positive gross in every period since 2008 (t 2.3-3.6) but thin after costs. Power of 3 at 08:30 is negative before 2019.
- **Research: time-anchored ICT models** (Judas sweep of the Asia range, Silver Bullet 03/10/14, CRT on 1h/4h/daily, 6 624 settings) on six years of XAUUSD 1m and on Binance: no edge (walk-forward −0.48 R per trade). One lead: Power of 3 from the midnight open (08:30 NY, side of the midnight open, stop behind the day's extreme, exit 16:00) +0.12 R gross, +0.07 R net at $0.35/oz, t = 2.7, both halves positive. Report `docs/research/smc_models.md`.
- **Fix (Bounce):** changing *Limit after absorption*, its distance, or *Density exit* did not change the backtest after the first run: they were missing from the simulation cache key, so the GA and trade selection reused old results. The GA no longer tunes *Min win probability* outside the slider's range or when the model is off. *Trade-through* stays active with the limit after absorption; absorption sliders are greyed with close entry, where they do nothing. Every setting was re-checked on real data (1-second engine included): all change the backtest when their prerequisites are on, except spread and funding filters (no such data in Binance history).
- **Research: smart money (DATA):** order blocks, FVGs and liquidity sweeps on 1h/4h with limit, grid or 1m CHoCH entry: no setup is positive after fees; order block + CHoCH is positive before fees but within noise; a wide search (6 000 settings, confluences, CHoCH retest limit, liquidity target, trailing) and a walk-forward "Auto" test (−0.31 R per trade) confirm no edge. Every engine is now checked on a random walk. Report `docs/research/smc.md`.

## 0.5.0 — 2026-09-28

- **1-second position engine** (Bounce → Position → *1-second engine*): entries and exits are simulated second by second on Binance aggTrades (downloaded once and kept as 1-second candles with buy and sell volume). It agrees with the bar engine when no management is set.
- **Position management:** breakeven after N R, trailing stop in ATR (from N R), partial exit (share and target), **flow exit** (aggressive volume against the position while price is through the level: the level is being eaten), **flip** into the breakout after the first stop or a flow exit.
- **Absorption entry** (Bounce → Absorption): wait at the level until aggressive volume hits it and price holds, then enter at market with the stop just behind the absorption extreme (small stop, higher leverage).
- **Density exit** (leave before the stop when the absorbed volume behind it is being eaten, e.g. 70%) and **limit after absorption** (wait for the pull-back instead of a market entry).
- **Risk:** daily loss limit in R; the backtest shows the result in money for a risk per trade (% of account) and the leverage each trade needs, cut to a max leverage.
- The genetic algorithm tunes the position settings when the 1-second engine is on; the sliders that have no effect with the current toggles are greyed out with the reason.
- **Research:** still **no edge**. The best rule (trailing 0.3 ATR on ATR ≥ $5) loses −0.09 R per trade and −0.02 R even with no costs; the flow exit halves the loss of a plain limit; the walk-forward GA loses out of sample (−0.215 R; with the 70% win-rate target: 66.9% wins, 10 trades a day, −0.066 R). Details: `docs/research/bounce.md`.
- **Research: order-book densities** on Bybit XAUUSDT (200-level book + tape since 2026-03-09): large resting orders do hold price more often (bounce rate 14.5% below 7× the median level, 31.6% above 21×), and a cascade of limits with an exit when the density is eaten or pulled earns +$0.19/oz per trade before fees out of sample, but Bybit's taker fee on the exit turns it into −$0.59/oz. Scripts in `python/aegis_lab/research/density*.py`, report `docs/research/density.md`.
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
