"""Density cycle on Bybit XAUUSDT with limit orders only: breakout, bounce, bounce -> flip.

  breakout  a density that stood `stand` s was traded through (eaten). A cascade of limits
            waits for the pull-back towards the broken density (from `near` ticks beyond it
            down to the density price, bigger at the density); stop `stop` ticks back inside.
  bounce    as in `density_sim`, but every exit except the stop rests as a limit.
  flip      bounce; if it is stopped or the density is gone and the density then breaks, the
            breakout trade on the same density starts after the bounce exit.

Exits: target (resting limit), trailing (after `trail_start` ticks from the entry, trail
`trail` ticks behind the best price), fade (no new best for `stall_ms` after `fade_min`),
gone (bounce: the density was eaten or pulled), time. With `passive` an exit signal posts a
limit one tick on the other side of the last trade and waits; it fills when price trades one
tick through it and gives up to a market exit `chase` ticks further against. The stop is always
a market order.

    python -m aegis_lab.research.density_cycle [--grid breakout|bounce|flip]
"""

from __future__ import annotations

import argparse
import itertools
import pathlib
import pickle

import numba as nb
import numpy as np
import pandas as pd

from aegis_lab.research.density import DATA, EATEN, THROUGH

FEES = {"none": (0.0, 0.0), "promo": (0.0, 0.000275), "standard": (0.0002, 0.00055)}
EXITS = ("stop", "gone", "target", "trail", "fade", "time", "chase", "end")
SPLIT = pd.Timestamp("2026-07-01").value / 1e6
GONE = (0.3, 0.5)
INF = 1e18


def load(cache: pathlib.Path = DATA / "density_cycle.pkl") -> dict:
    files = sorted((DATA / "density").glob("*.npz"))
    if cache.exists():
        c = pickle.load(cache.open("rb"))
        if c["days"] == [f.stem for f in files]:
            return c
    trades, bounce, brk = [], [], []
    for di, f in enumerate(files):
        z = np.load(f)
        ep = {k[3:]: z[k] for k in z.files if k.startswith("ep_")}
        tt, tp = z["tr_t"], z["tr_p"].astype(np.int64)
        h_id, h_t, h_s, h_tr = z["h_id"], z["h_t"], z["h_size"], z["h_traded"]
        h_lo = np.searchsorted(h_id, np.arange(len(ep["p"]) + 1))
        trades.append((tt, tp))
        ok = (ep["peak"] >= 5.0) & (ep["t_end"] - ep["t0"] >= 10e3)
        for i in np.flatnonzero(ok):
            sl = slice(h_lo[i], h_lo[i + 1])
            ht, hs, htr = h_t[sl], h_s[sl], h_tr[sl]
            side, p, t_end = int(ep["side"][i]), int(ep["p"][i]), float(ep["t_end"][i])
            broke = ep["reason"][i] in (THROUGH, EATEN)
            dow = pd.Timestamp(t_end, unit="ms").dayofweek
            # bounce: qualifies after standing 10 s
            tq = ep["t0"][i] + 10e3
            k = np.searchsorted(ht, tq, "right") - 1
            size = hs[k]
            if size >= 5.0 and tq < t_end:
                gone = []
                for g in GONE:
                    low = np.flatnonzero(hs[k + 1 :] < g * size)
                    gone.append(ht[k + 1 + low[0]] if len(low) else t_end)
                bounce.append((di, i, side, p, tq, size, *gone, t_end, float(broke), dow))
            # breakout: stood 10 s before the first touch, then traded through
            tt0 = ep["t_touch"][i]
            if broke and not np.isnan(tt0) and tt0 - ep["t0"][i] >= 10e3:
                k0 = max(np.searchsorted(ht, tt0, "right") - 1, 0)
                eaten = htr[-1] - htr[k0]
                brk.append((di, i, side, p, t_end, ep["touch_size"][i], eaten / max(ep["touch_size"][i], 1e-9),
                            (t_end - tt0) / 1e3, dow))
    c = {
        "days": [f.stem for f in files], "trades": trades,
        "bounce": pd.DataFrame(bounce, columns=["day", "ep", "side", "p", "tq", "size", "gone_0", "gone_1",
                                                "t_end", "broke", "dow"]),
        "breakout": pd.DataFrame(brk, columns=["day", "ep", "side", "p", "t_end", "size", "eaten", "held_s",
                                               "dow"]),
    }
    pickle.dump(c, cache.open("wb"), protocol=5)
    return c


