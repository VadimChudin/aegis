"""Remaining ICT plan items on gold: IPDA 20/40/60-day liquidity, SMT gold vs silver, and the
breaker / inversion FVG / BPR / unicorn / OTE zones. 1-minute simulation, honest checks.

Items (all levels and zones are built from information available at the time):
  ipda    (a) sweep-and-reverse of the previous 20/40/60 trading days' high/low (trading day =
          17:00-17:00 NY): the first cross of the level, 1m MSS entry (market / FVG / CE / OTE
          as in `smc_models`), target the opposite level, the range midpoint or `rr` R.
          Control: the same levels moved to random distances from the day's open.
          (b) the Power of 3 08:30 rule with IPDA filters (level in the trade direction already
          taken / far / near; opposite level swept before 08:30) or the level as the target.
  smt     (a) gold sweeps the previous NY day's high/low or the last confirmed 1h swing; SMT =
          silver has not exceeded its own level (previous-day extreme, or its high/low in the
          swing hour) up to the sweep minute. Reversal with a 1m MSS entry; SMT vs non-SMT
          sweeps through the identical code. (b) SMT as a filter for the 08:30 rule.
          Silver: HistData XAGUSD 1m on Hugging Face (`elthariel/histdata_fx_1m`, fixed EST
          stamps re-localised to New York time) and Binance XAGUSDT 1m for 2026.
  zones   on 1h / 4h bars: breaker (an order block closed through, traded from the other side),
          ifvg (an FVG closed through, flipped), bpr (overlap of a new FVG with an opposite FVG
          of the last 10 bars), unicorn (breaker overlapping a same-side FVG within 3 bars), ote
          (after a BOS: zone from the 62% retracement of the leg to the leg extreme). A zone is
          valid after the close of the bar that proves it. Entries (`smc_search._sim`): limit at
          the edge, 25% or 50% of the zone, or a 1m MSS inside it. Control: zones moved to random
          times 5-30 days away, shifted to the price there.
Every grid: in sample 2019-2022 / out of sample 2023-01 .. 2025-02 (XAUUSD CFD, $0.35/oz per
trade), the identical code on random walks (`--seeds`, silver = correlated walk), walk-forward
"Auto" (best setting of the previous 6 months traded the next month). Binance 2026 on the real
data only (fees 0.02% maker / 0.05% taker).

    python -m aegis_lab.research.smc_more [--item ipda|smt|zones|all] [--seeds 3]
"""

from __future__ import annotations

import argparse
import io
import itertools
import pathlib
import urllib.request
import zipfile

import numpy as np
import pandas as pd

from aegis_lab.research import smc, smc_models, smc_search
from aegis_lab.research.smc_models import BASE, NAN, OUT, Market, random_walk, walk_forward

HIST_SPLIT = int(pd.Timestamp("2023-01-01", tz="UTC").timestamp())
BIN_SPLIT = int(pd.Timestamp("2026-06-01", tz="UTC").timestamp())
XAG_URL = "https://huggingface.co/datasets/elthariel/histdata_fx_1m/resolve/main/xagusd/ticks.parquet"
XAG_BIN = OUT / "binance" / "XAGUSDT" / "klines-1m"
IPDA_N = (20, 40, 60)


# ---------------------------------------------------------------- data


def silver_hist() -> pd.DataFrame:
    """HistData XAGUSD 1m. The export converted fixed EST (UTC-5, no DST) to UTC, so in summer the
    stamps are an hour late (checked: 1m return correlation with gold peaks at -60 min in July,
    0 in January). Re-localise EST wall time to New York time."""
    raw = OUT / "xagusd_histdata.parquet"
    if not raw.exists():
        urllib.request.urlretrieve(XAG_URL, raw)
    s = pd.read_parquet(raw)
    est = s.ts.dt.tz_localize(None) - pd.Timedelta(hours=5)
    utc = est.dt.tz_localize("America/New_York", ambiguous="NaT", nonexistent="NaT").dt.tz_convert("UTC")
    ok = utc.notna().to_numpy()
    t = (utc[ok] - pd.Timestamp("1970-01-01", tz="UTC")).dt.total_seconds().astype(np.int64).to_numpy()
    out = pd.DataFrame({"t": t, "o": s.open.to_numpy()[ok], "h": s.high.to_numpy()[ok], "l": s.low.to_numpy()[ok],
                        "c": s.close.to_numpy()[ok]})
    return out.drop_duplicates("t").sort_values("t").reset_index(drop=True)


def silver_binance() -> pd.DataFrame:
    frames = []
    for f in sorted(XAG_BIN.glob("*.zip")):
        with zipfile.ZipFile(f) as z:
            raw = z.read(z.namelist()[0])
        head = raw[:9] == b"open_time"
        d = pd.read_csv(io.BytesIO(raw), header=0 if head else None, usecols=range(5))
        d.columns = ["t", "o", "h", "l", "c"]
        frames.append(d)
    m = pd.concat(frames).drop_duplicates("t").sort_values("t")
    m["t"] = m.t // 1000
    return m.reset_index(drop=True)


