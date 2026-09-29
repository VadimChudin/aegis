"""Time-series momentum on crypto spot, daily bars, volatility-targeted. No parameter fitting.

Rule (long-only, spot, one decision per day at the daily close):
  score  = mean over L in (20, 60, 120, 250) of sign(close / close L days ago - 1), clipped at 0
  weight = score x TARGET_VOL / realised 60-day vol, capped at MAX_W
  the basket is equal-risk over every coin with enough history; trade on the next day
Costs: COST per unit of turnover (taker + slippage). Spot only: a long perpetual would pay
funding, which was 10-30%/yr in the bull years.

    python -m aegis_lab.research.trend             # backtest, robustness table, today's weights
    python -m aegis_lab.research.trend --today     # today's weights only
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import datetime as dt
import io
import pathlib
import urllib.request
import zipfile

import numpy as np
import pandas as pd

COINS = ["BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "LTCUSDT", "ADAUSDT", "LINKUSDT", "TRXUSDT",
         "DOGEUSDT", "BCHUSDT", "SOLUSDT", "AVAXUSDT", "DOTUSDT", "ATOMUSDT"]
LOOKBACKS = (20, 60, 120, 250)
TARGET_VOL = 0.10
MAX_W = 2.0
COST = 0.001
YEAR = 365
BASE = "https://data.binance.vision/data/spot"


def _fetch(url: str) -> bytes | None:
    try:
        with urllib.request.urlopen(url, timeout=60) as r:
            z = zipfile.ZipFile(io.BytesIO(r.read()))
            return z.read(z.namelist()[0])
    except Exception:
        return None


def download(coins: list[str], out: pathlib.Path) -> None:
    """Monthly daily-kline archives, plus daily ones for the current month. Cached."""
    today = dt.date.today()
    jobs = []
    for c in coins:
        d = out / c
        d.mkdir(parents=True, exist_ok=True)
        for y in range(2017, today.year + 1):
            for m in range(1, 13):
                if (y, m) >= (today.year, today.month):
                    break
                jobs.append((f"{BASE}/monthly/klines/{c}/1d/{c}-1d-{y}-{m:02d}.zip", d / f"{y}-{m:02d}.csv", False))
        for day in range(1, today.day):
            ds = dt.date(today.year, today.month, day).isoformat()
            jobs.append((f"{BASE}/daily/klines/{c}/1d/{c}-1d-{ds}.zip", d / f"{ds}.csv", True))

    def run(job):
        url, path, refresh = job
        if path.exists() and not refresh:
            return
        body = _fetch(url)
        if body:
            path.write_bytes(body)
        elif not refresh:
            path.write_bytes(b"")  # listed before the coin existed: remember the miss

    with cf.ThreadPoolExecutor(16) as ex:
        list(ex.map(run, jobs))


def load(coin: str, out: pathlib.Path) -> pd.Series | None:
    parts = []
    for f in sorted((out / coin).glob("*.csv")):
        if f.stat().st_size == 0:
            continue
        a = np.loadtxt(f, delimiter=",", usecols=(0, 4), ndmin=2)
        t = a[:, 0].copy()
        t[t > 1e14] /= 1000  # archives switched to microseconds in 2025
        parts.append(pd.Series(a[:, 1], index=pd.to_datetime(t, unit="ms").normalize()))
    if not parts:
        return None
    s = pd.concat(parts).sort_index()
    return s[~s.index.duplicated(keep="last")]


def weights(px: pd.Series, long_only: bool = True, lookbacks=LOOKBACKS) -> pd.Series:
    """Target weight decided at each close (before the next-day lag)."""
    ret = px.pct_change()
    vol = ret.rolling(60).std() * np.sqrt(YEAR)
    score = sum(np.sign(px / px.shift(L) - 1) for L in lookbacks) / len(lookbacks)
    if long_only:
        score = score.clip(lower=0)
    return (score * TARGET_VOL / vol).clip(-MAX_W, MAX_W)


def returns(px: pd.Series, lag: int = 1, cost: float = COST, **kw) -> pd.Series:
    pos = weights(px, **kw).shift(lag)
    return pos * px.pct_change() - pos.diff().abs() * cost


def basket(prices: dict[str, pd.Series], **kw) -> pd.Series:
    """Equal risk: average of the coins' returns, scaled by sqrt(n) as if uncorrelated."""
    r = pd.DataFrame({c: returns(p, **kw) for c, p in prices.items()})
    return (r.mean(axis=1) * np.sqrt(r.notna().sum(axis=1).clip(lower=1))).loc["2018":]


