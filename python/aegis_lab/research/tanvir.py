"""Replication of the TanvirCCC "lnterqo" ICT system (github.com/TanvirCCC/algo-trading) and a
GoldTraderEA-style weighted vote, re-implemented from their descriptions (their repos carry no
licence, so no code is copied).

Rules (5m bars, UTC):
- sessions: London 07-10, New York 12-15; cooldown 6 bars between signals;
- CISD: bullish when the last two confirmed 5m swing lows rise and the bar closes above the last
  swing high (bearish mirrored); swings need 2 bars on each side;
- entry zone ("confidence"): +2 breaker (a broken swing-origin block), +1 inverse FVG, +1 FVG
  near price, +1 Asia range (00-07 UTC) swept on the opposite side today;
- daily bias: last 10 daily bars' structure breaks vs the 20-day equilibrium; trade only with it
  or when neutral;
- stop: beyond the CISD bar's extreme + 0.1 ATR; target: nearest of Asia opposite extreme,
  previous day high/low, older swing extremes with at least `min_rr` R.

Tanvir enters at the CISD bar close and resolves a bar that hits both stop and target by its
colour. Here the entry is the next 1m open (the close is only known after it), exits are
resolved minute by minute (stop first inside one minute), R is net of a $/oz cost, and a stop
smaller than `min_stop_atr` ATR is skipped (a $0.05 stop against a $0.35 cost is not a trade).

    python -m aegis_lab.research.tanvir --data hist|binance [--rw SEED]
"""

from __future__ import annotations

import argparse

import numba as nb
import numpy as np
import pandas as pd

from aegis_lab.research import smc_models as sm


