"""Whole reference paths, not a native CLI, scanner or execution attestation.

Review tests execute trusted Python fixtures in disposable ordinary directories.
AVC does not run these tests or treat their exit status as authoring authority.
The interrupt path starts with existing work handles; allocation is not modeled.
"""
from dataclasses import replace
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from codec_reference import K_REV, K_STATE
from codec_verify import Cursor
from selective_authoring_reference import Authority, Choice, File, Refused, read_state
from work_lifecycle_model import Base, Blocked, Repository, WorkTarget

MAIN, TEST, NOTES = (b'src', b'main.py'), (b'test_answer.py',), (b'private-notes.txt',)
SECRET = b'PRIVATE-OMITTED-WORKFLOW-9371'


def program(answer, note=b'base'):
    return File(b'def answer():\n    return ' + str(answer).encode()
                + b'\n\n\nnote = "' + note + b'"\n')


def check(answer):
    return File(b'import runpy\nanswer = runpy.run_path("src/main.py")["answer"]\n'
                + b'assert answer() == ' + str(answer).encode() + b'\n')


def materialize(files, directory):
    """Write only these trusted ASCII regular-file fixtures, not arbitrary source."""
    directory.mkdir()
    for path, value in files.items():
        target = directory.joinpath(*(part.decode('ascii') for part in path))
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(value.content)


def run_check(directory):
    return subprocess.run([sys.executable, '-I', '-B', 'test_answer.py'], cwd=directory,
                          capture_output=True, timeout=10)


class SelectedReviewPath(unittest.TestCase):
    def fixture(self, expected=42):
        authority = Authority()
        authority.live = {MAIN: program(1), TEST: check(1)}
        initial = authority.checkpoint(authority.live)
        authority.confirm(authority.prepare(initial.receipt, (Choice(()),)).key)
        authority.live = {MAIN: program(42, SECRET), TEST: check(expected), NOTES: File(SECRET)}
        capture = authority.checkpoint(authority.live)
        # Select the functional edit, not the later private note in the same file.
        candidate = authority.prepare(capture.receipt, (Choice(MAIN, (0,)), Choice(TEST)))
        return authority, capture, candidate

    def test_select_test_and_author_same_candidate_despite_later_directory_edits(self):
        authority, capture, candidate = self.fixture()
        recorded_base = authority.base
        selected = {MAIN: program(42), TEST: check(42)}
        self.assertEqual(read_state(authority.builder.store, candidate.state), selected)
        # Both the editor's bytes and independent recovery can advance while this
        # one immutable selection remains open for review and authoring.
        authority.live = {MAIN: program(99), TEST: check(99), NOTES: File(SECRET)}
        live = dict(authority.live)
        authority.checkpoint(authority.live)
        authority.collect()
        with tempfile.TemporaryDirectory(prefix='avc-review-') as tmp:
            review, original = Path(tmp) / 'review', Path(tmp) / 'original'
            materialize(read_state(authority.builder.store, candidate.state), review)
            materialize(live, original)
            result = run_check(review)
            self.assertEqual(result.returncode, 0, result.stderr)
            # A successful test in the original directory exercises different
            # source and is not evidence for the selected candidate.
            self.assertEqual(run_check(original).returncode, 0)
            self.assertNotEqual((original / 'src/main.py').read_bytes(),
                                (review / 'src/main.py').read_bytes())
            # A review directory is writable too. Later changes there do not
            # replace the selected value or become an implicit new selection.
            (review / 'src/main.py').write_bytes(program(43).content)
            changed = run_check(review)
            self.assertEqual(changed.returncode, 1, changed.stderr)
            self.assertIn(b'AssertionError', changed.stderr)
            revision = authority.confirm(candidate.key)
            self.assertEqual((original / 'src/main.py').read_bytes(), program(99).content)
            self.assertEqual((review / 'src/main.py').read_bytes(), program(43).content)
        exported = authority.export(revision)
        cursor = Cursor(exported[revision])
        self.assertEqual(cursor.ref({K_STATE}), candidate.state)
        self.assertEqual(cursor.number(4), 1)
        self.assertEqual(cursor.ref({K_REV}), recorded_base.revision)
        self.assertEqual(authority.base.state, candidate.state)
        self.assertEqual(read_state(exported, candidate.state), selected)
        self.assertEqual(authority.live, live)
        self.assertEqual(authority.recover(capture.receipt)[0], recorded_base)
        self.assertEqual(authority.recover(capture.receipt)[1][NOTES], File(SECRET))
        self.assertNotIn(SECRET, b''.join(exported.values()))

    def test_failed_candidate_test_is_not_rescanned_from_passing_live_files(self):
        authority, capture, candidate = self.fixture(expected=43)
        original_base, authored = authority.base, set(authority.authored)
        authority.live = {MAIN: program(99), TEST: check(99), NOTES: File(SECRET)}
        with tempfile.TemporaryDirectory(prefix='avc-review-') as tmp:
            review, original = Path(tmp) / 'review', Path(tmp) / 'original'
            materialize(read_state(authority.builder.store, candidate.state), review)
            materialize(authority.live, original)
            self.assertEqual(run_check(original).returncode, 0)
            result = run_check(review)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn(b'AssertionError', result.stderr)
        # The caller chooses cancellation; AVC is not a build runner or an
        # automatic test gate. A failed run cannot authorize a different value.
        authority.cancel(candidate.key)
        with self.assertRaisesRegex(Refused, 'cancelled'):
            authority.confirm(candidate.key)
        authority.collect()
        self.assertEqual((authority.base, authority.authored), (original_base, authored))
        self.assertEqual(authority.recover(capture.receipt)[1][MAIN], program(42, SECRET))
        self.assertEqual(authority.live[MAIN], program(99))

    def test_passing_candidate_test_does_not_override_changed_selection(self):
        authority, capture, candidate = self.fixture()
        original_base, authored = authority.base, set(authority.authored)
        with tempfile.TemporaryDirectory(prefix='avc-review-') as tmp:
            review = Path(tmp) / 'review'
            materialize(read_state(authority.builder.store, candidate.state), review)
            result = run_check(review)
            self.assertEqual(result.returncode, 0, result.stderr)
            authority.select(original_base)  # Equal value, newer authority token.
            with self.assertRaisesRegex(Refused, 'selection changed'):
                authority.confirm(candidate.key)
        authority.collect()
        self.assertEqual((authority.base, authority.authored), (original_base, authored))
        self.assertIn(candidate.key, authority.prepared)
        self.assertEqual(authority.recover(capture.receipt)[1][NOTES], File(SECRET))


