"""Gold component library: public gold systems re-implemented as components, tested alone,
then merged pairwise (the "2048" approach: only survivors are combined).

Data: XAUUSD CFD 1h, 2004-06 … 2025-02 (MetaTrader export on Hugging Face, server time = New York
+ 7 h). Every component returns a target position in [-1, 1] decided at a bar's close and held
from the next bar. Cost: COST_USD per unit of turnover (one way), i.e. $0.40/oz per round trip.

A component "works" only if its *timing* earns: returns are measured both raw and with gold's
average hourly drift removed (pos x (r - mean r)), because long-only rules on a market that rose
10x look good by just being long. Selection uses 2004-2014; 2015-2025 is reported apart.

    python -m aegis_lab.research.gold_lab [--data DIR]
"""

from __future__ import annotations

import argparse
import itertools
import pathlib
import urllib.request

import numpy as np
import pandas as pd

HF = "https://huggingface.co/datasets/agzdws/xauusd-gold-price-historical-data-2004-2025/resolve/main/"
COST_USD = 0.20
SPLIT = "2015-01-01"
END = "2025-03-01"
HOURS_PER_YEAR = 24 * 5 * 52 * 0.96  # gold trades ~23h x 5 days


def load(data: pathlib.Path) -> pd.DataFrame:
    data.mkdir(parents=True, exist_ok=True)
    raw = data / "XAU_1h_data.jsonl"
    if not raw.exists():
        urllib.request.urlretrieve(HF + raw.name, raw)
    df = pd.read_json(raw, lines=True)
    ny = pd.to_datetime(df.Date, format="%Y.%m.%d %H:%M") - pd.Timedelta(hours=7)
    df = df.assign(ny=ny).rename(columns=str.lower)
    df = df[(df.ny < END) & (df.low > 0)].drop_duplicates("ny").sort_values("ny").set_index("ny")
    return df[["open", "high", "low", "close", "volume"]]


# ---------------------------------------------------------------- helpers


def ema(s, n):
    return s.ewm(span=n, adjust=False).mean()


def atr(df, n=14):
    pc = df.close.shift()
    tr = pd.concat([df.high - df.low, (df.high - pc).abs(), (df.low - pc).abs()], axis=1).max(axis=1)
    return tr.rolling(n).mean()


def rsi(s, n):
    d = s.diff()
    up = d.clip(lower=0).ewm(alpha=1 / n, adjust=False).mean()
    dn = (-d.clip(upper=0)).ewm(alpha=1 / n, adjust=False).mean()
    return 100 - 100 / (1 + up / dn)


def resample(df, rule):
    """Bars of a higher timeframe, labelled by their last hour (known at that hour's close)."""
    o = df.resample(rule, label="right", closed="right")
    r = pd.DataFrame({"open": o.open.first(), "high": o.high.max(), "low": o.low.min(),
                      "close": o.close.last(), "volume": o.volume.sum()}).dropna()
    return r


def to_hourly(pos_htf: pd.Series, idx: pd.Index) -> pd.Series:
    """Carry a higher-timeframe decision forward onto hourly bars (no look-ahead)."""
    return pos_htf.reindex(idx.union(pos_htf.index)).ffill().reindex(idx).fillna(0)


def daily(df):
    """Trading days end at 17:00 New York (the daily break)."""
    shifted = df.copy()
    shifted.index = shifted.index + pd.Timedelta(hours=7)  # 17:00 NY -> midnight
    d = resample(shifted, "1D")
    d.index = d.index - pd.Timedelta(hours=7)
    return d


def hold(entry: pd.Series, exit_: pd.Series) -> pd.Series:
    """1 from an entry bar until an exit bar (state machine)."""
    out = np.zeros(len(entry))
    on = False
    e, x = entry.to_numpy(), exit_.to_numpy()
    for i in range(len(out)):
        if on and x[i]:
            on = False
        elif not on and e[i]:
            on = True
        out[i] = on
    return pd.Series(out, index=entry.index)


# ---------------------------------------------------------------- components
# Each: (source, description, function(df) -> position series on the hourly index)


def c_friday(df):
    """TradingView 'Gold Friday Anomaly' (piirsalu): long from Thursday's close to Friday's close."""
    h, wd = df.index.hour, df.index.dayofweek
    fri_session = ((wd == 3) & (h >= 17)) | ((wd == 4) & (h < 17))
    return pd.Series(fri_session.astype(float), index=df.index)


