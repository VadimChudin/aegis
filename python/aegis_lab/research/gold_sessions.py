"""Session effects on gold at 1-minute resolution, 2008-2025, and Power of 3 before 2019.

  asia    long 18:05 -> 02:00 New York, Mon-Thu evenings. Opened after the 17:00 rollover and
          closed before the next one, so a CFD pays no swap.
  po3     smc_models.md rule: at 08:30 NY go with the side of the midnight open, stop at the
          day's extreme so far, exit 16:00; reported in R like the original, now from 2008.
  hours   mean return of every New York hour by period.
Cost: $0.25/oz per round trip (tight ECN). Data: XAUUSD CFD 1m (Hugging Face, server = NY + 7 h).

    python -m aegis_lab.research.gold_sessions [--data DIR]
"""

from __future__ import annotations

import argparse
import pathlib
import urllib.request

import numpy as np
import pandas as pd

from .gold_lab import HF

COST = 0.25
PERIODS = (("2008-14", slice("2008", "2014")), ("2015-19", slice("2015", "2019")), ("2020-25", slice("2020", "2025")))


def minutes(data: pathlib.Path) -> pd.DataFrame:
    cache = data / "xau_1m_2008.parquet"
    if cache.exists():
        return pd.read_parquet(cache)
    import orjson

    raw = data / "XAU_1m_data.jsonl"
    if not raw.exists():
        urllib.request.urlretrieve(HF + raw.name, raw)
    rows = []
    with raw.open("rb") as f:
        for line in f:
            if line[9:13] < b"2008":
                continue
            r = orjson.loads(line)
            rows.append((r["Date"], r["Open"], r["High"], r["Low"], r["Close"]))
    m = pd.DataFrame(rows, columns=["d", "o", "h", "l", "c"])
    m["ny"] = pd.to_datetime(m.d, format="%Y.%m.%d %H:%M") - pd.Timedelta(hours=7)
    m = m.drop(columns="d").drop_duplicates("ny").sort_values("ny").set_index("ny")
    m = m[m.index < "2025-03-01"]
    m.to_parquet(cache)
    return m


def _at(m, hhmm, col="o"):
    hm = m.index.hour * 60 + m.index.minute
    s = m[col][hm == hhmm]
    return s.groupby(s.index.normalize()).first()


def asia(m: pd.DataFrame, enter=18 * 60 + 5, leave=2 * 60, days=(0, 1, 2, 3)) -> pd.Series:
    e, x = _at(m, enter), _at(m, leave)
    x.index = x.index - pd.Timedelta(days=1)
    d = pd.concat([e.rename("e"), x.rename("x")], axis=1, sort=True).dropna()
    d = d[d.index.dayofweek.isin(days)]
    return np.log(d.x / d.e) - COST / d.e


def po3(m: pd.DataFrame) -> pd.DataFrame:
    mid = _at(m, 0)
    day = m[m.index.hour < 16]
    rows = []
    for d, g in day.groupby(day.index.normalize()):
        if d not in mid.index:
            continue
        hm = g.index.hour * 60 + g.index.minute
        pre, post = g[hm < 8 * 60 + 30], g[hm >= 8 * 60 + 30]
        if len(pre) < 60 or len(post) < 60:
            continue
        entry = post.o.iloc[0]
        side = 1 if entry > mid[d] else -1
        stop = pre.l.min() if side == 1 else pre.h.max()
        risk = abs(entry - stop)
        if risk <= 0:
            continue
        hit = (post.l <= stop) if side == 1 else (post.h >= stop)
        out = stop if hit.any() else post.c.iloc[-1]
        rows.append((d, side * (out - entry) / risk, (side * (out - entry) - COST) / risk))
    return pd.DataFrame(rows, columns=["d", "gross_r", "net_r"]).set_index("d")


def _cell(x: pd.Series, scale=1e4, unit="bp") -> str:
    t = x.mean() / x.std() * np.sqrt(len(x)) if len(x) > 1 and x.std() > 0 else 0.0
    return f"{x.mean() * scale:+.2f} {unit} (t {t:+.1f}, n {len(x)})"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default=str(pathlib.Path.home() / "aegis-data" / "hf-xau"))
    a = ap.parse_args()
    data = pathlib.Path(a.data)
    data.mkdir(parents=True, exist_ok=True)
    m = minutes(data)

    print("## Session trades, net of $0.25/oz\n")
    print("| Trade | " + " | ".join(p for p, _ in PERIODS) + " |\n|---|---|---|---|")
    rows = [
        ("Asia 18:05 -> 02:00, Mon-Thu evenings", asia(m)),
        ("Asia 18:05 -> 01:00", asia(m, leave=60)),
        ("Asia 18:05 -> 03:00", asia(m, leave=180)),
        ("Asia 19:00 -> 02:00", asia(m, enter=19 * 60)),
        ("Sunday evening 18:05 -> 02:00", asia(m, days=(6,))),
    ]
    for name, r in rows:
        print(f"| {name} | " + " | ".join(_cell(r.loc[s]) for _, s in PERIODS) + " |")

    print("\n## Power of 3 at 08:30 (net R, $0.25/oz)\n")
    p = po3(m)
    print("| Period | Net R |\n|---|---|")
    for lo, hi in (("2008", "2011"), ("2012", "2015"), ("2016", "2018"), ("2019", "2022"), ("2023", "2025")):
        print(f"| {lo}-{hi} | {_cell(p.net_r.loc[lo:hi], 1, 'R')} |")

    print("\n## Mean return by New York hour, bp\n")
    h = pd.read_parquet(data / "xau_1m_2008.parquet").c.resample("1h").last().dropna()
    r = np.log(h).diff()
    print("| Hour | " + " | ".join(p for p, _ in PERIODS) + " |\n|---|---|---|---|")
    for hour in range(24):
        cells = []
        for _, s in PERIODS:
            x = r.loc[s]
            cells.append(f"{x[x.index.hour == hour].mean() * 1e4:+.2f}")
        print(f"| {hour:02d} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
