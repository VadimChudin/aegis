# Paper start: timestamp diagnostics

## Verified status
The latest two-AI audit commit dde92d3dba748a546ffd37f3d5ae99447e9ac935 passed full GitHub CI, including workspace Clippy/tests with native Tauri libraries:
https://github.com/VadimChudin/aegis/actions/runs/37761735544
This is build/test evidence, not native-button E2E or live brokerage acceptance.

## Reported symptom
The user reported a Paper start error mentioning "2 seconds". No exact text, screenshot or local logs were provided. Source contains the matching message "timestamp is more than 2 seconds in the future". This is a plausible match, not a confirmed diagnosis of their computer.

observer_start reads connector.market_sample and validates it against app Unix milliseconds BEFORE checking local model readiness or consulting cloud. Thus this particular error, if confirmed, blocks before either AI performs analysis.

## Changes
Future/stale timestamp errors now include:
- source label: observation, quote, ticks[index], or book;
- actual Unix timestamp_ms and app_now_ms;
- ahead_ms or age_ms;
- exact allowed boundary;
- practical checks for clock synchronization/MT5 reconnection or stale market data.

Limits remain unchanged: future <=2000 ms and fresh sample age <=10000 ms. Historical ticks need not be fresh but may not be future-dated. No broker timestamp is replaced with receipt time, no timezone offset is inferred from stale ticks, and no trading mode is enabled by the diagnostics.

## Local verification
cargo test -p aegis-core: 143 unit + 6 bounce integration + 3 venue integration = 152 passing tests; 0 failures. Clippy core all-targets -D warnings, workspace formatting and git diff --check passed. Added explicit exact-boundary and diagnostic-content tests, including rejection at 2001/10001 ms.

## Still required from the affected computer
Exact error text including source label and diagnostic milliseconds, installed AEGIS version, OS, broker/symbol. A simple timezone display change does not correct Unix time. Synchronization may help if computer time is wrong, but source clocks/tick timestamps need investigation if divergence persists. Do not relax freshness protections or enable Money to test.

These diagnostics change source in PR #55 only; existing installers are not automatically updated. Full successful production two-AI chain, native install button, CPU performance remediation and repeated reduction intent identity remain separate outstanding items.
