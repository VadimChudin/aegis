"""Trade the bounce off a density the way the trader does it, on Bybit XAUUSDT book + tape.

A density qualifies once it has stood `stand` seconds holding at least `min_size` oz. Then a
cascade of `n` buy limits (sell limits for an ask density) rests in front of it from `width`
ticks away down to 1 tick in front, bigger the closer to the density. Unfilled orders are
cancelled when the density is gone (its size falls below `gone` x the size it qualified with)
or after `wait_ms`. The position exits on:
  stop    `stop` ticks behind the density (market)
  gone    the density was eaten or pulled while in the trade (market)
  target  part at `t1`, the rest at `t2` ticks from the density (limit)
  fade    after `fade_min` ticks in favour, no new extreme for `stall_ms` (market)
  time    `max_hold_ms` after the first fill (market)
Limit fills are conservative: price must trade 1 tick through the order.

    python -m aegis_lab.research.density_sim
"""

from __future__ import annotations

import argparse
import itertools
import pathlib
import pickle

import numba as nb
import numpy as np
import pandas as pd

from aegis_lab.research.density import DATA, THROUGH

GONE = (0.3, 0.5, 0.7)
STANDS = (5, 10, 30, 60)
EXITS = ("stop", "gone", "t2", "fade", "time", "end")
FEES = {"none": (0.0, 0.0), "promo": (0.0, 0.000275), "standard": (0.0002, 0.00055)}  # maker, taker


def load(cache: pathlib.Path = DATA / "density_sim.pkl") -> dict:
    files = sorted((DATA / "density").glob("*.npz"))
    if cache.exists():
        c = pickle.load(cache.open("rb"))
        if c["days"] == [f.stem for f in files]:
            return c
    days, cand = [], []
    for di, f in enumerate(files):
        z = np.load(f)
        ep = {k[3:]: z[k] for k in z.files if k.startswith("ep_")}
        tt, tp = z["tr_t"], z["tr_p"].astype(np.int64)
        h_id, h_t, h_s = z["h_id"], z["h_t"], z["h_size"]
        h_lo = np.searchsorted(h_id, np.arange(len(ep["p"]) + 1))
        st, ref_b, ref_a = z["s_t"], z["s_ref_b"], z["s_ref_a"]
        days.append((tt, tp))
        life = ep["t_end"] - ep["t0"]
        for i in np.flatnonzero((life >= STANDS[0] * 1e3) & (ep["peak"] >= 2.0)):
            ht, hs = h_t[h_lo[i] : h_lo[i + 1]], h_s[h_lo[i] : h_lo[i + 1]]
            side = int(ep["side"][i])
            t_end = ep["t_end"][i]
            for s in STANDS:
                tq = ep["t0"][i] + s * 1e3
                if tq >= t_end:
                    break
                k = np.searchsorted(ht, tq, "right") - 1
                size = hs[k]
                if size < 2.0:
                    continue
                gone = []
                for g in GONE:
                    low = np.flatnonzero(hs[k + 1 :] < g * size)
                    gone.append(ht[k + 1 + low[0]] if len(low) else t_end)
                j = np.searchsorted(tt, tq)
                if j >= len(tt):
                    continue
                d = 1 if side == 0 else -1
                m = min(max(np.searchsorted(st, tq) - 1, 0), len(st) - 1)
                cand.append((di, s, side, int(ep["p"][i]), tq, size,
                             size / (ref_b if side == 0 else ref_a)[m],
                             (tp[j] - ep["p"][i]) * d / 100, *gone,
                             float(ep["reason"][i] == THROUGH), (tq / 3.6e6) % 24,
                             pd.Timestamp(tq, unit="ms").dayofweek))
    cols = ["day", "stand", "side", "p", "tq", "size", "ratio", "dist", "gone_0", "gone_1",
            "gone_2", "through_end", "hour", "dow"]
    c = {"days": [f.stem for f in files], "trades": days, "cand": pd.DataFrame(cand, columns=cols)}
    pickle.dump(c, cache.open("wb"), protocol=5)
    return c


