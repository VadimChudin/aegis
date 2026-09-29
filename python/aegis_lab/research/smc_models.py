"""Time-anchored ICT models on gold, 1-minute simulation with honest checks.

Models (setups are built from past data only; times are New York local):
  judas   Asia range (20:00-00:00) swept in the London window (02:00-05:00); trade the
          reversal, target the other side of the Asia range
  sb      Silver Bullet: in a one-hour window (03-04, 10-11 or 14-15) the first sweep of a
          liquidity level (previous day, Asia, London and previous-hour highs/lows) that was
          on the far side of the window open; trade the reversal, target the nearest level
          on the other side
  crt     Candle range theory on 1h / 4h / NY-day candles: candle 2 takes candle 1's high
          (low) and closes back inside; target candle 1's midpoint or far side
Entries after the sweep:
  market  1m market structure shift (close beyond the last confirmed 1m swing formed before
          the sweep extreme): market order at that close
  fvg     limit at the edge of the first FVG of the displacement leg (after the MSS)
  ce      limit at the middle of that FVG (consequent encroachment)
  ote     limit at the `ote` retracement of the leg (extreme -> high since)
  close   CRT only: market at the open after candle 2 closed back inside, stop behind it
Stop `buf` x hourly ATR behind the sweep extreme; target the model's level if it is at least
`min_rr` R away, else `rr` R. Exits: stop (a gap through it fills at the open), target, time.
Filters: `bias` previous NY day candle direction (with / against), `po3` long only below the
NY midnight open and short only above it.

Data: Binance XAUUSDT 1m (fees 0.02% maker / 0.05% taker) or Dukascopy XAUUSD 1m bid with the
recorded spread (cost = spread + $0.07/oz commission), or XAUUSD CFD 1m 2019-2025 from
Hugging Face (`hist`, cost `--cost` $/oz per trade).

    python -m aegis_lab.research.smc_models --data binance|duka|hist [--rw SEED] [--po3]
"""

from __future__ import annotations

import argparse
import datetime as dt
import itertools
import pathlib

import numba as nb
import numpy as np
import pandas as pd

from aegis_lab.research import smc

OUT = pathlib.Path.home() / "aegis-data"
NAN = np.nan
HIST_COST = 0.35  # $/oz round trip for the CFD data (typical ECN spread + commission)
ENTRY = {"market": 0, "fvg": 1, "ce": 2, "ote": 3, "close": 4}


# ---------------------------------------------------------------- data


def duka_minutes(start="2023-09-01", end="2026-09-27") -> pd.DataFrame:
    from aegis_lab.data import dukascopy as dk

    cache = OUT / "duka_1m.parquet"
    if cache.exists():
        return pd.read_parquet(cache)
    rows, spr = [], []
    raw = OUT / "dukascopy"
    day, last = dt.date.fromisoformat(start), dt.date.fromisoformat(end)
    while day <= last:
        bp, ap = dk._raw_path(raw, "XAUUSD", day, "BID"), dk._raw_path(raw, "XAUUSD", day, "ASK")
        if bp.exists():
            bid = dk.decode(bp.read_bytes(), day)
            ask = {r[0]: r[4] for r in dk.decode(ap.read_bytes(), day)} if ap.exists() else {}
            for ts, o, h, l, c, v in bid:
                if l > 0 and l <= min(o, c) and h >= max(o, c):
                    rows.append((ts, o, h, l, c, v))
                    spr.append(ask.get(ts, NAN) - c)
        day += dt.timedelta(days=1)
    m = pd.DataFrame(rows, columns=["t", "o", "h", "l", "c", "v"])
    m["spread"] = pd.Series(spr).clip(lower=0).ffill().fillna(0.3).to_numpy()
    m.to_parquet(cache)
    return m


HIST_URL = ("https://huggingface.co/datasets/agzdws/xauusd-gold-price-historical-data-2004-2025/"
            "resolve/main/XAU_1m_data.jsonl")


