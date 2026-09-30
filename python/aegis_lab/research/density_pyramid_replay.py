#!/usr/bin/env python3
"""Bounded causal density-pyramid research approximation, NOT exact user codification.
Stdlib only; no integrations. Prices use Decimal; quantities assumed ounces.
Inputs must be chronological within each file. No sorting using future data.
Optional --require-refill: selected MAIN must have >=2 positive executed tape
hits at its EXACT price, with the matching aggressor, and >=1 observed increase
in its aggregated visible size after a hit, all strictly before the candidate
and within a frozen inclusive 120-second window. Same-timestamp evidence is
excluded. Snapshots, invalid ordering and observed level removal discard
history. This proves observed aggregate replenishment, NOT exact order identity
or that an increase was caused by the hit. No alternative MAIN is substituted.
Without the flag the original signal criteria remain enabled (no refill veto).
Correctness change: observed MAIN removal/reinsert is irreversible and cancels
entries / requests a terminal exit, including removal before order arrival.
FIFO still uses actual exact-price aggressive executed volume only, one shared
budget per level; book decreases never fill orders or reduce public queue ahead.
0.5/1/1.5 targets remain provisional and frozen; no optimizer or fee override.
Source quantity units and tape completeness/deduplication cannot be established
from these input formats: ounces are an explicit assumption, not verified fact.
Run: python density_pyramid_replay.py --book-dir DIR --tape-dir DIR --out DIR \
       --days 2025-01-01,2025-01-02
Run tests: python density_pyramid_replay.py --self-test

Outputs: frozen_config.json, daily.jsonl, episodes.jsonl. Invalid executions
are unpriced (never valued using last trade); their P&L is null, not zero.
"""
import argparse
import csv
import gzip
import heapq
import json
import math
import random
import unittest
import zipfile
from collections import Counter, deque
from decimal import Decimal as D
from pathlib import Path

NS = 1_000_000_000
REFILL_WINDOW_NS = 120 * NS  # frozen causal research window, not optimized
GATE_COUNTERS = (
    "refill_exact_price_hits", "refill_observed_size_increases_after_hit",
    "refill_gate_candidates", "refill_gate_pass", "refill_gate_fail",
    "refill_gate_fail_hits", "refill_gate_fail_refill",
    "refill_gate_candidates_with_two_hits", "refill_gate_candidates_with_refill")
EPS = 1e-12
CONFIG = dict(classification='causal research approximation; not exact user codification',
    planned_oz=1.0, density_notional_usd=1_000_000, density_median_multiple=10,
    persistence_seconds=60, distance_usd=[0.02, 1.0], tape_window_seconds=30,
    pilottrain_range_ceiling_usd=0.74, approach_usd=0.05,
    density_threshold_status='illustrative user example; frozen research assumption, not definitive',
    market_exit_latency_ms=250, cancellations_latency_ms=0, leverage_simulated=False,
    entry_offsets_usd=[0.50, 0.25, 0.02], weights=[0.05, 0.15, 0.80],
    stop_beyond_density_usd=0.10, entry_arrival_ms=250, freeze_distance_usd=0.50,
    target_distances_usd=[0.50, 1.0, 1.5], target_fractions=[0.50, 0.20, 0.20],
    runner_fraction=0.10, trail_usd=0.50, maker_fee=0.0002, taker_fee=0.00055,
    exit_slippage_usd=0.05, maximum_quote_age_seconds=1,
    break_remaining_fraction=0.30, break_range_multiple=1.5,
    break_aggressor_ratio=0.65, break_volume_oz=17.62,
    maximum_hold_seconds=900, no_fill_expiry_seconds=300, cooldown_seconds=300,
    candidate_evaluation_hz=1, funding=0,
    assumptions=[
        'Size and trade volume are ounces; notional = price * size.',
        'Book ts is receive/availability clock; cts recorded only, never used to backdate.',
        'Same timestamp priority: trades, timers, book. Book state strictly earlier than tape.',
        'Snapshot resets all persistence, including main-level continuity.',
        'Main density baseline is its size at signal posting; no replacement density.',
        '50/20/20/10 fractions refer to quantity frozen at first favorable 0.50 move.',
        'Tape price triggers exits; executions use currently known bid/ask plus adverse slip.',
        'BE includes maker entry fee, taker exit fee and adverse slip, exact fee algebra.',
        'Entry and market exit latency 250ms; cancellations immediate, optimistic approximation.',
        'Exit at 250ms arrival uses already-known quote aged at most1s; unavailable quote invalidates immediately.',
        'Timestamp reversal invalidates book until non-reversed snapshot; reversed deltas ignored.',
        'No tick rounding: off-tick limits may never fill; exact-price matching only.',
        'Shared per-price queue: displayed queue consumed once, then own orders FIFO.',
        'Later book cancellations never decrease queue; trades through prices do not fill.',
        'No funding, hidden liquidity, rebates, impact model, or profitability guarantee.',
        'No parameter optimization, day selection, or outcome-based signal classification.',
        'Input monotonicity enforced; sequence gaps counted, not repaired.',
        'Positive-size median includes all visible same-side levels, not just near touch.'
    ])


def enc(x):
    if isinstance(x, D):
        return str(x)
    raise TypeError(type(x).__name__)


def timestamp(s):
    return int(D(str(s)) * NS)


