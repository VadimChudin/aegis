"""Bounce off order-book densities: what separates a bounce from a break.

Input: the per-day .npz files of `aegis_lab.research.density extract`.

    python -m aegis_lab.research.density_report [--min-stand 10] [--csv touches.csv]

For every density that stood `--min-stand` seconds before price first traded at it (the touch),
a long off a bid density (short off an ask density) is followed trade by trade from the touch:
`win_<stop>_<target>` = price moved `target` ticks away before trading `stop` ticks through.
1 tick = $0.01.
"""

from __future__ import annotations

import argparse
import pathlib

import numpy as np
import pandas as pd

from aegis_lab.research.density import DATA, PULLED

STOPS = (1, 10, 30, 50)
TARGETS = (50, 100, 200, 300)
HORIZON_MS = 3600e3
POST_S = (2, 5)


def _outcomes(x: np.ndarray) -> dict:
    """x: favourable excursion in ticks of every trade after the touch."""
    out = {}
    n = len(x)
    for s in STOPS:
        hit = np.flatnonzero(x <= -s)
        fs = hit[0] if len(hit) else n
        out[f"mfe_{s}"] = float(x[:fs].max()) if fs > 0 else 0.0
        for t in TARGETS:
            ht = np.flatnonzero(x[:fs] >= t)
            out[f"win_{s}_{t}"] = 1.0 if len(ht) else (0.0 if fs < n else np.nan)
    return out


def day_frame(path: pathlib.Path, min_stand: float) -> pd.DataFrame:
    z = np.load(path)
    ep = {k[3:]: z[k] for k in z.files if k.startswith("ep_")}
    tt, tp, tq, ts = z["tr_t"], z["tr_p"].astype(np.int64), z["tr_q"], z["tr_s"].astype(np.int64)
    cum_n = np.arange(len(tt) + 1)
    cum_d = np.concatenate([[0.0], np.cumsum(tq * ts)])
    cum_v = np.concatenate([[0.0], np.cumsum(tq)])
    st, sb, sa = z["s_t"], z["s_bid"], z["s_ask"]
    ref = (z["s_ref_b"], z["s_ref_a"])
    h_id, h_t, h_size = z["h_id"], z["h_t"], z["h_size"]
    h_tr, h_cn = z["h_traded"], z["h_cancel"]
    h_lo = np.searchsorted(h_id, np.arange(len(ep["p"]) + 1))

    stand = (ep["t_touch"] - ep["t0"]) / 1000
    cand = np.flatnonzero(~np.isnan(ep["t_touch"]) & (stand >= min_stand))

    pulled = ep["reason"] == PULLED
    by_side = {}
    for s in (0, 1):
        m = np.flatnonzero(pulled & (ep["side"] == s))
        m = m[np.argsort(ep["t_end"][m])]
        by_side[s] = (ep["t_end"][m], ep["p"][m], ep["peak"][m])

    def win(t0: float, t1: float) -> tuple[int, int]:
        return np.searchsorted(tt, t0, "left"), np.searchsorted(tt, t1, "right")

    rows = []
    for i in cand:
        side, p, t = int(ep["side"][i]), int(ep["p"][i]), float(ep["t_touch"][i])
        d = 1 if side == 0 else -1
        j0, j1 = win(t, t + HORIZON_MS)
        x = (tp[j0:j1] - p) * d
        if len(x) == 0:
            continue
        k = min(max(np.searchsorted(st, t) - 1, 0), len(st) - 1)
        r = {
            "time": t / 1000, "side": side, "price": p / 100, "stand_s": stand[i],
            "size": ep["touch_size"][i], "peak_pre": ep["pre_peak"][i],
            "ratio": ep["touch_size"][i] / ref[side][k],
            "usd_k": ep["touch_size"][i] * p / 100 / 1000,
            "dist0": ep["dist0"][i] / 100,
            "pre_pulled": ep["pre_cancel"][i] / ep["pre_peak"][i],
            "pre_added": ep["pre_added"][i] / ep["pre_peak"][i],
            "touch_vs_peak": ep["touch_size"][i] / ep["pre_peak"][i],
            "spread": (sa[k] - sb[k]) / 100,
            "hour": (t / 3.6e6) % 24,
            "reason": ep["reason"][i],
        }
        # tape before the touch; approach aggression = aggressive volume towards the density
        a10, b10 = win(t - 10e3, t)
        a300, b300 = win(t - 300e3, t)
        a30, b30 = win(t - 30e3, t)
        r["tape_speed"] = (cum_n[b10] - cum_n[a10]) / 10
        r["tape_accel"] = r["tape_speed"] / max((cum_n[b300] - cum_n[a300]) / 300, 1e-9)
        r["approach_delta"] = -d * (cum_d[b30] - cum_d[a30])
        r["approach_vol_vs_size"] = (cum_v[b30] - cum_v[a30]) / max(r["size"], 1e-9)
        a60, _ = win(t - 60e3, t)
        r["approach_move"] = (tp[a60] - p) * d / 100 if a60 < j0 else 0.0
        # relocation: a similar density on the same side was pulled just before this one appeared
        te, pe, pk = by_side[side]
        lo, hi = np.searchsorted(te, ep["t0"][i] - 5e3), np.searchsorted(te, ep["t0"][i])
        near = (np.abs(pe[lo:hi] - p) <= 50) & (pe[lo:hi] != p)
        near &= (pk[lo:hi] > 0.6 * ep["peak"][i]) & (pk[lo:hi] < 1.6 * ep["peak"][i])
        r["relocated"] = float(near.any())
        # the level while price is on it
        hs = slice(h_lo[i], h_lo[i + 1])
        ht, hsz, htr, hcn = h_t[hs], h_size[hs], h_tr[hs], h_cn[hs]
        base = max(np.searchsorted(ht, t, "right") - 1, 0)
        for w in POST_S:
            e = max(np.searchsorted(ht, t + w * 1e3, "right") - 1, base)
            inc = np.diff(hsz[base : e + 1])
            r[f"eat_{w}s"] = (htr[e] - htr[base]) / r["size"]
            r[f"pull_{w}s"] = (hcn[e] - hcn[base]) / r["size"]
            r[f"refill_{w}s"] = inc[inc > 0].sum() / r["size"]
            r[f"left_{w}s"] = hsz[e] / r["size"] if ht[e] <= ep["t_end"][i] else 0.0
            a, b = win(t, t + w * 1e3)
            r[f"against_{w}s"] = -d * (cum_d[b] - cum_d[a])  # aggressive volume into the density
            r[f"resolved_{w}s"] = float(np.any(x[: b - j0] <= -10))
            # the trader's test: the level is being eaten and refilled, not pulled
            r[f"real_{w}s"] = float(r[f"eat_{w}s"] > 0.05 and r[f"refill_{w}s"] > 0.05
                                    and r[f"pull_{w}s"] < 0.2)
        r.update(_outcomes(x))
        rows.append(r)
    return pd.DataFrame(rows)


