# Beta.5 telemetry tools — acceptance gates

Base audit/local-ai-startup a27ccc8. Point6: actual bounded access to observed tick/book fragments, not arbitrary website browsing.

- Authorized MT5 history_ticks command UTC start_ms/end_ms positive integers; <=10h window, end<=host now+2sec, max1..2000 ticks; requesting count+1 avoids unbounded copy_ticks_range allocation. Full history is never claimed by cap-limited response. Account/server verified before and after source call; exact connected symbol required. Source identity is not model supplied authorization.
- Rust tools enforce event/source/window binding, output rows/bytes, tool call counts/deadlines and deterministic whitelist. No shell/arbitrary URL/network credentials exposed to model. Tick/book slices explicit provenance, timeframe, volume kind, missing/truncated history.
- Historical tick API uses real MT5 when supported; missing API/errors/coverage recorded. Historic DOM only from captured immutable archive, never fabricated from candles/current book. Quote time and receipt time remain separate.
- Selected bounded fragments/features must actually reach local scoring and cloud review. score8 boundary and Paper-only permission remain unchanged. Position protection independent of model tool failures.
- Model-selectable slice stage is finite and JSON-validated, host tools return data not instructions. Context budget accounts for fragments and prompts; oversized evidence fails clearly rather than silently spending tokens or enabling trade.
- Tests cover source mix, future/outside range, row caps, missing/degraded inputs, observed book features, actual payload wiring, account change during source read and incomplete response rejection. Replay is not Windows/GPU/demo proof.

Release:0.7.0-beta.5 prerelease, no main merge. Build WindowsNSIS/LinuxDEB/macOSuniversalDMG. Publish only when all platformjobs passed and nonempty three installers exist. Include permanent Paper archive and cumulativepartialP/L improvements fromPR#55. No real-money arming. Authentic Windows MT5 demo/native UI/performance remain external acceptance; archived checksum/bundle contents verified where possible. Existing observerJournal32MiB limit and missing pastDOM remain documented.
