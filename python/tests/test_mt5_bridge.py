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
        self.assertEqual(hello, {"id": 1, "ok": True, "result": {"protocol": 1}})
        self.assertTrue(conn["ok"], conn)
        self.assertEqual(conn["result"]["symbol"], "XAUUSD.r")
        self.assertEqual(conn["result"]["server_offset"], 3 * 3600)
        self.assertEqual([b["time"] for b in bars["result"]], [1_790_208_000, 1_790_208_900, 1_790_209_800])
        self.assertEqual(bars["result"][0]["volume"], 100.0)

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
