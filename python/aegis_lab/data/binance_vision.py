"""Download Binance public history (data.binance.vision) for USDⓈ-M futures.

Monthly archives are used where they exist, daily archives for the current month.
Files are cached; re-running only fetches what is missing.

    python -m aegis_lab.data.binance_vision --symbol XAUUSDT --kinds klines-5m klines-1m aggTrades metrics
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import datetime as dt
import pathlib
import re
import time
import urllib.request

BASE = "https://data.binance.vision"
LIST = "https://s3-ap-northeast-1.amazonaws.com/data.binance.vision?prefix={prefix}&delimiter=/"


def _list(prefix: str) -> list[str]:
    keys: list[str] = []
    marker = ""
    while True:
        url = LIST.format(prefix=prefix) + (f"&marker={marker}" if marker else "")
        body = _get(url).decode()
        page = re.findall(r"<Key>([^<]+\.zip)</Key>", body)
        keys += page
        if "<IsTruncated>true</IsTruncated>" not in body or not page:
            return keys
        marker = page[-1]


def _get(url: str, tries: int = 5) -> bytes:
    for i in range(tries):
        try:
            with urllib.request.urlopen(url, timeout=120) as r:
                return r.read()
        except Exception:
            if i == tries - 1:
                raise
            time.sleep(2**i)
    raise AssertionError


def _prefixes(kind: str, symbol: str, period: str) -> str:
    if kind.startswith("klines-"):
        tf = kind.split("-", 1)[1]
        return f"data/futures/um/{period}/klines/{symbol}/{tf}/"
    return f"data/futures/um/{period}/{kind}/{symbol}/"


def download(symbol: str, kind: str, out: pathlib.Path, workers: int = 8) -> list[pathlib.Path]:
    dest = out / symbol / kind
    dest.mkdir(parents=True, exist_ok=True)
    monthly = [] if kind == "metrics" else _list(_prefixes(kind, symbol, "monthly"))
    months = {re.search(r"(\d{4}-\d{2})\.zip$", k).group(1) for k in monthly}
    daily = [
        k
        for k in _list(_prefixes(kind, symbol, "daily"))
        if re.search(r"(\d{4}-\d{2})-\d{2}\.zip$", k).group(1) not in months
    ]
    keys = monthly + daily

    def fetch(key: str) -> pathlib.Path:
        path = dest / key.rsplit("/", 1)[1]
        if not path.exists():
            tmp = path.with_suffix(".part")
            tmp.write_bytes(_get(f"{BASE}/{key}"))
            tmp.rename(path)
        return path

    with cf.ThreadPoolExecutor(workers) as ex:
        return sorted(ex.map(fetch, keys))


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--symbol", default="XAUUSDT")
    ap.add_argument("--kinds", nargs="+", default=["klines-5m", "klines-1m", "aggTrades", "metrics"])
    ap.add_argument("--out", type=pathlib.Path, default=pathlib.Path.home() / "aegis-data" / "binance")
    a = ap.parse_args()
    for kind in a.kinds:
        t = dt.datetime.now()
        files = download(a.symbol, kind, a.out)
        print(f"{kind}: {len(files)} files in {(dt.datetime.now() - t).seconds}s", flush=True)


if __name__ == "__main__":
    main()
