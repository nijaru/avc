"""Exact marker bindings and explicit resolution; no native filesystem claims."""
from dataclasses import replace
from hashlib import sha256
import unittest
from unittest.mock import patch

import conflict_presentation_reference as presentation
from conflict_presentation_reference import (
    Absent, Context, File, NoPresentation, Unknown, capture, prepare, resolve,
)
from codec_reference import Builder
from codec_runtime_checks import commands, corpus, invoke
from codec_verify import Invalid, Verifier, entry_ref
from operation_reference import Evaluation, MergeTooComplex, lookup, replay
from protected_inputs import EvidenceUnavailable, NotAuthorized, ProtectedInputs, Refused
from source_query_reference import Known, SourceQueries


class ConflictPresentationTests(unittest.TestCase):
    def setUp(self):
        self.b = Builder()
        self.terms = [self.file(data, True) for data in
                      (b'old\r\n', b'left\n<<<<<<< literal\n', b'right without newline')]
        self.conflict = self.b.conflict(0, self.terms[0], self.terms[1:])
        self.entry = self.b.conflict_entry(self.conflict)
        self.state = self.directory_state({b'f': self.entry, b'g': self.entry})
        self.context = Context(self.state, b'private destination identity', b'private work', 3, 7)
        self.inputs = self.protect(self.state)
        self.manifest = prepare(self.inputs, self.context, (b'f',))

    def file(self, data, executable=False):
        return self.b.file(self.b.blob(data), executable)

    def directory_state(self, entries):
        return self.b.state(self.b.directory(self.b.map(entries)))

    def protect(self, *roots):
        return ProtectedInputs(self.b.store, set(roots), set(roots))

    def observe(self, value, manifest=None):
        return capture(self.inputs, self.context, (b'f',), value, manifest)

    def check_resolution(self, captured, result, value):
        inputs = ProtectedInputs(result.objects, {result.provenance}, {result.provenance})
        claim = inputs.provenance(result.provenance)
        observation = claim.records[1]
        expected = None if isinstance(value, Absent) else self.file(value.data, value.executable)
        self.assertEqual(observation.base, captured.context.state)
        self.assertEqual(observation.generation, captured.context.selection_generation)
        self.assertEqual(observation.scope, (captured.path,))
        self.assertEqual(observation.covered, (captured.path,))
        self.assertEqual(observation.resolutions, ((captured.path, self.conflict, expected),))
        self.assertEqual(claim.inputs, ())
        self.assertIsNone(observation.receipt)
        actual = lookup(Evaluation(inputs).state(result.state), captured.path)
        self.assertEqual(actual, expected)
        self.assertEqual(result.state, claim.result)
        verifier = Verifier(result.objects)
        verifier.generic(result.provenance)
        self.assertEqual(set(result.objects), verifier.closure)
        self.assertNotIn(captured.context.state, result.objects)
        # g still owns the shared Conflict; use the one-conflict case below to
        # independently check that resolutions do not retain their audit inputs.
        combined = dict(captured.objects) | dict(result.objects)
        protected = ProtectedInputs(combined, {captured.context.state, result.provenance},
                                    {captured.context.state, result.provenance})
        self.assertEqual(replay(result.provenance, protected, (captured.context.state,)).state,
                         result.state)
        for runtime, command in commands('--objects-stdin'):
            with self.subTest(runtime=runtime):
                self.assertEqual(invoke(command, corpus(result.objects, [result.provenance])), [True])

    def test_unchanged_exact_binding_keeps_opaque_conflict_and_owned_base(self):
        before = dict(self.b.store)
        observed = self.manifest.generated
        captured = self.observe(observed, self.manifest)
        self.assertEqual(captured.disposition, 'conflict')
        self.assertEqual(captured.entry, self.entry)
        self.assertEqual(captured.context.state, self.state)
        self.assertEqual(captured.observed, observed)
        self.assertEqual(captured.manifest, self.manifest)
        verifier = Verifier(captured.objects)
        verifier.state(self.state)
        self.assertEqual(verifier.conflicts[self.conflict].base, self.terms[0])
        self.assertEqual(set(captured.objects), verifier.closure)
        self.assertEqual(dict(self.b.store), before)
        self.assertEqual(prepare(self.inputs, self.context, [b'f']), self.manifest)
        # A protected snapshot is independent of mutable caller residency.
        self.b.store.clear()
        self.assertEqual(prepare(ProtectedInputs(captured.objects, {self.state}, {self.state}),
                                 captured.context, (b'f',)), self.manifest)

    def test_every_binding_dimension_and_forged_projection_fail_closed(self):
        edited = File(self.manifest.generated.data + b'editor change', True)
        other_conflict = self.b.conflict(0, None, self.terms[1:])
        foreign_state = self.directory_state({b'f': self.entry})
        manifests = [None,
            replace(self.manifest, context=replace(self.context, state=foreign_state)),
            replace(self.manifest, context=replace(self.context, destination=b'another inode')),
            replace(self.manifest, context=replace(self.context, work=b'another work')),
            replace(self.manifest, context=replace(self.context, selection_generation=4)),
            replace(self.manifest, context=replace(self.context, materialization_generation=8)),
            prepare(self.inputs, self.context, (b'g',)),
            replace(self.manifest, conflict=other_conflict),
            replace(self.manifest, codec='future-unknown'),
            replace(self.manifest, generated=File(b'')),
            replace(self.manifest, checksum=b''),
            replace(self.manifest, generated=edited, checksum=sha256(edited.data).digest())]
        for manifest in manifests:
            with self.subTest(manifest=manifest):
                for value in (self.manifest.generated, edited, Absent()):
                    captured = self.observe(value, manifest)
                    self.assertEqual(captured.disposition, 'pending')
                    self.assertEqual(captured.entry, self.entry)
                    self.assertEqual(captured.observed, value)
                    self.assertIn(self.conflict, captured.objects)
                    self.assertIsNone(captured.manifest)
        # The same Conflict/bytes in the same destination do not refresh a stale
        # generation or establish a binding for another path/work automatically.
        for context in (replace(self.context, selection_generation=4),
                        replace(self.context, materialization_generation=8),
                        replace(self.context, destination=b'another inode'),
                        replace(self.context, work=b'another work')):
            self.assertEqual(capture(self.inputs, context, (b'f',),
                                     self.manifest.generated, self.manifest).disposition, 'pending')

    def test_changes_and_deletion_need_exact_confirmation_and_context(self):
        for value in (File(b'resolved\0binary\r\n', False),
                      replace(self.manifest.generated, executable=False), Absent()):
            with self.subTest(value=value):
                captured = self.observe(value, self.manifest)
                self.assertEqual(captured.disposition, 'pending')
                self.assertEqual(captured.entry, self.entry)
                before = dict(self.b.store)
                result = resolve(captured, self.context, value)
                self.assertEqual(dict(self.b.store), before)
                self.check_resolution(captured, result, value)
                for context in (replace(self.context, selection_generation=4),
                                replace(self.context, materialization_generation=8),
                                replace(self.context, state=self.directory_state({})),
                                replace(self.context, destination=b'different destination'),
                                replace(self.context, work=b'different work')):
                    with self.assertRaisesRegex(Refused, 'stale'):
                        resolve(captured, context, value)
                for confirmation in (File(b'later editor bytes'),
                                     File(bytearray(value.data), value.executable)
                                     if isinstance(value, File) else File(b'')):
                    with self.assertRaisesRegex(Refused, 'exact known'):
                        resolve(captured, self.context, confirmation)
        # Explicitly choosing marker-looking bytes is legal; content heuristics
        # cannot replace or override the user's exact whole-entry choice.
        captured = self.observe(self.manifest.generated, self.manifest)
        self.check_resolution(captured, resolve(captured, self.context, captured.observed),
                              captured.observed)

    def test_ordinary_marker_lookalikes_are_literal_unknown_is_not_deletion(self):
        for source in ({}, {b'f': self.file(b'ordinary')},
                       {b'f': self.file(self.manifest.generated.data)}):
            state = self.directory_state(source)
            context = replace(self.context, state=state)
            inputs = self.protect(state)
            for value in (self.manifest.generated, File(b'<<<<<<< user text\n'), Absent()):
                captured = capture(inputs, context, (b'f',), value, self.manifest)
                self.assertEqual(captured.disposition, 'literal')
                self.assertEqual(captured.observed, value)
                with self.assertRaisesRegex(Refused, 'whole nonroot'):
                    resolve(captured, context, value)
            unknown = capture(inputs, context, (b'f',), Unknown('permission denied'))
            self.assertEqual(unknown.disposition, 'pending')
            with self.assertRaisesRegex(Refused, 'exact known'):
                resolve(unknown, context, Absent())
        for value in (Unknown('watcher gap'), Unknown('read failure')):
            captured = self.observe(value, self.manifest)
            self.assertEqual(captured.disposition, 'pending')
            self.assertEqual(captured.entry, self.entry)
            self.assertEqual(captured.observed, value)
        directory = self.b.directory(self.b.map({b'child': self.terms[0]}))
        state = self.directory_state({b'f': directory})
        self.assertEqual(capture(self.protect(state), replace(self.context, state=state),
                                 (b'f',), Absent()).disposition, 'pending')

    def test_agreed_descendant_is_not_a_resolution_unit(self):
        common = self.file(b'agreed but not write-authorized')
        left = self.b.directory(self.b.map({b'f': common, b'left': self.terms[0]}))
        right = self.b.directory(self.b.map({b'f': common, b'right': self.terms[1]}))
        coupled = self.b.conflict(1, None, [left, right])
        state = self.b.state(self.b.conflict_entry(coupled))
        context, inputs = replace(self.context, state=state), self.protect(state)
        self.assertEqual(SourceQueries(state, inputs).read((b'f',)), Known(common))
        captured = capture(inputs, context, (b'f',), File(b'changed'))
        self.assertEqual(captured.unit, ())
        self.assertEqual(captured.entry, self.b.conflict_entry(coupled))
        self.assertEqual(captured.disposition, 'pending')
        with self.assertRaisesRegex(Refused, 'whole nonroot'):
            resolve(captured, context, captured.observed)
        root = capture(inputs, context, (), File(b'not a root directory'))
        with self.assertRaisesRegex(Refused, 'whole nonroot'):
            resolve(root, context, root.observed)
        with self.assertRaises(NoPresentation):
            prepare(inputs, context, ())

    def test_no_marker_required_for_binary_nested_type_or_deleted_alternatives(self):
        binary = self.file(b'\0binary')
        invalid_utf8 = self.file(b'\xff')
        directory = self.b.directory(self.b.map({b'child': self.terms[0]}))
        cases = ((self.terms[0], [binary, self.terms[1]]),
                 (self.terms[0], [invalid_utf8, self.terms[1]]),
                 (self.terms[0], [self.entry, self.terms[1]]),
                 (self.terms[0], [directory, self.terms[1]]),
                 (self.terms[0], [None, self.terms[1]]),
                 (self.terms[0], [self.file(b'mode', False), self.terms[1]]))
        for base, alternatives in cases:
            conflict = self.b.conflict(0, base, alternatives)
            state = self.directory_state({b'f': self.b.conflict_entry(conflict)})
            context, inputs = replace(self.context, state=state), self.protect(state)
            with self.subTest(conflict=conflict):
                with self.assertRaises(NoPresentation):
                    prepare(inputs, context, (b'f',))
                captured = capture(inputs, context, (b'f',), File(b'explicit replacement'))
                self.assertEqual(captured.disposition, 'pending')
                self.assertEqual(captured.entry, self.b.conflict_entry(conflict))
                result = resolve(captured, context, captured.observed)
                self.assertNotIn(conflict, result.objects)
                self.assertNotIn(state, result.objects)
                claim = ProtectedInputs(result.objects, {result.provenance},
                                        {result.provenance}).provenance(result.provenance)
                self.assertEqual(claim.records[1].resolutions[0][1], conflict)
                self.assertIn(conflict, captured.objects)  # old choices still inspectable privately

    def test_protection_and_invalid_observations_never_guess_source(self):
        for objects, error in (({}, EvidenceUnavailable),
                               (dict(self.b.store) | {self.state: b'corrupt'}, Invalid)):
            with self.assertRaises(error):
                ProtectedInputs(objects, {self.state}, {self.state})
        unacquired = ProtectedInputs(self.b.store, {self.state}, set())
        with self.assertRaises(EvidenceUnavailable):
            capture(unacquired, self.context, (b'f',), self.manifest.generated, self.manifest)
        unauthorized = ProtectedInputs(self.b.store, set(), set())
        with self.assertRaises(NotAuthorized):
            capture(unauthorized, self.context, (b'f',), Absent())
        for value in (None, File(bytearray(b'mutable')), File(b'bytes', 1), Unknown([])):
            with self.assertRaises(Refused):
                self.observe(value)
        for path in ((b'..',), (b'',), (b'slash/name',)):
            with self.assertRaises(Invalid):
                capture(self.inputs, self.context, path, Absent())
        for context in (replace(self.context, destination=b''), replace(self.context, work=b''),
                        replace(self.context, selection_generation=-1),
                        replace(self.context, materialization_generation=2**64)):
            with self.assertRaises(Refused):
                capture(self.inputs, context, (b'f',), Absent())

    def test_cancellation_and_failed_qualification_publish_nothing(self):
        captured = self.observe(File(b'confirmed exact bytes'), self.manifest)
        before, protected = dict(self.b.store), dict(self.inputs.objects)
        for error in (MemoryError, KeyboardInterrupt, EvidenceUnavailable):
            with patch.object(presentation, 'prepare', side_effect=error):
                with self.assertRaises(error):
                    self.observe(self.manifest.generated, self.manifest)
        for error in (MemoryError, KeyboardInterrupt, MergeTooComplex, Refused):
            with patch.object(presentation, 'qualify_authoring', side_effect=error):
                with self.assertRaises(error):
                    resolve(captured, self.context, captured.observed)
        original = Builder.observation_record
        def wrong_choice(*args, **kwargs):
            path, conflict, _ = kwargs['resolutions'][0]
            return original(*args, **(kwargs | {'resolutions': ((path, conflict, None),)}))
        with patch.object(Builder, 'observation_record', side_effect=wrong_choice):
            with self.assertRaisesRegex(Refused, 'differs from observing authority'):
                resolve(captured, self.context, captured.observed)
        self.assertEqual(dict(self.b.store), before)
        self.assertEqual(dict(self.inputs.objects), protected)
        self.assertEqual(captured.entry, self.entry)


if __name__ == '__main__':
    unittest.main()
