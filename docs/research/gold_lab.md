# Gold component library ("2048"): public gold systems, tested and merged

Goal: collect gold systems published on trading platforms, re-implement each as a component
(signal → position), keep only what survives, and merge survivors. Nothing is copied: rules are
re-implemented from the public descriptions.

```bash
python -m aegis_lab.research.gold_lab       # H1 components, 2004-2025, and pair merges
python -m aegis_lab.research.gold_sessions  # 1m session effects 2008-2025, Power of 3 from 2008
```

Data: XAUUSD CFD 1h 2004-06 … 2025-02 and 1m 2008 … 2025-02 (MetaTrader export on Hugging Face,
server = New York + 7 h). Binance XAUUSDT 1m 2025-12 … 2026-09 as an independent check.

## Test

- Position decided at a bar's close, held from the next bar; cost $0.40/oz (H1) or $0.25/oz (1m)
  per round trip.
- **Timing return** = position × (return − gold's average hourly drift in 2004-2014) − costs.
  Gold rose 10× since 2004, so a long-only rule looks good by being long; only timing counts.
- Survivor = timing t ≥ 2 in 2004-2014; 2015-2025 is never used for selection.

## Level 1: components (H1, 2004-2025)

| Component (source) | Exposure | Trades/yr | Net %/yr in / out | Timing %/yr in / out | Timing t in / out |
|---|---|---|---|---|---|
| Friday anomaly (TradingView, piirsalu) | 20% | 52 | +12.6 / +3.5 | +10.0 / +0.9 | +3.3 / +0.4 |
| Turn of month (classic) | 19% | 12 | +1.4 / +4.8 | -1.0 / +2.3 | -0.4 / +1.2 |
| TSMOM 20/60/120/250d long-only (trend.md) | 42% | 16 | +6.8 / +4.6 | +1.2 / -0.9 | +0.3 / -0.3 |
| TSMOM long/short | 58% | 24 | +6.6 / +2.4 | +3.0 / -0.7 | +0.6 / -0.2 |
| Turtle 55/20 on H4 | 66% | 20 | +3.5 / +1.9 | +1.9 / +0.3 | +0.4 / +0.1 |
| EMA 50/200 H1 (GitHub maker-tung, FvgGold filter) | 56% | 17 | +9.5 / +6.4 | +2.1 / -0.9 | +0.5 / -0.3 |
| MACD(16,26) > 0 on H4 (TradingView, Golden Edge Pro) | 55% | 22 | +12.7 / +7.6 | +5.3 / +0.4 | +1.2 / +0.1 |
| Swing structure + EMA200 + chop filter (TradingView, Golden Trident) | 33% | 9 | +9.1 / +2.7 | +4.7 / -1.5 | +1.4 / -0.6 |
| Connors RSI(2) daily | 11% | 7 | +2.4 / +0.5 | +0.9 / -1.0 | +0.4 / -0.6 |
| Long in the Asian session, hourly | 39% | 264 | -7.2 / -2.1 | -12.4 / -7.3 | -4.3 / -3.1 |
| Asia-range (London) breakout | 28% | 274 | -10.6 / -1.5 | -10.6 / -1.5 | -2.3 / -0.5 |
| Bollinger mean reversion H1 | 57% | 218 | -23.3 / -10.3 | -22.8 / -9.9 | -4.8 / -2.9 |

- Every trend rule makes money only because gold went up: its timing is ≈ 0. The single in-sample
  survivor (Friday) is dead after 2014 (t +0.4), so there is nothing to merge at H1.
- Daily macro filters on COMEX gold 2001-2026 (dollar and 10-year note trend, S&P regime,
  gold/silver ratio, 1-5-day reversal, vol-managed long; 22 rules) found nothing that holds in
  both 2001-2012 and 2013-2026 (exploratory script, not in the repository).
- Also read, not re-tested: FvgGold, SMC Gold (MQL5 Code Base), BAKOME ICT scalper — the same
  FVG / order block / sweep elements that `smc.md` found without edge; their published results
  are 6-month to 2-year backtests.

## Level 1b: where gold's return happens (1m, 2008-2025)

Mean return by New York hour: **nearly all of gold's drift is earned 18:00-02:00** (the Asian
session after the daily break), +2.1…+2.6 bp in the 18:00 hour alone in every period (t = 6.2
over 2004-2025). London and New York hours average ≈ 0. The hourly component above loses only
because it re-enters every hour boundary it crosses and pays $0.40 each time; a single trade per
night keeps the effect:

| Trade, net of $0.25/oz | 2008-14 | 2015-19 | 2020-25 |
|---|---|---|---|
| **Asia 18:05 → 02:00, Mon-Thu evenings** | +0.90 bp (t +0.7) | +1.94 bp (t +1.8) | +1.55 bp (t +1.4) |
| Asia 18:05 → 01:00 | +0.78 bp | +1.38 bp | +0.98 bp |
| Asia 18:05 → 03:00 | +0.63 bp | +0.39 bp | +2.61 bp |
| Asia 19:00 → 02:00 | +1.30 bp | +1.01 bp | +0.36 bp |
| Sunday evening 18:05 → 02:00 | −0.57 bp | −1.08 bp | −2.60 bp |

- Gross (before cost) t = 2.3 / 3.6 / 2.6: positive in every period, ≈ 200 trades a year, no swap
  (opened after the 17:00 rollover, closed before the next).
- Net it is thin: +2.8%/yr at 1× notional, Sharpe ≈ 0.5, max drawdown −24% at 10% vol. As gold's
  price rises the same $ cost is fewer bp: at $4 000, $0.25 is 0.6 bp.
- **Binance XAUUSDT 2026 (independent source, 165 trades):** +3.6 bp gross, same sign but t = 0.5
  — 2026 volatility is too high for one year to confirm or reject it.
- Filters (60-day trend, previous New York session up / down) do not help consistently.

## Correction: Power of 3 does not hold before 2019

`smc_models.md` found +0.07 R net on 2019-2025. The same rule from 2008:

| Period | Net R ($0.25/oz) |
|---|---|
| 2008-2011 | −0.05 R (t −0.9) |
| 2012-2015 | −0.04 R (t −0.8) |
| 2016-2018 | −0.08 R (t −1.8) |
| 2019-2022 | +0.07 R (t +1.3) |
| 2023-2025 | +0.09 R (t +1.3) |

It works only in the 2019+ bull market, so it is not a structural edge. Paper-trade it only as a
regime bet.

## The board

| Tile | Level | Status |
|---|---|---|
| Asian session drift 18:05-02:00 | 1 | **alive**, thin (net Sharpe ≈ 0.5, needs ≤ $0.25/oz round trip) |
| Power of 3 08:30 | 1 | regime-only (2019+) |
| Friday anomaly | 1 | dead after 2014 |
| Trend, Turtle, EMA, MACD, Trident, RSI(2), macro filters | 1 | no timing edge on gold |
| Breakout / mean reversion H1, SMC, bounce, density | 1 | lose after costs |
| Asia + Power of 3 (disjoint hours, correlation +0.03) | 2 | Sharpe +0.29 over 2008-2025, carried by 2020+ only |

Next tiles worth adding: the Asian drift at lower cost (limit entry at 18:05, real RoboForex
spread from MT5), and effects tied to scheduled events (FOMC, CPI) on the full 2008 history.
