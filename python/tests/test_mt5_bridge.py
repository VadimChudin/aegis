import io
import json
import os
import sys
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "fake_mt5"))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import MetaTrader5 as fake  # noqa: E402

from aegis_lab.bridges.mt5_bridge import Bridge, serve  # noqa: E402


def run(*requests):
    out = io.StringIO()
    stdin = io.StringIO("".join(json.dumps(r) + "\n" for r in requests))
    serve(Bridge(fake), stdin, out)
    return [json.loads(line) for line in out.getvalue().splitlines()]


class BridgeTest(unittest.TestCase):
    def test_login_symbol_and_utc_candles(self):
        hello, conn, bars = run(
            {"id": 1, "cmd": "hello"},
            {"id": 2, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
            {"id": 3, "cmd": "candles", "timeframe": "15m", "limit": 3},
        )
        self.assertEqual(hello["result"]["protocol"], 1)
        self.assertEqual(hello["result"]["mt5_package"], "5.0.5120")
        self.assertTrue(conn["ok"], conn)
        self.assertEqual(conn["result"]["symbol"], "XAUUSD.r")
        self.assertEqual(conn["result"]["server_offset"], 0)
        self.assertEqual([b["time"] for b in bars["result"]], [1_790_208_000, 1_790_208_900, 1_790_209_800])
        self.assertEqual(bars["result"][0]["volume"], 100.0)
        checks = {c["id"]: c["status"] for c in conn["result"]["checks"]}
        self.assertEqual(checks, {"terminal": "ok", "algo": "warn", "trading": "ok", "symbol": "ok", "balance": "ok"})

    def test_order_book_uses_terminal_dom_and_releases_subscription(self):
        released_before = fake._state["book_releases"]
        conn, book = run(
            {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
            {"id": 2, "cmd": "order_book"},
        )
        self.assertTrue(conn["ok"], conn)
        self.assertTrue(book["ok"], book)
        snapshot = book["result"]
        self.assertEqual(snapshot["symbol"], "XAUUSD.r")
        self.assertGreater(snapshot["timestamp"], 0)
        self.assertEqual([level["price"] for level in snapshot["bids"]], [4293.0, 4292.0])
        self.assertEqual(snapshot["bids"][1]["quantity"], 3.5)
        self.assertEqual([level["price"] for level in snapshot["asks"]], [4294.0, 4295.0])
        self.assertEqual(fake._state["book_releases"], released_before + 1)

    def test_market_snapshot_contains_recent_quote_ticks_and_optional_depth(self):
        conn, snapshot = run(
            {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
            {"id": 2, "cmd": "market_snapshot"},
        )
        self.assertTrue(conn["ok"], conn)
        self.assertTrue(snapshot["ok"], snapshot)
        data = snapshot["result"]
        self.assertEqual(data["symbol"], "XAUUSD.r")
        self.assertEqual(data["volume_kind"], "mt5_ticks_not_exchange_tape")
        self.assertNotIn("account", data)
        self.assertNotIn("password", data)
        self.assertEqual(data["quote"]["bid"], 4293.0)
        self.assertEqual(data["quote"]["ask"], 4294.0)
        self.assertGreater(data["quote"]["time_ms"], 0)
        self.assertEqual(len(data["ticks"]), 6)
        self.assertEqual(data["ticks"][0]["volume"], 1.5)
        self.assertIsNotNone(data["book"])
        self.assertEqual(fake._state["book_releases"] > 0, True)

    def test_market_snapshot_keeps_quote_when_depth_is_unavailable(self):
        fake._state["book_supported"] = False
        try:
            _, snapshot = run(
                {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
                {"id": 2, "cmd": "market_snapshot"},
            )
            self.assertTrue(snapshot["ok"], snapshot)
            self.assertIsNone(snapshot["result"]["book"])
            self.assertIn("market depth is unavailable", snapshot["result"]["book_note"])
            self.assertEqual(snapshot["result"]["quote"]["bid"], 4293.0)
        finally:
            fake._state["book_supported"] = True

    def test_market_snapshot_fails_when_terminal_is_disconnected(self):
        fake._state["connected"] = False
        try:
            _, snapshot = run(
                {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
                {"id": 2, "cmd": "market_snapshot"},
            )
            self.assertFalse(snapshot["ok"])
            self.assertIn("disconnected", snapshot["error"])
        finally:
            fake._state["connected"] = True

    def test_market_snapshot_works_without_optional_tick_history_api(self):
        saved = fake.copy_ticks_from
        del fake.copy_ticks_from
        try:
            _, snapshot = run(
                {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
                {"id": 2, "cmd": "market_snapshot"},
            )
            self.assertTrue(snapshot["ok"], snapshot)
            self.assertEqual(snapshot["result"]["ticks"], [])
            self.assertEqual(snapshot["result"]["quote"]["bid"], 4293.0)
        finally:
            fake.copy_ticks_from = saved

    def test_market_snapshot_preserves_stale_quote_timestamp(self):
        fake._state["quote_age_seconds"] = 20
        try:
            _, snapshot = run(
                {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
                {"id": 2, "cmd": "market_snapshot"},
            )
            self.assertTrue(snapshot["ok"], snapshot)
            self.assertLess(time.time_ns() // 1_000_000 - snapshot["result"]["quote"]["time_ms"], 30_000)
            self.assertGreater(time.time_ns() // 1_000_000 - snapshot["result"]["quote"]["time_ms"], 10_000)
        finally:
            fake._state["quote_age_seconds"] = 0

    def test_live_fake_uses_plain_symbol_and_fresh_candles_on_all_timeframes(self):
        previous = os.environ.get("AEGIS_TEST_LIVE_MT5")
        os.environ["AEGIS_TEST_LIVE_MT5"] = "1"
        try:
            replies = run(
                {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
                *({"id": i + 2, "cmd": "candles", "timeframe": tf, "limit": 2}
                  for i, tf in enumerate(("1m", "5m", "15m", "1h", "4h", "1d"))),
            )
            conn, bars = replies[0], replies[1:]
            self.assertTrue(conn["ok"], conn)
            self.assertEqual(conn["result"]["symbol"], "XAUUSD")
            steps = (60, 300, 900, 3600, 14400, 86400)
            now = int(time.time())
            for response, step in zip(bars, steps):
                self.assertTrue(response["ok"], response)
                self.assertEqual(len(response["result"]), 2)
                self.assertLessEqual(now - response["result"][-1]["time"], step)
        finally:
            if previous is None:
                os.environ.pop("AEGIS_TEST_LIVE_MT5", None)
            else:
                os.environ["AEGIS_TEST_LIVE_MT5"] = previous

    def test_hours_old_quote_is_not_relabelled_as_current(self):
        fake._state["quote_age_seconds"] = 3 * 3600
        try:
            conn, snapshot = run(
                {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
                {"id": 2, "cmd": "market_snapshot"},
            )
            self.assertEqual(conn["result"]["server_offset"], 0)
            self.assertTrue(snapshot["ok"], snapshot)
            self.assertGreater(time.time_ns() // 1_000_000 - snapshot["result"]["quote"]["time_ms"], 10_000_000)
        finally:
            fake._state["quote_age_seconds"] = 0

    def test_unsupported_mt5_depth_returns_error_not_synthetic_data(self):
        fake._state["book_supported"] = False
        try:
            conn, book = run(
                {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
                {"id": 2, "cmd": "order_book"},
            )
            self.assertTrue(conn["ok"], conn)
            self.assertFalse(book["ok"])
            self.assertIn("market depth is unavailable", book["error"])
        finally:
            fake._state["book_supported"] = True

    def test_market_orders_are_not_resting_densities(self):
        fake._state["book"] = (
            {"type": fake.BOOK_TYPE_BUY, "price": 4293.0, "volume": 2},
            {"type": fake.BOOK_TYPE_SELL, "price": 4294.0, "volume": 3},
            {"type": fake.BOOK_TYPE_BUY_MARKET, "price": 0.0, "volume": 100},
            {"type": fake.BOOK_TYPE_SELL_MARKET, "price": 0.0, "volume": 100},
        )
        try:
            _, book = run(
                {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
                {"id": 2, "cmd": "order_book"},
            )
            self.assertTrue(book["ok"], book)
            self.assertEqual(len(book["result"]["bids"]), 1)
            self.assertEqual(len(book["result"]["asks"]), 1)
        finally:
            fake._state["book"] = None

    def test_empty_mt5_dom_returns_an_explicit_error(self):
        fake._state["book"] = ()
        try:
            conn, book = run(
                {"id": 1, "cmd": "connect", "login": 1, "password": "good", "server": "RoboForex-ECN"},
                {"id": 2, "cmd": "order_book"},
            )
            self.assertTrue(conn["ok"], conn)
            self.assertFalse(book["ok"])
            self.assertIn("market depth is empty", book["error"])
        finally:
            fake._state["book"] = None

    def test_failed_login_is_a_reply_not_a_crash(self):
        (conn, bars) = run(
            {"id": 1, "cmd": "connect", "login": 1, "password": "bad", "server": "x"},
            {"id": 2, "cmd": "candles", "timeframe": "1m"},
        )
        self.assertFalse(conn["ok"])
        self.assertIn("Authorization failed", conn["error"])
        self.assertEqual(bars, {"id": 2, "ok": False, "error": "not connected"})

    def test_unknown_command_and_bad_json(self):
        out = io.StringIO()
        serve(Bridge(fake), io.StringIO('garbage\n{"id": 7, "cmd": "nope"}\n'), out)
        self.assertEqual(json.loads(out.getvalue()), {"id": 7, "ok": False, "error": "unknown command: nope"})

    def test_valid_json_with_invalid_request_shape_does_not_crash_bridge(self):
        out = io.StringIO()
        serve(Bridge(fake), io.StringIO('[]\nnull\n{"id":8,"cmd":[]}\n{"id":9,"cmd":"hello"}\n'), out)
        replies = [json.loads(line) for line in out.getvalue().splitlines()]
        self.assertEqual(len(replies), 4)
        self.assertTrue(all(not reply["ok"] for reply in replies[:3]))
        self.assertTrue(replies[-1]["ok"])
        self.assertEqual(replies[-1]["id"], 9)

    def test_hello_reports_a_missing_package(self):
        bridge = Bridge()
        saved = sys.modules.pop("MetaTrader5", None)
        sys.path.remove(os.path.join(os.path.dirname(__file__), "fake_mt5"))
        try:
            (reply,) = [json.loads(x) for x in self._serve(bridge, {"id": 1, "cmd": "hello"})]
            self.assertNotIn("mt5_package", reply["result"])
            self.assertIn("pip install MetaTrader5", reply["result"]["mt5_error"])
        finally:
            sys.path.insert(0, os.path.join(os.path.dirname(__file__), "fake_mt5"))
            if saved is not None:
                sys.modules["MetaTrader5"] = saved

    def test_missing_package_is_explained(self):
        bridge = Bridge()
        saved = sys.modules.pop("MetaTrader5", None)
        sys.path.remove(os.path.join(os.path.dirname(__file__), "fake_mt5"))
        try:
            (reply,) = [json.loads(x) for x in self._serve(bridge, {"id": 1, "cmd": "connect", "login": 1,
                                                                    "password": "good", "server": "x"})]
            self.assertIn("pip install MetaTrader5", reply["error"])
        finally:
            sys.path.insert(0, os.path.join(os.path.dirname(__file__), "fake_mt5"))
            if saved is not None:
                sys.modules["MetaTrader5"] = saved

    @staticmethod
    def _serve(bridge, *requests):
        out = io.StringIO()
        serve(bridge, io.StringIO("".join(json.dumps(r) + "\n" for r in requests)), out)
        return out.getvalue().splitlines()


if __name__ == "__main__":
    unittest.main()
