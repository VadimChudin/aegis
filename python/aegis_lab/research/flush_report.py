"""Flush → buyback ("пролив → откуп") on Bybit XAUUSDT: events, outcomes and what separates them.

Input: the 1-second .npz files of `aegis_lab.research.flush extract`, plus Binance XAUUSDT
aggTrades and metrics (open interest) from `aegis_lab.data.binance_vision`.

    python -m aegis_lab.research.flush report

Event (no look-ahead): the mid falls `theta` of price below its high of the last `window`
seconds, in open gold hours (not Fri 21:00 → Sun 22:00 UTC, not the 21:00-22:00 UTC daily
break). At that second (the trigger) the leg L = high - mid. Outcome: "buyback" if the mid then
climbs 0.5 L before it falls 0.5 L more ("continuation"); unresolved after 4 h is dropped. The
next event can only start after the previous one resolves. Rallies (pump → sell-off) are the same
on the mirrored series. Every metric uses data up to the trigger only; "hit side" is the side
being traded into (bids in a drop), "push side" the other one.
"""

from __future__ import annotations

import pathlib
import zipfile

import numba
import numpy as np
import pandas as pd

from aegis_lab.research.flush import COLS, OUT

BINANCE = pathlib.Path.home() / "aegis-data" / "binance" / "XAUUSDT"
CALENDAR = pathlib.Path(__file__).resolve().parents[3] / "crates" / "aegis-core" / "data" / "calendar.csv"
HORIZON = 4 * 3600
BASE = 3600
SETS = (
    ("0.15% in 10 min", 0.0015, 600),
    ("0.30% in 30 min", 0.003, 1800),
    ("0.50% in 60 min", 0.005, 3600),
)
HOURS = [0, 3, 6, 7, 9, 12, 13, 14, 15, 17, 21, 24]


def load() -> pd.DataFrame:
    parts = []
    for f in sorted(OUT.glob("*.npz")):
        z = np.load(f)
        d = {c: z[c] for c in COLS}
        d["t"] = int(z["t0"]) // 1000 + np.arange(86400, dtype=np.int64)
        parts.append(pd.DataFrame(d))
    df = pd.concat(parts, ignore_index=True)
    assert (np.diff(df.t.to_numpy()) == 1).all(), "days must be consecutive"
    ok = (df.bid > 0) & (df.ask > df.bid)
    df["mid"] = ((df.bid + df.ask) / 2).where(ok).ffill().astype(np.float64)
    df["spread"] = (df.ask - df.bid).where(ok)
    ts = pd.to_datetime(df.t, unit="s", utc=True)
    wd, hr = ts.dt.weekday.to_numpy(), ts.dt.hour.to_numpy()
    closed = (wd == 5) | ((wd == 4) & (hr >= 21)) | ((wd == 6) & (hr < 22)) | (hr == 21)
    df["open"] = ~closed & ok.to_numpy()
    df["hour"] = hr
    return df


def binance(t: np.ndarray) -> pd.DataFrame:
    """Binance buy / sell aggressor volume per second and open interest, on the Bybit clock."""
    lo, hi = int(t[0]), int(t[-1])
    buy = np.zeros(len(t))
    sell = np.zeros(len(t))
    for f in sorted((BINANCE / "aggTrades").glob("*.zip")):
        z = zipfile.ZipFile(f)
        with z, z.open(z.namelist()[0]) as fh:
            a = pd.read_csv(fh, usecols=[2, 5, 6], header=0, names=["q", "t", "m"])
        s = a.t.to_numpy() // 1000
        keep = (s >= lo) & (s <= hi)
        if not keep.any():
            continue
        i = s[keep] - lo
        m = a.m.astype(str).str.lower().eq("true").to_numpy()[keep]
        q = a.q.to_numpy()[keep]
        np.add.at(sell, i[m], q[m])  # buyer is maker -> seller is the aggressor
        np.add.at(buy, i[~m], q[~m])
    oi = []
    for f in sorted((BINANCE / "metrics").glob("*.zip")):
        z = zipfile.ZipFile(f)
        with z, z.open(z.namelist()[0]) as fh:
            oi.append(pd.read_csv(fh, usecols=["create_time", "sum_open_interest"]))
    o = pd.concat(oi)
    ot = pd.to_datetime(o.create_time, utc=True).astype("int64").to_numpy() // 10**9
    order = np.argsort(ot)
    ot, ov = ot[order], o.sum_open_interest.to_numpy(float)[order]
    # last value published at least 5 minutes before each second
    k = np.searchsorted(ot, t - 300, side="right") - 1
    oi_s = np.where(k >= 0, ov[np.clip(k, 0, None)], np.nan)
    return pd.DataFrame({"bn_buy": buy, "bn_sell": sell, "oi": oi_s})


