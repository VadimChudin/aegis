"""The full ICT 2022 model on gold, rule by rule, on XAUUSD 1m 2008 … 2025-02.

smc.md / smc_long.py tested zones one at a time. This is the whole top-down sequence as taught in
the 2022 mentorship (episodes 1-2) and in the ICT daily-bias lessons, every step explicit:

  day      trading day 18:00 → 17:00 New York; PDH/PDL/close = previous trading day
  1 bias   decided before the day: none | flow (yesterday closed in the upper / lower half of its
           range) | pdclose (yesterday closed above the high / below the low of the day before;
           inside = no trade) | draw20 (the nearer of the 20-day high / low is the draw) |
           oracle (the day's real direction: NOT tradable, only shows what a perfect bias is worth)
  2 window killzone: London 02:00-05:00 or New York 07:00-10:00 (NY AM)
  3 raid   inside the killzone price trades through liquidity not taken earlier that day, against
           the bias: longs need a sweep of the Asian low (20:00-00:00), PDL, or for New York the
           London low; shorts mirror it
  4 MSS    after the raid a 5m close beyond the last 5m swing (k = 2) formed before the raid,
           before the killzone ends
  5 displacement  the leg from the raid extreme to the MSS bar leaves a 5m fair value gap; the
           one nearest the MSS is used; no FVG = no trade
  6 entry  limit at the FVG edge or its middle (consequent encroachment), valid until 60 min after
           the killzone; cancelled if the target or the stop trades first
  7 stop   beyond the raid extreme + max($0.20, 1% of daily ATR); risk must be 5-50% of daily ATR
  8 target the nearest opposite liquidity (Asian high, PDH, London high for New York) at ≥ 2 R
           ("liq"; none = no trade), or a fixed 2 R / 3 R
  9 manage optional breakeven at +1 R; exit at 16:00 New York at the latest; one trade per
           killzone
  cost     $0.25/oz round trip, +$0.05 slippage on stop and time exits. 1m bars; a bar that
           reaches the stop and the target counts as a stop.
Selection: 2008-2014 only. Control: every trade replayed with the same side, risk and target at
the same clock minute of a random other day (5 seeds): does the setup's timing matter?

    python -m aegis_lab.research.ict_full [--data DIR]
"""

from __future__ import annotations

import argparse
import itertools
import pathlib

import numpy as np
import pandas as pd

from .gold_sessions import minutes as xau_minutes

COST, SLIP = 0.25, 0.05
H = 3600
KZ = {"london": (2 * H, 5 * H), "ny": (7 * H, 10 * H)}
ASIA = (-4 * H, 0)
EXIT_AT = 16 * H
SPLIT = pd.Timestamp("2015-01-01")
PERIODS = (("2008-11", "2008", "2011"), ("2012-14", "2012", "2014"), ("2015-18", "2015", "2018"),
           ("2019-22", "2019", "2022"), ("2023-25", "2023", "2025"))
EXITS = [(tgt, be) for tgt in ("liq", "rr2", "rr3") for be in (False, True)]


def first(mask: np.ndarray) -> int:
    i = int(np.argmax(mask)) if len(mask) else 0
    return i if len(mask) and mask[i] else -1


def exit_r(o, h, l, rel, f, entry, stop, target, be) -> tuple[float, float] | None:
    """Walk 1m bars after the fill bar `f` (long space). -> (exit price, slippage) or None."""
    risk = entry - stop
    if l[f] <= stop:
        return min(o[f], stop), SLIP
    end = first(rel[f + 1:] >= EXIT_AT)
    end = len(o) - f - 1 if end < 0 else end
    O, Hh, L = o[f + 1:f + 1 + end], h[f + 1:f + 1 + end], l[f + 1:f + 1 + end]
    it = first(Hh >= target)
    if be:
        ib = first(Hh >= entry + risk)
        cut = len(L) if ib < 0 else ib + 1
        is1 = first(L[:cut] <= stop)
        is2 = first(L[cut:] <= entry)
        istop = is1 if is1 >= 0 else (cut + is2 if is2 >= 0 else -1)
        stop_px = (lambda i: stop if i < cut else entry)
    else:
        istop = first(L <= stop)
        stop_px = (lambda i: stop)
    if istop >= 0 and (it < 0 or istop <= it):
        return min(O[istop], stop_px(istop)), SLIP
    if it >= 0:
        return target, 0.0
    k = f + 1 + end
    if k < len(o):
        return o[k], SLIP
    return None


