"""Clean history visits reuse work; private return destinations remain distinct.

Symbolic admitted closure and atomic metadata, not filesystem cleanliness proofs,
native receipts or a user-interface implementation. Existing resume schedules are
owned by work_lifecycle_model, not duplicated here.
"""
from copy import deepcopy
from dataclasses import replace
import unittest

from work_lifecycle_model import (
    Base, Blocked, Point, Repository, SourceTarget, Workspace, WorkTarget,
)


class WorkNavigationTests(unittest.TestCase):
    def clean(self, base=Base('base-state', 'R0')):
        r = Repository(base)
        w = r.workspaces['w1']
        w.live = Point('clean-B', base, base.state, work='B')
        w.presentation = ()
        return r

    def finish(self, r, p):
        r.begin(p)
        r.write(p.key)
        r.verify(p.key)
        r.finish(p)

    def admit(self, r, base):
        # Stand-in for separately acquired/verified source closure, not an audit
        # ID whose presence in a navigation plan would confer read protection.
        r.resident |= base.closure()
        return base

    def test_clean_visits_and_back_reuse_identity_and_pin_only_one_source_target(self):
        for initial in (Base('base-state', 'R0'), Base('empty')):
            r = self.clean(initial)
            handles = r.discover()
            r.checkpoint('historical', 'w1', 'B', r.workspaces['w1'].versions(), r.b)
            bases = (Base('one', 'R1'), Base('one', 'equal-State-other-Revision'),
                     Base('three', 'R3'))
            for index, base in enumerate(bases):
                self.admit(r, base)
                w = r.workspaces['w1']
                old = w.base
                binding = w.binding_gen
                p = r.prepare_visit(f'visit-{index}', base)
                self.assertEqual(r.discover(), handles)  # preparation allocates no work
                self.finish(r, p)
                self.assertEqual((w.work, w.base, w.live.source), ('B', base, base.state))
                self.assertEqual(w.previous, SourceTarget('B', old))
                self.assertEqual(w.binding_gen, binding)
                self.assertEqual(r.discover(), handles)
                self.assertEqual(r.works['B'].owner, 'w1')
                self.assertIsNone(r.works['B'].saved)
            before = deepcopy(r.__dict__)
            with self.assertRaisesRegex(Blocked, 'already selected'):
                r.prepare_visit('noop', bases[-1])
            self.assertEqual(r.__dict__, before)
            # Once ordinary capture protection expires, only current and one-slot previous
            # source targets are protected, not every visited version forever.
            r.now = 8
            r.collect()
            self.assertTrue(bases[-2].closure() | bases[-1].closure() <= r.resident)
            self.assertNotIn(('revision', 'R1'), r.resident)
            self.assertTrue(r.a.closure() <= r.resident)  # unresumed work unaffected
            back = r.prepare_back('source-back')
            self.assertTrue(back.visit)
            self.assertEqual(back.returning.base, bases[-2])
            self.finish(r, back)
            self.assertEqual(r.workspaces['w1'].previous, SourceTarget('B', bases[-1]))
            again = r.prepare_back('source-forward')
            self.finish(r, again)
            self.assertEqual(r.workspaces['w1'].base, bases[-1])
            self.assertEqual(r.discover(), handles)
            # Registration closure keeps current work recoverable but does not
            # promise an indefinitely retained former workspace navigation slot.
            r.unregister('w1')
            r.now = 16
            r.collect()
            self.assertTrue(bases[-1].closure() <= r.resident)
            self.assertNotIn(('revision', bases[-2].revision), r.resident)

    def test_dirty_pending_unavailable_and_post_prepare_edits_do_not_clean_visit(self):
        r = Repository()
        base = self.admit(r, Base('target', 'R-target'))
        before = deepcopy(r.__dict__)
        with self.assertRaisesRegex(Blocked, 'preserve'):
            r.prepare_visit('dirty', base)
        self.assertEqual(r.__dict__, before)
        r = self.clean()
        r.workspaces['w1'].presentation = ('pending-resolution',)
        with self.assertRaisesRegex(Blocked, 'preserve'):
            r.prepare_visit('pending', base)
        r = self.clean()
        r.workspaces['w1'].live = replace(r.workspaces['w1'].live, covered=False)
        before = deepcopy(r.__dict__)
        for prepare in (lambda: r.prepare_visit('unknown', base),
                        lambda: r.prepare('unknown-resume'),
                        lambda: r.unregister('w1')):
            with self.assertRaisesRegex(Blocked, 'unknown departure coverage'):
                prepare()
            self.assertEqual(r.__dict__, before)
        r = self.clean()
        p = r.prepare_visit('unavailable', base)
        before = deepcopy(r.__dict__)
        with self.assertRaisesRegex(Blocked, 'unavailable source'):
            r.begin(p)
        self.assertEqual(r.__dict__, before)
        self.admit(r, base)
        p = r.prepare_visit('external-edit', base)
        r.workspaces['w1'].live = replace(p.departing, source='later external bytes')
        r.begin(p)
        r.write(p.key)
        self.assertEqual(r.operations[p.key].phase, 'incomplete')
        with self.assertRaises(Blocked):
            r.finish(p)
        r.now = 8
        r.collect()
        self.assertTrue(p.departing.closure() | p.returning.closure() <= r.resident)
        self.assertEqual(r.workspaces['w1'].live.source, 'later external bytes')
        self.assertIsNone(r.workspaces['w1'].previous)
        with self.assertRaises(Blocked):
            r.unregister('w1')

    def test_unknown_coverage_invalidates_write_or_verification_observation(self):
        for boundary in ('write', 'verify'):
            with self.subTest(boundary=boundary):
                r = self.clean()
                base = self.admit(r, Base('target', 'R-target'))
                p = r.prepare_visit('visit', base)
                r.begin(p)
                if boundary == 'verify':
                    r.write(p.key)
                # Equal source IDs do not turn an explicitly incomplete read
                # into an observation that permits writing or verification.
                w = r.workspaces['w1']
                w.live = replace(w.live, covered=False)
                getattr(r, boundary)(p.key)
                self.assertEqual(r.operations[p.key].phase, 'incomplete')
                self.assertFalse(w.live.covered)
                with self.assertRaises(Blocked):
                    r.finish(p)
                self.assertEqual(w.base, p.departing.base)
                self.assertIsNone(w.previous)
                r.now = 8
                r.collect()
                self.assertTrue(p.departing.closure() | p.returning.closure() <= r.resident)

        # Verification is a recorded observation, not a continuing guarantee.
        # Unknown coverage observed later cannot justify overwriting those files
        # or retroactively turn the completed verification into a fresh scan.
        r = self.clean()
        base = self.admit(r, Base('target', 'R-target'))
        p = r.prepare_visit('later-unknown', base)
        r.begin(p)
        r.write(p.key)
        r.verify(p.key)
        r.workspaces['w1'].live = replace(r.workspaces['w1'].live, covered=False)
        r.finish(p)
        self.assertFalse(r.workspaces['w1'].live.covered)
        self.assertEqual(r.workspaces['w1'].base, base)

    def test_source_back_never_substitutes_saved_work_or_crosses_scope(self):
        r = self.clean()
        original = r.workspaces['w1'].base
        target = self.admit(r, Base(original.state, 'another-Revision-same-State'))
        self.finish(r, r.prepare_visit('visit', target))
        # A's saved edits have exactly the old base, but source-back must NOT
        # infer resume A, hydrate its pending bytes or adopt its work identity.
        back = r.prepare_back('back-exact-source')
        self.assertEqual((back.source, back.destination, back.returning.base),
                         ('B', 'B', original))
        self.assertEqual(back.returning.source, original.state)
        self.assertEqual(back.returning.pending, ())
        self.finish(r, back)
        self.assertEqual(r.works['A'].saved, r.a)
        self.assertEqual(r.workspaces['w1'].work, 'B')
        # Explicit resume uses the saved descriptor instead, even at equal base.
        resume = r.prepare('resume-A', destination='A')
        self.finish(r, resume)
        self.assertEqual(r.workspaces['w1'].live.source, r.a.source)
        self.assertEqual(r.workspaces['w1'].previous, WorkTarget('B'))
        return_b = r.prepare_back('resume-B')
        self.assertFalse(return_b.visit)
        self.assertEqual(return_b.returning.base, original)
        self.finish(r, return_b)
        self.assertEqual(r.workspaces['w1'].previous, WorkTarget('A'))
        # Scope corruption and unknown/closed work never fall back by equality.
        r.workspaces['w1'].previous = SourceTarget('A', original)
        with self.assertRaisesRegex(Blocked, 'another work scope'):
            r.prepare_back('wrong-scope')
        r.workspaces['w1'].previous = WorkTarget('closed')
        with self.assertRaisesRegex(Blocked, 'unavailable'):
            r.prepare_back('closed')
        r.workspaces['w1'].previous = WorkTarget('A')
        r.works['A'].owner, r.works['A'].saved = 'elsewhere', None
        r.workspaces['elsewhere'] = Workspace('A', r.a.base, r.a, presentation=r.a.pending)
        with self.assertRaisesRegex(Blocked, 'already active'):
            r.prepare_back('active-elsewhere')

    def test_visit_reservations_generations_receipts_and_late_capture(self):
        r = self.clean()
        base = self.admit(r, Base('target', 'R-target'))
        p = r.prepare_visit('stale', base)
        r.select_base('w1', Base('other', 'R-other'))
        r.select_base('w1', p.departing.base)
        with self.assertRaisesRegex(Blocked, 'stale'):
            r.begin(p)
        r = self.clean()
        self.admit(r, base)
        p = r.prepare_visit('visit', base)
        r.begin(p)
        with self.assertRaisesRegex(Blocked, 'reserved'):
            r.begin(r.prepare('competing-resume', destination='A'))
        with self.assertRaisesRegex(Blocked, 'reserved'):
            r.begin(r.prepare_visit('competing-visit', base))
        with self.assertRaises(Blocked):
            r.close('B')
        with self.assertRaises(Blocked):
            r.select_base('w1', p.returning.base)
        r.write(p.key)
        r.verify(p.key)
        r.checkpoint('late-old-base', 'w1', 'B', p.versions, p.departing)
        # A write after verification is not overwritten by metadata completion.
        external = replace(p.returning, source='post-verification edit')
        r.workspaces['w1'].live = external
        r.finish(p)
        self.assertEqual(r.workspaces['w1'].live, external)
        self.assertIsNone(r.current_head('w1'))
        self.assertIn('late-old-base', r.captures)
        before = deepcopy(r.__dict__)
        self.assertEqual(r.begin(p), 'completed')
        self.assertEqual(r.finish(p), p)
        self.assertEqual(r.__dict__, before)
        with self.assertRaisesRegex(Blocked, 'changed request'):
            r.begin(replace(p, returning=replace(p.returning, source='changed')))
        with self.assertRaisesRegex(Blocked, 'preserve'):
            r.prepare_back('dirty-back')
        # A superseded same-work completion cannot publish navigation or drop
        # either intent closure. Normal updates cannot bypass the reservation.
        stale = self.clean()
        self.admit(stale, base)
        plan = stale.prepare_visit('superseded-completion', base)
        stale.begin(plan)
        stale.write(plan.key)
        stale.verify(plan.key)
        stale.workspaces['w1'].selection_gen += 1
        with self.assertRaises(Blocked):
            stale.finish(plan)
        stale.now = 8
        stale.collect()
        self.assertIsNone(stale.workspaces['w1'].previous)
        self.assertTrue(plan.departing.closure() | plan.returning.closure() <= stale.resident)


if __name__ == '__main__':
    unittest.main()
