""""There is always a small bounce after a flush": how often, how big, and against what baseline.

    python -m aegis_lab.research.flush_bounce

Drops are triggered as in `flush_report` (no look-ahead) at scales from the 1-minute to the
4-hour chart. From the trigger: the share where the mid rises a·L before it falls b·L more, for
small bounces a and wide stops b, next to the same share from random open seconds with the same
distances, and next to b / (a + b), the answer for a price with no memory. Then the same from the
true low of each leg, which is only known after the fact.
"""

from __future__ import annotations

import numba
import numpy as np
import pandas as pd

from aegis_lab.research.flush_report import _events, load

SCALES = (
    ("0.15% in 10 min (1m chart)", 0.0015, 600),
    ("0.30% in 30 min (5m chart)", 0.003, 1800),
    ("0.50% in 60 min (15m chart)", 0.005, 3600),
    ("1.0% in 4 h (1h chart)", 0.01, 4 * 3600),
    ("2.0% in 24 h (4h chart)", 0.02, 24 * 3600),
)
BOUNCES = (0.1, 0.2, 0.3, 0.5)
STOPS = (0.5, 1.0)
HORIZON = 3 * 86400


@numba.njit(cache=True)
def first_hit(x, i, up, dn, horizon):
    """1 if x reaches x[i] + up before x[i] - dn within the horizon, 0 if the other way, -1 neither."""
    for j in range(i + 1, min(i + horizon, len(x))):
        if x[j] >= x[i] + up:
            return 1
        if x[j] <= x[i] - dn:
            return 0
    return -1


def share(x, idx, legs, a, b) -> tuple[float, int]:
    r = np.array([first_hit(x, int(i), a * L, b * L, HORIZON) for i, L in zip(idx, legs)])
    r = r[r >= 0]
    return (float(r.mean()) if len(r) else np.nan), len(r)


def main() -> None:
    pd.set_option("display.width", 200)
    df = load()
    mid = df.mid.to_numpy()
    is_open = df.open.to_numpy()
    days = len(df) / 86400
    rng = np.random.default_rng(7)
    cand = np.flatnonzero(is_open[: len(mid) - 3600])
    print(f"# Is there always a small bounce? Bybit XAUUSDT mid, {days:.0f} days\n")
    print("Share (%) of drops where price rises a·L before it falls b·L more (L = the leg at the "
          "trigger). *rand* = random open seconds with the same distances; *theory* = b/(a+b), a "
          "price with no memory.\n")
    rows = []
    for name, theta, window in SCALES:
        rmax = pd.Series(mid).rolling(window, min_periods=window // 2).max().to_numpy()
        trig, high, res, out = _events(mid, rmax, is_open, theta, window, HORIZON)
        keep = out >= 0
        i, h = trig[keep], high[keep]
        legs = mid[h] - mid[i]
        ri = rng.choice(cand, min(len(i) * 20, 20000), replace=False)
        rl = rng.choice(legs, len(ri))
        for b in STOPS:
            row = {"scale": name, "drops": len(i), "a week": len(i) / days * 7,
                   "median leg $": float(np.median(legs)), "stop b": b}
            for a in BOUNCES:
                s, _ = share(mid, i, legs, a, b)
                sr, _ = share(mid, ri, rl, a, b)
                row[f"a={a}"] = f"{s * 100:.0f} / {sr * 100:.0f} / {b / (a + b) * 100:.0f}"
            rows.append(row)
    print(pd.DataFrame(rows).to_markdown(index=False, floatfmt=".1f"), "\n")
    print("Cells: drops / rand / theory.\n")

    print("## From the true low of the leg (after the fact)\n")
    rows = []
    for name, theta, window in SCALES:
        rmax = pd.Series(mid).rolling(window, min_periods=window // 2).max().to_numpy()
        trig, high, res, out = _events(mid, rmax, is_open, theta, window, HORIZON)
        keep = out >= 0
        lows = np.array([h + np.argmin(mid[h : j + 1]) for h, j in zip(high[keep], res[keep])])
        legs = mid[high[keep]] - mid[lows]
        bounce = np.array([mid[lo : min(lo + window, len(mid))].max() - mid[lo] for lo in lows]) / legs
        rows.append({"scale": name, "median bounce within the window, share of the leg": np.median(bounce),
                     "bounce ≥ 0.2 L": (bounce >= 0.2).mean() * 100})
    print(pd.DataFrame(rows).to_markdown(index=False, floatfmt=".2f"), "\n")


if __name__ == "__main__":
    main()
