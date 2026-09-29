"""Wide search for smart-money settings on gold, with honest checks.

Adds to `smc.py`:
  zones     ob_fvg (order block overlapping a same-side FVG), sweep_ob (order block whose move
            started with a liquidity sweep), and a liquidity target for every zone (the range
            extreme of the last `LIQ_BARS` higher-timeframe bars on the other side)
  entries   choch_limit: after the 1m CHoCH, a limit on the retest of the broken 1m swing
            (maker entry instead of market)
  exits     target in R or at the liquidity, breakeven, trailing stop in R
Checks:
  split     tune on Jan-May, report Jun-Sep
  auto      walk-forward: every month trade the setting that was best over the previous 3
            months (what an "Auto" switch would do)
  control   the chosen settings on zones moved to random times near price

    python -m aegis_lab.research.smc_search [--samples 4000]
"""

from __future__ import annotations

import argparse
import pathlib

import numba as nb
import numpy as np
import pandas as pd

from aegis_lab.research import smc

LIQ_BARS = 24
OUT = pathlib.Path.home() / "aegis-data"


def derived(z: pd.DataFrame, bars: pd.DataFrame) -> pd.DataFrame:
    z = z.copy()
    h, l = bars.h.to_numpy(), bars.l.to_numpy()
    hi = pd.Series(h).rolling(LIQ_BARS, min_periods=1).max().to_numpy()
    lo = pd.Series(l).rolling(LIQ_BARS, min_periods=1).min().to_numpy()
    b = z.bar.to_numpy()
    z["liq"] = np.where(z.dir > 0, hi[b], lo[b])
    extra = []
    obs, fvgs, sws = z[z.kind == "ob"], z[z.kind == "fvg"], z[z.kind == "sweep"]
    for _, r in obs.iterrows():
        f = fvgs[(fvgs.dir == r.dir) & (fvgs.bar >= r.bar - 3) & (fvgs.bar <= r.bar)]
        if ((f.bot < r.top) & (f.top > r.bot)).any():
            extra.append({**r.to_dict(), "kind": "ob_fvg"})
        s = sws[(sws.dir == r.dir) & (sws.bar >= r.bar - 8) & (sws.bar <= r.bar)]
        if len(s):
            extra.append({**r.to_dict(), "kind": "sweep_ob"})
    return pd.concat([z, pd.DataFrame(extra)], ignore_index=True)


