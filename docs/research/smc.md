# Smart money (SMC) on gold: zones on 1h/4h, entry on 1m (DATA strategy research)

> **Update (September 2026): the same zones on 17 years of XAUUSD.** See *17-year test* at the
> end. No setting is positive after costs in both 2008-14 and 2015-25, and the order block +
> CHoCH lead does not hold.

Question: do mechanical SMC setups (order blocks, fair value gaps, liquidity sweeps, with
structure, premium/discount and killzone filters) earn on XAUUSDT after fees, with the entry
the trader uses: a limit, a grid of limits, or a 1m change of character (CHoCH)?

```bash
python -m aegis_lab.data.binance_vision --kinds klines-1m
python -m aegis_lab.research.smc            # one setting
python -m aegis_lab.research.smc --grid     # 1 536 settings, in sample Jan-May, out of sample Jun-Sep
```

Binance XAUUSDT 1m, 2026-01-05 … 2026-09-27 (266 days); cross-checked on Bybit 1m built from its
tape (from 2026-03-09). Zones are known only after the bar that proves them closed; swings are
confirmed 3 bars later. Fees Binance USDⓈ-M 0.02% maker / 0.05% taker (≈ $0.9 / $2.2 per oz).
Zones found: 1h - 394 order blocks, 1 267 FVGs, 1 296 sweeps; 4h - 80 / 296 / 352.

## Result

**No mechanical SMC setup has an edge after fees.** Of 800 settings with ≥ 30 trades in sample,
5 are positive both in and out of sample (what chance gives). Median out of sample: −0.3…−0.4 R.

| Zone | Entry | Gross R, in / out of sample (median) | Net R out of sample |
|---|---|---|---|
| FVG | limit / grid / CHoCH | −0.05 / −0.04, −0.03 / −0.03, +0.05 / −0.02 | −0.32, −0.19, −0.44 |
| Order block | limit / grid / CHoCH | +0.07 / −0.19, −0.01 / −0.12, **+0.30 / +0.18** | −0.42, −0.29, −0.18 |
| Sweep | limit / grid / CHoCH | −0.14 / −0.12, −0.07 / −0.16, −0.07 / −0.10 | −0.43, −0.42, −0.56 |

- Filters: trend alignment helps a little (−0.33 vs −0.38 R); premium/discount and killzones
  make it worse; 4h is not better than 1h and gives 4x fewer trades.
- **The only lead: 1h order block + 1m CHoCH entry.** Gross is positive in both halves in most
  settings (median out of sample +0.13…+0.28 R), but the market entry and stop cost more:
  net ≈ 0 (median −0.01…−0.2 R). It is rare: 17-45 trades in 4 months, too few to trust the best
  cells (+0.35 R on 17 trades). For an order block the trend filter does nothing (a block is
  always in the direction of the break that created it).

## A trap found on the way: the grid looked like +0.33 R

The first run showed "FVG + grid, target 3R" at +0.43 R in sample and +0.33 R out of sample,
positive every month and on Bybit too. Controls exposed it: random zones of the same size at
random times and random direction gave +0.23 R. The cause was the metric: R was per filled
risk, so a trade where only the smallest limit filled (price touched the edge and left, 36% of
trades, almost all winners) weighed as much as a full grid that was stopped. Measured in the
risk planned for the whole grid, the same setting is +0.14 R in sample and **+0.04 R out of
sample**, inside the random-zone range (−0.1…+0.16). The research code now measures R against the
planned risk and applies the minimum-risk filter to it (known before the entry). The app's
grid entry must report results the same way.

## What this means for DATA

- DATA can host every SMC element as a setting (zones, filters, entry limit/grid/CHoCH), but
  with default settings it is not a money maker on gold with Binance fees.
- Worth building on: order block + CHoCH, with a cheaper entry (a limit on the 1m retest after
  the CHoCH instead of market) and RoboForex costs; more history as it accumulates.
- "Auto" mode (walk-forward re-tuning) must only trade settings that were positive on data they
  were not tuned on; with this result it would stay flat most of the time, which is correct.

## Wide search (`smc_search.py`)

```bash
python -m aegis_lab.research.smc_search --samples 6000
```

Added: order block + FVG confluence, sweep → order block, a liquidity target (the other side's
range extreme of the last 24 bars), a **limit entry on the retest after the 1m CHoCH** (maker
instead of market), breakeven and a trailing stop in R, one trade per direction per minute.
6 000 random settings; 1 860 with ≥ 30 trades in sample and ≥ 15 out of sample.

- **Split:** 20 of 1 860 are net-positive in both halves (1%). The 20 best in sample average
  **−0.24 R** out of sample (3/20 positive). Median out of sample: gross −0.10 R, net −0.38 R.
  The CHoCH retest limit is not better than the market CHoCH (−0.42 vs −0.48 R).
