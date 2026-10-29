"""Selective authoring/privacy contracts, not native filesystem/durability tests."""
from __future__ import annotations

import unittest

from codec_reference import Builder, K_BLOB, K_REV, K_STATE
from codec_records import Observation, Transform
from codec_verify import Cursor, Invalid, Verifier
from codec_runtime_checks import commands, corpus, invoke
from selective_authoring_reference import Authority, Base, Choice, File, Refused, read_state, write_state

README, MAIN, NOTES = (b'README.md',), (b'src', b'main.py'), (b'private-notes.txt',)
FAILED, SKIPPED = (b'private-client-audit',), (b'private-excluded-env',)
SECRET = b'OMITTED-PRIVATE-CHECKPOINT-BYTES-9371'
LATER = b'UNOBSERVED-LATER-EDITOR-BYTES-4826'


def observation(objects, provenance):
    """Read verified public records, rather than trusting plan attributes."""
    verifier = Verifier(objects)
    verifier.generic(provenance)
    claim = verifier.provenances[provenance]
    assert not claim.inputs and not claim.occurrences  # No broad capture inputs.
    observed = claim.records[-1]
    assert isinstance(observed, Observation) and not observed.resolutions
    transform = None
    if isinstance(claim.records[0], Transform):
        record = claim.records[0]
        assert not record.structural
        transform = (record.base, [(e.path, Builder.maybe(e.expected), Builder.maybe(e.value))
                                  for e in record.edits])
    return dict(vars(observed), result=claim.result, transform=transform)


