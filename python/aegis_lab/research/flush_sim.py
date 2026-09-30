"""Trading the flush: grid / confirmation entries, partial take-profit and trailing, on the
1-second Bybit XAUUSDT rows of `flush extract`, with fills against real trades.

    python -m aegis_lab.research.flush_sim

Events and the book filter come from `flush_report` (drops traded long, rallies short). Per
second, a buy limit at P fills when a trade prints below P or the ask closes at or below P; a
sell limit when a trade prints above P or the bid closes at or above P. Stops and time exits
fill at the worse of the stop and the closing bid (ask for shorts). Fees: Bybit contract maker
0.02% (limits) and taker 0.055% (market, stops, time exits); a second cost set models an
ECN CFD at $0.25/oz round trip. The filter thresholds are fitted on the first half only.
"""

from __future__ import annotations

import gzip

import numba
import numpy as np
import pandas as pd

from aegis_lab.research.flush import DATA
from aegis_lab.research.flush_report import CALENDAR, features, load

PX = DATA / "flush_px"
MAKER, TAKER = 0.0002, 0.00055
CFD_RT = 0.25
# entry kinds
MARKET, GRID, GRID_DEEP, CONFIRM = 0, 1, 2, 3
ENTRIES = {"market": MARKET, "grid 0/0.25/0.5 L": GRID, "grid 0.25/0.5/0.75 L": GRID_DEEP,
           "confirm +0.15 L off the low": CONFIRM}
# exit kinds
EXITS = {"TP 0.5 L": (1.0, 0.5, 0.0), "½ at 0.5 L + trail 0.3 L": (0.5, 0.5, 0.3),
         "trail 0.3 L": (0.0, 0.0, 0.3), "½ at 0.3 L + trail 0.5 L": (0.5, 0.3, 0.5)}
WAIT = 1800
HOLD = 4 * 3600


def trade_range(days: list[str]) -> tuple[np.ndarray, np.ndarray]:
    """Lowest and highest trade price per second (NaN without trades), cached per day."""
    PX.mkdir(parents=True, exist_ok=True)
    lo, hi = [], []
    for day in days:
        f = PX / f"{day}.npz"
        if not f.exists():
            with gzip.open(DATA / "trades" / f"XAUUSDT{day}.csv.gz") as g:
                t = pd.read_csv(g, usecols=["timestamp", "price"])
            t0 = pd.Timestamp(day, tz="UTC").value // 10**9
            s = (t.timestamp.to_numpy() - t0).astype(np.int64)
            ok = (s >= 0) & (s < 86400)
            a = np.full(86400, np.inf)
            b = np.full(86400, -np.inf)
            np.minimum.at(a, s[ok], t.price.to_numpy()[ok])
            np.maximum.at(b, s[ok], t.price.to_numpy()[ok])
            np.savez_compressed(f, lo=a, hi=b)
        z = np.load(f)
        lo.append(z["lo"]), hi.append(z["hi"])
    return np.concatenate(lo), np.concatenate(hi)


