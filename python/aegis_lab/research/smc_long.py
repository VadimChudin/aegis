"""Smart-money zones on 17 years of XAUUSD 1m (2008 … 2025-02), same engine as smc.py.

smc.md tested order blocks, FVGs and sweeps only on ~9 months of Binance XAUUSDT. This runs the
same zone builder (`smc.zones`) and the same 1m simulator (`smc._sim`) on the MetaTrader CFD
history, with RoboForex-like costs instead of Binance fees:

  cost      COST $/oz per round trip (spread + commission), plus SLIP $/oz on market entries
  split     settings are chosen on 2008-2014 only; 2015-2025 is never used for selection
  control   each chosen setting is re-run on random zones: same count, sizes and directions,
            placed at random bars near the price of that bar, so only "where" is random
  clock     New York (data server = NY + 7 h). Killzones: London 02-05, New York 07-11.

    python -m aegis_lab.research.smc_long [--data DIR]
"""

from __future__ import annotations

import argparse
import itertools
import pathlib

import numpy as np
import pandas as pd

from . import smc
from .gold_sessions import minutes as xau_minutes

COST = 0.25
SLIP = 0.05
SPLIT = int(pd.Timestamp("2015-01-01").value // 10**9)
PERIODS = (("2008-11", "2008", "2012"), ("2012-14", "2012", "2015"), ("2015-18", "2015", "2019"),
           ("2019-22", "2019", "2023"), ("2023-25", "2023", "2026"))
KILLZONES_NY = ((2, 5), (7, 11))
GRID = dict(htf=[3600, 14400], kinds=[("ob",), ("fvg",), ("sweep",)],
            entry=["limit", "limit_mid", "grid", "choch"], trend=[True, False], killzone=[True, False],
            rr=[1.5, 3.0])
ENTRY = {"limit": ("limit", 0.0), "limit_mid": ("limit", 0.5), "grid": ("grid", 0.0), "choch": ("choch", 0.0)}


def _ts(s: str) -> int:
    return int(pd.Timestamp(s).value // 10**9)


class LongLab:
    """smc.Lab on a NY-clock minute frame (t = seconds of the naive NY timestamp)."""

    def __init__(self, m: pd.DataFrame, tick: float = 0.01):
        m = m.assign(v=0.0)
        m["t"] = m.index.to_numpy().astype("datetime64[s]").astype(np.int64)
        m = m.reset_index(drop=True)[["t", "o", "h", "l", "c", "v"]]
        self.m, self.tick = m, tick
        self.mt = m.t.to_numpy()
        self.mh, self.ml, self.mc = m.h.to_numpy(), m.l.to_numpy(), m.c.to_numpy()
        hour = (self.mt // 3600) % 24
        dow = ((self.mt // 86400) + 3) % 7
        kz = np.zeros(len(m), bool)
        for a, b in KILLZONES_NY:
            kz |= (hour >= a) & (hour < b)
        self.kz = kz & (dow < 5)
        self.all = np.ones(len(m), bool)
        self.bars, self.z, self.hour_end = {}, {}, {}
        for sec in GRID["htf"]:
            self.bars[sec] = smc.resample(m, sec)
            self.z[sec] = smc.zones(self.bars[sec], sec)
            self.hour_end[sec] = ((self.mt + 60) % sec) == 0

    def select(self, cfg: dict) -> pd.DataFrame:
        z = self.z[cfg["htf"]]
        z = z[z.kind.isin(cfg["kinds"]) & (z.size_atr >= cfg["min_size"]) & (z.size_atr <= cfg["max_size"])]
        if cfg["trend"]:
            z = z[z.trend == z.dir]
        return z

    def random_zones(self, z: pd.DataFrame, htf: int, seed: int) -> pd.DataFrame:
        """Same sizes and directions, at random bars, the zone edge 0-1.5 ATR from that bar's close."""
        rng = np.random.default_rng(seed)
        b = self.bars[htf].dropna(subset=["atr"])
        pick = b.iloc[rng.integers(0, len(b), len(z))]
        a = pick.atr.to_numpy()
        size = z.size_atr.to_numpy() * a
        d = z.dir.to_numpy()
        gap = rng.uniform(0.0, 1.5, len(z)) * a
        top = np.where(d > 0, pick.c.to_numpy() - gap, pick.c.to_numpy() + gap + size)
        return pd.DataFrame({"kind": z.kind.to_numpy(), "dir": d, "valid": pick.t.to_numpy() + htf, "top": top,
                             "bot": top - size, "atr": a}).sort_values("valid")

    def run(self, cfg: dict, z: pd.DataFrame | None = None) -> pd.DataFrame:
        z = self.select(cfg) if z is None else z
        mode_name, depth = ENTRY[cfg["entry"]]
        mode = smc.MODES[mode_name]
        active = self.kz if cfg["killzone"] else self.all
        he = self.hour_end[cfg["htf"]]
        valid, d, top, bot, atr = (z[c].to_numpy() for c in ("valid", "dir", "top", "bot", "atr"))
        i0s = np.searchsorted(self.mt, valid)
        i1s = np.searchsorted(self.mt, valid + cfg["max_age"])
        # a trade holds at most max_hold after an entry that is at most max_age after the zone
        i_end = np.searchsorted(self.mt, valid + cfg["max_age"] + cfg["max_hold"] + 3600)
        plan_cfg = {**cfg, "mode": mode_name, "depth": depth}
        rows = []
        for k in range(len(z)):
            if i0s[k] >= len(self.mt):
                continue
            buf = cfg["buf"] * atr[k]
            qty, avg, stop, pnl, _mkt, _exit, ti = smc._sim(
                self.mt, self.mh, self.ml, self.mc, he, active, i0s[k], i_end[k], i1s[k], d[k], top[k], bot[k],
                mode, cfg["n"], depth, cfg["grid_depth"], buf, cfg["rr"], cfg["be"], cfg["max_hold"], cfg["fk"],
                self.tick)
            if qty <= 0:
                continue
            risk = qty * (avg - stop)
            zone = pd.Series({"top": top[k], "bot": bot[k]})
            plan = smc.Lab.plan_risk(plan_cfg, zone, buf) if mode <= 1 else risk
            if (plan if mode <= 1 else risk / qty) < cfg["min_risk"]:
                continue
            fee = qty * COST + (qty * SLIP if mode == 2 else 0.0)
            rows.append((self.mt[ti], pnl / plan, (pnl - fee) / plan))
        return pd.DataFrame(rows, columns=["t", "r_gross", "r_net"])


def stats(r: pd.DataFrame, lo: int, hi: int) -> dict:
    x = r[(r.t >= lo) & (r.t < hi)].r_net
    if len(x) < 2:
        return {"n": len(x), "r": np.nan, "t": np.nan}
    return {"n": len(x), "r": x.mean(), "t": x.mean() / x.std() * np.sqrt(len(x))}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default=str(pathlib.Path.home() / "aegis-data" / "hf-xau"))
    a = ap.parse_args()
    lab = LongLab(xau_minutes(pathlib.Path(a.data)))
    for sec in GRID["htf"]:
        print(f"zones {sec // 3600}h: {lab.z[sec].kind.value_counts().to_dict()}")

    rows = []
    for vals in itertools.product(*GRID.values()):
        cfg = {**smc.BASE, **dict(zip(GRID, vals)), "min_risk": 1.0}
        r = lab.run(cfg)
        g, go = r[r.t < SPLIT].r_gross, r[r.t >= SPLIT].r_gross
        rows.append({**{k: ("+".join(v) if isinstance(v, tuple) else v) for k, v in zip(GRID, vals)},
                     "is_gross": g.mean() if len(g) else np.nan,
                     "oos_gross": go.mean() if len(go) else np.nan,
                     **{f"is_{k}": v for k, v in stats(r, 0, SPLIT).items()},
                     **{f"oos_{k}": v for k, v in stats(r, SPLIT, 1 << 40).items()}})
    df = pd.DataFrame(rows)
    df.to_csv(pathlib.Path(a.data) / "smc_long_grid.csv", index=False)
    ok = df[df.is_n >= 100]
    fmt = {"is_gross": "{:+.3f}", "oos_gross": "{:+.3f}", "is_r": "{:+.3f}", "is_t": "{:+.1f}", "oos_r": "{:+.3f}",
           "oos_t": "{:+.1f}"}
    show = ok.sort_values("is_r", ascending=False).head(15).copy()
    for c, f in fmt.items():
        show[c] = show[c].map(f.format)
    print(f"\n## Top 15 of {len(ok)} settings (≥ 100 trades 2008-14) by net R 2008-14\n")
    print(show.to_markdown(index=False))
    both = ok[(ok.is_r > 0) & (ok.oos_r > 0)]
    print(f"\nPositive net in both 2008-14 and 2015-25: {len(both)} of {len(ok)}; "
          f"positive gross 2008-14: {(ok.is_gross > 0).sum()}, gross in both: "
          f"{((ok.is_gross > 0) & (ok.oos_gross > 0)).sum()}")
    print("\n## Order block + CHoCH (the lead from smc.md)\n")
    obc = df[(df.kinds == "ob") & (df.entry == "choch") & df.trend]
    print(obc[["htf", "killzone", "rr", "is_n", "is_gross", "is_r", "oos_n", "oos_gross", "oos_r"]]
          .round(3).to_markdown(index=False))
    print("\n## Median net R by choice (all settings)\n")
    for col in ("kinds", "entry", "htf", "trend", "killzone", "rr"):
        print(ok.groupby(col)[["is_gross", "oos_gross", "is_r", "oos_r"]].median().round(3).to_markdown(), "\n")

    print("## The 3 best in-sample settings, by period, against random zones\n")
    print("| Setting | " + " | ".join(p for p, _, _ in PERIODS) + " | 2015-25 random zones (5 seeds) |")
    print("|---|" + "---|" * (len(PERIODS) + 1))
    for _, row in ok.sort_values("is_r", ascending=False).head(3).iterrows():
        cfg = {**smc.BASE, "min_risk": 1.0, "htf": int(row.htf), "kinds": tuple(row.kinds.split("+")),
               "entry": row.entry, "trend": bool(row.trend), "killzone": bool(row.killzone), "rr": float(row.rr)}
        r = lab.run(cfg)
        cells = []
        for _, lo, hi in PERIODS:
            s = stats(r, _ts(lo), _ts(hi))
            cells.append(f"{s['r']:+.3f} (n {s['n']})")
        z = lab.select(cfg)
        ctrl = [stats(lab.run(cfg, lab.random_zones(z, cfg["htf"], seed)), SPLIT, 1 << 40)["r"] for seed in range(5)]
        name = f"{row.kinds} {row.entry} {int(row.htf) // 3600}h trend={row.trend} kz={row.killzone} rr={row.rr}"
        print(f"| {name} | " + " | ".join(cells) + f" | {min(ctrl):+.3f} … {max(ctrl):+.3f} |")


if __name__ == "__main__":
    main()
