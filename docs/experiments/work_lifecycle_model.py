#!/usr/bin/env python3
"""Bounded local-work lifecycle model, independent of the codec/store references.

Symbolic immutable source IDs stand for complete admitted closure. Atomic methods
stand for authority transactions; write/verify are separate observations, NOT real
filesystem operations. The scheduler explores one switch with expiry, a delayed
capture, an external write, interruption and competing resume. Named scenarios
also cover return, registration closure, clean same-work navigation and stale/unknown
replies. Source/work return targets are distinct; clean navigation is not a scanner
or implicit permission to carry edits into another base. This is not a
CLI, a full workspace state machine, or crash/power-loss qualification.

Run: PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
    -s docs/experiments -p work_lifecycle_model.py -v
"""
from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass, replace
from itertools import permutations
import unittest


class Blocked(Exception):
    pass


@dataclass(frozen=True)
class Base:
    state: str
    revision: str | None = None

    def closure(self):
        return {('state', self.state)} | ({('revision', self.revision)}
                                        if self.revision is not None else set())


@dataclass(frozen=True)
class WorkTarget:
    work: str


@dataclass(frozen=True)
class SourceTarget:
    work: str  # navigation scope, NOT permission to resume this work's bytes
    base: Base


def presentation_roots(pending):
    # Each symbolic pending ID stands for the manifest, bound conflict alternatives
    # and any still-required pending bytes, NOT just generated marker text.
    return {('pending', value) for value in pending}


@dataclass(frozen=True)
class Point:
    key: str
    base: Base
    source: str
    # Origins/pending metadata must survive even when source equals another work.
    origins: tuple[tuple[str, str], ...] = ()
    pending: tuple[str, ...] = ()
    work: str | None = None
    covered: bool = True  # trusted complete-observation fixture fact, not a scan

    def closure(self):
        return ({('descriptor', self), ('state', self.source)} | self.base.closure()
                | presentation_roots(self.pending))

    @property
    def dirty(self):
        return self.source != self.base.state or bool(self.pending)


@dataclass
class Work:
    generation: int = 0
    owner: str | None = None
    saved: Point | None = None


@dataclass
class Workspace:
    work: str
    base: Base
    live: Point
    selection_gen: int = 0
    binding_gen: int = 0
    materialization_gen: int = 0
    previous: WorkTarget | SourceTarget | None = None
    presentation: tuple[str, ...] = ()
    capture_gen: int = 0  # advancing recovery cannot invalidate a history plan

    def versions(self):
        return self.selection_gen, self.binding_gen, self.materialization_gen


@dataclass(frozen=True)
class Plan:
    key: str
    workspace: str
    source: str
    target: WorkTarget | SourceTarget
    versions: tuple[int, int, int]
    work_generations: tuple[int, int]
    departing: Point
    returning: Point

    @property
    def destination(self):
        return self.target.work

    @property
    def visit(self):
        return isinstance(self.target, SourceTarget)


@dataclass
class Operation:
    plan: Plan
    phase: str = 'writing'


@dataclass(frozen=True)
class CheckpointRequest:
    workspace: str
    work: str
    versions: tuple[int, int, int]
    point: Point


@dataclass(frozen=True)
class CaptureReceipt:
    request: CheckpointRequest | Plan
    deadline: int

    @property
    def point(self):
        return self.request.returning if isinstance(self.request, Plan) else self.request.point