@numba.njit(cache=True)
def simulate(bid, ask, tlo, thi, i0, leg, entry, part, tp, trail, maker, taker, cfd):
    """One long trade on oriented prices. Returns (filled, pnl $/oz Bybit, pnl $/oz CFD, R)."""
    n = len(bid)
    p0 = ask[i0]
    if entry == MARKET:
        lv = np.array([p0])
    elif entry == GRID:
        lv = np.array([p0, p0 - 0.25 * leg, p0 - 0.5 * leg])
    elif entry == GRID_DEEP:
        lv = np.array([p0 - 0.25 * leg, p0 - 0.5 * leg, p0 - 0.75 * leg])
    else:
        lv = np.array([np.nan])
    k = len(lv)
    size = np.zeros(k)
    fill = np.zeros(k)
    fee = 0.0
    stop = lv[-1] - 0.3 * leg
    qty, cost = 0.0, 0.0
    low = bid[i0]
    risk0 = 0.0
    te = i0
    armed = False
    # entry phase
    if entry == MARKET:
        qty, cost = 1.0, p0
        fee += taker * abs(p0)
    elif entry == CONFIRM:
        for t in range(i0 + 1, min(i0 + WAIT, n)):
            low = min(low, tlo[t], bid[t])
            if bid[t] <= p0 - 1.5 * leg:
                break
            if bid[t] >= low + 0.15 * leg and low < p0:
                qty, cost, te = 1.0, ask[t], t
                fee += taker * abs(ask[t])
                stop = low - 0.1 * leg
                risk0 = ask[t] - stop
                armed = True
                break
        if not armed:
            return 0.0, 0.0, 0.0, 0.0
    peak = -1e18
    realized = 0.0
    out_q = 0.0
    first_tp = False
    avg = cost
    end = min(i0 + HOLD, n - 1)
    t = te + 1
    while t <= end:
        # grid fills (resting until WAIT or the first take-profit)
        if (entry == GRID or entry == GRID_DEEP) and t - i0 <= WAIT and not first_tp:
            for m in range(k):
                if size[m] == 0.0 and (tlo[t] < lv[m] or ask[t] <= lv[m]):
                    size[m] = 1.0 / k
                    fill[m] = lv[m]
                    fee += maker * abs(lv[m]) / k
            qty = size.sum()
            if qty > 0:
                avg = (size * fill).sum() / qty
        if qty - out_q <= 1e-12:
            if qty > 0 or t - i0 > WAIT:
                break
            t += 1
            continue
        live = qty - out_q
        # stop first (conservative within a second)
        if bid[t] <= stop or tlo[t] <= stop:
            px = min(stop, bid[t])
            realized += live * (px - avg)
            fee += taker * abs(px) * live
            out_q = qty
            break
        peak = max(peak, bid[t], thi[t])
        if part > 0 and not first_tp and (thi[t] > avg + tp * leg or bid[t] >= avg + tp * leg):
            px = avg + tp * leg
            q = qty * part
            realized += q * (px - avg)
            fee += maker * abs(px) * q
            out_q += q
            first_tp = True
            if trail > 0:
                stop = max(stop, avg)  # breakeven on the rest
        if trail > 0 and (part == 0 or first_tp):
            stop = max(stop, peak - trail * leg)
        if part >= 1.0 and first_tp:
            break
        t += 1
    live = qty - out_q
    if live > 1e-12:
        px = bid[min(t, n - 1)]
        realized += live * (px - avg)
        fee += taker * abs(px) * live
    if qty <= 0:
        return 0.0, 0.0, 0.0, 0.0
    risk = max(risk0 if entry == CONFIRM else avg - (lv[-1] - 0.3 * leg), 1e-9)
    return qty, realized - fee, realized - cfd * qty, realized / (risk * qty)


def run(df, lo, hi, ev, d, entry, ex):
    part, tp, trail = EXITS[ex]
    # seconds without a book (daily reopen) carry bid = ask = 0: hold the last quote
    ok = (df.bid > 0) & (df.ask > df.bid)
    b, a = df.bid.where(ok).ffill().to_numpy(float), df.ask.where(ok).ffill().to_numpy(float)
    if d == 1:
        bid, ask, tlo, thi = b, a, lo, hi
    else:
        bid, ask, tlo, thi = -a, -b, -hi, -lo
    rows = []
    for i0, leg in zip(ev.i.to_numpy(), ev.leg_usd.to_numpy()):
        rows.append(simulate(bid, ask, tlo, thi, int(i0), float(leg), ENTRIES[entry], part, tp,
                             trail, MAKER, TAKER, CFD_RT))
    r = np.array(rows).reshape(-1, 4)
    return pd.DataFrame(r, columns=["qty", "net", "cfd", "R"], index=ev.index)


def summary(ev: pd.DataFrame, r: pd.DataFrame) -> dict:
    half = ev.t >= ev.t.median()
    f = r.qty > 0
    out = {"trades": int(f.sum()), "filled": r.qty[f].mean()}
    for name, m in (("A", ~half & f), ("B", half & f)):
        out[f"$/oz {name}"] = r.net[m].mean()
    out["$/oz"] = r.net[f].mean()
    x = r.net[f]
    out["median $/oz"] = x.median()
    out["t"] = x.mean() / (x.std() / np.sqrt(max(len(x), 1)))
    out["CFD $/oz"] = r.cfd[f].mean()
    out["gross R"] = r.R[f].mean()
    out["win %"] = (r.net[f] > 0).mean() * 100
    return out


