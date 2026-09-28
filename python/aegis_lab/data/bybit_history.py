"""Download Bybit public history for a linear perpetual: order book (200 levels, 100 ms deltas)
and trades with the aggressor side.

    python -m aegis_lab.data.bybit_history --symbol XAUUSDT --start 2026-03-09

Order book: quote-saver.bycsi.com/orderbook/linear/<SYM>/<day>_<SYM>_ob200.data.zip (~58 MB/day),
one JSON line per update: a snapshot at 00:00 UTC, then deltas (size "0" removes the price).
Trades: public.bybit.com/trading/<SYM>/<SYM><day>.csv.gz (~10 MB/day).
Files are cached; re-running only fetches what is missing.
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import datetime as dt
import pathlib
import time
import urllib.error
import urllib.request

OB = "https://quote-saver.bycsi.com/orderbook/linear/{s}/{d}_{s}_ob200.data.zip"
TRADES = "https://public.bybit.com/trading/{s}/{s}{d}.csv.gz"


def _get(url: str, tries: int = 5) -> bytes | None:
    for i in range(tries):
        try:
            with urllib.request.urlopen(url, timeout=300) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            if i == tries - 1:
                raise
        except Exception:
            if i == tries - 1:
                raise
        time.sleep(2**i)
    raise AssertionError


def paths(out: pathlib.Path, symbol: str, day: str) -> tuple[pathlib.Path, pathlib.Path]:
    return (
        out / symbol / "ob200" / f"{day}_{symbol}_ob200.data.zip",
        out / symbol / "trades" / f"{symbol}{day}.csv.gz",
    )


def days(start: str, end: str) -> list[str]:
    a, b = dt.date.fromisoformat(start), dt.date.fromisoformat(end)
    return [(a + dt.timedelta(n)).isoformat() for n in range((b - a).days + 1)]


def download(symbol: str, start: str, end: str, out: pathlib.Path, workers: int = 4) -> list[str]:
    jobs = []
    for d in days(start, end):
        ob, tr = paths(out, symbol, d)
        jobs += [(OB.format(s=symbol, d=d), ob), (TRADES.format(s=symbol, d=d), tr)]

    def fetch(job: tuple[str, pathlib.Path]) -> str | None:
        url, path = job
        if path.exists():
            return None
        path.parent.mkdir(parents=True, exist_ok=True)
        body = _get(url)
        if body is None:
            return path.name
        tmp = path.with_suffix(".part")
        tmp.write_bytes(body)
        tmp.rename(path)
        return None

    with cf.ThreadPoolExecutor(workers) as ex:
        return [m for m in ex.map(fetch, jobs) if m]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--symbol", default="XAUUSDT")
    ap.add_argument("--start", default="2026-03-09")
    ap.add_argument("--end", default=(dt.date.today() - dt.timedelta(1)).isoformat())
    ap.add_argument("--out", type=pathlib.Path, default=pathlib.Path.home() / "aegis-data" / "bybit")
    a = ap.parse_args()
    t = time.time()
    missing = download(a.symbol, a.start, a.end, a.out)
    print(f"done in {time.time() - t:.0f}s; not published: {missing or 'none'}", flush=True)


if __name__ == "__main__":
    main()