def align(gold: pd.DataFrame, silver: pd.DataFrame) -> pd.DataFrame:
    """Silver on gold's clock; missing minutes stay NaN (nan-aware maxima below)."""
    return silver.set_index("t").reindex(gold.t.to_numpy())[["o", "h", "l", "c"]].reset_index(drop=True)


def silver_walk(gold_rw: pd.DataFrame, silver: pd.DataFrame, seed: int, rho: float = 0.7) -> pd.DataFrame:
    """Random-walk silver correlated with the random-walk gold (1m return correlation ~rho, the real
    value is 0.64-0.73), so that SMT events occur at a realistic rate but carry no information."""
    other = random_walk(gold_rw, seed + 1000)
    g0, o0 = float(gold_rw.c.iloc[0]), float(other.c.iloc[0])
    s0 = float(np.nanmedian(silver.c))
    lg = {k: np.log(gold_rw[k].to_numpy() / g0) for k in "ohlc"}
    lo_ = {k: np.log(other[k].to_numpy() / o0) for k in "ohlc"}
    a, b = rho, np.sqrt(1 - rho**2)
    o = a * lg["o"] + b * lo_["o"]
    c = a * lg["c"] + b * lo_["c"]
    up = a * (lg["h"] - np.maximum(lg["o"], lg["c"])) + b * (lo_["h"] - np.maximum(lo_["o"], lo_["c"]))
    dn = a * (np.minimum(lg["o"], lg["c"]) - lg["l"]) + b * (np.minimum(lo_["o"], lo_["c"]) - lo_["l"])
    out = pd.DataFrame({"o": o, "c": c, "h": np.maximum(o, c) + up, "l": np.minimum(o, c) - dn})
    out = s0 * np.exp(out)
    mask = np.isnan(silver.c.to_numpy())  # keep the real data's missing minutes
    out.loc[mask] = NAN
    return out[["o", "h", "l", "c"]]


# ---------------------------------------------------------------- common


def trading_days(mk: Market) -> tuple[np.ndarray, pd.DataFrame]:
    """Trading day = 17:00 NY to 17:00 NY, labelled by the NY date it ends on."""
    td = mk.ny_day + (mk.ny_min >= 17 * 60)
    df = pd.DataFrame({"td": td, "h": mk.h, "l": mk.l, "i": np.arange(len(td))})
    g = df.groupby("td")
    d = pd.DataFrame({"h": g.h.max(), "l": g.l.min(), "i0": g.i.first(), "i1": g.i.last() + 1, "n": g.i.size()})
    d = d[d.n >= 300]
    for n in IPDA_N:
        d[f"h{n}"] = d.h.shift().rolling(n).max()
        d[f"l{n}"] = d.l.shift().rolling(n).min()
    return td, d


def evaluate(runs: dict, split: int, min_is: int = 30, min_oos: int = 15, back: int = 6) -> dict:
    rows = []
    for k, r in runs.items():
        a, b = r[r.t < split], r[r.t >= split]
        rows.append({"k": k, "is_n": len(a), "is_g": a.r_gross.mean(), "is_r": a.r_net.mean(), "oos_n": len(b),
                     "oos_g": b.r_gross.mean(), "oos_r": b.r_net.mean()})
    df = pd.DataFrame(rows).set_index("k")
    ok = df[(df.is_n >= min_is) & (df.oos_n >= min_oos)]
    if not len(ok):
        return {"settings": len(df), "ok": 0}
    top = ok.sort_values("is_r", ascending=False).head(15)
    wf = walk_forward({k: runs[k] for k in ok.index}, back)
    return {"settings": len(df), "ok": len(ok), "is_n": ok.is_n.median(), "oos_n": ok.oos_n.median(),
            "is_g": ok.is_g.median(), "is_r": ok.is_r.median(), "oos_g": ok.oos_g.median(),
            "oos_r": ok.oos_r.median(), "both_pos": int(((ok.is_r > 0) & (ok.oos_r > 0)).sum()),
            "best_is": ok.is_r.max(), "top15_oos": top.oos_r.mean(), "wf_n": int(wf.n.sum()) if len(wf) else 0,
            "wf_r": wf.sum_r.sum() / max(wf.n.sum(), 1) if len(wf) else NAN,
            "wf_pos": f"{int((wf.sum_r > 0).sum())}/{len(wf)}" if len(wf) else "-", "_df": df, "_top": top}


def show(label: str, s: dict) -> None:
    keys = ["settings", "ok", "is_n", "oos_n", "is_g", "is_r", "oos_g", "oos_r", "both_pos", "best_is", "top15_oos",
            "wf_n", "wf_r", "wf_pos"]
    txt = ", ".join(f"{k} {s[k]:+.3f}" if isinstance(s.get(k), float) else f"{k} {s.get(k)}" for k in keys if k in s)
    print(f"  {label}: {txt}", flush=True)


def group_medians(s: dict, names: list[str]) -> None:
    df = s.get("_df")
    if df is None:
        return
    ok = df[(df.is_n >= 30) & (df.oos_n >= 15)].copy()
    for j, nm in enumerate(names):
        ok[nm] = [k[j] for k in ok.index]
        print(ok.groupby(nm)[["is_n", "oos_n", "is_g", "is_r", "oos_g", "oos_r"]].median().round(3).to_string())


