"""Smart-money (SMC) setups on Binance XAUUSDT: zones on the higher timeframe, entry on 1m.

Zones (built on 1h or 4h bars from 1m, each known only after the bar that proves it closed):
  ob     order block: the last opposite candle before the move that broke structure (BOS/CHoCH)
  fvg    fair value gap: three-candle imbalance, bar1 high < bar3 low (bullish)
  sweep  liquidity sweep: a wick through a confirmed swing / previous-day extreme that closes
         back inside; zone = wick .. swept price
Filters: trade with the higher-timeframe structure, premium/discount (buy only in the lower half
of the current dealing range), killzones (London 07-10, New York 12-16 UTC), zone size in ATR.

Entries on 1m:
  limit  one limit at the zone edge (depth 0) or inside it (depth 0.5 = middle)
  grid   `n` limits from the edge to `grid_depth` of the zone, bigger deeper
  choch  price enters the zone, then a 1m change of character (close above the last confirmed
         1m swing high before the low made in the zone): market entry, stop under that low
Stop `buffer` x ATR beyond the zone (limit/grid) or the zone low (choch); target `rr` x risk
(limit order); optional breakeven at 1R; time exit. Limit fills need a trade 1 tick through.
Fees: Binance USDⓈ-M 0.02% maker (limits, target) / 0.05% taker (market entry, stop, time).

    python -m aegis_lab.research.smc [--grid]
"""

from __future__ import annotations

import argparse
import io
import itertools
import pathlib
import zipfile

import numba as nb
import numpy as np
import pandas as pd

DATA = pathlib.Path.home() / "aegis-data" / "binance" / "XAUUSDT"
LAUNCH = pd.Timestamp("2026-01-05", tz="UTC").value // 10**9
SPLIT = pd.Timestamp("2026-06-01", tz="UTC").value // 10**9
MAKER, TAKER = 0.0002, 0.0005
KILLZONES = ((7, 10), (12, 16))


def minutes() -> pd.DataFrame:
    frames = []
    for f in sorted((DATA / "klines-1m").glob("*.zip")):
        with zipfile.ZipFile(f) as z:
            raw = z.read(z.namelist()[0])
        head = raw[:9] == b"open_time"
        d = pd.read_csv(io.BytesIO(raw), header=0 if head else None, usecols=range(6))
        d.columns = ["t", "o", "h", "l", "c", "v"]
        frames.append(d)
    m = pd.concat(frames).drop_duplicates("t").sort_values("t")
    m["t"] = m.t // 1000
    return m[m.t >= LAUNCH].reset_index(drop=True)


