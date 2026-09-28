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

## Position management on 1-second data (v0.5.0)

Engine: `position.rs` walks 1-second candles built from every Binance aggTrade (`secs.rs`,
13.8 M seconds since the listing), so the entry, the stop, the target and every management rule
are checked second by second. With no management it agrees with the bar engine: 19 716 vs
19 688 trades, −0.310 vs −0.316 R (weekdays, no model, defaults).

All numbers below use weekdays only (RoboForex gold does not trade on weekends; on Binance
weekend touches lose −1.7…−2.1 R each because the ATR is so small that the $0.20 spread is most
of the stop), no probability model, one position at a time, RoboForex costs with a $0.20 spread.

| Setting (weekdays) | Trades | Win | Avg R |
|---|---|---|---|
| Limit at level, stop 0.5 ATR, target 1 R (base) | 19 716 | 40.0% | −0.310 |
| + breakeven after 0.5 R | 21 579 | 35.0% | −0.294 |
| + half off at 0.5 R, breakeven 0.5 R, target 2 R | 20 184 | 52.3% | −0.301 |
| Stop 1 ATR, trailing 0.3 ATR, target 3 R | 25 078 | 23.5% | −0.133 |
| same, ATR ≥ $5 | 11 182 | 27.1% | −0.088 |
| same, ATR ≥ $8 | 3 716 | 28.9% | −0.067 |
| same, ATR ≥ $5, **no costs at all** | 11 338 | 34.6% | −0.018 |
| Stop 1 ATR, flow exit 5× | 12 240 | 41.5% | −0.182 |
| same + flip into the breakout | 7 433 | 46.3% | −0.223 |
| Absorption entry 3×, target 1 R | 1 870 | 41.6% | −0.340 |
| Absorption 3×, trailing 0.3 ATR from 0.5 R, ATR ≥ $5 | 599 | 49.2% | −0.103 |
| Walk-forward GA over every setting incl. position ones, out of sample (seed 7) | 451 | 42.1% | −0.215 |
| Same budget, random search, out of sample | 1 159 | 33.8% | −0.177 |
| GA with the app's targets (70% wins, 20 trades/day), out of sample (seed 7) | 2 123 | 66.9% | −0.066 |

- **Still no edge.** Trailing on a volatile market is the least bad rule, but even without any
  costs it stays below zero, so the entries themselves carry no edge, and exits cannot create one.
- **The flow exit** (leave when aggressive volume goes through the level) roughly halves the loss
  of a plain limit: the order flow does tell early when a level is being eaten. **Flipping** into
  the breakout at that moment does not pay after costs.
- **Absorption entries** give the small stop they promise (median leverage for 1% risk ≈ 8×,
  up to 75× on the tightest stops, capped by *Max leverage*), but they fill rarely (2–7 a day) and
  lose too.
- **The GA** fits every training window (+0.02…+0.41 R) and loses in 6 of 7 test windows; 2 of 9
  checks pass. With the GA's settings, random price levels do better than the real levels
  (−0.027 R vs −0.215 R), so the levels themselves are not what earns.
- **With the app's targets** (70% wins, 20 trades a day) the GA gets close to the win rate out of
  sample (66.9%, 10 trades a day) at −0.066 R per trade, the best out-of-sample result so far, but
  still a loss: 4 of 9 checks pass. The metrics matter (shuffled metrics: −0.53 R), the levels do
  not (random levels: −0.060 R).
- The GA's final pick (edge-only fitness) uses a 1.3 ATR stop, trailing 1.6 ATR, 85% partial at 0.5 R,
  a flow exit at 3× and ATR filters: many parameters, fitted to noise.

## What next

- **Breakout.** Bounce loses because price usually runs through levels; the same data suggests
  testing the opposite trade (the Breakout slot) with the same engine and checks.
- **Longer history:** Dukascopy XAUUSD 1m since 2023 (downloading; the server allows about one
  file a minute), and RoboForex MT5 ticks with the real spread.
- **Order flow entries:** tested in v0.5.0 as the absorption entry and the flow exit (above);
  the flow exit helps, the absorption entry alone does not.
- **Order book:** real resting size at the level ("плотность") is not in the public archive
  (only depth sums in ±0.2…5% bands every 30 s); absorption from the tape stands in for it.

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
# 1-second engine: build 1-second candles once, then set sec_engine in the params
$B seconds ~/aegis-data/binance/XAUUSDT/aggTrades ~/aegis-data/xau_1s.sec
export SECONDS_FILE=~/aegis-data/xau_1s.sec
echo '{"sec_engine":true,"use_model":false,"trail_atr":0.3,"trail_from_r":0,"tp_r":3,"sl_atr":1,
  "filters":{"weekday":{"on":true,"min":0,"max":4},"atr_usd":{"on":true,"min":5,"max":20}}}' > trail.json
$B backtest ~/aegis-data/xau_5m_live.csv trail.json
```

`xau_5m_live.csv` / `xau_1m_live.csv` are the bar files cut at the listing (2026-01-05).