class Repository:
    """No work head shadows an active workspace's Selection."""

    def __init__(self, base=Base('base-state', 'R0')):
        self.now = 0
        self.a = Point('A-return', base, 'A-bytes', (('new-a', 'new'),), ('markers-a',), work='A')
        self.b = Point('B-observed', base, 'B-bytes', (('new-b', 'new'),), work='B')
        self.works = {'A': Work(saved=self.a), 'B': Work(owner='w1')}
        self.workspaces = {'w1': Workspace('B', base, self.b)}
        self.operations: dict[str, Operation] = {}
        self.receipts: dict[str, Plan] = {}
        self.captures: dict[str, CaptureReceipt] = {
            'A-old': CaptureReceipt(CheckpointRequest('old-workspace', 'A', (0, 0, 0), self.a), 7)}
        self.heads: dict[str, tuple[str, tuple[int, int, int], Point]] = {}
        self.resident = self.a.closure() | self.b.closure()

    def prepare(self, key='switch', workspace='w1', destination='A'):
        w = self.workspaces[workspace]
        dst = self.works.get(destination)
        if dst is None or dst.owner is not None or dst.saved is None:
            raise Blocked('already active or unavailable; use an independent work reference')
        departing = replace(w.live, base=w.base, pending=w.presentation, work=w.work)
        if not departing.covered:
            raise Blocked('unknown departure coverage')
        return Plan(key, workspace, w.work, WorkTarget(destination), w.versions(),
                    (self.works[w.work].generation, dst.generation), departing, dst.saved)

    def prepare_visit(self, key, base, workspace='w1'):
        w = self.workspaces[workspace]
        departing = replace(w.live, base=w.base, pending=w.presentation, work=w.work)
        if not departing.covered:
            raise Blocked('unknown departure coverage')
        if departing.dirty:
            raise Blocked('preserve and start independent work; do not carry edits')
        if base == w.base:
            raise Blocked('already selected; no completed switch')
        generation = self.works[w.work].generation
        returning = Point(key + '-source', base, base.state, work=w.work)
        return Plan(key, workspace, w.work, SourceTarget(w.work, base), w.versions(),
                    (generation, generation), departing, returning)

    def prepare_back(self, key, workspace='w1'):
        w = self.workspaces[workspace]
        target = w.previous
        if isinstance(target, SourceTarget):
            if target.work != w.work:
                raise Blocked('previous source belongs to another work scope')
            return self.prepare_visit(key, target.base, workspace)
        if isinstance(target, WorkTarget):
            return self.prepare(key, workspace, target.work)
        raise Blocked('no previous completed switch')

    def matches(self, p):
        w = self.workspaces.get(p.workspace)
        src, dst = self.works.get(p.source), self.works.get(p.destination)
        if p.visit:
            destination_matches = (p.source == p.destination and not p.departing.dirty
                                   and p.returning.base == p.target.base
                                   and not p.returning.dirty and p.returning.work == p.source)
        else:
            destination_matches = (p.source != p.destination and dst is not None
                                   and dst.owner is None and dst.saved == p.returning)
        return (w is not None and src is not None and dst is not None
                and w.work == p.source and src.owner == p.workspace
                and p.departing.covered and p.returning.covered
                and destination_matches and w.versions() == p.versions
                and (src.generation, dst.generation) == p.work_generations)

    def begin(self, p):
        previous = self.receipts.get(p.key)
        active = self.operations.get(p.key)
        if previous is not None or active is not None:
            if (previous or active.plan) != p:
                raise Blocked('retry key changed request')
            return 'completed' if previous else active.phase
        if p.key in self.captures:
            raise Blocked('retry key changed request')
        if not self.matches(p):
            raise Blocked('stale plan')
        involved = {p.source, p.destination}
        if any(op.plan.workspace == p.workspace
               or involved & {op.plan.source, op.plan.destination}
               for op in self.operations.values()):
            raise Blocked('workspace or work reference reserved, including incomplete operations')
        # A clean target's descriptor is constructed from a protected base; a
        # resume instead requires the already-retained exact return descriptor.
        required = p.returning.base.closure() if p.visit else p.returning.closure()
        if not required <= self.resident:
            raise Blocked('unavailable source')
        # Plan carries observed bytes abstractly; admission models their durable
        # capture here, not a rescan or a full preview/staging implementation.
        self.resident |= p.departing.closure() | p.returning.closure()
        # Durable intent + both descriptors/before-images precede the first write.
        self.operations[p.key] = Operation(p)
        return 'writing'

    def write(self, key):
        op = self.operations.get(key)
        if op is None or op.phase != 'writing':
            raise Blocked('not writing')
        w = self.workspaces[op.plan.workspace]
        if not w.live.covered or w.live.source != op.plan.departing.source:
            op.phase = 'incomplete'  # abstract path-guard failure; no blind rollback
        else:
            w.live = op.plan.returning
            op.phase = 'written'

    def verify(self, key):
        op = self.operations.get(key)
        if op is None or op.phase != 'written':
            raise Blocked('not ready to verify')
        w = self.workspaces[op.plan.workspace]
        op.phase = ('verified' if w.live.covered and w.live.source == op.plan.returning.source
                    else 'incomplete')

    def finish(self, p):
        if p.key in self.receipts:
            if self.receipts[p.key] != p:
                raise Blocked('retry key changed request')
            return self.receipts[p.key]
        op = self.operations.get(p.key)
        if op is None or op.plan != p or op.phase != 'verified' or not self.matches(p):
            raise Blocked('unverified or stale completion; protections retained')
        # One transaction: recovery handoff, binding/Selection, navigation, receipt.
        # The restored source is not a replacement history base.
        self.captures[p.key] = CaptureReceipt(p, self.now + 7)
        src, dst = self.works[p.source], self.works[p.destination]
        w = self.workspaces[p.workspace]
        if p.visit:
            src.generation += 1
            w.previous = SourceTarget(p.source, p.departing.base)
        else:
            src.owner, src.saved = None, p.departing
            dst.owner, dst.saved = p.workspace, None
            src.generation += 1
            dst.generation += 1
            w.previous = WorkTarget(p.source)
            w.binding_gen += 1
        w.work, w.base = p.destination, p.returning.base
        w.presentation = p.returning.pending
        w.selection_gen += 1
        w.materialization_gen += 1
        self.receipts[p.key] = p
        del self.operations[p.key]
        return p

    def checkpoint(self, key, workspace, work, versions, point):
        request = CheckpointRequest(workspace, work, versions, point)
        previous = self.captures.get(key)
        if previous is not None:
            if previous.request != request:
                raise Blocked('retry key changed request')
            return previous
        if key in self.operations or key in self.receipts:
            raise Blocked('retry key changed request')
        # Delayed admission keeps its own acknowledged protection, not attribution
        # to whichever effort now happens to select an equal revision. This model
        # has no partial-coverage descriptor; unknown observations cannot qualify.
        if not point.covered:
            raise Blocked('unknown capture coverage')
        if point.work != work:
            raise Blocked('capture origin cannot be relabeled to current work')
        receipt = CaptureReceipt(request, self.now + 7)
        self.resident |= point.closure()
        self.captures[key] = receipt
        w = self.workspaces.get(workspace)
        if w and w.work == work and w.versions() == versions and w.base == point.base:
            self.heads[workspace] = (work, versions, point)
            w.capture_gen += 1
        return receipt

    def select_base(self, workspace, base):
        if any(op.plan.workspace == workspace for op in self.operations.values()):
            raise Blocked('workspace reserved')
        w = self.workspaces[workspace]
        w.base = base
        w.selection_gen += 1
        self.resident |= base.closure()

    def current_head(self, workspace):
        w = self.workspaces[workspace]
        head = self.heads.get(workspace)
        return head[2] if head and head[:2] == (w.work, w.versions()) else None

    def unregister(self, workspace):
        if any(op.plan.workspace == workspace for op in self.operations.values()):
            raise Blocked('finish or recover incomplete materialization first')
        w = self.workspaces[workspace]
        # This models the explicit preserve-and-unregister choice after a covered
        # scan, not an unrequested background scan or deletion of local files.
        observed = replace(w.live, base=w.base, pending=w.presentation, work=w.work)
        if not observed.covered:
            raise Blocked('unknown departure coverage')
        self.resident |= observed.closure()
        ref = self.works[w.work]
        ref.owner, ref.saved = None, observed
        ref.generation += 1
        del self.workspaces[workspace]

    def close(self, work, *, discard=False):
        ref = self.works[work]
        if ref.owner is not None or any(work in (op.plan.source, op.plan.destination)
                                        for op in self.operations.values()):
            raise Blocked('active or reserved work')
        if ref.saved and ref.saved.dirty and not discard:
            raise Blocked('unfinished work; resume or explicitly discard')
        del self.works[work]

    def discover(self):
        return set(self.works)  # repository-owned, not a registration join

    def roots(self):
        roots = set()
        for w in self.workspaces.values():
            roots |= presentation_roots(w.presentation)
            roots |= w.base.closure()
            if isinstance(w.previous, SourceTarget):
                roots |= w.previous.base.closure()
        for ref in self.works.values():
            if ref.saved:
                roots |= ref.saved.closure()
        for receipt in self.captures.values():
            if receipt.deadline > self.now:
                roots |= receipt.point.closure()
        for op in self.operations.values():
            roots |= op.plan.departing.closure() | op.plan.returning.closure()
        return roots

    def collect(self):
        self.resident &= self.roots()


