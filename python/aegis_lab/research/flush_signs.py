"""Do algorithms leave traces on the gold book and tape? A wide, second-by-second search.

    python -m aegis_lab.research.flush_signs

Instead of "event → outcome" this samples every 5th open second of 204 days of Bybit XAUUSDT
and asks whether ~45 causal features (book depth and imbalance at four distances, refill /
eaten / pulled flows, aggressor delta and volume, absorption, largest trades, position in the
range, time spent at price (volume-profile node), VWAP, the Binance price and tape as a lead,
news, time) predict the mid 5 s, 30 s, 2 min, 10 min and 30 min ahead.

1. Harness checks: a planted edge must be found, shuffled targets must give nothing.
2. Rank correlation of every feature with every horizon, in both halves.
3. Walk-forward by month, linear (ridge) and non-linear (LightGBM); out-of-sample IC and the
   trades the predictions would make, with Bybit, maker-only and CFD costs.
4. The "bottom" subset: seconds at a fresh 30-min low after a drop — what the model sees there.
"""

from __future__ import annotations

import pathlib
import zipfile

import numba
import numpy as np
import pandas as pd

from aegis_lab.research.flush_report import CALENDAR, load

BINANCE = pathlib.Path.home() / "aegis-data" / "binance" / "XAUUSDT"
STEP = 5
HORIZONS = (5, 30, 120, 600, 1800)
TAKER_BP, MAKER_BP, CFD_BP = 11.0, 4.0, 0.55  # round trip: Bybit 2x0.055%, 2x0.02%, $0.25/oz


