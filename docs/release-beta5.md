# AEGIS 0.7.0-beta.5

Experimental prerelease, not a stable release and not acceptance for real-money trading.
Paper is the default; Money remains off unless explicitly armed for the current session.

## Changes since beta.4

- Local AI startup/recovery and bounded decision handling reliability fixes from the audit branch.
- Opt-in Paper event telemetry and score gate, with bounded raw event samples, practical
  hour windows, source-change invalidation and outcome tracking. CFD quote/tick volume is
  not exchange tape or proof of genuine resting liquidity.
- Permanent Paper event archive, cumulative partial-exit PnL and refreshed weekly reports
  from archived history rather than only the bounded live buffer.
- Release checks include matching workspace/core/app versions, locked Rust dependencies,
  Node regressions and Python bridge tests on Linux, Windows and macOS.

## Verification and limitations

- Fixture/regression logs are not proof of live broker behavior. Real local Qwen CPU tests
  can hit the decision deadline; a timeout must remain a non-trading result, not be presented
  as a successful market decision. See the audit and verification documents for exact evidence.
- The published beta.4 installers contain the old code: checking updated sources does not
  upgrade an existing installation. Only verified beta.5 assets contain these changes.
- Windows MT5 demo acceptance, installed desktop smoke tests, broker history availability,
  and real-money acceptance are separate checks. No profitability or live-money safety claim.
- Optional Python research-extra tests may be skipped when their dependencies are absent;
  Linux-only runtime tests are expected to skip on Windows/macOS.

## Distribution and release gate

Expected installers: Windows NSIS setup `.exe`, Linux `.deb`, and macOS universal `.dmg`
(Apple Silicon + Intel). Packaging on three runners is not an installation smoke test.
The macOS bundle uses the existing ad-hoc signing setting; notarization is not claimed.

All three platform test jobs must pass before packaging starts. Each packaging job reruns
Rust, Python and Node tests before the Tauri action. All three Tauri jobs upload to the same
draft release. Only a final job depending on the entire successful build matrix may publish:
it checks that the tag matches the tested checkout and nonempty, uploaded `.exe`, `.deb`,
and `.dmg` assets exist, then publishes beta.5 with `prerelease: true`. A failed matrix or
missing asset leaves the release draft; there is no publicly partial installer release.
Matching branch/tag runs are serialized and an already-published release is rejected before
building, preventing accidental re-upload. Installer checks verify existence/upload state,
not installation, signing, notarization or runtime behavior.

Maintainer procedure: merge the point-6 implementation and this release patch into the audited
branch, run CI, and only after green checks create `release/0.7.0-beta.5` at that exact merged
commit. Its push runs the existing release workflow; Tauri creates `v0.7.0-beta.5` with the
workflow's `GITHUB_TOKEN`. No raw credential or local git push is required. Normal `v*` tag
releases and manual builds are retained. Do not create the release branch before point 6 is
merged and verified. Existing tags are never moved by this patch.

## Point 6 implemented: bounded telemetry tools

- Paper event analysis can select up to two typed read-only tools: `query_tick_slice`
  and `query_book_slice`, constrained to the current source/event and 2/3/10-hour window.
  Candidate strategy/rationale is supplied as data, not executable instructions. A four-second
  selection deadline falls back to a deterministic bounded plan.
- Authorized MT5 tick history uses UTC bounds, at most 2,000 bridge rows (512 per event tool),
  before/after account identity checks, reply size limits and explicit incomplete/truncated flags.
  Missing history falls back to actually captured archive snapshots.
- Observed book snapshots expose levels/imbalance only where recorded; no historical DOM is
  reconstructed. Returned tick features include spread, range, return, observed rate and native
  volume; raw selected rows are bounded. Distinct same-millisecond ticks remain distinguishable.
- Tool results are included in the local scoring prompt and in the cloud event review.
  No arbitrary website, URL, shell command or account change can be requested by the model.
- Local verification: 187 Rust core/integration tests, 83 Python tests, 28 Node tests, strict core
  Clippy and formatting. Full desktop and three-platform checks run in the release workflow.

Limitations: queries are bounded samples, not a complete exchange tape. MT5 source availability,
real Windows/GPU performance and installed UI acceptance still require demo testing. CFD DOM
and tick volume must not be interpreted as the complete global gold order book. Real-money
permission is unchanged; the new event tools are Paper-only. The existing 32 MiB observer journal
limit and weekly auto-tuning limitations remain documented.