@nb.njit(cache=True)
def _sim(tt, tp, p, d, t_start, t_gone, offs, wts, stop, target, trail_start, trail, fade_min,
         stall_ms, max_hold_ms, wait_ms, exit_gone, passive, chase, stop_post, cat):
    """-> qty, avg entry (ticks), pnl (ticks x qty), qty closed at market, exit code, exit time."""
    n = len(tt)
    j = np.searchsorted(tt, t_start)
    no = len(offs)
    live = np.zeros(no, np.bool_)
    x0 = (tp[j] - p) * d if j < n else 0
    for k in range(no):
        live[k] = x0 > offs[k]
    qty = 0.0
    cost = 0.0
    left = 0.0
    t_first = 0.0
    best = 0
    t_best = 0.0
    exiting = False
    post = 0
    code = 0
    while j < n:
        t = tt[j]
        x = (tp[j] - p) * d
        if left == 0.0 and (t >= t_gone or t - t_start >= wait_ms):
            return 0.0, 0.0, 0.0, 0.0, -1, t
        if left > 0.0 and x <= -cat:
            return qty, cost / qty, left * x - cost, left, 0, t
        if left > 0.0 and x <= -stop and not (exiting and code == 0):
            if stop_post <= -(1 << 20):
                return qty, cost / qty, left * x - cost, left, 0, t
            # stop by limit: wait for the pull-back to `stop_post`, market only at `cat`
            exiting = True
            post = stop_post
            code = 0
            for k in range(no):
                live[k] = False
        if not exiting and t < t_gone and t - t_start < wait_ms:
            for k in range(no):
                if live[k] and x <= offs[k] - 1:
                    live[k] = False
                    qty += wts[k]
                    left += wts[k]
                    cost += wts[k] * offs[k]
                    if t_first == 0.0:
                        t_first = t
                        best = x
                        t_best = t
        if left > 0.0:
            avg = cost / qty
            if x > best:
                best = x
                t_best = t
            if exiting:
                if x >= post + 1:
                    return qty, avg, left * post - cost, 0.0, code, t
                if code != 0 and x <= post - chase:
                    return qty, avg, left * x - cost, left, 6, t
            else:
                if target > 0 and x >= target + 1:
                    return qty, avg, left * target - cost, 0.0, 2, t
                sig = -1
                if exit_gone and t >= t_gone:
                    sig = 1
                elif trail > 0 and best - avg >= trail_start and x <= best - trail:
                    sig = 3
                elif fade_min > 0 and best - avg >= fade_min and t - t_best >= stall_ms:
                    sig = 4
                elif t - t_first >= max_hold_ms:
                    sig = 5
                if sig >= 0:
                    if passive:
                        exiting = True
                        post = x + 1
                        code = sig
                        for k in range(no):
                            live[k] = False
                    else:
                        return qty, avg, left * x - cost, left, sig, t
        j += 1
    if qty > 0.0:
        x = (tp[n - 1] - p) * d
        return qty, cost / qty, left * x - cost, left, 7, tt[n - 1]
    return 0.0, 0.0, 0.0, 0.0, -1, tt[n - 1] if n else t_start


def cascade(n: int, near: int, far: int) -> tuple[np.ndarray, np.ndarray]:
    """`n` limits from `near` to `far` ticks (from the density), the biggest at `far`."""
    offs = np.unique(np.rint(np.linspace(far, near, n)).astype(np.int64))
    wts = np.linspace(1.0, 2.0, len(offs))
    if near > far:
        wts = wts[::-1]
    return offs, wts / wts.sum()


def _row(data, day, p, d, t0, t_gone, offs, wts, cfg):
    tt, tp = data["trades"][day]
    return _sim(tt, tp, p, d, t0, t_gone, offs, wts, cfg["stop"], cfg["target"], cfg["trail_start"],
                cfg["trail"], cfg["fade_min"], cfg["stall_ms"], cfg["max_hold_ms"], cfg["wait_ms"],
                cfg.get("exit_gone", False), cfg["passive"], cfg["chase"],
                cfg.get("stop_post", -(1 << 21)), cfg.get("cat", 1 << 20))


