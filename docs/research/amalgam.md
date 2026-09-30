# AMALGAM migration and regression safeguards

This change replaces the default optimizer behind the historical `ga` API. Existing command IDs remain compatible; the interface identifies AMALGAM explicitly. This is a mixed-variable adaptation of Vrugt & Robinson's multimethod multiobjective search, not a certified reproduction of every original-paper benchmark.

## Implemented design

- Training-only nondominated evolution, crowding/diversity and adaptive offspring allocation.
- Four proposal families: NSGA-II genetic operators, differential evolution, persistent particle swarm and adaptive Metropolis-style proposals.
- A bounded training Pareto candidate set. Validation chooses the final setting; test data must never influence evolution, allocation or final selection.
- Fixed generation budget instead of repeatedly consulting validation to stop the search.
- No default 70% win-rate / 20-trades-per-day requirement. High win rate is not positive expectancy.
- Mixed-variable snapping and constraints; evaluate actual repaired Params rather than treating inactive or equivalent encodings as different financial hypotheses.
- Explicit diagnostics and reproducible seeds. Objectives in R are proxies, not dollar portfolio return or verified live Sharpe.

## Regression gates implemented

The transition must be accompanied by tests for:

1. Full TP before partial TP when the partial threshold is farther away; long and short, gaps and nonzero costs.
2. Entry-fixed position sizing when trades overlap; each flip leg must have independent exposure/cap or be rejected explicitly.
3. Losses attributed to the day they close, including overnight positions.
4. Training fitness cannot include labels whose exits are not known by the training cutoff. Changing only post-cutoff prices must not change already-known training outcomes.
5. Bootstrap zero-trade calendar days; selected DSR and trials evaluated over the same interval; insufficient control samples cannot pass.
6. Deterministic synthetic multiobjective search, nondominance, bounds, adaptive allocation and all proposal families receiving a nonzero opportunity.

## Market smoke protocol

`python scripts/amalgam_market_smoke.py --binary target/release/examples/bounce --out /tmp/aegis-amalgam`

- Data: public Binance XAUUSDT 5m and 1m, June-August 2026. Every monthly archive must match its published SHA-256 CHECKSUM.
- Model off because this short interval cannot support the normal probability-model warmup. Seconds engine off; second-engine fixes require synthetic regression tests separately.
- Assumed costs: maker 2 bp, taker 5 bp, spread $0.02/oz, slippage $0.05/oz. These are assumptions, not measured account execution.
- Seeds 7/21/99; population 16; five generations; train 45 days/test 15 days; same protocol for each seed. This intentionally small budget tests integration and reproducibility, not optimal profitability.
- The period has already been researched: it is NOT a pristine final holdout. A positive result would need new forward data and paper trading, not a profit claim.
- Report all seeds, including losses; never choose a seed after seeing test performance.

## Remaining limitations

Walk-forward OOS admission shares position occupancy and realized-loss ledgers across folds; candidate generation is separate from admission. Flip settlements are split by leg. Simulation keys preserve exact float bits. Selection PBO is bounded to the declared training range.

This migration does not magically produce bid/ask quotes from trade candles, queue fills, market impact, funding/swap charges or a full margin/liquidation engine. OHLC intrabar paths remain an assumption. The FOMC research calendar still needs event-specific historical publication timestamps. PBO remains diagnostic for a restricted candidate pool, not proof against all adaptive multiple testing. Optimized fixed-parameter control comparisons must be labelled inconclusive until fold-matched controls exist.

Paper trading and an untouched forward evaluation remain necessary before real-money use. No guarantee of profitability or freedom from all future bugs is made.

Training drawdown is realized net R ordered by exit; daily R statistics use entry cohorts, not mark-to-market dollar portfolio returns. Money view uses realized-equity sizing and defers partial cash flows until final leg settlement; aggregate portfolio margin and liquidation are not simulated.

## Executed verification (2026-09-30 UTC)

- 91 Rust core unit tests and 7 Rust integration tests passed. Seven Python tests passed.
- Core clippy with warnings denied and workspace rustfmt check passed; both edited JS files passed syntax checks. Full Tauri desktop build/packaging was not run locally.
- Synthetic production-loop benchmark checks a known Pareto set, three seeds and operator contributions. Future-price mutation leaves every AMALGAM training objective unchanged. End-to-end fixed-parameter candidate generation plus shared admission is invariant to splitting the test interval.
- Final market run repeated seed 7: optimizer parameters, windows, Pareto archive, operator stats, baseline and OOS results identical, excluding wall-clock runtime.

| Seed | OOS trades | AMALGAM net mean R | Random net mean R | Engine calls per method |
|---|---:|---:|---:|---:|
| 7 | 1572 | -1.9941 | -2.0999 | 2060 |
| 21 | 1448 | -2.1064 | -2.0663 | 2060 |
| 99 | 1539 | -1.9821 | -2.0088 | 2060 |

All three AMALGAM runs lost after the stated costs. These results do not demonstrate a profitable strategy or optimizer superiority. Do not trade them live. Detailed assumptions, per-operator counts and archive SHA-256 values are in `amalgam_test_results.json`.
