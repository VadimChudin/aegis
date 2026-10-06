import os
import sys
import tempfile
import importlib.util
from pathlib import Path
import unittest
from types import SimpleNamespace

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from aegis_lab.bridges.mt5_bridge import Bridge, BridgeError, MAGIC  # noqa: E402

_fake_path = Path(__file__).parent / "fake_mt5" / "MetaTrader5.py"
_spec = importlib.util.spec_from_file_location("aegis_fake_mt5_trading", _fake_path)
fake = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fake)


class Mt5TradingTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.old_env = os.environ.get("AEGIS_TRADE_LEDGER_DIR")
        os.environ["AEGIS_TRADE_LEDGER_DIR"] = self.temp.name
        fake._state.update(trade_enabled=True, demo=True, hedging=True, positions=[], deals=[],
                           send_mode="filled", order_checks=0, order_sends=0, close_fail_tickets=set(),
                           connected=True, balance=1000.0)
        os.environ["AEGIS_TEST_LIVE_MT5"] = "1"
        self.bridge = Bridge(fake)
        self.bridge.connect({"login": 1, "password": "good", "server": "RoboForex-ECN"})
        fake._state["trade_enabled"] = True

    def tearDown(self):
        if self.old_env is None:
            os.environ.pop("AEGIS_TRADE_LEDGER_DIR", None)
        else:
            os.environ["AEGIS_TRADE_LEDGER_DIR"] = self.old_env
        os.environ.pop("AEGIS_TEST_LIVE_MT5", None)
        fake._state["logged_in"] = False
        fake._state["trade_enabled"] = False
        self.temp.cleanup()

    @staticmethod
    def request(request_id="order-1", **extra):
        return {"request_id": request_id, "side": "BUY", "risk_pct": 0.5, "stop": 4291.0,
                "target": 4300.0, "max_spread": 2.0, "max_positions": 2,
                "daily_loss_limit_pct": 5.0, "confirm_real": True, **extra}

    def test_account_balance_positions_and_owned_trade_close(self):
        state = self.bridge.trading_state({})
        self.assertEqual(state["account"]["balance"], 1000.0)
        self.assertTrue(state["account"]["demo"])
        fake._state["balance"] = 1042.5
        self.assertEqual(self.bridge.trading_state({})["account"]["balance"], 1042.5)
        fake._state["balance"] = 1000.0
        opened = self.bridge.place_order(self.request())
        self.assertEqual(opened["status"], "filled")
        self.assertLessEqual(opened["volume_lots"], 0.02)
        position = self.bridge.trading_state({})["positions"][0]
        self.assertEqual(position["magic"], MAGIC)
        self.assertEqual(position["quantity_oz"], position["volume_lots"] * 100)
        closed = self.bridge.close_position({"ticket": position["ticket"]})
        self.assertEqual(closed["status"], "filled")
        sends = fake._state["order_sends"]
        self.assertEqual(self.bridge.close_position({"ticket": position["ticket"]}), closed)
        self.assertEqual(fake._state["order_sends"], sends)
        self.assertEqual(self.bridge.trading_state({})["positions"], [])

    def test_explicit_confirmation_permissions_and_risk_rounding_are_required(self):
        with self.assertRaisesRegex(BridgeError, "confirm_real"):
            self.bridge.place_order(self.request(confirm_real=False))
        fake._state["trade_enabled"] = False
        with self.assertRaisesRegex(BridgeError, "not enabled"):
            self.bridge.place_order(self.request())
        fake._state["trade_enabled"] = True
        with self.assertRaisesRegex(BridgeError, "risk-based volume"):
            self.bridge.place_order(self.request(risk_pct=0.00001))
        self.assertEqual(fake._state["order_sends"], 0)

    def test_partial_is_reported_and_never_blindly_retried(self):
        fake._state["send_mode"] = "partial"
        result = self.bridge.place_order(self.request())
        self.assertEqual(result["status"], "partial")
        self.assertEqual(self.bridge.place_order(self.request()), result)
        self.assertEqual(fake._state["order_sends"], 1)
        with self.assertRaisesRegex(BridgeError, "different trade parameters"):
            self.bridge.place_order(self.request(risk_pct=0.4))

    def test_ambiguous_send_is_persisted_across_bridge_restart_and_blocks_new_orders(self):
        fake._state["send_mode"] = "unknown"
        request = self.request("ambiguous")
        first = self.bridge.place_order(request)
        self.assertEqual(first["status"], "unknown")
        restarted = Bridge(fake)
        restarted.connect({"login": 1, "password": "good", "server": "RoboForex-ECN"})
        fake._state["trade_enabled"] = True
        self.assertEqual(restarted.place_order(request), first)
        with self.assertRaisesRegex(BridgeError, "outcome is unknown"):
            restarted.place_order(self.request("new-order"))
        self.assertEqual(fake._state["order_sends"], 1)

    def test_timeout_retcode_is_unknown_and_emergency_close_remains_available(self):
        fake._state["send_mode"] = "timeout"
        request = self.request("timed-out")
        result = self.bridge.place_order(request)
        self.assertEqual(result["status"], "unknown")
        self.assertEqual(self.bridge.place_order(request), result)
        fake._state["positions"] = [SimpleNamespace(ticket=909, symbol="XAUUSD", type=fake.POSITION_TYPE_BUY,
            volume=0.01, price_open=4293.0, price_current=4293.0, sl=4290.0, tp=4300.0,
            profit=0.0, magic=MAGIC, comment="AEGIS XAUUSD")]
        fake._state["send_mode"] = "filled"
        self.assertEqual(self.bridge.close_position({"ticket": 909})["status"], "filled")
        self.assertEqual(fake._state["order_sends"], 2)

    def test_foreign_positions_are_never_managed_and_netting_exposure_blocks_open(self):
        foreign = SimpleNamespace(ticket=77, symbol="XAUUSD", type=fake.POSITION_TYPE_BUY, volume=0.1,
                                  price_open=4293.0, price_current=4293.0, sl=0.0, tp=0.0,
                                  profit=0.0, magic=99, comment="manual")
        fake._state["positions"] = [foreign]
        with self.assertRaisesRegex(BridgeError, "not owned"):
            self.bridge.close_position({"ticket": 77})
        self.assertEqual(self.bridge.close_all({}), [])
        fake._state["hedging"] = False
        with self.assertRaisesRegex(BridgeError, "netting account"):
            self.bridge.place_order(self.request())
        self.assertEqual(fake._state["order_sends"], 0)

    def test_close_all_reports_each_ticket_even_if_one_check_fails(self):
        fake._state["positions"] = [
            SimpleNamespace(ticket=ticket, symbol="XAUUSD", type=fake.POSITION_TYPE_BUY, volume=0.01,
                            price_open=4293.0, price_current=4293.0, sl=4290.0, tp=4300.0,
                            profit=0.0, magic=MAGIC, comment="AEGIS XAUUSD")
            for ticket in (101, 102)
        ]
        fake._state["close_fail_tickets"] = {102}
        outcomes = self.bridge.close_all({})
        self.assertEqual([outcome["status"] for outcome in outcomes], ["filled", "rejected"])
        self.assertEqual([outcome["request_id"] for outcome in outcomes], ["101", "102"])
        self.assertEqual(fake._state["order_sends"], 1)

    def test_owned_position_stop_update_and_partial_reduction(self):
        position = SimpleNamespace(ticket=303, symbol="XAUUSD", type=fake.POSITION_TYPE_BUY, volume=0.05,
                                  price_open=4293.0, price_current=4293.0, sl=4290.0, tp=4300.0,
                                  profit=0.0, magic=MAGIC, comment="AEGIS XAUUSD")
        fake._state["positions"] = [position]
        updated = self.bridge.modify_stop({"ticket": 303, "stop": 4292.0, "target": None})
        self.assertEqual(updated["status"], "filled")
        self.assertEqual(position.sl, 4292.0)
        reduced = self.bridge.reduce_position({"ticket": 303, "fraction": 0.5})
        self.assertEqual(reduced["status"], "filled")
        self.assertEqual(reduced["volume_lots"], 0.02)
        self.assertAlmostEqual(position.volume, 0.03)

    def test_close_and_modify_work_when_algo_button_is_off_but_not_when_terminal_is_disconnected(self):
        position = SimpleNamespace(ticket=404, symbol="XAUUSD", type=fake.POSITION_TYPE_BUY, volume=0.03,
                                  price_open=4293.0, price_current=4293.0, sl=4290.0, tp=4300.0,
                                  profit=0.0, magic=MAGIC, comment="AEGIS XAUUSD")
        fake._state["positions"] = [position]
        fake._state["trade_enabled"] = False
        self.assertEqual(self.bridge.modify_stop({"ticket": 404, "stop": 4291.0})["status"], "filled")
        self.assertEqual(self.bridge.reduce_position({"ticket": 404, "fraction": 1.0})["status"], "filled")
        self.assertEqual(fake._state["positions"], [])

    def test_stop_cannot_be_widened_and_frozen_stop_is_refused(self):
        position = SimpleNamespace(ticket=505, symbol="XAUUSD", type=fake.POSITION_TYPE_BUY, volume=0.03,
                                  price_open=4293.0, price_current=4293.0, sl=4292.0, tp=4300.0,
                                  profit=0.0, magic=MAGIC, comment="AEGIS XAUUSD")
        fake._state["positions"] = [position]
        with self.assertRaisesRegex(BridgeError, "stop-distance"):
            self.bridge.modify_stop({"ticket": 505, "stop": 4291.0})
        fake._state["freeze_level"] = 150
        with self.assertRaisesRegex(BridgeError, "stop-distance"):
            self.bridge.modify_stop({"ticket": 505, "stop": 4292.5})
        fake._state["freeze_level"] = 0


if __name__ == "__main__":
    unittest.main()
