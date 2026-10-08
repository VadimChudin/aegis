# Autonomous event analysis — implementation contract

User choices: RoboForex MT5 / XAUUSD; local score then cloud only for score > 8; telemetry windows 2, 3, 10 HOURS; active event/trade retained until confirmed closure; persist conditions/result and weekly strategy improvement.

## Status
This is a staged implementation, not a declaration of operational autonomous trading. Existing observer and Paper/Money permissions remain active until an explicitly validated replacement is integrated. New modules must not silently switch execution logic or enable real money.

## Event lifecycle
Detect candidate with deterministic strategy checks and source evidence. Create stable event identity including broker/account/symbol/strategy/config version. Forecast earliest/latest time or unknown; model score0..10 is a ranking, not calibrated probability. Collect requested allowed window from bounded time-indexed persistent storage. Empty coverage/unknown historical DOM are explicit missing data. Maintain active pinned evidence on disk; never rely on hours of RAM-only storage. Deduplicate repeated observations. A rejected or expired candidate is recorded too.

Local score <=8 produces no new cloud request or new entry. >8 permits one budgeted detailed cloud review for that event/version, not a guaranteed order. Review returns entry conditions, side, SL/TP, expiry, invalidation and reasoning; deterministic policy validates source identity, fresh quote, sizing and safety. Unknown outcomes and retries never duplicate broker orders. Managing an existing position and emergency close are not disabled merely because its new entry score fell.

## Telemetry
Use authorized MT5 connector, not arbitrary websites called by model. 2/3/10h tick histories depend on available MT5 history. Historical order books cannot be reconstructed from a current snapshot; book history exists only for previously captured snapshots or supported source archives. Enforce sample/byte limits, explicit partial coverage and gaps. At resource exhaustion fail closed to new signals; preserve active trade evidence; never evict it merely to satisfy rolling retention. Capture UTC source time and receipt time separately. No mixing Binance XAUUSDT book with RoboForex XAUUSD.

Raw local archive differs from model prompt: deterministic summaries/time buckets and selected evidence within context budget, never dumping 10h raw ticks into Qwen. Any drill-down tool request is restricted by event/source/window/byte budget and logged. Closed trade archive includes entry/exit timestamps, actual fills, commission/swap/slippage (when known), net P/L, model/provider identifiers, score/rationale and strategy version. Incomplete costs/outcomes stay explicit unknown rather than estimated as actual.

## Weekly review
Weekly report uses finalized trades plus rejected/expired signals separately. Show sample count, coverage, net P/L, drawdown, expectancy, R, costs, latency, score calibration and source/data gaps. UTC windows anchored deterministically. Generate candidate settings versions; do not rewrite history or train model weights implicitly. Validate candidates with chronological out-of-sample/walk-forward checks without look-ahead. Insufficient sample size produces no change. Never automatically increase real-money risk, remove safeguards or enable Money. Apply approved candidate at safe boundary without mutating an open trade's strategy/version; retain rollback.

## UI
Separate purpose (analysis enabled), execution mode (Paper vs real money), current-session authorization and strategy conformance strictness. Strictness is not a trading right and never disables risk rules. Planned observation-only mode requires backend support before exposing it. Show runtime readiness, source coverage, event status, local score, cloud calls/cost and active configuration version. Explain Start failures precisely. Restart/Stop/account change revokes execution authorization.

## Acceptance gates
Unit/property tests for windows, source mismatch, bounds, atomic persistence, crash recovery, pinned eviction, unknown closure, score8 boundary, malformed/incomplete model envelopes, duplicate events, no cloud at <=8, cooldown/budget caps, safe expiry and weekly no-look-ahead. Integration replay follows candidate→sliced telemetry→local score→conditional cloud→policy→Paper outcome→archive→weekly candidate. Real Windows MT5/native UI/GPU/demo acceptance are distinct from fixtures. No release is labelled fully autonomous until that sequence is measured and observed.

## Release integration checklist (not yet complete)
1. Register and test core event telemetry/scoring modules; serialize contracts and deterministic strict gate.
2. Adapter captures authorized MT5 market samples into persistent store and requests available historical ticks; ingest reports source/time/coverage. Historical book gaps are never backfilled synthetically.
3. Deterministic candidate detector and dedup/cooldown bind actual strategy rules to event ID/config version. Forecast from local model only if supported by evidence.
4. Local scoring transport uses strict complete envelope, context/output/deadline limits, and schema. Gate is implemented before any cloud consultation; existing active-position management takes separate route.
5. Cloud tool returns bounded trade-plan schema. Caller validates IDs/expiry/price/risk and source evidence. Token/cost counters and budget caps deduplicate retries.
6. Paper acceptance links event ID→simulated position→confirmed ClosedOutcome including net costs. Live adapter separately maps broker intent/deal IDs; unknown broker outcome remains pinned until reconcile.
7. Weekly background job inside desktop app (not this chat schedule) computes candidate reports at UTC boundaries and catches up after restart. Runtime will not run when the application/host is off. Add optional supervised service separately if 24/7 is required.
8. Native settings expose implemented fields only; explicit execution permission remains disabled on restart/account change. No UI promises observation-only execution until backend supports it.
9. End-to-end replay and Windows MT5 demo soak before enabling feature; no actual broker orders in developer tests.

Core contracts alone do not mean adapters, background collection, local scoring, cloud gate, archived fills and weekly job are wired into observer. Report phase completion precisely.

## Foundation implementation and verification
Added event_scoring and event_telemetry core modules (compiled/exported, NOT wired into active observer/provider loop). Score schema and strict >8 helper have 9 regressions. Durable single-writer telemetry store has 10 regressions: exact source binding, 2/3/10h coverage, unknown historic DOM/cadence, restart recovery, pending evidence retained for weeks, rejection under quota, confirmed-close-only outcomes, rejected/expired signals and UTC weekly summaries. These snapshots are bounded and single-writer; temporary writes require up to twice max_bytes; weekly summaries explicitly cover retained events, not an unlimited complete ledger. No collector, tick-history backfill, model tool executor, weekly scheduler or auto-tuning deployment is enabled by registering modules.

Scoring identities use milliseconds and connector/source/instrument; telemetry uses UTC seconds and server/account/feed identity. Future adapter must convert units explicitly and bind source hashes without sending raw account IDs to cloud. Do not directly splice the two schemas or derive authorization from model-echoed IDs.

UI labels/help clarified in EN/RU/Kazakh; backend rights unchanged and no unimplemented Observation mode displayed. Integrated local verification: 162 core unit +6 bounce integration +3 venue integration =171 passing Rust tests;22 Node tests;75 Python baseline tests; core Clippy all-targets -D warnings, fmt and whitespace pass. No real MT5 account or cloud requests for these modules.

Demo MT5 secrets were supplied via secure form. Their values were not read into audit files or tested against broker. Current Linux sandbox lacks MetaTrader5 and a Windows terminal; authentic broker connection remains blocked. No native MT5 telemetry/Paper end-to-end or real order execution is claimed. Need Windows test host with terminal for that phase; supplied credentials alone do not provide such a host.