def stat(r: pd.Series) -> str:
    if len(r) < 2:
        return f"n {len(r)}"
    return f"n {len(r)}, {r.mean():+.3f} R (t {r.mean() / r.std() * np.sqrt(len(r)):.2f})"


# ---------------------------------------------------------------- 1. IPDA


def ipda_setups(mk: Market, n: int, target: str, rand: np.random.Generator | None = None,
                window: int = 120) -> list:
    """First cross of the previous-n-day high (short) / low (long) during the trading day."""
    _, d = trading_days(mk)
    d = d.dropna(subset=[f"h{n}"])
    hs, ls = d[f"h{n}"].to_numpy(), d[f"l{n}"].to_numpy()
    if rand is not None:
        # control: same distances from the day's open, shuffled across days
        op = mk.o[d.i0.to_numpy()]
        up, dn = hs - op, op - ls
        pu, pd_ = rand.permutation(len(d)), rand.permutation(len(d))
        hs, ls = op + up[pu], op - dn[pd_]
    out = []
    for (i0, i1), H, L in zip(d[["i0", "i1"]].to_numpy(), hs, ls):
        mid = 0.5 * (H + L)
        up = np.flatnonzero(mk.h[i0:i1] > H)
        dn = np.flatnonzero(mk.l[i0:i1] < L)
        if len(up):
            i = i0 + up[0]
            out.append((i, i + window, -1, L if target == "opp" else mid, NAN, int(mk.ny_day[i])))
        if len(dn):
            i = i0 + dn[0]
            out.append((i, i + window, 1, H if target == "opp" else mid, NAN, int(mk.ny_day[i])))
    return sorted(out)


IPDA_GRID = dict(n=list(IPDA_N), target=["opp", "mid", "rr"], entry=["market", "fvg", "ce", "ote"], rr=[2.0, 3.0],
                 buf=[0.05, 0.2], min_risk=[1.0, 3.0])


def ipda_grid(mk: Market, rand_seed: int | None = None) -> dict:
    runs, cache = {}, {}
    for vals in itertools.product(*IPDA_GRID.values()):
        g = dict(zip(IPDA_GRID, vals))
        key = (g["n"], g["target"])
        if key not in cache:
            rng = np.random.default_rng(rand_seed) if rand_seed is not None else None
            cache[key] = ipda_setups(mk, g["n"], "mid" if g["target"] == "rr" else g["target"], rng)
        cfg = {**BASE, "entry": g["entry"], "rr": g["rr"], "buf": g["buf"], "min_risk": g["min_risk"],
               "use_tgt": g["target"] != "rr", "extra": 0}
        runs[vals] = mk.run(cache[key], cfg)
    return runs


# ---------------------------------------------------------------- Power of 3 with extras


def po3_trades(mk: Market, silver: pd.DataFrame | None = None, entry_min: int = 510, exit_min: int = 960,
               gap_fill: bool = True) -> pd.DataFrame:
    """The 08:30 midnight-open rule (as `smc_models.po3_rule`) plus IPDA / SMT features and exits at the
    IPDA levels. `gap_fill`: a minute opening beyond the stop fills at its open (the original fills at
    the stop)."""
    td, d = trading_days(mk)
    lv = d[[f"{s}{n}" for n in IPDA_N for s in "hl"]]
    so = sh = sl = None
    if silver is not None:
        so, sh, sl = silver.o.to_numpy(), silver.h.to_numpy(), silver.l.to_numpy()
    days = mk.days
    rows = []
    for day in days.index:
        a, b = np.searchsorted(mk.ny_day, day), np.searchsorted(mk.ny_day, day, "right")
        if b - a < 600 or mk.dow[a] >= 5:
            continue
        mn = mk.ny_min[a:b]
        end = a + int(np.searchsorted(mn, 17 * 60))
        if end - a < 600:
            continue
        k, k2 = a + int(np.searchsorted(mn, entry_min)), a + int(np.searchsorted(mn, exit_min))
        if k - a < 30 or k2 >= end:
            continue
        px = mk.o[k]
        d_ = 1 if px > mk.o[a] else -1
        hi, lo = mk.h[a:k].max(), mk.l[a:k].min()
        risk = px - lo if d_ > 0 else hi - px
        if risk <= 0:
            continue
        stop = lo if d_ > 0 else hi
        tday = td[k]
        row = {"day": day, "t": int(mk.t[k]), "dir": d_, "risk": risk}
        tgts = {}
        if tday in lv.index:
            L = lv.loc[tday]
            # today's part of the trading day before the entry (from 18:00 the evening before)
            j0 = int(d.i0.get(tday, a))
            thi, tlo = mk.h[j0:k].max(), mk.l[j0:k].min()
            for n in IPDA_N:
                H, Lo = L[f"h{n}"], L[f"l{n}"]
                if H != H:
                    continue
                lvl_dir, lvl_opp = (H, Lo) if d_ > 0 else (Lo, H)
                taken = (thi > H) if d_ > 0 else (tlo < Lo)
                row[f"taken{n}"] = taken
                row[f"opp_swept{n}"] = (tlo < Lo) if d_ > 0 else (thi > H)
                row[f"dist{n}"] = 0.0 if taken else (lvl_dir - px) * d_ / risk
                if not taken:
                    tgts[n] = lvl_dir
        if silver is not None:
            s_mid, s_px = so[a], so[k]
            row["s_agree"] = (s_px > s_mid) == (d_ > 0) if s_mid == s_mid and s_px == s_px else NAN
            ai = np.searchsorted(mk.ny_day, day - 1)
            asia = slice(ai + int(np.searchsorted(mk.ny_min[ai:a], 20 * 60)), a)
            if asia.stop - asia.start > 60:
                if d_ > 0:
                    g_sw = lo < mk.l[asia].min()
                    s_sw = np.nanmin(sl[a:k]) < np.nanmin(sl[asia])
                else:
                    g_sw = hi > mk.h[asia].max()
                    s_sw = np.nanmax(sh[a:k]) > np.nanmax(sh[asia])
                row["smt"] = "smt" if g_sw and not s_sw else ("both" if g_sw else "none")
        # exits: 16:00, or a limit at the IPDA level in the trade direction
        for name, tg in [("pnl", NAN)] + [(f"pnl_t{n}", v) for n, v in tgts.items()]:
            pnl = NAN
            for i in range(k, k2):
                op = mk.o[i] * d_
                lo_i = (mk.l[i] if d_ > 0 else -mk.h[i])
                hi_i = (mk.h[i] if d_ > 0 else -mk.l[i])
                if lo_i <= stop * d_:
                    fill = min(op, stop * d_) if (gap_fill and i > k) else stop * d_
                    pnl = fill - px * d_
                    break
                if tg == tg and hi_i >= tg * d_ + 0.01:
                    pnl = (max(op, tg * d_) if i > k else tg * d_) - px * d_
                    break
            if pnl != pnl:
                pnl = (mk.c[k2 - 1] - px) * d_
            row[name] = pnl
        rows.append(row)
    r = pd.DataFrame(rows)
    r["R"] = r.pnl / r.risk
    return r