@nb.njit(cache=True)
def _sim(mt, mo, mh, ml, mc, hour_end, active, i0, i_exp, d, top, bot, liq, mode, n, depth, grid_depth,
         buf, rr, tgt_liq, min_rr, be, trail_start, trail, max_hold, fk, retest_m, tick):
    """Long-space simulation of one zone.
    -> qty, avg, stop0, pnl ($ x qty), maker entry, market exit, exit code, entry index.
    exit: 0 stop, 1 target, 2 breakeven, 3 trailing, 4 time, 5 end of data."""
    nm = len(mt)
    if d > 0:
        edge, far, lq = top, bot, liq
    else:
        edge, far, lq = -bot, -top, -liq
    zh = edge - far
    stop = far - buf
    prices = np.zeros(max(n, 1))
    wts = np.zeros(max(n, 1))
    nl = 0
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
    target = 0.0
    if nl > 0:
        avg_plan = 0.0
        for k in range(nl):
            avg_plan += prices[k] * wts[k]
        target = avg_plan + rr * (avg_plan - stop)
        if tgt_liq and lq - avg_plan >= min_rr * (avg_plan - stop):
            target = lq - tick
    filled = np.zeros(max(nl, 1), np.bool_)
    qty = 0.0
    cost = 0.0
    t_in = -1
    inside = False
    low_in = 1e18
    low_i = -1
    last_sh = 1e18
    last_sh_i = -1
    lim = 0.0
    t_ch = -1
    best = -1e18
    risk = 0.0
    init_stop = stop
    for i in range(i0, nm):
        if d > 0:
            op, hi, lo, cl = mo[i], mh[i], ml[i], mc[i]
        else:
            op, hi, lo, cl = -mo[i], -ml[i], -mh[i], -mc[i]
        if qty == 0.0:
            if t_ch < 0 and (i >= i_exp or (hour_end[i] and cl < far)):
                return 0.0, 0.0, 0.0, 0.0, False, False, -1, -1
            if mode <= 1:
                if active[i]:
                    for k in range(nl):
                        if not filled[k] and lo <= prices[k] - tick:
                            filled[k] = True
                            qty += wts[k]
                            cost += wts[k] * prices[k]
                            t_in = i
                if qty > 0.0:
                    risk = cost / qty - stop
                    best = cost / qty
                    if lo <= stop:
                        return qty, cost / qty, stop, qty * stop - cost, True, True, 0, t_in
                continue
            if t_ch >= 0:
                # waiting for the retest of the broken swing (mode 3)
                if i - t_ch > retest_m or hi >= target:
                    return 0.0, 0.0, 0.0, 0.0, False, False, -1, -1
                if lo <= lim - tick:
                    qty = 1.0
                    cost = lim
                    t_in = i
                    init_stop = stop
                    risk = lim - stop
                    best = lim
                    if lo <= stop:
                        return qty, lim, stop, stop - lim, True, True, 0, t_in
                continue
            if lo <= edge:
                inside = True
            if not inside:
                continue
            if lo < low_in:
                low_in = lo
                low_i = i
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
                return 0.0, 0.0, 0.0, 0.0, False, False, -1, -1
            if active[i] and last_sh_i >= 0 and last_sh_i < low_i and cl > last_sh:
                stop = low_in - buf
                if mode == 2:
                    e = cl + tick
                    if e - stop <= 0.02:
                        continue
                    qty = 1.0
                    cost = e
                    t_in = i
                    init_stop = stop
                    risk = e - stop
                    best = e
                    target = e + rr * risk
                    if tgt_liq and lq - e >= min_rr * risk:
                        target = lq - tick
                else:
                    lim = last_sh
                    if lim - stop <= 0.02:
                        continue
                    t_ch = i
                    target = lim + rr * (lim - stop)
                    if tgt_liq and lq - lim >= min_rr * (lim - stop):
                        target = lq - tick
            continue
        # in a position
        if mode <= 1 and hi < target and active[i]:
            for k in range(nl):
                if not filled[k] and lo <= prices[k] - tick:
                    filled[k] = True
                    qty += wts[k]
                    cost += wts[k] * prices[k]
            risk = cost / qty - (far - buf)
        avg = cost / qty
        if lo <= stop:
            code = 0
            if stop >= avg - 1e-9:
                code = 2
            if trail > 0 and stop > avg + 1e-9:
                code = 3
            # a stop moved on the previous bar's high can be gapped through at this bar's open
            px = min(stop, op) if i > t_in else stop
            stop0 = far - buf if mode <= 1 else init_stop
            return qty, avg, stop0, qty * px - cost, mode != 2, True, code, t_in
        if hi >= target + tick:
            stop0 = far - buf if mode <= 1 else init_stop
            return qty, avg, stop0, qty * target - cost, mode != 2, False, 1, t_in
        if hi > best:
            best = hi
        if be > 0 and best >= avg + be * risk and stop < avg:
            stop = avg
        if trail > 0 and best >= avg + trail_start * risk:
            ns = best - trail * risk
            if ns > stop:
                stop = ns
        if mt[i] - mt[t_in] >= max_hold:
            return qty, avg, far - buf if mode <= 1 else init_stop, qty * cl - cost, mode != 2, True, 4, t_in
    if qty > 0.0:
        cl = mc[nm - 1] if d > 0 else -mc[nm - 1]
        return qty, cost / qty, far - buf if mode <= 1 else init_stop, qty * cl - cost, mode != 2, True, 5, t_in
    return 0.0, 0.0, 0.0, 0.0, False, False, -1, -1


