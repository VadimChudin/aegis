# AEGIS: AI trading continuation audit

Verified against GitHub and fetched repository refs on 2026-10-04. The attached
conversation describes intended functionality, not evidence of completed code.

## Repository and release state

- Repository: `VadimChudin/aegis`.
- `origin/main`: `ab651a9`, version 0.5.1.
- Latest published release: [v0.6.1](https://github.com/VadimChudin/aegis/releases/tag/v0.6.1).
- Release source: `bd7257b`, also `origin/hoplite/structural-setup-release`.
- [PR #48](https://github.com/VadimChudin/aegis/pull/48) remains open against main.
  GitHub reported passing test and Windows/macOS build checks during this audit.
- Release assets exist: `AEGIS_0.6.1_x64-setup.exe` and
  `AEGIS_0.6.1_universal.dmg`. They are not AI-trading installers. This audit did
  not download, install or execute those binaries.
- `origin/hoplite/gold-lab-app` contains saved work at `816d540`: historical-data
  support and Asia/FOMC backtest/search. It is based on 0.5.1, not on the latest
  release. Do not replace the release tree with this branch or silently discard
  these independent changes.
- [PR #47](https://github.com/VadimChudin/aegis/pull/47) contains separate feed/MT5
  reconnection work and remains open. Review before integrating it.

The current thread checkout is unchanged apart from this audit document. No
branches were merged, no orders were submitted, and no new release was created.

## Implemented at v0.6.1

- Broker connection/checks for Binance, Bybit and RoboForex through local MT5.
- Broker-specific candles/chart and encrypted saved broker credentials.
- Density monitoring/recording; MT5 DOM is unavailable if the symbol does not
  provide it, rather than replaced with another market's book.
- Bounce configuration, simulation/backtest, genetic search and diagnostics.
- Experimental structural-reversal backtest and saved custom presets.
- Existing Windows/macOS packaging and release workflow.

## Missing from the attached AI requirements

| Requirement | Verified status |
| --- | --- |
| Local Qwen3-8B agent | No model runtime, connector or agent loop found |
| Anthropic via OpenRouter | No client, provider settings or connection test found |
| Per-strategy AI preprompts | Not implemented |
| Numeric market snapshots, event-driven plans and plan expiry | No AI planning protocol/loop found |
| AI Paper position management | No autonomous AI Paper executor found; existing strategy backtests are not that feature |
| MT5 open/reduce/close/change stop | Not implemented in the checked bridge protocol |
| Manual/Auto independent of Paper/Money, visible stop-auto | No complete trading-mode state machine found |
| AI-only/manual-position permissions | Not implemented |
| User stop/risk/leverage/position-count/daily-loss limits | No enforced live AI risk boundary found; backtest parameters do not implement this |
| API daily/monthly budgets, reservation and actual cost accounting | Not implemented |
| Decision/execution journal and post-trade memory | No AI decision/execution journal found; density JSONL is a different dataset |
| Scheduled AI reports and email delivery | Not implemented |
| End-to-end AI installer/release verification | No such release exists |

## Code evidence

Paths below refer to the release tree at `bd7257b`/`v0.6.1`:

- `crates/aegis-core/src/strategy.rs`: Bounce and Structural have backtest status;
  Breakout, Liquidity Sweep and DATA remain stubs. The live-signal test verifies
  that all strategy slots return no signal.
- `crates/aegis-core/src/settings.rs`: broker credentials/preferences and strategy
  JSON settings exist; no AI provider/model/budget settings.
- `python/aegis_lab/bridges/mt5_bridge.py`: request dispatch supports `hello`,
  `connect`, `candles`, `order_book`, `shutdown`. There is no trade request path.
- `app/src-tauri/src/main.rs`: connection/chart/density/backtest commands, not
  autonomous AI or trade-execution commands.
- `.github/workflows/release.yml`: tag-based build/publication. Successful
  packaging is not a runtime test of AI trading or of the broker terminal.

Searches across the release and inspected saved branches found no Qwen,
OpenRouter, Ollama or `order_send` implementation. Tests were not rerun locally
during this read-only code audit; CI results were retrieved from GitHub.

## Agreed target retained from the conversation

RoboForex MT5 is the first execution venue. Qwen3-8B is a candidate local agent,
with Anthropic accessed through OpenRouter for escalation. Observation uses
structured numbers rather than screenshots. Models select entry, sizing, stops
and exits within user-defined hard limits; execution code validates permissions,
freshness and technical correctness rather than inventing a different strategy.

Local model speed and GPU fit are unverified. The user stated 8 GB VRAM and
16 GB RAM, but exact GPU details are not available. Financial-model naming and
backtest results are not proof of future profitability.

## Implementation order and acceptance gates

1. **Release-compatible base:** integrate the released v0.6.1 tree on the existing
   task branch without dropping independent saved work; keep trading disabled.
2. **AI contracts/settings/prompts:** provider endpoints, encrypted secret
   storage, strategy-specific prompts, strict bounded JSON actions and freshness
   checks. Unknown fields/metrics/actions fail closed. No broker secrets in model
   context. User-entered endpoints must not receive unrelated stored credentials.
3. **Local/OpenRouter observation:** compact, broker-specific snapshots, explicit
   unavailable data, bounded context, one in-flight request, cooldown and cost
   reservation/accounting. Hardware performance must be measured, not promised.
4. **Autonomous Paper:** positions, model-selected protective stops, partial
   exits, deterministic risk checks, journal and emergency halt. Test stale
   decisions, invalid JSON, duplicated actions, budget exhaustion and restart.
5. **MT5 execution:** strict symbol/volume/tick/stops-level validation, idempotent
   order tracking, ownership of positions, broker reconciliation and server-side
   protective stops. Verify on an explicitly authorised demo account before live
   money. No claim of live readiness without terminal-level evidence.
6. **Memory/reporting:** bounded factual trade reviews, relevant notes that cannot
   override risk permissions; in-app reports and separately configured email.
7. **Installer/release:** full tests and UI interaction evidence, Windows build,
   fresh installer smoke test on supported Windows, model/MT5 prerequisite checks,
   then publication. Money mode stays unavailable until its execution and recovery
   gates pass. Installing software is not permission to submit real trades.

This document records the continuation boundary. It does not implement or enable
any AI, Paper or Money functionality.