def binance_seconds(t: np.ndarray) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Binance last trade price, aggressor buy and sell volume per second (cached)."""
    cache = BINANCE / f"sec_{t[0]}_{t[-1]}.npz"
    if cache.exists():
        z = np.load(cache)
        return z["px"], z["buy"], z["sell"]
    lo = int(t[0])
    n = len(t)
    px = np.full(n, np.nan)
    buy = np.zeros(n)
    sell = np.zeros(n)
    for f in sorted((BINANCE / "aggTrades").glob("*.zip")):
        z = zipfile.ZipFile(f)
        with z, z.open(z.namelist()[0]) as fh:
            a = pd.read_csv(fh, usecols=[1, 2, 5, 6], header=0, names=["p", "q", "t", "m"])
        s = a.t.to_numpy() // 1000 - lo
        keep = (s >= 0) & (s < n)
        if not keep.any():
            continue
        s, p, q = s[keep], a.p.to_numpy()[keep], a.q.to_numpy()[keep]
        m = a.m.astype(str).str.lower().eq("true").to_numpy()[keep]
        np.add.at(sell, s[m], q[m])
        np.add.at(buy, s[~m], q[~m])
        px[s] = p  # rows are in time order, so the last write per second is the last trade
    px = pd.Series(px).ffill(limit=60).to_numpy()
    np.savez_compressed(cache, px=px, buy=buy, sell=sell)
    return px, buy, sell


@numba.njit(cache=True)
def time_at_price(mid, idx, window, half):
    """Share of the last `window` seconds the mid spent within ±half bins of the mid at idx."""
    b = np.empty(len(mid), np.int64)
    lo = 1 << 60
    for i in range(len(mid)):
        b[i] = round(mid[i] * 4)  # $0.25 bins
        lo = min(lo, b[i])
    hist = np.zeros(int(b.max() - lo) + 1 + 2 * half)
    out = np.full(len(idx), np.nan)
    k = 0
    for i in range(len(mid)):
        hist[b[i] - lo + half] += 1
        if i >= window:
            hist[b[i - window] - lo + half] -= 1
        while k < len(idx) and idx[k] == i:
            if i >= window:
                c = b[i] - lo + half
                out[k] = hist[c - half: c + half + 1].sum() / window
            k += 1
    return out


def build(df: pd.DataFrame) -> pd.DataFrame:
    ok = (df.bid > 0) & (df.ask > df.bid)
    bid = df.bid.where(ok).ffill().bfill().to_numpy(np.float64)
    ask = df.ask.where(ok).ffill().bfill().to_numpy(np.float64)
    mid = (bid + ask) / 2
    n = len(mid)
    is_open = df.open.to_numpy()
    closed_c = np.concatenate([[0], np.cumsum(~is_open)])

    def cs(x) -> np.ndarray:
        return np.concatenate([[0.0], np.cumsum(np.nan_to_num(np.asarray(x, np.float64)))])

    buy, sell = df.buy_v.to_numpy(np.float64), df.sell_v.to_numpy(np.float64)
    c = {k: cs(v) for k, v in {
        "buy": buy, "sell": sell, "n": df.buy_n + df.sell_n,
        "badd": df.b_add, "bpull": df.b_pull, "beat": df.b_eat,
        "aadd": df.a_add, "apull": df.a_pull, "aeat": df.a_eat,
        "bd100": df.bd100, "ad100": df.ad100, "bd200": df.bd200, "ad200": df.ad200,
        "pv": mid * (buy + sell),
    }.items()}
    bn_px, bn_buy, bn_sell = binance_seconds(df.t.to_numpy())
    c["bnb"], c["bns"] = cs(bn_buy), cs(bn_sell)

    maxh = max(HORIZONS)
    i = np.arange(4 * 3600, n - maxh, STEP)
    i = i[is_open[i] & (closed_c[i + maxh + 1] - closed_c[i - 3600] == 0)]

    def w(k: str, s: int) -> np.ndarray:
        return c[k][i + 1] - c[k][i + 1 - s]

    eps = 1e-9
    f = pd.DataFrame(index=np.arange(len(i)))
    for s in (10, 60, 300, 1800):
        f[f"ret_{s}"] = (mid[i] - mid[i - s]) / mid[i] * 1e4
    for s in (10, 60, 300):
        b, a = w("buy", s), w("sell", s)
        f[f"dlt_{s}"] = (b - a) / (b + a + eps)
    vol60 = w("buy", 60) + w("sell", 60)
    vol1h = w("buy", 3600) + w("sell", 3600)
    f["vol_60x"] = vol60 / (vol1h / 60 + eps)
    f["trades_60x"] = w("n", 60) / (w("n", 3600) / 60 + eps)
    # absorption: net aggression per $ of price move (big selling that does not move price)
    f["flow_per_move_60"] = (w("buy", 60) - w("sell", 60)) / (np.abs(mid[i] - mid[i - 60]) + 0.05)
    f["flow_per_move_300"] = (w("buy", 300) - w("sell", 300)) / (np.abs(mid[i] - mid[i - 300]) + 0.05)
    for band in (50, 100, 200, 500):
        bd, ad = df[f"bd{band}"].to_numpy()[i], df[f"ad{band}"].to_numpy()[i]
        f[f"imb_{band}"] = (bd - ad) / (bd + ad + eps)
    imb100 = (df.bd100 - df.ad100) / (df.bd100 + df.ad100 + eps)
    f["dimb_100_60"] = imb100.to_numpy()[i] - imb100.to_numpy()[i - 60]
    f["bdep_100x"] = df.bd100.to_numpy()[i] / (w("bd100", 3600) / 3600 + eps)
    f["adep_100x"] = df.ad100.to_numpy()[i] / (w("ad100", 3600) / 3600 + eps)
    bref = w("bd200", 3600) / 3600 + eps
    aref = w("ad200", 3600) / 3600 + eps
    for s in (10, 60):
        f[f"bflow_{s}"] = (w("badd", s) - w("bpull", s) - w("beat", s)) / bref
        f[f"aflow_{s}"] = (w("aadd", s) - w("apull", s) - w("aeat", s)) / aref
        f[f"bpull_{s}"] = w("bpull", s) / bref
        f[f"apull_{s}"] = w("apull", s) / aref
    f["beat_share_60"] = w("beat", 60) / (w("beat", 60) + w("bpull", 60) + eps)
    f["aeat_share_60"] = w("aeat", 60) / (w("aeat", 60) + w("apull", 60) + eps)
    med_trade = float(np.median(df.max_sell[df.max_sell > 0]))
    f["big_sell_60"] = df.max_sell.rolling(60, min_periods=1).max().to_numpy()[i] / med_trade
    f["big_buy_60"] = df.max_buy.rolling(60, min_periods=1).max().to_numpy()[i] / med_trade
    f["spread_bp"] = (ask[i] - bid[i]) / mid[i] * 1e4
    f["bmax_share"] = df.bmax.to_numpy()[i] / (df.bd500.to_numpy()[i] + eps)
    f["amax_share"] = df.amax.to_numpy()[i] / (df.ad500.to_numpy()[i] + eps)
    f["bmax_usd"] = df.bmax_d.to_numpy()[i]
    f["amax_usd"] = df.amax_d.to_numpy()[i]
    ms = pd.Series(mid)
    for s in (1800, 14400):
        lo, hi = ms.rolling(s).min().to_numpy()[i], ms.rolling(s).max().to_numpy()[i]
        f[f"pos_{s}"] = (mid[i] - lo) / (hi - lo + eps)
        f[f"from_low_{s}"] = (mid[i] - lo) / mid[i] * 1e4
        f[f"from_high_{s}"] = (hi - mid[i]) / mid[i] * 1e4
    f["time_at_price_4h"] = time_at_price(mid, i, 14400, 4)
    vwap = w("pv", 3600) / (vol1h + eps)
    f["vwap_dev_1h"] = np.where(vol1h > 0, (mid[i] - vwap) / mid[i] * 1e4, np.nan)
    bnb, bns = w("bnb", 10), w("bns", 10)
    f["bn_dlt_10"] = (bnb - bns) / (bnb + bns + eps)
    bnb, bns = w("bnb", 60), w("bns", 60)
    f["bn_dlt_60"] = (bnb - bns) / (bnb + bns + eps)
    f["bn_premium_bp"] = (bn_px[i] - mid[i]) / mid[i] * 1e4
    f["bn_ret_10_rel"] = ((bn_px[i] - bn_px[i - 10]) - (mid[i] - mid[i - 10])) / mid[i] * 1e4
    news = np.sort(pd.read_csv(CALENDAR).time.to_numpy(np.int64))
    tt = df.t.to_numpy()[i]
    k = np.clip(np.searchsorted(news, tt), 1, len(news) - 1)
    prev, nxt = tt - news[k - 1], news[k] - tt
    f["news_min"] = np.where(prev < nxt, prev, -nxt) / 60
    ts = pd.to_datetime(tt, unit="s", utc=True)
    f["hour"] = ts.hour + ts.minute / 60
    f["weekday"] = ts.weekday
    for h in HORIZONS:
        f[f"y_{h}"] = (mid[i + h] - mid[i]) / mid[i] * 1e4
    f["t"], f["i"] = tt, i
    f["month"] = ts.strftime("%Y-%m")
    return f, bid, ask


FEATURES: list[str] = []


def rank_ic(x: pd.Series, y: pd.Series) -> float:
    m = x.notna() & y.notna() & np.isfinite(x)
    return float(x[m].rank().corr(y[m].rank()))


def ic_table(f: pd.DataFrame) -> pd.DataFrame:
    half = f.t >= f.t.median()
    rows = []
    for col in FEATURES:
        row = {"feature": col}
        for h in HORIZONS:
            a, b = rank_ic(f[col][~half], f[f"y_{h}"][~half]), rank_ic(f[col][half], f[f"y_{h}"][half])
            row[f"{h}s A"], row[f"{h}s B"] = a, b
        rows.append(row)
    out = pd.DataFrame(rows)
    # the strongest horizon where both halves agree in sign
    best = np.zeros(len(out))
    for h in HORIZONS:
        a, b = out[f"{h}s A"].to_numpy(), out[f"{h}s B"].to_numpy()
        best = np.maximum(best, np.where(np.sign(a) == np.sign(b), np.minimum(abs(a), abs(b)), 0.0))
    out["min |IC| same sign"] = best
    return out.sort_values("min |IC| same sign", ascending=False)


def fit_predict(kind: str, xtr, ytr, xte, seed: int = 0):
    if kind == "ridge":
        lo, hi = np.nanpercentile(xtr, 1, axis=0), np.nanpercentile(xtr, 99, axis=0)
        def prep(x):
            x = np.clip(x, lo, hi)
            return np.nan_to_num((x - mu) / sd)
        mu = np.nanmean(np.clip(xtr, lo, hi), axis=0)
        sd = np.nanstd(np.clip(xtr, lo, hi), axis=0) + 1e-9
        a, b = prep(xtr), prep(xte)
        a1 = np.c_[a, np.ones(len(a))]
        beta = np.linalg.solve(a1.T @ a1 + 10.0 * np.eye(a1.shape[1]), a1.T @ ytr)
        return a1 @ beta, np.c_[b, np.ones(len(b))] @ beta
    import lightgbm as lgb

    params = {"objective": "regression", "learning_rate": 0.03, "num_leaves": 31,
              "min_data_in_leaf": 500, "bagging_fraction": 0.5, "bagging_freq": 1,
              "feature_fraction": 0.7, "lambda_l2": 10.0, "seed": seed, "verbose": -1,
              "num_threads": 4}
    m = lgb.train(params, lgb.Dataset(xtr, ytr), num_boost_round=300)
    return m.predict(xtr), m.predict(xte)


@numba.njit(cache=True)
def trades(idx, pred, lo_thr, hi_thr, y, cost, h):
    """Non-overlapping trades on the signal. y: the target move (bp) of each row, cost: half
    spread at entry + half spread at exit (bp). Returns (gross bp, bp after crossing the spread)."""
    g, x = [], []
    busy = -1
    for k in range(len(idx)):
        if idx[k] < busy:
            continue
        s = 1 if pred[k] >= hi_thr else (-1 if pred[k] <= lo_thr else 0)
        if s == 0:
            continue
        g.append(s * y[k])
        x.append(s * y[k] - cost[k])
        busy = idx[k] + h
    return np.array(g), np.array(x)


def walk_forward(f: pd.DataFrame, bid, ask, kind: str, target: str | None = None,
                 shuffle: bool = False, planted: np.ndarray | None = None,
                 y_of=None, feats: list[str] | None = None, label: str = "",
                 horizons=HORIZONS) -> pd.DataFrame:
    months = sorted(f.month.unique())
    rows = []
    rng = np.random.default_rng(0)
    x_all = f[feats or FEATURES].to_numpy(np.float32)
    ii_all = f.i.to_numpy()
    for h in ([int(target)] if target else horizons):
        y = (y_of(h) if y_of else f[f"y_{h}"].to_numpy(np.float64)).copy()
        mid = (bid + ask) / 2
        cost_all = ((ask - bid)[ii_all] + (ask - bid)[ii_all + h]) / 2 / mid[ii_all] * 1e4
        if planted is not None:
            y = y + planted
        if shuffle:
            y = rng.permutation(y)
        preds, idx, ys, mon = [], [], [], []
        thr = []
        for m in months[2:]:
            tr = (f.month < m).to_numpy().copy()
            # drop the last horizon of the train set so no train target overlaps the test month
            tr &= f.t.to_numpy() < f.t[f.month == m].min() - h
            te = (f.month == m).to_numpy()
            sub = np.flatnonzero(tr)
            if len(sub) > 800_000:
                sub = np.sort(rng.choice(sub, 800_000, replace=False))
            sub = sub[np.isfinite(y[sub])]
            ptr, pte = fit_predict(kind, x_all[sub], y[sub], x_all[te])
            preds.append(pte), idx.append(ii_all[te]), ys.append(y[te]), mon.append(cost_all[te])
            thr.append((np.quantile(ptr, 0.005), np.quantile(ptr, 0.995)))
        p, ii, yy = np.concatenate(preds), np.concatenate(idx), np.concatenate(ys)
        ic = pd.Series(p).rank().corr(pd.Series(yy).rank())  # NaN targets are skipped
        g_all, x_all_t = [], []
        for (lo, hi), pp, ix, y_, c_ in zip(thr, preds, idx, ys, mon):
            ok = np.isfinite(y_)
            g, x = trades(ix[ok], pp[ok], lo, hi, y_[ok], c_[ok], h)
            g_all.append(g), x_all_t.append(x)
        g, x = np.concatenate(g_all), np.concatenate(x_all_t)
        mon_ic = [pd.Series(pp).rank().corr(pd.Series(y_).rank()) for pp, y_ in zip(preds, ys)]
        rows.append({
            "model": kind + label + (" (shuffled y)" if shuffle else "") + (" (planted)" if planted is not None else ""),
            "horizon s": h, "OOS IC": ic, "months IC>0": f"{sum(v > 0 for v in mon_ic)}/{len(mon_ic)}",
            "trades": len(g), "gross bp (mid)": g.mean() if len(g) else np.nan,
            "t": g.mean() / (g.std() / np.sqrt(len(g))) if len(g) > 2 else np.nan,
            "net bp crossing spread": x.mean() if len(x) else np.nan,
            "net Bybit taker": x.mean() - TAKER_BP if len(x) else np.nan,
            "net maker (fills at mid)": g.mean() - MAKER_BP if len(g) else np.nan,
            "net CFD $0.25": g.mean() - CFD_BP if len(g) else np.nan,
        })
        if target:
            return pd.DataFrame(rows), p, ii
    return pd.DataFrame(rows)


def main() -> None:
    import argparse

    ap = argparse.ArgumentParser()
    ap.add_argument("--only", type=int, nargs="*", default=[1, 2, 3, 4, 5])
    only = ap.parse_args().only
    pd.set_option("display.width", 250)
    cache = pathlib.Path.home() / "aegis-data" / "bybit" / "XAUUSDT" / "signs"
    if (cache / "features.parquet").exists():
        f = pd.read_parquet(cache / "features.parquet")
        z = np.load(cache / "quotes.npz")
        bid, ask = z["bid"], z["ask"]
    else:
        df = load()
        f, bid, ask = build(df)
        del df
        cache.mkdir(parents=True, exist_ok=True)
        f.to_parquet(cache / "features.parquet")
        np.savez(cache / "quotes.npz", bid=bid, ask=ask)
    FEATURES[:] = [c for c in f.columns if not c.startswith("y_") and c not in ("t", "i", "month")]
    print(f"# Traces on the gold book and tape: {len(f):,} sampled seconds, {len(FEATURES)} features, "
          f"{f.month.nunique()} months\n")
    print(f"Costs, round trip: Bybit taker {TAKER_BP} bp (+ spread), maker {MAKER_BP} bp, CFD at "
          f"$0.25/oz ≈ {CFD_BP} bp. Median |move| by horizon (bp): "
          + ", ".join(f"{h}s {f[f'y_{h}'].abs().median():.2f}" for h in HORIZONS) + "\n")

    if 1 in only:
        harness(f, bid, ask)
    if 2 in only:
        print("## 2. Rank correlation of each feature with the move ahead (A / B = halves)\n")
        print(ic_table(f).to_markdown(index=False, floatfmt="+.3f"), "\n")
    if 3 in only:
        print("## 3. Walk-forward by month (train on all earlier months), top / bottom 0.5% of predictions\n")
        res = [walk_forward(f, bid, ask, k) for k in ("ridge", "lgbm")]
        print(pd.concat(res).to_markdown(index=False, floatfmt="+.3f"), "\n")
    if 4 in only:
        bottoms(f, bid, ask)
    if 5 in only:
        other_venue(f, bid, ask)


def harness(f, bid, ask) -> None:
    print("## 1. Does the harness find an edge when there is one?\n")
    # plant +0.5 bp on the 2-min return after a real book state (imbalance top decile), −0.5 bp
    # after the bottom decile; everything else untouched
    z = f.imb_100.to_numpy()
    hi, lo = np.nanquantile(z, 0.9), np.nanquantile(z, 0.1)
    planted = np.where(z >= hi, 0.5, np.where(z <= lo, -0.5, 0.0))
    checks = [walk_forward(f, bid, ask, "ridge", "120")[0],
              walk_forward(f, bid, ask, "ridge", "120", planted=planted)[0],
              walk_forward(f, bid, ask, "ridge", "120", shuffle=True)[0]]
    print(pd.concat(checks).to_markdown(index=False, floatfmt="+.3f"), "\n")


def other_venue(f, bid, ask) -> None:
    print("## 5. Does the Bybit signal move another venue? (target: the Binance XAUUSDT price)\n")
    bn_px = binance_seconds(np.arange(int(f.t.iloc[0]) - int(f.i.iloc[0]),
                                      int(f.t.iloc[0]) - int(f.i.iloc[0]) + len(bid)))[0]
    ii = f.i.to_numpy()

    def y_of(h):
        return (bn_px[ii + h] - bn_px[ii]) / bn_px[ii] * 1e4

    bybit_only = [c for c in FEATURES if not c.startswith("bn_")]
    res = [walk_forward(f, bid, ask, "ridge", y_of=y_of, feats=bybit_only, label=" · Bybit features",
                        horizons=(5, 30, 120)),
           walk_forward(f, bid, ask, "ridge", y_of=y_of, label=" · all features", horizons=(5, 30, 120))]
    print(pd.concat(res).to_markdown(index=False, floatfmt="+.3f"), "\n")
    print("Costs in this table are Bybit spreads; Binance taker fees are about the same as Bybit's.\n")


def bottoms(f, bid, ask) -> None:
    print("## 4. At a fresh 30-min low after a drop (the \"bottom\" the eye sees)\n")
    bottom = (f.from_low_1800 <= 0.5) & (f.from_high_1800 >= 20)
    rows = []
    for h in HORIZONS:
        y = f[f"y_{h}"]
        rows.append({"horizon s": h, "rows": int(bottom.sum()), "mean bp at the low": y[bottom].mean(),
                     "mean bp elsewhere": y[~bottom].mean(), "up share at the low": (y[bottom] > 0).mean()})
    print(pd.DataFrame(rows).to_markdown(index=False, floatfmt="+.3f"), "\n")
    fb = f[bottom]
    half = fb.t >= fb.t.median()
    rows = []
    for col in FEATURES:
        a, b = rank_ic(fb[col][~half], fb.y_600[~half]), rank_ic(fb[col][half], fb.y_600[half])
        rows.append({"feature": col, "IC 10 min A": a, "IC 10 min B": b})
    rb = pd.DataFrame(rows)
    rb["same sign"] = np.sign(rb["IC 10 min A"]) == np.sign(rb["IC 10 min B"])
    rb = rb.reindex(rb[["IC 10 min A", "IC 10 min B"]].abs().min(axis=1).sort_values(ascending=False).index)
    print("Which features separate lows that turn from lows that break (10-min move, both halves):\n")
    print(rb.head(15).to_markdown(index=False, floatfmt="+.3f"), "\n")
    _, p, ii = walk_forward(f, bid, ask, "lgbm", "600")
    fb_i = set(f.i[bottom].tolist())
    sel = np.array([x in fb_i for x in ii])
    y600 = f.set_index("i").y_600.reindex(ii).to_numpy()
    q = pd.qcut(pd.Series(p[sel]).rank(method="first"), 5, labels=False)
    print("Out-of-sample 10-min model at those lows, by quintile of its prediction (mean bp, up share):\n")
    print(pd.DataFrame({"y": y600[sel], "q": q.to_numpy()}).groupby("q").y.agg(
        ["size", "mean", lambda v: (v > 0).mean()]).rename(columns={"<lambda_0>": "up share"})
          .to_markdown(floatfmt="+.3f"), "\n")


if __name__ == "__main__":
    main()
