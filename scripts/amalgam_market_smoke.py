"""Bounded engineering smoke test, NOT a pristine holdout or profitability proof.

Run after `cargo build -p aegis-core --release --example bounce`:
  python scripts/amalgam_market_smoke.py --binary target/release/examples/bounce --out /tmp/aegis-amalgam

Uses public Binance XAUUSDT Jun-Aug 2026 5m/1m candles, checksum-verified archives,
explicit Binance cost assumptions, model OFF (short history), seeds 7/21/99.
No quote, queue, funding or 1-second data; those require independent execution tests.
The history was previously investigated; do not interpret it as untouched OOS.
"""
import argparse
import csv
import hashlib
import io
import json
import pathlib
import subprocess
import urllib.request
import zipfile
import os

BASE = "https://data.binance.vision/"
MONTHS = ("2026-06", "2026-07", "2026-08")
SEEDS = (7, 21, 99)


def fetch(url):
    with urllib.request.urlopen(url, timeout=90) as response:
        return response.read()


def candles(out, tf):
    rows, provenance = [], []
    for month in MONTHS:
        key = f"data/futures/um/monthly/klines/XAUUSDT/{tf}/XAUUSDT-{tf}-{month}.zip"
        archive = out / key.rsplit("/", 1)[1]
        raw = archive.read_bytes() if archive.exists() else fetch(BASE + key)
        expected = fetch(BASE + key + ".CHECKSUM").decode().split()[0]
        digest = hashlib.sha256(raw).hexdigest()
        if digest != expected:
            raise ValueError(f"Archive checksum mismatch: {key}")
        archive.write_bytes(raw)
        with zipfile.ZipFile(io.BytesIO(raw)) as z:
            with z.open(z.namelist()[0]) as f:
                part = list(csv.reader(io.TextIOWrapper(f)))
        if not part or part[0][0] != "open_time":
            raise ValueError(f"Unexpected archive CSV header: {key}")
        part = part[1:]
        for r in part:
            op, hi, lo, cl = map(float, r[1:5])
            if not lo <= min(op, cl) <= max(op, cl) <= hi:
                raise ValueError(f"Invalid OHLC in {key}")
        rows.extend(part)
        provenance.append({"key": key, "sha256": digest, "rows": len(part)})
    rows.sort(key=lambda r: int(r[0]))
    stamps = [r[0] for r in rows]
    if len(set(stamps)) != len(stamps):
        raise ValueError("Duplicate timestamps")
    dest = out / (tf + ".csv")
    with dest.open("w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["time", "open", "high", "low", "close", "volume", "trades", "buy_volume"])
        writer.writerows([[r[0], *r[1:6], r[8], r[9]] for r in rows])
    return dest, provenance


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--reuse-csv", action="store_true", help="Use previously checksum-verified normalized CSVs in out")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    provenance = []
    if args.reuse_csv:
        bars, minutes = args.out / "5m.csv", args.out / "1m.csv"
        for p in (bars, minutes):
            if not p.is_file():
                raise FileNotFoundError(p)
        provenance = json.loads((args.out / "provenance.json").read_text())
    else:
        bars, p5 = candles(args.out, "5m")
        minutes, p1 = candles(args.out, "1m")
        provenance = p5 + p1
    (args.out / "provenance.json").write_text(json.dumps(provenance, indent=2))
    params = {"use_model": False, "sec_engine": False, "maker_bps": 2.0, "taker_bps": 5.0, "slippage": 0.05, "spread": 0.02}
    params_file = args.out / "params.json"
    params_file.write_text(json.dumps(params, indent=2))
    summary = []
    for seed in SEEDS:
        spec = {"population": 16, "generations": 5, "seed": seed, "target_win_rate": 0.0,
                "target_trades_per_day": 0.0, "min_trades": 30, "train_days": 45, "test_days": 15,
                "metrics": ["none"], "tune": ["sl_atr", "tp_r", "max_bars"]}
        spec_file = args.out / f"spec-{seed}.json"
        spec_file.write_text(json.dumps(spec, indent=2))
        output, log = args.out / f"seed-{seed}.json", args.out / f"seed-{seed}.log"
        env = os.environ.copy()
        env["MINUTES"] = str(minutes.resolve())
        with output.open("w") as stdout, log.open("w") as stderr:
            subprocess.run([str(binary), "optimize", str(bars.resolve()), str(spec_file.resolve()), str(params_file.resolve())],
                           env=env, stdout=stdout, stderr=stderr, check=True, timeout=900)
        report = json.loads(output.read_text())
        optimizer = report.get("optimizer", report)
        summary.append({"seed": seed, "report_keys": list(report), "out_of_sample": optimizer.get("out_of_sample"),
                        "baseline": optimizer.get("baseline"), "evaluations": optimizer.get("evaluations")})
    (args.out / "summary.json").write_text(json.dumps({"limitations": "Previously studied history; engineering smoke only; model OFF; candle execution assumptions, not live fills", "seeds": summary}, indent=2))
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
