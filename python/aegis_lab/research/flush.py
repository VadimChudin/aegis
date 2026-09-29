"""Flushes and buybacks ("пролив → откуп") on Bybit XAUUSDT: what the book and the tape do when
gold drops fast, and what separates a drop that is bought back from one that keeps going.

    python -m aegis_lab.research.flush extract      # one .npz of 1-second book + tape per day
    python -m aegis_lab.research.flush report       # events, outcomes, metrics by quintile

`extract` replays the 200-level book (100 ms deltas) with the tape and writes, per second: best
bid / ask, resting size within $0.5 / $1 / $2 / $5 of the mid on each side, the largest level
within $5 and its distance, aggressive buy / sell volume and trade counts, the largest trade, and
book flows within $2 of the mid: size added, size eaten (hit by trades) and size pulled
(cancelled) on each side. Per second, eaten = the aggressive volume into that side (capped by the
decrease); the book trails the tape by ~40 ms, so both almost always fall in the same second.
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
OUT = DATA / "flush"
BANDS = (50, 100, 200, 500)  # ticks of $0.01
FLOW_BAND = 200
COLS = (
    ["bid", "ask"]
    + [f"{s}{b}" for s in ("bd", "ad") for b in BANDS]
    + ["bmax", "bmax_d", "amax", "amax_d"]
    + ["buy_v", "sell_v", "buy_n", "sell_n", "max_buy", "max_sell"]
    + ["b_add", "b_eat", "b_pull", "a_add", "a_eat", "a_pull"]
)


def _trades(day: str) -> tuple[np.ndarray, ...]:
    with gzip.open(DATA / "trades" / f"XAUUSDT{day}.csv.gz") as f:
        t = pd.read_csv(f, usecols=["timestamp", "side", "size", "price"])
    t = t.sort_values("timestamp", kind="stable")
    return (
        t.timestamp.to_numpy() * 1000.0,
        np.rint(t.price.to_numpy() * 100).astype(np.int64),
        t["size"].to_numpy(dtype=float),
        np.where(t.side.to_numpy() == "Buy", 1, -1).astype(np.int8),
    )


def extract(day: str) -> pathlib.Path:
    tt, _, tq, ts = _trades(day)
    t0 = pd.Timestamp(day, tz="UTC").value // 1_000_000
    n = 86400
    out = {c: np.zeros(n, np.float32) for c in COLS}
    book: tuple[dict[int, float], dict[int, float]] = ({}, {})
    best = [0, 1 << 40]
    # per-second decreases and additions within FLOW_BAND of the mid
    dec = [0.0, 0.0]
    add = [0.0, 0.0]
    ti, nt = 0, len(tt)
    sec = -1

    def close_second(s: int) -> None:
        if not (0 <= s < n) or not book[0] or not book[1]:
            return
        b, a = best
        mid2 = b + a  # twice the mid, in ticks
        out["bid"][s], out["ask"][s] = b / 100, a / 100
        for side, pre in ((0, "bd"), (1, "ad")):
            depth = [0.0] * len(BANDS)
            big, big_d = 0.0, 0
            for p, q in book[side].items():
                d2 = (mid2 - 2 * p) if side == 0 else (2 * p - mid2)
                for k, bd in enumerate(BANDS):
                    if d2 <= 2 * bd:
                        depth[k] += q
                if d2 <= 2 * BANDS[-1] and q > big:
                    big, big_d = q, d2 / 2
            for k, bd in enumerate(BANDS):
                out[f"{pre}{bd}"][s] = depth[k]
            key = "bmax" if side == 0 else "amax"
            out[key][s], out[key + "_d"][s] = big, big_d / 100
        # sells hit bids, buys hit asks
        eat_b = min(dec[0], out["sell_v"][s])
        eat_a = min(dec[1], out["buy_v"][s])
        out["b_eat"][s], out["b_pull"][s], out["b_add"][s] = eat_b, dec[0] - eat_b, add[0]
        out["a_eat"][s], out["a_pull"][s], out["a_add"][s] = eat_a, dec[1] - eat_a, add[1]
        dec[0] = dec[1] = add[0] = add[1] = 0.0

    def trade(i: int) -> None:
        s = int((tt[i] - t0) // 1000)
        if not 0 <= s < n:
            return
        q = float(tq[i])
        if ts[i] > 0:
            out["buy_v"][s] += q
            out["buy_n"][s] += 1
            out["max_buy"][s] = max(out["max_buy"][s], q)
        else:
            out["sell_v"][s] += q
            out["sell_n"][s] += 1
            out["max_sell"][s] = max(out["max_sell"][s], q)

    def set_level(side: int, p: int, size: float, snapshot: bool) -> None:
        lv = book[side]
        old = lv.get(p, 0.0)
        if size == 0.0:
            lv.pop(p, None)
        else:
            lv[p] = size
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
        if snapshot or not book[0] or not book[1]:
            return
        d2 = (best[0] + best[1] - 2 * p) if side == 0 else (2 * p - best[0] - best[1])
        if d2 <= 2 * FLOW_BAND:
            if size < old:
                dec[side] += old - size
            else:
                add[side] += size - old

    z = zipfile.ZipFile(DATA / "ob200" / f"{day}_XAUUSDT_ob200.data.zip")
    with z, z.open(z.namelist()[0]) as raw:
        for line in io.BufferedReader(raw, 1 << 20):
            d = orjson.loads(line)
            t = float(d["ts"])
            s = int((t - t0) // 1000)
            while ti < nt and tt[ti] <= t:
                trade(ti)
                ti += 1
            while sec < s:
                close_second(sec)
                sec += 1
            snap = d["type"] == "snapshot"
            if snap:
                book[0].clear(), book[1].clear()
                best[0], best[1] = 0, 1 << 40
            dd = d["data"]
            for side, arr in ((0, dd["b"]), (1, dd["a"])):
                for ps, ss in arr:
                    set_level(side, round(float(ps) * 100), float(ss), snap)
    while ti < nt:
        trade(ti)
        ti += 1
    close_second(sec)
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / f"{day}.npz"
    np.savez_compressed(path, t0=np.int64(t0), **out)
    return path


def _one(day: str) -> str:
    if (OUT / f"{day}.npz").exists():
        return day + " cached"
    extract(day)
    return day


def _days() -> list[str]:
    return sorted(
        p.name[:10] for p in (DATA / "ob200").glob("*.zip")
        if (DATA / "trades" / f"XAUUSDT{p.name[:10]}.csv.gz").exists()
    )


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["extract", "report"])
    ap.add_argument("--day", nargs="*")
    ap.add_argument("--workers", type=int, default=4)
    a = ap.parse_args()
    if a.cmd == "extract":
        with cf.ProcessPoolExecutor(a.workers) as ex:
            for r in ex.map(_one, a.day or _days()):
                print(r, flush=True)
    else:
        from aegis_lab.research import flush_report

        flush_report.main()


if __name__ == "__main__":
    main()