def po3_filters(r: pd.DataFrame, cost: float, silver: bool) -> dict:
    """Fixed, a-priori filter rules -> trades frames (t, r_gross, r_net) for walk-forward."""
    def fr(mask, col="pnl"):
        x = r[mask & r[col].notna()]
        return pd.DataFrame({"t": x.t, "r_gross": x[col] / x.risk, "r_net": (x[col] - cost) / x.risk})
    all_ = pd.Series(True, index=r.index)
    out = {"none": fr(all_)}
    for n in IPDA_N:
        if f"taken{n}" not in r:
            continue
        tk = r[f"taken{n}"].fillna(False).astype(bool)
        ds = r[f"dist{n}"]
        osw = r[f"opp_swept{n}"].fillna(False).astype(bool)
        out[f"ipda{n}_taken"] = fr(tk)
        out[f"ipda{n}_far3R"] = fr(~tk & (ds >= 3))
        out[f"ipda{n}_near3R"] = fr(~tk & (ds < 3))
        out[f"ipda{n}_not_taken"] = fr(~tk & ds.notna())
        out[f"ipda{n}_opp_swept"] = fr(osw)
        out[f"ipda{n}_target"] = fr(all_, f"pnl_t{n}").pipe(
            lambda x: pd.concat([x, fr(r[f"pnl_t{n}"].isna())]).sort_values("t"))
    if silver:
        ag = r.s_agree
        out["smt_agree"] = fr(ag == True)  # noqa: E712
        out["smt_disagree"] = fr(ag == False)  # noqa: E712
        for v in ("smt", "both", "none"):
            out[f"smt_asia_{v}"] = fr(r.smt == v)
    return out


def po3_table(r: pd.DataFrame, cost: float, silver: bool, split: int) -> pd.DataFrame:
    rows = []
    base = (r.pnl / r.risk).to_numpy()
    rng = np.random.default_rng(0)
    for k, x in po3_filters(r, cost, silver).items():
        a, b = x[x.t < split], x[x.t >= split]
        t = x.r_gross.mean() / x.r_gross.std() * np.sqrt(len(x)) if len(x) > 1 else NAN
        # random-subset control: share of same-size random subsets of all trades with a higher mean
        p = NAN
        if 0 < len(x) < len(base) and not k.endswith("_target"):
            sims = np.array([base[rng.choice(len(base), len(x), replace=False)].mean() for _ in range(2000)])
            p = (sims >= x.r_gross.mean()).mean()
        rows.append({"filter": k, "n": len(x), "is_n": len(a), "is_g": a.r_gross.mean(), "is_r": a.r_net.mean(),
                     "oos_n": len(b), "oos_g": b.r_gross.mean(), "oos_r": b.r_net.mean(), "t": t, "p_subset": p})
    return pd.DataFrame(rows).set_index("filter")


# ---------------------------------------------------------------- 2. SMT


