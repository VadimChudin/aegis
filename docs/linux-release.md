# Linux release verification — AEGIS 0.6.1

Verified on 2026-10-04 in Ubuntu 24.04 x86-64. Application source is the published
v0.6.1 commit `bd7257b`; this change adds Linux packaging/documentation, not AI
trading. Existing Windows and macOS release assets remain unchanged.

## Install

Download `AEGIS_0.6.1_amd64.deb` from the v0.6.1 release and run:

```sh
sudo apt install ./AEGIS_0.6.1_amd64.deb
aegis
```

The package requires a graphical session, GTK 3 and WebKitGTK 4.1. Its declared
dependencies include `libgtk-3-0` and `libwebkit2gtk-4.1-0`. The package was built
on Ubuntu 24.04; compatibility with older distributions is not verified.

## Confirmed checks

- `cargo fmt --all --check` passed.
- `cargo clippy --workspace --all-targets -- -D warnings` passed.
- `cargo test --workspace`: 78 tests passed, none failed.
- Python tests with research extras installed: 11 tests passed, none skipped.
- `npm ci` and `npm run build -- --bundles deb` completed; the `.deb` contains the
  desktop executable and packaged MT5 bridge resource.
- Installed the actual `.deb` with apt; dpkg reported `aegis 0.6.1` installed.
- Launched `/usr/bin/aegis` under Xvfb/Openbox with software rendering. This is
  the packaged native Tauri application, not just an HTTP preview of its HTML.
- Connected to the repository's local Binance mock with mock credentials;
  candles rendered and the price updated. Changed 15m to 5m; the mock received
  5m candle requests and the legend showed 5m.
- Opened the structural strategy settings panel. After restarting the installed
  application, mock auto-connect restored the chart and the saved 5m timeframe.
- The density window displayed sampled mock-book densities. These are synthetic
  test data, not an observation of a real exchange or trading results.

No real orders were submitted, no broker credentials were used, and no cloud
model API key was sent. Saving a custom preset through desktop automation was
not confirmed in this run; do not infer that from the settings-panel check.

## Display workaround used in the headless test environment

The initial accelerated-rendering launch produced EGL/DRI3 warnings. The
verified launch used `LIBGL_ALWAYS_SOFTWARE=1` and
`WEBKIT_DISABLE_DMABUF_RENDERER=1` in this sandbox only. Those switches are not
forced on users or embedded in the package. On a normal supported graphical
desktop use the standard launcher first.

## Important scope limits

The Linux release has the existing chart, broker checks, screener and research
backtests. It does not implement Qwen, OpenRouter/Anthropic, autonomous AI Paper
or live AI trading. The user-supplied test key was not used because this version
has no AI provider/client to test. Refer to `ai-trading-status.md` for missing AI
functionality.

The official MetaTrader5 Python integration is Windows-only. Packaging the
bridge script does not make RoboForex MT5 execution available on Linux. MT5
unit/integration checks use the repository's fake terminal; they are not a test
of the real Windows terminal.

The static browser preview, when used, has no Tauri IPC backend and is not a
replacement for the packaged desktop verification above. This release was not
tested on a physical GPU, Wayland desktop, or non-Ubuntu distribution.
