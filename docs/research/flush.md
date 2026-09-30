# Flush → buyback ("пролив → откуп") on Bybit XAUUSDT

Question: gold often drops fast and is then bought back (or rallies and is sold off). Is that
the rule, and does the book or the tape at the moment of the drop tell which way it will go?

## Data

Bybit XAUUSDT full 200-level book (100 ms deltas) and the tape with the aggressor side,
**204 days (2026-03-09 … 2026-09-28)**, replayed into 1-second rows: best bid / ask, resting size
within $0.5 / $1 / $2 / $5 of the mid on each side, the largest level within $5, aggressive buy
and sell volume, trade counts, the largest trade, and book flows within $2 (size added, eaten by
trades, pulled). Binance XAUUSDT aggTrades (the same move on another venue) and open interest
are joined on the same clock; the macro calendar (CPI, NFP, PPI, PCE, GDP, FOMC) marks releases.

```bash
pip install -e 'python[research]'
python -m aegis_lab.data.bybit_history                         # book + tape, ~12 GB, cached
python -m aegis_lab.data.binance_vision --symbol XAUUSDT --kinds aggTrades metrics
python -m aegis_lab.research.flush extract                     # 1-second rows, ~40 min on 4 cores
python -m aegis_lab.research.flush report                      # all tables below, ~3 min
python -m aegis_lab.research.flush_bounce                      # "is there always a small bounce?"
python -m aegis_lab.research.flush_sim                         # grid, partial exits, trailing
python -m aegis_lab.research.flush_signs                       # wide search for traces (~25 min)
python -m aegis_lab.research.flush_maker                       # limit-order execution of the signal
```

## Events

A drop is triggered, with no look-ahead, when the mid falls a set share of price below its high
of the window (in open gold hours). At that second the leg L = high − mid. **Buyback** = the mid
climbs 0.5 L before it falls 0.5 L more; **continuation** = the opposite. Rallies are mirrored.
25 metrics are measured at the trigger from past data only. The first and second halves of the
period are checked separately.

| Scale | Drops a week | Median leg | Buyback | Rallies: sold off |
|---|---|---|---|---|
| 0.15% in 10 min | 368 | $7.6 in 6 min | 49.4% (47.8 / 51.0) | 49.9% |
| 0.30% in 30 min | 104 | $14.7 in 19 min | 49.4% (48.2 / 50.6) | 49.2% |
| 0.50% in 60 min | 38 | $24.1 in 39 min | 50.1% (47.9 / 52.3) | 50.7% |
| Random second, same distance up vs down | | | 46.5-48.7% | |

**A flush is not usually bought back.** At every scale it is a coin flip, 1-3 points above a
random moment (the random baseline is below 50% because gold fell in part of the period). The
mean move back 15 minutes after the trigger is +0.01 … +0.04 L for drops. What the eye remembers
on the chart are the V-shapes; the drops that kept going are as frequent.

## What separates a buyback from a continuation

Buyback rate by quintile (Q1 → Q5) of a metric at the trigger; "A / B" is Q5 − Q1 in each half.
Only metrics with the same sign in both halves at two or more scales are listed.

| Metric at the trigger | 0.15% drops | 0.30% drops | 0.50% drops | Rallies |
|---|---|---|---|---|
| Resting size on the push side within $1 (asks in a drop) vs its 1 h mean | 51 → 47 (−3.6 / −2.5) | 49 → 45 (−4.2 / −2.9) | 53 → 42 (−11.6 / −8.9) | 0.30%: 50 → 44 (−4.7 / −8.2) |
| Book imbalance within $1 (hit side minus push side) | 46 → 52 (+3.5 / +6.7) | 46 → 50 (+1.9 / +8.5) | 44 → 54 (+5.6 / +19.9) | 0.15%: 47 → 51 |
| Resting size on the hit side within $1 (bids in a drop) vs its 1 h mean | 46 → 51 (+4.2 / +7.0) | 48 → 53 (+3.5 / +5.3) | | 0.15%: 47 → 52 |
| Size of the drop at the trigger (overshoot) | 48 → 52 (+5.8 / +3.1) | 45 → 53 (+10.2 / +5.1) | | |