def smt_setups(mk: Market, silver: pd.DataFrame, source: str, window: int = 120, k: int = 3) -> list:
    """-> (i_s, i_dead, dir, target, ext, day, smt flag)."""
    sh, sl = silver.h.to_numpy(), silver.l.to_numpy()
    out = []
    if source == "pd":
        dd = mk.days
        idx = {day: i for i, day in enumerate(dd.index)}
        starts = dd.i0.to_numpy().astype(int)
        ends = np.r_[starts[1:], len(mk.t)]
        s_dh = np.array([np.nanmax(sh[a:b]) if b > a and np.isfinite(sh[a:b]).any() else NAN
                         for a, b in zip(starts, ends)])
        s_dl = np.array([np.nanmin(sl[a:b]) if b > a and np.isfinite(sl[a:b]).any() else NAN
                         for a, b in zip(starts, ends)])
        for day, r in dd.iterrows():
            j = idx[day]
            if j == 0 or r.prev_h != r.prev_h:
                continue
            a, b = starts[j], ends[j]
            up = np.flatnonzero(mk.h[a:b] > r.prev_h)
            dn = np.flatnonzero(mk.l[a:b] < r.prev_l)
            if len(up):
                i = a + up[0]
                s_now = np.nanmax(sh[a:i + 1]) if np.isfinite(sh[a:i + 1]).any() else NAN
                flag = bool(s_now <= s_dh[j - 1]) if s_now == s_now and s_dh[j - 1] == s_dh[j - 1] else None
                out.append((i, i + window, -1, r.prev_l, NAN, int(day), flag))
            if len(dn):
                i = a + dn[0]
                s_now = np.nanmin(sl[a:i + 1]) if np.isfinite(sl[a:i + 1]).any() else NAN
                flag = bool(s_now >= s_dl[j - 1]) if s_now == s_now and s_dl[j - 1] == s_dl[j - 1] else None
                out.append((i, i + window, 1, r.prev_h, NAN, int(day), flag))
        return sorted(out, key=lambda x: x[0])
    # 1h swings: confirmed k hours later; the first cross after confirmation is the sweep
    key = mk.t // 3600
    starts = np.flatnonzero(np.r_[True, key[1:] != key[:-1]])
    ends = np.r_[starts[1:], len(mk.t)]
    H = np.maximum.reduceat(mk.h, starts)
    L = np.minimum.reduceat(mk.l, starts)
    with np.errstate(all="ignore"):
        SH = np.array([np.nanmax(sh[a:b]) if np.isfinite(sh[a:b]).any() else NAN for a, b in zip(starts, ends)])
        SL = np.array([np.nanmin(sl[a:b]) if np.isfinite(sl[a:b]).any() else NAN for a, b in zip(starts, ends)])
    last_hi = last_lo = None  # (bar, price) of the latest confirmed swing
    for i in range(2 * k, len(starts) - 1):
        j = i - k
        if H[j] > H[j - k:j].max() and H[j] >= H[j + 1:i + 1].max():
            last_hi = (j, H[j])
        if L[j] < L[j - k:j].min() and L[j] <= L[j + 1:i + 1].min():
            last_lo = (j, L[j])
        a, b = starts[i + 1], ends[i + 1]  # next hour: the swing is known from its first minute
        for side, sw in ((-1, last_hi), (1, last_lo)):
            if sw is None:
                continue
            j, lvl = sw
            x = np.flatnonzero(mk.h[a:b] > lvl) if side < 0 else np.flatnonzero(mk.l[a:b] < lvl)
            if not len(x):
                continue
            s = a + x[0]
            win = slice(ends[j], s + 1)
            if side < 0:
                s_now = np.nanmax(sh[win]) if np.isfinite(sh[win]).any() else NAN
                flag = bool(s_now <= SH[j]) if s_now == s_now and SH[j] == SH[j] else None
                tgt = last_lo[1] if last_lo is not None else NAN
                last_hi = None
            else:
                s_now = np.nanmin(sl[win]) if np.isfinite(sl[win]).any() else NAN
                flag = bool(s_now >= SL[j]) if s_now == s_now and SL[j] == SL[j] else None
                tgt = last_hi[1] if last_hi is not None else NAN
                last_lo = None
            out.append((s, s + window, side, tgt, NAN, int(mk.ny_day[s]), flag))
    return sorted(out, key=lambda x: x[0])


SMT_GRID = dict(source=["pd", "swing1h"], smt=["smt", "no_smt", "all"], entry=["market", "fvg", "ce", "ote"],
                rr=[2.0, 3.0], buf=[0.05, 0.2], use_tgt=[True, False])


def smt_grid(mk: Market, silver: pd.DataFrame) -> dict:
    runs, cache = {}, {}
    for vals in itertools.product(*SMT_GRID.values()):
        g = dict(zip(SMT_GRID, vals))
        if g["source"] not in cache:
            cache[g["source"]] = smt_setups(mk, silver, g["source"])
        st = cache[g["source"]]
        if g["smt"] == "smt":
            st = [x for x in st if x[6] is True]
        elif g["smt"] == "no_smt":
            st = [x for x in st if x[6] is False]
        cfg = {**BASE, "entry": g["entry"], "rr": g["rr"], "buf": g["buf"], "use_tgt": g["use_tgt"], "extra": 0}
        runs[vals] = mk.run([x[:6] for x in st], cfg)
    return runs


# ---------------------------------------------------------------- 3. zones