# Order-statistic treap: exact same-side size median, O(log levels) expected
# updates. Fixed RNG affects data structure only, NEVER research parameters.
class Node:
    def __init__(self, key, priority):
        self.key, self.priority = key, priority
        self.left = self.right = None
        self.count = self.n = 1


def n(t):
    return t.n if t else 0


def fix(t):
    if t:
        t.n = t.count + n(t.left) + n(t.right)
    return t


def merge(a, b):
    if not a or not b:
        return a or b
    if a.priority < b.priority:
        a.right = merge(a.right, b)
        return fix(a)
    b.left = merge(a, b.left)
    return fix(b)


class Sizes:
    def __init__(self):
        self.root = None
        self.rng = random.Random(0)

    def change(self, key, delta):
        def rec(t):
            if not t:
                if delta < 0:
                    raise ValueError('missing size')
                return Node(key, self.rng.random())
            if key == t.key:
                t.count += delta
                return fix(t) if t.count else merge(t.left, t.right)
            if key < t.key:
                t.left = rec(t.left)
                if t.left and t.left.priority < t.priority:
                    a = t.left
                    t.left, a.right = a.right, t
                    fix(t)
                    return fix(a)
            else:
                t.right = rec(t.right)
                if t.right and t.right.priority < t.priority:
                    a = t.right
                    t.right, a.left = a.left, t
                    fix(t)
                    return fix(a)
            return fix(t)
        self.root = rec(self.root)

    def kth(self, k):
        t = self.root
        while t:
            if k < n(t.left):
                t = t.left
            elif k < n(t.left) + t.count:
                return t.key
            else:
                k -= n(t.left) + t.count
                t = t.right
        raise IndexError(k)

    def median(self):
        m = n(self.root)
        if not m:
            return D(0)
        return (self.kth((m-1)//2) + self.kth(m//2)) / 2


class Book:
    def __init__(self):
        self.level = {'b': {}, 'a': {}}
        self.sizes = {'b': Sizes(), 'a': Sizes()}
        self.heap = {'b': [], 'a': []}
        self.large = {'b': {}, 'a': {}}
        self.since = {'b': {}, 'a': {}}
        self.ts = None
        self.generation = 0
        self.seq = None
        self.invalid_state = True
        self.counts = Counter()
        self.incarnation = {'b': {}, 'a': {}}
        self.next_incarnation = 0
        self.evidence = {}

    def hit(self, t, price, volume, aggressor):
        # Aggregated displayed level only: no individual order identity is inferred.
        side = 'a' if aggressor == 'Buy' else 'b'
        if volume <= 0 or not self.quote(t) or price not in self.level[side]:
            return
        key = (side, price)
        history = self.evidence.setdefault(key, {'hits': deque(), 'refills': deque()})
        self.prune_evidence(history, t)
        history['hits'].append(t)
        self.counts['refill_exact_price_hits'] += 1

    @staticmethod
    def prune_evidence(history, t):
        cutoff = t-REFILL_WINDOW_NS
        while history['hits'] and history['hits'][0] < cutoff:
            history['hits'].popleft()
        while history['refills'] and history['refills'][0][0] < cutoff:
            history['refills'].popleft()

    def refill_coverage(self, side, price, t):
        history = self.evidence.get((side, price))
        if not history:
            return 0, 0
        self.prune_evidence(history, t)
        hits = sum(t-REFILL_WINDOW_NS <= h < t for h in history['hits'])
        refills = sum(t-REFILL_WINDOW_NS <= hit < observed < t
                      for hit, observed in history['refills'])
        return hits, refills

    def update(self, t, msg):
        if msg.get('_ts_reversal'):
            self.invalid_state = True
            self.evidence.clear()
            self.counts['timestamp_reversals'] += 1
            return
        if self.invalid_state and msg['type'] != 'snapshot':
            self.counts['ignored_deltas_until_snapshot'] += 1
            return
        if msg['type'] == 'snapshot':
            self.invalid_state = False
            self.level = {'b': {}, 'a': {}}
            self.sizes = {'b': Sizes(), 'a': Sizes()}
            self.heap = {'b': [], 'a': []}
            self.large = {'b': {}, 'a': {}}
            self.since = {'b': {}, 'a': {}}
            self.incarnation = {'b': {}, 'a': {}}
            self.evidence.clear()
            self.generation += 1
            self.seq = None
            self.counts['snapshots'] += 1
        elif msg['type'] != 'delta':
            raise ValueError('unknown book message type')
        seq = msg['data'].get('seq')
        if self.seq is not None and seq is not None and seq <= self.seq:
            if seq < self.seq:
                self.counts['sequence_reversals'] += 1
                self.invalid_state = True
                self.evidence.clear()
            else:
                self.counts['duplicate_sequences_ignored'] += 1
            return
        if self.seq is not None and seq is not None and seq > self.seq + 1:
            self.counts['sequence_nonconsecutive_increments'] += 1
        self.seq = seq
        for side in ('b', 'a'):
            for ps, qs in msg['data'].get(side, []):
                p, q = D(ps), D(qs)
                if not p.is_finite() or not q.is_finite() or p <= 0 or q < 0:
                    raise ValueError('invalid book price/size')
                old = self.level[side].get(p)
                key = (side, p)
                if q and old is None:
                    self.next_incarnation += 1
                    self.incarnation[side][p] = self.next_incarnation
                    self.evidence.pop(key, None)
                elif not q:
                    self.incarnation[side].pop(p, None)
                    self.evidence.pop(key, None)
                elif old is not None and q > old and key in self.evidence:
                    history = self.evidence[key]
                    self.prune_evidence(history, t)
                    preceding = [h for h in history['hits'] if h < t]
                    if preceding:
                        history['refills'].append((preceding[-1], t))
                        self.counts['refill_observed_size_increases_after_hit'] += 1
                if old is not None:
                    self.sizes[side].change(old, -1)
                if q:
                    self.level[side][p] = q
                    self.sizes[side].change(q, 1)
                    if old is None:
                        heapq.heappush(self.heap[side], -p if side == 'b' else p)
                else:
                    self.level[side].pop(p, None)
                self.counts['max_level_notional_usd'] = max(self.counts['max_level_notional_usd'], int(p*q))
                if p*q >= D(1_000_000):
                    self.counts['million_level_updates'] += 1
                    self.large[side][p] = q
                else:
                    self.large[side].pop(p, None)
                    self.since[side].pop(p, None)
            # Only large-level candidates scanned, never full book arrays.
            med = self.sizes[side].median()
            for p, q in self.large[side].items():
                if q >= 10*med:
                    self.since[side].setdefault(p, t)
                    age = (t-self.since[side][p])//NS
                    self.counts['max_eligible_density_age_seconds'] = max(self.counts['max_eligible_density_age_seconds'], age)
                else:
                    self.since[side].pop(p, None)
        # Amortized heap compaction prevents deleted deep levels accumulating forever.
        for side in ('b', 'a'):
            if len(self.heap[side]) > 2*len(self.level[side])+100:
                self.heap[side] = [-p if side == 'b' else p for p in self.level[side]]
                heapq.heapify(self.heap[side])
        self.ts = t
        self.counts['book_events'] += 1

    def best(self, side):
        h = self.heap[side]
        while h:
            p = -h[0] if side == 'b' else h[0]
            if p in self.level[side]:
                return p
            heapq.heappop(h)
        return None

    def quote(self, t):
        b, a = self.best('b'), self.best('a')
        if self.invalid_state or self.ts is None or self.ts > t or t-self.ts > NS or b is None or a is None or b >= a:
            return None
        return b, a


class Queue:
    """One execution-volume budget and one public FIFO queue per exact price."""
    def __init__(self, ahead):
        self.ahead = float(ahead)
        if not math.isfinite(self.ahead) or self.ahead < 0:
            raise ValueError('invalid queue ahead quantity')
        self.orders = []

    def consume(self, volume):
        if not math.isfinite(volume) or volume < 0:
            raise ValueError('invalid executed trade volume')
        used = min(self.ahead, volume)
        self.ahead -= used
        volume -= used
        fills = []
        for o in self.orders:
            if not o['live']:
                continue
            q = min(o['left'], volume)
            if q > EPS:
                o['left'] -= q
                volume -= q
                fills.append((o, q))
        return fills


def breakeven(entry, sign):
    # Stop trigger is a quote/tape price BEFORE 0.05 adverse execution slip.
    # long: (stop-slip)*(1-taker) >= entry*(1+maker)
    # short: (stop+slip)*(1+taker) <= entry*(1-maker)
    if sign == 1:
        return entry*(1.0002)/(1-.00055) + .05
    return entry*(1-.0002)/(1+.00055) - .05


class Replay:
    def __init__(self, day, require_refill=False):
        self.require_refill = require_refill
        self.day, self.book = day, Book()
        self.window = deque()
        self.minimum = deque()
        self.maximum = deque()
        self.volume = self.buy_volume = 0.
        self.active = None
        self.episodes = []
        self.counts = Counter()
        self.invalid = []
        self.next_eval = 0
        self.cooldown = 0
        self.last_tape = None

    def prune(self, t):
        cutoff = t-30*NS
        while self.window and self.window[0][0] < cutoff:
            _, _, q, side = self.window.popleft()
            self.volume -= q
            if side == 'Buy':
                self.buy_volume -= q
        while self.minimum and self.minimum[0][0] < cutoff:
            self.minimum.popleft()
        while self.maximum and self.maximum[0][0] < cutoff:
            self.maximum.popleft()

    def stats(self):
        if not self.window:
            return None
        return float(self.maximum[0][1]-self.minimum[0][1]), max(0., self.volume), max(0., self.buy_volume)

    def signal(self, t):
        if t < self.next_eval:
            return
        self.next_eval = t+NS
        self.counts['candidate_evaluations'] += 1
        if self.active or t < self.cooldown or not self.book.quote(t):
            return
        if not self.window or t-self.window[0][0] < 29*NS:
            return  # require nearly complete 30s observation, no padding
        r, _, _ = self.stats()
        if r > .74:
            return
        move = float(self.window[-1][1]-self.window[0][1])
        candidates = []
        for side, sign in [('a', -1), ('b', 1)]:
            if -sign*move < .05:
                continue
            touch = self.book.best(side)
            for p, start in self.book.since[side].items():
                dist = float(p-touch) if side == 'a' else float(touch-p)
                if t-start >= 60*NS and .02 <= dist <= 1:
                    candidates.append((dist, side, p, sign))
        if not candidates:
            return
        # Fixed tie rule: nearest touch, then side, then price; no future ranking.
        _, side, density, sign = min(candidates)
        hits, refills = self.book.refill_coverage(side, density, t)
        self.counts['refill_gate_candidates'] += 1
        self.counts['refill_gate_candidates_with_two_hits'] += int(hits >= 2)
        self.counts['refill_gate_candidates_with_refill'] += int(refills >= 1)
        passes = hits >= 2 and refills >= 1
        self.counts['refill_gate_pass' if passes else 'refill_gate_fail'] += 1
        if not passes:
            self.counts['refill_gate_fail_hits'] += int(hits < 2)
            self.counts['refill_gate_fail_refill'] += int(refills < 1)
        if self.require_refill and not passes:
            return  # do not substitute another MAIN or inspect subsequent events
        e = dict(day=self.day, signal_ns=t, side=side, sign=sign, density=density,
                 baseline=self.book.level[side][density], generation=self.book.generation,
                 main_incarnation=self.book.incarnation[side][density],
                 require_refill=self.require_refill, preceding_main_hits=hits,
                 observed_main_refills=refills, refill_window_seconds=120,
                 initial_range=r, arrival_ns=t+NS//4, orders=[], queues={}, fills=[],
                 exit_legs=[], qty=0., remaining=0., entry_cost=0., fees=0., gross=0.,
                 frozen=None, targets_done=0, extreme=None,
                 stop=float(density)-sign*.10, first_fill_ns=None, invalid=False,
                 main_broken=False, pending_exits=[], terminal_requested=False)
        for offset, weight in zip(CONFIG['entry_offsets_usd'], CONFIG['weights']):
            e['orders'].append(dict(price=density+D(sign)*D(str(offset)),
                                    left=weight, planned=weight, live=False, status='pending'))
        self.active = e
        self.counts['signals'] += 1

    def arrive(self, t):
        e = self.active
        if not e or e.get('arrived') or t < e['arrival_ns']:
            return
        if e.get('main_incarnation') is not None:
            self.manage(t)  # reject posting if MAIN continuity changed before arrival
            if self.active is not e or e.get('terminal_requested'):
                return
        e['arrived'] = True
        quote = self.book.quote(t)
        for o in e['orders']:
            if o['status'] != 'pending':
                continue
            if not quote:
                e['invalid'] = True
                o['status'] = 'rejected_stale_quote'
                self.counts[o['status']] += 1
                self.invalid.append(dict(ns=t, reason=o['status'], price=o['price']))
                continue
            b, a = quote
            marketable = o['price'] >= a if e['sign'] == 1 else o['price'] <= b
            if marketable:
                o['status'] = 'rejected_marketable_arrival'
                self.counts[o['status']] += 1
                continue
            side = 'b' if e['sign'] == 1 else 'a'
            queue = e['queues'].setdefault(o['price'], Queue(self.book.level[side].get(o['price'], D(0))))
            queue.orders.append(o)
            o.update(live=True, status='resting', placed_ns=t)
            self.counts['orders_placed'] += 1

    def cancel(self, reason):
        for o in self.active['orders']:
            if o['live'] or o['status'] == 'pending':
                o.update(live=False, status='cancelled_'+reason)
                self.counts['entry_cancellations'] += 1

    def finish(self, t, reason):
        e = self.active
        self.cancel(reason)
        e['end_ns'], e['end_reason'] = t, reason
        e['actual_filled_qty'] = e['qty']
        e['gross_per_planned_oz'] = None if e['invalid'] else e['gross']
        e['net_per_planned_oz'] = None if e['invalid'] else e['gross']-e['fees']
        e['fees_usd'] = e['fees']
        e.pop('queues')
        self.episodes.append(e)
        self.active = None
        self.cooldown = t+300*NS
        self.counts['episodes_finished'] += 1

    def execute_exit(self, t, q, reason, trigger_ns):
        e = self.active
        q = min(q, e['remaining'])
        if q <= EPS:
            return True
        quote = self.book.quote(t)
        if not quote:
            e['invalid'] = True
            e['exit_legs'].append(dict(ns=t, qty=q, reason=reason, price=None, invalid=True))
            self.invalid.append(dict(ns=t, reason='invalid_exit_quote', requested_reason=reason, qty=q))
            self.counts['invalid_exits'] += 1
            # Terminate invalid episode; do not carry synthetic/unpriced exposure.
            self.finish(t, 'invalid_exit_quote')
            return False
        px = float(quote[0] if e['sign'] == 1 else quote[1])-e['sign']*.05
        avg = e['entry_cost']/e['qty']
        gross = e['sign']*(px-avg)*q
        fee = px*q*.00055
        e['gross'] += gross
        e['fees'] += fee
        e['remaining'] -= q
        e['exit_legs'].append(dict(ns=t, qty=q, reason=reason, price=px, gross=gross, fee=fee, trigger_ns=trigger_ns, latency_ns=t-trigger_ns))
        self.counts['exit_legs'] += 1
        if e['remaining'] <= EPS:
            self.finish(t, reason)
            return False
        return True

    def exit(self, t, q, reason):
        """Reserve quantity now, execute no earlier than 250ms on a new valid quote."""
        e = self.active
        pending = e.setdefault('pending_exits', [])
        available = e['remaining']-sum(x['qty'] for x in pending)
        q = min(q, max(0., available))
        if q > EPS:
            pending.append(dict(trigger_ns=t, due_ns=t+NS//4, qty=q, reason=reason, attempted=False))
            self.counts['market_exit_requests'] += 1
        if not reason.startswith('target_'):
            e['terminal_requested'] = True
            self.cancel(reason)  # explicit immediate-cancellation approximation
        return True

    def settle(self, t):
        e = self.active
        if not e:
            return
        if e.get('generation') is not None and self.book.generation != e['generation']:
            e['invalid'] = True
            e['main_broken'] = True
            if not e.get('main_reset_reported'):
                self.invalid.append(dict(ns=t, reason='main_identity_snapshot_reset_during_settlement'))
                e['main_reset_reported'] = True
        pending = e.setdefault('pending_exits', [])
        while pending and pending[0]['due_ns'] <= t:
            request = pending[0]
            request['attempted'] = True
            # At arrival use the already-known quote if age-valid; no indefinite
            # wait for a later observation that could turn a stopped loss into profit.
            # execute_exit invalidates immediately when no such quote exists.
            pending.pop(0)
            if not self.execute_exit(t, request['qty'], request['reason'], request['trigger_ns']):
                return

    def manage(self, t, px=None):
        e = self.active
        if not e or e.get('terminal_requested'):
            return
        if self.book.generation != e['generation']:
            e['main_broken'] = True
            e['invalid'] = True  # cannot prove MAIN continuity across snapshot replacement
            self.invalid.append(dict(ns=t, reason='main_identity_snapshot_reset'))
            if e['remaining'] > EPS:
                self.exit(t, e['remaining'], 'main_identity_snapshot_reset')
            else:
                self.finish(t, 'main_identity_snapshot_reset')
            return
        if (e['density'] not in self.book.level[e['side']] or
                (e.get('main_incarnation') is not None and
                 self.book.incarnation[e['side']].get(e['density']) != e['main_incarnation'])):
            e['main_removed'] = True  # observed aggregate removal is irreversible
        if e.get('main_removed'):
            # A new displayed level at the same price cannot replace selected MAIN.
            # Removal is observable; unobserved individual-order turnover is not.
            if e['remaining'] > EPS:
                self.exit(t, e['remaining'], 'main_level_removed')
            else:
                self.finish(t, 'main_level_removed')
            return
        if not e['qty']:
            if t >= e['signal_ns']+300*NS:
                self.finish(t, 'no_fill_expiry')
            return
        if t >= e['first_fill_ns']+900*NS:
            self.exit(t, e['remaining'], 'max_hold')
            return
        stats = self.stats()
        main_size = D(0) if e.get('main_removed') else self.book.level[e['side']].get(e['density'], D(0))
        if stats:
            r, volume, buy = stats
            toward = buy if e['sign'] == -1 else volume-buy
            if main_size < e['baseline']*D('.30') and r > 1.5*e['initial_range'] and volume >= 17.62 and toward/volume >= .65:
                self.exit(t, e['remaining'], 'density_break')
                return
        if px is None:
            return
        avg, sign = e['entry_cost']/e['qty'], e['sign']
        # Recompute average after EVERY partial fill, before freeze/cancellation.
        favorable = sign*(float(px)-avg)
        if e['frozen'] is None and favorable >= .5:
            e['frozen'] = e['qty']
            self.cancel('favorable_freeze')
            self.counts['quantity_freezes'] += 1
        be = breakeven(avg, sign)
        quote = self.book.quote(t)
        support = float(quote[0] if sign == 1 else quote[1]) if quote else None
        if support is not None and sign*(support-be) >= 0:
            e['stop'] = max(e['stop'], be) if sign == 1 else min(e['stop'], be)
        if sign*(float(px)-e['stop']) <= 0:
            self.exit(t, e['remaining'], 'stop')
            return
        if e['frozen'] is None:
            return
        for i, (distance, fraction) in enumerate(zip(CONFIG['target_distances_usd'], CONFIG['target_fractions'])):
            if i >= e['targets_done'] and favorable >= distance:
                e['targets_done'] = i+1
                if not self.exit(t, e['frozen']*fraction, 'target_'+str(distance)):
                    return
        if e['targets_done'] == 3:
            x = float(px)
            e['extreme'] = x if e['extreme'] is None else (max(e['extreme'], x) if sign == 1 else min(e['extreme'], x))
            trailing = e['extreme']-sign*.5
            e['stop'] = max(e['stop'], trailing) if sign == 1 else min(e['stop'], trailing)
            if sign*(x-e['stop']) <= 0:
                self.exit(t, e['remaining'], 'runner_trail')

    def trade(self, t, row):
        self.counts['trades'] += 1
        if self.book.ts is not None and self.book.ts >= t:
            self.invalid.append(dict(ns=t, reason='book_leads_trade'))
            self.book.invalid_state = True
            self.book.evidence.clear()
        self.settle(t)
        self.prune(t)
        px, size, side = D(row['price']), float(row['size']), row['side']
        if not math.isfinite(size) or not px.is_finite() or size < 0 or px <= 0 or side not in ('Buy', 'Sell'):
            raise ValueError('invalid trade')
        self.book.hit(t, px, size, side)
        self.window.append((t, px, size, side))
        self.volume += size
        if side == 'Buy':
            self.buy_volume += size
        while self.minimum and self.minimum[-1][1] >= px:
            self.minimum.pop()
        while self.maximum and self.maximum[-1][1] <= px:
            self.maximum.pop()
        self.minimum.append((t, px))
        self.maximum.append((t, px))
        self.last_tape = t
        e = self.active
        if e and e.get('arrived') and e['frozen'] is None and not e.get('terminal_requested') and not self.book.invalid_state:
            correct = side == ('Sell' if e['sign'] == 1 else 'Buy')
            if correct and px in e['queues']:
                for o, q in e['queues'][px].consume(size):
                    o['status'] = 'filled' if o['left'] <= EPS else 'partially_filled'
                    if o['left'] <= EPS:
                        o['live'] = False
                    e['qty'] += q
                    e['remaining'] += q
                    e['entry_cost'] += float(px)*q
                    fee = float(px)*q*.0002
                    e['fees'] += fee
                    if e['first_fill_ns'] is None:
                        e['first_fill_ns'] = t
                    e['fills'].append(dict(ns=t, price=px, qty=q, fee=fee, trade_id=row.get('trdMatchID')))
                    self.counts['partial_fill_events'] += 1
        self.manage(t, px)
        self.signal(t)

    def timer(self):
        e = self.active
        if not e:
            return None
        options = []
        if not e.get('terminal_requested'):
            options.append(e['first_fill_ns']+900*NS if e['qty'] else e['signal_ns']+300*NS)
        if not e.get('arrived'):
            options.append(e['arrival_ns'])
        options.extend(x['due_ns'] for x in e.get('pending_exits', []) if not x['attempted'])
        return min(options) if options else None



def books(path):
    with zipfile.ZipFile(path) as z:
        members = [x for x in z.namelist() if x.endswith('.data')]
        if len(members) != 1:
            raise ValueError('expected exactly one .data member')
        prior = -1
        with z.open(members[0]) as f:
            for line in f:
                if not line.strip():
                    continue
                msg = json.loads(line)
                t = int(D(str(msg['ts']))*1_000_000)
                if t < prior:
                    msg['_ts_reversal'] = True
                prior = max(prior, t)
                yield prior, msg  # availability watermark, never backdate reversed data


def trades(path):
    prior = -1
    with gzip.open(path, 'rt', newline='') as f:
        for row in csv.DictReader(f):
            t = timestamp(row['timestamp'])
            if t < prior:
                raise ValueError('tape timestamps out of order')
            prior = t
            yield t, row


def run_day(day, book_dir, tape_dir, require_refill=False):
    r = Replay(day, require_refill=require_refill)
    bi, ti = iter(books(book_dir/(day+'.zip'))), iter(trades(tape_dir/(day+'.csv.gz')))
    b, trade = next(bi, None), next(ti, None)
    last = 0
    try:
        while b is not None or trade is not None:
            timer = r.timer()
            choices = []
            if trade is not None:
                choices.append((trade[0], 0, 'trade'))
            if b is not None:
                choices.append((b[0], 2, 'book'))
            if timer is not None:
                choices.append((timer, 1, 'timer'))
            t, _, kind = min(choices)
            last = t
            r.prune(t)
            if kind == 'trade':
                r.trade(t, trade[1])
                trade = next(ti, None)
            elif kind == 'book':
                r.book.update(t, b[1])
                if b[1].get('_ts_reversal'):
                    r.invalid.append(dict(ns=t, reason='book_timestamp_reversal_until_snapshot'))
                r.settle(t)
                r.manage(t)
                r.signal(t)
                b = next(bi, None)
            else:
                r.arrive(t)
                r.settle(t)
                r.manage(t)
        if r.active:
            if r.active['remaining'] > EPS:
                r.exit(last, r.active['remaining'], 'EOD')
                r.settle(last)
                if r.active:
                    r.active['invalid'] = True
                    r.invalid.append(dict(ns=last, reason='EOD_unpriced_pending_exit',
                                          qty=r.active['remaining']))
                    r.finish(last, 'EOD_unpriced_pending_exit')
            elif r.active:
                r.finish(last, 'EOD_no_fill')
    except Exception as exc:
        r.invalid.append(dict(ns=last, reason='input_or_replay_error', detail=str(exc)))
        if r.active:
            r.active['invalid'] = True
            r.finish(last, 'input_or_replay_error')
        r.counts['day_failed'] += 1
    counts = r.counts + r.book.counts
    for counter in GATE_COUNTERS:
        counts.setdefault(counter, 0)
    valid = [e for e in r.episodes if not e['invalid']]
    return r, dict(day=day, event_counts=dict(counts), invalid_events=r.invalid,
        episodes=len(r.episodes), valid_episodes=len(valid),
        actual_filled_qty=sum(e['qty'] for e in r.episodes),
        gross_usd_valid_only=sum(e['gross'] for e in valid),
        net_usd_valid_only=sum(e['gross']-e['fees'] for e in valid),
        pnl_complete=not r.invalid and not any(e['invalid'] for e in r.episodes))


class Tests(unittest.TestCase):
    def test_fifo_shared_volume(self):
        q = Queue(2)
        a = dict(live=True, left=.5)
        b = dict(live=True, left=.5)
        q.orders = [a, b]
        self.assertEqual(q.consume(1), [])
        fills = q.consume(1.75)
        self.assertEqual([x[1] for x in fills], [.5, .25])
        self.assertAlmostEqual(b['left'], .25)

    def test_partial_targets(self):
        frozen = .37
        legs = [frozen*x for x in [.5, .2, .2]]
        self.assertAlmostEqual(frozen-sum(legs), frozen*.1)

    def test_short_mirror(self):
        for sign in [1, -1]:
            entry, exit_px, qty = 2000., 2000.+sign, .37
            self.assertAlmostEqual(sign*(exit_px-entry)*qty, .37)
            density = D('2000')
            self.assertEqual(density+D(sign)*D('.5'), D('2000.5') if sign == 1 else D('1999.5'))

    def test_be_fees(self):
        entry = 2000.
        for sign in [1, -1]:
            trigger = breakeven(entry, sign)
            execution = trigger-sign*.05
            net = sign*(execution-entry)-entry*.0002-execution*.00055
            self.assertAlmostEqual(net, 0., places=9)

    def test_no_retrofill(self):
        r = Replay('test')
        t = 100*NS
        r.book.update(t-NS//2, dict(type='snapshot', data=dict(b=[['1999','1']], a=[['2001','1']])))
        e = dict(sign=1, frozen=None, arrived=False, queues={}, qty=0., remaining=0.,
                 entry_cost=0., fees=0., fills=[], first_fill_ns=None)
        r.active = e
        r.manage = lambda *args: None
        r.signal = lambda *args: None
        r.trade(t, dict(price='2000', size='100', side='Sell'))
        self.assertEqual(e['qty'], 0)
        queue = Queue(0)
        queue.orders = [dict(price=D('2000'), live=True, left=.05)]
        e.update(arrived=True, queues={D('2000'): queue})
        self.assertEqual(e['qty'], 0)  # historical tape is never rescanned
        r.trade(t+1, dict(price='2000', size='.02', side='Sell'))
        self.assertAlmostEqual(e['qty'], .02)

    def test_realized_partial_exit_mirror(self):
        for sign in (1, -1):
            r = Replay('test')
            side = 'b' if sign == 1 else 'a'
            density = D('1999') if sign == 1 else D('2001')
            e = dict(day='test', signal_ns=0, sign=sign, side=side,
                     density=density, generation=1, baseline=D(1000),
                     initial_range=.4, orders=[], queues={}, fills=[], exit_legs=[],
                     qty=.37, remaining=.37, entry_cost=740., fees=740.*.0002,
                     gross=0., frozen=None, targets_done=0, extreme=None,
                     stop=1998.9 if sign == 1 else 2001.1,
                     first_fill_ns=0, invalid=False, main_broken=False)
            r.active = e
            for i, distance in enumerate((.5, 1., 1.5, 1.)):
                t = (i+1)*NS
                px = 2000.+sign*distance
                # Fresh executable quotes, without tape-derived fallback.
                b, a = D(str(px-.01)), D(str(px+.01))
                levels = dict(b=[[str(b),'1']], a=[[str(a),'1']])
                levels[side].append([str(density),'1000'])
                if i == 0:
                    r.book.update(t-1, dict(type='snapshot', data=levels))
                else:
                    changes = {s: [[str(p), '0'] for p in r.book.level[s]] + levels[s]
                               for s in ('b', 'a')}
                    r.book.update(t-1, dict(type='delta', data=changes))
                r.manage(t, D(str(px)))
                r.book.update(t+NS//4, dict(type='delta', data={}))
                r.settle(t+NS//4)
            self.assertIsNone(r.active)
            self.assertEqual(len(r.episodes), 1)
            legs = r.episodes[0]['exit_legs']
            self.assertEqual(len(legs), 4)
            for leg, fraction in zip(legs, (.5, .2, .2, .1)):
                self.assertAlmostEqual(leg['qty'], .37*fraction)
            self.assertAlmostEqual(sum(x['qty'] for x in legs), .37)
            self.assertAlmostEqual(r.episodes[0]['gross'],
                sum(sign*(x['price']-2000.)*x['qty'] for x in legs))

    def test_marketable_arrival_rejected(self):
        r = Replay('test')
        t = NS
        r.book.update(t-1, dict(type='snapshot', data=dict(b=[['1999','1']], a=[['2000','1']])))
        order = dict(price=D('2000'), left=.05, live=False, status='pending')
        r.active = dict(arrival_ns=t, sign=1, orders=[order], queues={})
        r.arrive(t)
        self.assertFalse(order['live'])
        self.assertEqual(order['status'], 'rejected_marketable_arrival')

    def test_timestamp_recovery(self):
        b = Book()
        snap = dict(type='snapshot', data=dict(b=[['1999','1']], a=[['2001','1']]))
        b.update(NS, snap)
        self.assertIsNotNone(b.quote(NS))
        b.update(NS, dict(type='delta', data={}, _ts_reversal=True))
        self.assertIsNone(b.quote(NS))
        b.update(2*NS, dict(type='delta', data=dict(b=[['2000','1']])))
        self.assertIsNone(b.quote(2*NS))
        b.update(3*NS, snap)
        self.assertIsNotNone(b.quote(3*NS))
        self.assertIsNone(b.quote(3*NS-1))  # never use future book state

    def test_exit_latency_and_fresh_quote(self):
        r = Replay('test')
        r.book.update(NS-1, dict(type='snapshot', data=dict(b=[['2000','1']], a=[['2001','1']])))
        r.active = dict(remaining=1., orders=[], pending_exits=[])
        executions = []
        r.execute_exit = lambda t, q, reason, trigger: executions.append((t, q)) or True
        r.exit(NS, .5, 'target_0.5')
        r.settle(NS+NS//4-1)
        self.assertEqual(executions, [])
        r.settle(NS+NS//4)
        self.assertEqual(executions, [(NS+NS//4, .5)])  # known quote still fresh at arrival
        r.book.update(100*NS, dict(type='delta', data=dict(b=[['3000','1']])))
        r.settle(100*NS)
        self.assertEqual(executions, [(NS+NS//4, .5)])  # no later repricing of same request

    def test_reversed_sequence_does_not_change_usable_quotes(self):
        b = Book()
        b.update(0, dict(type='snapshot', data=dict(seq=10, b=[['1999','1']], a=[['2001','1']])))
        b.update(NS, dict(type='delta', data=dict(seq=9, b=[['2000','1']])))
        self.assertIsNone(b.quote(NS))
        self.assertNotIn(D('2000'), b.level['b'])
        b.update(2*NS, dict(type='snapshot', data=dict(seq=1, b=[['1999','1']], a=[['2001','1']])))
        self.assertIsNotNone(b.quote(2*NS))

    def test_stale_arrival_is_invalid_not_a_genuine_no_fill(self):
        r = Replay('test')
        r.active = dict(arrival_ns=NS, sign=1, orders=[dict(price=D('2000'), left=1., live=False, status='pending')], queues={}, invalid=False)
        r.arrive(NS)
        self.assertTrue(r.active['invalid'])
        self.assertEqual(r.active['orders'][0]['status'], 'rejected_stale_quote')

    def test_snapshot_cannot_restore_main_identity(self):
        r = Replay('test')
        r.book.update(NS, dict(type='snapshot', data=dict(b=[['1999','1']], a=[['2001','1000']])))
        r.active = dict(generation=0, invalid=False, remaining=1., orders=[], pending_exits=[])
        r.manage(NS)
        self.assertTrue(r.active['invalid'])
        self.assertEqual(r.active['pending_exits'][0]['reason'], 'main_identity_snapshot_reset')

    def test_snapshot_reset_invalidates_even_pending_terminal_exit(self):
        r = Replay('test')
        r.book.update(0, dict(type='snapshot', data=dict(b=[['1999','1']], a=[['2001','1']])))
        r.active = dict(generation=1, remaining=1., orders=[], pending_exits=[], invalid=False, main_broken=False)
        seen = []
        r.execute_exit = lambda *args: seen.append(r.active['invalid']) or True
        r.exit(NS, 1., 'stop')
        r.book.update(NS+NS//5, dict(type='snapshot', data=dict(b=[['1999','1']], a=[['2001','1']])))
        r.settle(NS+NS//4)
        self.assertTrue(r.active['invalid'])
        self.assertTrue(r.active['main_broken'])
        self.assertEqual(seen, [True])

    def test_median_and_persistence_reset(self):
        s = Sizes()
        for x in [1, 2, 2, 4]:
            s.change(D(x), 1)
        self.assertEqual(s.median(), D(2))
        s.change(D(2), -1)
        self.assertEqual(s.median(), D(2))
        b = Book()
        msg = dict(type='snapshot', data=dict(a=[['2000','1000'], ['2001','1']]))
        b.update(0, msg)
        self.assertNotIn(D(2000), b.since['a'])  # median 500.5, fails 10x
        b.update(NS, dict(type='delta', data=dict(a=[['2002','1']])))
        self.assertEqual(b.since['a'][D(2000)], NS)
        b.update(2*NS, dict(type='delta', data=dict(a=[['2000','0']])))
        self.assertNotIn(D(2000), b.since['a'])


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--book-dir', type=Path)
    p.add_argument('--tape-dir', type=Path)
    p.add_argument('--out', type=Path)
    p.add_argument('--days', help='comma-separated exact dates; no automated selection')
    p.add_argument('--self-test', action='store_true')
    p.add_argument('--require-refill', action='store_true',
                   help='require >=2 prior exact MAIN-price aggressive hits and an observed post-hit size increase in preceding 120s')
    args = p.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests))
        raise SystemExit(0 if result.wasSuccessful() else 1)
    if not all([args.book_dir, args.tape_dir, args.out, args.days]):
        p.error('--book-dir --tape-dir --out --days required unless --self-test')
    days = args.days.split(',')
    if len(set(days)) != len(days) or any(not d or '/' in d or '..' in d for d in days):
        p.error('days must be unique plain date strings')
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out/'frozen_config.json').write_text(json.dumps(dict(CONFIG, requested_days=days, require_refill=args.require_refill,
        refill_window_seconds=120, refill_minimum_hits=2, refill_minimum_size_increases=1,
        quantity_unit='ounces (input assumption; verify source externally)',
        target_status='0.5/1/1.5 provisional frozen existing targets',
        refill_identity='continuous aggregated side/price only; removal or snapshot discards evidence',
        refill_same_timestamp_policy='strictly preceding timestamps only',
        correctness_changes=['observed MAIN removal/reinsert terminates episode, including before arrival',
                             'nonfinite input quantities/prices rejected']), indent=2))
    with (args.out/'daily.jsonl').open('w') as df, (args.out/'episodes.jsonl').open('w') as ef:
        for day in days:
            try:
                replay, summary = run_day(day, args.book_dir, args.tape_dir, require_refill=args.require_refill)
                for e in replay.episodes:
                    ef.write(json.dumps(e, default=enc)+'\n')
            except Exception as exc:
                summary = dict(day=day, pnl_complete=False, invalid_events=[dict(reason='input_error', detail=str(exc))])
            df.write(json.dumps(summary, default=enc)+'\n')
            df.flush()
            ef.flush()
            print(json.dumps(summary, default=enc), flush=True)


if __name__ == '__main__':
    main()