def five_minute(m: pd.DataFrame) -> pd.DataFrame:
    g = m.assign(b=(m.t // 300) * 300).groupby("b")
    return pd.DataFrame({"t": g.t.first().index, "o": g.o.first().values, "h": g.h.max().values,
                         "l": g.l.min().values, "c": g.c.last().values}).reset_index(drop=True)


@nb.njit(cache=True)
def _signals(t, o, h, l, c, atr, day, dbias, asia_hi, asia_lo, sw_lb, min_conf, use_bias, k):
    n = len(c)
    out_i = np.full(n, -1)
    out_d = np.zeros(n, np.int8)
    out_stop = np.zeros(n)
    out_conf = np.zeros(n, np.int8)
    last = -100
    m = 0
    for i in range(sw_lb + 5, n - 1):
        hr = (t[i] // 3600) % 24
        if not ((7 <= hr < 10) or (12 <= hr < 15)) or i - last < 6:
            continue
        # confirmed swings in [i - sw_lb, i - k)
        sh1 = sh2 = sl1 = sl2 = np.nan
        shi1 = sli1 = -1
        for j in range(max(k, i - sw_lb), i - k):
            okh = True
            okl = True
            for q in range(1, k + 1):
                if h[j] < h[j - q] or h[j] < h[j + q]:
                    okh = False
                if l[j] > l[j - q] or l[j] > l[j + q]:
                    okl = False
            if okh:
                sh2 = sh1
                sh1 = h[j]
                shi1 = j
            if okl:
                sl2 = sl1
                sl1 = l[j]
                sli1 = j
        d = 0
        if sl1 == sl1 and sl2 == sl2 and sh1 == sh1 and sl1 > sl2 and c[i] > sh1:
            d = 1
        elif sh1 == sh1 and sh2 == sh2 and sl1 == sl1 and sh1 < sh2 and c[i] < sl1:
            d = -1
        if d == 0:
            continue
        if use_bias and dbias[i] != 0 and dbias[i] != d:
            continue
        conf = 0
        # breaker: the last opposite candle before the broken swing, price back inside its range
        piv = shi1 if d > 0 else sli1
        for q in range(piv, max(piv - 10, 0), -1):
            if (d > 0 and c[q] < o[q]) or (d < 0 and c[q] > o[q]):
                if l[q] <= c[i] <= h[q] or (d > 0 and l[i] <= h[q]) or (d < 0 and h[i] >= l[q]):
                    conf += 2
                break
        # FVG in the displacement leg of the last 12 bars in the trade direction, price near it
        for q in range(i, max(i - 12, 2), -1):
            if d > 0 and l[q] > h[q - 2] and l[i] <= l[q] + 0.5 * atr[i]:
                conf += 1
                break
            if d < 0 and h[q] < l[q - 2] and h[i] >= h[q] - 0.5 * atr[i]:
                conf += 1
                break
        # inverse FVG: an opposite FVG from the last 30 bars that price has closed through
        for q in range(i - 1, max(i - 30, 2), -1):
            if d > 0 and h[q] < l[q - 2] and c[i] > l[q - 2]:
                conf += 1
                break
            if d < 0 and l[q] > h[q - 2] and c[i] < h[q - 2]:
                conf += 1
                break
        # Asia range of today swept on the opposite side before this bar
        if asia_hi[i] == asia_hi[i]:
            swept = False
            for q in range(i, max(i - 144, 0), -1):
                if day[q] != day[i] or (t[q] // 3600) % 24 < 7:
                    break
                if (d > 0 and l[q] < asia_lo[i]) or (d < 0 and h[q] > asia_hi[i]):
                    swept = True
                    break
            if swept:
                conf += 1
        if conf < min_conf:
            continue
        stop = (l[i] - 0.1 * atr[i]) if d > 0 else (h[i] + 0.1 * atr[i])
        out_i[m] = i
        out_d[m] = d
        out_stop[m] = stop
        out_conf[m] = conf
        m += 1
        last = i
    return out_i[:m], out_d[:m], out_stop[:m], out_conf[:m]


@nb.njit(cache=True)
def _resolve(mt, mo, mh, ml, mc, t_entry, d, stop, target, max_min):
    """Entry at the first 1m open at or after t_entry; stop first inside a minute."""
    n = len(mt)
    a = np.searchsorted(mt, t_entry)
    if a >= n:
        return np.nan, np.nan
    e = mo[a]
    if d * (e - stop) <= 0 or d * (target - e) <= 0:
        return np.nan, np.nan
    for j in range(a, min(a + max_min, n)):
        if (d > 0 and ml[j] <= stop) or (d < 0 and mh[j] >= stop):
            px = min(mo[j], stop) if d > 0 else max(mo[j], stop)
            return e, px if j > a else stop
        if (d > 0 and mh[j] >= target) or (d < 0 and ml[j] <= target):
            return e, target
    return e, mc[min(a + max_min, n) - 1]


@nb.njit(cache=True)
def _resolve_colour(t, o, h, l, c, i, d, e, stop, target):
    """Tanvir's resolution: entry at the CISD close, following 5m bars, a bar that hits both is a
    win if it closes in the trade direction."""
    for j in range(i + 1, min(i + 289, len(c))):
        sl = l[j] <= stop if d > 0 else h[j] >= stop
        tp = h[j] >= target if d > 0 else l[j] <= target
        with_trade = (c[j] >= o[j]) == (d > 0)
        if sl and tp:
            return target if with_trade else stop
        if sl:
            return stop
        if tp:
            return target
    return c[min(i + 288, len(c) - 1)]


def run(m: pd.DataFrame, cost: float, min_rr=2.0, min_conf=0, use_bias=True, rr_fixed=None,
        min_stop_atr=0.0, k=2, sw_lb=40, colour=False):
    b = five_minute(m)
    t, o, h, l, c = (b[x].to_numpy() for x in "tohlc")
    tr = np.maximum(h - l, np.maximum(abs(h - np.r_[c[0], c[:-1]]), abs(l - np.r_[c[0], c[:-1]])))
    atr = pd.Series(tr).ewm(alpha=1 / 14).mean().shift(1).bfill().to_numpy()
    day = t // 86400
    # daily bars and bias known at the start of each day
    dd = b.assign(day=day).groupby("day").agg(o=("o", "first"), h=("h", "max"), l=("l", "min"), c=("c", "last"))
    mid = (dd.h.rolling(20).max() + dd.l.rolling(20).min()) / 2
    up = (dd.c > dd.h.shift(1)).rolling(10).sum()
    dn = (dd.c < dd.l.shift(1)).rolling(10).sum()
    bias_d = np.where((up > dn) & (dd.c < mid), 1, np.where((dn > up) & (dd.c > mid), -1, 0))
    bias = pd.Series(bias_d, index=dd.index).shift(1).fillna(0)
    pdh = dd.h.shift(1)
    pdl = dd.l.shift(1)
    dbias = bias.reindex(day).to_numpy().astype(np.int8)
    # Asia range 00-07 UTC, known from 07:00
    hr = (t // 3600) % 24
    asia = b[hr < 7].assign(day=day[hr < 7]).groupby("day").agg(hi=("h", "max"), lo=("l", "min"))
    asia_hi = np.where(hr >= 7, asia.hi.reindex(day).to_numpy(), np.nan)
    asia_lo = np.where(hr >= 7, asia.lo.reindex(day).to_numpy(), np.nan)
    si, sd, sstop, sconf = _signals(t, o, h, l, c, atr, day, dbias, asia_hi, asia_lo, sw_lb,
                                    min_conf, use_bias, k)
    mt, mo, mh, ml, mc = (m[x].to_numpy() for x in "tohlc")
    rows = []
    pdh_a, pdl_a = pdh.reindex(day).to_numpy(), pdl.reindex(day).to_numpy()
    for i, d, stop, conf in zip(si, sd, sstop, sconf):
        e0 = c[i]
        risk0 = d * (e0 - stop)
        if risk0 <= 0 or risk0 < min_stop_atr * atr[i]:
            continue
        if rr_fixed:
            target = e0 + d * rr_fixed * risk0
        else:
            cands = [asia_hi[i] if d > 0 else asia_lo[i], pdh_a[i] if d > 0 else pdl_a[i]]
            lo = max(0, i - 500)
            cands += list(h[lo:i][h[lo:i] > e0]) if d > 0 else list(l[lo:i][l[lo:i] < e0])
            ok = [x for x in cands if x == x and d * (x - e0) >= min_rr * risk0]
            if not ok:
                continue
            target = min(ok, key=lambda x: abs(x - e0))
        if colour:
            e, x = e0, _resolve_colour(t, o, h, l, c, i, d, e0, stop, target)
        else:
            e, x = _resolve(mt, mo, mh, ml, mc, t[i] + 300, d, stop, target, 1440)
        if e != e:
            continue
        risk = d * (e - stop)
        rows.append((t[i], d, conf, risk, risk / atr[i], d * (x - e) / risk, (d * (x - e) - cost) / risk))
    return pd.DataFrame(rows, columns=["t", "d", "conf", "risk", "risk_atr", "gross", "net"])


def summary(df: pd.DataFrame, label: str) -> str:
    if df.empty:
        return f"{label}: no trades"
    yr = pd.to_datetime(df.t, unit="s").dt.year
    by = df.groupby(yr).net.mean()
    t = df.net.mean() / df.net.std() * np.sqrt(len(df))
    return (f"{label}: n={len(df)} win={100 * (df.net > 0).mean():.1f}% gross {df.gross.mean():+.3f}R "
            f"net {df.net.mean():+.3f}R (t={t:.1f}) median stop ${df.risk.median():.2f} | "
            f"years+ {int((by > 0).sum())}/{len(by)}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default="hist", choices=["hist", "binance"])
    ap.add_argument("--rw", type=int, default=None)
    a = ap.parse_args()
    if a.data == "hist":
        m, cost = sm.hist_minutes(), 0.35
    else:
        m = pd.read_csv(sm.OUT / "xau_1m_live.csv").rename(columns={"time": "t", "open": "o", "high": "h",
                                                                    "low": "l", "close": "c", "volume": "v"})
        cost = 0.35
    if a.rw is not None:
        m = sm.random_walk(m, a.rw)
    tag = f"{a.data}{'' if a.rw is None else f' rw{a.rw}'}"
    print(summary(run(m, 0.0, colour=True), f"{tag} Tanvir's own accounting (close entry, bar colour, no cost)"))
    print(summary(run(m, 0.0), f"{tag} next-minute entry, 1m resolution, no cost"))
    print(summary(run(m, cost), f"{tag} Tanvir as published (no min stop)"))
    for ms in [0.25, 0.5, 1.0]:
        print(summary(run(m, cost, min_stop_atr=ms), f"{tag} min stop {ms} ATR"))
    for mc in [2, 3, 4]:
        print(summary(run(m, cost, min_conf=mc, min_stop_atr=0.5), f"{tag} confidence >= {mc}, min stop 0.5"))
    print(summary(run(m, cost, use_bias=False, min_stop_atr=0.5), f"{tag} no daily bias, min stop 0.5"))
    for rr in [1.0, 2.0]:
        print(summary(run(m, cost, rr_fixed=rr, min_stop_atr=0.5), f"{tag} fixed {rr}R, min stop 0.5"))


if __name__ == "__main__":
    main()