def zones_more(m: pd.DataFrame, sec: int, k: int = 3, max_bars: int = 120) -> pd.DataFrame:
    """Breaker, IFVG, BPR, unicorn and OTE zones on `sec` bars. `valid` = first second the zone is
    known (after the close of the bar that proves it); `bar` = that bar."""
    b = smc.resample(m, sec)
    base = smc.zones(b, sec, k)
    t, o, h, l, c, atr = (b[x].to_numpy() for x in ("t", "o", "h", "l", "c", "atr"))
    n = len(b)
    # structure trend per bar and OTE zones, same swing rule as smc.zones
    trend = np.zeros(n)
    tr = 0
    sh = sl = None
    ote = []
    for i in range(n):
        j = i - k
        if j >= k:
            if h[j] > h[j - k:j].max() and h[j] >= h[j + 1:i + 1].max():
                sh = (j, h[j])
            if l[j] < l[j - k:j].min() and l[j] <= l[j + 1:i + 1].min():
                sl = (j, l[j])
        if sh is not None and c[i] > sh[1]:
            lo_i = sh[0] + int(np.argmin(l[sh[0]:i + 1]))
            lo, hi = l[lo_i], h[lo_i:i + 1].max()
            if atr[i] == atr[i]:
                ote.append(("ote", 1, t[i] + sec, hi - 0.62 * (hi - lo), lo, atr[i], 1, i))
            tr, sh = 1, None
        if sl is not None and c[i] < sl[1]:
            hi_i = sl[0] + int(np.argmax(h[sl[0]:i + 1]))
            hi, lo = h[hi_i], l[hi_i:i + 1].min()
            if atr[i] == atr[i]:
                ote.append(("ote", -1, t[i] + sec, hi, lo + 0.62 * (hi - lo), atr[i], -1, i))
            tr, sl = -1, None
        trend[i] = tr
    rows = list(ote)

    def flipped(z):
        """First bar after the zone's bar that closes through its far side (strictly after it is known)."""
        res = []
        for r in z.itertuples():
            e = min(r.bar + 1 + max_bars, n)
            cc = c[r.bar + 1:e]
            x = np.flatnonzero(cc < r.bot) if r.dir > 0 else np.flatnonzero(cc > r.top)
            if len(x):
                i = r.bar + 1 + x[0]
                res.append((-r.dir, t[i] + sec, r.top, r.bot, atr[i], trend[i], i))
        return res

    obs, fvgs = base[base.kind == "ob"], base[base.kind == "fvg"]
    brk = flipped(obs)
    rows += [("breaker", d, v, tp, bt, a, trd, i) for d, v, tp, bt, a, trd, i in brk]
    rows += [("ifvg", d, v, tp, bt, a, trd, i) for d, v, tp, bt, a, trd, i in flipped(fvgs)]
    fb = fvgs.bar.to_numpy()
    fd, ft, fbo = fvgs.dir.to_numpy(), fvgs.top.to_numpy(), fvgs.bot.to_numpy()
    for q in range(len(fvgs)):
        sel = np.flatnonzero((fb >= fb[q] - 10) & (fb < fb[q]) & (fd == -fd[q]))
        best = None
        for s in sel[::-1]:
            top, bot = min(ft[q], ft[s]), max(fbo[q], fbo[s])
            if top > bot:
                best = (top, bot)
                break
        if best:
            i = fb[q]
            rows.append(("bpr", fd[q], t[i] + sec, best[0], best[1], atr[i], trend[i], i))
    for d, v, tp, bt, a, trd, i in brk:
        sel = np.flatnonzero((fd == d) & (fb >= i - 3) & (fb <= i + 3))
        for s in sel:
            top, bot = min(tp, ft[s]), max(bt, fbo[s])
            if top > bot:
                j = max(i, fb[s])
                rows.append(("unicorn", d, t[j] + sec, top, bot, atr[j], trend[j], j))
                break
    z = pd.DataFrame(rows, columns=["kind", "dir", "valid", "top", "bot", "atr", "trend", "bar"])
    z = z[(z.top > z.bot) & z.atr.notna()].copy()
    z["size_atr"] = (z.top - z.bot) / z.atr
    hh = pd.Series(h).rolling(smc_search.LIQ_BARS, min_periods=1).max().to_numpy()
    ll = pd.Series(l).rolling(smc_search.LIQ_BARS, min_periods=1).min().to_numpy()
    bb = z.bar.to_numpy().astype(int)
    z["liq"] = np.where(z.dir > 0, hh[bb], ll[bb])
    return z.sort_values("valid").reset_index(drop=True)


def random_zones(z: pd.DataFrame, mt: np.ndarray, mc: np.ndarray, seed: int) -> pd.DataFrame:
    """Control: every zone moved 5-30 days earlier or later and shifted to the price there."""
    rng = np.random.default_rng(seed)
    z = z.copy()
    off = rng.uniform(5, 30, len(z)) * 86400 * rng.choice([-1, 1], len(z))
    nv = (z.valid.to_numpy() + off).astype(np.int64)
    i_old = np.clip(np.searchsorted(mt, z.valid.to_numpy()) - 1, 0, len(mt) - 1)
    i_new = np.clip(np.searchsorted(mt, nv) - 1, 0, len(mt) - 1)
    sh = mc[i_new] - mc[i_old]
    z["valid"], z["top"], z["bot"], z["liq"] = nv, z.top + sh, z.bot + sh, z.liq + sh
    return z[(nv > mt[0]) & (nv < mt[-1])].sort_values("valid").reset_index(drop=True)