class InterruptReturnPath(unittest.TestCase):
    def complete(self, repository, plan):
        repository.begin(plan)
        repository.write(plan.key)
        repository.verify(plan.key)
        repository.finish(plan)

    def test_return_preserves_latest_edits_and_survives_interrupted_second_departure(self):
        for base in (Base('base-state', 'R0'), Base('empty')):
            with self.subTest(base=base):
                r = Repository(base)  # B active, independent A already preserved.
                self.complete(r, r.prepare('interrupt-B', destination='A'))
                w = r.workspaces['w1']
                edited_a = replace(w.live, key='A-later', source='later-A-bytes',
                                   origins=(*w.live.origins, ('another-a', 'new')))
                w.live = edited_a
                r.now = 8  # Historical captures expire, not unresumed B.
                r.collect()
                self.assertTrue(r.b.closure() <= r.resident)
                back = r.prepare_back('return-B')
                self.assertEqual(back.target, WorkTarget('B'))
                self.complete(r, back)
                self.assertEqual((w.work, w.base, w.live), ('B', base, r.b))
                self.assertEqual(r.works['A'].saved, edited_a)
                # A second departure encounters an external edit after admission.
                # It must retain A and the admitted B before-image without
                # publishing navigation, retrying blindly or clobbering the edit.
                second = r.prepare_back('return-A')
                r.begin(second)
                edited_b = replace(w.live, key='B-later', source='later-B-bytes')
                w.live = edited_b
                r.write(second.key)
                self.assertEqual(r.operations[second.key].phase, 'incomplete')
                with self.assertRaises(Blocked):
                    r.finish(second)
                with self.assertRaises(Blocked):
                    r.begin(r.prepare('competing-return', destination='A'))
                r.now = 16
                r.collect()
                self.assertEqual((w.work, w.base, w.live), ('B', base, edited_b))
                self.assertEqual(w.previous, WorkTarget('A'))
                self.assertEqual(r.discover(), {'A', 'B'})
                self.assertEqual(r.works['A'].saved, edited_a)
                self.assertTrue(edited_a.closure() | r.b.closure() <= r.resident)
                self.assertNotIn(second.key, r.receipts)


if __name__ == '__main__':
    unittest.main()