@nb.njit(cache=True)
def _sim(tt, tp, p, d, tq, t_gone, offs, wts, stop, t1, f1, t2, fade_min, stall_ms,
         max_hold_ms, wait_ms, exit_gone):
    """-> qty, avg entry (ticks from density), pnl (ticks x qty), qty closed at market, exit, hold."""
    j = np.searchsorted(tt, tq)
    n = len(tt)
    fee_m = 0.0  # limit fills and limit exits: counted outside from qty
    fee_t = 1.0  # market exits: `fees` accumulates the quantity closed at market
    no = len(offs)
    live = np.zeros(no, np.bool_)
    x0 = (tp[j] - p) * d if j < n else 0
    for k in range(no):
        live[k] = x0 > offs[k]
    qty = 0.0
    cost = 0.0
    pnl = 0.0
    fees = 0.0
    left = 0.0
    t_first = 0.0
    best = -1 << 30
    t_best = 0.0
    t1_done = False
    while j < n:
        t = tt[j]
        x = (tp[j] - p) * d
        if left == 0.0:
            if qty > 0.0:
                break
            if t >= t_gone or t - tq >= wait_ms:
                return 0.0, 0.0, 0.0, 0.0, -1, 0.0
        else:
            if t >= t_gone and exit_gone:
                pnl += left * (x * 1.0)
                fees += left * fee_t
                return qty, cost / qty, pnl, fees, 1, t - t_first
            if x <= -stop:
                pnl += left * x
                fees += left * fee_t
                return qty, cost / qty, pnl, fees, 0, t - t_first
        if not t1_done and (t < t_gone) and (t - tq < wait_ms):
            for k in range(no):
                if live[k] and x <= offs[k] - 1:
                    live[k] = False
                    qty += wts[k]
                    left += wts[k]
                    cost += wts[k] * offs[k]
                    fees += wts[k] * fee_m
                    if t_first == 0.0:
                        t_first = t
                        best = x
                        t_best = t
        if left > 0.0:
            if x > best:
                best = x
                t_best = t
            if not t1_done and f1 > 0.0 and x >= t1 + 1:
                t1_done = True
                part = qty * f1
                pnl += part * t1
                fees += part * fee_m
                left -= part
            if x >= t2 + 1:
                pnl += left * t2
                fees += left * fee_m
                return qty, cost / qty, pnl, fees, 2, t - t_first
            if best - cost / qty >= fade_min and t - t_best >= stall_ms:
                pnl += left * x
                fees += left * fee_t
                return qty, cost / qty, pnl, fees, 3, t - t_first
            if t - t_first >= max_hold_ms:
                pnl += left * x
                fees += left * fee_t
                return qty, cost / qty, pnl, fees, 4, t - t_first
        j += 1
    if qty > 0.0:
        x = (tp[n - 1] - p) * d
        pnl += left * x
        fees += left * fee_t
        return qty, cost / qty, pnl, fees, 5, tt[n - 1] - t_first
    return 0.0, 0.0, 0.0, 0.0, -1, 0.0


@nb.njit(cache=True)
def _reach(tt, tp, p, d, tq, t_stop):
    """Closest price came to the density (ticks, favourable side positive) in [tq, t_stop)."""
    j = np.searchsorted(tt, tq)
    best = 1 << 30
    while j < len(tt) and tt[j] < t_stop:
        x = (tp[j] - p) * d
        if x < best:
            best = x
        j += 1
    return best


def reach(data: dict, c: pd.DataFrame, gone: float, wait_ms: float) -> np.ndarray:
    g = c[f"gone_{GONE.index(gone)}"].to_numpy()
    out = np.empty(len(c), np.int64)
    for k, r in enumerate(zip(c.day.to_numpy(), c.side.to_numpy(), c.p.to_numpy(), c.tq.to_numpy())):
        tt, tp = data["trades"][r[0]]
        out[k] = _reach(tt, tp, r[2], 1 if r[1] == 0 else -1, r[3], min(g[k], r[3] + wait_ms))
    return out


def cascade(n: int, width: int) -> tuple[np.ndarray, np.ndarray]:
    offs = np.unique(np.rint(np.linspace(1, width, n)).astype(np.int64))
    wts = np.arange(len(offs), 0, -1, dtype=float)  # biggest next to the density
    return offs, wts / wts.sum()


def run(data: dict, sel: pd.DataFrame, cfg: dict) -> pd.DataFrame:
    offs, wts = cascade(cfg["n"], cfg["width"])
    g = GONE.index(cfg["gone"])
    out = []
    for r in sel.itertuples():
        tt, tp = data["trades"][r.day]
        d = 1 if r.side == 0 else -1
        qty, avg, pnl, mkt, ex, hold = _sim(
            tt, tp, r.p, d, r.tq, getattr(r, f"gone_{g}"), offs, wts, cfg["stop"], cfg["t1"],
            cfg["f1"], cfg["t2"], cfg["fade_min"], cfg["stall_ms"], cfg["max_hold_ms"],
            cfg["wait_ms"], cfg["exit_gone"])
        if qty > 0:
            out.append((r.Index, r.p, r.tq, qty, avg, pnl, mkt, ex, hold, qty * (avg + cfg["stop"])))
    return pd.DataFrame(out, columns=["cand", "p", "tq", "qty", "avg", "pnl", "mkt", "exit", "hold",
                                      "risk"]).set_index("cand")