def summary(r: pd.Series) -> dict:
    r = r.dropna()
    eq = (1 + r).cumprod()
    sharpe = lambda x: float(x.mean() / x.std() * np.sqrt(YEAR)) if x.std() > 0 else 0.0
    half = r.index[len(r) // 2]
    return {
        "cagr": float(eq.iloc[-1] ** (YEAR / len(r)) - 1),
        "sharpe": sharpe(r),
        "max_dd": float((eq / eq.cummax() - 1).min()),
        "sharpe_1st_half": sharpe(r[r.index < half]),
        "sharpe_2nd_half": sharpe(r[r.index >= half]),
    }


def _row(name: str, s: dict) -> str:
    return (f"| {name} | {s['cagr'] * 100:.1f}% | {s['sharpe']:.2f} | {s['max_dd'] * 100:.1f}% | "
            f"{s['sharpe_1st_half']:.2f} / {s['sharpe_2nd_half']:.2f} |")


def report(prices: dict[str, pd.Series]) -> None:
    print("| Variant | CAGR | Sharpe | Max DD | Sharpe halves |\n|---|---|---|---|---|")
    for L in LOOKBACKS:
        print(_row(f"long-only, {L}d only", summary(basket(prices, lookbacks=(L,)))))
    print(_row("**long-only ensemble (the rule)**", summary(basket(prices))))
    print(_row("long/short ensemble", summary(basket(prices, long_only=False))))
    print(_row("trade 2 days later", summary(basket(prices, lag=2))))
    print(_row("costs x4 (0.40%)", summary(basket(prices, cost=4 * COST))))
    btc_eth = {c: prices[c] for c in ("BTCUSDT", "ETHUSDT") if c in prices}
    print(_row("BTC + ETH only", summary(basket(btc_eth))))
    if "BTCUSDT" in prices:
        print(_row("BTC only", summary(basket({"BTCUSDT": prices["BTCUSDT"]}))))
    hold = pd.DataFrame({c: p.pct_change() for c, p in btc_eth.items()}).mean(axis=1).loc["2018":]
    print(_row("control: BTC+ETH buy & hold, unlevered", summary(hold)))
    rng = np.random.default_rng(1)
    shuffled = []
    for _ in range(20):
        r = {}
        for c, p in prices.items():
            w = weights(p)
            w = pd.Series(rng.permutation(w.fillna(0).to_numpy()), index=w.index).where(w.notna())
            pos = w.shift(1)
            r[c] = pos * p.pct_change() - pos.diff().abs() * COST
        b = pd.DataFrame(r)
        shuffled.append(summary((b.mean(axis=1) * np.sqrt(b.notna().sum(axis=1).clip(lower=1))).loc["2018":])["sharpe"])
    print(f"\nShuffled weights (20 runs): Sharpe mean {np.mean(shuffled):.2f}, best {np.max(shuffled):.2f}")
    base = basket(prices).dropna()
    print("\nBy year: " + "  ".join(f"{y}: {((1 + base[str(y)]).prod() - 1) * 100:+.1f}%"
                                  for y in sorted(set(base.index.year))))


def today(prices: dict[str, pd.Series]) -> None:
    """Weights for the next day, as a share of the capital given to the strategy."""
    n = len(prices)
    rows = []
    for c, p in prices.items():
        w = weights(p).iloc[-1] * np.sqrt(n) / n  # the basket scaling, per coin
        rows.append((c, p.index[-1].date(), float(p.iloc[-1]), 0.0 if np.isnan(w) else float(w)))
    total = sum(r[3] for r in rows)
    print(f"\nTarget weights for the next day (share of strategy capital; sum {total * 100:.0f}%, rest in USDT):")
    for c, d, px, w in sorted(rows, key=lambda r: -r[3]):
        print(f"  {c:<9} close {d} {px:>12.4f}   weight {w * 100:5.1f}%")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(pathlib.Path.home() / "aegis-data" / "binance-spot-1d"))
    ap.add_argument("--today", action="store_true")
    a = ap.parse_args()
    out = pathlib.Path(a.out)
    download(COINS, out)
    prices = {c: s for c in COINS if (s := load(c, out)) is not None and len(s) > 300}
    if not a.today:
        report(prices)
    today(prices)


if __name__ == "__main__":
    main()
