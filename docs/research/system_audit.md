# Aegis system audit: Bounce and SMC

Scope: bounded audit of core simulation, optimizer, research SMC, density-pyramid replay, desktop input boundary and automated tests. Not an exhaustive certification. Draft PR45 is extended; no automatic main merge and no live orders. Full desktop workspace CI must be verified for the new commit; manual GUI and real broker connections not tested.

## Verified local gates

97 Rust core unit tests +7 integration tests passed; core clippy warnings denied; workspace rustfmt check passed. Python pytest57 passed, including function-style SMC tests that unittest discovery previously missed. Additional14 pyramid inline tests passed. Both edited JS files passed syntax checks. Core research CLI rebuilt and prior three-seed Bounce smoke repeated. Desktop guard tests await full workspace CI because this sandbox lacks Tauri system libraries.

## Corrected bugs

- Candle parser: invalid/nonfinite timestamps, impossible OHLC, negative/infinite volumes and conflicting same-time candles are rejected rather than silently used.
- Core: calibration respects requested limit/close entry basis; zero/negative optimizer windows and oversized budgets cannot hang planning; saturating dates handle no-score sentinel; empty/sparse history validation cannot index invalid bars. Desktop rejects malformed search before the running flag/downloads and rejects negative/nonfinite execution costs.
- SMC: complete HTF bars and confirmed swings only; complete bars are filtered BEFORE ATR; no future ATR backfill; no same-minute backdated target credit; stop-first ambiguity and actual adverse gap-open losses; fees at actual exits; one position globally with competing-order cancellation; true exit timestamps and settled cutoffs; OOS trade count is not a selection eligibility filter. Independent cohorts use fresh zones only, preventing resurrection of zones with discarded warmup lifecycle. Alternate SMC fill-bar gap losses fixed, legacy wrapper verified.
- Pyramid: MAIN removal/reinsertion does not preserve identity; nonfinite FIFO input rejected; optional refill gate requires2 preceding aggressive executions at EXACT MAIN price plus1 observed post-hit size increase within120s. Same-timestamp/future evidence cannot authorize entry. Snapshot identity, invalid arrivals and delayed-exit regressions retained.
- CI installs research dependencies and pytest so new SMC tests actually run; explicit pyramid self-tests added.

## Recent SMC configuration and results

Binance XAUUSDT1m, July1–August15 training, August16–31 validation, September1–15 test. June supplies indicator warmup only; effective evaluation77days.154080 input minutes, full coverage of requested cohorts. New September daily archives verified against official SHA256 checksums. Selection is frozen before test evaluation.

36 configurations: OB/FVG/sweep ×1h/4h ×RR1/2/3 ×stop buffer0.1/0.25ATR. Minimum10train/3validation trades; top3 by train total net R, then highest validation total net R. Selected candidate20: FVG4h, limit at zone edge, RR2, buffer0.1ATR, trend on, premium/discount and killzone off, one position, zone age72h, maximum hold24h. This is a finite grid diagnostic, NOT AMALGAM integrated with SMC and NOT the entire discretionary ICT method.

| Cohort | Settled trades | Total net R | Mean net R | Wins |
|---|---:|---:|---:|---:|
|Train|12|+1.520974|+0.126748|50%|
|Validation|6|+1.665783|+0.277630|50%|
|Test|7|−3.591386|−0.513055|2/7|

Maker2bp/taker5bp, spread0.02USD, slippage0.05USD, trade-through0.01USD. Funding omitted, quote/queue absent. R is diagnostic, not compounded account returns. Train/validation/test cohorts start flat with fresh zones; this is not a continuous carried portfolio. Settled-only/censored cohorts can be selection-biased; censor counts are reported (selected result0). Repeated base benchmark identical. Spread0.20 stress retained same selected candidate and test7trades,total−3.629350R. No positive test advantage demonstrated;7trades insufficient for reliable general conclusion.

## Bounce: two distinct methods

Simple level Bounce, AMALGAM, original Jun–Aug engineering regression: seeds7/21/99 still−1.994098/−2.106399/−1.982074R mean. This previously studied smoke is not fresh independent evidence.

Density-pyramid with causal refill gate, no parameter optimization:
- September1–3 new tape:839602 trades and2008873 book updates. Baseline had1signal, only1order survived post-only arrival, no fills. Required-refill mode had0signals; the only candidate lacked prior qualifying MAIN hits/refill. Zero trades does NOT prove profitability or the absence of an edge. All day integrity flags reviewed.
- Retrospective August17–19 with the same refill gate:2signals,1filled episode0.05oz, gross+0.049850USD, fees0.162960USD, net−0.113110USD at planned1oz per signal. This history was already viewed and is NOT a pristine holdout.
- Pyramid5/15/80, MAIN30%, stop0.10USD and partial50/20/20/10 implemented. $1m/60sec/10×median,120sec hit/refill window, and exit distances0.50/1/1.50USD remain provisional numeric rules, not confirmed discretionary method. Aggregate refill cannot prove individual order identity. Exact-price queue matching is deliberately conservative and may underfill.

## Unresolved and not silently patched

Credential storage key in settings.rs is SHA256 of a public constant+hostname+username, not a secret OS-keystore key. AES authentication alone does not make this robust key custody. A deliberate OS keychain migration with backup/recovery is required; no stored credentials inspected or automatically rewritten. Existing redaction/encryption regression tests do not prove secure key storage.

No full margin/liquidation, funding/swaps, marketimpact or true exchange FIFO. Pyramid immediate cancellation is optimistic; price-size units remain a documented source assumption. Daily R entry-cohort statistics are not mark-to-market returns. SMC24h holding can cross funding. Missing-data/snapshot counters are diagnostics, not proof of archival completeness. Full broker behavior and UI visuals were not exercised. Fixed-value masks/refill filters and short windows do not establish a statistically profitable strategy.

Next: confirm density size, stop0.1 units and partial-exit distances; train a bounded hypothesis on separate historical data, then validate on NEW periods; include account-specific fees, funding and queue/cancellation sensitivity. Do not keep changing rules until the same test window turns positive. No live deployment from these results.
