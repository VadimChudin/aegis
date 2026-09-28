# Bounce research

Data: Binance XAUUSDT USDⓈ-M TradFi perpetual, 5m bars from 2026-01-05 (listing) to 2026-09-26,
with order-flow columns from aggTrades (`python/aegis_lab/research/bars.py`), the metrics archive
(open interest, top-trader long/short, taker ratio), funding, and 1m candles for the order of
events inside 5m bars. Engine: `crates/aegis-core/src/bounce` (the same code the app runs).

## Correction (v0.4.0): the v0.3.0 results were wrong

v0.3.0 reported a 70–72% win rate and +0.33 R per trade. Those numbers came from a look-ahead bug:
when one bar touched several levels, the scanner took the level **nearest the bar's low/high**,
i.e. it knew where price turned. A resting limit fills at the **first** level price reaches. The
bug grew with dense level grids, which is why the genetic algorithm liked a $2.5 round grid.

How it was found: the engine was run on a synthetic driftless random walk with the volatility of
the real bars. A correct engine cannot make money there; the old one made +0.11 R per trade with a
69.5% win rate (theory for that trade: 62.5% and 0 R).

What changed in the engine:

1. **First level reached** is the touched level (test `a_falling_bar_touches_the_first_level_it_reaches`).
2. **1m candles decide the order** of stop and target inside a 5m bar, and locate the limit fill
   inside the touch bar (test `minutes_decide_the_order_inside_a_bar`). "Stop first" alone biased
   results down by about −0.10 R per trade; a plain OHLC path heuristic still by −0.06 R.
3. **Spread** for data without quotes (Binance trade prints): buy limits fill only when the ask
   reaches them, exits pay the spread.

Engine check against an exact path: on a random walk generated tick by tick, every engine trade
was replayed on the ticks. 99.8% of the outcomes agree (12 158 trades). The engine averages
−0.068 R against −0.089 R on ticks: the engine exits at the stop price, while coarse ticks overshoot
it. The *Slippage per market fill* slider covers that gap in real trading.

## Result with the corrected engine

| Setup (RoboForex costs, $0.20 spread, 1m resolution) | Trades | Win | Avg R |
|---|---|---|---|
| Every touch, limit at level, stop 0.5 ATR, target 1 R, no model | 26 964 | 43.4%* | −0.155 |
| Same, no costs at all | 18 938 | 43.4% | −0.133 |
| Model, calibrated probability ≥ 0.4 (Apr–Sep) | 9 156 | 39.9% | −0.340 |
| Close entry (market at touch-bar close), no model | 11 221 | 43.4% | −0.241 |
| Genetic algorithm, walk-forward, out of sample (seeds 7 / 21 / 99) | 227 / 1 180 / 449 | 71% / 65% / 62% | −0.043 / −0.087 / −0.121 |
| Same budget, random search, out of sample | 318 / 620 / 274 | — | −0.087 / −0.171 / −0.184 |
| Same GA on shuffled metrics, out of sample | 123 / 91 / 62 | 54% / 45% / 47% | −0.359 / −0.463 / −0.426 |

\* win = trade closed with R > 0.

- **No bounce edge on 5m Binance gold.** Even without costs, a limit at the level loses: price
  runs through levels more often than it turns (a level holds in 33–38% of touches; the bug made
  it look like 53%).
- Calibrated probabilities are honest: they match the realised win rates (36–41%). The first
  scored month (March) only feeds the calibration and has no probabilities.
- The metrics do separate touches (win rate 7% at the lowest volatility quintile vs 45% at the
  highest; weekends 11–15% vs 39–41% on weekdays; see `research.json`), but no subset reaches the
  break-even rate after costs.
- The genetic algorithm beats random search and beats itself on shuffled metrics in every seed, so
  it does learn from the metrics, but nothing it finds is profitable out of sample, and the checks
  say so (2–3 of 9 passed). A win rate of 62–71% with a negative average R shows why win rate
  alone is not a goal: small targets win often and still lose money.
- Binance at standard fees (maker 2 bp, taker 5 bp) is worse still.

## The genetic algorithm (`ga.rs`)

- Real-coded GA (Deb): binary tournament selection, SBX crossover (η=15, p=0.9), polynomial
  mutation (η=20, p=1/genes), uniform crossover and bit-flip for toggles, 2 elites, 10% random
  immigrants. Genes: every tunable slider and toggle plus min/max/on of up to 14 metric filters.
- Fitness: t-statistic of mean R minus penalties for missing the win-rate and trades-per-day
  targets and for each active filter.
- Walk-forward: train 60 days (split 75% train / 25% validation), test the next 30 days; the GA
  sees only train; early stopping and the final choice use validation and the fitness of
  perturbed neighbours ("plateau, not peak").
- Checked: finds a planted optimum far better than random search (unit test); deterministic for a
  seed; beats random search with the same budget out of sample on real data; on permuted metrics
  its out-of-sample result is negative.

## Statistical checks (`validate.rs`)

| Check | Method |
|---|---|
| Positive expectancy | 95% stationary bootstrap over days (Politis & Romano 1994) |
| Probabilistic Sharpe ≥ 0.95 | Bailey & López de Prado 2012 |
| Daily t-statistic > 3 | Harvey & Liu 2015 |
| Limit fills | same trades filled on touch / as set / only $0.10 through |
| Deflated Sharpe ≥ 0.95 | Bailey & López de Prado 2014; effective trials N̂ = ρ̄ + (1−ρ̄)·M |
| Levels beat random prices | levels moved by ±(0.5…1.5)·median ATR; Welch test |
| Metrics matter | metrics shuffled between touches (Aronson 2006) |
| PBO < 0.5 | CSCV over the GA's final population, 10 blocks, 252 splits (Bailey et al. 2017) |
| GA beats random search | out of sample, same evaluations per window |

## What next

- **Breakout.** Bounce loses because price usually runs through levels; the same data suggests
  testing the opposite trade (the Breakout slot) with the same engine and checks.
- **Longer history:** Dukascopy XAUUSD 1m since 2023 (downloading; the server allows about one
  file a minute), and RoboForex MT5 ticks with the real spread.
- **Order flow entries:** cluster and tape metrics were built but only tested for the limit
  entry; they may matter for a confirmation entry.

## Reproduce

```bash
python -m aegis_lab.data.binance_vision --kinds klines-5m klines-1m aggTrades metrics
python -m aegis_lab.research.bars                                  # ~/aegis-data/xau_5m.csv
# 1m CSV: time,open,high,low,close,volume from the klines-1m archives (see the release notes)
export MINUTES=~/aegis-data/xau_1m_live.csv
B="cargo run --release -p aegis-core --example bounce --"
$B backtest ~/aegis-data/xau_5m_live.csv            # defaults
$B validate ~/aegis-data/xau_5m_live.csv            # checks of the defaults
$B optimize ~/aegis-data/xau_5m_live.csv ga.json    # walk-forward GA + checks
$B ga-noise ~/aegis-data/xau_5m_live.csv ga.json    # the same GA on shuffled metrics
$B liveness ~/aegis-data/xau_5m_live.csv p.json     # does every slider change the result?
```
