"""Stand-in for the Windows-only MetaTrader5 package, used by the tests.

Login 1 with password "good" succeeds; anything else fails like a real terminal.
The server clock runs 3 hours ahead of UTC, like RoboForex in summer.
"""

import time
from types import SimpleNamespace

TIMEFRAME_M1, TIMEFRAME_M5, TIMEFRAME_M15, TIMEFRAME_H1, TIMEFRAME_H4, TIMEFRAME_D1 = 1, 5, 15, 16385, 16388, 16408
SERVER_OFFSET = 3 * 3600

_state = {"logged_in": False, "error": (1, "Success")}


def initialize(path=None, login=None, password=None, server=None, timeout=None):
    if login == 1 and password == "good":
        _state["logged_in"] = True
        return True
    _state["error"] = (-6, "Terminal: Authorization failed")
    return False


def shutdown():
    _state["logged_in"] = False


def last_error():
    return _state["error"]


def account_info():
    return SimpleNamespace(login=1, server="RoboForex-ECN", currency="USD", balance=1000.0)


def symbols_get(group=None):
    return (SimpleNamespace(name="XAUUSD.r"),)


def symbol_info(name):
    return SimpleNamespace(name=name) if name == "XAUUSD.r" else None


def symbol_select(name, enable):
    return name == "XAUUSD.r"


def symbol_info_tick(name):
    return SimpleNamespace(time=int(time.time()) + SERVER_OFFSET)


def copy_rates_from_pos(symbol, timeframe, start, count):
    if not _state["logged_in"]:
        _state["error"] = (-10004, "No IPC connection")
        return None
    base = 1_790_208_000 + SERVER_OFFSET
    step = {TIMEFRAME_M15: 900}.get(timeframe, 60)
    return [
        {"time": base + i * step, "open": 4290.0 + i, "high": 4292.0 + i, "low": 4289.0 + i, "close": 4291.0 + i,
         "tick_volume": 100 + i, "spread": 30, "real_volume": 0}
        for i in range(count)
    ]