- **The book near price is the only consistent signal.** A drop into thin bids with offers
  piling up just above keeps going; a drop into bids that are still there, with offers thin, is
  bought back more often. At the 0.5% scale the spread is 42% vs 53-54%.
- **The tape does not separate them.** Aggressive volume, delta, its acceleration in the last
  30 s, the largest trade, the speed of the leg, and the Binance tape all stay within noise.
- **News, hour and prior trend do not either.** Drops within 30 min of a release are bought back
  43-54% (few events), no hour stands out, and "rose before, then dropped" (4 h and 1 h before
  the leg) does not change the odds.
- Binance open interest is published every 5 minutes; with a 5-minute publication lag it rarely
  changes inside a leg, so it could not be tested at this resolution.

## Anatomy of the low (after the fact, 0.30% drops)

Mean per second in a 10 s window around the lowest point of the leg:

| Seconds from the low | −300 | −60 | −10 | 0 | +10 | +60 |
|---|---|---|---|---|---|---|
| Aggressive sells, oz/s (buyback / continuation) | 0.20 / 0.18 | 0.34 / 0.36 | 1.02 / 0.82 | **2.08 / 2.80** | 0.45 / 0.87 | 0.23 / 0.37 |
| Bids within $1, oz | 22.8 / 18.9 | 19.9 / 18.1 | 17.2 / 16.2 | 16.6 / 13.8 | 20.9 / 17.6 | 20.0 / 17.8 |
| Bids added − pulled, oz/s | −0.07 / −0.30 | −0.80 / −0.53 | −2.6 / −2.4 | −0.8 / −4.2 | **+3.6 / +1.2** | +1.4 / +0.3 |

Every low is a selling climax: aggressive selling runs at 10x its normal rate in the last
seconds while market makers pull bids. What differs is the next 10 seconds: in a buyback the bids
come back three times faster and the selling stops at once; in a continuation the selling slows
but stays, and bids come back slowly. This is visible only after the low, so it is a
confirmation, not an early signal.

## Is it tradeable?

Not as it stands. The best filter moves the odds of a symmetric 0.5 L / 0.5 L bet from 50% to
about 53-54%, worth ~0.04 L (~$1 on a $24 leg) before costs; a Bybit taker round trip on gold is
about $5 at $4 500. What remains usable:

- **As a filter for other tiles:** do not buy a drop into a thick offer wall within $1 above.
- **As a confirmation:** a bid refill of several oz/s right after a selling climax.

Limits: Bybit is a small venue (a typical level holds ~0.5 oz); price discovery happens on COMEX
GC, whose order-by-order data (CME MBO) is paid. Six months of one regime (gold at $4 300-5 200).

## "After a flush there is always a small bounce"

True, and it is the same after any moment. Share of drops where price rises a·L before it falls
b·L more, next to random open seconds with the same distances and to b / (a + b), the answer for a
price with no memory (`flush_bounce.py`). Cells: drops / random / no memory.

| Scale (chart) | Drops | Stop b | Bounce 0.1 L | 0.2 L | 0.3 L | 0.5 L |
|---|---|---|---|---|---|---|
| 0.15% in 10 min (1m) | 10 590 | 0.5 L | 79 / 78 / 83 | 69 / 68 / 71 | 60 / 60 / 62 | 49 / 48 / 50 |
| 0.30% in 30 min (5m) | 2 985 | 0.5 L | 82 / 80 / 83 | 71 / 69 / 71 | 62 / 60 / 62 | 50 / 47 / 50 |
| 0.50% in 60 min (15m) | 1 107 | 0.5 L | 83 / 81 / 83 | 73 / 68 / 71 | 63 / 59 / 62 | 50 / 46 / 50 |
| 1.0% in 4 h (1h) | 310 | 0.5 L | 85 / 81 / 83 | 71 / 67 / 71 | 60 / 57 / 62 | 47 / 45 / 50 |
| 2.0% in 24 h (4h) | 92 | 0.5 L | 82 / 80 / 83 | 66 / 67 / 71 | 59 / 58 / 62 | 45 / 43 / 50 |