ZMODE = {"edge": (0, 0.0), "d25": (0, 0.25), "mid": (0, 0.5), "mss": (2, 0.0)}
ZONE_GRID = dict(htf=[3600, 14400], kind=["breaker", "ifvg", "bpr", "unicorn", "ote"], entry=list(ZMODE),
                 rr=[2.0, 3.0], tgt_liq=[False, True], buf=[0.1, 0.3], trend=[False, True])
ZBASE = dict(min_size=0.1, max_size=5.0, max_age=72 * 3600, max_hold=86400, min_risk=1.0, min_rr=1.0, fk=3)


class ZoneLab:
    def __init__(self, m: pd.DataFrame, venue: str):
        self.venue = venue
        self.mt = m.t.to_numpy()
        self.mo, self.mh, self.ml, self.mc = (m[x].to_numpy() for x in "ohlc")
        self.all = np.ones(len(m), bool)
        self.hour_end = {s: ((self.mt + 60) % s) == 0 for s in (3600, 14400)}
        self.z = {s: zones_more(m, s) for s in (3600, 14400)}

    def cost(self, px: float, maker: bool, mkt: bool) -> float:
        if self.venue == "binance":
            return ((smc.MAKER if maker else smc.TAKER) + (smc.TAKER if mkt else smc.MAKER)) * px
        return smc_models.HIST_COST

    def run(self, g: dict, z: pd.DataFrame | None = None) -> pd.DataFrame:
        z = self.z[g["htf"]] if z is None else z
        z = z[(z.kind == g["kind"]) & (z.size_atr >= ZBASE["min_size"]) & (z.size_atr <= ZBASE["max_size"])]
        if g["trend"]:
            z = z[z.trend == z.dir]
        mode, depth = ZMODE[g["entry"]]
        he = self.hour_end[g["htf"]]
        i0s = np.searchsorted(self.mt, z.valid.to_numpy())
        i1s = np.searchsorted(self.mt, z.valid.to_numpy() + ZBASE["max_age"])
        rows, taken = [], {1: -1, -1: -1}
        for r, i0, i1 in zip(z.itertuples(), i0s, i1s):
            if i0 >= len(self.mt):
                continue
            buf = g["buf"] * r.atr
            qty, avg, stop0, pnl, maker, mkt, ex, ti = smc_search._sim(
                self.mt, self.mo, self.mh, self.ml, self.mc, he, self.all, i0, i1, r.dir, r.top, r.bot, r.liq,
                mode, 1, depth, 1.0, buf, g["rr"], g["tgt_liq"], ZBASE["min_rr"], 0.0, 1.0, 0.0,
                ZBASE["max_hold"], ZBASE["fk"], 60, 0.01)
            if qty <= 0 or ti <= taken[r.dir]:
                continue
            risk = abs(avg - stop0)  # planned risk of one unit, fixed before the fill
            if risk < ZBASE["min_risk"]:
                continue
            taken[r.dir] = ti + 1
            fee = self.cost(abs(avg), maker, mkt)
            rows.append((self.mt[ti], r.dir, pnl, fee, risk, ex))
        df = pd.DataFrame(rows, columns=["t", "dir", "pnl", "fee", "risk", "exit"])
        df["r_gross"] = df.pnl / df.risk
        df["r_net"] = (df.pnl - df.fee) / df.risk
        return df.sort_values("t").reset_index(drop=True)

    def grid(self, zs: dict | None = None) -> dict:
        runs = {}
        for vals in itertools.product(*ZONE_GRID.values()):
            g = dict(zip(ZONE_GRID, vals))
            if g["kind"] == "ote" and g["trend"]:
                continue  # OTE zones are with the BOS by construction
            runs[vals] = self.run(g, None if zs is None else zs[g["htf"]])
        return runs


# ---------------------------------------------------------------- driver


def load(data: str):
    if data == "hist":
        g = smc_models.hist_minutes()
        return g, align(g, silver_hist()), HIST_SPLIT, "hist", smc_models.HIST_COST
    g = smc.minutes()
    return g, align(g, silver_binance()), BIN_SPLIT, "binance", 0.0007 * float(g.c.median())


def item_ipda(data: str, seeds: int) -> None:
    g, _, split, venue, cost = load(data)
    back = 6 if data == "hist" else 3
    print(f"\n######## IPDA sweeps ({data})", flush=True)
    mk = Market(g, venue)
    s = evaluate(ipda_grid(mk), split, back=back)
    show("real", s)
    group_medians(s, list(IPDA_GRID))
    if "_top" in s:
        print(s["_top"].head(8).round(3).to_string())
    for sd in range(seeds):
        show(f"random levels {sd}", evaluate(ipda_grid(mk, rand_seed=sd), split, back=back))
    if data == "hist":
        for sd in range(seeds):
            show(f"random walk {sd}", evaluate(ipda_grid(Market(random_walk(g, sd), venue)), split, back=back))