def run_breakout(data, sel, cfg, start=None) -> pd.DataFrame:
    # breakout of a bid density is a short (d = -1), of an ask density a long
    offs, wts = cascade(cfg["n"], cfg["near"], 0)
    out = []
    for k, r in enumerate(sel.itertuples()):
        t0 = r.t_end if start is None else max(r.t_end, start[k])
        res = _row(data, r.day, r.p, -1 if r.side == 0 else 1, t0, INF, offs, wts, cfg)
        if res[0] > 0:
            out.append((r.Index, r.p, r.t_end, *res, res[0] * (res[1] + cfg["stop"])))
    return _frame(out)


@nb.njit(cache=True)
def _reach(tt, tp, p, d, t0, t1):
    j = np.searchsorted(tt, t0)
    best = 1 << 30
    while j < len(tt) and tt[j] < t1:
        x = (tp[j] - p) * d
        if x < best:
            best = x
        j += 1
    return best


_REACH: dict = {}


def reach(data, sel, gone: float, wait_ms: float) -> np.ndarray:
    """Closest approach to the density before it is gone; cached per candidate."""
    key = (gone, wait_ms)
    cache = _REACH.setdefault(key, {})
    g = f"gone_{GONE.index(gone)}"
    out = np.empty(len(sel), np.int64)
    for k, r in enumerate(sel.itertuples()):
        v = cache.get(r.Index)
        if v is None:
            tt, tp = data["trades"][r.day]
            v = cache[r.Index] = _reach(tt, tp, r.p, 1 if r.side == 0 else -1, r.tq,
                                        min(getattr(r, g), r.tq + wait_ms))
        out[k] = v
    return out


def run_bounce(data, sel, cfg) -> pd.DataFrame:
    offs, wts = cascade(cfg["n"], cfg["width"], 1)
    g = f"gone_{GONE.index(cfg['gone'])}"
    sel = sel[reach(data, sel, cfg["gone"], cfg["wait_ms"]) <= cfg["width"] - 1]
    out = []
    for r in sel.itertuples():
        res = _row(data, r.day, r.p, 1 if r.side == 0 else -1, r.tq, getattr(r, g), offs, wts,
                   {**cfg, "exit_gone": True})
        if res[0] > 0:
            out.append((r.Index, r.p, r.tq, *res, res[0] * (res[1] + cfg["stop"])))
    return _frame(out)


def _frame(out) -> pd.DataFrame:
    return pd.DataFrame(out, columns=["cand", "p", "t", "qty", "avg", "pnl", "mkt", "exit", "t_exit",
                                      "risk"]).set_index("cand")


def priced(res: pd.DataFrame, fees: str) -> pd.DataFrame:
    """Fees in ticks: entries and limit exits pay maker, market exits taker."""
    maker, taker = FEES[fees]
    r = res.copy()
    r["fee"] = (maker * (2 * r.qty - r.mkt) + taker * r.mkt) * r.p
    r["net"] = r.pnl - r.fee
    return r


def summary(r: pd.DataFrame, days: int) -> dict:
    if len(r) == 0:
        return {"trades": 0}
    return {"trades": len(r), "per_day": len(r) / days, "win": (r.net > 0).mean(),
            "usd_gross": r.pnl.mean() / 100, "usd_net": r.net.mean() / 100,
            "r_net": (r.net / r.risk).mean(), "maker_exit": 1 - (r.mkt / r.qty).mean(),
            **{f"x_{e}": (r.exit == i).mean() for i, e in enumerate(EXITS)}}


BASE = dict(stand=10, min_size=20.0, n=4, width=10, near=10, gone=0.3, stop=30, target=1000,
            trail_start=50, trail=30, fade_min=0, stall_ms=30e3, max_hold_ms=1800e3,
            wait_ms=300e3, passive=True, chase=20, min_eaten=0.0, stop_post=-(1 << 21), cat=1 << 20)


