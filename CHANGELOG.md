# Changelog

## 0.1.0 — 2026-09-26

First skeleton.

- Tauri 2 window with the gold chart (candles + volume), timeframes 1m–1d, live last bar.
- Broker login for Binance (`XAUUSDT` USDⓈ-M futures), Bybit (`XAUUSDT` USDT perpetual) and RoboForex (MT5 `XAUUSD` through a Python bridge). The chart shows data only from the connected broker; keys are verified with a signed read-only request and kept in memory only.
- Strategy slots: Breakout, Bounce, Liquidity Sweep, DATA — stubs that never signal.
- No order placement.