- **"Auto" (walk-forward):** every month trade the setting that was best over the previous 3
  months: 240 trades, **−74 R (−0.31 R per trade)**; 5 of 6 months negative.

### Simulator check on a random walk

Every engine is now also run on a synthetic random walk with gold's minute volatility, where
any result above −fees is a bug. The first version of `smc_search` showed +0.3…+0.6 R there: a
trailing stop moved on a bar's high was filled at the stop price even when the next bar opened
below it. Fixed (the exit is at the worse of the stop and the open; R uses the initial stop);
the random walk now gives +0.03…+0.06 R gross, −0.2 R net. With the bug, "order block + FVG,
grid, trailing" looked like +0.24 R in both halves and beat random zones.
`smc.py` passes the same check (gross −0.3…+0.2 R on the random walk for the CHoCH entry with
~100 trades): that spread is also why the order block + CHoCH lead above is within noise.

### Conclusion

On XAUUSDT 1m/1h/4h (Jan-Sep 2026, Binance fees) mechanical SMC in every combination tested
(zones, confluences, filters, three entries, targets, trailing) does not beat random zones after
fees, and automatic re-tuning loses. Tuning more settings on the same 9 months will only find
curve fits. What could change the answer: much cheaper execution (RoboForex spread/commission),
more history (XAUUSD CFD from Dukascopy goes back years: `aegis_lab.data.dukascopy`), or a
discretionary element the rules above miss.

## 17-year test: XAUUSD 1m, 2008 … 2025-02 (`smc_long.py`)

This is the "more history" test above. The zone builder (`smc.zones`) and the 1m simulator
(`smc._sim`) are unchanged. They run on the MetaTrader CFD history used in `gold_lab.md`
(5.9 M minutes, New York clock) with RoboForex-like costs: $0.25/oz round trip plus $0.05/oz
slippage on the CHoCH market entry. R is per planned risk, as above.

- Zones: 1h has 6 167 order blocks, 19 866 FVGs and 22 185 sweeps; 4h has 1 541 / 5 651 / 7 344.
- The grid has 192 settings: 1h/4h × OB/FVG/sweep × limit / limit at the middle / grid / CHoCH
  × trend filter × killzones (London 02-05, New York 07-11 NY) × RR 1.5/3. Settings are chosen on
  2008-14 only; 2015-25 is never used for selection.

```bash
python -m aegis_lab.research.smc_long   # ≈ 1.5 min after the 1m cache from gold_sessions exists
```

**Result: 0 of 186 settings (≥ 100 trades in 2008-14) are positive net in both halves.**

| Best in 2008-14 | 2008-11 | 2012-14 | 2015-18 | 2019-22 | 2023-25 | 2015-25, random zones (5 seeds) |
|---|---|---|---|---|---|---|
| sweep, CHoCH, 4h, killzones, RR 3 | +0.19 | +0.09 | −0.17 | −0.27 | +0.05 | −0.31 … +0.13 |
| sweep, CHoCH, 4h, trend, RR 3 | +0.31 | −0.06 | −0.02 | −0.47 | −0.07 | −0.22 … +0.01 |
| sweep, CHoCH, 4h, RR 3 | +0.22 | −0.04 | −0.17 | −0.30 | +0.02 | −0.32 … −0.07 |

Net R per trade, 75-320 trades per cell. The top in-sample settings lose in 2015-25 (t −1.8 to
−3.0) and sit inside the random-zone range.

**Order block + CHoCH** (the lead from the Binance test):

| OB + CHoCH, trend | Trades 2008-14 | Gross / net 2008-14 | Trades 2015-25 | Gross / net 2015-25 |
|---|---|---|---|---|
| 1h, RR 1.5 | 561 | +0.06 / −0.08 | 950 | −0.04 / −0.19 |
| 1h, RR 3 | 561 | +0.09 / −0.05 | 950 | −0.12 / −0.27 |
| 4h, killzones, RR 1.5 | 80 | +0.24 / +0.14 | 158 | +0.04 / −0.06 |
| 4h, RR 1.5 | 124 | +0.21 / +0.09 | 252 | −0.05 / −0.18 |

- The CHoCH entry is the only one with a positive median before costs in 2008-14 (+0.06 R). In
  2015-25 it is −0.04 R. Limit, grid and middle-of-zone entries are negative before costs in both
  halves.
- 23 settings are positive before costs in both halves. None has more than +0.13 R gross, and
  after costs all are negative (best −0.001 R). A $0.25 round trip on these risks costs about
  0.1 R.
- Killzones make results worse (median −0.15 vs −0.09 R). The trend filter changes nothing, and
  for order blocks it is identical by construction: an OB is created by a break in its own
  direction.

**Conclusion.** With 17 years of history and cheap CFD costs, mechanical OB/FVG/sweep zones have
no edge on gold. This is the same answer as the 9-month Binance test, now on 20× more data and
across bull and bear regimes (2012-15 and 2016-18 included). What remains untested is only the
discretionary element.
