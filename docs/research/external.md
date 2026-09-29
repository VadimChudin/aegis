# External gold systems: TanvirCCC "lnterqo" and GoldTraderEA

Both repositories have no licence, so nothing is copied: the rules were re-implemented from their
READMEs and inputs. Data: XAUUSD CFD 1m 2019-01 … 2025-02 (the Hugging Face export used in
`smc_models.md`), Binance XAUUSDT 1m 2026; cost $0.35/oz per trade; exits resolved on 1m candles;
every run repeated on a random walk with gold's hour-of-day volatility (`smc_models.random_walk`).

```bash
python -m aegis_lab.research.tanvir --data hist [--rw 1]
python -m aegis_lab.research.vote --data hist [--rw 1]
```

## TanvirCCC/algo-trading — claimed 622 out-of-sample trades, +2.13 R, PF 4.74

ICT/MMXM: 5m CISD (close beyond the last swing after higher lows / lower highs) in the London and
New York killzones, with breaker / inverse FVG / FVG / Asia-range sweep as a confidence score,
daily bias, stop behind the CISD bar, target at the nearest liquidity (Asia extreme, previous day,
older swings) at ≥ 2 R, plus a random-forest filter.

His own trade file (`backtest/lnterqo_v3_oos_trades.csv`) already shows where the number comes from:

- median stop **$0.62** (5m ATR ≈ $4); 43% of stops under $0.50. At $0.35/oz the file's mean
  falls from +2.13 R to **+0.02 R**;
- 42% of trades close within 5–10 minutes, inside one 5m bar, where his backtest decides stop vs
  target by the **bar's colour**;
- by year: +2.3…2.6 R in 2021–2023, +0.87 R in 2025, +0.48 R in 2026.

Re-implementation, six years (≈ 5 800 signals):

| Accounting | Real gold | Random walk (2 seeds) |
|---|---|---|
| His: entry at the CISD close, bar colour, no cost | +0.068 R | **+0.092 / +0.120 R** |
| Next 1m open, 1m resolution, no cost | +0.021 R (t = 1.0) | +0.025 / +0.057 R |
| Same, $0.35/oz | −0.450 R | −0.505 / −0.509 R |
| Min stop 0.5 ATR, $0.35/oz | −0.305 R | −0.265 / −0.260 R |

- The accounting alone creates the edge: it is **larger on a random walk than on gold**.
- No filter helps: confidence ≥ 2/3/4, no daily bias, fixed 1 R/2 R targets are all −0.26…−0.31 R
  net and negative in every year. Binance 2026: −0.26 R (min stop 0.5 ATR).

## mehdi-jahani/GoldTraderEA — claimed 68% win rate, +287% (no trades or code for it published)

A weighted vote of 14 modules on H1 (≥ 7 weighted confirmations, MA100 trend, stop 2 ATR, target
4 ATR). The mechanical modules (candles, price action, RSI/MACD, divergence, support/resistance,
EMA cross, pivots, session, volume, H4 slope) were re-implemented with the EA's weights.

| Votes needed | Trades (6 years) | Win | Gross R | Net R | Random walk net |
|---|---|---|---|---|---|
| ≥ 5 | 1 357 | 34.1% | +0.024 | −0.016 | −0.127 |
| ≥ 6 | 1 226 | 36.0% | +0.079 | +0.039 (t = 0.9) | −0.121 |
| ≥ 7 (EA default) | 1 092 | 35.7% | +0.071 | +0.031 (t = 0.7) | −0.105 |
| walk-forward fitted weights | 689 | 33.7% | +0.011 | −0.025 | −0.050 |

- Nowhere near 68%: a 2:1 target wins 34–36%.
- Net ≈ 0 (t < 1). The ~0.15 R gap to the random walk mostly reflects the MA100 trend filter,
  i.e. the documented time-series momentum, not the vote; fitting the weights walk-forward does
  not improve it.

## Verdict

Neither system has an edge that survives realistic accounting. What is worth taking into DATA:

- the detectors as switchable elements (CISD, breaker, inverse FVG, Asia sweep, killzones,
  confidence score) — with a **minimum stop in ATR** and 1m resolution enforced, so the app cannot
  reproduce Tanvir's artefact;
- the vote as a structure (elements → weighted score → threshold), with weights tuned by the
  walk-forward GA only if they beat random weights out of sample;
- the only lead found in this repository so far remains Power of 3 from the midnight open
  (`smc_models.md`, +0.07 R net at $0.35, t = 2.7 gross).
