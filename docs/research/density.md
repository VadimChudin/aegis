# Bounce off order-book densities (Bybit XAUUSDT)

The trader's method: see a large resting order ("плотность") at a level, check that it is real
(it is not moved; it gets eaten and refilled), rest a cascade of limits in front of it (bigger
the closer to the density), hold while there are no market hits against the position, scale
out, close everything when the move fades (tape slows, delta turns, price stalls), and exit at
once when the structure breaks. Stop right behind the density.

## Data

Bybit publishes the full 200-level order book of XAUUSDT (100 ms deltas) and the tape with the
aggressor side from the listing on **2026-03-09**. 203 days (to 2026-09-27), ~12 GB.
Binance has no order-book history, so this is the only source for a backtest.

```bash
python -m aegis_lab.data.bybit_history                 # download book + tape (cached)
python -m aegis_lab.research.density extract           # replay the book, one .npz per day
python -m aegis_lab.research.density_report            # bounce vs break by metric
python -m aegis_lab.research.density_sim [--grid]      # the trader's execution
```

Replay notes: the book feed trails the tape by ~40 ms (median) and leads it in ~13% of
changes; both cases are matched, so a size decrease is split into traded (eaten) and cancelled
(pulled) volume. An episode ("density") starts at 10x the median level size of its side within
$5 of the mid and ends below 4x for 3 s or when price trades through it.

## What a density is on gold

- A typical level holds **0.45 oz** (~$2k); the largest level within reach is ~58 oz. The book
  spans ~$7 each side.
- Most "densities" are market-maker quotes that flicker: 339k episodes a day at 10x, half of them
  live under 2 s.
- Bounces rise with size: win rate (price moves $1 away before trading $0.30 through, 131 764
  touches) is **22% below 5 oz, 27% at 10-20 oz, 31% at 20-30 oz**; by ratio to the median
  level, **14.5% below 7x vs 31.6% above 21x**. Interesting size on gold: **≥ 20 oz (≈ $90k),
  ≥ 20-50x the median level**.

## What separates a bounce from a break (touch-time, all 203 days)

| Metric | Worse | Better |
|---|---|---|
| Size left at the touch vs its peak (shrinking on approach = being pulled) | < 0.4: 14.6% | ≥ 0.85: 29% |
| Volume added before the touch ("доливают") | none: 16.7% | ≥ 1.2x peak: 27.3% |
| Relocated (a similar density was pulled nearby just before) | yes: 21.6% | no: 24.8% |
| Tape speed at the touch | 0.7-1.8 trades/s: 14.8% | < 0.3 trades/s: 31% |
| Volume on the approach vs the density size | > 0.8x: 18-20% | < 0.07x: 32.8% |
| Market hits into the density in the first 5 s on it | any: 34-37% | none: **48.5%** |

"Eaten and refilled while price is on it" (traded > 5% and refilled > 5% of the size, pulled
< 20%, first 2-5 s) does **not** separate: 37.0% vs 34.8%. Refilling *before* the touch does.

Combined (≥ 20 oz, not shrunk, quiet tape, not relocated): 35.6% (2 516 touches, weekdays);
plus no hits against in the first 5 s: **50.6%** (537).

## Cascade width

Of approaches that came within $0.5 of a standing density only 29% touched it; within $1, 16%.
Most turns happen in front of the density, so the outer orders matter. In the execution grid
width $0.10, $0.30 and $0.60 give about the same result per trade; wider fills 3x more often.

## The trader's execution (`density_sim`)

Cascade of 4 limits (weights 4:3:2:1, biggest next to the density), conservative fills (price
trades 1 tick through), orders cancelled when the density is gone; exits: stop behind the
density, *gone* (eaten or pulled while in the trade, market), partial + final target (limit),
fade (no new extreme for 30 s after $0.50 in favour, market), 30 min. Weekdays; in sample
Mar-Jun (82 days), out of sample Jul-Sep (63 days). 576 settings: size ≥ 8/20/30/50 oz, width
$0.10/0.30/0.60, stop $0.10/0.30/0.60, gone at 30/50% of the size, exit on gone on/off, target
$1/3/6/10.

- **Gross edge is real:** 495 of 576 settings earn before fees both in and out of sample; every
  month is positive. It grows with the density size (median out of sample +$0.02/oz at ≥ 8 oz,
  +$0.07 at ≥ 20, +$0.11 at ≥ 50).
- Best: ≥ 50 oz, width $0.10, exit on gone, target $10 - **+$0.19 per oz per trade out of
  sample** (588 trades, 9/day, 44% winners; in sample +$0.33 on 83 trades). 79% of exits are
  *gone*, 20% fade: the edge is "leave the moment the density is eaten or pulled".
- **Fees erase it.** Bybit XAUUSDT: standard 0.02% maker / 0.055% taker ($0.89 / $2.45 per oz at
  $4 450); the promotion since April 2026 is 0% maker / 0.0275% taker ($1.22 per oz). The gross
  edge is ~0.004% of the notional. Out of sample the best setting is **−$0.59/oz with the
  promotion, −$2.09 standard**; no setting is positive with fees in either half.

Gold moves ~5x less than a typical crypto altcoin in % per day while the fee in % is the same:
a density scalp that pays on altcoins does not cover a taker exit on gold.

## Next

- Maker-only exits (exits as resting limits, the stop only as the last resort) - the only way
  the promotion fee can be 0 on most exits.
- Cheaper execution: RoboForex spread + commission per oz (need the account's numbers), with the
  density read from Bybit.
- Densities as a filter for the level strategies rather than a scalp: a standing ≥ 20 oz density
  with no hits against lifts the bounce rate from 23% to ~50%.
- Moving to the Rust engine and sliders only after one of the above makes a net result.
