#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Linux ]]; then
  exit 0
fi
if ! pkg-config --exists gtk+-3.0 webkit2gtk-4.1 ayatana-appindicator3-0.1 librsvg-2.0 openssl || ! command -v Xvfb >/dev/null || ! command -v zstd >/dev/null; then
  if [[ "$EUID" == 0 ]]; then
    APT=(apt-get)
  else
    sudo -n true
    APT=(sudo -n apt-get)
  fi
  "${APT[@]}" update -qq
  "${APT[@]}" install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev xvfb xdotool python3 zstd
fi