class SelectiveAuthoring(unittest.TestCase):
    def initial_capture(self):
        authority = Authority()
        authority.live = {README: File(b'Public project\n'), MAIN: File(b'print("hello")\n'),
                          NOTES: File(SECRET)}
        capture = authority.checkpoint(authority.live, failed=(FAILED,), skipped=(SKIPPED,), partial=True)
        return authority, capture

    def assert_independent_export(self, objects, root):
        Verifier(objects).generic(root)
        for runtime, command in commands('--objects-stdin'):
            with self.subTest(runtime=runtime):
                self.assertEqual(invoke(command, corpus(objects, [root])), [True])

    def test_first_subset_commit_recovery_and_selected_export(self):
        authority, capture = self.initial_capture()
        initial = authority.base
        with self.assertRaisesRegex(Refused, 'nothing selected'):
            authority.prepare(capture.receipt)  # Private membership is not tracking.
        candidate = authority.prepare(capture.receipt, (Choice(README), Choice((b'src',))))
        authority.live[NOTES] = File(LATER)
        revision = authority.confirm(candidate.key)
        self.assertEqual(authority.live[NOTES], File(LATER))
        self.assertEqual(read_state(authority.builder.store, authority.base.state), {
            README: File(b'Public project\n'), MAIN: File(b'print("hello")\n')})
        cursor = Cursor(authority.builder.store[revision])
        self.assertEqual(cursor.ref({K_STATE}), candidate.state)
        self.assertEqual(cursor.number(4), 0)  # First commit has no synthetic parent.
        authority.collect()
        base, recovered, origins = authority.recover(capture.receipt)
        self.assertEqual(base, initial)
        self.assertEqual(recovered, {README: File(b'Public project\n'), MAIN: File(b'print("hello")\n'),
                                     NOTES: File(SECRET)})
        self.assertEqual(origins, {README: 'new', MAIN: 'new', NOTES: 'new'})
        self.assertEqual(read_state(authority.builder.store, base.state), {})
        # A fresh recovery view uses the recorded base, not the broad captured
        # State or today's authored revision as its tracking selection.
        fresh = Authority()
        fresh.builder.store.update(authority.builder.store)
        fresh.select(base)
        fresh.live = recovered
        fresh_capture = fresh.checkpoint(fresh.live)
        with self.assertRaisesRegex(Refused, 'nothing selected'):
            fresh.prepare(fresh_capture.receipt)
        self.assertEqual(fresh.live[NOTES], File(SECRET))
        exported = authority.export(revision)
        self.assert_independent_export(exported, revision)
        public = observation(exported, candidate.provenance)
        self.assertEqual(public['scope'], (README, (b'src',)))
        self.assertEqual(public['covered'], (README, MAIN))
        self.assertEqual((public['failed'], public['skipped'], public['receipt']), ((), (), None))
        self.assertFalse(public['partial'])  # Unrelated private gaps are not inherited.
        self.assertEqual((public['base'], public['revision']), (initial.state, None))
        payloads = b''.join(exported.values())
        for private in (SECRET, LATER, NOTES[0], FAILED[0], SKIPPED[0], capture.receipt):
            self.assertNotIn(private, payloads)
        self.assertNotIn(capture.state, exported)
        # Export remains independently reconstructible without private custody.
        del authority.captures[capture.receipt]
        authority.collect()
        self.assertEqual(authority.export(revision), exported)
        with self.assertRaises(Invalid):
            Verifier(exported).state(capture.state)

    def test_preview_is_pinned_and_capture_advancement_does_not_invalidate_it(self):
        authority, capture = self.initial_capture()
        candidate = authority.prepare(capture.receipt, (Choice(README),))
        authority.live[README] = File(LATER)
        authority.checkpoint(authority.live)  # Independent recovery advancement.
        revision = authority.confirm(candidate.key)
        self.assertEqual(read_state(authority.export(revision), candidate.state), {README: File(b'Public project\n')})
        self.assertEqual(authority.live[README], File(LATER))
        # Unknown-result retry returns its original receipt before testing the
        # now-advanced selection; it must not create another authored revision.
        self.assertEqual(authority.confirm(candidate.key), revision)
        self.assertEqual(authority.authored, {revision})
        later_capture = authority.checkpoint((README,))
        later_candidate = authority.prepare(later_capture.receipt)
        self.assertNotEqual(later_candidate.change, candidate.change)
        later_revision = authority.confirm(later_candidate.key)
        cursor = Cursor(authority.builder.store[later_revision])
        self.assertEqual(cursor.ref({K_STATE}), later_candidate.state)
        self.assertEqual(cursor.number(4), 1)
        self.assertEqual(cursor.ref({K_REV}), revision)  # Exact recorded parent.
        self.assertEqual(cursor.take(16), later_candidate.change)

    def test_cancelled_selection_is_not_later_authoring_consent(self):
        authority, capture = self.initial_capture()
        candidate = authority.prepare(capture.receipt, (Choice(README, (0,)),))
        authority.cancel(candidate.key)
        authority.collect()
        self.assertFalse(authority.authored)
        with self.assertRaisesRegex(Refused, 'cancelled'):
            authority.confirm(candidate.key)
        self.assertNotIn(candidate.provenance, authority.builder.store)
        self.assertEqual(authority.recover(capture.receipt)[1][NOTES], File(SECRET))
        with self.assertRaisesRegex(Refused, 'nothing selected'):
            authority.prepare(capture.receipt)
        # Conversely, an earlier omission is not a sticky exclusion.
        next_candidate = authority.prepare(capture.receipt, (Choice(NOTES),))
        revision = authority.confirm(next_candidate.key)
        self.assertEqual(read_state(authority.export(revision), next_candidate.state), {NOTES: File(SECRET)})

    def test_stale_selection_and_aba_have_no_half_authored_result(self):
        for return_to_original in (False, True):
            with self.subTest(aba=return_to_original):
                authority, capture = self.initial_capture()
                candidate = authority.prepare(capture.receipt, (Choice(README),))
                original = authority.base
                # Same State but distinct history still invalidates the plan.
                other = authority.builder.revision(original.state, [], b'R' * 16, b'a', b'other history', 1)
                authority.select(Base(original.state, other))
                if return_to_original:
                    authority.select(original)
                current = authority.base
                with self.assertRaisesRegex(Refused, 'selection changed'):
                    authority.confirm(candidate.key)
                if not return_to_original:
                    with self.assertRaisesRegex(Refused, 'recorded base'):
                        authority.prepare(capture.receipt, (Choice(README),))
                authority.collect()
                self.assertEqual(authority.base, current)
                self.assertFalse(authority.authored)
                self.assertFalse(authority.receipts)
                self.assertIn(candidate.provenance, authority.builder.store)
                self.assertEqual(authority.recover(capture.receipt)[1][NOTES], File(SECRET))
                if return_to_original:
                    # Re-review can reuse historical bytes at their exact base.
                    # Its new CAS token is not the old observation generation.
                    reviewed = authority.prepare(capture.receipt, (Choice(README),), zero_parent=True)
                    self.assertEqual(reviewed.generation, authority.generation)
                    public = observation(authority.builder.store, reviewed.provenance)
                    self.assertEqual(public['generation'], capture.generation)
                    self.assertNotEqual(reviewed.generation, capture.generation)
                    self.assertEqual(reviewed.state, candidate.state)
                    authority.confirm(reviewed.key)

    def tracked_fixture(self):
        authority = Authority()
        authority.live = {MAIN: File(b'old first\nunchanged middle\nold last\n')}
        capture = authority.checkpoint(authority.live)
        first = authority.prepare(capture.receipt, (Choice(MAIN),))
        authority.confirm(first.key)
        return authority

    def test_constructed_hunk_candidate_does_not_disclose_omitted_hunk(self):
        authority = self.tracked_fixture()
        recorded_base = authority.base
        authority.live[MAIN] = File(b'new first\nunchanged middle\n' + SECRET + b'\n')
        capture = authority.checkpoint((MAIN,))
        candidate = authority.prepare(capture.receipt, (Choice(MAIN, (0,)),))
        authority.live[MAIN] = File(LATER)
        revision = authority.confirm(candidate.key)
        exported = authority.export(revision)
        self.assertEqual(read_state(exported, candidate.state), {
            MAIN: File(b'new first\nunchanged middle\nold last\n')})
        self.assertEqual(authority.live[MAIN], File(LATER))
        self.assertEqual(authority.recover(capture.receipt)[0], recorded_base)
        self.assertEqual(authority.recover(capture.receipt)[2], {MAIN: 'tracked'})
        public = observation(exported, candidate.provenance)
        self.assertIsNotNone(public['transform'])  # Constructed, not a snapshot claim.
        self.assertEqual(public['consistency'], 0)
        self.assertEqual(public['transform'][0], recorded_base.state)
        # Replay the exported Transform against its parent, without the broad
        # observed file. Audit values are checked explicitly for this replay.
        replay = read_state(exported, recorded_base.state)
        for path, expected, replacement in public['transform'][1]:
            old = replay.get(path)
            expected_entry = (b'\0' if old is None else b'\1' + authority.builder.file(
                authority.builder.blob(old.content), old.executable))
            self.assertEqual(expected, expected_entry)
            cursor = Cursor(replacement)
            self.assertTrue(cursor.flag())
            self.assertEqual(cursor.number(1), 1)
            blob = cursor.ref({K_BLOB})
            replay[path] = File(exported[blob], cursor.flag())
            cursor.done()
        independently_built = Builder()
        self.assertEqual(write_state(independently_built, replay), candidate.state)
        self.assertNotIn(SECRET, b''.join(exported.values()))
        self.assertNotIn(capture.state, exported)
        self.assert_independent_export(exported, revision)

    def test_selected_gap_requires_consent_and_never_implies_deletion(self):
        authority = self.tracked_fixture()
        original = dict(authority.live)
        authority.live[README] = File(b'visible change\n')
        del authority.live[MAIN]  # Scanner could not establish what happened.
        capture = authority.checkpoint((README,), failed=(MAIN,), partial=True)
        with self.assertRaisesRegex(Refused, 'partial consent'):
            authority.prepare(capture.receipt, (Choice(()),))
        candidate = authority.prepare(capture.receipt, (Choice(()),), partial=True)
        revision = authority.confirm(candidate.key)
        self.assertEqual(read_state(authority.export(revision), candidate.state), {
            **original, README: File(b'visible change\n')})
        public = observation(authority.export(revision), candidate.provenance)
        self.assertEqual(public['failed'], (MAIN,))
        self.assertTrue(public['partial'])
        self.assertEqual(public['covered'], (README,))

    def test_bare_commit_defers_entire_tracked_subtree_removal(self):
        authority = self.tracked_fixture()
        del authority.live[MAIN]
        capture = authority.checkpoint((MAIN,))  # Explicitly proven absence.
        with self.assertRaisesRegex(Refused, 'nothing selected'):
            authority.prepare(capture.receipt)
        candidate = authority.prepare(capture.receipt, (Choice((b'src',)),))
        authority.confirm(candidate.key)
        self.assertEqual(read_state(authority.builder.store, candidate.state), {})

    def test_noninitial_stateonly_requires_root_intent_even_when_empty(self):
        for empty in (True, False):
            with self.subTest(empty=empty):
                authority = Authority() if empty else self.tracked_fixture()
                authority.select(Base(authority.base.state))
                authority.live[README] = File(b'explicitly chosen new root\n')
                capture = authority.checkpoint(authority.live)
                with self.assertRaisesRegex(Refused, 'explicit root intent'):
                    authority.prepare(capture.receipt, (Choice(README),))
                candidate = authority.prepare(capture.receipt, (Choice(README),), zero_parent=True)
                revision = authority.confirm(candidate.key)
                cursor = Cursor(authority.builder.store[revision])
                self.assertEqual(cursor.ref({K_STATE}), candidate.state)
                self.assertEqual(cursor.number(4), 0)

    def test_binary_and_mode_changes_require_whole_entry_choice(self):
        for value in (File(b'binary\0bytes'), File(b'new first\n', True)):
            with self.subTest(value=value):
                authority = self.tracked_fixture()
                authority.live[MAIN] = value
                capture = authority.checkpoint((MAIN,))
                with self.assertRaisesRegex(Refused, 'whole-entry'):
                    authority.prepare(capture.receipt, (Choice(MAIN, (0,)),))
                candidate = authority.prepare(capture.receipt, (Choice(MAIN),))
                authority.confirm(candidate.key)
                self.assertEqual(read_state(authority.builder.store, candidate.state)[MAIN], value)

    def test_real_parent_history_is_not_a_privacy_filter(self):
        authority, capture = self.initial_capture()
        private = authority.prepare(capture.receipt, (Choice(NOTES),))
        parent = authority.confirm(private.key)
        authority.live.clear()
        deletion = authority.checkpoint((NOTES,))
        candidate = authority.prepare(deletion.receipt)  # Root file deletion is eligible.
        revision = authority.confirm(candidate.key)
        exported = authority.export(revision)
        self.assertEqual(read_state(exported, candidate.state), {})
        self.assertIn(parent, exported)
        self.assertIn(SECRET, b''.join(exported.values()))
        self.assert_independent_export(exported, revision)


if __name__ == '__main__':
    unittest.main()