def swings_high(h5: np.ndarray, k: int = 2) -> list[int]:
    return [j for j in range(k, len(h5) - k) if h5[j] > h5[j - k:j].max() and h5[j] >= h5[j + 1:j + k + 1].max()]


FUNNEL: dict[str, int] = {}


def _step(name: str) -> None:
    FUNNEL[name] = FUNNEL.get(name, 0) + 1


def setups_day(o, h, l, c, rel, levels, targets, atr_d, kz, bar=300):
    """Long-space setups of one day and killzone. -> list of dicts, one per entry kind."""
    _step("0 day x killzone x side")
    k0, k1 = kz
    pre = rel < k0
    inkz = (rel >= k0) & (rel < k1)
    if not inkz.any():
        return []
    # liquidity not already taken between the level's own window and the killzone
    live = []
    for x, since in levels:
        w = (rel >= since) & pre
        if np.isfinite(x) and (not w.any() or l[w].min() > x):
            live.append(x)
    if not live:
        return []
    _step("1 untaken liquidity")
    lvl = max(live)
    s = first(inkz & (l < lvl))
    if s < 0:
        return []
    _step("2 raid in killzone")
    # MSS bars (5m or 1m) from 60 min before the killzone
    win = (rel >= k0 - H) & (rel < k1 + H)
    idx = np.nonzero(win)[0]
    b = (rel[idx] - (k0 - H)) // bar
    nb = int(b.max()) + 1
    h5 = np.full(nb, -np.inf)
    l5 = np.full(nb, np.inf)
    c5 = np.full(nb, np.nan)
    last5 = np.zeros(nb, int)
    np.maximum.at(h5, b, h[idx])
    np.minimum.at(l5, b, l[idx])
    c5[b] = c[idx]  # idx ascending: the last write is the bar's close
    last5[b] = idx
    counts = np.bincount(b, minlength=nb)
    ok = np.isfinite(c5) & (counts == bar // 60)
    sb = int((rel[s] - (k0 - H)) // bar)
    sw = [j for j in swings_high(np.where(ok, h5, -np.inf)) if j + 2 < sb]
    if not sw:
        return []
    _step("3 swing before raid")
    j = sw[-1]
    kz_end_b = (k1 - (k0 - H)) // bar
    mss = next((q for q in range(max(sb, j + 2) + 1, min(nb, kz_end_b)) if ok[q] and c5[q] > h5[j]), None)
    if mss is None:
        return []
    _step("4 MSS in killzone")
    m_end = last5[mss]
    raid_lo = l[s:m_end + 1].min()
    lo_b = sb + int(np.argmin(np.where(ok[sb:mss + 1], l5[sb:mss + 1], np.inf)))
    fvg = [q for q in range(lo_b + 2, mss + 1) if ok[q] and ok[q - 2] and l5[q] > h5[q - 2]]
    if not fvg:
        return []
    _step("5 displacement FVG")
    q = fvg[-1]
    top, bot = l5[q], h5[q - 2]
    stop = raid_lo - max(0.2, 0.01 * atr_d)
    out = []
    expire = first(rel[m_end + 1:] >= k1 + H)
    expire = len(o) if expire < 0 else m_end + 1 + expire
    for kind, entry in (("edge", top), ("ce", (top + bot) / 2)):
        risk = entry - stop
        if not (0.05 * atr_d <= risk <= 0.5 * atr_d):
            continue
        _step(f"6 risk ok ({kind})")
        above = sorted(x for x in targets if np.isfinite(x) and x >= entry + 2 * risk)
        liq = above[0] if above else np.nan
        seg_l, seg_h = l[m_end + 1:expire], h[m_end + 1:expire]
        f = first(seg_l < entry)
        if f < 0:
            continue
        _step(f"7 filled ({kind})")
        # cancelled if the target (the nearest one used) was reached before the fill
        near_t = entry + 2 * risk if np.isnan(liq) else min(liq, entry + 2 * risk)
        if first(seg_h[:f] >= near_t) >= 0:
            continue
        out.append({"kind": kind, "f": m_end + 1 + f, "entry": entry, "stop": stop, "liq": liq})
    return out


def build(m: pd.DataFrame) -> tuple[pd.DataFrame, dict]:
    t = m.index.to_numpy().astype("datetime64[s]").astype(np.int64)
    tday = (t + 6 * H) // 86400
    o, h, l, c = (m[x].to_numpy() for x in ("o", "h", "l", "c"))
    days, starts = np.unique(tday, return_index=True)
    ends = np.append(starts[1:], len(t))
    dh = np.maximum.reduceat(h, starts)
    dl = np.minimum.reduceat(l, starts)
    dc = c[ends - 1]
    dopen = o[starts]
    tr = np.maximum(dh - dl, np.maximum(np.abs(dh - np.roll(dc, 1)), np.abs(dl - np.roll(dc, 1))))
    atr = pd.Series(tr).rolling(14).mean().shift(1).to_numpy()
    h20 = pd.Series(dh).rolling(20).max().shift(1).to_numpy()
    l20 = pd.Series(dl).rolling(20).min().shift(1).to_numpy()
    rows = []
    for i in range(21, len(days)):
        a, z = starts[i], ends[i]
        rel = t[a:z] - days[i] * 86400
        if np.isnan(atr[i]) or len(rel) < 600:
            continue
        pdh, pdl, pc = dh[i - 1], dl[i - 1], dc[i - 1]
        bias = {
            "flow": 1 if pc > (pdh + pdl) / 2 else -1,
            "pdclose": 1 if pc > dh[i - 2] else (-1 if pc < dl[i - 2] else 0),
            "draw20": 1 if (h20[i] - dopen[i]) < (dopen[i] - l20[i]) else -1,
        }
        asia = (rel >= ASIA[0]) & (rel < ASIA[1])
        lon = (rel >= KZ["london"][0]) & (rel < KZ["london"][1])
        seg = {k: v[a:z] for k, v in (("o", o), ("h", h), ("l", l), ("c", c))}
        for side in (1, -1):
            if side == 1:
                so, sh, sl, sc = seg["o"], seg["h"], seg["l"], seg["c"]
                lv_pd, tg_pd = pdl, pdh
            else:
                so, sh, sl, sc = -seg["o"], -seg["l"], -seg["h"], -seg["c"]
                lv_pd, tg_pd = -pdh, -pdl
            a_lo = sl[asia].min() if asia.any() else np.nan
            a_hi = sh[asia].max() if asia.any() else np.nan
            l_lo = sl[lon].min() if lon.any() else np.nan
            l_hi = sh[lon].max() if lon.any() else np.nan
            for sess, kz in KZ.items():
                levels = [(a_lo, 0), (lv_pd, ASIA[0] - 2 * H)] + ([(l_lo, KZ["london"][1])] if sess == "ny" else [])
                targets = [a_hi, tg_pd] + ([l_hi] if sess == "ny" else [])
                for tf, st in ((bar, st) for bar in (300, 60)
                               for st in setups_day(so, sh, sl, sc, rel, levels, targets, atr[i], kz, bar)):
                    f = st["f"]
                    close_16 = first(rel >= EXIT_AT)
                    real = np.sign(sc[close_16 if close_16 >= 0 else -1] - so[first(rel >= kz[0])])
                    row = {"day": pd.Timestamp(days[i] * 86400, unit="s"), "di": i, "sess": sess, "side": side,
                           "tf": "5m" if tf == 300 else "1m",
                           "kind": st["kind"], "fill_rel": int(rel[f]), "risk": st["entry"] - st["stop"],
                           "oracle": real, **{f"bias_{k}": v * side for k, v in bias.items()}}
                    for tgt, be in EXITS:
                        risk = st["entry"] - st["stop"]
                        target = st["liq"] if tgt == "liq" else st["entry"] + (2 if tgt == "rr2" else 3) * risk
                        res = None if np.isnan(target) else exit_r(so, sh, sl, rel, f, st["entry"], st["stop"],
                                                                   target, be)
                        key = f"{tgt}{'_be' if be else ''}"
                        if res is None:
                            row[f"g_{key}"] = row[f"n_{key}"] = np.nan
                            continue
                        px, slip = res
                        row[f"g_{key}"] = (px - st["entry"]) / risk
                        row[f"n_{key}"] = (px - st["entry"] - COST - slip) / risk
                        row[f"tgt_{key}"] = (target - st["entry"]) / risk
                    rows.append(row)
    arrays = {"t": t, "o": o, "h": h, "l": l, "starts": starts, "ends": ends, "days": days}
    return pd.DataFrame(rows), arrays


def choose(df: pd.DataFrame, bias: str, sess: str, kind: str, key: str, tf: str) -> pd.DataFrame:
    x = df[(df.kind == kind) & (df.tf == tf) & df[f"n_{key}"].notna()]
    if sess != "both":
        x = x[x.sess == sess]
    if bias == "oracle":
        x = x[x.oracle * x.side > 0]
    elif bias != "none":
        x = x[x[f"bias_{bias}"] > 0]
    # one trade per day and killzone: the earliest fill
    return x.sort_values("fill_rel").drop_duplicates(["di", "sess"]).sort_values(["day", "fill_rel"])


def random_control(x: pd.DataFrame, key: str, arr: dict, seed: int) -> np.ndarray:
    """Same side, risk and R target at the same clock minute of random other days."""
    rng = np.random.default_rng(seed)
    t, o, h, l, starts, ends, days = (arr[k] for k in ("t", "o", "h", "l", "starts", "ends", "days"))
    tgt_r = x[f"tgt_{key}"].to_numpy()
    be = key.endswith("_be")
    out = []
    for (_, r), tr in zip(x.iterrows(), tgt_r):
        for _ in range(5):
            i = int(rng.integers(21, len(days)))
            a, z = starts[i], ends[i]
            rel = t[a:z] - days[i] * 86400
            f = first(rel >= r.fill_rel)
            if f >= 0:
                break
        else:
            continue
        if r.side > 0:
            so, sh, sl = o[a:z], h[a:z], l[a:z]
        else:
            so, sh, sl = -o[a:z], -l[a:z], -h[a:z]
        entry = so[f]
        res = exit_r(so, sh, sl, rel, f, entry, entry - r.risk, entry + tr * r.risk, be)
        if res is not None:
            out.append((res[0] - entry - COST - res[1]) / r.risk)
    return np.array(out)


def stat(x: pd.Series) -> str:
    if len(x) < 3:
        return f"n {len(x)}"
    return f"{x.mean():+.3f} (t {x.mean() / x.std() * np.sqrt(len(x)):+.1f}, n {len(x)})"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default=str(pathlib.Path.home() / "aegis-data" / "hf-xau"))
    a = ap.parse_args()
    df, arr = build(xau_minutes(pathlib.Path(a.data)))
    df.to_parquet(pathlib.Path(a.data) / "ict_full_setups.parquet")
    print(f"setups (all sides, entry kinds, MSS timeframes): {len(df)}; days {df.day.nunique()}\n")
    print("Funnel (day x killzone x side, both MSS timeframes):\n")
    for k in sorted(FUNNEL):
        print(f"- {k}: {FUNNEL[k]}")
    print()

    rows = []
    for bias, sess, kind, tf, (tgt, be) in itertools.product(
            ("none", "flow", "pdclose", "draw20", "oracle"), ("london", "ny", "both"), ("edge", "ce"), ("5m", "1m"),
            EXITS):
        key = f"{tgt}{'_be' if be else ''}"
        x = choose(df, bias, sess, kind, key, tf)
        ins, oos = x[x.day < SPLIT], x[x.day >= SPLIT]
        rows.append({"bias": bias, "sess": sess, "entry": kind, "tf": tf, "exit": key,
                     "is_n": len(ins), "is_win": (ins[f"g_{key}"] > 0).mean(), "is_gross": ins[f"g_{key}"].mean(),
                     "is_net": ins[f"n_{key}"].mean(), "oos_n": len(oos), "oos_win": (oos[f"g_{key}"] > 0).mean(),
                     "oos_gross": oos[f"g_{key}"].mean(), "oos_net": oos[f"n_{key}"].mean()})
    g = pd.DataFrame(rows)
    g.to_csv(pathlib.Path(a.data) / "ict_full_grid.csv", index=False)
    tradable = g[(g.bias != "oracle") & (g.is_n >= 100)]
    print(f"## Tradable settings: {len(tradable)} (≥ 100 trades 2008-14), net R per trade\n")
    print(f"Positive net in both halves: {((tradable.is_net > 0) & (tradable.oos_net > 0)).sum()}; "
          f"positive gross in both: {((tradable.is_gross > 0) & (tradable.oos_gross > 0)).sum()}\n")
    print(tradable.sort_values("is_net", ascending=False).head(12).round(3).to_markdown(index=False))
    for col in ("bias", "sess", "tf", "entry", "exit"):
        print("\n" + g[g.is_n >= 100].groupby(col)[["is_win", "is_gross", "is_net", "oos_gross", "oos_net"]]
              .median().round(3).to_markdown())

    print("\n## Best tradable settings (chosen on 2008-14) by period, and random-timing control 2015-25\n")
    print("| Setting | trades/yr | " + " | ".join(p for p, _, _ in PERIODS)
          + " | 2015-25 random timing (5 seeds) | setup − random, t |")
    print("|---|---|" + "---|" * (len(PERIODS) + 2))
    best = tradable.sort_values("is_net", ascending=False).head(3)
    orc = g[(g.bias == "oracle") & (g.is_n >= 100)].sort_values("is_net", ascending=False).head(1)
    for _, r in pd.concat([best, orc]).iterrows():
        x = choose(df, r.bias, r.sess, r.entry, r.exit, r.tf)
        cells = [stat(x[(x.day >= lo) & (x.day < str(int(hi) + 1))][f"n_{r.exit}"]) for _, lo, hi in PERIODS]
        oos = x[x.day >= SPLIT]
        ctrl = [random_control(oos, r.exit, arr, s) for s in range(5)]
        means = [c.mean() for c in ctrl]
        pool = np.concatenate(ctrl)
        so = oos[f"n_{r.exit}"]
        dt = (so.mean() - pool.mean()) / np.sqrt(so.var() / len(so) + pool.var() / len(pool))
        yrs = (x.day.max() - x.day.min()).days / 365.25
        print(f"| {r.bias} · {r.sess} · MSS {r.tf} · {r.entry} · {r.exit} | {len(x) / yrs:.0f} | " + " | ".join(cells)
              + f" | {min(means):+.3f} … {max(means):+.3f} | {so.mean() - pool.mean():+.3f} (t {dt:+.1f}) |")


if __name__ == "__main__":
    main()