def schedules(foreground, background):
    """Every permutation respecting the authority operation's program order."""
    for order in permutations((*foreground, *background)):
        if tuple(event for event in order if event in foreground) == foreground:
            yield order


class WorkLifecycleQualification(unittest.TestCase):
    def test_resume_interleavings_keep_promised_source_and_metadata(self):
        foreground = ('begin', 'write', 'verify', 'finish')
        background = ('expire', 'late-capture', 'external-edit', 'interrupt', 'compete')
        for trace in schedules(foreground, background):
            r = Repository()
            p = r.prepare()
            begun = resumed = False
            for event in trace:
                try:
                    if event == 'begin':
                        r.begin(p)
                        begun = True
                    elif event == 'write':
                        r.write(p.key)
                    elif event == 'verify':
                        r.verify(p.key)
                    elif event == 'finish':
                        r.finish(p)
                        resumed = True
                    elif event == 'expire':
                        r.now = 8
                    elif event == 'late-capture':
                        r.checkpoint('late-B', 'w1', 'B', p.versions, r.b)
                    elif event == 'external-edit':
                        r.workspaces['w1'].live = Point('external', p.departing.base, 'external-bytes')
                    elif event == 'interrupt' and p.key in r.operations:
                        r.operations[p.key].phase = 'incomplete'
                    elif event == 'compete':
                        r.workspaces['w2'] = Workspace('C', r.a.base, r.b)
                        r.works['C'] = Work(owner='w2')
                        r.begin(r.prepare('competitor', 'w2', 'A'))
                except Blocked:
                    pass
                r.collect()
                # Oracle obligations follow acknowledgements, NOT roots()/pins.
                if not resumed:
                    self.assertTrue(r.a.closure() <= r.resident, (trace, event, 'unresumed A'))
                else:
                    deadline = r.captures[p.key].deadline
                    if r.now < deadline:
                        self.assertTrue(r.a.closure() <= r.resident, (trace, event, 'handoff A'))
                    self.assertNotEqual(r.current_head('w1'), r.b, (trace, event, 'late B attribution'))
                if resumed:
                    self.assertTrue(presentation_roots(r.a.pending) <= r.resident,
                                    (trace, event, 'active presentation'))
                if begun:
                    self.assertTrue(r.b.closure() <= r.resident, (trace, event, 'departing B'))
                for workspace, w in r.workspaces.items():
                    self.assertEqual(r.works[w.work].owner, workspace, (trace, event))
                for work, ref in r.works.items():
                    if ref.owner is not None:
                        self.assertEqual(r.workspaces[ref.owner].work, work, (trace, event))
                reserved = [work for op in r.operations.values()
                            for work in {op.plan.source, op.plan.destination}]
                self.assertEqual(len(reserved), len(set(reserved)), (trace, event))
                if not resumed:
                    self.assertIsNone(r.workspaces['w1'].previous, (trace, event))

    def test_return_preserves_identity_base_origins_and_deadline(self):
        for base in (Base('base-state', 'R0'), Base('empty')):
            r = Repository(base)
            p = r.prepare()
            r.begin(p)
            r.write(p.key)
            r.verify(p.key)
            r.finish(p)
            self.assertEqual((r.workspaces['w1'].work, r.workspaces['w1'].base), ('A', base))
            self.assertEqual(r.captures[p.key].point, r.a)
            self.assertEqual(r.works['B'].saved, r.b)
            r.now = 8
            r.collect()
            self.assertTrue(r.b.closure() <= r.resident)
            self.assertNotIn(('descriptor', r.a), r.resident)  # no accidental forever retention
            self.assertTrue(presentation_roots(r.a.pending) <= r.resident)
            back = r.prepare('back', destination='B')
            r.begin(back)
            r.write(back.key)
            r.verify(back.key)
            r.finish(back)
            self.assertEqual(r.workspaces['w1'].live.origins, (('new-b', 'new'),))
            self.assertEqual(r.workspaces['w1'].base, base)
            self.assertEqual(r.workspaces['w1'].previous, WorkTarget('A'))  # toggle, not stack pop
            self.assertTrue(presentation_roots(r.a.pending) <= r.resident)
            r.close('A', discard=True)
            r.collect()
            self.assertTrue(presentation_roots(r.a.pending).isdisjoint(r.resident))

    def test_unregister_and_close_do_not_hide_or_drop_unfinished_work(self):
        r = Repository()
        with self.assertRaises(Blocked):
            r.close('B')
        with self.assertRaises(Blocked):
            r.close('A')
        r.unregister('w1')
        r.now = 8
        r.collect()
        self.assertEqual(r.discover(), {'A', 'B'})
        self.assertTrue(r.a.closure() | r.b.closure() <= r.resident)
        r.close('B', discard=True)
        r.collect()
        self.assertNotIn(('descriptor', r.b), r.resident)
        # Discarding one descriptor never erases another receipt's horizon.
        still_retained = Repository()
        still_retained.close('A', discard=True)
        still_retained.collect()
        self.assertTrue(still_retained.a.closure() <= still_retained.resident)
        still_retained.now = 8
        still_retained.collect()
        self.assertNotIn(('descriptor', still_retained.a), still_retained.resident)

    def test_stale_preparation_completion_and_unknown_reply(self):
        r = Repository()
        p = r.prepare()
        r.select_base('w1', Base('different-state', 'R1'))
        r.select_base('w1', p.departing.base)
        with self.assertRaises(Blocked):
            r.begin(p)  # actual equal-value ABA via independent Selection updates
        r = Repository()
        p = r.prepare()
        r.begin(p)
        r.write(p.key)
        r.verify(p.key)
        # Fault-inject a superseded completion token. Normal Selection updates
        # are blocked by the reservation; recovery/supersession is not modeled.
        r.workspaces['w1'].selection_gen += 1
        with self.assertRaises(Blocked):
            r.finish(p)
        with self.assertRaises(Blocked):
            r.unregister('w1')
        r.now = 8
        r.collect()
        self.assertTrue(r.a.closure() | r.b.closure() <= r.resident)
        r = Repository()
        p = r.prepare()
        r.begin(p)
        r.write(p.key)
        r.verify(p.key)
        receipt = r.finish(p)
        before = deepcopy(r.__dict__)
        self.assertEqual(r.finish(p), receipt)
        self.assertEqual(r.begin(p), 'completed')
        self.assertEqual(r.__dict__, before)
        with self.assertRaises(Blocked):
            r.begin(Plan(p.key, p.workspace, p.source, p.target, p.versions,
                         p.work_generations, p.returning, p.departing))

    def test_capture_scope_and_reservation_are_not_equal_base_shortcuts(self):
        r = Repository()
        p = r.prepare()
        r.checkpoint('current-B', 'w1', 'B', p.versions, r.b)
        self.assertEqual(r.workspaces['w1'].capture_gen, 1)
        r.begin(p)  # independent capture advancement did not stale the plan
        r.workspaces['w2'] = Workspace('C', r.a.base, r.b)
        r.works['C'] = Work(owner='w2')
        competing = r.prepare('competitor', 'w2', 'A')
        with self.assertRaises(Blocked):
            r.begin(competing)
        r.write(p.key)
        r.verify(p.key)
        edited = Point('later-edit', r.a.base, 'later-bytes', work='A')
        r.workspaces['w1'].live = edited  # after verification, not overwritten by finish
        r.finish(p)
        self.assertEqual(r.workspaces['w1'].live, edited)
        self.assertIsNone(r.current_head('w1'))
        before = deepcopy(r.__dict__)
        with self.assertRaises(Blocked):
            r.checkpoint('forged-A', 'w1', 'A', r.workspaces['w1'].versions(), r.b)
        self.assertEqual(r.__dict__, before)
        r.checkpoint('late-B', 'w1', 'B', p.versions, r.b)
        self.assertIsNone(r.current_head('w1'))
        self.assertIn('late-B', r.captures)  # historical acknowledgement still kept

    def test_checkpoint_retry_keeps_original_deadline_and_head_generation(self):
        r = Repository()
        versions = r.workspaces['w1'].versions()
        r.checkpoint('capture', 'w1', 'B', versions, r.b)
        r.now = 1
        before = deepcopy(r.__dict__)
        r.checkpoint('capture', 'w1', 'B', versions, r.b)
        self.assertEqual(r.__dict__, before)

    def test_checkpoint_key_cannot_replace_acknowledged_source_or_scope(self):
        for changed in ('point', 'workspace', 'versions'):
            with self.subTest(changed=changed):
                r = Repository()
                versions = r.workspaces['w1'].versions()
                args = ['capture', 'w1', 'B', versions, r.b]
                r.checkpoint(*args)
                if changed == 'point':
                    args[-1] = replace(r.b, key='different', source='different-bytes')
                elif changed == 'workspace':
                    args[1] = 'other-workspace'
                else:
                    args[3] = (1, 0, 0)
                before = deepcopy(r.__dict__)
                with self.assertRaisesRegex(Blocked, 'changed request'):
                    r.checkpoint(*args)
                self.assertEqual(r.__dict__, before)
                r.now = 1
                r.collect()
                self.assertTrue(r.b.closure() <= r.resident)

    def test_unknown_checkpoint_cannot_acknowledge_protection_or_advance_head(self):
        r = Repository()
        before = deepcopy(r.__dict__)
        with self.assertRaisesRegex(Blocked, 'unknown.*coverage'):
            r.checkpoint('unknown', 'w1', 'B', r.workspaces['w1'].versions(),
                         replace(r.b, covered=False))
        self.assertEqual(r.__dict__, before)
        self.assertIsNone(r.current_head('w1'))

    def test_capture_and_materialization_cannot_reuse_one_request_key(self):
        for first in ('checkpoint', 'materialization'):
            with self.subTest(first=first):
                r = Repository()
                p = r.prepare()
                if first == 'checkpoint':
                    r.checkpoint(p.key, 'w1', 'B', p.versions, r.b)
                    before = deepcopy(r.__dict__)
                    with self.assertRaises(Blocked):
                        r.begin(p)
                else:
                    r.begin(p)
                    before = deepcopy(r.__dict__)
                    with self.assertRaises(Blocked):
                        r.checkpoint(p.key, 'w1', 'B', p.versions, r.b)
                self.assertEqual(r.__dict__, before)

    def test_clean_reference_closure_releases_base_not_capture_receipts(self):
        r = Repository()
        clean = Point('clean', Base('clean-state', 'R-clean'), 'clean-state')
        r.works['clean'] = Work(saved=clean)
        r.resident |= clean.closure()
        r.close('clean')
        r.collect()
        self.assertNotIn(('revision', 'R-clean'), r.resident)
        self.assertTrue(r.a.closure() <= r.resident)


if __name__ == '__main__':
    unittest.main()
