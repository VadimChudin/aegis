# Smart money / ICT: knowledge base and what it means for AEGIS

Collected from public ICT/SMC material and independent backtests (September 2026). For each
concept: the rule as it is taught, whether it can be coded without hindsight, and its status here.
Sources are at the end. ICT = Michael Huddleston's "Inner Circle Trader"; SMC is the community
name for the same ideas.

## 1. Building blocks

| Concept | Rule as taught | Codable | Status in AEGIS |
|---|---|---|---|
| Swing / structure (BOS) | break of the last swing high/low in the trend direction | yes | done (`smc.py`) |
| CHoCH / MSS | first break against the trend; MSS = with displacement | yes | done (1m CHoCH entry) |
| CISD (change in state of delivery) | close beyond the open of the last opposite run of candles | yes | **not tested** |
| Displacement | a strong, large-bodied move that leaves an FVG | yes (body/ATR) | partly (FVG only) |
| Order block | last opposite candle before the move that broke structure | yes | done |
| Breaker block | an order block that failed: price broke through it; used from the other side | yes | **not tested** |
| Mitigation block | failed swing without a sweep, retested from the other side | yes | **not tested** |
| FVG | 3 candles, candle 1 high < candle 3 low | yes | done |
| Inversion FVG (IFVG) | an FVG closed through; flips side | yes | **not tested** |
| Consequent encroachment (CE) | 50% of an FVG, the usual limit price | yes | done (depth 0.5) |
| Balanced price range (BPR) | overlap of a bullish and a bearish FVG | yes | **not tested** |
| Unicorn | breaker block overlapping an FVG | yes | **not tested** |
| Liquidity: BSL/SSL, equal highs/lows | stops above highs / below lows | yes | done (sweeps, EQH/EQL levels) |
| Inducement | minor swing in front of the zone that price sweeps first | partly | not tested |
| Dealing range, premium/discount, equilibrium | range between swings, buy under 50%, sell above | yes | done (filter, did not help) |
| OTE | entry at 62-79% retracement of the displacement leg | yes | **not tested** |
| IRL → ERL | price alternates internal (FVG) and external (range extremes) liquidity | partly | liquidity target done |
| Opening gaps NDOG / NWOG | 17:00→18:00 NY daily / weekly gap as a magnet | yes (CFD/futures) | **not tested**; Binance/Bybit trade 24/7, use CFD data |
| IPDA 20/40/60 days | liquidity at the 20/40/60-day highs/lows is the draw | yes | **not tested** |
| SMT divergence | one correlated market makes a new extreme, the other does not (gold vs silver, gold vs DXY) | yes (needs silver/DXY data) | **not tested** |

## 2. Time

- Killzones (NY time): Asia 20:00-00:00, London 02:00-05:00, New York 07:00-10:00, London
  close 10:00-12:00. Tested as a filter: made results worse.
- Silver Bullet windows: 03-04, **10-11**, 14-15 NY.
- Macros: short 20-minute windows (e.g. 09:50-10:10) — too fine to test honestly.
- Midnight NY open and 08:30 open as the day's reference prices.

## 3. Complete models (entry rules end to end)

| Model | Rules | Status |
|---|---|---|
| **Silver Bullet** | in the 10-11 NY window: liquidity sweep → MSS on 1-3m → first FVG → limit at FVG (or CE), stop behind the sweep, target the opposite liquidity, min 2R | **not tested** |
| **Power of 3 / AMD / Judas swing** | Asia range = accumulation, London sweeps one side (Judas), NY delivers the other way; trade the reversal after the sweep | **not tested** |
| **London sweep of the Asian range** | Asia 00-06 UTC high/low; in London 07-10 UTC a sweep that closes back inside → MSS → entry, target the other side | **not tested** |
| **CRT (candle range theory)** | reference candle (1h/4h/daily) high/low; next candle sweeps one side and closes back inside; entry on 5m MSS/FVG; targets 50% and the other side | **not tested** |
| **Turtle soup** | fade a new 20-bar high/low that fails | close to our sweeps (negative) |
| **2022 model** | HTF bias → sweep → displacement with FVG → entry at FVG | ≈ our sweep/FVG + CHoCH tests (negative) |
| **Unicorn** | breaker + FVG overlap as the entry zone | **not tested** |
| **OTE after MSS** | after a CHoCH/MSS leg, limit at 62-79% of the leg, stop behind the leg | **not tested** |
| **NDOG/NWOG model** | trade towards/away from the opening gaps in NY AM or after lunch | **not tested** (CFD data) |
| **Daily bias (previous day)** | close above previous day high → bullish, etc.; trade only in the bias direction | **not tested** as a filter |

## 4. What independent tests found

- **StatOasis (Sept 2026):** order blocks, FVG, sweeps and OTE coded mechanically, 648 backtests
  on US index ETFs (daily bars): none significant on SPY (best: order blocks, t = 1.22), 1 of 32
  tests significant across four markets; 0 of 648 beat buy-and-hold. ICT entries were not worse
  than simple textbook entries either.
