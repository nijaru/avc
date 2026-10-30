"""Revision/source identity and lineage checks, not native history commands."""
import itertools
import unittest
from unittest.mock import patch

from codec_reference import Builder, Ref, K_REV
from codec_records import Merge, Transform
from codec_runtime_checks import commands, corpus, invoke
from codec_verify import Invalid, Verifier, entry_ref
from operation_reference import Evaluation, MergeTooComplex, UnsupportedRecipe, replay
from protected_inputs import EvidenceUnavailable, NotAuthorized, ProtectedInputs, Refused
from revision_evolution_reference import (
    Evolution, Lineage, Metadata, amend, read_revision, reapply, rebase, rebase_stack, revert,
)

META = Metadata(b'fixture author', b'explicit operation', -123)


class RevisionEvolutionTests(unittest.TestCase):
    def setUp(self):
        self.b = Builder()
        self.counter = 0

    def change(self):
        self.counter += 1
        return self.counter.to_bytes(16, 'little')

    def file(self, data):
        return self.b.file(self.b.blob(data))

    def state(self, entries):
        return self.b.state(self.b.directory(self.b.map(entries)))

    def revision(self, state, parents=(), *, change=None):
        return self.b.revision(state, list(parents), self.change() if change is None else change,
                               META.author, META.description, META.unix_ns)

    def protect(self, *roots, authorized=()):
        return ProtectedInputs(self.b.store, set(roots) | set(authorized), set(roots))

    def decoded(self, result):
        protected = ProtectedInputs(result.objects, {result.revision}, {result.revision})
        return read_revision(protected, result.revision), protected

    def claim(self, result):
        revision, protected = self.decoded(result)
        return protected.provenance(revision.provenance)

    def check_export(self, result):
        verifier = Verifier(result.objects)
        verifier.generic(result.revision)
        self.assertEqual(set(result.objects), verifier.closure)
        for runtime, command in commands('--objects-stdin'):
            with self.subTest(runtime=runtime):
                self.assertEqual(invoke(command, corpus(result.objects, [result.revision])), [True])

    def test_amend_keeps_ordered_parents_and_change_but_not_private_predecessor(self):
        public = self.state({b'f': self.file(b'public')})
        p, q = self.revision(public), self.revision(public)
        secret = self.b.blob(b'discarded-private-source-9341')
        private = self.state({b'f': self.file(b'public'), b'secret': self.b.file(secret)})
        old = self.revision(private, (q, p))
        descendant = self.revision(private, (old,))
        before = dict(self.b.store)
        inputs = self.protect(old, descendant, public)
        result = amend(inputs, old, public, metadata=Metadata(b'new author', b'cleaned', 456))
        revised, protected = self.decoded(result)
        self.assertEqual(revised.parents, (q, p))
        self.assertEqual(revised.change, read_revision(inputs, old).change)
        self.assertEqual(revised.state, public)
        self.assertNotEqual(result.revision, old)
        self.assertEqual(result.evolution, Evolution((old,), (result.revision,), 'amend'))
        claim = self.claim(result)
        self.assertEqual(claim.inputs, (old,))
        self.assertIsInstance(claim.records[0], Transform)
        self.assertEqual(claim.records[0].base, public)
        self.assertEqual(replay(revised.provenance, protected, (public,)).state, public)
        self.assertTrue({p, q}.issubset(result.objects))
        self.assertTrue({old, descendant, private, secret}.isdisjoint(result.objects))
        self.assertEqual(self.b.store, before)
        # Retained metadata does not acquire its unavailable predecessor bytes.
        snapshot = Lineage().append(result.evolution)
        self.assertEqual(snapshot.events[0].predecessors, (old,))
        expired = ProtectedInputs(result.objects, {result.revision, old}, {result.revision})
        with self.assertRaises(EvidenceUnavailable):
            read_revision(expired, old)
        self.check_export(result)
        # If that old revision really IS a parent, privacy cannot hide history.
        actual_child = self.revision(private, (old,))
        retained = amend(self.protect(actual_child, public), actual_child, public)
        self.assertTrue({old, private, secret}.issubset(retained.objects))
        self.check_export(retained)

    def test_noop_and_divergent_amendments_do_not_conflate_source_and_history(self):
        state = self.state({b'f': self.file(b'value')})
        old = self.revision(state)
        inputs = self.protect(old)
        noop = amend(inputs, old, state)
        self.assertEqual(noop.revision, old)
        self.assertIsNone(noop.evolution)
        outputs = [amend(inputs, old, state, metadata=Metadata(b'author', text, META.unix_ns))
                   for text in (b'one description', b'another description')]
        self.assertEqual(len({old, *(out.revision for out in outputs)}), 3)
        for out in outputs:
            decoded, _ = self.decoded(out)
            self.assertEqual(decoded.state, state)
            self.assertEqual(decoded.parents, ())
            self.assertEqual(decoded.change, read_revision(inputs, old).change)
        lineage = Lineage().append(*(out.evolution for out in outputs))
        self.assertEqual(len(lineage.events), 2)
        self.check_export(outputs[0])

    def test_binary_formula_table_matches_manual_three_way_oracle(self):
        values = (None, self.file(b'a\0'), self.file(b'b\0'))
        for p, c, q in itertools.product(values, repeat=3):
            states = [self.state({b'f': value} if value is not None else {}) for value in (p, c, q)]
            parent = self.revision(states[0])
            subject = self.revision(states[1], (parent,))
            onto = self.revision(states[2])
            protected = self.protect(subject, onto)
            for operation, roles in ((rebase, (p, q, c)), (revert, (c, q, p)), (reapply, (p, q, c))):
                with self.subTest(operation=operation.__name__, values=(p, c, q)):
                    base, left, right = roles
                    if left == right:
                        expected = left
                    elif base == left:
                        expected = right
                    elif base == right:
                        expected = left
                    else:
                        expected = self.b.conflict_entry(self.b.conflict(0, base, [left, right]))
                    expected_state = self.state({b'f': expected} if expected is not None else {})
                    extra = {} if operation is rebase else {'new_change': self.change(), 'metadata': META}
                    before = dict(self.b.store)
                    result = operation(protected, subject, onto, **extra)
                    self.assertEqual(self.b.store, before)
                    decoded, _ = self.decoded(result)
                    self.assertEqual(decoded.state, expected_state)
                    self.assertEqual(decoded.parents, (onto,))
                    if operation is rebase:
                        self.assertEqual(decoded.change, read_revision(protected, subject).change)
                    else:
                        self.assertEqual(decoded.change, extra['new_change'])
                        self.assertIsNone(result.evolution)
                    source = ((states[1], states[2], states[0]) if operation is revert
                              else (states[0], states[2], states[1]))
                    claim = self.claim(result)
                    self.assertIsInstance(claim.records[0], Merge)
                    self.assertEqual(claim.records[0].source, source)
                    self.assertEqual(claim.inputs, (subject, parent, onto))

    def test_revert_then_reapply_is_new_authorship_not_seen_change_suppression(self):
        initial = self.state({b'f': self.file(b'a\nb\nc\n')})
        changed = self.state({b'f': self.file(b'A\nb\nc\n')})
        concurrent = self.state({b'f': self.file(b'a\nb\nC\n')})
        combined = self.state({b'f': self.file(b'A\nb\nC\n')})
        parent = self.revision(initial)
        original = self.revision(changed, (parent,))
        onto = self.revision(concurrent, (parent,))
        rebased = rebase(self.protect(original, onto), original, onto)
        decoded, _ = self.decoded(rebased)
        self.assertEqual(decoded.state, combined)
        self.b.store.update(rebased.objects)
        before = dict(self.b.store)
        undone = revert(self.protect(original, rebased.revision), original, rebased.revision,
                        new_change=self.change(), metadata=META)
        self.assertEqual(self.decoded(undone)[0].state, concurrent)
        self.assertEqual(self.b.store, before)
        self.b.store.update(undone.objects)
        repeated = reapply(self.protect(original, undone.revision), original, undone.revision,
                           new_change=self.change(), metadata=META)
        restored, _ = self.decoded(repeated)
        self.assertEqual(restored.state, combined)
        self.assertEqual(restored.parents, (undone.revision,))
        revisions = (original, rebased.revision, undone.revision, repeated.revision)
        self.assertEqual(len(set(revisions)), 4)
        self.assertNotEqual(restored.change, self.decoded(undone)[0].change)
        self.assertNotEqual(restored.change, read_revision(self.protect(original), original).change)
        pool = dict(self.b.store) | dict(repeated.objects)
        protected = ProtectedInputs(pool, {original, undone.revision, repeated.revision},
                                    {original, undone.revision, repeated.revision})
        self.assertEqual(replay(restored.provenance, protected, (initial, concurrent, changed)).state, combined)
        self.check_export(repeated)

    def test_inverse_of_conflicted_source_preserves_nested_binary_terms(self):
        base, prior, proposed, current = map(self.file, (b'base\0', b'prior\0', b'proposed\0', b'current\0'))
        inner = self.b.conflict_entry(self.b.conflict(0, base, [prior, proposed]))
        parent = self.revision(self.state({b'f': prior}))
        subject = self.revision(self.state({b'f': inner}), (parent,))
        onto = self.revision(self.state({b'f': current}))
        expected = self.b.conflict_entry(self.b.conflict(0, inner, [current, prior]))
        expected_state = self.state({b'f': expected})
        result = revert(self.protect(subject, onto), subject, onto, new_change=self.change(), metadata=META)
        self.assertEqual(self.decoded(result)[0].state, expected_state)
        verifier = Verifier(result.objects)
        verifier.generic(result.revision)
        self.assertEqual(verifier.conflicts[entry_ref(expected)].base, inner)
        self.assertEqual(set(verifier.conflicts[entry_ref(inner)].alternatives), {prior, proposed})
        self.check_export(result)

    def test_merge_revert_records_explicit_revision_mainline_even_for_equal_states(self):
        state = self.state({b'f': self.file(b'old')})
        p, q = self.revision(state), self.revision(state)
        subject = self.revision(self.state({b'f': self.file(b'new')}), (q, p))
        inputs = self.protect(subject)
        options = {'new_change': self.change(), 'metadata': META}
        with self.assertRaisesRegex(Refused, 'explicit mainline'):
            revert(inputs, subject, subject, **options)
        with self.assertRaisesRegex(Refused, 'not a parent'):
            revert(inputs, subject, subject, mainline=subject, **options)
        # Alternative recipes at one deliberately fixed candidate identity.
        outputs = [revert(inputs, subject, subject, mainline=parent, **options) for parent in (p, q)]
        self.assertNotEqual(outputs[0].revision, outputs[1].revision)
        for parent, result in zip((p, q), outputs):
            decoded, _ = self.decoded(result)
            self.assertEqual(decoded.state, state)
            self.assertEqual(decoded.parents, (subject,))
            self.assertEqual(self.claim(result).inputs, (subject, parent))
            self.check_export(result)

    def test_selected_stack_maps_only_selected_parents_and_exports_only_outputs(self):
        pstate = self.state({b'p': self.file(b'parent')})
        parent = self.revision(pstate)
        cstate = self.state({b'p': self.file(b'parent'), b'c': self.file(b'change')})
        c = self.revision(cstate, (parent,))
        dstate = self.state({b'p': self.file(b'parent'), b'c': self.file(b'change'), b'd': self.file(b'next')})
        d = self.revision(dstate, (c,))
        secret = self.b.blob(b'unselected descendant 93753')
        e = self.revision(self.state({b'e': self.b.file(secret)}), (d,))
        qstate = self.state({b'p': self.file(b'parent'), b'q': self.file(b'other branch')})
        onto = self.revision(qstate, (parent,))
        granted_only = self.state({b'private': self.file(b'authorized but not acquired')})
        inputs = self.protect(e, onto, authorized=(granted_only,))
        before = dict(self.b.store)
        result = rebase_stack(inputs, (c, d), onto)
        self.assertEqual(self.b.store, before)
        cnew, dnew = (result.replacements[ref] for ref in (c, d))
        self.assertTrue({cnew, dnew}.issubset(result.objects))
        protected = ProtectedInputs(result.objects, {dnew}, {dnew})
        first, last = (read_revision(protected, ref) for ref in (cnew, dnew))
        self.assertEqual(first.parents, (onto,))
        self.assertEqual(last.parents, (cnew,))
        self.assertEqual((first.change, last.change), tuple(read_revision(inputs, ref).change for ref in (c, d)))
        expected = self.state({b'p': self.file(b'parent'), b'c': self.file(b'change'),
                               b'd': self.file(b'next'), b'q': self.file(b'other branch')})
        self.assertEqual(last.state, expected)
        self.assertTrue({c, d, e, secret, granted_only}.isdisjoint(result.objects))
        self.assertEqual(len(Lineage().append(*result.evolution).events), 2)
        self.assertEqual(dict(inputs.objects), {ref: before[ref] for ref in inputs.objects})
        self.assertEqual(inputs.pins, {e, onto})
        for selection in ((d, c), (c, e), (c, c), ()):
            with self.assertRaises(Refused):
                rebase_stack(inputs, selection, onto)
        noop = rebase_stack(inputs, (c, d), parent)
        self.assertEqual(dict(noop.replacements), {c: c, d: d})
        self.assertEqual(noop.evolution, ())
        for _, command in commands('--objects-stdin'):
            self.assertEqual(invoke(command, corpus(result.objects, [dnew])), [True])

    def test_undefined_operations_and_reused_authored_change_ids_refuse(self):
        state = self.state({})
        p, q = self.revision(state), self.revision(state)
        child = self.revision(state, (p,))
        merge = self.revision(state, (p, q))
        inputs = self.protect(child, merge)
        before = dict(self.b.store)
        for subject in (p, merge):
            for operation in (rebase, reapply):
                extra = {} if operation is rebase else {'new_change': self.change(), 'metadata': META}
                with self.assertRaises(UnsupportedRecipe):
                    operation(inputs, subject, q, **extra)
        with self.assertRaises(UnsupportedRecipe):
            revert(inputs, p, q, new_change=self.change(), metadata=META)
        for change in (None, b'short', *(read_revision(inputs, ref).change for ref in (p, q, child))):
            for operation in (revert, reapply):
                with self.assertRaises(Refused):
                    operation(inputs, child, q, new_change=change, metadata=META)
        with self.assertRaises(UnsupportedRecipe):
            rebase(inputs, child, p, text=(1, 99))  # even a no-op cannot bless an unknown driver
        self.assertEqual(self.b.store, before)

    def test_unavailable_corrupt_and_unprotected_inputs_never_become_empty_history(self):
        state = self.state({b'f': self.file(b'bytes')})
        p = self.revision(state)
        c = self.revision(state, (p,))
        q = self.revision(state)
        unacquired = self.state({b'new': self.file(b'unacquired')})
        before = dict(self.b.store)
        for inputs, exception in ((ProtectedInputs(self.b.store, {c, q}, set()), EvidenceUnavailable),
                                  (self.protect(q), NotAuthorized)):
            with self.assertRaises(exception):
                rebase(inputs, c, q)
        missing = dict(self.b.store)
        del missing[p]
        with self.assertRaises(EvidenceUnavailable):
            ProtectedInputs(missing, {c, q}, {c, q})
        corrupt = dict(self.b.store)
        corrupt[p] = corrupt[p] + b'wrong'
        with self.assertRaises(Invalid):
            ProtectedInputs(corrupt, {c, q}, {c, q})
        with self.assertRaises(EvidenceUnavailable):
            amend(self.protect(c, authorized=(unacquired,)), c, unacquired)
        self.assertEqual(self.b.store, before)

    def test_computation_failures_publish_nothing_even_after_a_stack_prefix(self):
        a, b, c = map(self.file, (b'a\nb\nc\n', b'A\nb\nc\n', b'a\nb\nC\n'))
        p, q = self.revision(self.state({b'f': a})), self.revision(self.state({b'f': c}))
        first = self.revision(self.state({b'f': b}), (p,))
        second = self.revision(self.state({b'f': b, b'g': self.file(b'second')}), (first,))
        inputs = self.protect(second, q)
        before, protected_before = dict(self.b.store), dict(inputs.objects)
        for exception in (MemoryError('allocation'), KeyboardInterrupt('cancel')):
            with patch('operation_reference.merge_text', side_effect=exception):
                with self.assertRaises(type(exception)):
                    rebase(inputs, first, q)
        original = Evaluation.merge_sources
        calls = 0

        def fail_second(evaluation, *args):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise MergeTooComplex('stack probe budget')
            return original(evaluation, *args)

        with patch.object(Evaluation, 'merge_sources', fail_second):
            with self.assertRaises(MergeTooComplex):
                rebase_stack(inputs, (first, second), q)
        self.assertEqual(calls, 2)
        self.assertEqual(self.b.store, before)
        self.assertEqual(dict(inputs.objects), protected_before)
        # Real complete-State bound: a 64-deep opaque conflict cannot be wrapped.
        leaf = self.file(b'binary\0')
        nested = leaf
        for _ in range(64):
            nested = self.b.conflict_entry(self.b.conflict(0, None, [nested, None]))
        parent = self.revision(self.state({b'f': leaf}))
        subject = self.revision(self.state({b'f': nested}), (parent,))
        onto = self.revision(self.state({b'f': self.file(b'other\0')}))
        before = dict(self.b.store)
        with self.assertRaisesRegex(MergeTooComplex, 'nesting >64'):
            revert(self.protect(subject, onto), subject, onto, new_change=self.change(), metadata=META)
        self.assertEqual(self.b.store, before)

    def test_lineage_is_atomic_many_to_many_metadata_even_with_unavailable_endpoints(self):
        # None of these IDs has payload bytes. Metadata must neither fetch nor pin.
        a, b, c, d, e, f, g = (Ref(K_REV, bytes([i]) * 32) for i in range(1, 8))
        events = (Evolution((a,), (b,), 'amend'), Evolution((a,), (c,), 'rebase'),
                  Evolution((b, c), (d,), 'fold'), Evolution((d,), (e, f), 'split'))
        original = Lineage().append(*events)
        self.assertEqual(original.events, events)
        with self.assertRaisesRegex(Refused, 'cycle'):
            original.append(Evolution((e,), (g,), 'amend'), Evolution((g,), (a,), 'rebase'))
        self.assertEqual(original.events, events)
        for event in (Evolution((a,), (a,), 'amend'), Evolution((g, g), (a,), 'fold'),
                      Evolution((), (a,), 'split'), Evolution((a,), (b,), 'revert')):
            with self.assertRaises(Refused):
                original.append(event)
        with self.assertRaisesRegex(Refused, 'cycle'):
            Lineage((Evolution((a,), (b,), 'amend'), Evolution((b,), (a,), 'rebase')))
        with self.assertRaisesRegex(MergeTooComplex, 'event budget'):
            original.append(*(events[0] for _ in range(129)))
        many = tuple(Ref(K_REV, i.to_bytes(32, 'little')) for i in range(100, 165))
        with self.assertRaisesRegex(MergeTooComplex, 'edge budget'):
            Lineage().append(Evolution(many[:32], many[32:], 'split'))
        self.assertEqual(original.events, events)


if __name__ == '__main__':
    unittest.main()