def priced(res: pd.DataFrame, fees: str) -> pd.DataFrame:
    """Fees in ticks: entries and limit exits pay maker, market exits pay taker."""
    maker, taker = FEES[fees]
    r = res.copy()
    r["fee"] = (maker * (2 * r.qty - r.mkt) + taker * r.mkt) * r.p
    r["r_gross"] = r.pnl / r.risk
    r["r_net"] = (r.pnl - r.fee) / r.risk
    return r


BASE = dict(stand=10, min_size=8.0, n=4, width=30, gone=0.5, stop=10, t1=100, f1=0.5, t2=300,
            fade_min=50, stall_ms=30e3, max_hold_ms=1800e3, wait_ms=1800e3, exit_gone=True)


def select(c: pd.DataFrame, cfg: dict) -> pd.DataFrame:
    m = (c.stand == cfg["stand"]) & (c["size"] >= cfg["min_size"]) & (c.dist <= cfg.get("max_dist", 5))
    m &= c.dow < 5 if cfg.get("weekdays", True) else True
    return c[m]


def summary(res: pd.DataFrame, days: int) -> dict:
    if len(res) == 0:
        return {"trades": 0}
    return {"trades": len(res), "per_day": len(res) / days, "win": (res.pnl > res.fee).mean(),
            "r_gross": res.r_gross.mean(), "r_net": res.r_net.mean(),
            "usd_gross": res.pnl.mean() / 100, "usd_net": (res.pnl - res.fee).mean() / 100,
            **{f"x_{e}": (res.exit == i).mean() for i, e in enumerate(EXITS)}}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--fees", default="promo", choices=list(FEES))
    ap.add_argument("--grid", action="store_true")
    a = ap.parse_args()
    data = load()
    c = data["cand"]
    c = c[(c.stand == BASE["stand"]) & (c.dow < 5) & (c["size"] >= 8.0)]
    split = pd.Timestamp("2026-07-01").value / 1e6
    wd = pd.to_datetime(data["days"]).dayofweek < 5
    n_is = int((wd & (pd.to_datetime(data["days"]) < "2026-07-01")).sum())
    n_oos = int(wd.sum()) - n_is
    print(f"weekdays {n_is} in sample (Mar-Jun) + {n_oos} out of sample (Jul-Sep), candidates {len(c)}")
    if not a.grid:
        res = priced(run(data, select(c, BASE), BASE), a.fees)
        print(BASE)
        print(pd.Series(summary(res, n_is + n_oos)).round(3).to_string())
        return
    grid = dict(min_size=[8.0, 20.0, 30.0, 50.0], width=[10, 30, 60], stop=[10, 30, 60],
                gone=[0.3, 0.5], exit_gone=[True, False], t2=[100, 300, 600, 1000])
    # only densities price came close enough to fill the outermost order can trade
    near = {g: reach(data, c, g, BASE["wait_ms"]) for g in grid["gone"]}
    rows = []
    for vals in itertools.product(*grid.values()):
        cfg = {**BASE, **dict(zip(grid, vals))}
        cfg["t1"] = min(cfg["t1"], cfg["t2"] // 2)
        sel = select(c, cfg)
        sel = sel[near[cfg["gone"]][c.index.get_indexer(sel.index)] <= cfg["width"] - 1]
        raw = run(data, sel, cfg)
        row = dict(zip(grid, vals))
        for fees in FEES:
            res = priced(raw, fees)
            tr, te = res[res.tq < split], res[res.tq >= split]
            row.update({f"{fees}_is_{k}": v for k, v in summary(tr, n_is).items()})
            row.update({f"{fees}_oos_{k}": v for k, v in summary(te, n_oos).items()})
        rows.append(row)
    df = pd.DataFrame(rows)
    df.to_csv(DATA / "density_grid.csv", index=False)
    for fees in FEES:
        cols = list(grid) + [f"{fees}_is_{k}" for k in ("trades", "win", "usd_net", "r_net")]
        cols += [f"{fees}_oos_{k}" for k in ("trades", "win", "usd_net", "r_net")]
        print(f"\n== {fees}: best 10 in sample by $ per oz ==")
        print(df.sort_values(f"{fees}_is_usd_net", ascending=False)[cols].head(10).round(3).to_string())


if __name__ == "__main__":
    main()
