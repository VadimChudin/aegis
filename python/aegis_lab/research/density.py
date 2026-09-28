"""Order-book densities ("плотности") on Bybit XAUUSDT: replay the 200-level book with the tape
and record the life of every large resting level.

    python -m aegis_lab.research.density extract [--days N]   # one .npz per day
    python -m aegis_lab.research.density_report               # analysis

An episode starts when a level within `MAX_DIST` of the mid holds at least `K_START` x the median
level size of its side, and ends when it falls below `K_END` x that median for `DORMANT_S`
seconds, or when price trades through it. Every size decrease is split into traded volume
(trades at that price on that side since the previous change of the level) and cancelled volume;
an increase after the level was hit is a refill.

The book feed lags the tape by ~40 ms (median) and leads it in ~13% of changes, so trades wait
up to `PENDING_MS` for the book decrease they explain, and a decrease with no trades yet is
re-labelled as traded if trades at that price arrive within `LATE_MS`.
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import gzip
import io
import pathlib
import zipfile

import numpy as np
import orjson
import pandas as pd

DATA = pathlib.Path.home() / "aegis-data" / "bybit" / "XAUUSDT"
K_START = 10.0
K_END = 4.0
MAX_DIST = 500  # ticks ($5)
DORMANT_S = 3.0
PENDING_MS = 1000.0
LATE_MS = 300.0

# end reasons
THROUGH, PULLED, EATEN, OUT, EOD = 1, 2, 3, 4, 5


class Ep:
    __slots__ = (
        "side", "p", "t0", "size0", "peak", "dist0", "traded", "cancel", "added", "refill",
        "n_refill", "hit_since_add", "t_touch", "touch_size", "pre_cancel", "pre_added",
        "pre_peak", "t_top", "t_low", "t_end", "reason", "hist", "size", "size_before_low",
        "late", "n_changes",
    )

    def __init__(self, side: int, p: int, t: float, size: float, dist: int):
        self.side, self.p, self.t0, self.size0, self.peak, self.dist0 = side, p, t, size, size, dist
        self.traded = self.cancel = self.added = self.refill = 0.0
        self.n_refill = 0
        self.hit_since_add = False
        self.t_touch = self.t_top = self.t_low = self.t_end = np.nan
        self.touch_size = self.pre_cancel = self.pre_added = self.pre_peak = np.nan
        self.size_before_low = np.nan
        self.late = (0.0, 0.0)  # (time, cancelled volume still waiting for its trades)
        self.n_changes = 0
        self.reason = 0
        self.size = size
        self.hist: list[tuple[float, float, float, float]] = [(t, size, 0.0, 0.0)]

    def touch(self, t: float, size: float) -> None:
        if np.isnan(self.t_touch):
            self.t_touch, self.touch_size = t, size
            self.pre_cancel, self.pre_added, self.pre_peak = self.cancel, self.added, self.peak


def _trades(day: str) -> pd.DataFrame:
    with gzip.open(DATA / "trades" / f"XAUUSDT{day}.csv.gz") as f:
        t = pd.read_csv(f, usecols=["timestamp", "side", "size", "price"])
    t = t.sort_values("timestamp", kind="stable").reset_index(drop=True)
    return pd.DataFrame({
        "t": t.timestamp.to_numpy() * 1000.0,
        "p": np.rint(t.price.to_numpy() * 100).astype(np.int64),
        "q": t["size"].to_numpy(),
        # aggressor: +1 buy (hits asks), -1 sell (hits bids)
        "s": np.where(t.side.to_numpy() == "Buy", 1, -1).astype(np.int8),
    })


def extract(day: str, out: pathlib.Path) -> pathlib.Path:
    tr = _trades(day)
    tt, tp, tq, ts = tr.t.to_numpy(), tr.p.to_numpy(), tr.q.to_numpy(), tr.s.to_numpy()
    nt, ti = len(tt), 0
    book = ({}, {})  # 0 bids, 1 asks: price tick -> size
    best = [0, 1 << 40]
    pending: dict[tuple[int, int], tuple[float, float]] = {}  # (last trade time, volume)
    active: dict[tuple[int, int], Ep] = {}
    dormant: dict[tuple[int, int], Ep] = {}
    done: list[Ep] = []
    ref = [np.nan, np.nan]
    next_ref = 0.0
    sec_t, sec_b, sec_a, sec_rb, sec_ra = [], [], [], [], []

    def finish(e: Ep, t: float, reason: int) -> None:
        e.t_end, e.reason = t, reason
        done.append(e)

    def trade(i: int) -> None:
        # sell aggressor hits bids (side 0), buy aggressor hits asks (side 1)
        side = 0 if ts[i] < 0 else 1
        p, t, q = int(tp[i]), tt[i], float(tq[i])
        key = (side, p)
        e = active.get(key) or dormant.get(key)
        if e is not None:
            lt, lv = e.late
            if lv > 0 and t - lt <= LATE_MS:
                m = min(q, lv)
                e.cancel -= m
                e.traded += m
                e.late = (lt, lv - m)
                q -= m
                e.hist.append((t, e.size, e.traded, e.cancel))
            if key in active:
                e.touch(t, e.size)
                e.hit_since_add = True
            elif t - e.t_low <= LATE_MS:
                # the book showed the drop before the tape: price was on a live density
                e.touch(e.t_low, e.size_before_low)
        if q > 0:
            pt, pv = pending.get(key, (t, 0.0))
            pending[key] = (t, (pv if t - pt <= PENDING_MS else 0.0) + q)
        for k, e in list(active.items()) + list(dormant.items()):
            if e.side != side or p == e.p:
                continue
            if (p < e.p) if side == 0 else (p > e.p):
                if k in active:
                    e.touch(t, e.size)
                    active.pop(k)
                    finish(e, t, THROUGH)
                else:
                    dormant.pop(k)
                    finish(e, e.t_low, EATEN if _drop_traded(e) else PULLED)

    def set_level(side: int, p: int, size: float, t: float) -> None:
        lv = book[side]
        old = lv.get(p, 0.0)
        if size == 0.0:
            lv.pop(p, None)
        else:
            lv[p] = size
        # best price
        if side == 0:
            if size > 0 and p > best[0]:
                best[0] = p
            elif size == 0 and p == best[0]:
                best[0] = max(lv) if lv else 0
        else:
            if size > 0 and p < best[1]:
                best[1] = p
            elif size == 0 and p == best[1]:
                best[1] = min(lv) if lv else 1 << 40
        key = (side, p)
        pt, pend = pending.pop(key, (t, 0.0))
        if t - pt > PENDING_MS:
            pend = 0.0
        dec = max(old - size, 0.0)
        traded = min(dec, pend)
        cancel = dec - traded
        if pend > traded:
            pending[key] = (pt, pend - traded)
        add = max(size - old, 0.0)
        e = active.get(key) or dormant.get(key)
        if e is not None:
            e.n_changes += 1
            e.traded += traded
            e.cancel += cancel
            e.added += add
            if cancel > 0:
                e.late = (t, cancel)
            if add > 0 and e.hit_since_add:
                e.refill += add
                e.n_refill += 1
                e.hit_since_add = False
            before = e.size
            e.size = size
            e.peak = max(e.peak, size)
            e.hist.append((t, size, e.traded, e.cancel))
            if e.p == best[side] and np.isnan(e.t_top):
                e.t_top = t
            r = ref[side]
            if key in active and size < K_END * r:
                active.pop(key)
                dormant[key] = e
                e.t_low, e.size_before_low = t, before
            elif key in dormant and size >= K_START * r:
                dormant.pop(key)
                active[key] = e
        elif not np.isnan(ref[side]) and size >= K_START * ref[side]:
            dist = (best[0] - p) if side == 0 else (p - best[1])
            if 0 <= dist <= MAX_DIST:
                active[key] = Ep(side, p, t, size, dist)

    def expire(t: float) -> None:
        for k, e in list(dormant.items()):
            if t - e.t_low >= DORMANT_S * 1000:
                dormant.pop(k)
                dist = (best[0] - e.p) if e.side == 0 else (e.p - best[1])
                if dist > MAX_DIST + 150:
                    reason = OUT
                else:
                    # the drop into dormancy: traded vs cancelled since the peak
                    reason = EATEN if _drop_traded(e) else PULLED
                finish(e, e.t_low, reason)

    with zipfile.ZipFile(DATA / "ob200" / f"{day}_XAUUSDT_ob200.data.zip") as z:
        name = z.namelist()[0]
        with z.open(name) as raw:
            for line in io.BufferedReader(raw, 1 << 20):
                d = orjson.loads(line)
                t = float(d["ts"])
                while ti < nt and tt[ti] <= t:
                    trade(ti)
                    ti += 1
                dd = d["data"]
                if d["type"] == "snapshot":
                    book[0].clear(), book[1].clear()
                    best[0], best[1] = 0, 1 << 40
                for side, arr in ((0, dd["b"]), (1, dd["a"])):
                    for ps, ss in arr:
                        set_level(side, int(round(float(ps) * 100)), float(ss), t)
                if t >= next_ref:
                    for s in (0, 1):
                        if book[s]:
                            ref[s] = float(np.median(np.fromiter(book[s].values(), float)))
                    expire(t)
                    sec_t.append(t), sec_b.append(best[0]), sec_a.append(best[1])
                    sec_rb.append(ref[0]), sec_ra.append(ref[1])
                    next_ref = t + 1000.0
    t_last = float(sec_t[-1]) if sec_t else 0.0
    for e in list(active.values()) + list(dormant.values()):
        finish(e, t_last, EOD)

    cols = {k: [] for k in Ep.__slots__ if k not in ("hist", "hit_since_add", "late")}
    h_id, h_t, h_s, h_tr, h_cn = [], [], [], [], []
    for i, e in enumerate(done):
        for k in cols:
            cols[k].append(getattr(e, k))
        for t, s, a, c in e.hist:
            h_id.append(i), h_t.append(t), h_s.append(s), h_tr.append(a), h_cn.append(c)
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"{day}.npz"
    np.savez_compressed(
        path,
        **{f"ep_{k}": np.asarray(v, dtype=float) for k, v in cols.items()},
        h_id=np.asarray(h_id, np.int32), h_t=np.asarray(h_t), h_size=np.asarray(h_s),
        h_traded=np.asarray(h_tr), h_cancel=np.asarray(h_cn),
        tr_t=tt, tr_p=tp.astype(np.int32), tr_q=tq, tr_s=ts,
        s_t=np.asarray(sec_t), s_bid=np.asarray(sec_b, np.int64), s_ask=np.asarray(sec_a, np.int64),
        s_ref_b=np.asarray(sec_rb), s_ref_a=np.asarray(sec_ra),
    )
    return path


def _drop_traded(e: Ep) -> bool:
    """Was the fall from the last peak mostly trades (eaten) rather than cancels (pulled)?"""
    hs = e.hist
    k = max(range(len(hs)), key=lambda i: hs[i][1])
    tr = hs[-1][2] - hs[k][2]
    cn = hs[-1][3] - hs[k][3]
    return tr > cn


def _one(day: str) -> str:
    out = DATA / "density"
    if (out / f"{day}.npz").exists():
        return day + " cached"
    extract(day, out)
    return day


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["extract"])
    ap.add_argument("--day", nargs="*")
    ap.add_argument("--workers", type=int, default=4)
    a = ap.parse_args()
    days = a.day or sorted(
        p.name[:10] for p in (DATA / "ob200").glob("*.zip")
        if (DATA / "trades" / f"XAUUSDT{p.name[:10]}.csv.gz").exists()
    )
    with cf.ProcessPoolExecutor(a.workers) as ex:
        for r in ex.map(_one, days):
            print(r, flush=True)


if __name__ == "__main__":
    main()
