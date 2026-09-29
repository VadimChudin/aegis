"""Gold parts bin: the systems in gold_lab.py taken apart, each part tested on the live tiles.

Whole public systems failed (gold_lab.md), but a part of one (its trend filter, its chop filter,
its day of week) may still sharpen a tile that works. Every part is a yes/no flag known at the
tile's decision time. A part is kept only if days with the flag on beat days with it off in BOTH
2008-2014 and 2015-2025 (difference t >= 1.5 in each), or lose in both ("keep (off)" = skip them).

  tiles   asia   long 18:05 -> 02:00 NY, Mon-Thu (gold_sessions.asia), decided at 18:05
          po3    Power of 3 at 08:30 (gold_sessions.po3), decided at 08:30
  parts   trend (EMA200 H1, MACD H4, 60-day momentum), chop (efficiency ratio), volatility regime,
          RSI(2) oversold, last day / New York session direction, turn of month, day of week, FOMC
  events  FOMC statement (14:00 ET, scheduled meetings, federalreserve.gov): pre-drift, the shock,
          continuation after it, each against the same clock window on all other days

    python -m aegis_lab.research.gold_parts [--data DIR]
"""

from __future__ import annotations

import argparse
import datetime as dt
import pathlib
import re

import numpy as np
import pandas as pd

from .gold_lab import ema, load
from .gold_sessions import COST, _at, asia, minutes, po3