def bybit_minutes() -> pd.DataFrame:
    """1m candles from Bybit's public tape (independent venue, from 2026-03-09)."""
    cache = pathlib.Path.home() / "aegis-data" / "bybit" / "XAUUSDT" / "minutes.parquet"
    if cache.exists():
        return pd.read_parquet(cache)
    frames = []
    for f in sorted((pathlib.Path.home() / "aegis-data" / "bybit" / "XAUUSDT" / "trades").glob("*.csv.gz")):
        d = pd.read_csv(f, usecols=["timestamp", "size", "price"])
        d["t"] = (d.timestamp // 60 * 60).astype(np.int64)
        g = d.sort_values("timestamp", kind="stable").groupby("t")
        frames.append(pd.DataFrame({"o": g.price.first(), "h": g.price.max(), "l": g.price.min(),
                                    "c": g.price.last(), "v": g["size"].sum()}).reset_index())
    m = pd.concat(frames).drop_duplicates("t").sort_values("t").reset_index(drop=True)
    m.to_parquet(cache)
    return m


def resample(m: pd.DataFrame, sec: int) -> pd.DataFrame:
    g = m.groupby(m.t // sec * sec)
    b = pd.DataFrame({"o": g.o.first(), "h": g.h.max(), "l": g.l.min(), "c": g.c.last(), "v": g.v.sum()})
    b.index.name = "t"
    b = b.reset_index()
    # Drop incomplete aggregates BEFORE indicators: malformed prior bars must
    # not contaminate the ATR of subsequently complete bars.
    complete = (g.t.count() == sec // 60) & (g.t.min() == g.t.count().index) & (g.t.max() + 60 == g.t.count().index + sec)
    b = b.loc[complete.to_numpy()].reset_index(drop=True)
    tr = np.maximum(b.h - b.l, np.maximum((b.h - b.c.shift()).abs(), (b.l - b.c.shift()).abs()))
    b["atr"] = tr.rolling(14, min_periods=5).mean()
    return b


def zones(b: pd.DataFrame, sec: int, k: int = 3) -> pd.DataFrame:
    """All zones of `b`. `valid` = first second the zone is known; dir +1 bullish, -1 bearish."""
    t, o, h, l, c, atr = (b[x].to_numpy() for x in ("t", "o", "h", "l", "c", "atr"))
    n = len(b)
    if not n:
        return pd.DataFrame(columns=["kind", "dir", "valid", "top", "bot", "atr", "trend", "pd_ok", "choch", "bar", "size_atr"])
    out = []
    sh = sl = None  # last confirmed unbroken swing (index, price)
    highs, lows = [], []  # confirmed swings for the dealing range and sweeps
    trend = 0
    day = t // 86400
    pdh = pdl = np.nan
    cur_hi, cur_lo, cur_day = -np.inf, np.inf, day[0]
    for i in range(n):
        if day[i] != cur_day:
            pdh, pdl, cur_day = cur_hi, cur_lo, day[i]
            cur_hi, cur_lo = -np.inf, np.inf
        cur_hi, cur_lo = max(cur_hi, h[i]), min(cur_lo, l[i])
        # swing at j = i - k confirmed now (strictly higher than k bars left, >= k bars right)
        j = i - k
        if j >= k:
            if h[j] > h[j - k : j].max() and h[j] >= h[j + 1 : i + 1].max():
                sh = (j, h[j])
                highs.append((j, h[j]))
            if l[j] < l[j - k : j].min() and l[j] <= l[j + 1 : i + 1].min():
                sl = (j, l[j])
                lows.append((j, l[j]))
        valid = t[i] + sec
        a = atr[i]
        if np.isnan(a):
            continue
        rng_hi = highs[-1][1] if highs else np.nan
        rng_lo = lows[-1][1] if lows else np.nan

        def add(kind, d, top, bot, extra=0.0):
            if top > bot:
                mid = (rng_hi + rng_lo) / 2
                zmid = (top + bot) / 2
                pd_ok = float((zmid < mid) if d > 0 else (zmid > mid)) if rng_hi > rng_lo else np.nan
                out.append((kind, d, valid, top, bot, a, trend, pd_ok, extra, i))

        # structure breaks -> order blocks
        if sh is not None and c[i] > sh[1]:
            choch = trend < 0
            m = sh[0] + int(np.argmin(l[sh[0] : i + 1]))
            ob = next((q for q in range(m, max(sh[0] - 1, m - 6), -1) if c[q] < o[q]), None)
            trend = 1
            if ob is not None:
                add("ob", 1, h[ob], l[ob], float(choch))
            sh = None
        if sl is not None and c[i] < sl[1]:
            choch = trend > 0
            m = sl[0] + int(np.argmax(h[sl[0] : i + 1]))
            ob = next((q for q in range(m, max(sl[0] - 1, m - 6), -1) if c[q] > o[q]), None)
            trend = -1
            if ob is not None:
                add("ob", -1, h[ob], l[ob], float(choch))
            sl = None
        # fair value gaps
        if i >= 2:
            if l[i] > h[i - 2]:
                add("fvg", 1, l[i], h[i - 2])
            if h[i] < l[i - 2]:
                add("fvg", -1, l[i - 2], h[i])
        # liquidity sweeps of confirmed swings (last 20) and the previous day's extremes
        tgt_lo = [p for q, p in lows[-20:] if q < i] + ([pdl] if not np.isnan(pdl) else [])
        tgt_hi = [p for q, p in highs[-20:] if q < i] + ([pdh] if not np.isnan(pdh) else [])
        sw = [p for p in tgt_lo if l[i] < p < c[i] and l[i - 1] >= p]
        if sw:
            add("sweep", 1, max(sw), l[i])
        sw = [p for p in tgt_hi if c[i] < p < h[i] and h[i - 1] <= p]
        if sw:
            add("sweep", -1, h[i], min(sw))
    z = pd.DataFrame(out, columns=["kind", "dir", "valid", "top", "bot", "atr", "trend", "pd_ok", "choch", "bar"])
    z["size_atr"] = (z.top - z.bot) / z.atr
    return z


@nb.njit(cache=True)
def _sim_details(mt, mh, ml, mc, hour_end, active, i0, i1, i_exp, d, top, bot, mode, n, depth, grid_depth,
         buf, rr, be, max_hold, fk, tick, mo, slippage, spread):
    """-> filled qty, avg entry, stop, pnl ($/oz x qty), market qty, exit code, entry index.
    exit: 0 stop, 1 target, 2 breakeven, 3 time, 4 end of data. Works in "long" space."""
    if d > 0:
        edge, far = top, bot
    else:
        edge, far = -bot, -top
    zh = edge - far
    stop = far - buf
    prices = np.empty(n)
    wts = np.empty(n)
    if mode == 0:
        prices[0] = edge - depth * zh
        wts[0] = 1.0
        nl = 1
    elif mode == 1:
        nl = n
        for k in range(n):
            prices[k] = edge - zh * grid_depth * (k / (n - 1) if n > 1 else 0.0)
            wts[k] = 1.0 + k
        s = wts[:nl].sum()
        for k in range(nl):
            wts[k] /= s
    else:
        nl = 0
    avg_plan = 0.0
    for k in range(nl):
        avg_plan += prices[k] * wts[k]
    target = avg_plan + rr * (avg_plan - stop) if nl > 0 else 0.0
    filled = np.zeros(max(nl, 1), np.bool_)
    qty = 0.0
    cost = 0.0
    t_in = -1
    inside = False
    low_in = 1e18
    low_i = -1
    last_sh = 1e18
    last_sh_i = -1
    be_on = False
    for i in range(i0, i1):
        op = mo[i] * d
        if d > 0:
            hi, lo, cl = mh[i], ml[i], mc[i]
        else:
            hi, lo, cl = -ml[i], -mh[i], -mc[i]
        if qty == 0.0:
            if i >= i_exp or (i > i0 and hour_end[i - 1] and mc[i - 1] * d < far):
                return 0.0, 0.0, 0.0, 0.0, 0.0, -1, -1, -1
            if mode <= 1:
                if active[i]:
                    for k in range(nl):
                        if not filled[k] and lo <= prices[k] - tick:
                            filled[k] = True
                            qty += wts[k]
                            cost += wts[k] * prices[k]
                            t_in = i
                if qty > 0.0 and lo <= stop:
                    return qty, cost / qty, stop, qty * (min(stop, op) - slippage - spread / 2) - cost, qty, 0, t_in, i
                continue
            # choch entry
            if lo <= edge:
                inside = True
            if inside:
                if lo < low_in:
                    low_in = lo
                    low_i = i
                # 1m swing high confirmed `fk` bars later
                j = i - fk
                if j - fk >= i0:
                    hj = mh[j] if d > 0 else -ml[j]
                    ok = True
                    for q in range(j - fk, i + 1):
                        if q != j:
                            hq = mh[q] if d > 0 else -ml[q]
                            if hq > hj:
                                ok = False
                                break
                    if ok:
                        last_sh = hj
                        last_sh_i = j
                if low_in < far - buf:
                    return 0.0, 0.0, 0.0, 0.0, 0.0, -1, -1, -1
                if active[i] and last_sh_i >= 0 and last_sh_i < low_i and cl > last_sh:
                    e = cl + slippage + spread / 2
                    stop = low_in - buf
                    if e - stop <= 0.02:
                        continue
                    qty = 1.0
                    cost = e
                    target = e + rr * (e - stop)
                    t_in = i
                    last_sh = 1e18
            continue
        # in a position
        avg = cost / qty
        if mode <= 1:
            for k in range(nl):
                if not filled[k] and lo <= prices[k] - tick and active[i]:
                    filled[k] = True
                    qty += wts[k]
                    cost += wts[k] * prices[k]
            avg = cost / qty
        sp = avg if be_on else stop
        if lo <= sp:
            return qty, avg, stop, qty * (min(sp, op) - slippage - spread / 2) - cost, qty, 2 if be_on else 0, t_in, i
        if hi >= target + tick:
            return qty, avg, stop, qty * target - cost, 0.0, 1, t_in, i
        if be > 0 and hi >= avg + be * (avg - stop):
            be_on = True
        if mt[i] - mt[t_in] >= max_hold:
            return qty, avg, stop, qty * (cl - slippage - spread / 2) - cost, qty, 3, t_in, i
    if qty > 0.0:
        cl = mc[i1 - 1] if d > 0 else -mc[i1 - 1]
        return qty, cost / qty, stop, qty * (cl - slippage - spread / 2) - cost, qty, 4, t_in, i1 - 1
    return 0.0, 0.0, 0.0, 0.0, 0.0, -1, -1, -1


@nb.njit(cache=True)
def _sim(mt, mh, ml, mc, hour_end, active, i0, i1, i_exp, d, top, bot, mode, n, depth, grid_depth,
         buf, rr, be, max_hold, fk, tick):
    """Compatibility interface; production Lab supplies actual opens and execution costs."""
    result = _sim_details(mt, mh, ml, mc, hour_end, active, i0, i1, i_exp, d, top, bot, mode,
                          n, depth, grid_depth, buf, rr, be, max_hold, fk, tick, mc, 0.05, 0.0)
    return result[:7]


MODES = {"limit": 0, "grid": 1, "choch": 2}
BASE = dict(htf=3600, kinds=("ob",), mode="limit", n=4, depth=0.0, grid_depth=1.0, buf=0.1, rr=2.0,
            be=0.0, max_hold=86400, max_age=72 * 3600, trend=True, pd=False, killzone=False,
            min_size=0.2, max_size=3.0, fk=3, min_risk=1.0, portfolio="one_position")


class Lab:
    def __init__(self, venue: str = "binance", tick_through: float = 0.01, m=None,
                 maker=MAKER, taker=TAKER, slippage=0.05, spread=0.0):
        m = (minutes() if venue == "binance" else bybit_minutes()) if m is None else m.copy()
        if len(m) == 0 or not m.t.is_monotonic_increasing or m.t.duplicated().any():
            raise ValueError("Minute timestamps must be nonempty, unique and sorted")
        if min(maker, taker, slippage, spread) < 0:
            raise ValueError("Execution costs must be nonnegative")
        self.maker, self.taker, self.slippage, self.spread = maker, taker, slippage, spread
        self.mo = m.o.to_numpy()
        self.tick = tick_through
        self.m = m
        self.mt = m.t.to_numpy()
        self.mh, self.ml, self.mc = m.h.to_numpy(), m.l.to_numpy(), m.c.to_numpy()
        self.days = (self.mt[-1] - self.mt[0]) / 86400
        hour = (self.mt // 3600) % 24
        dow = ((self.mt // 86400) + 3) % 7  # 0 = Monday
        kz = np.zeros(len(m), bool)
        for a, b in KILLZONES:
            kz |= (hour >= a) & (hour < b)
        self.kz = kz & (dow < 5)
        self.all = np.ones(len(m), bool)
        self.z = {}
        self.hour_end = {}
        for sec in (3600, 14400):
            self.z[sec] = zones(resample(m, sec), sec)
            self.hour_end[sec] = ((self.mt + 60) % sec) == 0

    def run(self, cfg: dict, start=None, end=None) -> pd.DataFrame:
        z = self.z[cfg["htf"]]
        # Independent cohorts start flat with fresh zones only. Warmup bars
        # provide indicators, not resurrected orders with discarded lifecycle.
        if start is not None:
            z = z[z.valid >= start]
        z = z[z.kind.isin(cfg["kinds"]) & (z.size_atr >= cfg["min_size"]) & (z.size_atr <= cfg["max_size"])]
        if cfg["trend"]:
            z = z[z.trend == z.dir]
        if cfg["pd"]:
            z = z[z.pd_ok == 1]
        active = self.kz if cfg["killzone"] else self.all
        he = self.hour_end[cfg["htf"]]
        i0s = np.searchsorted(self.mt, z.valid.to_numpy())
        i1s = np.searchsorted(self.mt, z.valid.to_numpy() + cfg["max_age"])
        mode = MODES[cfg["mode"]]
        rows = []
        end_i = len(self.mt) if end is None else int(np.searchsorted(self.mt + 60, end, side="right"))
        start_i = 0 if start is None else int(np.searchsorted(self.mt, start))
        for (idx, r), i0, i1 in zip(z.iterrows(), i0s, i1s):
            i0 = max(i0, start_i)
            if i0 >= end_i or i0 >= i1:
                continue
            buf = cfg["buf"] * r.atr
            qty, avg, stop, pnl, mkt, ex, ti, tx = _sim_details(
                self.mt, self.mh, self.ml, self.mc, he, active, i0, end_i, i1, r.dir, r.top, r.bot,
                mode, cfg["n"], cfg["depth"], cfg["grid_depth"], buf, cfg["rr"], cfg["be"],
                cfg["max_hold"], cfg["fk"], self.tick, self.mo, self.slippage, self.spread)
            if qty <= 0:
                continue
            risk = qty * (avg - stop)
            px = abs(avg)
            entry_fee = (self.taker if mode == 2 else self.maker) * px * qty
            exit_px = abs(avg + pnl / qty)
            exit_fee = (self.taker * mkt + self.maker * (qty - mkt)) * exit_px
            fee = entry_fee + exit_fee
            # R per unit of the risk planned for the whole order (all limits filled): a trade where
            # only the smallest limit filled must not weigh as much as a full one
            plan = self.plan_risk(cfg, r, buf) if mode <= 1 else risk
            if (plan if mode <= 1 else risk / qty) < cfg["min_risk"]:
                continue
            rows.append((idx, r.kind, r.dir, self.mt[ti], qty, risk / qty, pnl / qty, fee / qty, ex,
                         pnl / plan, (pnl - fee) / plan, pnl, fee, self.mt[tx] + 60))
        trades = pd.DataFrame(rows, columns=["zone", "kind", "dir", "t", "qty", "risk", "usd", "fee", "exit",
                                           "r_gross", "r_net", "pnl_total", "fee_total", "exit_t"])
        if cfg.get("portfolio", "one_position") == "one_position" and len(trades):
            # Competing resting orders are cancelled when the first order fills. No reissue.
            trades = trades.sort_values(["t", "zone"], kind="stable")
            keep, available = [], -1
            for idx, trade in trades.iterrows():
                armed = max(z.loc[trade.zone, "valid"], self.mt[start_i])
                if trade.t >= available and armed >= available:
                    keep.append(idx)
                    available = trade.exit_t
            trades = trades.loc[keep]
        elif cfg.get("portfolio", "one_position") not in ("one_position", "independent"):
            raise ValueError("portfolio must be one_position or independent")
        return trades.reset_index(drop=True)

    @staticmethod
    def plan_risk(cfg: dict, r, buf: float) -> float:
        zh = r.top - r.bot
        if cfg["mode"] == "limit":
            return zh * (1 - cfg["depth"]) + buf
        n = cfg["n"]
        w = np.arange(1, n + 1, dtype=float)
        depth = cfg["grid_depth"] * (np.arange(n) / (n - 1) if n > 1 else np.zeros(1))
        return float((w * (zh * (1 - depth) + buf)).sum() / w.sum())

    def summary(self, r: pd.DataFrame, lo: int, hi: int) -> dict:
        r = r[(r.t >= lo) & (r.t < hi) & (r.exit_t <= hi) & (r.exit != 4)]
        days = (min(hi, self.mt[-1]) - max(lo, self.mt[0])) / 86400
        if len(r) < 1:
            return {"n": 0}
        return {"n": len(r), "per_day": len(r) / days, "win": (r.r_net > 0).mean(), "r_gross": r.r_gross.mean(),
                "r_net": r.r_net.mean(), "usd_net": (r.pnl_total - r.fee_total).mean(), "risk_usd": r.risk.median(),
                "sum_r": r.r_net.sum()}


GRID = dict(htf=[3600, 14400], kinds=[("ob",), ("fvg",), ("sweep",)], mode=["limit", "grid", "choch"],
            depth=[0.0, 0.5], trend=[True, False], pd=[True, False], killzone=[True, False], rr=[1.5, 3.0],
            buf=[0.1, 0.3], min_risk=[2.0, 5.0])


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--grid", action="store_true")
    a = ap.parse_args()
    lab = Lab()
    print(f"minutes {len(lab.mt)}, days {lab.days:.0f}; zones 1h: "
          + str(lab.z[3600].kind.value_counts().to_dict()) + " 4h: " + str(lab.z[14400].kind.value_counts().to_dict()))
    if not a.grid:
        r = lab.run(BASE)
        print(BASE)
        print("in sample", lab.summary(r, 0, SPLIT))
        print("out of sample", lab.summary(r, SPLIT, 1 << 40))
        return
    rows = []
    for vals in itertools.product(*GRID.values()):
        cfg = {**BASE, **dict(zip(GRID, vals))}
        if cfg["mode"] != "limit" and cfg["depth"] > 0:
            continue
        r = lab.run(cfg)
        row = {k: (v if not isinstance(v, tuple) else "+".join(v)) for k, v in zip(GRID, vals)}
        row.update({f"is_{k}": v for k, v in lab.summary(r, 0, SPLIT).items()})
        row.update({f"oos_{k}": v for k, v in lab.summary(r, SPLIT, 1 << 40).items()})
        rows.append(row)
    df = pd.DataFrame(rows)
    df.to_csv(DATA.parent.parent / "smc_grid.csv", index=False)
    ok = df[(df.is_n >= 30)]
    cols = list(GRID) + ["is_n", "is_win", "is_r_gross", "is_r_net", "oos_n", "oos_per_day", "oos_win", "oos_r_net",
                         "oos_usd_net", "oos_risk_usd"]
    print(ok.sort_values("is_r_net", ascending=False)[cols].head(25).round(3).to_string())
    both = ok[(ok.is_r_net > 0) & (ok.oos_r_net > 0) & (ok.oos_n >= 30)]
    print(f"\nsettings with >= 30 trades in sample: {len(ok)}; positive net in and out of sample: {len(both)}")
    for col in ("kinds", "mode", "htf", "trend", "pd", "killzone", "rr", "buf", "min_risk"):
        print(ok.groupby(col)[["is_r_net", "oos_r_net"]].median().round(3).to_string(), "\n")


if __name__ == "__main__":
    main()
