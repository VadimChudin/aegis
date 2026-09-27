"""Bounce research: which touch metrics separate bounces from breaks.

Input: the touches CSV written by the Rust engine
    cargo run --release -p aegis-core --example bounce -- touches bars.csv touches.csv
(metrics as of the bar before the touch, i.e. what a resting limit order at the level can know;
`r_<tp>` = gross trade result in R with the stop 0.5 ATR beyond the level).

Writes:
  crates/aegis-core/src/bounce/research.json   per-metric quintile win rates shown next to sliders
  a text report on stdout (univariate lift, walk-forward model check)

    python -m aegis_lab.research.bounce_analysis ~/aegis-data/touches.csv
"""

from __future__ import annotations

import json
import pathlib
import sys

import numpy as np
import pandas as pd

LAUNCH = 1767571200  # XAUUSDT TradFi perpetual listed 2026-01-05
LABEL = "r_1"  # take profit 1R
ROOT = pathlib.Path(__file__).resolve().parents[3]
SKIP = {"time", "level", "close", "fill", "risk", "atr"}


def quintiles(t: pd.DataFrame, y: pd.Series, col: str) -> dict | None:
    x = t[col]
    if x.notna().sum() < 1000 or x.nunique() < 2:
        return None
    if x.nunique() <= 7:
        groups = x
    else:
        groups = pd.qcut(x.rank(method="first"), 5, labels=False)
    g = pd.DataFrame({"x": x, "y": y, "grp": groups}).dropna().groupby("grp")
    bins = [
        {"lo": float(v.x.min()), "hi": float(v.x.max()), "win": float(v.y.mean()), "n": int(len(v))}
        for _, v in g
    ]
    wins = [b["win"] for b in bins]
    return {"bins": bins, "spread": max(wins) - min(wins)}


def main() -> None:
    path = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path.home() / "aegis-data/touches.csv")
    t = pd.read_csv(path)
    t = t[t.time >= LAUNCH].reset_index(drop=True)
    t["atr_usd"] = t["atr_usd"] if "atr_usd" in t else t.atr
    y = (t[LABEL] > 0).astype(float)
    feats = [c for c in t.columns if c not in SKIP and not c.startswith(("bounce_", "mfe", "mae", "r_"))]
    out = {"label": "win rate, limit at level, stop 0.5 ATR beyond, target 1R, no costs",
           "base": float(y.mean()), "touches": int(len(t)), "metrics": {}}
    for c in feats:
        q = quintiles(t, y, c)
        if q:
            out["metrics"][c] = q
    dest = ROOT / "crates/aegis-core/src/bounce/research.json"
    dest.write_text(json.dumps(out, indent=1))
    print(f"touches {len(t)}  base win {y.mean():.3f}  -> {dest}")
    for c, q in sorted(out["metrics"].items(), key=lambda kv: -kv[1]["spread"]):
        cells = "  ".join(f"{b['win']:.2f}@{b['lo']:.3g}..{b['hi']:.3g}" for b in q["bins"])
        print(f"{c:15s} {q['spread']:.3f}  {cells}")


if __name__ == "__main__":
    main()
