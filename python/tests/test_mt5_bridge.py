import io
import json
import os
import sys
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
        self.assertEqual(conn["result"]["server_offset"], 3 * 3600)
        self.assertEqual([b["time"] for b in bars["result"]], [1_790_208_000, 1_790_208_900, 1_790_209_800])
        self.assertEqual(bars["result"][0]["volume"], 100.0)
        checks = {c["id"]: c["status"] for c in conn["result"]["checks"]}
        self.assertEqual(checks, {"terminal": "ok", "algo": "warn", "trading": "ok", "symbol": "ok", "balance": "ok"})

    def setUp(self):
        fake._state.update(running=True, online=True, tick_age=0)

    def tearDown(self):
        fake._state.update(running=True, online=True, tick_age=0)

    def _connected(self):
        bridge = Bridge(fake)
        bridge.connect({"login": 1, "password": "good", "server": "RoboForex-ECN"})
        return bridge

    def test_offset_is_only_taken_from_a_fresh_tick(self):
        measure = Bridge._measure_offset
        now = 1_790_000_000.0
        self.assertEqual(measure(now + 3 * 3600 + 3, now), 3 * 3600)
        self.assertEqual(measure(now + 2 * 3600 - 30, now), 2 * 3600)
        self.assertIsNone(measure(now + 3 * 3600 - 45 * 60, now))  # tick 45 min old: daily break
        self.assertIsNone(measure(now - 2 * 86400, now))  # weekend
        self.assertIsNone(measure(0, now))

    def test_login_while_market_closed_learns_offset_later(self):
        fake._state["tick_age"] = 40 * 60
        bridge = self._connected()
        self.assertFalse(bridge.offset_known)
        fake._state["tick_age"] = 0
        bars = bridge.candles({"timeframe": "15m", "limit": 1})
        self.assertTrue(bridge.offset_known)
        self.assertEqual(bars[0]["time"], 1_790_208_000)

    def test_stale_tick_keeps_the_known_offset(self):
        bridge = self._connected()
        fake._state["tick_age"] = 50 * 60
        self.assertEqual(bridge.ping()["server_offset"], 3 * 3600)

    def test_offline_terminal_is_an_error_not_stale_bars(self):
        bridge = self._connected()
        fake._state["online"] = False
        with self.assertRaisesRegex(Exception, "lost the connection"):
            bridge.candles({"timeframe": "1m", "limit": 2})
        fake._state["online"] = True
        self.assertEqual(len(bridge.candles({"timeframe": "1m", "limit": 2})), 2)

    def test_closed_terminal_is_reopened_with_the_saved_login(self):
        bridge = self._connected()
        before = fake._state["inits"]
        fake._state["running"] = False
        bars = bridge.candles({"timeframe": "1m", "limit": 2})
        self.assertEqual(len(bars), 2)
        self.assertEqual(fake._state["inits"], before + 1)
        fake._state["running"] = False
        fake._state["logged_in"] = False
        with self.assertRaisesRegex(Exception, "reconnecting"):
            bridge.candles({"timeframe": "1m", "limit": 2})  # within the retry pause

    def test_ping_before_connect(self):
        (reply,) = run({"id": 1, "cmd": "ping"})
        self.assertEqual(reply, {"id": 1, "ok": False, "error": "not connected"})

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