def c_turn_of_month(df):
    """Classic turn-of-month: long over the last and first three trading days of each month."""
    d = daily(df)
    m = d.index.to_period("M")
    rank = d.groupby(m).cumcount()
    left = d.groupby(m).cumcount(ascending=False)
    pos = ((rank < 3) | (left < 1)).astype(float)
    return to_hourly(pos.shift(-1).fillna(0), df.index)  # decided at the previous close


def c_tsmom_daily(df, long_only=True):
    """Time-series momentum ensemble (20/60/120/250 days), the rule from trend.md."""
    c = daily(df).close
    score = sum(np.sign(c / c.shift(L) - 1) for L in (20, 60, 120, 250)) / 4
    return to_hourly(score.clip(lower=0) if long_only else score, df.index)


def c_donchian_h4(df):
    """Turtle on H4: long on a 55-bar high, short on a 55-bar low, exit on the opposite 20-bar extreme."""
    b = resample(df, "4h")
    hi55, lo55 = b.high.rolling(55).max().shift(), b.low.rolling(55).min().shift()
    hi20, lo20 = b.high.rolling(20).max().shift(), b.low.rolling(20).min().shift()
    lng = hold(b.close > hi55, b.close < lo20)
    sht = hold(b.close < lo55, b.close > hi20)
    return to_hourly(lng - sht, df.index)


def c_ema_h1(df):
    """maker-tung/xauusd-trend-follow & FvgGold trend filter: long while EMA50 > EMA200 on H1."""
    return (ema(df.close, 50) > ema(df.close, 200)).astype(float)


def c_macd_zero_h4(df):
    """TradingView 'Golden Edge Pro' core: long while H4 MACD(16,26) is above zero (long-only)."""
    b = resample(df, "4h")
    macd = ema(b.close, 16) - ema(b.close, 26)
    return to_hourly((macd > 0).astype(float), df.index)


def c_trident(df):
    """TradingView 'Golden Trident': swing structure (new 20-bar high flips bullish, new low bearish)
    on H4, long-only above EMA200, blocked when the 20-bar range is under 3 ATR (chop)."""
    b = resample(df, "4h")
    hi, lo = b.high.rolling(20).max().shift(), b.low.rolling(20).min().shift()
    bull = hold(b.close > hi, b.close < lo)
    trend = b.close > ema(b.close, 200)
    chop = (b.high.rolling(20).max() - b.low.rolling(20).min()) < 3 * atr(b)
    entry_ok = (bull.diff() > 0) & trend & ~chop
    pos = hold(entry_ok, bull == 0)
    return to_hourly(pos, df.index)


def c_rsi2_daily(df):
    """Connors RSI(2): long when daily RSI(2) < 10 above SMA200, exit when the close is above SMA5."""
    d = daily(df)
    pos = hold((rsi(d.close, 2) < 10) & (d.close > d.close.rolling(200).mean()), d.close > d.close.rolling(5).mean())
    return to_hourly(pos, df.index)


def c_asia_long(df):
    """Session drift: long during the Asian session (18:00-03:00 New York)."""
    h = df.index.hour
    return pd.Series(((h >= 18) | (h < 3)).astype(float), index=df.index)


def c_london_breakout(df):
    """Asia-range breakout: range 20:00-02:00 NY, trade the break 02:00-11:00, flat at 11:00."""
    h = df.index.hour
    day = (df.index - pd.Timedelta(hours=18)).normalize()
    asia = (h >= 20) | (h < 2)
    rng_hi = df.high.where(asia).groupby(day).transform("max")
    rng_lo = df.low.where(asia).groupby(day).transform("min")
    window = (h >= 2) & (h < 11)
    sig = np.where(window & (df.close > rng_hi), 1.0, np.where(window & (df.close < rng_lo), -1.0, np.nan))
    s = pd.Series(sig, index=df.index)
    s[~window] = 0.0
    return s.groupby(day).ffill().fillna(0)


def c_bollinger_mr_h1(df):
    """Mean reversion on H1: fade a close outside the 20-bar 2-sigma band, exit at the mean."""
    m, sd = df.close.rolling(20).mean(), df.close.rolling(20).std()
    lng = hold(df.close < m - 2 * sd, df.close >= m)
    sht = hold(df.close > m + 2 * sd, df.close <= m)
    return lng - sht