def item_po3(seeds: int) -> None:
    g, sv, split, venue, cost = load("hist")
    print("\n######## Power of 3 08:30 rule + IPDA / SMT filters (hist)", flush=True)
    orig = smc_models.po3_rule(g)
    mk = Market(g, venue)
    r0 = po3_trades(mk, sv, gap_fill=False)
    r = po3_trades(mk, sv)
    print(f"  original po3_rule: {stat(orig.R)}; re-implementation stop at stop: {stat(r0.R)}; "
          f"stop gap-filled at the open: {stat(r.R)}")
    tab = po3_table(r, cost, True, split)
    print(tab.round(3).to_string())
    wf = walk_forward(po3_filters(r, cost, True), 6)
    print(f"  walk-forward over filters: {wf.n.sum()} trades, {wf.sum_r.sum() / max(wf.n.sum(), 1):+.3f} R/trade, "
          f"months positive {(wf.sum_r > 0).sum()}/{len(wf)}; chosen: {wf.cfg.value_counts().head(5).to_dict()}")
    rws, wfs = [], []
    for sd in range(seeds):
        grw = random_walk(g, sd)
        rr_ = po3_trades(Market(grw, venue), silver_walk(grw, sv, sd))
        t = po3_table(rr_, cost, True, split)
        rws.append(t.is_g.rename(f"is{sd}"))
        rws.append(t.oos_g.rename(f"oos{sd}"))
        w = walk_forward(po3_filters(rr_, cost, True), 6)
        wfs.append(f"{w.sum_r.sum() / max(w.n.sum(), 1):+.3f} ({(w.sum_r > 0).sum()}/{len(w)})")
    rw = pd.concat(rws, axis=1)
    print("  random walks, gross R per filter (in / out of sample per seed):")
    print(rw.round(3).to_string())
    print("  random walks, walk-forward over filters R/trade (months positive):", ", ".join(wfs))


def item_smt(data: str, seeds: int) -> None:
    g, sv, split, venue, cost = load(data)
    back = 6 if data == "hist" else 3
    print(f"\n######## SMT gold vs silver ({data}); silver minutes present {np.isfinite(sv.c).mean():.1%}", flush=True)
    mk = Market(g, venue)
    for src in ("pd", "swing1h"):
        st = smt_setups(mk, sv, src)
        fl = pd.Series([x[6] for x in st])
        print(f"  {src}: {len(st)} sweeps, SMT {int((fl == True).sum())}, no SMT {int((fl == False).sum())}")  # noqa: E712
    runs = smt_grid(mk, sv)
    s = evaluate(runs, split, back=back)
    show("real", s)
    group_medians(s, list(SMT_GRID))
    for src in ("pd", "swing1h"):
        for f in ("smt", "no_smt", "all"):
            show(f"real {src} {f}", evaluate({k: v for k, v in runs.items() if k[0] == src and k[1] == f}, split,
                                              back=back))
    if data == "hist":
        for sd in range(seeds):
            grw = random_walk(g, sd)
            rrun = smt_grid(Market(grw, venue), silver_walk(grw, sv, sd))
            show(f"random walk {sd}", evaluate(rrun, split, back=back))
            for f in ("smt", "no_smt"):
                show(f"random walk {sd} {f}", evaluate({k: v for k, v in rrun.items() if k[1] == f}, split,
                                                        back=back))


def item_zones(data: str, seeds: int) -> None:
    g, _, split, venue, cost = load(data)
    back = 6 if data == "hist" else 3
    print(f"\n######## zones ({data})", flush=True)
    lab = ZoneLab(g, venue)
    for s_ in (3600, 14400):
        print(f"  {s_ // 3600}h zones: {lab.z[s_].kind.value_counts().to_dict()}")
    runs = lab.grid()
    s = evaluate(runs, split, back=back)
    show("real", s)
    group_medians(s, list(ZONE_GRID))
    if "_top" in s:
        print(s["_top"].head(10).round(3).to_string())
    for kd in ZONE_GRID["kind"]:
        show(f"real {kd}", evaluate({k: v for k, v in runs.items() if k[1] == kd}, split, back=back))
    for sd in range(seeds):
        zs = {h: random_zones(lab.z[h], lab.mt, lab.mc, sd) for h in (3600, 14400)}
        show(f"random zones {sd}", evaluate(lab.grid(zs), split, back=back))
    if data == "hist":
        for sd in range(seeds):
            show(f"random walk {sd}", evaluate(ZoneLab(random_walk(g, sd), venue).grid(), split, back=back))


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--item", default="all", choices=["ipda", "po3", "smt", "zones", "all"])
    ap.add_argument("--data", default="both", choices=["hist", "binance", "both"])
    ap.add_argument("--seeds", type=int, default=3)
    a = ap.parse_args()
    datas = ["hist", "binance"] if a.data == "both" else [a.data]
    for item in (["ipda", "po3", "smt", "zones"] if a.item == "all" else [a.item]):
        if item == "po3":
            item_po3(a.seeds)
            continue
        for d in datas:
            {"ipda": item_ipda, "smt": item_smt, "zones": item_zones}[item](d, a.seeds)


if __name__ == "__main__":
    main()
