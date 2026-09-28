"""Download Dukascopy XAUUSD 1-minute BID/ASK candles and aggregate them to 5m bars.

    python -m aegis_lab.data.dukascopy --from 2023-09-01 --to 2026-09-26 --out ~/aegis-data/dukascopy

Day files: https://datafeed.dukascopy.com/datafeed/XAUUSD/YYYY/MM/DD/{BID,ASK}_candles_min_1.bi5
(MM is 0-based). Payload is LZMA; records are 24 bytes big-endian:
uint32 seconds-from-day-start, uint32 open, close, low, high (price * 1000), float32 volume.
Minutes without ticks are emitted as flat candles with volume 0 and are dropped here.

The server answers 429 to requests without a browser User-Agent, so requests are sent
sequentially with browser headers, a delay and exponential backoff. Concurrent requests
get an immediate 503 ("No server is available"), so do not parallelise. Saturdays are skipped
(XAUUSD is closed all Saturday UTC; the files contain only zero-volume filler). Raw files are cached
under <out>/raw (empty marker files for weekends/holidays) so re-runs resume.
Standard library only.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import lzma
import pathlib
import struct
import sys
import time
import urllib.error
import urllib.request

BASE = "https://datafeed.dukascopy.com/datafeed"
HEADERS = {
    "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/128.0 Safari/537.36",
    "Referer": "https://www.dukascopy.com/swiss/english/marketwatch/historical/",
    "Accept": "*/*",
}
DIVISOR = 1000.0  # XAUUSD point value
REC = struct.Struct(">IIIIIf")


def _url(symbol: str, day: dt.date, side: str) -> str:
    return f"{BASE}/{symbol}/{day.year:04d}/{day.month - 1:02d}/{day.day:02d}/{side}_candles_min_1.bi5"


def _raw_path(out: pathlib.Path, symbol: str, day: dt.date, side: str) -> pathlib.Path:
    return out / "raw" / symbol / f"{day.year:04d}" / f"{day.month:02d}" / f"{day.day:02d}_{side}.bi5"


def fetch(url: str, delay: float, max_tries: int = 6) -> bytes | None:
    """Body (b"" for 404), or None if the server kept failing (retried in a later pass)."""
    wait = max(delay, 3.0)
    for i in range(max_tries):
        try:
            req = urllib.request.Request(url, headers=HEADERS)
            # server often takes 15-35 s to first byte
            with urllib.request.urlopen(req, timeout=150) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return b""
            err = f"HTTP {e.code}"
            # 429 = rate limited: back off hard. 503 "No server is available" is transient.
            wait = min(wait * (4 if e.code == 429 else 2), 600.0)
        except Exception as e:  # network errors: retry
            err = repr(e)
            wait = min(wait * 2, 120.0)
        print(f"  retry {i + 1} {url}: {err}; sleeping {wait:.0f}s", flush=True)
        time.sleep(wait)
    return None


def decode(blob: bytes, day: dt.date) -> list[tuple[int, float, float, float, float, float]]:
    """-> [(unix_ts, open, high, low, close, volume)] for minutes with volume > 0."""
    if not blob:
        return []
    data = lzma.decompress(blob)
    t0 = int(dt.datetime(day.year, day.month, day.day, tzinfo=dt.timezone.utc).timestamp())
    rows = []
    for sec, o, c, lo, hi, vol in REC.iter_unpack(data):
        if vol <= 0:
            continue
        rows.append((t0 + sec, o / DIVISOR, hi / DIVISOR, lo / DIVISOR, c / DIVISOR, float(vol)))
    return rows


def download(symbol: str, start: dt.date, end: dt.date, out: pathlib.Path, delay: float, passes: int = 5) -> int:
    """Fetch missing day files; returns the number still missing after all passes."""
    todo = []
    day = start
    while day <= end:
        if day.weekday() != 5:
            todo += [(day, side) for side in ("BID", "ASK") if not _raw_path(out, symbol, day, side).exists()]
        day += dt.timedelta(days=1)
    print(f"{len(todo)} files to fetch", flush=True)
    for pas in range(passes):
        failed = []
        n_new, t_start = 0, time.time()
        for idx, (day, side) in enumerate(todo):
            blob = fetch(_url(symbol, day, side), delay)
            if blob is None:
                failed.append((day, side))
            else:
                p = _raw_path(out, symbol, day, side)
                p.parent.mkdir(parents=True, exist_ok=True)
                tmp = p.with_suffix(".part")
                tmp.write_bytes(blob)
                tmp.replace(p)
                n_new += 1
            if idx % 50 == 49 or idx == len(todo) - 1:
                el = time.time() - t_start
                eta = el / (idx + 1) * (len(todo) - idx - 1)
                print(f"pass {pas + 1}: {idx + 1}/{len(todo)} ({day} {side}) ok={n_new} failed={len(failed)} "
                      f"{(idx + 1) / el:.2f} files/s ETA {eta / 60:.0f} min", flush=True)
            time.sleep(delay)
        todo = failed
        if not todo:
            break
        print(f"pass {pas + 1} left {len(todo)} failed files; retrying after 120s", flush=True)
        time.sleep(120)
    return len(todo)


def aggregate(symbol: str, start: dt.date, end: dt.date, out: pathlib.Path, csv_path: pathlib.Path) -> int:
    n = 0
    tmp = csv_path.with_suffix(".csv.part")
    with tmp.open("w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["time", "open", "high", "low", "close", "volume", "spread"])
        day = start
        while day <= end:
            bp, ap = _raw_path(out, symbol, day, "BID"), _raw_path(out, symbol, day, "ASK")
            if not bp.exists():
                day += dt.timedelta(days=1)
                continue
            bid = decode(bp.read_bytes(), day)
            ask = {r[0]: r[4] for r in decode(ap.read_bytes(), day)} if ap.exists() else {}
            bars: dict[int, list] = {}
            for ts, o, hi, lo, c, vol in bid:
                if not (lo <= min(o, c) and hi >= max(o, c) and lo > 0):
                    continue
                k = ts - ts % 300
                b = bars.get(k)
                if b is None:
                    b = bars[k] = [o, hi, lo, c, 0.0, 0.0, 0]
                b[1] = max(b[1], hi)
                b[2] = min(b[2], lo)
                b[3] = c
                b[4] += vol
                if ts in ask:
                    b[5] += ask[ts] - c
                    b[6] += 1
            for k in sorted(bars):
                o, hi, lo, c, vol, ssum, sn = bars[k]
                spread = f"{ssum / sn:.3f}" if sn else ""
                w.writerow([k, f"{o:.3f}", f"{hi:.3f}", f"{lo:.3f}", f"{c:.3f}", f"{vol:.4f}", spread])
                n += 1
            day += dt.timedelta(days=1)
    tmp.replace(csv_path)
    return n


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--symbol", default="XAUUSD")
    ap.add_argument("--from", dest="start", default="2023-09-01")
    ap.add_argument("--to", dest="end", default=None, help="inclusive; default yesterday UTC")
    ap.add_argument("--out", default="~/aegis-data/dukascopy")
    ap.add_argument("--csv", default="~/aegis-data/xau_duka_5m.csv")
    ap.add_argument("--delay", type=float, default=1.5, help="seconds between requests")
    ap.add_argument("--passes", type=int, default=8, help="retry passes over failed files")
    ap.add_argument("--no-download", action="store_true", help="only aggregate cached files")
    a = ap.parse_args(argv)
    start = dt.date.fromisoformat(a.start)
    end = dt.date.fromisoformat(a.end) if a.end else dt.datetime.now(dt.timezone.utc).date() - dt.timedelta(days=1)
    out = pathlib.Path(a.out).expanduser()
    csv_path = pathlib.Path(a.csv).expanduser()
    if not a.no_download:
        missing = download(a.symbol, start, end, out, a.delay, a.passes)
        if missing:
            print(f"WARNING: {missing} files still missing; re-run to resume", flush=True)
    n = aggregate(a.symbol, start, end, out, csv_path)
    print(f"wrote {n} 5m bars to {csv_path}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
