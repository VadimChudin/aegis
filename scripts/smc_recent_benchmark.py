#!/usr/bin/env python3
"""Benchmark the existing SMC zone/filter strategy; never tune on test.

python scripts/smc_recent_benchmark.py --csv /home/user/aegis_results/market/1m.csv --out smc_recent

CSV: t,o,h,l,c[,v], or timestamp/open/high/low/close/volume. t is UTC
open time (ISO, seconds or milliseconds). No downloads. Incomplete HTF bars
are unavailable. Prices without quotes assume a constant --spread (default 0).
All costs are in price units except maker/taker expressed as decimal rates.
Defaults: July 1-August 15 train, August 16-31 validation, September 1-15 test
(with supplied June bars used only as indicator warmup). All endpoints are
exclusive and --train-start/--train-end/--validation-end/--test-end accept
UTC dates or timezone-aware ISO timestamps. Original three-month alternative:
--train-start 2026-06-01 --train-end 2026-07-16 --validation-end 2026-08-01 --test-end 2026-09-01.
"""
from __future__ import annotations
import argparse
import hashlib
import itertools
import json
from pathlib import Path
import sys
import numpy as np
import pandas as pd
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))
from aegis_lab.research import smc


def timestamp(value):
    ts = pd.Timestamp(value)
    ts = ts.tz_localize("UTC") if ts.tzinfo is None else ts.tz_convert("UTC")
    return int(ts.value // 10**9)


def candidates():
    # Frozen before data access: all 36 combinations, not a winners-only sample.
    return [dict(smc.BASE, kinds=(kind,), htf=htf, rr=rr, buf=buf,
                 portfolio="one_position")
            for kind, htf, rr, buf in itertools.product(
                ("ob", "fvg", "sweep"), (3600, 14400), (1.0, 2.0, 3.0), (0.1, 0.25))]


def read_minutes(path):
    m = pd.read_csv(path)
    aliases = {"timestamp": "t", "open_time": "t", "time": "t", "open": "o",
               "high": "h", "low": "l", "close": "c", "volume": "v"}
    m = m.rename(columns=aliases)
    if not set(("t", "o", "h", "l", "c")).issubset(m):
        raise ValueError("CSV requires t,o,h,l,c or timestamp/open/high/low/close")
    numeric = pd.to_numeric(m.t, errors="coerce")
    if numeric.notna().all():
        scale = 1000 if numeric.abs().median() > 1e11 else 1
        m["t"] = (numeric / scale).astype(np.int64)
    else:
        m["t"] = pd.to_datetime(m.t, utc=True).astype("int64") // 10**9
    for key in ("o", "h", "l", "c"):
        m[key] = pd.to_numeric(m[key], errors="raise").astype(float)
    if "v" not in m:
        m["v"] = 0.0
    if m.t.duplicated().any() or (m.t % 60 != 0).any():
        raise ValueError("Duplicate or non-minute-aligned timestamps")
    if not np.isfinite(m[["o", "h", "l", "c"]]).all().all():
        raise ValueError("Nonfinite prices")
    if ((m.l > m[["o", "c"]].min(axis=1)) |
        (m.h < m[["o", "c"]].max(axis=1)) | (m.l > m.h) | (m.l <= 0)).any():
        raise ValueError("Invalid OHLC prices")
    return m.sort_values("t").reset_index(drop=True)


def metrics(trades, lo, hi):
    settled = trades[(trades.t >= lo) & (trades.t < hi) &
                     (trades.exit_t <= hi) & (trades.exit != 4)]
    censored = trades[(trades.exit == 4) | (trades.exit_t > hi)]
    return {"n": len(settled), "censored_n": len(censored),
            "r_net_sum": float(settled.r_net.sum()),
            "r_net_mean": float(settled.r_net.mean()) if len(settled) else None,
            "net_price_qty": float((settled.pnl_total - settled.fee_total).sum()),
            "fee_price_qty": float(settled.fee_total.sum()),
            "win_rate": float((settled.r_net > 0).mean()) if len(settled) else None}


def select(rows, min_train, min_validation):
    eligible = [r for r in rows if r["train_n"] >= min_train]
    if not eligible:
        return None, "No candidate met the preregistered minimum training count"
    eligible.sort(key=lambda r: (-r["train_r_net_sum"], r["candidate"]))
    shortlist = eligible[:3]
    validated = [r for r in shortlist if r["validation_n"] >= min_validation]
    if not validated:
        return eligible[0]["candidate"], "Training-only fallback: validation counts inadequate"
    validated.sort(key=lambda r: (-r["validation_r_net_sum"], -r["train_r_net_sum"], r["candidate"]))
    return validated[0]["candidate"], "Top three by training total net R; highest validation total net R"


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--csv", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--train-start", default="2026-07-01")
    ap.add_argument("--train-end", default="2026-08-16")
    ap.add_argument("--validation-end", default="2026-09-01")
    ap.add_argument("--test-end", default="2026-09-16")
    ap.add_argument("--maker", type=float, default=0.0002)
    ap.add_argument("--taker", type=float, default=0.0005)
    ap.add_argument("--slippage", type=float, default=0.05)
    ap.add_argument("--spread", type=float, default=0.0)
    ap.add_argument("--tick-through", type=float, default=0.01)
    ap.add_argument("--min-train", type=int, default=10)
    ap.add_argument("--min-validation", type=int, default=3)
    args = ap.parse_args()
    cuts = list(map(timestamp, (args.train_start, args.train_end, args.validation_end, args.test_end)))
    if not all(a < b for a, b in zip(cuts, cuts[1:])):
        ap.error("Date boundaries must increase; endpoints are exclusive UTC")
    if min(args.maker, args.taker, args.slippage, args.spread, args.tick_through) < 0:
        ap.error("Execution costs and tick-through must be nonnegative")
    if args.min_train < 1 or args.min_validation < 1:
        ap.error("Minimum counts must be positive")
    cfgs = candidates()
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "frozen_candidates.json").write_text(json.dumps(cfgs, indent=2))
    m = read_minutes(args.csv)
    execution = dict(maker=args.maker, taker=args.taker, slippage=args.slippage, spread=args.spread,
                     tick_through=args.tick_through)
    rows = [{"candidate": i, "settings": json.dumps(cfg, sort_keys=True)} for i, cfg in enumerate(cfgs)]
    coverage = {}
    for name, lo, hi in (("train", cuts[0], cuts[1]), ("validation", cuts[1], cuts[2])):
        period = m[(m.t >= lo) & (m.t + 60 <= hi)]
        coverage[name] = {"minute_n": len(period), "expected_minute_n": (hi - lo) // 60}
        prefix = m[m.t + 60 <= hi]
        if not len(period):
            raise ValueError(f"No {name} minutes in requested dates")
        lab = smc.Lab(m=prefix, **execution)
        for i, cfg in enumerate(cfgs):
            trades = lab.run(cfg, start=lo, end=hi)
            rows[i].update({f"{name}_{k}": v for k, v in metrics(trades, lo, hi).items()})
    selected, rule = select(rows, args.min_train, args.min_validation)
    # Selection is frozen here: test-period prices have not entered any Lab or score.
    pd.DataFrame(rows).to_csv(args.out / "all_candidates.csv", index=False)
    report = {"candidate_count": len(cfgs), "selection_rule": rule, "selected_candidate": selected,
              "settings": None if selected is None else cfgs[selected], "execution": execution,
              "dates_exclusive_utc": dict(zip(("train_start", "train_end", "validation_end", "test_end"),
                                                (args.train_start, args.train_end, args.validation_end, args.test_end))),
              "coverage": coverage, "csv": str(args.csv),
              "csv_sha256": hashlib.sha256(args.csv.read_bytes()).hexdigest(),
              "assumptions": ["Quotes absent: constant full spread, zero by default; stress with --spread",
                              "All 36 original OB/FVG/sweep limit-zone/filter configurations reported",
                              "Original filters: trend on; premium/discount and killzone off; original size/risk limits",
                              "One position globally; cancel competing zones on first fill; no same-zone reissue",
                              "Stop first on ambiguous minute; fill-bar TP not credited; resting fills one tick through",
                              "Censored/end-of-data trades excluded, not forced profitable boundary exits",
                              "Default July 1-August 15 training, August 16-31 validation, September 1-15 test; June is indicator warmup only",
                              "R and price-times-quantity are diagnostic, not compounded cash-account returns"]}
    if selected is not None:
        lo, hi = cuts[2:]
        period = m[(m.t >= lo) & (m.t + 60 <= hi)]
        report["coverage"]["test"] = {"minute_n": len(period), "expected_minute_n": (hi - lo) // 60}
        if len(period):
            lab = smc.Lab(m=m[m.t + 60 <= hi], **execution)
            trades = lab.run(cfgs[selected], start=lo, end=hi)
            trades.to_csv(args.out / "selected_test_trades.csv", index=False)
            report["test"] = metrics(trades, lo, hi)
        else:
            report["test"] = {"status": "No test minutes; performance unavailable"}
    (args.out / "summary.json").write_text(json.dumps(report, indent=2, allow_nan=False))
    print(json.dumps(report, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