@numba.njit(cache=True)
def _events(x, rmax, is_open, theta, window, horizon):
    """x: price oriented so the move is down. Returns trigger, high, resolve index, outcome."""
    n = len(x)
    trig = np.empty(n, np.int64)
    high = np.empty(n, np.int64)
    res = np.empty(n, np.int64)
    out = np.empty(n, np.int64)
    m = 0
    i = window
    while i < n - 1:
        if not is_open[i] or x[i] > rmax[i] - theta * abs(x[i]):
            i += 1
            continue
        h = i - window
        for k in range(i - window, i + 1):
            if x[k] >= x[h]:
                h = k
        leg = x[h] - x[i]
        up, dn = x[i] + 0.5 * leg, x[i] - 0.5 * leg
        j, o = i + 1, -1
        while j < n and j <= i + horizon:
            if x[j] >= up:
                o = 1
                break
            if x[j] <= dn:
                o = 0
                break
            j += 1
        trig[m], high[m], res[m], out[m] = i, h, min(j, n - 1), o
        m += 1
        i = min(j, n - 1) + 1
    return trig[:m], high[:m], res[:m], out[:m]


def _sum(c: np.ndarray, a: np.ndarray, b: np.ndarray) -> np.ndarray:
    """Sum of the series over [a, b) from its cumulative sum c (c[0] = 0)."""
    return c[b] - c[a]