def hist_minutes(start="2019-01-01", end="2025-03-01") -> pd.DataFrame:
    """XAUUSD CFD 1m (MetaTrader export on Hugging Face, 2004-2025). Server time is EET/EEST
    (UTC+2 / UTC+3 with EU daylight saving): the week opens at 01:00 server = 18:00 New York,
    except in the weeks when only the US is on summer time, when the day ends at 15:59 New York
    under a fixed "New York + 7" reading. Complete until 2025-02; later months have gaps.
    No spread column."""
    import orjson
    import urllib.request

    cache = OUT / "hist_1m.parquet"
    if cache.exists():
        return pd.read_parquet(cache)
    raw = OUT / "XAU_1m_data.jsonl"
    if not raw.exists():
        urllib.request.urlretrieve(HIST_URL, raw)
    rows = []
    lo, hi = start.replace("-", ".").encode(), end.replace("-", ".").encode()
    with raw.open("rb") as f:
        for line in f:
            key = line[9:19]
            if key < lo or key >= hi:
                continue
            r = orjson.loads(line)
            rows.append((r["Date"], r["Open"], r["High"], r["Low"], r["Close"], r["Volume"]))
    m = pd.DataFrame(rows, columns=["d", "o", "h", "l", "c", "v"])
    server = pd.to_datetime(m.d, format="%Y.%m.%d %H:%M")
    utc = server.dt.tz_localize("Europe/Athens", ambiguous="NaT", nonexistent="NaT").dt.tz_convert("UTC")
    secs = (utc - pd.Timestamp("1970-01-01", tz="UTC")).dt.total_seconds()
    m = m.assign(t=secs)[utc.notna().to_numpy()]
    m["t"] = m.t.astype(np.int64)
    m = m[(m.l > 0) & (m.l <= m[["o", "c"]].min(axis=1)) & (m.h >= m[["o", "c"]].max(axis=1))]
    m = m.drop_duplicates("t").sort_values("t")[["t", "o", "h", "l", "c", "v"]].reset_index(drop=True)
    m.to_parquet(cache)
    return m


