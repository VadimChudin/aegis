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
POSITION_TYPE_BUY, POSITION_TYPE_SELL = 0, 1
ORDER_TYPE_BUY, ORDER_TYPE_SELL = 0, 1
TRADE_ACTION_DEAL, TRADE_ACTION_SLTP, ORDER_TIME_GTC = 1, 6, 0
ORDER_FILLING_FOK, ORDER_FILLING_IOC, ORDER_FILLING_RETURN = 0, 1, 2
SYMBOL_FILLING_FOK, SYMBOL_FILLING_IOC = 1, 2
SYMBOL_TRADE_EXECUTION_INSTANT, SYMBOL_TRADE_EXECUTION_MARKET = 0, 2
TRADE_RETCODE_PLACED = 10008
TRADE_RETCODE_DONE, TRADE_RETCODE_DONE_PARTIAL = 10009, 10010
TRADE_RETCODE_TIMEOUT, TRADE_RETCODE_CONNECTION = 10012, 10031
ACCOUNT_TRADE_MODE_DEMO = 0
ACCOUNT_MARGIN_MODE_RETAIL_HEDGING = 2

_state = {
    "logged_in": False,
    "error": (1, "Success"),
    "book_supported": True,
    "book": None,
    "book_releases": 0,
    "connected": True,
    "quote_age_seconds": 0,
    "trade_enabled": False,
    "demo": True,
    "hedging": True,
    "positions": [],
    "deals": [],
    "send_mode": "filled",
    "order_checks": 0,
    "order_sends": 0,
    "order_requests": [],
    "filling_mode": SYMBOL_FILLING_FOK | SYMBOL_FILLING_IOC,
    "trade_exemode": SYMBOL_TRADE_EXECUTION_MARKET,
    "close_fail_tickets": set(),
    "account_login": 1,
    "live_symbol": False,
    "balance": 1000.0,
    "freeze_level": 0,
}


def initialize(path=None, login=None, password=None, server=None, timeout=None):
    if login in (1, 2) and password == "good":
        _state["logged_in"] = True
        _state["account_login"] = login
        _state["live_symbol"] = login == 2
        _state["trade_enabled"] = login == 2
        return True
    _state["error"] = (-6, "Terminal: Authorization failed")
    return False


def shutdown():
    _state["logged_in"] = False


def last_error():
    return _state["error"]


def account_info():
    profit = sum(p.profit for p in _state["positions"])
    balance = float(_state["balance"])
    return SimpleNamespace(login=_state["account_login"], server="RoboForex-ECN", currency="USD", balance=balance, equity=balance + profit,
                           profit=profit, margin=0.0, margin_free=balance + profit, trade_allowed=True,
                           trade_expert=True, trade_mode=ACCOUNT_TRADE_MODE_DEMO if _state["demo"] else 1,
                           margin_mode=ACCOUNT_MARGIN_MODE_RETAIL_HEDGING if _state["hedging"] else 0)


def terminal_info():
    return SimpleNamespace(connected=_state["connected"], trade_allowed=_state["trade_enabled"],
                           company="RoboForex Ltd", build=5120)


def symbols_get(group=None):
    live = _state["live_symbol"] or os.getenv("AEGIS_TEST_LIVE_MT5") == "1"
    return (SimpleNamespace(name="XAUUSD" if live else "XAUUSD.r"),)


def symbol_info(name):
    expected = "XAUUSD" if _state["live_symbol"] or os.getenv("AEGIS_TEST_LIVE_MT5") == "1" else "XAUUSD.r"
    return SimpleNamespace(name=name, spread=30, trade_contract_size=100.0, volume_min=0.01,
                           volume_max=100.0, volume_step=0.01, trade_stops_level=10, point=0.01,
                           trade_freeze_level=_state["freeze_level"],
                           filling_mode=_state["filling_mode"],
                           trade_exemode=_state["trade_exemode"]) if name == expected else None


def positions_get(symbol=None):
    if symbol is None:
        return tuple(_state["positions"])
    return tuple(p for p in _state["positions"] if p.symbol == symbol)


def history_deals_get(start, end):
    return tuple(_state["deals"])


def order_calc_profit(order_type, symbol, volume, price_open, price_close):
    direction = 1 if order_type == ORDER_TYPE_BUY else -1
    return (price_close - price_open) * 100.0 * volume * direction


def order_check(request):
    _state["order_checks"] += 1
    if request.get("position") in _state["close_fail_tickets"]:
        return SimpleNamespace(retcode=10016, comment="close check rejected")
    return SimpleNamespace(retcode=0, comment="done")


def order_send(request):
    _state["order_sends"] += 1
    mode = _state["send_mode"]
    _state["order_requests"].append(dict(request))
    if mode == "unknown":
        return None
    placed = mode == "placed"
    partial = mode == "partial"
    volume = request.get("volume", 0) / 2 if partial else request.get("volume", 0)
    order, deal = _state["order_sends"], _state["order_sends"] + 1000
    if placed:
        pass
    elif "position" in request:
        position = next((p for p in _state["positions"] if p.ticket == request["position"]), None)
        if position and request["action"] == TRADE_ACTION_SLTP:
            position.sl, position.tp = request["sl"], request["tp"]
        elif position and volume >= position.volume:
            _state["positions"].remove(position)
        elif position:
            position.volume -= volume
    elif request["action"] == TRADE_ACTION_DEAL:
        price = request.get("price", 4294.0 if request["type"] == ORDER_TYPE_BUY else 4293.0)
        _state["positions"].append(SimpleNamespace(ticket=order, symbol=request["symbol"],
            type=request["type"], volume=volume, price_open=price, price_current=price,
            sl=request["sl"], tp=request["tp"], profit=0.0, magic=request["magic"], comment=request["comment"]))
    default_price = 0.0 if request["action"] == TRADE_ACTION_SLTP else (
        4294.0 if request["type"] == ORDER_TYPE_BUY else 4293.0)
    retcode = TRADE_RETCODE_PLACED if placed else (
        TRADE_RETCODE_TIMEOUT if mode == "timeout" else (
            TRADE_RETCODE_DONE_PARTIAL if partial else TRADE_RETCODE_DONE))
    return SimpleNamespace(retcode=retcode,
                           comment=mode, order=order, deal=deal, volume=volume,
                           price=request.get("price", default_price))


def symbol_select(name, enable):
    expected = "XAUUSD" if _state["live_symbol"] or os.getenv("AEGIS_TEST_LIVE_MT5") == "1" else "XAUUSD.r"
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
    expected = "XAUUSD" if _state["live_symbol"] or os.getenv("AEGIS_TEST_LIVE_MT5") == "1" else "XAUUSD.r"
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