def features(df: pd.DataFrame, d: int, theta: float, window: int, news: np.ndarray) -> pd.DataFrame:
    """d = +1: drops (hit side = bids), d = -1: rallies (hit side = asks)."""
    mid = df.mid.to_numpy()
    x = mid if d == 1 else -mid
    rmax = pd.Series(x).rolling(window, min_periods=window // 2).max().to_numpy()
    trig, high, res, out = _events(x, rmax, df.open.to_numpy(), theta, window, HORIZON)
    keep = (out >= 0) & (high >= BASE + 4 * 3600)
    i, h, j, y = trig[keep], high[keep], res[keep], out[keep]
    hs, ps = ("b", "a") if d == 1 else ("a", "b")
    with_v = df.sell_v if d == 1 else df.buy_v
    agst_v = df.buy_v if d == 1 else df.sell_v
    with_n = df.sell_n if d == 1 else df.buy_n
    max_with = (df.max_sell if d == 1 else df.max_buy).to_numpy()
    bn_with = df.bn_sell if d == 1 else df.bn_buy
    bn_agst = df.bn_buy if d == 1 else df.bn_sell

    def cs(s: pd.Series) -> np.ndarray:
        return np.concatenate([[0.0], np.cumsum(np.nan_to_num(s.to_numpy(np.float64)))])

    c = {k: cs(v) for k, v in {
        "w": with_v, "a": agst_v, "wn": with_n, "bw": bn_with, "ba": bn_agst,
        "hadd": df[f"{hs}_add"], "hpull": df[f"{hs}_pull"],
        "padd": df[f"{ps}_add"], "ppull": df[f"{ps}_pull"],
        "hd1": df[f"{hs}d100"], "hd5": df[f"{hs}d500"], "pd1": df[f"{ps}d100"],
        "pd5": df[f"{ps}d500"], "hmax": df[f"{hs}max"],
    }.items()}
    e1 = i + 1
    b0 = h - BASE
    leg_s = (e1 - h).astype(float)
    last = np.maximum(e1 - 30, h)
    eps = 1e-9

    def rate(k, a, b):
        return _sum(c[k], a, b) / np.maximum(b - a, 1)

    wv, av = _sum(c["w"], h, e1), _sum(c["a"], h, e1)
    w30, a30 = _sum(c["w"], last, e1), _sum(c["a"], last, e1)
    base_rate = (rate("w", b0, h) + rate("a", b0, h)) + eps
    base_trade = (_sum(c["w"], b0, h) + _sum(c["a"], b0, h)) / (_sum(c["wn"], b0, h) * 2 + eps)
    bw, ba = _sum(c["bw"], h, e1), _sum(c["ba"], h, e1)
    bn_base = rate("bw", b0, h) + rate("ba", b0, h) + eps
    hd2_base = (df[f"{hs}d200"].to_numpy()[h] + eps)
    leg = np.abs(mid[h] - mid[i])
    spread = df.spread.to_numpy()
    sp_base = pd.Series(spread).rolling(BASE, min_periods=60).median().to_numpy()
    oi = df.oi.to_numpy()
    tt = df.t.to_numpy()
    k = np.searchsorted(news, tt[i])
    near = np.minimum(
        np.abs(tt[i] - news[np.clip(k - 1, 0, len(news) - 1)]),
        np.abs(news[np.clip(k, 0, len(news) - 1)] - tt[i]),
    ) / 60
    f = pd.DataFrame({
        "t": tt[i],
        "y": y,
        "leg_usd": leg,
        "leg_min": leg_s / 60,
        "move_pct": leg / mid[i] * 100,
        "vol_x": (wv + av) / leg_s / base_rate,
        "delta": (wv - av) / (wv + av + eps),
        "accel_30s": ((w30 + a30) / np.maximum(e1 - last, 1)) / ((wv + av) / leg_s + eps),
        "delta_30s": (w30 - a30) / (w30 + a30 + eps),
        "max_trade_x": np.array([max_with[a:b].max() for a, b in zip(h, e1)]) / (base_trade + eps),
        "hit_d1_x": df[f"{hs}d100"].to_numpy()[i] / (rate("hd1", b0, h) + eps),
        "hit_d5_x": df[f"{hs}d500"].to_numpy()[i] / (rate("hd5", b0, h) + eps),
        "push_d1_x": df[f"{ps}d100"].to_numpy()[i] / (rate("pd1", b0, h) + eps),
        "push_d5_x": df[f"{ps}d500"].to_numpy()[i] / (rate("pd5", b0, h) + eps),
        "imb_1": (df[f"{hs}d100"].to_numpy()[i] - df[f"{ps}d100"].to_numpy()[i])
        / (df[f"{hs}d100"].to_numpy()[i] + df[f"{ps}d100"].to_numpy()[i] + eps),
        "imb_5": (df[f"{hs}d500"].to_numpy()[i] - df[f"{ps}d500"].to_numpy()[i])
        / (df[f"{hs}d500"].to_numpy()[i] + df[f"{ps}d500"].to_numpy()[i] + eps),
        "hit_net_flow": (_sum(c["hadd"], h, e1) - _sum(c["hpull"], h, e1)) / hd2_base,
        "push_net_flow": (_sum(c["padd"], h, e1) - _sum(c["ppull"], h, e1)) / hd2_base,
        "hit_big_x": df[f"{hs}max"].to_numpy()[i] / (rate("hmax", b0, h) + eps),
        "hit_big_usd": df[f"{hs}max_d"].to_numpy()[i],
        "spread_x": spread[i] / sp_base[i],
        "prior_4h_pct": d * (mid[h] - mid[h - 4 * 3600]) / mid[i] * 100,
        "prior_1h_pct": d * (mid[h] - mid[h - 3600]) / mid[i] * 100,
        "bn_vol_x": (bw + ba) / leg_s / bn_base,
        "bn_delta": (bw - ba) / (bw + ba + eps),
        "oi_chg_pct": (oi[i] - oi[np.maximum(h - 300, 0)]) / oi[np.maximum(h - 300, 0)] * 100,
        "news_min": near,
        "hour_utc": df.hour.to_numpy()[i],
    })
    # r > 0: price came back against the move (a drop bought back), in units of the leg
    for name, k in (("r5", 300), ("r15", 900), ("r60", 3600)):
        f[name] = (x[np.minimum(i + k, len(x) - 1)] - x[i]) / leg
    f["i"], f["h"], f["j"] = i, h, j
    return f


def random_baseline(df: pd.DataFrame, leg: float, n: int = 20000, seed: int = 1) -> float:
    """Share of random open seconds where the mid rises leg/2 before it falls leg/2."""
    rng = np.random.default_rng(seed)
    mid = df.mid.to_numpy()
    cand = np.flatnonzero(df.open.to_numpy()[: len(mid) - HORIZON])
    ups = []
    for i in rng.choice(cand, n, replace=False):
        w = mid[i + 1 : i + HORIZON]
        u = np.flatnonzero(w >= mid[i] + leg / 2)
        dn = np.flatnonzero(w <= mid[i] - leg / 2)
        if len(u) or len(dn):
            ups.append(1.0 if (len(u) and (not len(dn) or u[0] < dn[0])) else 0.0)
    return float(np.mean(ups))


def quintiles(f: pd.DataFrame, cols: list[str]) -> pd.DataFrame:
    half = f.t < f.t.median()
    rows = []
    for col in cols:
        v = f[col].replace([np.inf, -np.inf], np.nan)
        ok = v.notna()
        if ok.sum() < 50 or v[ok].nunique() < 5:
            continue
        q = pd.qcut(v[ok].rank(method="first"), 5, labels=False)
        row = {"metric": col}
        for k in range(5):
            m = q == k
            row[f"Q{k + 1}"] = f"{f.y[ok][m].mean() * 100:.0f}"
        lo, hi = (q == 0), (q == 4)
        for name, s in (("A", half[ok]), ("B", ~half[ok])):
            row[f"Q5-Q1 {name}"] = (f.y[ok][hi & s].mean() - f.y[ok][lo & s].mean()) * 100
        row["rho r15"] = f.r15[ok].rank().corr(v[ok].rank())
        row["range"] = f"{v[ok].quantile(0.2):.3g} … {v[ok].quantile(0.8):.3g}"
        rows.append(row)
    out = pd.DataFrame(rows)
    out["stable"] = np.sign(out["Q5-Q1 A"]) == np.sign(out["Q5-Q1 B"])
    return out.sort_values("rho r15", key=np.abs, ascending=False)


def profile(df: pd.DataFrame, f: pd.DataFrame, d: int, at: str) -> pd.DataFrame:
    """Mean of book / tape series around the high (leg start), the trigger or the extreme."""
    hs, ps = ("b", "a") if d == 1 else ("a", "b")
    with_v = (df.sell_v if d == 1 else df.buy_v).to_numpy()
    agst_v = (df.buy_v if d == 1 else df.sell_v).to_numpy()
    hd = df[f"{hs}d100"].to_numpy()
    pdp = df[f"{ps}d100"].to_numpy()
    hflow = (df[f"{hs}_add"] - df[f"{hs}_pull"]).to_numpy()
    mid = df.mid.to_numpy()
    if at == "extreme":
        x = mid if d == 1 else -mid
        idx = np.array([h + np.argmin(x[h : j + 1]) for h, j in zip(f.h, f.j)])
    else:
        idx = f[at].to_numpy()
    rows = []
    for off in (-600, -300, -120, -60, -30, -10, 0, 10, 30, 60, 120, 300, 600):
        a = np.clip(idx + off - 5, 0, len(mid) - 1)
        b = np.clip(idx + off + 5, 1, len(mid))
        wv = np.array([with_v[p:q].sum() for p, q in zip(a, b)])
        av = np.array([agst_v[p:q].sum() for p, q in zip(a, b)])
        rows.append({
            "s": off,
            "with vol/s": wv.mean() / 10,
            "against vol/s": av.mean() / 10,
            "hit $1 depth": hd[np.clip(idx + off, 0, len(mid) - 1)].mean(),
            "push $1 depth": pdp[np.clip(idx + off, 0, len(mid) - 1)].mean(),
            "hit net add/s": np.array([hflow[p:q].sum() for p, q in zip(a, b)]).mean() / 10,
        })
    return pd.DataFrame(rows).set_index("s")


def main() -> None:
    pd.set_option("display.width", 220)
    df = load()
    df = pd.concat([df, binance(df.t.to_numpy())], axis=1)
    cal = pd.read_csv(CALENDAR)
    news = np.sort(cal.time.to_numpy(np.int64))
    days = len(df) // 86400
    print(f"# Bybit XAUUSDT, {days} days "
          f"({pd.to_datetime(df.t.iloc[0], unit='s'):%Y-%m-%d} … "
          f"{pd.to_datetime(df.t.iloc[-1], unit='s'):%Y-%m-%d}), 1-second book + tape\n")
    cols = None
    for name, theta, window in SETS:
        for d, kind in ((1, "drops (пролив → откуп?)"), (-1, "rallies (памп → слив?)")):
            f = features(df, d, theta, window, news)
            cols = cols or [c for c in f.columns if c not in ("t", "y", "r5", "r15", "r60", "i", "h", "j")]
            base = random_baseline(df, float(f.leg_usd.median())) if d == 1 else None
            half = f.t < f.t.median()
            print(f"## {name}: {kind}\n")
            print(f"{len(f)} events ({len(f) / days * 7:.1f} a week), median leg ${f.leg_usd.median():.2f} "
                  f"in {f.leg_min.median():.1f} min. Retrace 0.5 L before 0.5 L more: "
                  f"**{f.y.mean() * 100:.1f}%** (first half {f.y[half].mean() * 100:.1f}%, "
                  f"second {f.y[~half].mean() * 100:.1f}%)"
                  + (f"; random second, same distance up vs down: {base * 100:.1f}%" if base else "")
                  + f". Mean move back after 5 / 15 / 60 min: {f.r5.mean():+.2f} / {f.r15.mean():+.2f} / "
                  f"{f.r60.mean():+.2f} L.\n")
            q = quintiles(f, cols)
            print("Buyback rate (%) by quintile of each metric at the trigger; Q5-Q1 in each half; "
                  "Spearman ρ with the 15-min move back.\n")
            print(q.to_markdown(index=False, floatfmt="+.1f"), "\n")
            hours = pd.cut(f.hour_utc, HOURS, right=False)
            g = f.groupby(hours, observed=True).agg(events=("y", "size"), buyback=("y", "mean"),
                                                    r15=("r15", "mean"))
            g["events"] = g.events / days * 7
            g["buyback"] *= 100
            print("By trigger hour (UTC): events a week, buyback %, mean 15-min move back (L).\n")
            print(g.to_markdown(floatfmt=".2f"), "\n")
            nw = f.news_min <= 30
            print(f"Within 30 min of a CPI / NFP / PPI / PCE / GDP / FOMC release: {nw.sum()} events, "
                  f"buyback {f.y[nw].mean() * 100:.1f}% (others {f.y[~nw].mean() * 100:.1f}%).\n")
            for at in ("h", "i", "extreme"):
                label = {"h": "the high (leg start)", "i": "the trigger", "extreme": "the extreme (after the fact)"}[at]
                for yy, lab in ((1, "buyback"), (0, "continuation")):
                    p = profile(df, f[f.y == yy], d, at)
                    print(f"Around {label}, {lab} ({(f.y == yy).sum()}):\n")
                    print(p.to_markdown(floatfmt=".3f"), "\n")
            f.to_parquet(OUT.parent / f"flush_events_{theta}_{'down' if d == 1 else 'up'}.parquet")


if __name__ == "__main__":
    main()
