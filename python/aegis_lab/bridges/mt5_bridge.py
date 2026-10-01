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
# A tick this close to a whole-hour offset is fresh enough to measure the server clock.
OFFSET_TOLERANCE = 120
# Minimum pause between attempts to re-open a lost terminal (IPC) connection.
REINIT_PAUSE = 5.0


class BridgeError(Exception):
    pass


class Bridge:
    def __init__(self, mt5=None):
        self._mt5 = mt5
        self.symbol = None
        self.offset = 0
        self.offset_known = False
        self._login = None
        self._last_reinit = 0.0

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
        self._initialize(mt5, path, kwargs)
        self._login = (path, kwargs)
        self.symbol = self._find_symbol(mt5)
        self.offset_known = False
        self._refresh_offset(mt5)
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

    @staticmethod
    def _initialize(mt5, path, kwargs):
        ok = mt5.initialize(path, **kwargs) if path else mt5.initialize(**kwargs)
        if not ok:
            code, message = mt5.last_error()
            mt5.shutdown()
            raise BridgeError(f"MT5 login failed ({code}): {message}")

    @staticmethod
    def _measure_offset(tick_time, now):
        """Server offset in whole hours, or None when the tick is too old to tell.

        Bars carry trade-server time (RoboForex runs UTC+2 in winter, UTC+3 in summer).
        A fresh tick sits within a couple of minutes of a whole-hour offset; a stale one
        (weekend, daily break) does not, and must not overwrite a known offset.
        """
        if not tick_time:
            return None
        diff = tick_time - now
        if abs(diff) > 14 * 3600:
            return None
        hours = round(diff / 3600.0)
        if abs(diff - hours * 3600) > OFFSET_TOLERANCE:
            return None
        return int(hours) * 3600

    def _refresh_offset(self, mt5):
        # Re-measured on every request: catches the DST switch and a login made while
        # the market was closed (when the first measurement is impossible).
        tick = mt5.symbol_info_tick(self.symbol)
        offset = self._measure_offset(getattr(tick, "time", 0) if tick else 0, time.time())
        if offset is not None:
            self.offset = offset
            self.offset_known = True

    def _ensure_terminal(self, mt5):
        """Fails loudly when the terminal is gone or offline instead of serving stale bars.

        A closed or crashed terminal (no IPC) is re-opened with the saved login, at most
        once every few seconds. A terminal that lost the trade server reconnects by itself,
        so the bridge only reports it.
        """
        term = mt5.terminal_info()
        if term is None and self._login is not None:
            now = time.monotonic()
            if now - self._last_reinit < REINIT_PAUSE:
                raise BridgeError("the MT5 terminal is not running; reconnecting")
            self._last_reinit = now
            path, kwargs = self._login
            mt5.shutdown()
            self._initialize(mt5, path, kwargs)
            if not mt5.symbol_select(self.symbol, True):
                raise BridgeError(f"reconnected, but {self.symbol} cannot be selected")
            term = mt5.terminal_info()
        if term is None:
            raise BridgeError("the MT5 terminal is not running")
        if not term.connected:
            raise BridgeError("the MT5 terminal lost the connection to the trade server")

    def candles(self, req):
        if not self.symbol:
            raise BridgeError("not connected")
        mt5 = self.mt5()
        self._ensure_terminal(mt5)
        self._refresh_offset(mt5)
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

    def ping(self, _req=None):
        """Cheap health check: terminal alive, online, and the clock offset in use."""
        if not self.symbol:
            raise BridgeError("not connected")
        mt5 = self.mt5()
        self._ensure_terminal(mt5)
        self._refresh_offset(mt5)
        return {"symbol": self.symbol, "server_offset": self.offset, "offset_known": self.offset_known}

    def shutdown(self, _req=None):
        if self._mt5 is not None:
            self._mt5.shutdown()
        self.symbol = None
        self._login = None
        return {}


def serve(bridge, stdin, stdout):
    handlers = {
        "hello": bridge.hello,
        "connect": bridge.connect,
        "candles": bridge.candles,
        "ping": bridge.ping,
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
