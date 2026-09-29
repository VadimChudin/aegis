# The full ICT 2022 model on 17 years of gold (`ict_full.py`)

Earlier tests (`smc.md`, `smc_models.md`) took SMC apart and tested each zone or time model
alone. The objection was that traders trade the **whole sequence**, not single zones. This test
codes the complete top-down model from the ICT 2022 mentorship (episodes 1-2: killzone →
liquidity raid → market structure shift → displacement FVG → entry) with the ICT daily-bias
lessons (order flow, previous-day close, draw on liquidity). Every step is an explicit rule.

```bash
python -m aegis_lab.research.ict_full   # ≈ 10 s after the 1m cache from gold_sessions exists
```

Data: XAUUSD CFD 1m, 2008 … 2025-02, New York clock. Costs: $0.25/oz round trip, plus $0.05/oz
slippage on stop and time exits. A 1m bar that reaches both the stop and the target counts as a
stop.

## The rules

| Step | Rule |
|---|---|
| Day | trading day 18:00 → 17:00 NY; PDH / PDL / close = previous trading day |
| 1. Bias | `none`; `flow`: yesterday closed in the upper / lower half of its range; `pdclose`: yesterday closed above the high / below the low of the day before (inside = no trade); `draw20`: the nearer of the 20-day high / low is the draw; `oracle`: the day's real direction (**not tradable**, it only shows what a perfect bias is worth) |
| 2. Killzone | London 02:00-05:00 or New York 07:00-10:00 |
| 3. Liquidity raid | inside the killzone price trades through liquidity against the bias that was not taken earlier: Asian low (20:00-00:00), PDL, and for New York the London low (shorts mirror) |
| 4. MSS | after the raid, a 5m (or 1m) close beyond the last swing (k = 2) formed before the raid, before the killzone ends |
| 5. Displacement | the leg from the raid extreme to the MSS leaves a fair value gap; the one nearest the MSS is used; no FVG = no trade |
| 6. Entry | limit at the FVG edge or at its middle (CE), valid until 60 min after the killzone; cancelled if the target or the stop trades first |
| 7. Stop | beyond the raid extreme + max($0.20, 1% of daily ATR); risk must be 5-50% of daily ATR |
| 8. Target | the nearest opposite liquidity (Asian high, PDH, London high) at ≥ 2 R (none = no trade), or a fixed 2 R / 3 R |
| 9. Management | optional breakeven at +1 R; exit at 16:00 NY at the latest; one trade per killzone |

Settings are chosen on **2008-14 only**. The control replays every 2015-25 trade with the same
side, risk and target at the same clock minute of a random other day (5 seeds). It asks whether
the setup's timing matters, with the trade geometry and costs held fixed.

## How often the full setup occurs

Of 34 880 day × killzone × side combinations (both MSS timeframes): 31 128 have untaken
liquidity, 10 940 see a raid in the killzone, 4 207 get an MSS in time, 3 691 have a
displacement FVG, and 2 604-2 753 limits fill. One setting trades **19-42 times a year**.

## Result

288 tradable settings (4 biases without oracle × 3 sessions × 2 MSS timeframes × 2 entries ×
6 exits); 170 have ≥ 100 trades in 2008-14.

- **None is positive net in 2008-14.** The best is −0.013 R. Out of sample (2015-25), 56% are
  positive net.
- Before costs: positive in 15% of settings in 2008-14 and in 95% in 2015-25.
- MSS on 1m is better than 5m: median gross −0.03 vs −0.17 R in 2008-14, +0.14 vs +0.07 R in
  2015-25.
- Even the **oracle bias** (knowing the day's direction) earns only −0.07 R net in 2008-14 and
  −0.04 R in 2015-25 (medians). Bias is not the missing piece: the entry/exit geometry is.

| Best in 2008-14 (net R per trade) | trades/yr | 2008-11 | 2012-14 | 2015-18 | 2019-22 | 2023-25 | 2015-25 random timing | setup − random |
|---|---|---|---|---|---|---|---|---|
| draw20 · NY · MSS 1m · CE · liquidity target | 19 | −0.06 | +0.04 | −0.01 | **+0.31** (t 1.6) | +0.05 | −0.35 … −0.14 | +0.35 (t 2.7) |
| same, breakeven at 1 R | 19 | +0.02 | −0.08 | +0.02 | +0.08 | +0.06 | −0.16 … −0.01 | +0.12 (t 1.0) |
| none · London · MSS 1m · edge · 3 R | 42 | +0.03 | −0.09 | −0.06 | +0.10 | +0.06 | −0.15 … −0.05 | +0.13 (t 1.4) |
| *oracle · London · MSS 1m · edge · 3 R* | 22 | +0.21 | −0.10 | +0.13 | +0.10 | +0.20 | −0.25 … −0.07 | +0.27 (t 2.0) |

By side (MSS 1m, CE, 2 R, all setups):

| Period | shorts gross / net | longs gross / net |
|---|---|---|
| 2008-11 | −0.03 / −0.14 | −0.01 / −0.12 |
| 2012-14 | +0.09 / −0.03 | −0.11 / −0.23 |
| 2015-18 | +0.03 / −0.13 | +0.02 / −0.13 |
| 2019-22 | **+0.21 / +0.10** | **+0.22 / +0.11** |
| 2023-25 | −0.10 / −0.20 | +0.31 / +0.22 |

## Reading

1. **The sequence has some information.** The setup's timing beats random entries with the same
   stop, target and costs by +0.1…+0.35 R (t 1.0-2.7). The raid → MSS → FVG order is not noise.
   This is more than the single zones in `smc_long.py`, which were inside the random range.
2. **But it is not an edge after costs over the full history.** It was flat to negative in
   2008-18 and positive only in 2019-22. In 2019-22 both longs and shorts earned (+0.1 R net each),
   so that was not just the bull market. In 2023-25 only longs earned.
3. **Cost is the wall.** Median risk is $2-4/oz, so $0.25 + slippage is 0.1 R per trade, about
   the whole gross edge. With ≤ $0.10 all-in cost, the 2015-25 results would be clearly positive.
   2008-14 would still be flat.
4. A trader who trades this model by hand is doing what is coded here, plus judgement. The model
   is **regime-dependent**: it worked in 2019-22 and did not in 2008-18. That matches traders'
   reports that it "works" (2020-2022 is when ICT became popular) and the silence about earlier
   years.

Candidates worth keeping on the board: **draw20 · NY · MSS 1m · CE · liquidity target** (19
trades a year). It is regime-dependent and needs execution cost ≤ $0.10/oz; paper trading would
show whether 2023+ behaves like 2019-22.
