# Time-series momentum and funding carry: edges outside intraday gold

Intraday gold rules (`bounce.md`, `density.md`, `smc*.md`, `anomalies`) carry at most
+0.02…0.05 R before costs, and costs are 0.05…0.2 R. This report tests two effects with a long
record in the literature and far fewer trades: time-series momentum (Moskowitz, Ooi & Pedersen
2012) and the perpetual funding premium.

```bash
pip install numpy pandas
python -m aegis_lab.research.trend           # backtest + robustness + today's weights
python -m aegis_lab.research.trend --today   # today's weights only
```

## Rule (fixed, not fitted)

- Daily close of 14 Binance spot coins (2017-08 … 2026-09, `data.binance.vision`).
- Score = mean of sign(close / close L days ago − 1) for L = 20, 60, 120, 250; long-only (score < 0 → 0).
- Weight = score × 10% / realised 60-day vol, capped at 2×; equal risk across coins.
- Decide at the close, trade the next day; 0.10% per unit of turnover. **Spot only**: a long
  perpetual pays funding (10-30%/yr in bull years).

## Result

| Variant | CAGR | Sharpe | Max DD | Sharpe halves |
|---|---|---|---|---|
| long-only, 20d only | 23.9% | 1.23 | -28.6% | 1.62 / 0.89 |
| long-only, 60d only | 23.5% | 1.16 | -28.9% | 1.37 / 0.96 |
| long-only, 120d only | 22.2% | 1.17 | -21.0% | 1.38 / 0.98 |
| long-only, 250d only | 17.2% | 0.95 | -26.0% | 1.28 / 0.57 |
| **long-only ensemble (the rule)** | 17.3% | 1.27 | -18.5% | 1.58 / 0.92 |
| long/short ensemble | 16.3% | 0.91 | -28.5% | 1.44 / 0.33 |
| trade 2 days later | 18.2% | 1.30 | -13.7% | 1.58 / 1.01 |
| costs x4 (0.40%) | 14.0% | 1.05 | -22.8% | 1.44 / 0.62 |
| BTC + ETH only | 9.1% | 1.20 | -9.2% | 1.55 / 0.81 |
| BTC only | 7.4% | 1.23 | -6.2% | 1.45 / 1.00 |
| control: BTC+ETH buy & hold, unlevered | 22.9% | 0.65 | -87.9% | 0.74 / 0.56 |

Shuffled weights (20 runs): Sharpe mean 0.14, best 0.47.

By year: 2018 −2.9%, 2019 +10.7%, 2020 +48.6%, 2021 +55.7%, 2022 −2.3%, 2023 +12.3%,
2024 +37.8%, 2025 +0.7%, 2026 (to Sep) +1.3%.

- Every lookback, both halves, a 2-day delay and 4× costs stay positive; shuffled weights do not.
- **BTC alone (no survivorship bias) has the same Sharpe** as the 14-coin basket, so the result
  does not come from picking coins that survived. The basket still has that bias (no LUNA, FTT).
- The edge is risk, not return: roughly buy & hold's return in bull years, near-flat in bear
  years (2018 −3% vs −77%, 2022 −2% vs −65%). CAGR scales with the vol target; at 10% the
  portfolio realises 13% vol and is long 39% of the time.
- 2025-26 are flat: the effect is weakest in range-bound markets.

## Checked and not promoted (exploratory scripts, not in this repository)

- **Gold trend (COMEX futures 2001-2026):** long-only ensemble Sharpe 0.59, below vol-targeted
  buy & hold gold (0.79). Trend adds nothing on gold over this bull period.
- **8 futures (gold, silver, copper, crude, 10y note, S&P 500, EUR, JPY):** Sharpe 0.42 at 0.02%
  cost, −0.08 at 0.10%. A classic CTA basket, too thin for our costs.
- **Funding carry (long spot + short perp, Binance 2020-2026):** BTC/ETH positive every year
  (BTC +17, +31, +4, +8, +12, +5, +2% of notional), but shrinking: ≈ 3%/yr on capital in
  2025-26, the level of USDT lending. Switching on the funding sign only adds costs.

## Where this leaves AEGIS

| Strategy | Market | Evidence | Status |
|---|---|---|---|
| Crypto trend (this report) | Binance / Bybit spot, daily | Sharpe 1.2-1.3 over 2018-2026, robust | candidate: paper-trade the daily weights |
| Power of 3 at 08:30 NY (`smc_models.md`) | XAUUSD CFD, intraday | +0.07 R net, t = 2.7 gross, 2026 out of sample +0.19 R | candidate: paper-trade on MT5, measure the real spread |
| Bounce, density, SMC, anomalies | intraday gold | no edge after costs | research only |
