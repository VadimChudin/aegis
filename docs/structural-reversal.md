# Structural reversal (experimental)

An independent Strategies panel, like Bounce, with 31 controls, persistent settings and named
presets. No live orders, GA, or automatic claim of profitability. The existing density screener
and Bounce remain separate and unchanged.

## Red preset: Setup 1 + · +0.85R / 5 trades

This is a **retrospective exploratory diagnostic**, April–September 2026, not a validated edge.
The cost/risk gate is OFF and the minimum remaining R/R is zero. The entry was changed after
seeing the limit-retest results, so this is not pristine out-of-sample evidence. The red badge
always describes the historical study; current slider backtest results appear separately.

Frozen settings: previous UTC-day high/low, 0.2% approach from the prior 60 minutes, reclaim
and close beyond the previous five-minute structure within 15 minutes, first qualifying attempt
per level per day, weekdays 07:00–16:00 New York (DST respected), no volume confirmation.
Market proxy at first trade after confirmation close +250 ms; adverse $0.05 slippage. Stop beyond
the raid extreme +$0.50, target 2R from the **original level**, sufficient space to the approach
extreme, max holding 120 minutes/end of UTC day. Maker 0 bp, taker 2.75 bp per side: promotional
assumptions, not measured account costs. Result: +0.854523071874R on five trades (four positive).
The target is not moved outward after entry; the remaining R/R can be much less than 2.

| Date UTC | Side | Entry | Stop | Target | Net R |
|---|---|---|---|---|---|
| 2026-05-21 | short | 4546.51 | 4561.93 | 4543.30 | +0.127089 |
| 2026-07-23 | long | 4087.35 | 4075.84 | 4093.18 | +0.408860 |
| 2026-08-10 | long | 4338.24 | 4330.26 | 4341.48 | +0.256514 |
| 2026-08-11 | short | 4400.77 | 4408.16 | 4400.54 | −0.132640 |
| 2026-08-27 | long | 4592.09 | 4582.50 | 4595.22 | +0.194700 |

## Data and simulation

Select an inclusive date range up to 184 completed UTC days, starting no earlier than the
Bybit XAUUSDT listing on 2026-03-09. Daily public gzip trade archives download once into the app
history cache. The first selected day supplies the previous-day range and has no eligible
signals itself: select one extra day before the desired first trading day. This deliberately
preserves the original six-month study's warmup convention (no March data in that preset).
The files are stored in the app
history cache (`bybit-trades`). Each repeat uses disk and a same-range memory cache. Different
ranges load a new tape. Download errors abort rather than silently omit days. Trade IDs are
deduplicated, positive finite prices/volumes and daily timestamps validated; stable timestamp
sorting preserves archive order at equal timestamps. Several hundred MB RAM may be needed.

Source: `https://public.bybit.com/trading/XAUUSDT/XAUUSDTYYYY-MM-DD.csv.gz`.
`AEGIS_STRUCTURAL_DATA_DIR` optionally points to an already downloaded archive folder for
offline testing. It is not an alternate market feed or a secret. No Python installation needed.

Signal features use completed observed minutes; a missing minute invalidates the approach
window. Levels are from yesterday, never today's final high/low. One attempt is consumed even
when confirmation fails. Overlapping confirmed signals are rejected. Limit retests expire or
cancel when the original stop/target trades first. Target requires trade-through. Gap stops
exit at the actual worse trade with slippage, not a guaranteed −1R. Unfinished positions close
at the last observation and are counted as censored.

Trade prints are NOT executable bid/ask, FIFO, hidden liquidity, margin/liquidation or real
order acknowledgments. Funding is not included, especially material on longer holds. The
volume gate is aggressor dominance, not proof of absorption. Net R is not account return.
The sliders describe a model, not permission to trade live.

Result cards, equity in R, monthly totals, the latest 100 trades and rejection counts update
on Run backtest. Old results are explicitly marked stale after a setting/date change. Applying
the red preset restores all original parameters; named presets preserve custom parameter sets.

## Verification

`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all --check`, and JS syntax checks. Core synthetic tests cover completed-bar
confirmation, costs/direction/reward gates, target-before-limit-fill, adverse gaps and DST.
`cargo run --release -p aegis-core --example structural -- <archive-folder>` reproduces the
five-trade report with the same native engine used by the desktop command.

Before real-money use: new untouched forward observations, measured quotes/fees/funding and
own fills. Five retrospective observations do not establish statistical profitability.
