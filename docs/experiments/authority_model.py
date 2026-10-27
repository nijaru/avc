#!/usr/bin/env python3
"""Finite-state safety probes for AVC root admission, GC and publication.

This is *not* a SQLite/FS implementation or a power-loss test. Atomic actions
model one local authority's transactions under the assumptions in design.md §9.
Run: python -m unittest -v authority_model.py
"""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass, replace
from itertools import permutations
import unittest


@dataclass(frozen=True)
class Custody:
    # One previously written, verified immutable object and one participant of
    # each role. "gone" is an actual missing physical object, not a promise.
    location: str = 'present'   # present | gone
    roots: int = 0
    staging: int = 0
    readers: int = 0
    backups: int = 0
    writer_pc: int = 0         # 0 acquire; 1 commit; 2 done; 3 refused
    reader_pc: int = 0         # 0 acquire; 1 release; 2 done
    backup_pc: int = 0         # 0 acquire; 1 release; 2 done
    gc_pc: int = 0             # 0 scan; 1 atomic SQL delete; 3 done
    marked: bool = False       # scan is advisory; MUST re-check at deletion


def custody_steps(s: Custody, *, stale_mark_sweep: bool = False):
    """Return linearizable steps, rather than assuming a convenient schedule."""
    if s.gc_pc == 0:
        yield 'scan', replace(s, gc_pc=1, marked=not any((s.roots, s.staging, s.readers, s.backups)))
    if s.gc_pc == 1:
        if stale_mark_sweep:
            # The defect: treats the old mark as authority, ignoring new pins.
            yield 'sweep-old-mark', replace(s, gc_pc=3, location='gone' if s.marked else s.location)
        else:
            can_delete = s.location == 'present' and not any((s.roots, s.staging, s.readers, s.backups))
            yield 'delete-or-skip', replace(s, gc_pc=3,
                                            location='gone' if can_delete else s.location)
    if s.writer_pc == 0:
        if s.location == 'present':
            yield 'writer-pin', replace(s, writer_pc=1, staging=s.staging+1)
        else:
            yield 'writer-refused', replace(s, writer_pc=3)
    if s.writer_pc == 1:
        if s.location == 'present' and s.staging:
            yield 'writer-commit-root', replace(s, writer_pc=2, staging=s.staging-1, roots=s.roots+1)
    if s.reader_pc == 0:
        if s.location == 'present':
            yield 'reader-lease', replace(s, reader_pc=1, readers=s.readers+1)
        else:
            yield 'reader-refused', replace(s, reader_pc=2)
    if s.reader_pc == 1:
        yield 'reader-release', replace(s, reader_pc=2, readers=s.readers-1)
    if s.backup_pc == 0:
        # Lease acquisition and root-set observation are one atomic authority
        # step for multi-transaction native exports. A complete SQLite backup
        # instead carries its own consistent payload+metadata snapshot.
        if s.location == 'present' and s.roots > 0:
            yield 'backup-root-and-lease', replace(s, backup_pc=1, backups=s.backups+1)
        else:
            yield 'backup-skip', replace(s, backup_pc=2)
    if s.backup_pc == 1:
        yield 'backup-finish', replace(s, backup_pc=2, backups=s.backups-1)


def reachable(start: Custody, *, stale_mark_sweep=False):
    queue = deque([(start, ())])
    visited = {start}
    while queue:
        state, trace = queue.popleft()
        yield state, trace
        for action, nxt in custody_steps(state, stale_mark_sweep=stale_mark_sweep):
            if nxt not in visited:
                visited.add(nxt)
                queue.append((nxt, (*trace, action)))


def custody_violation(s: Custody) -> bool:
    return s.location != 'present' and any((s.roots, s.staging, s.readers, s.backups))


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
    def test_stale_gc_mark_has_a_real_data_loss_counterexample(self):
        witnesses = [trace for state, trace in reachable(Custody(), stale_mark_sweep=True)
                     if custody_violation(state)]
        self.assertTrue(witnesses)
        witness = min(witnesses, key=len)
        # A newly reserved staging object already needs GC protection.
        self.assertEqual(witness, ('scan', 'writer-pin', 'sweep-old-mark'))
        # Even after a successful root transaction, the same stale sweep can
        # delete the acknowledged source: the bug is not only in staging.
        accepted = [trace for state, trace in reachable(Custody(), stale_mark_sweep=True)
                    if state.roots > 0 and state.location == 'gone']
        self.assertIn(('scan', 'writer-pin', 'writer-commit-root', 'sweep-old-mark'), accepted)

    def test_transactional_gc_delete_excludes_new_pins_and_readers(self):
        for initially_rooted in (0, 1):
            states = list(reachable(Custody(roots=initially_rooted)))
            self.assertGreater(len(states), 10)
            for state, trace in states:
                self.assertFalse(custody_violation(state), trace)
            self.assertTrue(any(s.gc_pc == 3 for s, _ in states))

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
