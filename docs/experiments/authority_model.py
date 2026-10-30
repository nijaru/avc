#!/usr/bin/env python3
"""Finite-state safety probes for AVC closure custody and publication.

Not a SQLite/FS implementation or a power-loss test. See the closure model's
explicit fixture/atomicity bounds and design.md §9.
Run: python -m unittest -v authority_model.py
"""
from __future__ import annotations

from dataclasses import dataclass
from itertools import permutations
import unittest

from closure_custody_model import (Custody, OBJECTS, closure, reachable, steps,
                                  violation)


@dataclass(frozen=True)
class Target:
    root: str = 'A'
    generation: int = 1
    # Persistent key -> (payload digest, decision, accepted generation, event id).
    receipts: tuple[tuple[str, str, str, int, str], ...] = ()
    outbox: tuple[str, ...] = ()


def accept(target: Target, key: str, digest: str, expected_root: str,
           expected_generation: int, result_root: str) -> tuple[Target, str]:
    receipts = {r[0]: r for r in target.receipts}
    if key in receipts:
        old = receipts[key]
        if old[1] != digest:
            return target, 'KeyReuseMismatch'
        return target, old[2]
    if (target.root, target.generation) != (expected_root, expected_generation):
        decision = 'StaleTarget'
        receipt_generation = target.generation
        outbox = target.outbox
        new_root = target.root
        new_gen = target.generation
    else:
        decision = 'Accepted'
        new_root = result_root
        new_gen = target.generation+1
        receipt_generation = new_gen
        outbox = (*target.outbox, f'{key}:{new_gen}')
    receipt = (key, digest, decision, receipt_generation, new_root)
    return Target(new_root, new_gen, (*target.receipts, receipt), outbox), decision


class AuthorityQualification(unittest.TestCase):
    def test_complete_closures_survive_all_bounded_interleavings(self):
        outcomes = set()
        for state, trace in reachable():
            self.assertIsNone(violation(state), trace)
            if trace and trace[-1] in ('gc-restart', 'writer-refuse'):
                outcomes.add(trace[-1])
            if state.root == 'B' and state.export_root == 'A' and state.export_pc == 2:
                self.assertEqual(state.copied, {'A', 'shared', 'a'})
                outcomes.add('old-export-complete')
            if state.root == 'B' and state.reader_pc in (1, 2) and state.reader_root == 'A':
                outcomes.add('old-reader')
            if state.root == 'B' and 'A' in state.scan_roots:
                outcomes.add('old-scan')
            if 'audit' not in state.live:
                outcomes.add('audit-collected')
            if (state.gc_pc, state.writer_pc, state.reader_pc, state.export_pc) == (3, 3, 3, 2):
                outcomes.add('all-finished')
        self.assertEqual(outcomes, {'gc-restart', 'writer-refuse', 'old-export-complete',
                                    'old-reader', 'old-scan', 'audit-collected', 'all-finished'})

    def test_roots_only_unversioned_stage_and_wrapping_epoch_lose_closure(self):
        for fault in ('roots-only', 'unversioned-stage', 'wrapping-epoch'):
            with self.subTest(fault=fault):
                witness = next((trace for state, trace in reachable(fault=fault)
                                if violation(state)), None)
                self.assertIsNotNone(witness)

    def test_reservation_rejects_live_header_with_retired_descendant(self):
        for missing in ('B', 'b'):
            with self.subTest(missing=missing):
                state = Custody(live=frozenset(OBJECTS) - {missing})
                transitions = dict(steps(state))
                self.assertNotIn('writer-reserve', transitions)
                refused = transitions['writer-refuse']
                self.assertEqual((refused.root, refused.epoch), ('A', 0))
                self.assertEqual(refused.roots(), {'A'})

    def test_quiescent_scan_finishes_without_self_invalidating_and_ignores_audit(self):
        for root in ('A', 'B'):
            with self.subTest(root=root):
                state = Custody(root=root, writer_pc=3, reader_pc=3, export_pc=2)
                # One snapshot, one trace, one check per object, one release.
                for _ in range(len(OBJECTS) + 3):
                    transitions = [nxt for action, nxt in steps(state) if action.startswith('gc-')]
                    self.assertEqual(len(transitions), 1)
                    state = transitions[0]
                    self.assertIsNone(violation(state))
                self.assertEqual((state.gc_pc, state.epoch), (3, 2))
                self.assertEqual(state.live, closure(root))
                self.assertNotIn('audit', state.live)

    def test_renewal_and_expiry_are_versioned_authority_actions(self):
        state = Custody()
        for epoch, action in enumerate(('reader-acquire', 'reader-renew', 'reader-expire'), 1):
            state = dict(steps(state))[action]
            self.assertEqual(state.epoch, epoch)
        self.assertNotIn('reader-renew', dict(steps(state)))
        self.assertEqual(state.roots(), {'A'})

    def test_target_cas_and_persisted_receipts_survive_races_and_retries(self):
        proposals = [('p1', 'H1', 'A', 1, 'B'), ('p2', 'H2', 'A', 1, 'C')]
        for order in permutations(proposals):
            target = Target()
            results = []
            for proposal in order:
                target, outcome = accept(target, *proposal)
                results.append(outcome)
            self.assertEqual(sorted(results), ['Accepted', 'StaleTarget'])
            self.assertEqual(target.generation, 2)
            self.assertEqual(len(target.outbox), 1)
            for p in proposals:
                previous = target
                target, again = accept(target, *p)
                self.assertEqual(target, previous)  # unknown reply does not re-execute
                self.assertIn(again, ('Accepted', 'StaleTarget'))
                target, mismatch = accept(target, p[0], 'DIFFERENT', *p[2:])
                self.assertEqual(mismatch, 'KeyReuseMismatch')
                self.assertEqual(target, previous)

    def test_same_target_identity_does_not_defeat_monotonic_aba_generation(self):
        t, a = accept(Target(), 'first', 'h1', 'A', 1, 'B')
        t, b = accept(t, 'second', 'h2', 'B', 2, 'A')
        self.assertEqual((a, b, t.root, t.generation), ('Accepted', 'Accepted', 'A', 3))
        newer, stale = accept(t, 'old-validation', 'old-payload', 'A', 1, 'C')
        self.assertEqual(stale, 'StaleTarget')
        self.assertEqual(newer.root, 'A')
        self.assertEqual(newer.generation, 3)


if __name__ == '__main__':
    unittest.main()
