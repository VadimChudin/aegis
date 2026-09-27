# Bounce research (task #1–#6, first pass on Binance data)

Data: Binance XAUUSDT USDⓈ-M TradFi perpetual, 5m bars from 2026-01-05 (listing) to 2026-09-26,
rebuilt from aggTrades with order-flow columns (`python/aegis_lab/research/bars.py`) plus the
Binance metrics archive (open interest, top-trader long/short, taker ratio). 38 445 level touches.
Engine: `crates/aegis-core/src/bounce` (the same code the app runs).

## Levels and touches

Seven level kinds, each a toggle: 5m swing, 1h swing, previous day high/low, previous week
high/low, previous session high/low (Asia / London / New York), round price, previous day POC.
A touch is a bar that comes within 0.1·ATR of an armed level from the other side; a level re-arms
once price moves 1·ATR away. Every touch records 42 metrics in 10 groups (level, approach, market,
candle, volume, tape, clusters, derivatives, time); see `features.rs` for the definitions.

## What we found

1. **Levels hold more often than chance.** From the level, +1·ATR is reached before −0.5·ATR
   through the level in 52.7% of touches; a random walk gives about 33%.
2. **Entering at the close of the touch bar loses even before costs** (−0.03 to −0.26 R for
   targets 0.5–2 R): the stop behind the wick is far and much of the move is already gone.
3. **A resting limit order at the level has an edge before costs** (+0.27 to +0.41 R), and it
   pays the maker fee. Its metrics can only use data up to the bar before the touch, so the engine
   records every metric twice: before the touch (limit entry) and at the touch-bar close.
4. **Costs decide everything on 5m gold.** Median 5m ATR is about $4.3; Binance taker 4 bp is
   $1.8 per side. Trading every touch returns −1.3 R per trade after costs.
5. **Which metrics matter** (win rate by quintile, limit entry, target 1 R; `research.json`):
   - volatility: ATR in $ and its percentile, share of the average daily range already used
     (low volatility: 39% win against 54–61%);
   - weekday: Saturday and Sunday about 40% against 58% (the underlying gold market is closed);
   - confluence of level kinds (50% for one kind, 60% for 4–5);
   - 1h trend: an inverted U, best close to the 1h EMA50 (60%), worst far from it (47–48%);
   - room to the next level, level age, earlier touches and breaks of the level;
   - approach speed and distance, the approach delta, and the delta in the cluster at the bar
     extreme (all inverted U: moderate is best).
   Cluster and aggTrades metrics add nothing on top of kline metrics for the limit entry
   (logistic model out of sample: 73.2% win with klines only, 73.3% with everything). They may
   still matter for the touch-bar-close entry; not tested yet.
6. **A logistic model with squared terms is as good as gradient boosting** out of sample, so it
   runs inside the app (`model.rs`), retrained every month on the previous 4 months.

## The 70% setting (app defaults)

Limit at the level (+0.1·ATR), stop 0.5·ATR beyond the level, target 1 R, time exit 24 bars,
model probability ≥ 0.70, one position at a time. Out of sample March–September 2026
(January–February only train the model):

| Costs | Trades | Win | Avg R | Total R | PF | Max DD | Months positive |
|---|---|---|---|---|---|---|---|
| Binance (maker 0, taker 4 bp, $0.05) | 508 | 72.2% | +0.33 | +166 | 1.82 | 8.4 R | 6 / 7 |
| RoboForex ECN (0.2 bp, $0.15) | 549 | 70.5% | +0.36 | +195 | 2.11 | 5.6 R | 7 / 7 |

In the app the same run gives 536 trades, 70.5%, +0.28 R (klines archive only, one more day).

## Caveats

- About half of the trades fall in March 2026; later months have 22–107 trades each.
- The default (target, stop, threshold) was picked from a grid of 72 settings on the same months:
  mild selection bias on top of the out-of-sample probabilities.
- Fills: a limit fills only if price trades $0.02 through it; a stop and a target in the same bar
  count as a loss. Queue position and partial fills are not modelled.
- Only ~9 months of Binance history. Dukascopy (3 years) and RoboForex MT5 ticks with the real
  spread are the next step to confirm the edge on a longer and cheaper venue.
- Levels are an association, not a cause: a control with randomly shifted levels is still to do.

## Reproduce

```bash
python -m aegis_lab.data.binance_vision                       # ~1.1 GB into ~/aegis-data/binance
python -m aegis_lab.research.bars                             # ~/aegis-data/xau_5m.csv
cargo run --release -p aegis-core --example bounce -- touches ~/aegis-data/xau_5m.csv ~/aegis-data/touches.csv
python -m aegis_lab.research.bounce_analysis ~/aegis-data/touches.csv   # research.json + report
cargo run --release -p aegis-core --example bounce -- backtest ~/aegis-data/xau_5m.csv
```

The research scripts need `pandas`, `numpy` (and `lightgbm`, `scikit-learn` for the model checks).
