"""Build crates/aegis-core/data/calendar.csv: high-impact US macro events for gold.

    python -m aegis_lab.data.calendar --from 2023-01-01 --to 2026-12-31

Sources (only dates parsed from these pages are emitted; nothing is inferred):
  FOMC, FOMC_MINUTES  https://www.federalreserve.gov/monetarypolicy/fomccalendars.htm
                      statement 2:00 pm ET on the last meeting day; minutes 2:00 pm ET on the
                      "(Released <date>)" date. Future minutes dates are not published there.
  CPI, NFP, PPI       bls.gov answers 403 to scripts, so Wayback Machine copies are used:
                      past releases: https://www.bls.gov/bls/news-release/{cpi,empsit,ppi}.htm
                      (archive links are named <release>_MMDDYYYY.htm = actual release date);
                      upcoming: https://www.bls.gov/schedule/news_release/{cpi,empsit,ppi}.htm
                      (table with release date + time). Times 8:30 am ET.
  GDP (advance/initial estimate), PCE (Personal Income and Outlays)
                      https://www.bea.gov/news/schedule/full (current year),
                      https://www.bea.gov/news/schedule/full-2025, and Wayback copies of
                      /news/schedule/full captured in late 2023 and late 2024. Time as listed.
Standard library only.
"""

from __future__ import annotations

import argparse
import collections
import csv
import datetime as dt
import gzip
import html
import pathlib
import re
import sys
import time
import urllib.request
from zoneinfo import ZoneInfo

ET = ZoneInfo("America/New_York")
UA = {
    "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/128.0 Safari/537.36",
    "Accept": "text/html,application/xhtml+xml,*/*;q=0.8",
    "Accept-Language": "en-US,en;q=0.9",
}
WB = "https://web.archive.org/web/{ts}id_/{url}"
FED = "https://www.federalreserve.gov/monetarypolicy/fomccalendars.htm"
BLS_ARCHIVE = "https://www.bls.gov/bls/news-release/{r}.htm"
BLS_SCHEDULE = "https://www.bls.gov/schedule/news_release/{r}.htm"
BLS_EVENTS = {"cpi": "CPI", "empsit": "NFP", "ppi": "PPI"}
BEA_PAGES = [  # (url, via wayback timestamp or None)
    ("https://www.bea.gov/news/schedule/full", "20231204"),
    ("https://www.bea.gov/news/schedule/full", "20241215"),
    ("https://www.bea.gov/news/schedule/full-2025", None),
    ("https://www.bea.gov/news/schedule/full", None),
]
IMPORTANCE = {"FOMC": 3, "CPI": 3, "NFP": 3}
MONTHS = {m: i for i, m in enumerate(
    ["january", "february", "march", "april", "may", "june", "july", "august",
     "september", "october", "november", "december"], 1)}


def get(url: str, tries: int = 4) -> str:
    for i in range(tries):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=90) as r:
                b = r.read()
            if b[:2] == b"\x1f\x8b":  # wayback id_ returns the original Content-Encoding
                b = gzip.decompress(b)
            return b.decode("utf-8", "replace")
        except Exception as e:
            print(f"  retry {i + 1} {url}: {e!r}", file=sys.stderr)
            time.sleep(5 * (i + 1))
    raise RuntimeError(f"cannot fetch {url}")


def wayback(url: str, near: str | None = None, min_len: int = 5000, with_ts: bool = False):
    """Latest (or latest before `near`) 200 capture larger than min_len; blank captures exist."""
    q = f"https://web.archive.org/cdx/search/cdx?url={url}&filter=statuscode:200&fl=timestamp,length&from=2023"
    if near:
        q += f"&to={near}"
    caps = [ln.split() for ln in get(q).splitlines() if ln.strip()]
    good = [ts for ts, ln in caps if ln.isdigit() and int(ln) >= min_len]
    if not good:
        raise RuntimeError(f"no usable wayback capture of {url}")
    body = get(WB.format(ts=good[-1], url=url))
    return (body, dt.datetime.strptime(good[-1][:8], "%Y%m%d").date()) if with_ts else body


def text(s: str) -> str:
    return re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", " ", s))).strip()


def month(name: str) -> int:
    k = name.strip(". ").lower()[:3]
    return next(i for m, i in MONTHS.items() if m.startswith(k))


def et(d: dt.date, hh: int, mm: int) -> int:
    return int(dt.datetime(d.year, d.month, d.day, hh, mm, tzinfo=ET).timestamp())


def clock(s: str) -> tuple[int, int]:
    m = re.match(r"(\d{1,2}):(\d\d)\s*([AP])\.?M", s.strip().upper())
    h = int(m.group(1)) % 12 + (12 if m.group(3) == "P" else 0)
    return h, int(m.group(2))


