import sys
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from aegis_lab.bridges.mt5_bridge import Bridge, BridgeError

class HistoryTest(unittest.TestCase):
    def setUp(self):
        self.now = time.time_ns() // 1_000_000
        self.bridge = Bridge()
        self.bridge.symbol = 'XAUUSD.r'
        self.bridge._account_identity = (1, 'demo')
        self.mt5 = mock.Mock(COPY_TICKS_ALL=0)
        self.mt5.account_info.return_value = SimpleNamespace(login=1,server='demo')
        self.mt5.terminal_info.return_value = SimpleNamespace(connected=True)
        self.bridge._mt5 = self.mt5
        self.start, self.end = self.now-10_000, self.now-1000
        self.request={'start_ms':self.start,'end_ms':self.end,'max_ticks':2}
    def row(self, stamp):
        return {'time_msc':stamp,'bid':100.,'ask':100.1,'last':100.05,'volume':1.}
    def test_bounded_utc_slice_filters_and_discloses_incomplete_history(self):
        self.mt5.copy_ticks_from.return_value=[self.row(self.start),self.row(self.start+1),self.row(self.start+2)]
        data=self.bridge.history_ticks(self.request)
        self.assertEqual(len(data['ticks']),2)
        self.assertTrue(data['truncated'])
        self.assertFalse(data['complete_history'])
        args=self.mt5.copy_ticks_from.call_args.args
        self.assertEqual(args[0],'XAUUSD.r');self.assertEqual(args[2],3)
        self.assertEqual(args[1].utcoffset().total_seconds(),0)
        self.assertNotIn('password',data);self.assertNotIn('login',data)
    def test_invalid_query_never_reaches_tick_api(self):
        for change in [{'start_ms':True},{'max_ticks':0},{'max_ticks':2001},{'end_ms':self.now+10000},{'end_ms':self.start},{'start_ms':self.end-36_000_001},{'symbol':'OTHER'}]:
            with self.subTest(change=change),self.assertRaises(BridgeError):self.bridge.history_ticks({**self.request,**change})
        self.mt5.copy_ticks_from.assert_not_called()
    def test_account_change_during_query_rejects_reply(self):
        self.mt5.account_info.side_effect=[SimpleNamespace(login=1,server='demo'),SimpleNamespace(login=2,server='demo')]
        self.mt5.copy_ticks_from.return_value=[self.row(self.start)]
        with self.assertRaisesRegex(BridgeError,'account changed'):self.bridge.history_ticks(self.request)
    def test_none_history_and_terminal_disconnect_are_errors(self):
        self.mt5.copy_ticks_from.return_value=None
        with self.assertRaisesRegex(BridgeError,'unavailable'):self.bridge.history_ticks(self.request)
        self.mt5.terminal_info.return_value=SimpleNamespace(connected=False)
        with self.assertRaisesRegex(BridgeError,'disconnected'):self.bridge.history_ticks(self.request)
    def test_invalid_or_outside_rows_are_never_sent(self):
        bad=self.row(self.start+1);bad['bid']=float('nan')
        self.mt5.copy_ticks_from.return_value=[self.row(self.start-1),bad,self.row(self.start+2)]
        result=self.bridge.history_ticks(self.request)
        self.assertEqual(len(result['ticks']),1)
        self.assertIn('invalid_rows=1',result['coverage_note'])
        self.assertIn('outside_rows=1',result['coverage_note'])
    def test_empty_history_is_explicit_not_synthetic(self):
        self.mt5.copy_ticks_from.return_value=[]
        result=self.bridge.history_ticks(self.request)
        self.assertEqual(result['ticks'],[]);self.assertFalse(result['complete_history'])

if __name__=='__main__':unittest.main()

class HistoryExtraTest(unittest.TestCase):
    setUp = HistoryTest.setUp
    row = HistoryTest.row
    def test_out_of_order_rows_are_dropped_without_reordering_evidence(self):
        self.mt5.copy_ticks_from.return_value=[self.row(self.start+5),self.row(self.start+2),self.row(self.start+6)]
        result=self.bridge.history_ticks(self.request)
        self.assertEqual([r['time_ms'] for r in result['ticks']],[self.start+5,self.start+6])
        self.assertIn('invalid_rows=1',result['coverage_note'])
    def test_broker_api_exception_is_reported_without_exposing_response(self):
        self.mt5.copy_ticks_from.side_effect=RuntimeError('private terminal error')
        with self.assertRaisesRegex(BridgeError,'request failed') as caught:self.bridge.history_ticks(self.request)
        self.assertNotIn('private',str(caught.exception))