HALVES = (("2008-14", slice("2008", "2014")), ("2015-25", slice("2015", "2025")))
PERIODS5 = ("2008-11", "2012-15", "2016-18", "2019-22", "2023-25")
CLOSE = 16 * 60 + 50  # the broker day in this data ends ≈ 16:55 NY in most years
FED_HIST = "https://www.federalreserve.gov/monetarypolicy/fomchistorical{y}.htm"
MONTHS = {m: i for i, m in enumerate(
    ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"], 1)}
# "January 29-30", "July 31-August 1", "April/May 30-1", "Jan/Feb 31-1"
MEETING = re.compile(r"([A-Za-z]+)(?:/([A-Za-z]+))?\s+(\d+)(?:\s*-\s*(?:([A-Za-z]+)\s+)?(\d+))?\s+Meeting\s*-\s*(\d{4})")


# ---------------------------------------------------------------- FOMC dates


def parse_meeting(title: str) -> dt.date | None:
    """Last day of a scheduled meeting from a federalreserve.gov historical-page title."""
    if re.search(r"unscheduled|conference call|cancelled|notation", title, re.I):
        return None
    m = MEETING.match(title.strip())
    if not m:
        return None
    mon = m.group(4) or m.group(2) or m.group(1)
    return dt.date(int(m.group(6)), MONTHS[mon[:3].lower()], int(m.group(5) or m.group(3)))


def fomc_dates(data: pathlib.Path) -> pd.DatetimeIndex:
    """Last day of every scheduled FOMC meeting 2008 … 2025-02 (statement at 14:00 ET)."""
    cache = data / "fomc_2008_2025.csv"
    if cache.exists():
        return pd.DatetimeIndex(pd.to_datetime(pd.read_csv(cache).date))
    from ..data import calendar

    days: set[dt.date] = set()
    for y in range(2008, 2021):  # historical pages; the current calendar page covers 2021+
        s = calendar.get(FED_HIST.format(y=y))
        for title in re.findall(r"<h5[^>]*>([^<]+)</h5>", s):
            d = parse_meeting(calendar.text(title))
            if d:
                days.add(d)
    for ts, ev, _ in calendar.fomc():
        if ev == "FOMC":
            days.add(dt.datetime.fromtimestamp(ts, calendar.ET).date())
    out = sorted(d for d in days if dt.date(2008, 1, 1) <= d < dt.date(2025, 3, 1))
    pd.DataFrame({"date": out}).to_csv(cache, index=False)
    return pd.DatetimeIndex(pd.to_datetime(out))


# ---------------------------------------------------------------- parts


def parts(h: pd.DataFrame, m: pd.DataFrame, fomc: pd.DatetimeIndex, at_hour: int) -> pd.DataFrame:
    """Yes/no flags per calendar day from H1 bars that closed by at_hour:00 NY (no look-ahead)."""
    c = h.close
    h4 = c.resample("4h").last().dropna()
    macd = ema(h4, 16) - ema(h4, 26)
    macd = macd.shift(1).reindex(c.index, method="ffill")  # only completed H4 bars
    er = (c - c.shift(48)).abs() / c.diff().abs().rolling(48).sum()
    bar = pd.DataFrame({"ema": c > ema(c, 200), "macd": macd > 0, "er": er > 0.3})
    bar = bar[bar.index.hour <= at_hour - 1]  # H1 bar opening at_hour-1 closes at at_hour:00
    bar = bar.groupby(bar.index.normalize()).last()
    day = bar.index[bar.index.dayofweek < 5]

    # trading day closes at 17:00 NY; at 18:05 today's close is known, at 08:30 only yesterday's
    dcl = c[c.index.hour == 16]
    dc = dcl.groupby(dcl.index.normalize()).last()
    lag = 0 if at_hour >= 17 else 1
    known = lambda s: s.shift(lag).reindex(day, method="ffill").fillna(False).to_numpy(bool)  # noqa: E731
    lr = np.log(dc).diff()
    vol = lr.rolling(20).std()
    up = dc.diff().clip(lower=0).ewm(alpha=0.5, adjust=False).mean()
    dn = (-dc.diff().clip(upper=0)).ewm(alpha=0.5, adjust=False).mean()
    rsi2 = 100 - 100 / (1 + up / dn)
    ny_o, ny_c = _at(m, 8 * 60, "o").align(_at(m, CLOSE, "c"), join="inner")
    ny = ny_c > ny_o

    month = day.to_period("M")
    one = pd.Series(1, index=day)
    rank = one.groupby(month).cumcount().to_numpy()
    left = one.groupby(month).cumcount(ascending=False).to_numpy()
    before_fomc = fomc - pd.offsets.BDay(1)

    f = pd.DataFrame(index=day)
    f["trend: close > EMA200 H1 (Trident, maker-tung)"] = bar.ema.reindex(day).fillna(False).to_numpy(bool)
    f["trend: MACD(16,26) H4 > 0 (Golden Edge)"] = bar.macd.reindex(day).fillna(False).to_numpy(bool)
    f["trend: 60-day momentum up (TSMOM)"] = known(np.log(dc).diff(60) > 0)
    f["chop: efficiency ratio 48h > 0.3 (Trident)"] = bar.er.reindex(day).fillna(False).to_numpy(bool)
    f["vol: 20-day vol above its 1-year median"] = known(vol > vol.rolling(250).median())
    f["RSI(2) daily < 20 (Connors)"] = known(rsi2 < 20)
    f["last trading day up"] = known(lr > 0)
    f["last New York session up (08:00-17:00)"] = known(ny)
    f["turn of month (last 1 / first 3 days)"] = (rank < 3) | (left < 1)
    for i, name in enumerate(["Mon", "Tue", "Wed", "Thu", "Fri"]):
        f[f"day: {name}"] = day.dayofweek == i
    f["FOMC day"] = day.isin(fomc)
    f["business day before FOMC"] = day.isin(before_fomc)
    return f


def _t(x: pd.Series) -> float:
    return float(x.mean() / x.std() * np.sqrt(len(x))) if len(x) > 2 and x.std() > 0 else 0.0


def _diff_t(a: pd.Series, b: pd.Series) -> float:
    if len(a) < 3 or len(b) < 3:
        return 0.0
    se = np.sqrt(a.var() / len(a) + b.var() / len(b))
    return float((a.mean() - b.mean()) / se) if se > 0 else 0.0


def test_parts(tile: pd.Series, flags: pd.DataFrame, scale: float, unit: str) -> list[str]:
    tile = tile[tile.index.isin(flags.index)]
    fl = flags.reindex(tile.index)
    rows = [f"| Part | on / off {unit}, {HALVES[0][0]} | on / off {unit}, {HALVES[1][0]} | |", "|---|---|---|---|"]
    keep = []
    for name in flags.columns:
        if not fl[name].any():
            continue  # e.g. Friday on a Mon-Thu tile
        cells, ts = [], []
        for _, s in HALVES:
            x, on = tile.loc[s], fl[name].loc[s]
            a, b = x[on], x[~on]
            ts.append(_diff_t(a, b))
            cells.append(f"{a.mean() * scale:+.2f} / {b.mean() * scale:+.2f} (n {len(a)}, Δt {ts[-1]:+.1f})")
        verdict = "keep" if min(ts) >= 1.5 else "keep (off)" if max(ts) <= -1.5 else ""
        if verdict:
            keep.append(f"{name} [{verdict}]")
        rows.append(f"| {name} | " + " | ".join(cells) + f" | {verdict} |")
    return rows + ["", "Kept: " + (", ".join(keep) if keep else "none")]


# ---------------------------------------------------------------- FOMC tiles


def window(m: pd.DataFrame, start: int, end: int, next_day: bool = False) -> pd.Series:
    """log(open at `end` / open at `start`) per day, `end` taken on the next day if asked."""
    a, b = _at(m, start), _at(m, end)
    if next_day:
        b = b.shift(-1)  # next trading day in the index
    d = pd.concat([a.rename("a"), b.rename("b")], axis=1, sort=True).dropna()
    return np.log(d.b / d.a)


def fomc_tiles(m: pd.DataFrame, fomc: pd.DatetimeIndex) -> list[str]:
    px = _at(m, 14 * 60)
    pre = window(m, 14 * 60, 13 * 60 + 55, next_day=True).shift(1).dropna()  # indexed by the later day
    shock = window(m, 14 * 60, 14 * 60 + 30)
    after = window(m, 14 * 60 + 30, CLOSE)
    nxt = window(m, 14 * 60 + 30, 14 * 60, next_day=True)
    sgn = np.sign(shock)
    tiles = [
        ("long pre-FOMC: 14:00 day before -> 13:55", pre),
        ("long the shock 14:00 -> 14:30 (reference)", shock),
        ("follow the 14:00-14:30 move to 16:50", (sgn * after).dropna()),
        ("follow the 14:00-14:30 move to next day 14:00", (sgn * nxt).dropna()),
    ]
    rows = ["| FOMC tile, gross bp | " + " | ".join(f"FOMC {p} | other days {p}" for p, _ in HALVES)
            + " | FOMC net of $0.25, all | FOMC − other days, t |", "|---|---|---|---|---|---|---|"]
    for name, x in tiles:
        x = x[x.index.dayofweek < 5]
        on = pd.Series(x.index.isin(fomc), index=x.index)
        cells = []
        for _, s in HALVES:
            xs, ons = x.loc[s], on.loc[s]
            ev = xs[ons]
            cells += [f"{ev.mean() * 1e4:+.1f} (t {_t(ev):+.1f}, n {len(ev)})", f"{xs[~ons].mean() * 1e4:+.2f}"]
        ev = x[on]
        net = ev - COST / px.reindex(ev.index)
        rows.append(f"| {name} | " + " | ".join(cells) + f" | {net.mean() * 1e4:+.1f} bp (t {_t(net):+.1f}) "
                    f"| {_diff_t(ev, x[~on]):+.1f} |")
    return rows


def pre_fomc_checks(m: pd.DataFrame, fomc: pd.DatetimeIndex) -> list[str]:
    """The pre-FOMC drift: entry variants, sub-periods, and placebo dates shifted by whole weeks."""
    def pre(start: int, prev_day: bool) -> pd.Series:
        if prev_day:
            x = window(m, start, 13 * 60 + 55, next_day=True).shift(1)
        else:
            x = window(m, start, 13 * 60 + 55)
        return x[x.index.dayofweek < 5].dropna()

    px = _at(m, 14 * 60)
    out = ["| Entry → 13:55 on FOMC day, gross bp | " + " | ".join(PERIODS5)
           + " | all, net of $0.25 | years up | FOMC − other days, t |",
           "|---|" + "---|" * (len(PERIODS5) + 3)]
    for name, x in (("14:00 day before", pre(14 * 60, True)), ("18:05 evening before", pre(18 * 60 + 5, True)),
                    ("02:00 on the day", pre(2 * 60, False)), ("08:30 on the day", pre(8 * 60 + 30, False))):
        if name == "18:05 evening before":  # window() pairs 18:05 with the same day's 13:55
            x = window(m, 18 * 60 + 5, 13 * 60 + 55, next_day=True)
            x.index = x.index + pd.offsets.BDay(1)
            x = x[x.index.dayofweek < 5].dropna()
        on = x.index.isin(fomc)
        cells = []
        for p in PERIODS5:
            lo, hi = p.split("-")
            ev = x[on].loc[lo:"20" + hi]
            cells.append(f"{ev.mean() * 1e4:+.1f} (n {len(ev)})")
        ev = x[on]
        net = ev - COST / px.reindex(ev.index)
        yearly = net.groupby(net.index.year).sum()
        out.append(f"| {name} | " + " | ".join(cells) + f" | {net.mean() * 1e4:+.1f} (t {_t(net):+.1f}) "
                   f"| {(yearly > 0).sum()}/{len(yearly)} | {_diff_t(ev, x[~on]):+.1f} |")

    x = pre(14 * 60, True)
    out += ["", "Placebo: the same 14:00 → 13:55 window on dates shifted by whole weeks from each FOMC day", "",
            "| Shift | mean bp | t |", "|---|---|---|"]
    for w in (-3, -2, -1, 0, 1, 2, 3):
        ev = x[x.index.isin(fomc + pd.Timedelta(weeks=w))]
        out.append(f"| {w:+d} weeks | {ev.mean() * 1e4:+.1f} | {_t(ev):+.1f} |")
    return out


def po3_sides(p: pd.DataFrame, m: pd.DataFrame) -> list[str]:
    side = np.sign(_at(m, 8 * 60 + 30) - _at(m, 0)).reindex(p.index)
    rows = ["| Period | long days, net R | short days, net R |", "|---|---|---|"]
    for lo, hi in (("2008", "2011"), ("2012", "2015"), ("2016", "2018"), ("2019", "2022"), ("2023", "2025")):
        x, s = p.net_r.loc[lo:hi], side.loc[lo:hi]
        lg, sh = x[s > 0], x[s < 0]
        rows.append(f"| {lo}-{hi} | {lg.mean():+.3f} (t {_t(lg):+.1f}, n {len(lg)}) | "
                    f"{sh.mean():+.3f} (t {_t(sh):+.1f}, n {len(sh)}) |")
    return rows


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default=str(pathlib.Path.home() / "aegis-data" / "hf-xau"))
    data = pathlib.Path(ap.parse_args().data)
    data.mkdir(parents=True, exist_ok=True)
    m = minutes(data)
    h = load(data)
    fomc = fomc_dates(data)
    per_year = pd.Series(1, index=fomc).groupby(fomc.year).sum().to_dict()
    print(f"FOMC statements: {len(fomc)}, per year {per_year}\n")

    print("## Parts on the Asian tile (net bp per night, decided 18:05)\n")
    print("\n".join(test_parts(asia(m), parts(h, m, fomc, 18), 1e4, "bp")))

    p = po3(m)
    print("\n## Parts on Power of 3 (net R per day, decided 08:30)\n")
    print("\n".join(test_parts(p.net_r, parts(h, m, fomc, 8), 1, "R")))
    print("\n## Power of 3 by side\n")
    print("\n".join(po3_sides(p, m)))

    print("\n## FOMC tiles\n")
    print("\n".join(fomc_tiles(m, fomc)))
    print("\n## Pre-FOMC drift: robustness\n")
    print("\n".join(pre_fomc_checks(m, fomc)))


if __name__ == "__main__":
    main()
