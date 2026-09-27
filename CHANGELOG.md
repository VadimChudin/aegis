# Changelog

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
