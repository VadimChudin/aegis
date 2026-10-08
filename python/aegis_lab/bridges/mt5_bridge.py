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
import hashlib
import math
import os
import sys
import tempfile
import time
from contextlib import contextmanager
from decimal import Decimal, ROUND_DOWN
from datetime import datetime, timedelta, timezone
from pathlib import Path

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
MAX_TICKS = 256
MAGIC = 26070552
TRADE_SYMBOL = "XAUUSD"


class BridgeError(Exception):
    pass


class Bridge:
    def __init__(self, mt5=None):
        self._mt5 = mt5
        self.symbol = None
        self.offset = 0
        self._ledger_path = None
        self._account_identity = None

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
        self._account_identity = None
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
        self._account_identity = (int(info.login), str(info.server)) if info else None
        identity = f"{info.login}:{info.server}" if info else str(kwargs["login"])
        root = Path(os.environ.get("AEGIS_TRADE_LEDGER_DIR", Path.home() / ".aegis" / "trade-ledger"))
        self._ledger_path = root / (hashlib.sha256(identity.encode()).hexdigest() + ".json")
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
        # The Python MT5 API returns UTC. Never infer an offset from stale ticks.
        return 0

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

    def order_book(self, _req):
        if not self.symbol:
            raise BridgeError("not connected")
        mt5 = self.mt5()
        if not mt5.market_book_add(self.symbol):
            code, message = mt5.last_error()
            raise BridgeError(f"MT5 market depth is unavailable for {self.symbol} ({code}): {message}")
        try:
            entries = mt5.market_book_get(self.symbol)
            if entries is None:
                code, message = mt5.last_error()
                raise BridgeError(f"MT5 market_book_get failed ({code}): {message}")
            if not entries:
                raise BridgeError(f"MT5 market depth is empty for {self.symbol}")

            bids, asks = [], []
            buy_types = {mt5.BOOK_TYPE_BUY}
            sell_types = {mt5.BOOK_TYPE_SELL}
            for entry in entries:
                item = entry._asdict() if hasattr(entry, "_asdict") else entry
                kind = item["type"]
                side = bids if kind in buy_types else asks if kind in sell_types else None
                if side is None:
                    continue
                price = float(item["price"])
                quantity = float(item.get("volume_dbl", item.get("volume", 0)))
                if not (price > 0 and quantity > 0 and price < float("inf") and quantity < float("inf")):
                    raise BridgeError(f"MT5 market depth contains an invalid {self.symbol} level")
                side.append({"price": price, "quantity": quantity})

            bids.sort(key=lambda level: level["price"], reverse=True)
            asks.sort(key=lambda level: level["price"])
            if not bids or not asks:
                raise BridgeError(f"MT5 market depth has no {'bid' if not bids else 'ask'} levels for {self.symbol}")
            return {
                "symbol": self.symbol,
                "timestamp": time.time_ns() // 1_000_000,
                "bids": bids,
                "asks": asks,
            }
        finally:
            mt5.market_book_release(self.symbol)

    def market_snapshot(self, _req):
        if not self.symbol:
            raise BridgeError("not connected")
        mt5 = self.mt5()
        terminal = mt5.terminal_info()
        if terminal is None or not terminal.connected:
            raise BridgeError("MT5 terminal is disconnected")

        tick = mt5.symbol_info_tick(self.symbol)
        if tick is None:
            code, message = mt5.last_error()
            raise BridgeError(f"MT5 quote is unavailable for {self.symbol} ({code}): {message}")
        quote_time = getattr(tick, "time_msc", 0) or int(getattr(tick, "time", 0)) * 1000
        if not quote_time:
            raise BridgeError(f"MT5 quote has no timestamp for {self.symbol}")
        quote = {
            "time_ms": int(quote_time) - self.offset * 1000,
            "bid": float(tick.bid),
            "ask": float(tick.ask),
        }
        if quote["time_ms"] <= 0 or not all(math.isfinite(quote[k]) and quote[k] > 0 for k in ("bid", "ask")):
            raise BridgeError(f"MT5 quote is invalid for {self.symbol}")
        if quote["ask"] < quote["bid"]:
            raise BridgeError(f"MT5 quote is crossed for {self.symbol}")

        ticks = []
        copy_ticks = getattr(mt5, "copy_ticks_from", None)
        if copy_ticks is not None:
            since = datetime.fromtimestamp(quote["time_ms"] / 1000, timezone.utc) - timedelta(seconds=10)
            try:
                history = copy_ticks(self.symbol, since, MAX_TICKS, mt5.COPY_TICKS_ALL)
            except Exception:
                history = None
            for item in history if history is not None else ():
                try:
                    if hasattr(item, "_asdict"):
                        row = item._asdict()
                    elif getattr(getattr(item, "dtype", None), "names", None):
                        row = {name: item[name] for name in item.dtype.names}
                    else:
                        row = item
                    raw_time = row.get("time_msc") or int(row.get("time", 0)) * 1000
                    tick_time = int(raw_time) - self.offset * 1000
                    if tick_time <= 0:
                        continue
                    bid, ask = float(row.get("bid", 0)), float(row.get("ask", 0))
                    last = float(row["last"]) if row.get("last") is not None else None
                    raw_volume = row.get("volume_real")
                    if raw_volume is None or not math.isfinite(float(raw_volume)) or float(raw_volume) <= 0:
                        raw_volume = row.get("volume")
                    volume = (
                        float(raw_volume)
                        if raw_volume is not None and math.isfinite(float(raw_volume)) and float(raw_volume) > 0
                        else None
                    )
                    if (
                        not math.isfinite(bid)
                        or not math.isfinite(ask)
                        or bid <= 0
                        or ask < bid
                        or (last is not None and (not math.isfinite(last) or last <= 0))
                    ):
                        continue
                    ticks.append({
                        "time_ms": tick_time,
                        "bid": bid,
                        "ask": ask,
                        "last": last,
                        "volume": volume,
                    })
                except (KeyError, TypeError, ValueError, OverflowError):
                    continue
            ticks.sort(key=lambda item: item["time_ms"])
            ticks = ticks[-MAX_TICKS:]

        book = None
        try:
            book = self.order_book({})
            book_note = ""
        except BridgeError as exc:
            book_note = str(exc)
        return {
            "symbol": self.symbol,
            "observed_at_ms": time.time_ns() // 1_000_000,
            "quote": quote,
            "ticks": ticks,
            "book": book,
            "book_note": book_note,
            "volume_kind": "mt5_ticks_not_exchange_tape",
        }

    def _ledger(self):
        if self._ledger_path is None:
            raise BridgeError("not connected")
        try:
            with self._ledger_path.open("r", encoding="utf-8") as stream:
                data = json.load(stream)
            entries = data.get("entries") if isinstance(data, dict) else None
            if (not isinstance(data, dict) or type(data.get("version")) is not int or
                    data["version"] != 1 or not isinstance(entries, dict)):
                raise ValueError("invalid ledger structure")
            for key, entry in entries.items():
                if (not isinstance(key, str) or not isinstance(entry, dict) or
                        not isinstance(entry.get("payload"), str) or
                        entry.get("status") not in ("pending", "unknown", "filled", "partial", "rejected") or
                        (entry["status"] not in ("pending", "unknown") and not isinstance(entry.get("result"), dict))):
                    raise ValueError("invalid ledger entry")
            return data
        except FileNotFoundError:
            return {"version": 1, "entries": {}}
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            raise BridgeError(f"trade ledger unavailable or corrupt ({type(exc).__name__})") from None

    def _save_ledger(self, data):
        path = self._ledger_path
        try:
            path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            if os.name != "nt":
                os.chmod(path.parent, 0o700)
            fd, temp_name = tempfile.mkstemp(prefix=".trade-", dir=path.parent)
            try:
                if os.name != "nt":
                    os.fchmod(fd, 0o600)
                with os.fdopen(fd, "w", encoding="utf-8") as stream:
                    json.dump(data, stream, separators=(",", ":"), allow_nan=False)
                    stream.flush()
                    os.fsync(stream.fileno())
                os.replace(temp_name, path)
                if os.name != "nt":
                    directory_fd = os.open(path.parent, os.O_RDONLY)
                    try:
                        os.fsync(directory_fd)
                    finally:
                        os.close(directory_fd)
            finally:
                if os.path.exists(temp_name):
                    os.unlink(temp_name)
        except (OSError, ValueError) as exc:
            raise BridgeError(f"trade ledger could not be saved ({type(exc).__name__})") from None

    @contextmanager
    def _ledger_lock(self):
        if self._ledger_path is None:
            raise BridgeError("not connected")
        lock = None
        try:
            self._ledger_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            lock = open(self._ledger_path.with_suffix(".lock"), "a+b")
            if os.name != "nt":
                os.chmod(self._ledger_path.with_suffix(".lock"), 0o600)
            if os.name == "nt":
                import msvcrt

                lock.seek(0, os.SEEK_END)
                if lock.tell() == 0:
                    lock.write(b"0")
                    lock.flush()
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_LOCK, 1)
            else:
                import fcntl

                fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        except OSError as exc:
            if lock is not None:
                lock.close()
            raise BridgeError(f"trade ledger lock unavailable ({type(exc).__name__})") from None
        try:
            yield
        finally:
            try:
                if os.name == "nt":
                    import msvcrt

                    lock.seek(0)
                    msvcrt.locking(lock.fileno(), msvcrt.LK_UNLCK, 1)
                else:
                    import fcntl

                    fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
            finally:
                lock.close()

    def _run_once(self, key, payload, operation):
        with self._ledger_lock():
            return self._run_once_locked(key, payload, operation)

    def _run_once_locked(self, key, payload, operation):
        data = self._ledger()
        digest = hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        old = data["entries"].get(key)
        if old:
            if old.get("payload") != digest:
                raise BridgeError("request id already used with different trade parameters")
            if old.get("status") == "pending":
                old["status"] = "unknown"
                old["result"] = self._trade_result(key, "unknown", -1, "previous send outcome is ambiguous")
                self._save_ledger(data)
            return old["result"]
        if key.startswith("open:") and any(
            entry.get("status") in ("pending", "unknown")
            for entry in data["entries"].values()
        ):
            raise BridgeError("an earlier trade outcome is unknown; new orders are blocked")
        data["entries"][key] = {"payload": digest, "status": "pending"}
        self._save_ledger(data)
        dispatched = False

        def mark_dispatched():
            nonlocal dispatched
            dispatched = True

        try:
            result = operation(mark_dispatched)
        except Exception:
            if dispatched:
                result = self._trade_result(key, "unknown", -1, "MT5 send outcome is ambiguous; no retry was made")
            else:
                result = self._trade_result(key, "rejected", -1, "MT5 request failed before order send")
        data["entries"][key] = {"payload": digest, "status": result["status"], "result": result}
        self._save_ledger(data)
        return result

    def _cached_result(self, key, payload):
        data = self._ledger()
        old = data["entries"].get(key)
        if old is None:
            return None
        digest = hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        if old.get("payload") != digest:
            raise BridgeError("request id already used with different trade parameters")
        if old.get("status") == "pending":
            old["status"] = "unknown"
            old["result"] = self._trade_result(key, "unknown", -1, "previous send outcome is ambiguous")
            self._save_ledger(data)
        return old.get("result")

    def _cached_close(self, ticket):
        prefix = "close:" + str(ticket) + ":"
        data = self._ledger()
        attempts = sorted((key for key in data["entries"] if key.startswith(prefix)),
                          key=lambda key: int(key[len(prefix):]))
        if not attempts:
            return prefix + "0", None
        key = attempts[-1]
        old = data["entries"][key]
        if old.get("status") == "pending":
            old["status"] = "unknown"
            old["result"] = self._trade_result(str(ticket), "unknown", -1, "previous close outcome is ambiguous")
            self._save_ledger(data)
        if old.get("status") in ("rejected", "partial"):
            return prefix + str(int(key[len(prefix):]) + 1), None
        return key, old.get("result")

    @staticmethod
    def _trade_result(request_id, status, retcode, message, **fields):
        return {"request_id": str(request_id), "status": status, "retcode": int(retcode), "message": str(message),
                "order": int(fields.get("order", 0) or 0), "deal": int(fields.get("deal", 0) or 0),
                "volume_lots": float(fields.get("volume_lots", 0) or 0), "price": float(fields.get("price", 0) or 0)}

    @staticmethod
    def _send_status(mt5, retcode):
        if retcode == mt5.TRADE_RETCODE_DONE:
            return "filled"
        if retcode == mt5.TRADE_RETCODE_DONE_PARTIAL:
            return "partial"
        if retcode in (getattr(mt5, "TRADE_RETCODE_PLACED", 10008),
                       getattr(mt5, "TRADE_RETCODE_TIMEOUT", 10012),
                       getattr(mt5, "TRADE_RETCODE_CONNECTION", 10031)):
            return "unknown"
        return "rejected"

    @staticmethod
    def _fill_mode(mt5, info):
        allowed = int(info.filling_mode)
        if allowed & mt5.SYMBOL_FILLING_IOC:
            return mt5.ORDER_FILLING_IOC
        if allowed & mt5.SYMBOL_FILLING_FOK:
            return mt5.ORDER_FILLING_FOK
        if int(info.trade_exemode) != getattr(mt5, "SYMBOL_TRADE_EXECUTION_MARKET", 2):
            return mt5.ORDER_FILLING_RETURN
        raise BridgeError("XAUUSD broker does not advertise IOC or FOK market execution")

    def _positions(self, mt5):
        positions = mt5.positions_get(symbol=self.symbol)
        if positions is None:
            raise BridgeError("MT5 could not confirm current XAUUSD positions")
        return positions

    @staticmethod
    def _ticket(req):
        ticket = req.get("ticket")
        if type(ticket) is not int or ticket <= 0:
            raise BridgeError("ticket must be a positive integer")
        return ticket

    def trading_state(self, _req):
        if not self.symbol:
            raise BridgeError("not connected")
        mt5 = self.mt5()
        account, info, terminal = mt5.account_info(), mt5.symbol_info(self.symbol), mt5.terminal_info()
        if account is None or info is None:
            raise BridgeError("MT5 account or symbol information is unavailable")
        positions = []
        for pos in self._positions(mt5):
            positions.append({"ticket": int(pos.ticket), "symbol": str(pos.symbol),
                              "side": "BUY" if pos.type == mt5.POSITION_TYPE_BUY else "SELL",
                              "volume_lots": float(pos.volume), "quantity_oz": float(pos.volume * info.trade_contract_size),
                              "entry": float(pos.price_open), "current": float(pos.price_current),
                              "stop": float(pos.sl), "target": float(pos.tp), "profit": float(pos.profit),
                              "magic": int(pos.magic), "comment": str(pos.comment)})
        margin_mode = getattr(account, "margin_mode", -1)
        return {"account": {"login": int(account.login), "server": str(account.server),
                            "currency": str(account.currency), "balance": float(account.balance),
                            "equity": float(account.equity), "profit": float(account.profit),
                            "margin": float(account.margin), "margin_free": float(account.margin_free),
                            "trade_allowed": bool(terminal and terminal.connected and terminal.trade_allowed and
                                                   account.trade_allowed and account.trade_expert),
                            "demo": bool(account.trade_mode == mt5.ACCOUNT_TRADE_MODE_DEMO),
                            "time_ms": time.time_ns() // 1_000_000},
                "positions": positions, "contract_size": float(info.trade_contract_size),
                "volume_min": float(info.volume_min), "volume_max": float(info.volume_max),
                "volume_step": float(info.volume_step),
                "hedging": margin_mode == mt5.ACCOUNT_MARGIN_MODE_RETAIL_HEDGING}

    def _trading_permission(self, mt5, require_usd=True, require_algo=True):
        terminal, account = mt5.terminal_info(), mt5.account_info()
        if terminal is None or not terminal.connected:
            raise BridgeError("MT5 terminal is disconnected")
        if account is None or not account.trade_allowed or not account.trade_expert:
            raise BridgeError("MT5 account trading is not allowed")
        if self._account_identity is None or (int(account.login), str(account.server)) != self._account_identity:
            raise BridgeError("MT5 account identity changed; reconnect before trading")
        if require_algo and not terminal.trade_allowed:
            raise BridgeError("MT5 automated trading is not enabled for this terminal/account")
        if self.symbol != TRADE_SYMBOL:
            raise BridgeError("automated trading requires the exact XAUUSD symbol")
        if require_usd and account.currency != "USD":
            raise BridgeError("automated risk sizing currently requires a USD account")
        return account

    def _check_daily_loss(self, mt5, account, positions, loss_limit):
        today = datetime.now(timezone.utc).replace(hour=0, minute=0, second=0, microsecond=0)
        history = mt5.history_deals_get(today, datetime.now(timezone.utc))
        if history is None:
            raise BridgeError("MT5 could not confirm today's XAUUSD profit and loss")
        daily_profit = sum(float(d.profit) + float(getattr(d, "commission", 0)) + float(getattr(d, "swap", 0))
                           for d in history if d.symbol == self.symbol)
        daily_profit += sum(float(p.profit) for p in positions if int(p.magic) == MAGIC)
        if loss_limit and daily_profit <= -(float(account.balance) * loss_limit / 100):
            raise BridgeError("daily loss limit reached")

    def place_order(self, req):
        mt5 = self.mt5()
        request_id = str(req.get("request_id", "")).strip()
        if not request_id or len(request_id) > 128:
            raise BridgeError("request_id must contain 1 to 128 characters")
        side = str(req.get("side", "")).upper()
        side = {"LONG": "BUY", "SHORT": "SELL"}.get(side, side)
        try:
            risk_pct, stop, target = (float(req[k]) for k in ("risk_pct", "stop", "target"))
            max_spread = float(req["max_spread"])
            slots, loss_limit = int(req["max_positions"]), float(req["daily_loss_limit_pct"])
        except (KeyError, TypeError, ValueError, OverflowError):
            raise BridgeError("invalid trade request fields") from None
        if side not in ("BUY", "SELL") or not all(math.isfinite(v) for v in (risk_pct, stop, target, max_spread, loss_limit)):
            raise BridgeError("invalid trade side or non-finite trade parameter")
        if not 0 < risk_pct <= 1 or not 0 < loss_limit <= 10 or max_spread <= 0 or not 1 <= slots <= 5:
            raise BridgeError("risk, loss limit, spread or position limit is outside safe bounds")
        if req.get("confirm_real") is not True:
            raise BridgeError("explicit confirm_real=true is required for every order")
        payload = {"side": side, "risk_pct": risk_pct, "stop": stop, "target": target,
                   "max_spread": max_spread, "max_positions": slots, "daily_loss_limit_pct": loss_limit}
        cached = self._cached_result("open:" + request_id, payload)
        if cached is not None:
            return cached
        account = self._trading_permission(mt5)
        info, tick = mt5.symbol_info(self.symbol), mt5.symbol_info_tick(self.symbol)
        if tick is None or info is None:
            raise BridgeError("XAUUSD quote or contract information is unavailable")
        contract_data = (float(info.trade_contract_size), float(info.volume_min), float(info.volume_max),
                         float(info.volume_step), float(info.point), float(account.equity), float(account.balance))
        if (not all(math.isfinite(value) and value > 0 for value in contract_data) or
                contract_data[1] > contract_data[2] or contract_data[3] > contract_data[2]):
            raise BridgeError("XAUUSD contract or account risk data is invalid")
        stamp = getattr(tick, "time_msc", 0) or int(tick.time) * 1000
        now_ms = time.time_ns() // 1_000_000
        if stamp <= 0 or now_ms - stamp > 10_000 or stamp - now_ms > 2_000:
            raise BridgeError("XAUUSD quote is stale")
        bid, ask = float(tick.bid), float(tick.ask)
        if not all(math.isfinite(v) and v > 0 for v in (bid, ask)) or ask < bid or ask - bid > max_spread:
            raise BridgeError("XAUUSD spread or quote is invalid/exceeds max_spread")
        entry = ask if side == "BUY" else bid
        distance = max(float(info.trade_stops_level) * float(info.point), float(info.point))

        def valid_stops(current_bid, current_ask):
            if side == "BUY":
                return 0 < stop <= current_bid - distance and target >= current_ask + distance
            return stop >= current_ask + distance and 0 < target <= current_bid - distance

        if not valid_stops(bid, ask):
            raise BridgeError(f"{side} stop/target violates XAUUSD stop-distance rules")
        positions = self._positions(mt5)
        hedging = getattr(account, "margin_mode", -1) == mt5.ACCOUNT_MARGIN_MODE_RETAIL_HEDGING
        if not hedging and positions:
            raise BridgeError("netting account has existing XAUUSD exposure; automated order refused")
        if any(int(p.magic) != MAGIC for p in positions):
            raise BridgeError("XAUUSD has an externally owned position; automated order refused")
        if len(positions) >= slots:
            raise BridgeError("maximum XAUUSD position count reached")
        current_notional = sum(float(p.volume) * float(info.trade_contract_size) * float(p.price_current)
                               for p in positions if int(p.magic) == MAGIC)
        exposure_room = float(account.equity) * 10 - current_notional
        if exposure_room <= 0:
            raise BridgeError("maximum aggregate XAUUSD notional exposure reached")
        self._check_daily_loss(mt5, account, positions, loss_limit)
        per_lot = mt5.order_calc_profit(mt5.ORDER_TYPE_BUY if side == "BUY" else mt5.ORDER_TYPE_SELL,
                                        self.symbol, 1.0, entry, stop)
        if per_lot is None or not math.isfinite(float(per_lot)) or float(per_lot) >= 0:
            raise BridgeError("broker could not calculate a positive stop-loss risk")
        risk_money = float(account.equity) * risk_pct / 100
        raw_lots = risk_money / abs(float(per_lot))
        cap_lots = exposure_room / (float(info.trade_contract_size) * entry)
        raw_lots = min(raw_lots, cap_lots, float(info.volume_max))
        step, minimum = Decimal(str(info.volume_step)), Decimal(str(info.volume_min))
        lots = (Decimal(str(raw_lots)) / step).to_integral_value(rounding=ROUND_DOWN) * step
        if lots < minimum:
            raise BridgeError("risk-based volume is below broker minimum; refusing to round risk up")
        aggregate_risk = 0.0
        for position in positions:
            if int(position.magic) != MAGIC:
                continue
            if not math.isfinite(float(position.sl)) or float(position.sl) <= 0:
                raise BridgeError("existing AEGIS XAUUSD position has no stop; new risk is blocked")
            position_type = mt5.ORDER_TYPE_BUY if position.type == mt5.POSITION_TYPE_BUY else mt5.ORDER_TYPE_SELL
            position_risk = mt5.order_calc_profit(position_type, self.symbol, float(position.volume),
                                                  float(position.price_open), float(position.sl))
            if position_risk is None or not math.isfinite(float(position_risk)):
                raise BridgeError("broker could not calculate existing position risk")
            aggregate_risk += max(0.0, -float(position_risk))
        new_risk = mt5.order_calc_profit(mt5.ORDER_TYPE_BUY if side == "BUY" else mt5.ORDER_TYPE_SELL,
                                         self.symbol, float(lots), entry, stop)
        if new_risk is None or not math.isfinite(float(new_risk)) or float(new_risk) >= 0:
            raise BridgeError("broker could not validate aggregate stop risk")
        aggregate_risk += abs(float(new_risk))
        if aggregate_risk > float(account.equity) * 0.05:
            raise BridgeError("aggregate AEGIS XAUUSD stop risk would exceed the 5% hard limit")
        fill = self._fill_mode(mt5, info)
        def send(mark_dispatched):
            fresh = mt5.symbol_info_tick(self.symbol)
            fresh_time = getattr(fresh, "time_msc", 0) or int(fresh.time) * 1000 if fresh else 0
            if fresh is None or time.time_ns() // 1_000_000 - fresh_time > 10_000:
                return self._trade_result(request_id, "rejected", -1, "quote became stale before order check")
            price = float(fresh.ask if side == "BUY" else fresh.bid)
            if (not math.isfinite(float(fresh.bid)) or not math.isfinite(float(fresh.ask)) or
                    float(fresh.ask) < float(fresh.bid) or float(fresh.ask) - float(fresh.bid) > max_spread or
                    not valid_stops(float(fresh.bid), float(fresh.ask))):
                return self._trade_result(request_id, "rejected", -1, "spread or stop constraints changed before send")
            order = {"action": mt5.TRADE_ACTION_DEAL, "symbol": self.symbol, "volume": float(lots),
                     "type": mt5.ORDER_TYPE_BUY if side == "BUY" else mt5.ORDER_TYPE_SELL,
                     "price": price, "sl": stop, "tp": target, "deviation": 20, "magic": MAGIC,
                     "comment": "AEGIS XAUUSD", "type_time": mt5.ORDER_TIME_GTC, "type_filling": fill}
            if int(getattr(info, "trade_exemode", -1)) == getattr(mt5, "SYMBOL_TRADE_EXECUTION_MARKET", 2):
                order.pop("price")
            check = mt5.order_check(order)
            if check is None or int(check.retcode) != 0:
                return self._trade_result(request_id, "rejected", getattr(check, "retcode", -1),
                                          getattr(check, "comment", "MT5 order_check failed"))
            account_now = self._trading_permission(mt5)
            final_tick = mt5.symbol_info_tick(self.symbol)
            final_time = getattr(final_tick, "time_msc", 0) or int(final_tick.time) * 1000 if final_tick else 0
            if final_tick is None or time.time_ns() // 1_000_000 - final_time > 10_000:
                return self._trade_result(request_id, "rejected", -1, "quote became stale after order check")
            final_bid, final_ask = float(final_tick.bid), float(final_tick.ask)
            if (not math.isfinite(final_bid) or not math.isfinite(final_ask) or final_bid <= 0 or
                    final_ask < final_bid or final_ask - final_bid > max_spread or
                    not valid_stops(final_bid, final_ask)):
                return self._trade_result(request_id, "rejected", -1, "spread or stop constraints changed after order check")
            positions_now = self._positions(mt5)
            hedging_now = getattr(account_now, "margin_mode", -1) == mt5.ACCOUNT_MARGIN_MODE_RETAIL_HEDGING
            if ((not hedging_now and positions_now) or
                    any(int(p.magic) != MAGIC for p in positions_now) or len(positions_now) >= slots):
                return self._trade_result(request_id, "rejected", -1, "XAUUSD position limits changed before send")
            notional_now = sum(float(p.volume) * float(info.trade_contract_size) * float(p.price_current)
                               for p in positions_now if int(p.magic) == MAGIC)
            if notional_now + float(lots) * float(info.trade_contract_size) * final_ask > float(account_now.equity) * 10:
                return self._trade_result(request_id, "rejected", -1, "aggregate XAUUSD notional limit changed before send")
            final_price = final_ask if side == "BUY" else final_bid
            final_risk = mt5.order_calc_profit(mt5.ORDER_TYPE_BUY if side == "BUY" else mt5.ORDER_TYPE_SELL,
                                               self.symbol, float(lots), final_price, stop)
            if (final_risk is None or not math.isfinite(float(final_risk)) or float(final_risk) >= 0 or
                    abs(float(final_risk)) > float(account_now.equity) * risk_pct / 100):
                return self._trade_result(request_id, "rejected", -1, "risk limit changed before send")
            live_risk = abs(float(final_risk))
            for position in positions_now:
                position_risk = mt5.order_calc_profit(
                    mt5.ORDER_TYPE_BUY if position.type == mt5.POSITION_TYPE_BUY else mt5.ORDER_TYPE_SELL,
                    self.symbol, float(position.volume), float(position.price_open), float(position.sl))
                if position_risk is None or not math.isfinite(float(position_risk)):
                    return self._trade_result(request_id, "rejected", -1, "existing stop risk became unavailable")
                live_risk += max(0.0, -float(position_risk))
            if live_risk > float(account_now.equity) * 0.05:
                return self._trade_result(request_id, "rejected", -1, "aggregate AEGIS stop risk exceeds the 5% hard limit")
            if "price" in order:
                order["price"] = final_price
            final_check = mt5.order_check(order)
            if final_check is None or int(final_check.retcode) != 0:
                return self._trade_result(request_id, "rejected", getattr(final_check, "retcode", -1),
                                          getattr(final_check, "comment", "MT5 final order_check failed"))
            last_tick = mt5.symbol_info_tick(self.symbol)
            last_time = getattr(last_tick, "time_msc", 0) or int(last_tick.time) * 1000 if last_tick else 0
            if (last_tick is None or time.time_ns() // 1_000_000 - last_time > 10_000 or
                    abs(float(last_tick.bid) - final_bid) > float(info.point) or
                    abs(float(last_tick.ask) - final_ask) > float(info.point)):
                return self._trade_result(request_id, "rejected", -1, "quote moved during final broker check")
            account_end = self._trading_permission(mt5)
            last_bid, last_ask = float(last_tick.bid), float(last_tick.ask)
            if not valid_stops(last_bid, last_ask) or last_ask - last_bid > max_spread:
                return self._trade_result(request_id, "rejected", -1, "trade limits changed during final broker check")
            positions_end = self._positions(mt5)
            hedging_end = getattr(account_end, "margin_mode", -1) == mt5.ACCOUNT_MARGIN_MODE_RETAIL_HEDGING
            if ((not hedging_end and positions_end) or any(int(p.magic) != MAGIC for p in positions_end) or
                    len(positions_end) >= slots):
                return self._trade_result(request_id, "rejected", -1, "XAUUSD position limits changed during final check")
            end_price = last_ask if side == "BUY" else last_bid
            end_risk = mt5.order_calc_profit(mt5.ORDER_TYPE_BUY if side == "BUY" else mt5.ORDER_TYPE_SELL,
                                             self.symbol, float(lots), end_price, stop)
            if (end_risk is None or not math.isfinite(float(end_risk)) or float(end_risk) >= 0 or
                    abs(float(end_risk)) > float(account_end.equity) * risk_pct / 100):
                return self._trade_result(request_id, "rejected", -1, "risk limit changed during final broker check")
            ending_risk = abs(float(end_risk))
            ending_notional = float(lots) * float(info.trade_contract_size) * end_price
            for position in positions_end:
                position_risk = mt5.order_calc_profit(
                    mt5.ORDER_TYPE_BUY if position.type == mt5.POSITION_TYPE_BUY else mt5.ORDER_TYPE_SELL,
                    self.symbol, float(position.volume), float(position.price_open), float(position.sl))
                if position_risk is None or not math.isfinite(float(position_risk)):
                    return self._trade_result(request_id, "rejected", -1, "existing stop risk became unavailable")
                ending_risk += max(0.0, -float(position_risk))
                ending_notional += float(position.volume) * float(info.trade_contract_size) * float(position.price_current)
            if ending_risk > float(account_end.equity) * 0.05 or ending_notional > float(account_end.equity) * 10:
                return self._trade_result(request_id, "rejected", -1, "aggregate exposure limits changed before send")
            self._check_daily_loss(mt5, account_end, positions_end, loss_limit)
            self._trading_permission(mt5)
            mark_dispatched()
            result = mt5.order_send(order)
            if result is None:
                return self._trade_result(request_id, "unknown", -1, "MT5 order_send returned no result")
            code = int(result.retcode)
            status = self._send_status(mt5, code)
            return self._trade_result(request_id, status, code, result.comment, order=result.order, deal=result.deal,
                                      volume_lots=result.volume, price=result.price)

        return self._run_once("open:" + request_id, payload, send)

    def close_position(self, req):
        ticket = self._ticket(req)
        return self.reduce_position({"ticket": ticket, "fraction": 1.0})

    def _close_position_full(self, req):
        ticket = self._ticket(req)
        close_key, cached = self._cached_close(ticket)
        if cached is not None:
            return cached
        mt5 = self.mt5()
        self._trading_permission(mt5, require_usd=False, require_algo=False)
        pos = next((p for p in self._positions(mt5) if int(p.ticket) == ticket), None)
        if pos is None or int(pos.magic) != MAGIC or pos.symbol != TRADE_SYMBOL:
            raise BridgeError("position is absent or not owned by AEGIS for exact XAUUSD")
        tick = mt5.symbol_info_tick(self.symbol)
        stamp = getattr(tick, "time_msc", 0) or int(tick.time) * 1000 if tick else 0
        if tick is None or time.time_ns() // 1_000_000 - stamp > 10_000:
            raise BridgeError("XAUUSD quote is stale or unavailable")
        side = mt5.ORDER_TYPE_SELL if pos.type == mt5.POSITION_TYPE_BUY else mt5.ORDER_TYPE_BUY
        price = float(tick.bid if side == mt5.ORDER_TYPE_SELL else tick.ask)
        fill = self._close_fill(mt5)
        payload = {"ticket": ticket, "volume": float(pos.volume)}

        def send(mark_dispatched):
            current = next((p for p in self._positions(mt5) if int(p.ticket) == ticket), None)
            if current is None or int(current.magic) != MAGIC or current.symbol != TRADE_SYMBOL:
                return self._trade_result(str(ticket), "rejected", -1, "position ownership changed before close")
            if abs(float(current.volume) - float(pos.volume)) > float(mt5.symbol_info(self.symbol).volume_step) / 2:
                return self._trade_result(str(ticket), "rejected", -1, "position volume changed before close")
            fresh_tick = mt5.symbol_info_tick(self.symbol)
            fresh_stamp = getattr(fresh_tick, "time_msc", 0) or int(fresh_tick.time) * 1000 if fresh_tick else 0
            if fresh_tick is None or time.time_ns() // 1_000_000 - fresh_stamp > 10_000:
                return self._trade_result(str(ticket), "rejected", -1, "XAUUSD quote became stale before close")
            fresh_price = float(fresh_tick.bid if side == mt5.ORDER_TYPE_SELL else fresh_tick.ask)
            order = {"action": mt5.TRADE_ACTION_DEAL, "position": ticket, "symbol": self.symbol,
                     "volume": float(pos.volume), "type": side, "price": fresh_price, "deviation": 20,
                     "magic": MAGIC, "comment": "AEGIS close", "type_time": mt5.ORDER_TIME_GTC,
                     "type_filling": fill}
            info = mt5.symbol_info(self.symbol)
            if int(getattr(info, "trade_exemode", -1)) == getattr(mt5, "SYMBOL_TRADE_EXECUTION_MARKET", 2):
                order.pop("price")
            check = mt5.order_check(order)
            if check is None or int(check.retcode) != 0:
                return self._trade_result(str(ticket), "rejected", getattr(check, "retcode", -1),
                                          getattr(check, "comment", "MT5 order_check failed"), volume_lots=pos.volume)
            verified = next((p for p in self._positions(mt5) if int(p.ticket) == ticket), None)
            if (verified is None or int(verified.magic) != MAGIC or verified.symbol != TRADE_SYMBOL or
                    abs(float(verified.volume) - float(pos.volume)) > float(info.volume_step) / 2):
                return self._trade_result(str(ticket), "rejected", -1, "position changed during broker close check")
            self._trading_permission(mt5, require_usd=False, require_algo=False)
            mark_dispatched()
            result = mt5.order_send(order)
            if result is None:
                return self._trade_result(str(ticket), "unknown", -1, "MT5 close outcome is ambiguous", volume_lots=pos.volume)
            code = int(result.retcode)
            status = self._send_status(mt5, code)
            return self._trade_result(str(ticket), status, code, result.comment, order=result.order, deal=result.deal,
                                      volume_lots=result.volume, price=result.price)

        return self._run_once(close_key, payload, send)

    def _close_fill(self, mt5):
        info = mt5.symbol_info(self.symbol)
        if info is None:
            raise BridgeError("XAUUSD symbol information is unavailable")
        return self._fill_mode(mt5, info)

    def close_all(self, _req):
        if not self.symbol:
            raise BridgeError("not connected")
        mt5 = self.mt5()
        owned = [int(p.ticket) for p in self._positions(mt5)
                 if int(p.magic) == MAGIC and p.symbol == TRADE_SYMBOL]
        outcomes = []
        for ticket in owned:
            try:
                outcomes.append(self.close_position({"ticket": ticket}))
            except BridgeError as exc:
                outcomes.append(self._trade_result(str(ticket), "rejected", -1, str(exc)))
            except Exception:
                outcomes.append(self._trade_result(str(ticket), "unknown", -1, "close outcome is ambiguous"))
        return outcomes

    def reduce_position(self, req):
        try:
            ticket, fraction = self._ticket(req), float(req["fraction"])
        except (KeyError, TypeError, ValueError, OverflowError):
            raise BridgeError("ticket and reduction fraction are required") from None
        if not math.isfinite(fraction) or not 0 < fraction <= 1:
            raise BridgeError("reduction fraction must be greater than zero and no greater than one")
        if fraction == 1:
            return self._close_position_full({"ticket": ticket})
        fraction_text = format(fraction, ".12g")
        key = f"reduce:{ticket}:{fraction_text}"
        payload = {"ticket": ticket, "fraction": fraction}
        cached = self._cached_result(key, payload)
        if cached is not None:
            return cached
        mt5 = self.mt5()
        self._trading_permission(mt5, require_usd=False, require_algo=False)
        pos = next((p for p in self._positions(mt5) if int(p.ticket) == ticket), None)
        if pos is None or int(pos.magic) != MAGIC or pos.symbol != TRADE_SYMBOL:
            raise BridgeError("position is absent or not owned by AEGIS for exact XAUUSD")
        info = mt5.symbol_info(self.symbol)
        step, minimum = Decimal(str(info.volume_step)), Decimal(str(info.volume_min))
        amount = (Decimal(str(pos.volume)) * Decimal(str(fraction)) / step).to_integral_value(rounding=ROUND_DOWN) * step
        remainder = Decimal(str(pos.volume)) - amount
        if remainder < minimum:
            return self._close_position_full({"ticket": ticket})
        if amount < minimum:
            raise BridgeError("reduction amount is below broker minimum volume")
        tick = mt5.symbol_info_tick(self.symbol)
        stamp = getattr(tick, "time_msc", 0) or int(tick.time) * 1000 if tick else 0
        if tick is None or time.time_ns() // 1_000_000 - stamp > 10_000:
            raise BridgeError("XAUUSD quote is stale")
        side = mt5.ORDER_TYPE_SELL if pos.type == mt5.POSITION_TYPE_BUY else mt5.ORDER_TYPE_BUY
        price = float(tick.bid if side == mt5.ORDER_TYPE_SELL else tick.ask)
        fill = self._close_fill(mt5)

        def send(mark_dispatched):
            current = next((p for p in self._positions(mt5) if int(p.ticket) == ticket), None)
            if current is None or int(current.magic) != MAGIC or current.symbol != TRADE_SYMBOL:
                return self._trade_result(str(ticket), "rejected", -1, "position ownership changed before reduce")
            if abs(float(current.volume) - float(pos.volume)) > float(info.volume_step) / 2:
                return self._trade_result(str(ticket), "rejected", -1, "position volume changed before reduce")
            fresh = mt5.symbol_info_tick(self.symbol)
            fresh_stamp = getattr(fresh, "time_msc", 0) or int(fresh.time) * 1000 if fresh else 0
            if fresh is None or time.time_ns() // 1_000_000 - fresh_stamp > 10_000:
                return self._trade_result(str(ticket), "rejected", -1, "XAUUSD quote became stale before reduce")
            order = {"action": mt5.TRADE_ACTION_DEAL, "position": ticket, "symbol": self.symbol,
                     "volume": float(amount), "type": side, "price": price, "deviation": 20,
                     "magic": MAGIC, "comment": "AEGIS reduce", "type_time": mt5.ORDER_TIME_GTC,
                     "type_filling": fill}
            if int(getattr(info, "trade_exemode", -1)) == getattr(mt5, "SYMBOL_TRADE_EXECUTION_MARKET", 2):
                order.pop("price")
            check = mt5.order_check(order)
            if check is None or int(check.retcode) != 0:
                return self._trade_result(str(ticket), "rejected", getattr(check, "retcode", -1),
                                          getattr(check, "comment", "MT5 reduce check failed"), volume_lots=float(amount))
            self._trading_permission(mt5, require_usd=False, require_algo=False)
            after_check = mt5.symbol_info_tick(self.symbol)
            after_stamp = getattr(after_check, "time_msc", 0) or int(after_check.time) * 1000 if after_check else 0
            if after_check is None or time.time_ns() // 1_000_000 - after_stamp > 10_000:
                return self._trade_result(str(ticket), "rejected", -1, "XAUUSD quote became stale after reduce check")
            verified = next((p for p in self._positions(mt5) if int(p.ticket) == ticket), None)
            if (verified is None or int(verified.magic) != MAGIC or verified.symbol != TRADE_SYMBOL or
                    abs(float(verified.volume) - float(pos.volume)) > float(info.volume_step) / 2):
                return self._trade_result(str(ticket), "rejected", -1, "position changed during broker reduce check")
            self._trading_permission(mt5, require_usd=False, require_algo=False)
            mark_dispatched()
            result = mt5.order_send(order)
            if result is None:
                return self._trade_result(str(ticket), "unknown", -1, "MT5 reduce outcome is ambiguous",
                                          volume_lots=float(amount))
            code = int(result.retcode)
            status = self._send_status(mt5, code)
            return self._trade_result(str(ticket), status, code, result.comment, order=result.order, deal=result.deal,
                                      volume_lots=result.volume, price=result.price)

        return self._run_once(key, payload, send)

    def modify_stop(self, req):
        try:
            ticket, stop = self._ticket(req), float(req["stop"])
            target_value = req.get("target")
            target_value = float(target_value) if target_value is not None else None
        except (KeyError, TypeError, ValueError, OverflowError):
            raise BridgeError("ticket and stop price are required") from None
        if not math.isfinite(stop) or (target_value is not None and not math.isfinite(target_value)):
            raise BridgeError("stop or target price is not finite")
        mt5 = self.mt5()
        self._trading_permission(mt5, require_usd=False, require_algo=False)
        pos = next((p for p in self._positions(mt5) if int(p.ticket) == ticket), None)
        if pos is None or int(pos.magic) != MAGIC or pos.symbol != TRADE_SYMBOL:
            raise BridgeError("position is absent or not owned by AEGIS for exact XAUUSD")
        target = float(pos.tp) if target_value is None else target_value
        key = "modify:" + hashlib.sha256(f"{ticket}:{stop:.12g}:{target:.12g}".encode()).hexdigest()
        payload = {"ticket": ticket, "stop": stop, "target": target}
        cached = self._cached_result(key, payload)
        if cached is not None:
            return cached
        info, tick = mt5.symbol_info(self.symbol), mt5.symbol_info_tick(self.symbol)
        stamp = getattr(tick, "time_msc", 0) or int(tick.time) * 1000 if tick else 0
        if tick is None or time.time_ns() // 1_000_000 - stamp > 10_000:
            raise BridgeError("XAUUSD quote is stale")
        bid, ask = float(tick.bid), float(tick.ask)
        distance = max(float(info.trade_stops_level) * float(info.point), float(info.point))
        freeze_distance = float(getattr(info, "trade_freeze_level", 0)) * float(info.point)
        if pos.type == mt5.POSITION_TYPE_BUY:
            valid = 0 < stop <= bid - distance and (target == 0 or target >= ask + distance)
            tighter = float(pos.sl) <= 0 or stop >= float(pos.sl)
            frozen = (abs(bid - stop) <= freeze_distance or
                      (float(pos.sl) > 0 and abs(bid - float(pos.sl)) <= freeze_distance) or
                      (target > 0 and abs(ask - target) <= freeze_distance) or
                      (float(pos.tp) > 0 and abs(ask - float(pos.tp)) <= freeze_distance))
        else:
            valid = stop >= ask + distance and (target == 0 or 0 < target <= bid - distance)
            tighter = float(pos.sl) <= 0 or stop <= float(pos.sl)
            frozen = (abs(ask - stop) <= freeze_distance or
                      (float(pos.sl) > 0 and abs(ask - float(pos.sl)) <= freeze_distance) or
                      (target > 0 and abs(bid - target) <= freeze_distance) or
                      (float(pos.tp) > 0 and abs(bid - float(pos.tp)) <= freeze_distance))
        if not valid or not tighter or frozen:
            raise BridgeError("stop/target violates XAUUSD stop-distance rules")

        def send(mark_dispatched):
            current = next((p for p in self._positions(mt5) if int(p.ticket) == ticket), None)
            if current is None or int(current.magic) != MAGIC or current.symbol != TRADE_SYMBOL:
                return self._trade_result(str(ticket), "rejected", -1, "position ownership changed before stop update")
            if float(current.sl) > 0 and ((current.type == mt5.POSITION_TYPE_BUY and stop < current.sl) or
                                         (current.type == mt5.POSITION_TYPE_SELL and stop > current.sl)):
                return self._trade_result(str(ticket), "rejected", -1, "stop may not be widened")
            fresh = mt5.symbol_info_tick(self.symbol)
            fresh_stamp = getattr(fresh, "time_msc", 0) or int(fresh.time) * 1000 if fresh else 0
            if fresh is None or time.time_ns() // 1_000_000 - fresh_stamp > 10_000:
                return self._trade_result(str(ticket), "rejected", -1, "XAUUSD quote became stale before stop update")
            order = {"action": mt5.TRADE_ACTION_SLTP, "position": ticket, "symbol": self.symbol,
                     "sl": stop, "tp": target}
            check = mt5.order_check(order)
            if check is None or int(check.retcode) != 0:
                return self._trade_result(str(ticket), "rejected", getattr(check, "retcode", -1),
                                          getattr(check, "comment", "MT5 stop check failed"))
            self._trading_permission(mt5, require_usd=False, require_algo=False)
            mark_dispatched()
            result = mt5.order_send(order)
            if result is None:
                return self._trade_result(str(ticket), "unknown", -1, "MT5 stop update outcome is ambiguous")
            code = int(result.retcode)
            status = self._send_status(mt5, code)
            return self._trade_result(str(ticket), status, code, result.comment, order=result.order, deal=result.deal,
                                      volume_lots=result.volume, price=result.price)

        return self._run_once(key, payload, send)

    def shutdown(self, _req=None):
        if self._mt5 is not None:
            self._mt5.shutdown()
        self.symbol = None
        self._account_identity = None
        return {}


def serve(bridge, stdin, stdout):
    handlers = {
        "hello": bridge.hello,
        "connect": bridge.connect,
        "candles": bridge.candles,
        "order_book": bridge.order_book,
        "market_snapshot": bridge.market_snapshot,
        "trading_state": bridge.trading_state,
        "place_order": bridge.place_order,
        "close_position": bridge.close_position,
        "close_all": bridge.close_all,
        "reduce_position": bridge.reduce_position,
        "modify_stop": bridge.modify_stop,
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
                print("mt5 bridge: ignoring non-JSON input", file=sys.stderr)
                continue
            request_id = req.get("id") if isinstance(req, dict) else None
            cmd = None
            try:
                if not isinstance(req, dict):
                    raise BridgeError("request must be a JSON object")
                cmd = req.get("cmd")
                if not isinstance(cmd, str):
                    raise BridgeError("command must be a string")
                handler = handlers.get(cmd)
                if handler is None:
                    raise BridgeError(f"unknown command: {cmd}")
                reply = {"id": request_id, "ok": True, "result": handler(req)}
            except BridgeError as e:
                reply = {"id": request_id, "ok": False, "error": str(e)}
            except Exception as e:  # noqa: BLE001 - every failure must reach the app as a reply
                reply = {"id": request_id, "ok": False, "error": f"{type(e).__name__}: {e}"}
            stdout.write(json.dumps(reply) + "\n")
            stdout.flush()
            if cmd == "shutdown":
                return
    finally:
        bridge.shutdown()


if __name__ == "__main__":
    serve(Bridge(), sys.stdin, sys.stdout)