def random_events(df: pd.DataFrame, ev: pd.DataFrame, seed: int) -> pd.DataFrame:
    rng = np.random.default_rng(seed)
    cand = np.flatnonzero(df.open.to_numpy()[: len(df) - HOLD])
    cand = cand[cand > 5 * 3600]
    i = np.sort(rng.choice(cand, len(ev), replace=False))
    return pd.DataFrame({"i": i, "t": df.t.to_numpy()[i], "leg_usd": rng.permutation(ev.leg_usd.to_numpy())})


def main() -> None:
    pd.set_option("display.width", 220)
    df = load()
    for c in ("bn_buy", "bn_sell"):
        df[c] = 0.0
    df["oi"] = np.nan
    days = [pd.to_datetime(t, unit="s").strftime("%Y-%m-%d") for t in df.t.to_numpy()[::86400]]
    lo, hi = trade_range(days)
    lo = np.where(np.isfinite(lo), lo, df.bid.to_numpy(float))
    hi = np.where(np.isfinite(hi), hi, df.ask.to_numpy(float))
    news = np.sort(pd.read_csv(CALENDAR).time.to_numpy(np.int64))
    print(f"# Trading the flush: {len(days)} days of Bybit XAUUSDT, 1-second fills\n")
    print(f"Costs: Bybit maker {MAKER:.2%} / taker {TAKER:.3%} per side (≈ ${MAKER * 4500:.2f} / "
          f"${TAKER * 4500:.2f} per oz at $4 500), CFD ${CFD_RT}/oz round trip. $/oz per trade.\n")
    for name, theta, window in (("0.30% in 30 min", 0.003, 1800), ("0.50% in 60 min", 0.005, 3600)):
        evs = {d: features(df, d, theta, window, news) for d in (1, -1)}
        a = pd.concat([evs[d][evs[d].t < evs[d].t.median()] for d in (1, -1)])
        push_q, imb_q = a.push_d1_x.quantile(0.6), a.imb_1.quantile(0.4)
        print(f"## {name}\n\nBook filter (fitted on the first half): push-side $1 depth < "
              f"{push_q:.2f}x its 1 h mean and imbalance > {imb_q:+.2f}.\n")
        rows = []
        for flt in ("all", "book filter"):
            for entry in ENTRIES:
                for ex in EXITS:
                    parts, rs = [], []
                    for d in (1, -1):
                        ev = evs[d]
                        if flt == "book filter":
                            ev = ev[(ev.push_d1_x < push_q) & (ev.imb_1 > imb_q)]
                        parts.append(ev), rs.append(run(df, lo, hi, ev, d, entry, ex))
                    ev, r = pd.concat(parts, ignore_index=True), pd.concat(rs, ignore_index=True)
                    rows.append({"events": flt, "entry": entry, "exit": ex, **summary(ev, r)})
        t = pd.DataFrame(rows)
        print(t.to_markdown(index=False, floatfmt="+.2f"), "\n")
        best = t.sort_values("$/oz A", ascending=False).iloc[0]
        print(f"Best on the first half: {best.events} · {best.entry} · {best.exit}. Same rules from "
              "random seconds (same legs, 5 seeds):\n")
        rr = []
        for seed in range(5):
            parts, rs = [], []
            for d in (1, -1):
                ev = random_events(df, evs[d], seed * 2 + (d > 0))
                parts.append(ev), rs.append(run(df, lo, hi, ev, d, best.entry, best.exit))
            rr.append({"seed": seed, **summary(pd.concat(parts, ignore_index=True),
                                               pd.concat(rs, ignore_index=True))})
        print(pd.DataFrame(rr).to_markdown(index=False, floatfmt="+.2f"), "\n")


if __name__ == "__main__":
    main()
