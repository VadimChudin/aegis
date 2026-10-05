"""Stand-in for the Windows-only MetaTrader5 package, used by the tests.

Login 1 with password "good" succeeds; anything else fails like a real terminal.
Timestamps are UTC, matching the MetaTrader5 Python API.
"""

import time
import os
from types import SimpleNamespace

__version__ = "5.0.5120"

TIMEFRAME_M1, TIMEFRAME_M5, TIMEFRAME_M15, TIMEFRAME_H1, TIMEFRAME_H4, TIMEFRAME_D1 = 1, 5, 15, 16385, 16388, 16408
BOOK_TYPE_BUY, BOOK_TYPE_SELL, BOOK_TYPE_BUY_MARKET, BOOK_TYPE_SELL_MARKET = 1, 2, 3, 4
COPY_TICKS_ALL = -1
SERVER_OFFSET = 0

_state = {
    "logged_in": False,
    "error": (1, "Success"),
    "book_supported": True,
    "book": None,
    "book_releases": 0,
    "connected": True,
    "quote_age_seconds": 0,
}


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
    return SimpleNamespace(login=1, server="RoboForex-ECN", currency="USD", balance=1000.0, trade_allowed=True,
                           trade_expert=True)


def terminal_info():
    return SimpleNamespace(connected=_state["connected"], trade_allowed=False, company="RoboForex Ltd", build=5120)


def symbols_get(group=None):
    return (SimpleNamespace(name="XAUUSD" if os.getenv("AEGIS_TEST_LIVE_MT5") == "1" else "XAUUSD.r"),)


def symbol_info(name):
    expected = "XAUUSD" if os.getenv("AEGIS_TEST_LIVE_MT5") == "1" else "XAUUSD.r"
    return SimpleNamespace(name=name, spread=30) if name == expected else None


def symbol_select(name, enable):
    expected = "XAUUSD" if os.getenv("AEGIS_TEST_LIVE_MT5") == "1" else "XAUUSD.r"
    return name == expected


def symbol_info_tick(name):
    now_ms = int((time.time() - _state["quote_age_seconds"]) * 1000) + SERVER_OFFSET * 1000
    return SimpleNamespace(time=now_ms // 1000, time_msc=now_ms, bid=4293.0, ask=4294.0, last=4293.5, volume=2,
                           volume_real=2.5)


def copy_ticks_from(symbol, date_from, count, flags):
    now_ms = int(time.time() * 1000) + SERVER_OFFSET * 1000
    return [
        {"time": (now_ms - (5 - i) * 1000) // 1000, "time_msc": now_ms - (5 - i) * 1000,
         "bid": 4293.0 + i * 0.1, "ask": 4294.0 + i * 0.1, "last": 4293.5 + i * 0.1,
         "volume": 1, "volume_real": 1.5}
        for i in range(min(count, 6))
    ]


def copy_rates_from_pos(symbol, timeframe, start, count):
    if not _state["logged_in"]:
        _state["error"] = (-10004, "No IPC connection")
        return None
    if os.getenv("AEGIS_TEST_LIVE_MT5") == "1":
        step = {TIMEFRAME_M1: 60, TIMEFRAME_M5: 300, TIMEFRAME_M15: 900, TIMEFRAME_H1: 3600,
                TIMEFRAME_H4: 14400, TIMEFRAME_D1: 86400}.get(timeframe, 60)
        base = int(time.time()) - int(time.time()) % step - (count - 1) * step + SERVER_OFFSET
    else:
        base = 1_790_208_000 + SERVER_OFFSET
        step = {TIMEFRAME_M15: 900}.get(timeframe, 60)
    return [
        {"time": base + i * step, "open": 4290.0 + i, "high": 4292.0 + i, "low": 4289.0 + i, "close": 4291.0 + i,
         "tick_volume": 100 + i, "spread": 30, "real_volume": 0}
        for i in range(count)
    ]


def market_book_add(symbol):
    expected = "XAUUSD" if os.getenv("AEGIS_TEST_LIVE_MT5") == "1" else "XAUUSD.r"
    if not _state["logged_in"] or not _state["book_supported"] or os.getenv("AEGIS_TEST_NO_BOOK") == "1" or symbol != expected:
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
