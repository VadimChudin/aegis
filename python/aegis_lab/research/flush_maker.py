"""Can the second-scale book signal pay for itself with limit orders on Bybit XAUUSDT?

    python -m aegis_lab.research.flush_maker

Signals: the walk-forward ridge predictions of `flush_signs` (5 s and 30 s horizons, top and
bottom 0.5% / 2% of the train predictions, thresholds from earlier months only). On a buy signal
a limit buy rests at the best bid for `wait` seconds; it counts as filled only when a trade
prints *below* it (trade-through, no queue luck) or, in the optimistic variant, at it. After
`hold` seconds the position is closed either by a market order or by a limit at the best ask
(trade-through fill within `wait`, else a market order). Sells mirror this. Fees: the current
TradFi-perpetual discount (maker 0%, taker 0.0275%, Bybit Learn, Aug 2026) and the standard
rates (0.02% / 0.055%). Every trade is also shown at the signal's own mid-to-mid move, and the
signals that were not filled are kept to show the selection.
"""

from __future__ import annotations

import pathlib

import numba
import numpy as np
import pandas as pd

from aegis_lab.research import flush_signs as fs
from aegis_lab.research.flush_sim import trade_range

FEES = {"discount (maker 0, taker 2.75 bp)": (0.0, 2.75), "standard (2 / 5.5 bp)": (2.0, 5.5)}


@numba.njit(cache=True)
def maker(idx, pred, lo_thr, hi_thr, bid, ask, tlo, thi, wait, hold, touch, exit_maker):
    """Per signal: side, filled, pnl in bp vs entry mid (before fees), maker legs, mid-to-mid move."""
    n = len(bid)
    out = np.full((len(idx), 5), np.nan)
    busy = -1
    for k in range(len(idx)):
        t = idx[k]
        s = 1 if pred[k] >= hi_thr[k] else (-1 if pred[k] <= lo_thr[k] else 0)
        if s == 0 or t < busy or t + wait * 2 + hold + 2 >= n:
            continue
        m0 = (bid[t] + ask[t]) / 2
        lim = bid[t] if s > 0 else ask[t]
        fu = -1
        for u in range(t + 1, t + wait + 1):
            if s > 0 and (tlo[u] < lim or (touch and tlo[u] <= lim)):
                fu = u
                break
            if s < 0 and (thi[u] > lim or (touch and thi[u] >= lim)):
                fu = u
                break
        out[k, 0] = s
        out[k, 4] = s * ((bid[t + hold] + ask[t + hold]) / 2 - m0) / m0 * 1e4
        if fu < 0:
            out[k, 1] = 0.0
            busy = t + wait
            continue
        e = fu + hold
        legs = 1.0
        if exit_maker:
            xl = ask[e] if s > 0 else bid[e]
            px = np.nan
            for v in range(e + 1, e + wait + 1):
                if (s > 0 and (thi[v] > xl or (touch and thi[v] >= xl))) or \
                   (s < 0 and (tlo[v] < xl or (touch and tlo[v] <= xl))):
                    px = xl
                    legs = 2.0
                    e = v
                    break
            if np.isnan(px):
                e = e + wait
                px = bid[e] if s > 0 else ask[e]
        else:
            px = bid[e] if s > 0 else ask[e]
        out[k, 1] = 1.0
        out[k, 2] = s * (px - lim) / m0 * 1e4
        out[k, 3] = legs
        busy = e
    return out


def main() -> None:
    pd.set_option("display.width", 220)
    cache = pathlib.Path.home() / "aegis-data" / "bybit" / "XAUUSDT" / "signs"
    f = pd.read_parquet(cache / "features.parquet")
    z = np.load(cache / "quotes.npz")
    bid, ask = z["bid"], z["ask"]
    fs.FEATURES[:] = [c for c in f.columns if not c.startswith("y_") and c not in ("t", "i", "month")]
    t0 = int(f.t.iloc[0]) - int(f.i.iloc[0])
    days = pd.date_range(pd.to_datetime(t0, unit="s"), periods=len(bid) // 86400, freq="D").strftime("%Y-%m-%d")
    tlo, thi = trade_range(list(days))
    tlo = np.where(np.isfinite(tlo), tlo, np.inf)
    thi = np.where(np.isfinite(thi), thi, -np.inf)
    print("# Limit-order execution of the book signal, Bybit XAUUSDT\n")
    rows = []
    for h in (5, 30):
        _, p, ii = fs.walk_forward(f, bid, ask, "ridge", str(h))
        # thresholds: quantiles of the previous months' out-of-sample predictions (causal)
        mon = f.set_index("i").month.reindex(ii).to_numpy()
        for q in (0.005, 0.02):
            lo_thr = np.full(len(p), np.nan)
            hi_thr = np.full(len(p), np.nan)
            for m in np.unique(mon):
                past = mon < m
                if past.sum() < 10000:
                    continue
                cur = mon == m
                lo_thr[cur], hi_thr[cur] = np.quantile(p[past], q), np.quantile(p[past], 1 - q)
            ok = np.isfinite(lo_thr)
            for wait in (5, 30):
                for touch in (False, True):
                    for exit_maker in (False, True):
                        o = maker(ii[ok], p[ok], lo_thr[ok], hi_thr[ok], bid, ask, tlo, thi,
                                  wait, h, touch, exit_maker)
                        o = o[~np.isnan(o[:, 0])]
                        fl = o[:, 1] == 1
                        row = {"horizon s": h, "signal": f"top/bottom {q:.1%}", "wait s": wait,
                               "fill": "touch" if touch else "trade-through",
                               "exit": "limit" if exit_maker else "market",
                               "signals": len(o), "filled %": fl.mean() * 100,
                               "signal mid move bp (all)": o[:, 4].mean(),
                               "…filled": o[fl, 4].mean(), "…not filled": o[~fl, 4].mean(),
                               "trade bp before fees": o[fl, 2].mean()}
                        for name, (mk, tk) in FEES.items():
                            fee = mk + np.where(o[fl, 3] == 2, mk, tk)
                            net = o[fl, 2] - fee
                            row[f"net {name.split(' ')[0]}"] = net.mean()
                            row[f"t {name.split(' ')[0]}"] = net.mean() / (net.std() / np.sqrt(len(net)))
                        rows.append(row)
    print(pd.DataFrame(rows).to_markdown(index=False, floatfmt="+.2f"), "\n")


if __name__ == "__main__":
    main()