- **FXNX (1 000 mechanical trades, FX majors):** win rate 38-48%, long losing streaks; the
  discretionary "70-80%" drops to ~41% when coded.
- **FVG "fill rate" (Formiq, USDJPY 2 years):** gaps fill 96-99% of the time, but ordinary bars'
  highs/lows are revisited just as often (97-99%). The fill rate says nothing special about FVGs.
  Only ~49% of FVGs close fully (FXNX).
- **FVG on tick data (Curupira, EURUSD):** profit factor 0.80 on 5m, 0.94 on 15m, 1.04 on 1h, 1.17
  on 4h, 4.28 on daily OHLC: the edge grows with bar coarseness because bar backtests resolve
  stops too kindly. Same trap we found in our trailing stop.
- **Power of 3 on gold (CRTLAB, 5 years hourly):** on up days the low came before the high in
  90%, and was in before 10:00 NY in 93%; on down days the high came first in 88%, before 10:00
  in 90%. This is the strongest *structural* fact about gold in the whole field — but it is
  descriptive (it only says which extreme came first once you know the day's direction); CRTLAB
  notes the tradeable version is much rarer.
- **Liquidity sweep EA on gold M5 (MQL5 market, Jan-Aug 2026, real ticks):** session high/low
  sweep ≥ 0.1 ATR, close back inside, MSS within 10 bars, market entry, stop behind the sweep, 2R:
  183 trades, 38.8% wins, profit factor 1.22. Vendor numbers, one period, no out-of-sample; worth
  reproducing because it is exactly specified.

## 5. Interpretation for AEGIS

1. Our result (no mechanical OB/FVG/sweep edge on XAUUSDT after fees) agrees with every
   independent test found. The profitable claims are discretionary or on coarse bars.
2. What we have **not** tested is the time-anchored part of ICT: Silver Bullet, Asia-range
   sweep in London (Judas), CRT on 1h/4h/daily candles, Power of 3 with the midnight open, daily
   bias from the previous day, IPDA 20/40/60-day liquidity, and SMT gold vs silver. These are the
   models with fixed rules; gold's "extreme before 10:00 NY" statistic is the reason to expect
   something there if anywhere.
3. Also untested blocks: breaker, IFVG, BPR, unicorn, OTE, CISD — cheap to add to `smc.py`.
4. Data: Binance/Bybit gold trades 24/7 with no daily gap, so NDOG/NWOG need XAUUSD CFD data
   (Dukascopy, loader in `aegis_lab.data.dukascopy`, 3+ years with real bid/ask). SMT needs
   XAGUSDT (listed on Binance/Bybit) or Dukascopy XAGUSD.
5. Every test keeps our rules: 1m resolution with the stop filled at the worse of stop/open,
   in-sample / out-of-sample split, random-zone and random-walk controls, walk-forward "Auto".

## Test plan (DATA)

| # | Model | Data |
|---|---|---|
| 1 | Asia range sweep in London (Judas) → MSS → entry, target the other side | Binance 1m + Dukascopy 3y |
| 2 | Silver Bullet 10-11 NY (sweep → 1m MSS → FVG limit) | same |
| 3 | CRT on 1h / 4h / daily reference candles, 5m entry | same |
| 4 | Power of 3: midnight-open bias, trade the reversal after the sweep | same |
| 5 | Daily bias and IPDA 20/40/60 liquidity as filters/targets for 1-4 | same |
| 6 | SMT gold vs silver as a confirmation for 1-4 | + XAG 1m |
| 7 | Breaker, IFVG, BPR, unicorn, OTE, CISD zones in the existing search | Binance 1m |

## Sources

- TradingEdge ICT concept library — tradingedges.org/en/learn/ict-concepts
- Warzone Trading ICT/SMC glossary — wzt.fund/glossary
- The Inner Circle Traders: advanced concepts, CRT, SMT — theinnercircletraders.com
- ICT Flow lessons: Silver Bullet, Power of Three — ictflow.com/lesson/9, /lesson/6
- innercircletrader.org Silver Bullet; innercircletrader.net Unicorn, NDOG/NWOG lecture notes
- ICT Killzone: IPDA — ictkillzone.com/ict-ipda
- AlgoKings: Power of Three — algokings.net/learn/power-of-three-po3
- CRTLAB Power of 3 backtester — crtlab.pro/ict-power-of-3-backtester
- StatOasis, "I Backtested ICT / Smart Money Concepts — What Survives" (2026-09-03)
- FXNX, "Do Smart Money Concepts Work? The Backtest Evidence"; "FVG fill rate" (2026-08-23)
- Formiq, "Do Fair Value Gaps Really Fill? 2 Years of USD/JPY Tested" (2026-08-28)
- Curupira, "FVG Magnetism" (2026-02-12)
- GrandAlgo CRT cheat sheet; MQL5 "Liquidity Sweep Gold" EA listing (2026-09-07)
