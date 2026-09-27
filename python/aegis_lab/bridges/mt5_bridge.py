"""JSON-lines bridge between the AEGIS app and a local MetaTrader 5 terminal.

The app starts this script, writes one JSON request per line to stdin and reads
one JSON reply per line from stdout:

    {"id": 1, "cmd": "connect", "login": 123, "password": "...", "server": "RoboForex-ECN"}
    {"id": 1, "ok": true, "result": {"symbol": "XAUUSD", "account": "..."}}
    {"id": 2, "ok": false, "error": "..."}

Nothing else is ever written to stdout; diagnostics go to stderr. The script
exits when stdin closes, so it never outlives the app.
"""

import json
import sys
import time

PROTOCOL = 1
TIMEFRAMES = {
    "1m": "TIMEFRAME_M1",
    "5m": "TIMEFRAME_M5",
    "15m": "TIMEFRAME_M15",
    "1h": "TIMEFRAME_H1",
    "4h": "TIMEFRAME_H4",
    "1d": "TIMEFRAME_D1",
}
MAX_BARS = 5000


class BridgeError(Exception):
    pass


class Bridge:
    def __init__(self, mt5=None):
        self._mt5 = mt5
        self.symbol = None
        self.offset = 0

    def mt5(self):
        if self._mt5 is None:
            try:
                import MetaTrader5  # noqa: PLC0415 - optional, Windows-only dependency
            except ImportError:
                raise BridgeError(
                    "the MetaTrader5 Python package is not installed: run `pip install MetaTrader5` (Windows only)"
                ) from None
            self._mt5 = MetaTrader5
        return self._mt5

    def hello(self, _req):
        reply = {"protocol": PROTOCOL, "python": sys.version.split()[0]}
        try:
            mt5 = self.mt5()
            reply["mt5_package"] = str(getattr(mt5, "__version__", "installed"))
        except BridgeError as e:
            reply["mt5_error"] = str(e)
        return reply

    def connect(self, req):
        mt5 = self.mt5()
        kwargs = {
            "login": int(req["login"]),
            "password": str(req["password"]),
            "server": str(req["server"]),
            "timeout": 30000,
        }
        path = req.get("terminal_path")
        ok = mt5.initialize(path, **kwargs) if path else mt5.initialize(**kwargs)
        if not ok:
            code, message = mt5.last_error()
            mt5.shutdown()
            raise BridgeError(f"MT5 login failed ({code}): {message}")
        self.symbol = self._find_symbol(mt5)
        self.offset = self._server_offset(mt5)
        info = mt5.account_info()
        account = (
            f"{info.login} · {info.server} · {info.currency} {info.balance:.2f}" if info else f"{kwargs['login']}"
        )
        return {
            "symbol": self.symbol,
            "account": account,
            "server_offset": self.offset,
            "checks": self._checks(mt5, info),
        }

    def _checks(self, mt5, info):
        """Terminal and account state the app shows in the connect checklist."""
        checks = []

        def add(cid, label, status, detail=""):
            checks.append({"id": cid, "label": label, "status": status, "detail": detail})

        term = mt5.terminal_info()
        if term is None:
            add("terminal", "Terminal", "warn", "terminal_info() returned nothing")
        elif not term.connected:
            add("terminal", "Terminal", "fail", "the terminal is not connected to the trade server")
        else:
            add("terminal", "Terminal", "ok", f"{term.company} · build {term.build}")
        if term is not None:
            if term.trade_allowed:
                add("algo", "Algo Trading button", "ok", "enabled")
            else:
                add("algo", "Algo Trading button", "warn", "off: press Algo Trading on the toolbar before trading")
        if info is not None:
            if not info.trade_allowed:
                add("trading", "Trading on this account", "warn", "not allowed (investor password or disabled account)")
            elif not info.trade_expert:
                add("trading", "Trading on this account", "warn", "Expert Advisors are not allowed on this account")
            else:
                add("trading", "Trading on this account", "ok", "allowed, including automated trading")
        sym = mt5.symbol_info(self.symbol)
        spread = f" · spread {sym.spread} points" if sym is not None and getattr(sym, "spread", None) else ""
        add("symbol", "Gold symbol", "ok", f"{self.symbol}{spread}")
        if info is not None:
            status = "ok" if info.balance > 0 else "warn"
            add("balance", "Balance", status, f"{info.currency} {info.balance:.2f}")
        return checks

    def _find_symbol(self, mt5):
        # Account types name gold differently (XAUUSD, XAUUSD.r, ...): prefer the plain name.
        names = ["XAUUSD"] + sorted((s.name for s in (mt5.symbols_get("*XAUUSD*") or ())), key=len)
        for name in names:
            if mt5.symbol_info(name) is not None and mt5.symbol_select(name, True):
                return name
        raise BridgeError("this MT5 account has no XAUUSD symbol")

    def _server_offset(self, mt5):
        # Bars carry trade-server time (RoboForex runs UTC+2/+3), not UTC. The last
        # tick gives the offset in whole hours; a stale tick (market closed) gives none.
        tick = mt5.symbol_info_tick(self.symbol)
        if not tick or not tick.time:
            return 0
        diff = tick.time - time.time()
        if abs(diff) > 14 * 3600:
            return 0
        return int(round(diff / 3600.0)) * 3600

    def candles(self, req):
        if not self.symbol:
            raise BridgeError("not connected")
        mt5 = self.mt5()
        name = TIMEFRAMES.get(req.get("timeframe"))
        if name is None:
            raise BridgeError(f"unknown timeframe: {req.get('timeframe')}")
        limit = max(1, min(int(req.get("limit", 500)), MAX_BARS))
        rates = mt5.copy_rates_from_pos(self.symbol, getattr(mt5, name), 0, limit)
        if rates is None:
            code, message = mt5.last_error()
            raise BridgeError(f"copy_rates_from_pos failed ({code}): {message}")
        # CFDs have no exchange volume; tick volume is the closest proxy.
        return [
            {
                "time": int(r["time"]) - self.offset,
                "open": float(r["open"]),
                "high": float(r["high"]),
                "low": float(r["low"]),
                "close": float(r["close"]),
                "volume": float(r["tick_volume"]),
            }
            for r in rates
        ]

    def shutdown(self, _req=None):
        if self._mt5 is not None:
            self._mt5.shutdown()
        self.symbol = None
        return {}


def serve(bridge, stdin, stdout):
    handlers = {
        "hello": bridge.hello,
        "connect": bridge.connect,
        "candles": bridge.candles,
        "shutdown": bridge.shutdown,
    }
    try:
        for line in stdin:
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
            except ValueError:
                print(f"mt5 bridge: ignoring non-JSON input: {line[:80]}", file=sys.stderr)
                continue
            cmd = req.get("cmd")
            handler = handlers.get(cmd)
            try:
                if handler is None:
                    raise BridgeError(f"unknown command: {cmd}")
                reply = {"id": req.get("id"), "ok": True, "result": handler(req)}
            except BridgeError as e:
                reply = {"id": req.get("id"), "ok": False, "error": str(e)}
            except Exception as e:  # noqa: BLE001 - every failure must reach the app as a reply
                reply = {"id": req.get("id"), "ok": False, "error": f"{type(e).__name__}: {e}"}
            stdout.write(json.dumps(reply) + "\n")
            stdout.flush()
            if cmd == "shutdown":
                return
    finally:
        bridge.shutdown()


if __name__ == "__main__":
    serve(Bridge(), sys.stdin, sys.stdout)