A small bounce before a larger further drop happens 70-85% of the time, exactly as often as for a
price with no memory: a close target is simply reached more often than a far one. Measured from
the true low (known only afterwards), the median bounce is 0.6 L at every scale; that is what the
eye picks up on a chart, where the low is already visible.

## Trading it: grid, partial exits, trailing (`flush_sim.py`)

Drops traded long, rallies short, at 0.30% / 30 min (6 010 events) and 0.50% / 60 min (2 197).
Entries: market at the trigger; a limit grid at 0 / 0.25 / 0.5 L or 0.25 / 0.5 / 0.75 L below;
confirmation (+0.15 L off the running low). Exits: take-profit 0.5 L; half at 0.5 L and trail
0.3 L (breakeven after the first half); trail 0.3 L; half at 0.3 L and trail 0.5 L. Stop 0.3 L
below the last grid level, 4 h limit, 1-second fills against real trades, with and without the
book filter (thresholds fitted on the first half).

| 0.30% / 30 min | $/oz, Bybit fees | $/oz, CFD $0.25 | Win % |
|---|---|---|---|
| Market · TP 0.5 L | −4.62 | −0.57 | 37 |
| Grid 0/0.25/0.5 L · TP 0.5 L | −2.20 | −0.63 | **63** |
| Grid 0/0.25/0.5 L · half at 0.3 L + trail 0.5 L | −2.15 | −0.51 | 52 |
| Grid 0/0.25/0.5 L · trail 0.3 L | −1.61 | −0.26 | 20 |
| Confirm off the low · trail 0.3 L | −4.88 | −0.25 | 16 |
| Book filter · grid 0/0.25/0.5 L · trail 0.3 L (best on the first half) | −1.53 | −0.22 | 20 |
| Same rules from random seconds (5 seeds) | −1.39 … −1.51 | −0.11 … −0.22 | 19-21 |

All 64 combinations lose at both scales, in both halves, and even at $0.25/oz CFD costs (gross
per trade is −0.2 … −0.6 $/oz). A grid raises the win rate to 62-75% because it buys lower and
takes a close target, but the rare full-grid stop-outs cost more; it does not make the average
positive. The best combination does the same from random seconds. Grid, partial exits and
trailing change the shape of the results, not the sign: they cannot create an edge that the
entry does not have.

## Wide search: do algorithms leave traces? (`flush_signs.py`, `flush_maker.py`)

The event studies above ask "after a drop, what next?". The opposite approach: every 5th open
second (2.2 M samples, 7 months), 50 causal features — book depth and imbalance at $0.5 / $1 /
$2 / $5, depth vs its 1 h mean, added / eaten / pulled flows, aggressor delta and volume,
absorption (net aggression per $ of move), largest trades, position in the 30 min / 4 h range,
time spent at the price in the last 4 h (volume-profile node), VWAP, the Binance tape and price as
a lead, news, time — against the mid 5 s, 30 s, 2 min, 10 min and 30 min ahead. Walk-forward by
month, linear (ridge) and non-linear (LightGBM).

**Harness check first.** Planting ±0.5 bp on the 2-min move after a real book state (imbalance
top / bottom decile) raises the out-of-sample result from +0.65 to +1.26 bp per trade; shuffled
targets give IC 0.000. The pipeline finds an edge of that size when it exists. (The first version
of this check measured trades on real quotes instead of the planted target, so it could not see
the plant; fixed. Real-data results were not affected.)

**Traces exist, and they are strong at the scale of seconds.** Rank correlation with the move
ahead, first / second half:

| Feature | 5 s | 30 s | 2 min | 10 min |
|---|---|---|---|---|
| Book imbalance within $0.5 | +0.26 / +0.25 | +0.20 / +0.12 | +0.10 / +0.06 | +0.04 / +0.02 |
| Bid depth within $1 vs its 1 h mean | +0.22 / +0.19 | +0.17 / +0.10 | +0.09 / +0.05 | +0.04 / +0.02 |
| Imbalance change over 60 s | +0.19 / +0.18 | +0.15 / +0.09 | +0.08 / +0.04 | +0.04 / +0.02 |
| Binance moved more than Bybit in the last 10 s | +0.19 / +0.16 | +0.13 / +0.09 | +0.07 / +0.05 | +0.03 / +0.02 |
| Net bid refill in the last 10 s | +0.17 / +0.10 | +0.12 / +0.04 | +0.06 / +0.02 | +0.02 / +0.01 |
| Aggressor delta, last 10 s | +0.08 / +0.06 | +0.05 / +0.02 | +0.02 / +0.01 | 0 |

Everything slower — volume spikes, absorption, big trades, range position, volume nodes, VWAP,
news, hour — is below 0.03. Out of sample, every month positive:

| Ridge, top / bottom 0.5% | OOS IC | Trades | Gross bp | t | Net, Bybit taker (11 bp) | Net at CFD $0.25 |
|---|---|---|---|---|---|---|
| 5 s | 0.27 | 441 | **+1.16** | 9.7 | −9.9 | +0.61 |
| 30 s | 0.16 | 508 | **+1.54** | 6.9 | −9.5 | +0.99 |
| 2 min | 0.08 | 781 | +0.75 | 2.1 | −10.3 | +0.20 |
| 10 min | 0.03 | 971 | +0.47 | 0.9 | −10.6 | −0.08 |
| 30 min | 0.00 | 527 | −1.6 | −1.3 | | |

LightGBM matches the IC and is no better in trades; interactions add nothing to the linear book
signal. The edge is ~$0.5-0.7 per oz per trade and decays within minutes.

**It is local to Bybit.** The same Bybit features predict the *Binance* price with IC 0.09 at
5 s and 0.03 at 30 s, +0.2 bp per trade: most of the signal is Bybit's own quote catching up, not
the global gold price, so it does not carry over to a CFD at RoboForex.

**Limit orders do not rescue it** (`flush_maker.py`). With the current TradFi discount (maker 0%,
taker 0.0275%, per Bybit Learn, Aug 2026), a limit buy at the bid on a buy signal, filled only
when a trade prints through it:

| 30 s signal, top / bottom 0.5%, wait 30 s | Filled | Signal move, filled | …not filled | Round trip before fees, limit exit |
|---|---|---|---|---|
| | 53% | −0.57 bp | **+2.76 bp** | −1.14 bp |

The fill happens exactly when the signal is wrong: the trades that would have won run away from
the resting order. Limit in and out loses ~0.8-1.2 bp before fees in every variant (5 / 30 s,
0.5 / 2% signals, touch or trade-through fills). This trace is the market makers' own edge;
capturing it needs queue priority and latency, not a better rule.

**At the "bottom".** Seconds at a fresh 30-min low after a ≥ 0.2% drop (58 848): the next 5 s
continue down (30% up), the next 10 min drift +0.41 bp vs −0.14 bp elsewhere (52.7% up). What
separates turning lows, same sign in both halves (IC with the 10-min move): the low is in fresh
territory (little time spent at this price in 4 h, −0.09 / −0.07), far below the 1 h VWAP
(−0.07 / −0.06), a deep drop from the 4 h high (+0.05 / +0.10), bids thicker than usual
(+0.05 / +0.05), a large buy trade in the last minute (+0.04 / +0.05), bids eaten rather than
pulled (+0.03 / +0.04). These are the signs the eye looks for, and they are real, but the best
out-of-sample model quintile at the low gets +0.67 bp against +0.41 bp for the worst.
