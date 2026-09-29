"""GoldTraderEA-style weighted vote (github.com/mehdi-jahani/GoldTraderEA), re-implemented from
its README and inputs with the mechanical modules only (Elliott, harmonic and Wolfe waves are
off by default in the EA too).

H1 bars. Each module votes long or short with the EA's weight; a trade opens at the next bar's
open when the side's weighted votes reach `min_votes`, only with the MA100 trend; stop 2 ATR,
target 4 ATR (the EA's dynamic defaults); one position at a time; exits resolved on 1m candles.

Modules (weight): candle patterns (1: engulfing, pin bar), price action (2: close beyond the
previous bar's range), indicators (1: RSI 30/70 exit, MACD cross), divergence (3: RSI vs price
over 20 bars), support/resistance (3: close back above a 50-bar swing low / below a swing high),
MA crossover (2: EMA 9/21), pivot points (2: previous-day pivot reclaimed), time (1: London or
New York session), volume (2: tick volume above its 20-bar average, with the bar's direction),
multi-timeframe (2: H4 EMA 50 slope).

Weights are also re-fitted walk-forward (logistic regression on the modules, trained on the
previous 2 years) to see whether any combination carries information.

    python -m aegis_lab.research.vote --data hist [--rw SEED]
"""

from __future__ import annotations

import argparse

import numpy as np
import pandas as pd

from aegis_lab.research import smc_models as sm

W = {"candle": 1, "price_action": 2, "indicators": 1, "divergence": 3, "sr": 3, "ma_cross": 2,
     "pivot": 2, "time": 1, "volume": 2, "mtf": 2}