def approaches(path: pathlib.Path, min_stand: float) -> pd.DataFrame:
    """For densities that stood `min_stand` s: closest approach of price before touch or end."""
    z = np.load(path)
    ep = {k[3:]: z[k] for k in z.files if k.startswith("ep_")}
    tt, tp = z["tr_t"], z["tr_p"].astype(np.int64)
    rows = []
    t_stood = ep["t0"] + min_stand * 1e3
    t_stop = np.fmin(ep["t_touch"], ep["t_end"])
    for i in np.flatnonzero(t_stop > t_stood):
        d = 1 if ep["side"][i] == 0 else -1
        j0, j1 = np.searchsorted(tt, t_stood[i]), np.searchsorted(tt, t_stop[i], "right")
        if j1 <= j0:
            continue
        x = (tp[j0:j1] - ep["p"][i]) * d
        rows.append({"closest": x.min() / 100, "touched": float(not np.isnan(ep["t_touch"][i])),
                     "peak": ep["peak"][i], "reason": ep["reason"][i]})
    return pd.DataFrame(rows)


def bins(df: pd.DataFrame, col: str, label: str, q: int = 5) -> str:
    x = df[col]
    ok = df[label].notna() & x.notna()
    if ok.sum() < 200:
        return ""
    g = x[ok] if x[ok].nunique() <= 6 else pd.qcut(x[ok].rank(method="first"), q, labels=False)
    parts = []
    for _, v in df[ok].groupby(g):
        parts.append(f"{v[col].min():.3g}..{v[col].max():.3g}: {v[label].mean():.1%} ({len(v)})")
    return f"{col:>22} | " + " | ".join(parts)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--min-stand", type=float, default=10.0)
    ap.add_argument("--csv", type=pathlib.Path)
    ap.add_argument("--label", default="win_30_100")
    a = ap.parse_args()
    files = sorted((DATA / "density").glob("*.npz"))
    df = pd.concat([day_frame(f, a.min_stand) for f in files], ignore_index=True)
    ap_df = pd.concat([approaches(f, a.min_stand) for f in files], ignore_index=True)
    if a.csv:
        df.to_csv(a.csv, index=False)
    lab = a.label
    print(f"days {len(files)}, touches of densities that stood >= {a.min_stand}s: {len(df)}")
    print("\nwin rate by stop (ticks through) x target (ticks away):")
    for s in STOPS:
        print(f"  stop {s:>2}: " + "  ".join(f"t{t}: {df[f'win_{s}_{t}'].mean():.1%}" for t in TARGETS))
    print(f"\n{lab} by metric quintile (touch-time metrics):")
    for c in ["size", "usd_k", "ratio", "stand_s", "dist0", "pre_pulled", "pre_added", "touch_vs_peak", "relocated", "spread", "tape_speed", "tape_accel",
              "approach_delta", "approach_vol_vs_size", "approach_move", "hour", "side"]:
        s = bins(df, c, lab)
        if s:
            print(s)
    for w in POST_S:
        live = df[df[f"resolved_{w}s"] == 0]
        print(f"\n{lab} by what the level did in the first {w}s on it (touches not resolved by then, n={len(live)}):")
        for c in [f"real_{w}s", f"eat_{w}s", f"refill_{w}s", f"pull_{w}s", f"left_{w}s", f"against_{w}s"]:
            s = bins(live, c, lab)
            if s:
                print(s)
    print("\nclosest approach of price to a standing density before it was touched or ended ($):")
    far = ap_df[ap_df.touched == 0]
    for lim in (2.0, 1.0, 0.5):
        m = ap_df.closest <= lim
        print(f"  came within ${lim}: {m.sum()}, of them touched {ap_df.touched[m].mean():.1%}")
    near = far[far.closest <= 2.0].closest
    if len(near):
        print("  turned without touching, distance quantiles 10/25/50/75/90%: "
              + " ".join(f"{v:.2f}" for v in near.quantile([0.1, 0.25, 0.5, 0.75, 0.9])))


if __name__ == "__main__":
    main()