class Search(smc.Lab):
    def __init__(self):
        super().__init__()
        self.mo = self.m.o.to_numpy()
        for sec in (3600, 14400):
            self.z[sec] = derived(self.z[sec], smc.resample(self.m, sec))

    def run(self, cfg: dict, z: pd.DataFrame | None = None) -> pd.DataFrame:
        z = self.z[cfg["htf"]] if z is None else z
        z = z[z.kind.isin(cfg["kinds"]) & (z.size_atr >= cfg["min_size"]) & (z.size_atr <= cfg["max_size"])]
        if cfg["trend"]:
            z = z[z.trend == z.dir]
        if cfg["pd"]:
            z = z[z.pd_ok == 1]
        active = self.kz if cfg["killzone"] else self.all
        he = self.hour_end[cfg["htf"]]
        valid = z.valid.to_numpy()
        i0s = np.searchsorted(self.mt, valid)
        i1s = np.searchsorted(self.mt, valid + cfg["max_age"])
        mode = smc.MODES_ALL[cfg["mode"]]
        rows = []
        taken_until = {1: -1, -1: -1}
        for r, i0, i1 in zip(z.itertuples(), i0s, i1s):
            if i0 >= len(self.mt):
                continue
            buf = cfg["buf"] * r.atr
            qty, avg, stop0, pnl, maker_in, mkt_out, ex, ti = _sim(
                self.mt, self.mo, self.mh, self.ml, self.mc, he, active, i0, i1, r.dir, r.top, r.bot, r.liq, mode,
                cfg["n"], cfg["depth"], cfg["grid_depth"], buf, cfg["rr"], cfg["tgt_liq"], cfg["min_rr"],
                cfg["be"], cfg["trail_start"], cfg["trail"], cfg["max_hold"], cfg["fk"], cfg["retest_m"], 0.01)
            if qty <= 0:
                continue
            # one position per direction: the same move often sits in several overlapping zones
            if ti <= taken_until[r.dir]:
                continue
            if mode == 1:
                plan = self.plan_risk(cfg, r, buf)
            else:
                plan = avg - stop0 if mode == 0 else abs(avg - stop0)
            if plan < cfg["min_risk"]:
                continue
            taken_until[r.dir] = ti + 1
            px = abs(avg)
            fee = ((smc.MAKER if maker_in else smc.TAKER) * qty + (smc.TAKER if mkt_out else smc.MAKER) * qty) * px
            rows.append((self.mt[ti], r.dir, pnl, fee, plan * qty if mode != 1 else plan, ex))
        df = pd.DataFrame(rows, columns=["t", "dir", "pnl", "fee", "risk", "exit"])
        df["r_gross"] = df.pnl / df.risk
        df["r_net"] = (df.pnl - df.fee) / df.risk
        return df.sort_values("t").reset_index(drop=True)


smc.MODES_ALL = {"limit": 0, "grid": 1, "choch": 2, "choch_limit": 3}

SPACE = dict(
    htf=[3600, 3600, 14400], kinds=[("ob",), ("fvg",), ("sweep",), ("ob_fvg",), ("sweep_ob",), ("ob", "fvg"),
                                     ("ob", "sweep")],
    mode=["limit", "grid", "choch", "choch_limit", "choch_limit"], depth=[0.0, 0.25, 0.5], n=[3, 5],
    grid_depth=[0.5, 1.0], buf=[0.05, 0.1, 0.2, 0.3], rr=[1.5, 2.0, 3.0, 5.0], tgt_liq=[False, True],
    min_rr=[1.0, 2.0], be=[0.0, 0.0, 1.0], trail_start=[1.0, 2.0], trail=[0.0, 0.0, 0.5, 1.0],
    trend=[False, True], pd=[False, True], killzone=[False, True], min_risk=[2.0, 5.0, 8.0, 12.0],
    max_age=[24 * 3600, 72 * 3600], fk=[2, 3, 5], retest_m=[15, 60, 240], min_size=[0.1, 0.2],
    max_size=[3.0], max_hold=[86400],
)