def hourly(m: pd.DataFrame) -> pd.DataFrame:
    g = m.assign(b=(m.t // 3600) * 3600).groupby("b")
    return pd.DataFrame({"t": g.t.first().index, "o": g.o.first().values, "h": g.h.max().values,
                         "l": g.l.min().values, "c": g.c.last().values, "v": g.v.sum().values})


def modules(b: pd.DataFrame) -> dict[str, np.ndarray]:
    """Votes known at each bar's close: +1 long, -1 short, 0 none."""
    o, h, l, c, v = (b[x] for x in "ohlcv")
    rng = (h - l).replace(0, np.nan)
    body = (c - o).abs()
    out = {}
    bull_eng = (c > o) & (o.shift() > c.shift()) & (c >= o.shift()) & (o <= c.shift())
    bear_eng = (c < o) & (o.shift() < c.shift()) & (c <= o.shift()) & (o >= c.shift())
    pin_bull = (np.minimum(o, c) - l > 2 * body) & ((np.minimum(o, c) - l) / rng > 0.6)
    pin_bear = (h - np.maximum(o, c) > 2 * body) & ((h - np.maximum(o, c)) / rng > 0.6)
    out["candle"] = np.where(bull_eng | pin_bull, 1, np.where(bear_eng | pin_bear, -1, 0))
    out["price_action"] = np.where(c > h.shift(), 1, np.where(c < l.shift(), -1, 0))
    d = c.diff()
    rs = d.clip(lower=0).ewm(alpha=1 / 14).mean() / (-d.clip(upper=0)).ewm(alpha=1 / 14).mean()
    rsi = 100 - 100 / (1 + rs)
    macd = c.ewm(span=12).mean() - c.ewm(span=26).mean()
    sig = macd.ewm(span=9).mean()
    x_up = (macd > sig) & (macd.shift() <= sig.shift())
    x_dn = (macd < sig) & (macd.shift() >= sig.shift())
    rsi_up = (rsi > 30) & (rsi.shift() <= 30)
    rsi_dn = (rsi < 70) & (rsi.shift() >= 70)
    out["indicators"] = np.where(x_up | rsi_up, 1, np.where(x_dn | rsi_dn, -1, 0))
    ll = c <= c.rolling(20).min()
    hh = c >= c.rolling(20).max()
    out["divergence"] = np.where(ll & (rsi > rsi.rolling(20).min().shift()), 1,
                                 np.where(hh & (rsi < rsi.rolling(20).max().shift()), -1, 0))
    lo50 = l.rolling(50).min().shift()
    hi50 = h.rolling(50).max().shift()
    out["sr"] = np.where((l <= lo50 * 1.0005) & (c > lo50), 1, np.where((h >= hi50 * 0.9995) & (c < hi50), -1, 0))
    e9, e21 = c.ewm(span=9).mean(), c.ewm(span=21).mean()
    out["ma_cross"] = np.where((e9 > e21) & (e9.shift() <= e21.shift()), 1,
                               np.where((e9 < e21) & (e9.shift() >= e21.shift()), -1, 0))
    day = b.t // 86400
    dd = b.assign(day=day).groupby("day").agg(h=("h", "max"), l=("l", "min"), c=("c", "last"))
    piv = ((dd.h + dd.l + dd.c) / 3).shift().reindex(day).to_numpy()
    out["pivot"] = np.where((c > piv) & (c.shift() <= piv), 1, np.where((c < piv) & (c.shift() >= piv), -1, 0))
    hr = (b.t // 3600) % 24
    sess = ((hr >= 7) & (hr < 16)).to_numpy()
    out["time"] = np.where(sess, np.sign(c - o), 0)
    out["volume"] = np.where(v > v.rolling(20).mean().shift(), np.sign(c - o), 0)
    h4 = c.iloc[3::4]
    e50 = h4.ewm(span=50).mean()
    slope = np.sign(e50.diff()).reindex(c.index).ffill().shift()
    out["mtf"] = slope.fillna(0).to_numpy()
    return {k: np.nan_to_num(np.asarray(x, float)) for k, x in out.items()}


def simulate(b, m, side, stop_atr=2.0, tp_atr=4.0, cost=0.35):
    """side[i] in {-1,0,1}: signal at bar i's close, entry next bar open (first 1m open)."""
    h, l, c = b.h.to_numpy(), b.l.to_numpy(), b.c.to_numpy()
    tr = np.maximum(h - l, np.maximum(abs(h - np.r_[c[0], c[:-1]]), abs(l - np.r_[c[0], c[:-1]])))
    atr = pd.Series(tr).ewm(alpha=1 / 14).mean().to_numpy()
    mt, mo, mh, ml, mc = (m[x].to_numpy() for x in "tohlc")
    t = b.t.to_numpy()
    rows, busy = [], 0
    for i in np.nonzero(side)[0]:
        if t[i] < busy or i + 1 >= len(t):
            continue
        d = int(side[i])
        a = np.searchsorted(mt, t[i] + 3600)
        if a >= len(mt):
            break
        e = mo[a]
        stop, tp = e - d * stop_atr * atr[i], e + d * tp_atr * atr[i]
        x, j = None, a
        for j in range(a, min(a + 60 * 24 * 10, len(mt))):
            if (d > 0 and ml[j] <= stop) or (d < 0 and mh[j] >= stop):
                x = stop
                break
            if (d > 0 and mh[j] >= tp) or (d < 0 and ml[j] <= tp):
                x = tp
                break
        if x is None:
            x = mc[j]
        busy = mt[j] + 60
        risk = stop_atr * atr[i]
        rows.append((t[i], d, d * (x - e) / risk, (d * (x - e) - cost) / risk))
    return pd.DataFrame(rows, columns=["t", "d", "gross", "net"])


def line(df, label):
    if df.empty:
        return f"{label}: no trades"
    yr = pd.to_datetime(df.t, unit="s").dt.year
    by = df.groupby(yr).net.mean()
    tt = df.net.mean() / df.net.std() * np.sqrt(len(df))
    return (f"{label}: n={len(df)} win={100 * (df.net > 0).mean():.1f}% gross {df.gross.mean():+.3f}R "
            f"net {df.net.mean():+.3f}R (t={tt:.1f}) years+ {int((by > 0).sum())}/{len(by)}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default="hist", choices=["hist"])
    ap.add_argument("--rw", type=int, default=None)
    a = ap.parse_args()
    m = sm.hist_minutes()
    if a.rw is not None:
        m = sm.random_walk(m, a.rw)
    b = hourly(m).reset_index(drop=True)
    mods = modules(b)
    trend = np.sign(b.c - b.c.rolling(100).mean()).fillna(0).to_numpy()
    tag = "hist" if a.rw is None else f"rw{a.rw}"
    for mv in [4, 5, 6, 7, 8]:
        score_l = sum(W[k] * (mods[k] > 0) for k in W)
        score_s = sum(W[k] * (mods[k] < 0) for k in W)
        side = np.where((score_l >= mv) & (score_l > score_s) & (trend > 0), 1,
                        np.where((score_s >= mv) & (score_s > score_l) & (trend < 0), -1, 0))
        print(line(simulate(b, m, side), f"{tag} EA weights, votes >= {mv}"))
    # walk-forward weights: logistic regression of the next-24h direction on the module votes
    from sklearn.linear_model import LogisticRegression

    X = np.column_stack([mods[k] for k in W] + [trend])
    fut = np.sign(pd.Series(b.c.to_numpy()).shift(-24) - b.c.to_numpy()).to_numpy()
    yr = pd.to_datetime(b.t, unit="s").dt.year.to_numpy()
    side = np.zeros(len(b), int)
    for y in range(2021, 2026):
        tr = (yr >= y - 2) & (yr < y) & ~np.isnan(fut)
        te = yr == y
        mdl = LogisticRegression(C=0.1, max_iter=500).fit(X[tr], fut[tr] > 0)
        p = mdl.predict_proba(X[te])[:, 1]
        side[te] = np.where(p > 0.55, 1, np.where(p < 0.45, -1, 0))
    print(line(simulate(b, m, side), f"{tag} walk-forward fitted weights (2021-2025)"))


if __name__ == "__main__":
    main()
