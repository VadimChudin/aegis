# AEGIS 0.7.0-beta.3: execution preview and handoff

## What changes

Settings → AI contains the AI switch, RoboForex broker selection, Paper/Money mode, model
installation/readiness, OpenRouter key and cycle diagnostics. The rail keeps five strategies:
Bounce, Structural reversal, Liquidity Sweep, DATA and Breakout. With AI enabled they open SPA
(System Prompt Algorithm) editors rather than the ordinary research panels. SPA is JSON with
`objective`, `entry_rules`, `exit_rules`, and `timeframes`. Editing stops new entries first;
position protection is not removed. Existing free-text settings migrate into this JSON form.

Each strategy has its own risk percent and daily-loss limit. Live position count is bounded;
the selected strategy's count gate limits total XAUUSD exposure, and its daily-loss gate
checks symbol-wide realised/floating loss rather than a separate strategy sub-account.
Paper currently has one position total, so its position-count control is inactive. Adherence
0–100 influences model interpretation; 100 also checks canonical conditions mechanically.
No slider can guarantee exact natural-language adherence or future trading accuracy. A valid
DATA answer is allowed to wait: statistical edge must never be fabricated from a small sample.

## Execution boundaries

- Paper is default. Money needs an explicit confirmation every session, bound to MT5 login
  and server. Restart, Stop or configuration changes disarm new entries. Account changes stop
  automation. Only XAUUSD on RoboForex/MT5 and USD accounts are supported for new live entries.
- Qwen and `anthropic/claude-fable-5` must agree on action, snapshot, position, stop, target and
  reduction fraction. Missing keys, disagreement, malformed/truncated responses, old data or
  a deadline miss mean no action. Model inference does not block market/account collection.
- OpenRouter key is entered in Settings; never commit or send it in chat. There is **no app
  spending limit** in this mode. Automatic analysis and opt-in diagnostic calls are billable.
  Requests have fixed endpoint/model, size limits, one in-flight cycle and timeouts, not a
  financial budget. Connectivity tests do not send broker orders or prove profitability.
- Live orders use broker lot steps/minimums/contract size, loss-side stops, permission and
  spread checks, order_check, notional/risk limits, then a single order_send. Broker results
  are classified filled/partial/rejected/unknown; an unknown outcome blocks new entries,
  never creates an automatic retry. Close/close-all acts only on owned XAUUSD positions
  (`magic=26070552`), never arbitrary account positions.
- Market close buttons stop new decisions and await execution reconciliation. They request
  market execution; real network/broker delays cannot be made instantaneous or guaranteed.
  Partial/rejected/unknown results remain visible and require broker-side inspection.
- SL/TP reside at the broker after live entry. A model can tighten a stop or reduce/close a
  position, but cannot widen a stop. Paper fills remain estimates from sampled bid/ask,
  commissions and slippage. No martingale, unlimited grids or arbitrary external-position
  management is provided by this preview.
- Balance, equity, free margin and floating P/L come from MT5, not the simulator. Entry,
  stop and target lines reflect the latest returned owned broker positions; stale/disconnected
  state is not proof of an executed close. Paper and Money values must not be conflated.

## Persistence and recovery

`settings.key` is a random local installation key stored beside encrypted `settings.json`.
On Unix it is owner-only; Windows relies on the user's config-directory ACL inheritance.
It is not an OS keychain. Back up both files securely together; losing/corrupting the key
blocks credential saves rather than overwriting secrets. Legacy machine-derived credentials
migrate when decryptable. Validate Windows ACLs before client deployment.

AI config/journal lives in the application config directory under `observer/`. MT5 intent
ledger lives under `~/.aegis/trade-ledger/`, or `AEGIS_TRADE_LEDGER_DIR` when explicitly set.
Ledgers are account-scoped and locked across processes. Do not erase an unresolved intent to
make trading resume: first reconcile actual orders/deals/positions in MT5. Corrupt or ambiguous
records fail closed; emergency closing of known owned positions remains available. The JSONL
journal is bounded to 32 MiB and requires export/retention management. It contains account
identifiers and market/decision data, not passwords or API keys; treat it as private.

## Required acceptance before client use

The implementation is verified with unit/integration tests and a fake MT5 terminal; those are
NOT proof of live RoboForex execution. A real Windows MT5 session and authorized OpenRouter key
are unavailable in the agent sandbox. The owner must verify on a Windows **demo** account:

1. Correct account identity, XAUUSD contract size/lot step and account currency; terminal trading
   permissions and the API key/model work, with cycle diagnostics showing each latency.
2. Small entry with attached SL/TP, lot sizing and returned broker ticket; verify prices and
   volume in MT5, including stop/freeze distances, netting/hedging and filling policy.
3. Close one/all, partial execution and stop tightening; external trades are untouched.
4. Restart/disconnect, account switching and ambiguous sends stop new entries without repeats.
5. Match dynamic account P/L and chart positions to MT5. Run a multi-hour demo soak on the
   actual GPU/network: bounded buffers alone do not prove hours-long stability or latency.

This blocks **client deployment / real-money acceptance**, not merging the preview code.
Never describe the preview as certified live trading or guaranteed profitable. No model-weight
training is included. Previous beta.2 remains available as the observation-only rollback;
preserve the new settings key and broker-side protective orders during rollback.
Beta.2 cannot decrypt settings re-encrypted with the new random key: use a pre-upgrade backup
or enter credentials again in an isolated config directory. Never erase trade-intent ledgers
as part of rollback or retry.

## Verification for this build

- 141 Rust workspace tests, 42 Python tests without skips, Clippy/fmt and JS syntax passed.
- MT5 fake-terminal integration exercised entry, lot/risk sizing, attached stops, tightening,
  reduction, close, close-all, partial/rejected/unknown responses and no blind retry.
- OpenRouter transport tests used a local HTTP fixture: fixed model, auth headers, bounded
  responses, JSON parsing, truncation/error rejection and redacted failures. No paid key was used.
- Native Tauri and installed `.deb`: five original strategies, every SPA editor, risk/day-loss
  persistence, timeframe synchronization, dynamic balance/equity and owned position lines.
  One/all close buttons removed owned positions while preserving an external test position.
- Readiness diagnostics got an actual Qwen response, measured MT5 latency and reported the
  missing provider key honestly; no diagnostic order was sent. Restart kept SPA/mode but did
  not re-arm Money or start trading.
- Multi-hour soak, physical GPU, real Fable call and real Windows MT5 execution remain unverified.