def sample(rng: np.random.Generator) -> dict:
    return {k: v[rng.integers(len(v))] for k, v in SPACE.items()}


def month(t: np.ndarray) -> np.ndarray:
    return pd.to_datetime(t, unit="s").strftime("%Y-%m").to_numpy()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--samples", type=int, default=4000)
    ap.add_argument("--seed", type=int, default=1)
    a = ap.parse_args()
    s = Search()
    rng = np.random.default_rng(a.seed)
    cfgs, runs = [], []
    for k in range(a.samples):
        cfg = sample(rng)
        cfgs.append(cfg)
        runs.append(s.run(cfg))
        if k % 500 == 0:
            print(f"{k} settings", flush=True)
    rows = []
    for k, (cfg, r) in enumerate(zip(cfgs, runs)):
        ins, oos = r[r.t < smc.SPLIT], r[r.t >= smc.SPLIT]
        rows.append({"id": k, **{c: ("+".join(v) if isinstance(v, tuple) else v) for c, v in cfg.items()},
                     "is_n": len(ins), "is_g": ins.r_gross.mean(), "is_r": ins.r_net.mean(), "is_sum": ins.r_net.sum(),
                     "oos_n": len(oos), "oos_g": oos.r_gross.mean(), "oos_r": oos.r_net.mean(),
                     "oos_sum": oos.r_net.sum(), "oos_win": (oos.r_net > 0).mean()})
    df = pd.DataFrame(rows)
    df.to_csv(OUT / "smc_search.csv", index=False)
    ok = df[(df.is_n >= 30) & (df.oos_n >= 15)]
    print(f"\nsettings with >= 30 trades in sample and >= 15 out of sample: {len(ok)}")
    top = ok.sort_values("is_r", ascending=False).head(20)
    cols = ["id", "htf", "kinds", "mode", "rr", "tgt_liq", "be", "trail", "trend", "pd", "killzone", "min_risk",
            "is_n", "is_r", "oos_n", "oos_r", "oos_win"]
    print(top[cols].round(3).to_string())
    print(f"top 20 in sample: out-of-sample mean {top.oos_r.mean():+.3f} R, positive {int((top.oos_r > 0).sum())}/20")
    pos = ok[ok.is_r > 0]
    print(f"in-sample positive: {len(pos)}; of them positive out of sample: {(pos.oos_r > 0).mean():.1%}; "
          f"base rate out of sample: {(ok.oos_r > 0).mean():.1%}")
    for c in ("kinds", "mode", "htf", "tgt_liq", "be", "trail", "trend", "pd", "killzone", "min_risk", "rr"):
        print(ok.groupby(c)[["is_g", "is_r", "oos_g", "oos_r"]].median().round(3).to_string(), "\n")

    # walk-forward "Auto": each month take the best setting of the previous 3 months
    months = sorted(set(month(s.mt)))
    trades = [(r.assign(m=month(r.t.to_numpy())) if len(r) else r.assign(m=[])) for r in runs]
    auto = []
    for mi in range(3, len(months)):
        train, test = months[mi - 3 : mi], months[mi]
        score = []
        for r in trades:
            tr = r[r.m.isin(train)]
            score.append(tr.r_net.sum() if len(tr) >= 15 else -np.inf)
        best = int(np.argmax(score))
        te = trades[best][trades[best].m == test]
        auto.append((test, best, score[best], len(te), te.r_net.sum()))
        print(f"auto {test}: setting {best} (train sum {score[best]:+.1f} R) -> {len(te)} trades, {te.r_net.sum():+.2f} R",
              flush=True)
    au = pd.DataFrame(auto, columns=["month", "id", "train", "n", "sum_r"])
    print(f"auto total: {au.n.sum()} trades, {au.sum_r.sum():+.2f} R, {au.sum_r.sum() / max(au.n.sum(), 1):+.3f} R/trade")


if __name__ == "__main__":
    main()