def random_walk(m: pd.DataFrame, seed: int) -> pd.DataFrame:
    """Same clock and hour-of-day volatility as `m`, no structure."""
    rng = np.random.default_rng(seed)
    n, sub = len(m), 20
    hr = (m.t.to_numpy() // 3600) % 24
    vol = m.assign(r=np.log(m.c).diff().abs(), hr=hr).groupby("hr").r.mean().reindex(range(24)).ffill().to_numpy()
    steps = rng.standard_normal((n, sub)) * (vol[hr] * 1.25 / np.sqrt(sub))[:, None]
    p0 = float(m.c.iloc[0])
    path = p0 * np.exp(np.cumsum(steps.ravel())).reshape(n, sub)
    o = np.r_[p0, path[:-1, -1]]
    out = pd.DataFrame({"t": m.t.to_numpy(), "o": o, "h": np.maximum(path.max(1), o),
                        "l": np.minimum(path.min(1), o), "c": path[:, -1], "v": 1.0}).round(2)
    if "spread" in m:
        out["spread"] = m.spread.to_numpy()
    return out


# ---------------------------------------------------------------- simulation


@nb.njit(cache=True)
def _trade(mo, mh, ml, mc, i_s, i_dead, d, tgt, ext, mode, rr, min_rr, buf, fk, ote, min_risk,
           max_hold, exp_bars, tick):
    """One setup in long space (d = -1 mirrors prices).
    -> entry, stop, pnl, exit code (0 stop, 1 target, 2 time, 3 end, -1 none), fill index,
    maker entry, market exit."""
    n = len(mc)
    tl = tgt * d
    entry = 0.0
    stop = 0.0
    i_mss = -1
    maker = False
    i_fill = -1
    if mode == 4:
        if i_s >= n:
            return 0.0, 0.0, 0.0, -1, -1, False, False
        entry = mo[i_s] * d + tick
        stop = ext * d - buf
        i_fill = i_s
    else:
        low_ext = 1e18
        low_i = -1
        sh = 1e18
        sh_i = -1
        for i in range(max(i_s - 60, 2 * fk), min(i_dead, n)):
            j = i - fk
            hj = mh[j] if d > 0 else -ml[j]
            ok = True
            for q in range(j - fk, i + 1):
                if q != j:
                    hq = mh[q] if d > 0 else -ml[q]
                    if hq > hj or (q < j and hq == hj):
                        ok = False
                        break
            if ok:
                sh = hj
                sh_i = j
            if i < i_s:
                continue
            lo = ml[i] if d > 0 else -mh[i]
            cl = mc[i] * d
            if lo < low_ext:
                low_ext = lo
                low_i = i
            if sh_i >= 0 and sh_i < low_i and cl > sh:
                i_mss = i
                break
        if i_mss < 0:
            return 0.0, 0.0, 0.0, -1, -1, False, False
        stop = low_ext - buf
        leg_hi = -1e18
        for q in range(low_i, i_mss + 1):
            hq = mh[q] if d > 0 else -ml[q]
            if hq > leg_hi:
                leg_hi = hq
        if mode == 0:
            entry = mc[i_mss] * d + tick
            i_fill = i_mss
        else:
            if mode == 3:
                lim = leg_hi - ote * (leg_hi - low_ext)
            else:
                lim = 1e18
                for q in range(low_i + 2, i_mss + 1):
                    top = ml[q] if d > 0 else -mh[q]
                    bot = mh[q - 2] if d > 0 else -ml[q - 2]
                    if top > bot:
                        lim = top if mode == 1 else 0.5 * (top + bot)
                        break
                if lim > 1e17:
                    return 0.0, 0.0, 0.0, -1, -1, False, False
            entry = lim
            maker = True
    risk = entry - stop
    if risk < min_risk:
        return 0.0, 0.0, 0.0, -1, -1, False, False
    target = entry + rr * risk
    if tl == tl and tl - entry >= min_rr * risk:
        target = tl
    if i_fill < 0:
        # resting limit: fills on a trade one tick through it; cancelled at the target or expiry
        for i in range(i_mss + 1, min(i_mss + 1 + exp_bars, n)):
            hi = mh[i] if d > 0 else -ml[i]
            lo = ml[i] if d > 0 else -mh[i]
            if lo <= entry - tick:
                i_fill = i
                break
            if hi >= target:
                return 0.0, 0.0, 0.0, -1, -1, False, False
        if i_fill < 0:
            return 0.0, 0.0, 0.0, -1, -1, False, False
        lo = ml[i_fill] if d > 0 else -mh[i_fill]
        if lo <= stop:
            return entry, stop, stop - entry, 0, i_fill, maker, True
    for i in range(i_fill + 1, n):
        op = mo[i] * d
        hi = mh[i] if d > 0 else -ml[i]
        lo = ml[i] if d > 0 else -mh[i]
        if op <= stop:
            return entry, stop, op - entry, 0, i_fill, maker, True
        if lo <= stop:
            return entry, stop, stop - entry, 0, i_fill, maker, True
        if op >= target:
            return entry, stop, op - entry, 1, i_fill, maker, False
        if hi >= target + tick:
            return entry, stop, target - entry, 1, i_fill, maker, False
        if i - i_fill >= max_hold:
            return entry, stop, mc[i] * d - entry, 2, i_fill, maker, True
    return entry, stop, mc[n - 1] * d - entry, 3, i_fill, maker, True


# ---------------------------------------------------------------- setups


class Market:
    def __init__(self, m: pd.DataFrame, venue: str):
        self.m = m.reset_index(drop=True)
        self.venue = venue
        self.t = self.m.t.to_numpy()
        self.o, self.h, self.l, self.c = (self.m[x].to_numpy() for x in "ohlc")
        self.spread = self.m.spread.to_numpy() if "spread" in self.m else None
        ny = pd.to_datetime(self.t, unit="s", utc=True).tz_convert("America/New_York")
        self.ny_min = (ny.hour * 60 + ny.minute).to_numpy()
        self.ny_day = ny.tz_localize(None).normalize().values.astype("datetime64[D]").astype(np.int64)
        self.dow = ny.dayofweek.to_numpy()
        hb = smc.resample(self.m, 3600).set_index("t")
        atr = hb.atr.reindex(pd.Index(self.t // 3600 * 3600 - 3600)).to_numpy()
        self.atr = pd.Series(atr).ffill().bfill().to_numpy()
        self.days = self._days()

    def _days(self) -> pd.DataFrame:
        """Per NY day: open at midnight, previous day O/H/L/C, Asia and London ranges."""
        df = pd.DataFrame({"day": self.ny_day, "min": self.ny_min, "o": self.o, "h": self.h, "l": self.l,
                           "c": self.c, "i": np.arange(len(self.t))})
        g = df.groupby("day")
        day = pd.DataFrame({"o": g.o.first(), "h": g.h.max(), "l": g.l.min(), "c": g.c.last(), "i0": g.i.first()})
        day["midnight"] = df[df["min"] == 0].groupby("day").o.first()
        asia = df[df["min"] >= 20 * 60].assign(day=lambda x: x.day + 1).groupby("day")
        day["asia_h"], day["asia_l"] = asia.h.max(), asia.l.min()
        lon = df[(df["min"] >= 120) & (df["min"] < 300)].groupby("day")
        day["lon_h"], day["lon_l"] = lon.h.max(), lon.l.min()
        for k in ("o", "h", "l", "c"):
            day[f"prev_{k}"] = day[k].shift()
        return day

    def cost(self, i: int, px: float, maker: bool, mkt_exit: bool) -> float:
        if self.venue == "binance":
            return ((smc.MAKER if maker else smc.TAKER) + (smc.TAKER if mkt_exit else smc.MAKER)) * px
        if self.venue == "hist":
            return HIST_COST
        return float(self.spread[i]) + 0.07

    def idx(self, day: int, minute: int) -> int:
        """First bar at or after NY `minute` of `day`."""
        k = np.searchsorted(self.ny_day, day)
        k2 = np.searchsorted(self.ny_day, day, "right")
        return k + int(np.searchsorted(self.ny_min[k:k2], minute))

    def setups_judas(self) -> list:
        out = []
        for day, r in self.days.iterrows():
            if np.isnan(r.asia_h):
                continue
            a, b = self.idx(day, 120), self.idx(day, 300)
            if b <= a:
                continue
            up = np.flatnonzero(self.h[a:b] > r.asia_h)
            dn = np.flatnonzero(self.l[a:b] < r.asia_l)
            if len(dn):
                out.append((a + dn[0], b, 1, r.asia_h, NAN, day))
            if len(up):
                out.append((a + up[0], b, -1, r.asia_l, NAN, day))
        return sorted(out)

    def setups_sb(self, start_min: int) -> list:
        out = []
        for day, r in self.days.iterrows():
            a, b = self.idx(day, start_min), self.idx(day, start_min + 60)
            if b <= a + 5:
                continue
            p = self.o[a]
            pa = self.idx(day, start_min - 60)
            lv = [r.prev_h, r.prev_l, r.asia_h, r.asia_l]
            if start_min >= 300:
                lv += [r.lon_h, r.lon_l]
            if a > pa:
                lv += [self.h[pa:a].max(), self.l[pa:a].min()]
            lv = np.array([x for x in lv if x == x])
            above, below = np.sort(lv[lv > p]), np.sort(lv[lv < p])[::-1]
            if len(above):
                k = np.flatnonzero(self.h[a:b] > above[0])
                if len(k):
                    out.append((a + k[0], b, -1, below[0] if len(below) else NAN, NAN, day))
            if len(below):
                k = np.flatnonzero(self.l[a:b] < below[0])
                if len(k):
                    out.append((a + k[0], b, 1, above[0] if len(above) else NAN, NAN, day))
        return sorted(out)

    def setups_crt(self, sec: int, target: str, entry_close: bool) -> list:
        if sec == 86400:
            starts = self.days.i0.to_numpy().astype(int)
        else:
            key = self.t // sec
            starts = np.flatnonzero(np.r_[True, key[1:] != key[:-1]])
        ends = np.r_[starts[1:], len(self.t)]
        hs = np.maximum.reduceat(self.h, starts)
        ls = np.minimum.reduceat(self.l, starts)
        cs = self.c[ends - 1]
        out = []
        for k in range(1, len(starts)):
            h1, l1, s2, e2, h2, l2, c2 = hs[k - 1], ls[k - 1], starts[k], ends[k], hs[k], ls[k], cs[k]
            if e2 - s2 < 3:
                continue
            day = int(self.ny_day[s2])
            mid = 0.5 * (h1 + l1)
            if entry_close:
                # candle 2 is complete here: it took one side only and closed back inside
                if h2 > h1 and l1 < c2 < h1 and not l2 < l1:
                    out.append((e2, e2, -1, mid if target == "mid" else l1, h2, day))
                if l2 < l1 and l1 < c2 < h1 and not h2 > h1:
                    out.append((e2, e2, 1, mid if target == "mid" else h1, l2, day))
                continue
            # intra-candle: only what is known at the sweep (first cross of a side; the other side
            # not taken earlier in candle 2); the 1m MSS must come before candle 2 closes
            up = np.flatnonzero(self.h[s2:e2] > h1)
            dn = np.flatnonzero(self.l[s2:e2] < l1)
            iu = up[0] if len(up) else 1 << 40
            idn = dn[0] if len(dn) else 1 << 40
            if iu < idn:
                out.append((s2 + iu, e2, -1, mid if target == "mid" else l1, NAN, day))
            elif idn < iu:
                out.append((s2 + idn, e2, 1, mid if target == "mid" else h1, NAN, day))
        return out

    def run(self, setups: list, cfg: dict) -> pd.DataFrame:
        mode = ENTRY[cfg["entry"]]
        if not hasattr(self, "_dix"):
            d = self.days
            self._dix = {k: i for i, k in enumerate(d.index)}
            self._pc, self._po, self._mid = d.prev_c.to_numpy(), d.prev_o.to_numpy(), d.midnight.to_numpy()
        rows = []
        busy = -1
        for i_s, i_dead, d, tgt, ext, day in setups:
            if i_s >= len(self.t) or i_s <= busy or self.dow[i_s] >= 5:
                continue
            k = self._dix[day]
            pc, po, mid = self._pc[k], self._po[k], self._mid[k]
            if cfg["bias"] and pc == pc:
                b = 1 if pc > po else -1
                if (b != d) if cfg["bias"] == "with" else (b == d):
                    continue
            if cfg["po3"] and mid == mid:
                px = self.o[i_s]
                if (d > 0 and px > mid) or (d < 0 and px < mid):
                    continue
            buf = cfg["buf"] * self.atr[i_s]
            e, s, pnl, code, i_f, maker, mkt = _trade(
                self.o, self.h, self.l, self.c, int(i_s), int(i_dead) + cfg["extra"], int(d),
                tgt if cfg["use_tgt"] else NAN, ext if ext == ext else 0.0, mode, cfg["rr"], cfg["min_rr"], buf,
                cfg["fk"], cfg["ote"], cfg["min_risk"], cfg["max_hold"], cfg["exp_bars"], 0.01)
            if code < 0:
                continue
            busy = i_f
            fee = self.cost(i_f, abs(e), maker, mkt)
            rows.append((self.t[i_f], d, pnl, fee, e - s, code))
        df = pd.DataFrame(rows, columns=["t", "dir", "pnl", "fee", "risk", "exit"])
        df["r_gross"] = df.pnl / df.risk
        df["r_net"] = (df.pnl - df.fee) / df.risk
        return df


# ---------------------------------------------------------------- search

BASE = dict(entry="market", rr=2.0, min_rr=1.0, use_tgt=True, buf=0.1, fk=3, ote=0.705, min_risk=2.0,
            max_hold=480, exp_bars=60, extra=60, bias="", po3=False)

MODELS = {
    "judas": lambda mk, cfg: mk.setups_judas(),
    "sb_03": lambda mk, cfg: mk.setups_sb(180),
    "sb_10": lambda mk, cfg: mk.setups_sb(600),
    "sb_14": lambda mk, cfg: mk.setups_sb(840),
    "crt_1h": lambda mk, cfg: mk.setups_crt(3600, cfg["crt_tgt"], cfg["entry"] == "close"),
    "crt_4h": lambda mk, cfg: mk.setups_crt(14400, cfg["crt_tgt"], cfg["entry"] == "close"),
    "crt_1d": lambda mk, cfg: mk.setups_crt(86400, cfg["crt_tgt"], cfg["entry"] == "close"),
}

GRID = dict(entry=["market", "fvg", "ce", "ote"], rr=[1.5, 2.0, 3.0], use_tgt=[True, False], buf=[0.05, 0.2],
            min_risk=[1.0, 3.0], bias=["", "with", "against"], po3=[False, True])


def configs(model: str):
    grid = dict(GRID)
    if model.startswith("crt"):
        grid["entry"] = grid["entry"] + ["close"]
        grid["crt_tgt"] = ["mid", "far"]
    for vals in itertools.product(*grid.values()):
        yield {**BASE, **dict(zip(grid, vals))}


def search(mk: Market, split: int, models=None) -> tuple[pd.DataFrame, dict]:
    rows, trades, cache = [], {}, {}
    for model in models or MODELS:
        for cfg in configs(model):
            key = (model, cfg.get("crt_tgt"), cfg["entry"] == "close")
            if key not in cache:
                cache[key] = MODELS[model](mk, cfg)
            r = mk.run(cache[key], cfg)
            trades[len(rows)] = r
            a, b = r[r.t < split], r[r.t >= split]
            rows.append({"model": model, **{c: cfg[c] for c in list(GRID) + ["crt_tgt"] if c in cfg},
                         "is_n": len(a), "is_g": a.r_gross.mean(), "is_r": a.r_net.mean(),
                         "oos_n": len(b), "oos_g": b.r_gross.mean(), "oos_r": b.r_net.mean(),
                         "oos_win": (b.r_net > 0).mean()})
    return pd.DataFrame(rows), trades


def walk_forward(trades: dict, months_back: int = 6, min_n: int = 10) -> pd.DataFrame:
    tagged = {k: r.assign(m=pd.to_datetime(r.t, unit="s").dt.strftime("%Y-%m")) for k, r in trades.items()}
    months = sorted(set().union(*[set(r.m) for r in tagged.values()]))
    out = []
    for mi in range(months_back, len(months)):
        train, test = months[mi - months_back : mi], months[mi]
        best, score = None, -np.inf
        for k, r in tagged.items():
            tr = r[r.m.isin(train)]
            if len(tr) >= min_n and tr.r_net.mean() > score:
                best, score = k, tr.r_net.mean()
        if best is None:
            continue
        te = tagged[best][tagged[best].m == test]
        out.append((test, best, score, len(te), te.r_net.sum()))
    return pd.DataFrame(out, columns=["month", "cfg", "train_r", "n", "sum_r"])


def report(df: pd.DataFrame, trades: dict, label: str, months_back: int = 6) -> None:
    ok = df[(df.is_n >= 30) & (df.oos_n >= 15)]
    print(f"\n==== {label}: {len(df)} settings, {len(ok)} with >= 30 / 15 trades")
    print(ok.groupby("model")[["is_n", "oos_n", "is_g", "is_r", "oos_g", "oos_r"]].median().round(3).to_string())
    print(ok.groupby("entry")[["is_g", "is_r", "oos_g", "oos_r"]].median().round(3).to_string())
    for c in ("bias", "po3", "use_tgt", "rr"):
        print(ok.groupby(c)[["is_r", "oos_r"]].median().round(3).to_string())
    both = ok[(ok.is_r > 0) & (ok.oos_r > 0)]
    print(f"net positive in both halves: {len(both)} of {len(ok)}; "
          f"in-sample positive -> out-of-sample positive: {(ok[ok.is_r > 0].oos_r > 0).mean():.1%} "
          f"(base {(ok.oos_r > 0).mean():.1%})")
    top = ok.sort_values("is_r", ascending=False).head(15)
    print(top.round(3).to_string())
    print(f"top 15 in sample -> out of sample mean {top.oos_r.mean():+.3f} R")
    wf = walk_forward({k: trades[k] for k in ok.index}, months_back)
    print(wf.to_string())
    print(f"walk-forward Auto: {wf.n.sum()} trades, {wf.sum_r.sum():+.1f} R, "
          f"{wf.sum_r.sum() / max(wf.n.sum(), 1):+.3f} R/trade, months positive {(wf.sum_r > 0).sum()}/{len(wf)}")


def po3_rule(m: pd.DataFrame, entry_min: int = 510, exit_min: int = 960) -> pd.DataFrame:
    """Power of 3 from the midnight open: at `entry_min` (NY) trade the side of the midnight open,
    stop behind the day's extreme so far, exit at `exit_min`. One trade per weekday."""
    mk = Market(m, "x")
    df = pd.DataFrame({"day": mk.ny_day, "min": mk.ny_min, "o": mk.o, "h": mk.h, "l": mk.l, "c": mk.c,
                       "dow": mk.dow})
    df = df[(df["min"] < 17 * 60) & (df.dow < 5)]
    rows = []
    for day, g in df.groupby("day"):
        if len(g) < 600:
            continue
        mn, o, h, l, c = (g[x].to_numpy() for x in ("min", "o", "h", "l", "c"))
        k, k2 = np.searchsorted(mn, entry_min), np.searchsorted(mn, exit_min)
        if k < 30 or k2 >= len(mn):
            continue
        px = o[k]
        d = 1 if px > o[0] else -1
        hi, lo = h[:k].max(), l[:k].min()
        risk = px - lo if d > 0 else hi - px
        if risk <= 0:
            continue
        adverse = (l[k:k2] - px) if d > 0 else (px - h[k:k2])
        pnl = -risk if (adverse <= -risk).any() else (c[k2 - 1] - px) * d
        disc = (px < (hi + lo) / 2) if d > 0 else (px > (hi + lo) / 2)
        rows.append((day, d, risk, pnl, disc))
    r = pd.DataFrame(rows, columns=["day", "dir", "risk", "pnl", "discount"])
    r["R"] = r.pnl / r.risk
    return r


def po3_report(m: pd.DataFrame, cost: float) -> None:
    r = po3_rule(m)
    yr = pd.to_datetime(r.day, unit="D").dt.year
    t = r.R.mean() / r.R.std() * np.sqrt(len(r))
    print(f"08:30 -> 16:00: {len(r)} trades, gross {r.R.mean():+.3f} R (t {t:.2f}), "
          f"net {((r.pnl - cost) / r.risk).mean():+.3f} R at ${cost}/oz; by year "
          + str(r.groupby(yr).R.mean().round(3).to_dict()))
    grid = {f"{e // 60}:{e % 60:02d}": {f"{x // 60}:00": round(po3_rule(m, e, x).R.mean(), 3)
                                        for x in (720, 840, 960)} for e in range(420, 601, 30)}
    print(pd.DataFrame(grid).T.to_string())
    rws = [po3_rule(random_walk(m, s)).R.mean() for s in range(1, 6)]
    print("random walks:", np.round(rws, 3))


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default="binance", choices=["binance", "duka", "hist"])
    ap.add_argument("--cost", type=float, default=0.35, help="hist: $/oz per round trip")
    ap.add_argument("--rw", type=int, default=-1, help="run on a random walk with this seed")
    ap.add_argument("--po3", action="store_true", help="only the Power of 3 midnight-open rule")
    a = ap.parse_args()
    if a.data == "binance":
        m, split, back = smc.minutes(), int(pd.Timestamp("2026-06-01", tz="UTC").timestamp()), 3
    elif a.data == "duka":
        m, split, back = duka_minutes(), int(pd.Timestamp("2025-09-01", tz="UTC").timestamp()), 6
    else:
        global HIST_COST
        HIST_COST = a.cost
        m, split, back = hist_minutes(), int(pd.Timestamp("2023-01-01", tz="UTC").timestamp()), 6
    if a.po3:
        po3_report(m, HIST_COST if a.data == "hist" else 0.001 * float(m.c.iloc[-1]))
        return
    if a.rw >= 0:
        m = random_walk(m, a.rw)
    mk = Market(m, a.data)
    df, trades = search(mk, split)
    tag = f"{a.data}{'_rw' + str(a.rw) if a.rw >= 0 else ''}{'_c' + str(a.cost) if a.data == 'hist' else ''}"
    df.to_csv(OUT / f"smc_models_{tag}.csv", index=False)
    report(df, trades, tag, back)


if __name__ == "__main__":
    main()