COMPONENTS = {
    "friday": ("TradingView: Gold Friday Anomaly", c_friday),
    "turn_of_month": ("classic seasonality", c_turn_of_month),
    "tsmom_daily": ("trend.md / Moskowitz et al.", c_tsmom_daily),
    "tsmom_daily_ls": ("same, long/short", lambda df: c_tsmom_daily(df, long_only=False)),
    "donchian_h4": ("Turtle rules", c_donchian_h4),
    "ema50_200_h1": ("GitHub maker-tung, FvgGold filter", c_ema_h1),
    "macd_zero_h4": ("TradingView: Golden Edge Pro", c_macd_zero_h4),
    "trident_h4": ("TradingView: Golden Trident", c_trident),
    "rsi2_daily": ("Connors RSI(2)", c_rsi2_daily),
    "asia_long": ("session drift", c_asia_long),
    "london_breakout": ("Asia-range breakout", c_london_breakout),
    "bollinger_mr_h1": ("mean reversion H1", c_bollinger_mr_h1),
}


# ---------------------------------------------------------------- evaluation


def evaluate(df: pd.DataFrame, pos: pd.Series, mu: float) -> dict:
    r = df.close.pct_change().fillna(0)
    p = pos.shift(1).fillna(0).clip(-1, 1)
    cost = p.diff().abs().fillna(0) * COST_USD / df.close
    net = p * r - cost
    timing = p * (r - mu) - cost
    out = {"exposure": float(p.abs().mean()), "trades_yr": float((p.diff().abs() > 0).sum() / 2 /
                                                                  (len(df) / HOURS_PER_YEAR))}
    for name, x in (("", net), ("t_", timing)):
        for part, sel in (("all", slice(None)), ("in", slice(None, SPLIT)), ("out", slice(SPLIT, None))):
            y = x.loc[sel]
            d = y.groupby(y.index.normalize()).sum()
            d = d[d != 0] if part else d
            ann = y.mean() * HOURS_PER_YEAR
            sh = d.mean() / d.std() * np.sqrt(252) if d.std() > 0 else 0.0
            tstat = d.mean() / d.std() * np.sqrt(len(d)) if d.std() > 0 else 0.0
            out[f"{name}ann_{part}"] = ann
            out[f"{name}sh_{part}"] = sh
            out[f"{name}t_{part}"] = tstat
    return out


def table(rows: list[tuple[str, dict]]) -> None:
    print("| Component | Exposure | Trades/yr | Net %/yr in / out | Timing %/yr in / out | Timing t in / out |")
    print("|---|---|---|---|---|---|")
    for name, s in rows:
        print(f"| {name} | {s['exposure'] * 100:.0f}% | {s['trades_yr']:.0f} | "
              f"{s['ann_in'] * 100:+.1f} / {s['ann_out'] * 100:+.1f} | "
              f"{s['t_ann_in'] * 100:+.1f} / {s['t_ann_out'] * 100:+.1f} | "
              f"{s['t_t_in']:+.1f} / {s['t_t_out']:+.1f} |")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default=str(pathlib.Path.home() / "aegis-data" / "hf-xau"))
    a = ap.parse_args()
    df = load(pathlib.Path(a.data))
    r = df.close.pct_change().fillna(0)
    mu = float(r.loc[:SPLIT].mean())  # drift from the in-sample period only
    print(f"XAUUSD 1h {df.index[0].date()} … {df.index[-1].date()}, {len(df):,} bars; "
          f"gold drift {mu * HOURS_PER_YEAR * 100:.1f}%/yr in sample\n")

    positions = {k: f(df) for k, (_, f) in COMPONENTS.items()}
    stats = {k: evaluate(df, p, mu) for k, p in positions.items()}
    print("## Level 1: components alone\n")
    table([(f"{k} ({COMPONENTS[k][0]})", stats[k]) for k in COMPONENTS])

    # A component survives on in-sample evidence only: timing earns with t >= 2 in 2004-2014.
    alive = [k for k in COMPONENTS if stats[k]["t_t_in"] >= 2 and stats[k]["t_ann_in"] > 0]
    print(f"\nSurvivors (timing t >= 2 in 2004-2014): {', '.join(alive) or 'none'}")

    print("\n## Level 2: pairs of survivors (average of positions; AND for long-only filters)\n")
    merged = []
    for x, y in itertools.combinations(alive, 2):
        avg = (positions[x] + positions[y]) / 2
        merged.append((f"{x} + {y} (avg)", evaluate(df, avg, mu)))
        both = positions[x].clip(lower=0) * positions[y].clip(lower=0)
        if both.abs().sum() > 0:
            merged.append((f"{x} AND {y}", evaluate(df, (both > 0).astype(float), mu)))
    if merged:
        table(merged)
    if alive:
        allavg = sum(positions[k] for k in alive) / len(alive)
        print("\nAll survivors averaged:")
        table([("all survivors (avg)", evaluate(df, allavg, mu))])


if __name__ == "__main__":
    main()
