"""Build 5m bars with order-flow (microstructure) columns from Binance aggTrades.

Output CSV columns (one row per 5m bar, UTC open time in seconds):
  time,open,high,low,close,volume,buy_volume,trades,
  big_volume     volume of trades >= the rolling 99th pct trade size (large participants)
  big_delta      buy minus sell volume of those large trades
  max_trade      largest single aggressive order
  vol_top        volume traded in the top 20% of the bar's range
  vol_bottom     volume traded in the bottom 20% of the bar's range
  delta_top      delta in the top 20% of the range (cluster at the high)
  delta_bottom   delta in the bottom 20% of the range (cluster at the low)
  poc            price bucket ($0.5) with the most volume inside the bar
  imb_buy        count of $0.5 buckets where ask volume >= 3x bid volume
  imb_sell       count of $0.5 buckets where bid volume >= 3x ask volume
  oi             open interest at the bar (metrics, 5m)
  ls_top         top-trader long/short ratio
  taker_ratio    taker buy/sell volume ratio (metrics)

    python -m aegis_lab.research.bars --data ~/aegis-data/binance/XAUUSDT --out ~/aegis-data/xau_5m.csv
"""

from __future__ import annotations

import argparse
import pathlib
import zipfile

import numpy as np
import pandas as pd

BAR = 300
BUCKET = 0.5


def _read_zip_csv(path: pathlib.Path, **kw) -> pd.DataFrame:
    with zipfile.ZipFile(path) as z:
        with z.open(z.namelist()[0]) as f:
            head = f.read(64).decode()
        with z.open(z.namelist()[0]) as f:
            has_header = not head[:1].isdigit()
            return pd.read_csv(f, header=0 if has_header else None, **kw)


def klines(data: pathlib.Path) -> pd.DataFrame:
    cols = ["open_time", "open", "high", "low", "close", "volume", "close_time", "quote_volume", "count",
            "taker_buy_volume", "taker_buy_quote_volume", "ignore"]
    frames = []
    for p in sorted((data / "klines-5m").glob("*.zip")):
        df = _read_zip_csv(p)
        df.columns = cols
        frames.append(df)
    k = pd.concat(frames).drop_duplicates("open_time").sort_values("open_time")
    return pd.DataFrame({
        "time": k.open_time.astype("int64") // 1000,
        "open": k.open, "high": k.high, "low": k.low, "close": k.close,
        "volume": k.volume, "buy_volume": k.taker_buy_volume, "trades": k["count"],
    }).set_index("time")


def flow(data: pathlib.Path) -> pd.DataFrame:
    out = []
    big_ref = None
    for p in sorted((data / "aggTrades").glob("*.zip")):
        t = _read_zip_csv(p, usecols=["price", "quantity", "transact_time", "is_buyer_maker"])
        if t.empty:
            continue
        t["is_buyer_maker"] = t.is_buyer_maker.astype(str).str.lower().eq("true")
        t["bar"] = (t.transact_time // 1000 // BAR) * BAR
        t["sign"] = np.where(t.is_buyer_maker, -1.0, 1.0)  # buyer maker = aggressive sell
        t["sq"] = t.sign * t.quantity
        # Large-trade threshold from the previous file (no look-ahead inside the month).
        thr = big_ref if big_ref is not None else t.quantity.quantile(0.99)
        big_ref = t.quantity.quantile(0.99)
        big = t.quantity >= thr
        g = t.groupby("bar")
        hi = g.price.transform("max")
        lo = g.price.transform("min")
        rng = (hi - lo).replace(0, np.nan)
        pos = (t.price - lo) / rng
        top = pos >= 0.8
        bot = pos <= 0.2
        t["bucket"] = (t.price / BUCKET).round() * BUCKET
        f = pd.DataFrame({
            "big_volume": t.quantity.where(big, 0).groupby(t.bar).sum(),
            "big_delta": t.sq.where(big, 0).groupby(t.bar).sum(),
            "max_trade": g.quantity.max(),
            "vol_top": t.quantity.where(top, 0).groupby(t.bar).sum(),
            "vol_bottom": t.quantity.where(bot, 0).groupby(t.bar).sum(),
            "delta_top": t.sq.where(top, 0).groupby(t.bar).sum(),
            "delta_bottom": t.sq.where(bot, 0).groupby(t.bar).sum(),
        })
        cl = t.groupby(["bar", "bucket"]).agg(v=("quantity", "sum"), d=("sq", "sum"))
        cl["buy"] = (cl.v + cl.d) / 2
        cl["sell"] = (cl.v - cl.d) / 2
        f["poc"] = cl.v.groupby(level=0).idxmax().map(lambda x: x[1])
        f["imb_buy"] = (cl.buy >= 3 * cl.sell.clip(lower=1e-9)).groupby(level=0).sum()
        f["imb_sell"] = (cl.sell >= 3 * cl.buy.clip(lower=1e-9)).groupby(level=0).sum()
        out.append(f)
        print(p.name, len(t), flush=True)
    fl = pd.concat(out)
    return fl[~fl.index.duplicated()]


def metrics(data: pathlib.Path) -> pd.DataFrame:
    frames = [_read_zip_csv(p) for p in sorted((data / "metrics").glob("*.zip"))]
    m = pd.concat(frames)
    # Seconds since epoch regardless of the datetime resolution pandas picks (ns in 2.x, us in 3.x).
    m["time"] = (pd.to_datetime(m.create_time) - pd.Timestamp("1970-01-01")) // pd.Timedelta(seconds=1)
    m = m.drop_duplicates("time").set_index("time").sort_index()
    return pd.DataFrame({
        "oi": m.sum_open_interest,
        "ls_top": m.sum_toptrader_long_short_ratio,
        "taker_ratio": m.sum_taker_long_short_vol_ratio,
    })


def build(data: pathlib.Path) -> pd.DataFrame:
    k = klines(data)
    df = k.join(flow(data)).join(metrics(data))
    # Metrics are stamped at the end of their 5m window; use the value known at bar close.
    df[["oi", "ls_top", "taker_ratio"]] = df[["oi", "ls_top", "taker_ratio"]].ffill()
    return df.fillna({c: 0.0 for c in ["big_volume", "big_delta", "max_trade", "vol_top", "vol_bottom",
                                       "delta_top", "delta_bottom", "imb_buy", "imb_sell"]})


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", type=pathlib.Path, default=pathlib.Path.home() / "aegis-data/binance/XAUUSDT")
    ap.add_argument("--out", type=pathlib.Path, default=pathlib.Path.home() / "aegis-data/xau_5m.csv")
    a = ap.parse_args()
    df = build(a.data)
    df.to_csv(a.out, float_format="%.6g")
    print(a.out, len(df), df.index.min(), df.index.max())


if __name__ == "__main__":
    main()