def flip(data, bounce_sel, bounce_res, cfg) -> pd.DataFrame:
    """Bounce trades that ended by stop/gone/chase on a density that then broke -> breakout."""
    b = data["breakout"]
    key = b.set_index(["day", "ep"]).index
    m = bounce_res.exit.isin([0, 1, 6]).to_numpy()
    src = bounce_sel.loc[bounce_res.index[m]]
    pos = key.get_indexer(pd.MultiIndex.from_arrays([src.day, src.ep]))
    ok = pos >= 0
    sel = b.iloc[pos[ok]]
    return run_breakout(data, sel, cfg, start=bounce_res.t_exit.to_numpy()[m][ok])


def evaluate(data, kind: str, cfg: dict) -> dict:
    wd = pd.to_datetime(data["days"]).dayofweek < 5
    n_is = int((wd & (pd.to_datetime(data["days"]) < "2026-07-01")).sum())
    n_oos = int(wd.sum()) - n_is
    if kind == "breakout":
        b = data["breakout"]
        sel = b[(b.dow < 5) & (b["size"] >= cfg["min_size"]) & (b.eaten >= cfg["min_eaten"])]
        parts = {"": run_breakout(data, sel, cfg)}
    else:
        b = data["bounce"]
        sel = b[(b.dow < 5) & (b["size"] >= cfg["min_size"])]
        res = run_bounce(data, sel, cfg)
        parts = {"": res}
        if kind == "flip":
            fl = flip(data, sel, res, cfg)
            parts["flip_"] = fl
    row = {}
    for fees in FEES:
        for pre, res in parts.items():
            pr = priced(res, fees)
            row.update({f"{fees}_{pre}is_{k}": v for k, v in summary(pr[pr.t < SPLIT], n_is).items()})
            row.update({f"{fees}_{pre}oos_{k}": v for k, v in summary(pr[pr.t >= SPLIT], n_oos).items()})
        if kind == "flip":
            # per bounce trade: its own result plus the flip it led to
            both = pd.concat([priced(parts[""], fees), priced(parts["flip_"], fees)])
            for half, m in (("is", both.t < SPLIT), ("oos", both.t >= SPLIT)):
                row[f"{fees}_total_{half}_usd_per_day"] = both.net[m].sum() / 100 / (n_is if half == "is" else n_oos)
    return row


GRIDS = {
    "breakout": dict(min_size=[8.0, 20.0, 50.0], min_eaten=[0.0, 0.5], near=[0, 10, 30],
                     stop=[10, 30, 60], trail=[0, 30, 100], target=[100, 300, 1000], passive=[True, False]),
    "bounce": dict(min_size=[8.0, 20.0, 50.0], width=[10, 30, 60], stop=[10, 30],
                   trail=[0, 30, 100], target=[100, 300, 1000], passive=[True, False]),
    "flip": dict(min_size=[20.0, 50.0], width=[10, 30], stop=[10, 30], near=[0, 10],
                 trail=[30, 100], target=[300, 1000]),
}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--grid", choices=list(GRIDS))
    ap.add_argument("--kind", default="breakout", choices=list(GRIDS))
    a = ap.parse_args()
    data = load()
    print(f"days {len(data['days'])}, bounce candidates {len(data['bounce'])}, "
          f"breakouts {len(data['breakout'])}", flush=True)
    if not a.grid:
        row = evaluate(data, a.kind, BASE)
        print(pd.Series(row).round(3).to_string())
        return
    grid = GRIDS[a.grid]
    rows = []
    for vals in itertools.product(*grid.values()):
        cfg = {**BASE, **dict(zip(grid, vals))}
        rows.append({**dict(zip(grid, vals)), **evaluate(data, a.grid, cfg)})
    df = pd.DataFrame(rows)
    df.to_csv(DATA / f"density_cycle_{a.grid}.csv", index=False)
    for fees in ("none", "promo"):
        cols = list(grid) + [f"{fees}_{h}_{k}" for h in ("is", "oos") for k in ("trades", "win", "usd_net")]
        print(f"\n== {a.grid}, fees {fees}: best 10 in sample by $/oz ==")
        print(df.sort_values(f"{fees}_is_usd_net", ascending=False)[cols].head(10).round(3).to_string())
        ok = (df[f"{fees}_is_usd_net"] > 0) & (df[f"{fees}_oos_usd_net"] > 0)
        print(f"positive in and out of sample: {ok.sum()} of {len(df)}")


if __name__ == "__main__":
    main()
