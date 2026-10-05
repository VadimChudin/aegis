# AI Paper beta — 0.7.0-beta.1

This beta adds local-model observation and simulated execution. It is not the
complete autonomous live-trading system discussed in the task. No code path in
this feature sends an order to a real broker. Real MT5 still requires Windows.

## First run on Linux x86-64

1. Install the beta `.deb` with apt and open AEGIS. The package declares Python 3
   and zstd dependencies in addition to the graphical libraries.
2. Open **AI · Paper**, click **Установить / загрузить qwen3:8b**. This explicit
   action downloads Ollama from its official GitHub release into your AEGIS
   config directory and pulls `qwen3:8b`. Allow more than 10 GB free disk space
   for runtime, model and temporary archives; the model alone is about 4.9 GB.
   No sudo, system service or account sign-in is used by the model setup.
3. Progress is shown in the panel. The installed runtime starts when used; a
   process owned by AEGIS is stopped when AEGIS exits. An already-running Ollama
   on localhost can also be used and is not stopped by AEGIS.
4. Test the local model, then connect a supported broker and select it as Paper's
   quote source. Linux supports exchange data; an unavailable MT5 DOM/quote is
   explicitly rejected, not replaced with another venue.
5. Configure strategy, risk, leverage, position count, request interval and API
   budgets; save. Start Paper or request one step. Stop cancels pending decision
   application. Existing Paper positions can be closed manually from the panel.

Runtime installation is Linux x86-64 only. Windows/macOS users must install and
run their local OpenAI-compatible model server separately. Model endpoint is
limited to localhost/loopback and never receives the OpenRouter API key.

## OpenRouter

Enter the key in the application, not chat. AEGIS encrypts it using the existing
machine/user-bound settings mechanism; public settings and snapshots omit it.
This does not protect against software running as the same OS user. Forget-key
removes it. An empty input preserves the existing key.

The connection test uses OpenRouter's authenticated GET `/api/v1/key`, without a
paid model completion. The actual model is configurable and must exist in the
current OpenRouter model catalog. The supplied default is not a guarantee of
availability. A local `consult` action makes at most one cloud decision request;
the cloud is instructed not to recursively consult.

For each cloud call AEGIS fetches model pricing and reserves a conservative cost
for the bounded 65,536-byte request and 1,024-token output. Daily/monthly UTC
budgets and request counts persist across restarts. Failure or missing actual
usage keeps the reservation charged rather than retrying for free. Reported actual
cost replaces the reservation, including an unexpectedly higher bill; further
requests are then blocked if the budget is exceeded. The display therefore
shows **accounted cost/reservation**, not a precise billing statement. Provider
pricing can change; provider-side account spending limits are recommended too.

Sending a cloud consult transmits selected quotes, OHLCV, Paper positions and
recent decision records to OpenRouter/the selected model provider. It does not
send broker credentials, screenshots, a real account balance or real positions.

## Decisions and protection

- Prompts are defined separately for Bounce, Structural, Breakout, Liquidity
  Sweep and DATA. Optional user preprompt augments them, never changes execution
  permissions. These prompts are not proof of profitable strategies.
- Inputs are bid/ask and up to 60 M5 candles from the selected broker. No depth,
  absorption, news or H1/H4 indicators are supplied in this beta. Tick-by-tick
  continuous feature analysis and event-driven waiting plans are not implemented.
- Allowed JSON actions: wait, open, close, reduce, stop, consult. Unexpected
  fields/actions, mismatched snapshot IDs or invalid numbers are rejected.
- Paper quantity means **units of the asset**, not lots or exchange contracts.
  The 10,000 initial balance is fixed in this beta. Spread is included through
  ask/bid fills; commissions, funding, realistic latency, liquidity and margin
  mechanics are not modelled. Paper PnL is not broker PnL.
- A new position requires a loss-side stop and must satisfy aggregate risk,
  leverage, position-count and daily-loss constraints. Stop changes cannot exceed
  the original risk budget or current aggregate limit. Close/reduce remain
  available when risk limits are breached.
- Fresh bid/ask checks and Paper stop marking run independently of model
  requests and candle retrieval. They work only while the app and quote feed are
  running, not as broker/server-side protection when the app is closed.
- Before applying a decision, quotes are fetched again. Decisions older than
  180 seconds or with more than 0.05% quote drift are rejected. Execution uses
  the refreshed quote and recalculates risk. Quotes themselves must be fresh
  within 30 seconds. On CPU the local model can be slow; stale decisions must
  not be mistaken for a technical failure or executed regardless.
- Auto is always off after restart. Structurally valid Paper positions and costs
  are restored; changing risk limits with positions open is blocked. Corrupt
  Paper state blocks decisions and preserves the file instead of overwriting it.

## Journal and remaining work

The Paper journal records accepted actions, stop executions and invalid-decision
rejections. Up to 1,000 records are persisted; the last ten provide bounded
factual context to later decisions. This is not autonomous post-trade review,
long-term learned memory, scheduled reporting or email delivery.

Not implemented: real MT5 orders, management of manual real positions, external
server-side stops, full order reconciliation, event-driven multi-timeframe plans,
scheduled/email reports, model profitability validation or physical-GPU testing.
Money mode must not be enabled based on the Paper demo alone.

## Verification

Unit/integration tests cover strict parsing, loopback restriction, no local-key
forwarding, redirects, risk/leverage, stops, position reduction/restoration,
daily loss, API reservations and secret encryption. Bootstrap tests cover
archive traversal/links and installation staging. A real Qwen3-8B download and
local request were exercised on CPU with explicitly synthetic prices; the model
returned valid wait JSON. The cold request took roughly 90 seconds and the core
freshness guard rejected the old snapshot as expected. This is not performance
evidence for the user's GPU or real trading evidence.

Run a local synthetic check with `cargo run --release -p aegis-core --example
ai_paper` after Ollama is ready. It does not connect to any broker or use a key.
Native UI/installer verification is recorded in the release notes when completed.

The installed Linux beta was also exercised end-to-end with the actual local
Qwen model and a repository mock exchange: a wait decision passed fresh-quote
revalidation and was persisted in the Paper journal, with zero cloud spend.
Native model setup showed ready/100%; stopping an in-flight local request
prevented application of its result. A global **Стоп AI** button remains visible
when the AI panel is closed. These are functional checks, not trading-performance
or real-account evidence. Local tests: 95 Rust tests and 25 Python tests passed.
