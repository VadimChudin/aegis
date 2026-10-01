"""Stand-in for the Windows-only MetaTrader5 package, used by the tests.

Login 1 with password "good" succeeds; anything else fails like a real terminal.
The server clock runs 3 hours ahead of UTC, like RoboForex in summer.
"""

import time
from types import SimpleNamespace

__version__ = "5.0.5120"

TIMEFRAME_M1, TIMEFRAME_M5, TIMEFRAME_M15, TIMEFRAME_H1, TIMEFRAME_H4, TIMEFRAME_D1 = 1, 5, 15, 16385, 16388, 16408
BOOK_TYPE_BUY, BOOK_TYPE_SELL, BOOK_TYPE_BUY_MARKET, BOOK_TYPE_SELL_MARKET = 1, 2, 3, 4
SERVER_OFFSET = 3 * 3600

_state = {
    "logged_in": False,
    "error": (1, "Success"),
    "book_supported": True,
    "book": None,
    "book_releases": 0,
    "running": True,
    "online": True,
    "tick_age": 0,
    "inits": 0,
}


def initialize(path=None, login=None, password=None, server=None, timeout=None):
    _state["inits"] += 1
    if login == 1 and password == "good":
        _state["logged_in"] = True
        _state["running"] = True
        return True
    _state["error"] = (-6, "Terminal: Authorization failed")
    return False


def shutdown():
    _state["logged_in"] = False


def last_error():
    return _state["error"]


def account_info():
    return SimpleNamespace(login=1, server="RoboForex-ECN", currency="USD", balance=1000.0, trade_allowed=True,
                           trade_expert=True)


def terminal_info():
    if not _state["running"]:
        return None
    return SimpleNamespace(connected=_state["online"], trade_allowed=False, company="RoboForex Ltd", build=5120)


def symbols_get(group=None):
    return (SimpleNamespace(name="XAUUSD.r"),)


def symbol_info(name):
    return SimpleNamespace(name=name, spread=30) if name == "XAUUSD.r" else None


def symbol_select(name, enable):
    return name == "XAUUSD.r"


def symbol_info_tick(name):
    return SimpleNamespace(time=int(time.time()) + SERVER_OFFSET - _state["tick_age"])


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


def market_book_add(symbol):
    if not _state["logged_in"] or not _state["book_supported"] or symbol != "XAUUSD.r":
        _state["error"] = (-1, "Market depth is not supported")
        return False
    return True


def market_book_get(symbol):
    if _state["book"] is not None:
        return _state["book"]
    from collections import namedtuple

    BookInfo = namedtuple("BookInfo", "type price volume volume_dbl")
    return (
        BookInfo(BOOK_TYPE_BUY, 4293.0, 2, 2.0),
        BookInfo(BOOK_TYPE_BUY, 4292.0, 3, 3.5),
        BookInfo(BOOK_TYPE_SELL, 4294.0, 4, 4.0),
        BookInfo(BOOK_TYPE_SELL, 4295.0, 5, 5.0),
    )


def market_book_release(symbol):
    _state["book_releases"] += 1
    return True