def fomc() -> list[tuple[int, str, str]]:
    s = get(FED)
    out = []
    heads = list(re.finditer(r"(\d{4}) FOMC Meetings", s))
    for n, h in enumerate(heads):
        year = int(h.group(1))
        panel = s[h.end(): heads[n + 1].start() if n + 1 < len(heads) else len(s)]
        for blk in re.split(r'<div class="[^"]*\brow fomc-meeting\b', panel)[1:]:
            mon = re.search(r"fomc-meeting__month[^>]*>\s*<strong>([^<]+)</strong>", blk)
            day = re.search(r"fomc-meeting__date[^>]*>([^<]+)</div>", blk)
            if not (mon and day):
                continue
            raw_day = text(day.group(1))
            if re.search(r"notation|unscheduled", raw_day + text(blk[:400]), re.I):
                continue  # only scheduled meetings with a 2 pm statement
            last_mon = mon.group(1).split("/")[-1]
            last_day = int(re.findall(r"\d+", raw_day)[-1])
            d = dt.date(year + (1 if mon.group(1).startswith("Dec/") else 0), month(last_mon), last_day)
            out.append((et(d, 14, 0), "FOMC", "federalreserve.gov"))
            rel = re.search(r"Released ([A-Za-z]+) (\d{1,2}), (\d{4})", blk)
            if rel:
                md = dt.date(int(rel.group(3)), month(rel.group(1)), int(rel.group(2)))
                out.append((et(md, 14, 0), "FOMC_MINUTES", "federalreserve.gov"))
    return out


def bls() -> list[tuple[int, str, str]]:
    out = []
    for r, ev in BLS_EVENTS.items():
        src = "bls.gov via web.archive.org"
        arc = wayback(BLS_ARCHIVE.format(r=r))
        dates = {dt.date(int(y), int(m), int(d))
                 for m, d, y in re.findall(rf"/news\.release/archives/{r}_(\d\d)(\d\d)(\d{{4}})\.htm", arc)}
        rows = {d: (8, 30) for d in dates}
        sch = text(wayback(BLS_SCHEDULE.format(r=r)))
        for mon, day, year, tm in re.findall(
                r"[A-Z][a-z]+ \d{4} ([A-Z][a-z]{2,8})\.? (\d{1,2}), (\d{4}) (\d\d:\d\d [AP]M)", sch):
            rows[dt.date(int(year), month(mon), int(day))] = clock(tm)
        if not dates or not sch:
            raise RuntimeError(f"BLS {r}: empty source")
        out += [(et(d, *hm), ev, src) for d, hm in rows.items()]
    return out


def bea() -> list[tuple[int, str, str]]:
    out = []
    today = dt.date.today()
    for url, ts in BEA_PAGES:
        # rows dated before `asof` must carry a link to the published release
        s, asof = wayback(url, near=ts, with_ts=True) if ts else (get(url), today)
        src = "bea.gov via web.archive.org" if ts else "bea.gov"
        years = collections.Counter(re.findall(r'href="/news/(\d{4})/', s))
        page_year = int(years.most_common(1)[0][0])
        print(f"  BEA {url} @ {ts or 'live'}: page year {page_year}", file=sys.stderr)
        for tr in re.findall(r"<tr[^>]*>(.*?)</tr>", s, re.S):
            # 2023 layout: <td class="scheduled-date">January 5</td>; later: <div class="release-date">
            d = re.search(r'(?:scheduled-date[^>]*>|release-date">)\s*([A-Za-z]+) (\d{1,2})\s*<', tr)
            tm = re.search(r"(\d{1,2}:\d\d [AP]M)", tr, re.I)
            title = re.search(r'release-title[^>]*>(.*?)</td>', tr, re.S)
            if not (d and tm and title):
                continue
            title = text(title.group(1))
            if title.startswith("Personal Income and Outlays"):
                ev = "PCE"
            elif re.search(r"(GDP|Gross Domestic Product)\b.*\((Advance|Initial) Estimate\)", title):
                ev = "GDP"
            else:
                continue
            link = re.search(r'href="/news/(\d{4})/', tr)
            year = int(link.group(1)) if link else page_year
            day = dt.date(year, month(d.group(1)), int(d.group(2)))
            if day < asof and not link:
                continue  # past row never published (e.g. GDP Q3-2025 advance, cancelled in the shutdown)
            out.append((et(day, *clock(tm.group(1))), ev, src))
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--from", dest="start", default="2023-01-01")
    ap.add_argument("--to", dest="end", default="2026-12-31")
    ap.add_argument("--out", default=str(pathlib.Path(__file__).resolve().parents[3]
                                         / "crates/aegis-core/data/calendar.csv"))
    a = ap.parse_args(argv)
    lo = int(dt.datetime.fromisoformat(a.start).replace(tzinfo=dt.timezone.utc).timestamp())
    hi = int((dt.datetime.fromisoformat(a.end) + dt.timedelta(days=1)).replace(tzinfo=dt.timezone.utc).timestamp())
    rows: dict[tuple[str, dt.date], tuple[int, str, str]] = {}
    for t, ev, src in fomc() + bls() + bea():
        if lo <= t < hi:
            # one event per type per ET day; later sources (current pages) override older copies
            rows[(ev, dt.datetime.fromtimestamp(t, ET).date())] = (t, ev, src)
    out = pathlib.Path(a.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["time", "event", "importance", "source"])
        for t, ev, src in sorted(rows.values()):
            w.writerow([t, ev, IMPORTANCE.get(ev, 2), src])
    cnt = collections.Counter((ev, dt.datetime.fromtimestamp(t, ET).year) for t, ev, _ in rows.values())
    for ev in sorted({e for e, _ in cnt}):
        print(ev, {y: c for (e, y), c in sorted(cnt.items()) if e == ev})
    print(f"wrote {len(rows)} events to {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
