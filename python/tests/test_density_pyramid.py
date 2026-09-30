"""Extra stdlib-only regressions. Run: python -m unittest pyramid_review.Tests pyramid_synthetic_tests -v"""
import unittest
import tempfile
import gzip
import zipfile
import subprocess
import sys
import json
from pathlib import Path
from decimal import Decimal as D
from collections import deque
from aegis_lab.research import density_pyramid_replay as replay_module
Book, Queue, Replay, NS = replay_module.Book, replay_module.Queue, replay_module.Replay, replay_module.NS


def message(kind='delta', b=(), a=()):
    return dict(type=kind, data=dict(b=list(b), a=list(a)))


class RefillTests(unittest.TestCase):
    def book(self):
        b = Book()
        b.update(0, message('snapshot', b=[['1999.90','1'], ['1999.80','1000'], ['1999.70','1']], a=[['2000.10','1']]))
        return b

    def hit(self, book, seconds, price='1999.80', side='Sell', volume=1):
        t = int(seconds*NS)
        book.update(t-NS//10, message())
        book.hit(t, D(price), volume, side)

    def evidence(self, b):
        self.hit(b, 90)
        self.hit(b, 91)
        b.update(92*NS, message(b=[['1999.80','1100']]))

    def candidate(self, require=True, evidence=False):
        r = Replay('synthetic', require_refill=require)
        r.book = self.book()
        if evidence:
            self.evidence(r.book)
        r.book.update(120*NS-NS//10, message())
        r.window = deque([(90*NS, D('1999.99'), 1., 'Sell'), (120*NS-1, D('1999.90'), 1., 'Sell')])
        r.minimum = deque([(120*NS-1, D('1999.90'))])
        r.maximum = deque([(90*NS, D('1999.99'))])
        r.volume = 2.
        return r

    def test_gate_rejects_no_evidence_baseline_still_posts(self):
        gated, baseline = self.candidate(), self.candidate(require=False)
        gated.signal(120*NS)
        baseline.signal(120*NS)
        self.assertIsNone(gated.active)
        self.assertIsNotNone(baseline.active)
        self.assertEqual(gated.counts['refill_gate_fail_hits'], 1)
        self.assertEqual(gated.counts['refill_gate_fail_refill'], 1)

    def test_gate_posts_only_with_prior_exact_hits_and_refill(self):
        r = self.candidate(evidence=True)
        r.signal(120*NS)
        self.assertIsNotNone(r.active)
        self.assertEqual(r.active['preceding_main_hits'], 2)
        self.assertEqual(r.active['observed_main_refills'], 1)

    def test_gate_cannot_use_future_hits_or_refill(self):
        r = self.candidate()
        r.signal(120*NS)
        self.assertIsNone(r.active)
        self.hit(r.book, 121)
        self.hit(r.book, 122)
        r.book.update(123*NS, message(b=[['1999.80','1100']]))
        self.assertIsNone(r.active)  # no retrospective posting
        self.assertEqual(r.book.refill_coverage('b', D('1999.80'), 120*NS), (0, 0))
        self.assertEqual(r.book.refill_coverage('b', D('1999.80'), 124*NS), (2, 1))

    def test_future_refill_cannot_complete_prior_hits(self):
        b = self.book()
        self.hit(b, 90)
        self.hit(b, 91)
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 92*NS), (2, 0))
        b.update(93*NS, message(b=[['1999.80','1100']]))
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 92*NS), (2, 0))

    def test_zero_wrong_price_and_wrong_aggressor_not_hits(self):
        b = self.book()
        self.hit(b, 90, volume=0)
        self.hit(b, 91, price='1999.81')
        self.hit(b, 92, side='Buy')
        b.update(93*NS, message(b=[['1999.80','1100']]))
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 94*NS), (0, 0))

    def test_size_increase_before_hit_is_not_refill(self):
        b = self.book()
        b.update(89*NS, message(b=[['1999.80','1100']]))
        self.hit(b, 90)
        self.hit(b, 91)
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 92*NS), (2, 0))

    def test_rolling_window_expires_evidence(self):
        b = self.book()
        self.evidence(b)
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 211*NS), (1, 1))  # inclusive 120s boundary
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 212*NS), (0, 0))

    def test_same_timestamp_evidence_is_not_preceding(self):
        b = self.book()
        self.hit(b, 90)
        self.hit(b, 91)
        b.update(91*NS, message(b=[['1999.80','1100']]))
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 91*NS), (1, 0))
        # The 91-second hit cannot authorize a same-timestamp increase.
        # Earlier 90-second hit can, but observation must precede candidate.
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 92*NS), (2, 1))

    def test_removal_reinsert_discards_evidence_and_changes_incarnation(self):
        b = self.book()
        self.evidence(b)
        old = b.incarnation['b'][D('1999.80')]
        b.update(93*NS, message(b=[['1999.80','0'], ['1999.80','1200']]))
        self.assertNotEqual(b.incarnation['b'][D('1999.80')], old)
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 94*NS), (0, 0))

    def test_snapshot_discards_evidence(self):
        b = self.book()
        self.evidence(b)
        b.update(93*NS, message('snapshot', b=[['1999.80','1200']], a=[['2000.10','1']]))
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 94*NS), (0, 0))

    def test_main_removed_before_arrival_cancels_pending_orders(self):
        r = self.candidate(require=False)
        r.signal(120*NS)
        r.book.update(120*NS+NS//10, message(b=[['1999.80','0'], ['1999.80','1200']]))
        r.arrive(120*NS+NS//4)
        self.assertIsNone(r.active)
        self.assertEqual(r.episodes[0]['end_reason'], 'main_level_removed')
        self.assertEqual(r.counts['orders_placed'], 0)

    def test_ask_side_aggressive_buy_mirror(self):
        b = Book()
        b.update(0, message('snapshot', b=[['1999.90','1']], a=[['2000.10','1000']]))
        self.hit(b, 90, price='2000.10', side='Buy')
        self.hit(b, 91, price='2000.10', side='Buy')
        b.update(92*NS, message(a=[['2000.10','1100']]))
        self.assertEqual(b.refill_coverage('a', D('2000.10'), 93*NS), (2, 1))

    def test_actual_tape_volume_consumes_one_shared_fifo_budget(self):
        r = Replay('synthetic')
        r.book.update(0, message('snapshot', b=[['1999','1']], a=[['2001','1']]))
        queue = Queue(2)
        queue.orders = [dict(price=D('2000'), live=True, left=.5), dict(price=D('2000'), live=True, left=.5)]
        e = dict(sign=1, frozen=None, arrived=True, queues={D('2000'):queue}, qty=0., remaining=0., entry_cost=0., fees=0., fills=[], first_fill_ns=None)
        r.active = e
        r.manage = lambda *args: None
        r.signal = lambda *args: None
        r.trade(NS//10, dict(price='2000', size='1', side='Sell'))
        self.assertEqual(e['qty'], 0.)
        r.trade(NS//5, dict(price='2000', size='1.75', side='Sell'))
        self.assertAlmostEqual(e['qty'], .75)
        self.assertEqual([f['qty'] for f in e['fills']], [.5, .25])
        self.assertAlmostEqual(queue.orders[1]['left'], .25)
        # A through-price trade and the opposite aggressor do not create fills.
        r.trade(NS//4, dict(price='1999.99', size='100', side='Sell'))
        r.trade(NS//3, dict(price='2000', size='100', side='Buy'))
        self.assertAlmostEqual(e['qty'], .75)

    def test_main_removal_with_exposure_requests_terminal_exit(self):
        r = self.candidate(require=False)
        r.signal(120*NS)
        e = r.active
        e.update(qty=.37, remaining=.37, entry_cost=1999.9*.37, first_fill_ns=120*NS)
        r.book.update(120*NS+NS//10, message(b=[['1999.80','0'], ['1999.80','1200']]))
        r.manage(120*NS+NS//10)
        self.assertTrue(e['terminal_requested'])
        self.assertEqual(e['pending_exits'][0]['reason'], 'main_level_removed')
        self.assertAlmostEqual(e['pending_exits'][0]['qty'], .37)
        self.assertTrue(all(not o['live'] and o['status'].startswith('cancelled_') for o in e['orders']))
        r.settle(120*NS+NS//10+NS//4)
        self.assertIsNone(r.active)
        self.assertEqual(r.episodes[0]['end_reason'], 'main_level_removed')

    def test_timestamp_reversal_discards_refill_history(self):
        b = self.book()
        self.evidence(b)
        msg = message()
        msg['_ts_reversal'] = True
        b.update(93*NS, msg)
        self.assertEqual(b.refill_coverage('b', D('1999.80'), 94*NS), (0, 0))
        self.assertTrue(b.invalid_state)

    def test_cli_retains_jsonl_and_explicit_zero_coverage_counters(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with zipfile.ZipFile(root/'synthetic.zip', 'w') as archive:
                archive.writestr('book.data', '')
            with gzip.open(root/'synthetic.csv.gz', 'wt') as tape:
                tape.write('timestamp,price,size,side\n')
            for require in (False, True):
                out = root/str(require)
                command = [sys.executable, str(Path(replay_module.__file__)),
                           '--book-dir', str(root), '--tape-dir', str(root),
                           '--days', 'synthetic', '--out', str(out)]
                if require:
                    command.append('--require-refill')
                subprocess.run(command, check=True, capture_output=True, text=True)
                config = json.loads((out/'frozen_config.json').read_text())
                daily = json.loads((out/'daily.jsonl').read_text())
                self.assertEqual(config['require_refill'], require)
                self.assertEqual(config['refill_window_seconds'], 120)
                self.assertEqual(config['target_distances_usd'], [.5, 1., 1.5])
                self.assertEqual(daily['event_counts']['refill_gate_fail'], 0)
                self.assertEqual(daily['event_counts']['refill_exact_price_hits'], 0)
                self.assertEqual((out/'episodes.jsonl').read_text(), '')

    def test_nonfinite_and_negative_volumes_rejected(self):
        for volume in (float('nan'), float('inf'), -1):
            with self.assertRaises(ValueError):
                Queue(1).consume(volume)
        for volume in ('NaN', 'Infinity', '-1'):
            with self.assertRaises(ValueError):
                Replay('synthetic').trade(NS, dict(price='2000', size=volume, side='Sell'))


if __name__ == '__main__':
    unittest.main(verbosity=2)
