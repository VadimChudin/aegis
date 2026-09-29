# Time-anchored ICT models on gold: Judas, Silver Bullet, CRT, Power of 3

Plan items 1-4 of `smc_knowledge.md`, tested with fixed rules on 1-minute data.

```bash
python -m aegis_lab.research.smc_models --data hist            # XAUUSD CFD 2019-01 … 2025-02
python -m aegis_lab.research.smc_models --data hist --rw 0     # same on a random walk
python -m aegis_lab.research.smc_models --data binance         # XAUUSDT 2026
```

## Data

- **XAUUSD CFD 1m, 2019-01-02 … 2025-02-28** (2.17 M minutes) from a public MetaTrader export on
  Hugging Face (`agzdws/xauusd-gold-price-historical-data-2004-2025`). Server time = New York + 7 h
  (the week opens at 01:00 server = 18:00 NY in summer and winter; the 17:00 NY hour has 4% of
  the usual minutes — the daily break). Later months have gaps, so the test stops at 2025-02.
  No spread column: cost **$0.35/oz per trade** (typical ECN spread + commission); in sample
  2019-2022, out of sample 2023-01 … 2025-02.
- Binance XAUUSDT 1m 2026 (fees 0.02/0.05%), in sample Jan-May, out of sample Jun-Sep.
- Dukascopy was the plan but its feed answered 503 to almost every request (4 files in 20 min).

## Models (`smc_models.py`)

| Model | Setup (known at the time) |
|---|---|
| judas | Asia range 20:00-00:00 NY swept in London 02:00-05:00; reversal, target the other side |
| sb_03 / sb_10 / sb_14 | Silver Bullet hour: first sweep of previous-day / Asia / London / previous-hour high or low that was beyond the window open; target the nearest level on the other side |
| crt_1h / 4h / 1d | candle 2 takes candle 1's high or low; *close* entry after candle 2 closed back inside, or an intra-candle 1m MSS entry (only the first cross is known, not the close) |

Entries after the sweep: market at the 1m MSS, limit at the first FVG edge or its 50% (CE),
limit at OTE 70.5%. Filters: previous-day direction (with/against), Power of 3 (long only below
the midnight open). Grid: entry × RR 1.5/2/3 × model target on/off × stop buffer × min risk ×
filters (+ CRT target midpoint/far side): 6 624 settings.

**Look-ahead caught:** the first CRT version filtered intra-candle entries on candle 2's close,
which is only known at its end. Daily CRT looked like +1.36 R gross; without the look-ahead,
+0.22 R gross and −0.19 R net on Binance, ≈ 0 on six years.

## Result: no edge

| Data | Settings with ≥ 30/15 trades | Net positive in both halves | Top 15 in sample → out of sample | Walk-forward "Auto" |
|---|---|---|---|---|
| CFD 2019-2025 | 5 011 | 185 (3.7%) | −0.32 R | 150 trades, **−0.48 R/trade**, 14/68 months positive |
| same on a random walk | 3 844 | 35 (0.9%) | −0.41 R | −0.35 R/trade |
| Binance 2026 | 3 198 | 0 | −0.60 R | −0.65 R/trade, 0/6 months |

Median gross R per model on 2019-2025 is −0.09 … +0.04 in both halves; net −0.12 … −0.30.
Median by entry: CRT close entry is the least bad (−0.05 R out of sample). No filter changes
the median by more than 0.03 R.

## Power of 3 from the midnight open: the one lead

The famous statistic "on up days gold's low is in before 10:00 NY in 93% of days" is mostly
arithmetic: on our data the extreme set before 10:00 holds to the close 79.6% of days, on a
random walk with the same clock 77%. But a tradeable form of the idea has an edge on six years:

> At 08:30 NY go long if price is above the midnight open (short if below), stop behind the
> day's low (high) so far, exit at 16:00 NY.

- 1 434 trades, gross **+0.122 R** (t = 2.7, bootstrap 95% CI +0.04 … +0.21), +0.119 R in
  2019-22 and +0.128 R in 2023-25; 6 of 7 years positive (2025: two months, −0.08).
- Net +0.09 / +0.07 / +0.03 R at $0.20 / $0.35 / $0.60 per trade. Median risk $8.
- Every entry time 07:00-10:00 × exit 12:00/14:00/16:00 is positive (+0.03 … +0.12 R); 08:30 is
  the best, the ICT "8:30 open".
- Five random walks: −0.09 … +0.05 R. The edge is about 2 standard deviations of that spread.
- Binance 2026 (190 days): +0.125 R gross, but the taker fees (~$4.3/oz round trip vs a $32
  median risk) make it −0.04 R.
- ICT filters on top: London Judas sweep, previous-day direction and distance from the open do
  not help consistently. Premium/discount (long only in the lower half of the day's range)
  shows +0.35 / +0.47 R in both halves, but on 253 trades carried by 2022 and 2024, gone at a
  09:00 entry and within the random-walk spread (−0.35 … +0.10): not trusted.

This is consistent with the documented intraday momentum in futures (the morning move tends to
continue into the close). It is small: about +$0.55/oz per trade net on a CFD at $0.35 cost,
one trade a day.

## Next

- Paper-trade the 08:30 rule on the RoboForex/MT5 feed (cheap spread matters: at Binance fees it
  is negative). It has no parameters to tune, so "Auto" does not apply.
- Remaining plan items: IPDA 20/40/60-day liquidity, SMT gold vs silver, breaker / IFVG / BPR /
  unicorn / OTE zones.
