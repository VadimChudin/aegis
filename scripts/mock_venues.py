"""Local stand-in for the Binance and Bybit endpoints AEGIS uses, for development
where the real APIs are unreachable (geo-blocks, CI). Prices are a random walk.

    python scripts/mock_venues.py            # listens on 127.0.0.1:8765
    AEGIS_BINANCE_URL=http://127.0.0.1:8765/binance AEGIS_BINANCE_SPOT_URL=http://127.0.0.1:8765/binance \
    AEGIS_BYBIT_URL=http://127.0.0.1:8765/bybit cargo run -p aegis-app

Log in with API key `test-key` and secret `test-secret`; the mock checks the
HMAC signature exactly like the venues do.
"""

import hashlib
import hmac
import json
import random
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

KEY, SECRET = "test-key", "test-secret"
SECONDS = {"1m": 60, "5m": 300, "15m": 900, "1h": 3600, "4h": 14400, "1d": 86400}
BYBIT_TF = {"1": "1m", "5": "5m", "15": "15m", "60": "1h", "240": "4h", "D": "1d"}


def sign(payload):
    return hmac.new(SECRET.encode(), payload.encode(), hashlib.sha256).hexdigest()


def bars(tf, limit, depth=1500):
    """Last `limit` bars of one fixed history, so history and live polls agree."""
    return _series(tf, depth)[-limit:]


def _series(tf, limit):
    step = SECONDS[tf]
    now = int(time.time())
    last_open = now - now % step
    rng = random.Random(step)  # stable history per timeframe
    price, out = 4300.0, []
    for i in range(limit):
        t = last_open - (limit - 1 - i) * step
        o = price
        c = o + rng.gauss(0, 0.6 * (step / 60) ** 0.5)
        if i == limit - 1:  # forming bar moves on every request
            c += random.gauss(0, 0.4)
        h, lo = max(o, c) + abs(rng.gauss(0, 0.8)), min(o, c) - abs(rng.gauss(0, 0.8))
        out.append((t, o, h, lo, c, abs(rng.gauss(50, 20))))
        price = c
    return out


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        sys.stderr.write("mock: " + fmt % args + "\n")

    def reply(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def binance_signed(self, url, q):
        unsigned = url.query.rsplit("&signature=", 1)[0]
        if self.headers.get("X-MBX-APIKEY") != KEY or q.get("signature") != sign(unsigned):
            self.reply(401, {"code": -2015, "msg": "Invalid API-key, IP, or permissions for action."})
            return False
        return True

    def bybit_signed(self, url):
        h = self.headers
        expected = sign(f"{h.get('X-BAPI-TIMESTAMP')}{KEY}{h.get('X-BAPI-RECV-WINDOW')}{url.query}")
        if h.get("X-BAPI-API-KEY") != KEY or h.get("X-BAPI-SIGN") != expected:
            self.reply(200, {"retCode": 10003, "retMsg": "API key is invalid."})
            return False
        return True

    def do_GET(self):
        url = urlsplit(self.path)
        q = {k: v[0] for k, v in parse_qs(url.query).items()}
        now_ms = int(time.time() * 1000)
        path = url.path
        if path == "/binance/fapi/v1/time":
            return self.reply(200, {"serverTime": now_ms})
        if path == "/binance/fapi/v1/exchangeInfo":
            return self.reply(200, {"symbols": [
                {"symbol": "BTCUSDT", "status": "TRADING", "contractType": "PERPETUAL"},
                {"symbol": "XAUUSDT", "status": "TRADING", "contractType": "TRADIFI_PERPETUAL"}]})
        if path == "/binance/fapi/v1/klines":
            rows = bars(q["interval"], int(q.get("limit", 500)))
            return self.reply(200, [[t * 1000, f"{o:.2f}", f"{h:.2f}", f"{lo:.2f}", f"{c:.2f}", f"{v:.3f}",
                                     t * 1000 + 59999, "0", 0, "0", "0", "0"] for t, o, h, lo, c, v in rows])
        if path == "/binance/fapi/v2/balance":
            if self.binance_signed(url, q):
                self.reply(200, [{"asset": "USDT", "balance": "1250.40", "availableBalance": "1250.40"}])
            return None
        if path == "/binance/fapi/v1/positionSide/dual":
            if self.binance_signed(url, q):
                self.reply(200, {"dualSidePosition": False})
            return None
        if path == "/binance/sapi/v1/account/apiRestrictions":
            if self.binance_signed(url, q):
                self.reply(200, {"ipRestrict": False, "createTime": now_ms, "enableReading": True,
                                 "enableWithdrawals": False, "enableFutures": False,
                                 "enableSpotAndMarginTrading": False})
            return None
        if path == "/bybit/v5/market/time":
            return self.reply(200, {"retCode": 0, "retMsg": "OK", "result": {
                "timeSecond": str(now_ms // 1000), "timeNano": str(now_ms * 1_000_000)}, "time": now_ms})
        if path == "/bybit/v5/market/instruments-info":
            return self.reply(200, {"retCode": 0, "retMsg": "OK", "result": {
                "category": "linear", "list": [{"symbol": q.get("symbol"), "status": "Trading"}]}})
        if path == "/bybit/v5/market/kline":
            rows = bars(BYBIT_TF[q["interval"]], int(q.get("limit", 200)))
            lst = [[str(t * 1000), f"{o:.2f}", f"{h:.2f}", f"{lo:.2f}", f"{c:.2f}", f"{v:.3f}", "0"]
                   for t, o, h, lo, c, v in reversed(rows)]
            return self.reply(200, {"retCode": 0, "retMsg": "OK", "result": {"list": lst}})
        if path == "/bybit/v5/user/query-api":
            if self.bybit_signed(url):
                self.reply(200, {"retCode": 0, "retMsg": "OK", "result": {
                    "readOnly": 1, "uta": 1, "ips": ["*"], "deadlineDay": 83,
                    "permissions": {"ContractTrade": [], "Wallet": [], "Spot": []}}})
            return None
        if path == "/bybit/v5/account/wallet-balance":
            if self.bybit_signed(url):
                self.reply(200, {"retCode": 0, "retMsg": "OK", "result": {"list": [{"totalEquity": "980.15"}]}})
            return None
        return self.reply(404, {"code": -1, "msg": "not found"})


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    print(f"mock venues on http://127.0.0.1:{port}", file=sys.stderr)
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
