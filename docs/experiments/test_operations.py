"""Source computation, protected evidence and observation admission contracts."""
import random
import unittest
from unittest.mock import patch

from codec_reference import Builder, K_REV, Ref
from codec_records import Observation
from codec_verify import Verifier
from codec_runtime_checks import commands, corpus, invoke
from operation_reference import (
    EvidenceUnavailable, MergeTooComplex, NotAuthorized, ObservationFacts,
    ProtectedInputs, Refused, UnsupportedRecipe, check_move_evidence,
    qualify_authoring, replay,
)


class OperationTests(unittest.TestCase):
    def setUp(self):
        self.b = Builder()

    def file(self, content, executable=False):
        return self.b.file(self.b.blob(content), executable)

    def state(self, entries):
        def directory(values):
            return self.b.directory(self.b.map({name: directory(v) if isinstance(v, dict) else v
                                               for name, v in values.items()}))
        return self.b.state(directory(entries))

    def revision(self, state, parents=()):
        # Fixture identity, no implication of a native random ChangeId allocator.
        n = sum(ref.kind == K_REV for ref in self.b.store)
        return self.b.revision(state, list(parents), n.to_bytes(16, 'little'), b'fixture', b'', n)

    def provenance(self, result, record, roles=(), observation=None):
        return self.b.provenance(result, list(roles), [], record, observation)

    def protect(self, *roots, authorized=None):
        return ProtectedInputs(self.b.store, set(roots) if authorized is None else set(authorized), set(roots))

    def observation(self, provenance):
        verifier = Verifier(self.b.store)
        verifier.generic(provenance)
        return next(r for r in verifier.provenances[provenance].records if isinstance(r, Observation))

    def source_merge(self, base, left, right, result, **options):
        record = self.b.merge_record(source=(base, left, right), **options)
        return self.provenance(result, record)

    def admit(self, provenance, roots, facts, selected):
        return qualify_authoring(provenance, self.protect(provenance, *roots), facts,
                                 selected, authoring_base=facts.observation.base)

    def test_structural_batch_then_ordered_exact_leaf_edits(self):
        old, intermediate, new = map(self.file, (b'old', b'intermediate', b'new'))
        subtree = self.b.directory(self.b.map({b'f': old, b'empty': self.b.directory(self.b.map({}))}))
        base = self.state({b'src': subtree, b'copy': old})
        result = self.state({b'lib': {b'f': new, b'empty': {}}, b'copy': subtree})
        structural = (self.b.move_op((b'src',), (b'lib',), subtree, None),
                      self.b.copy_op(subtree, (b'copy',), old))
        edits = (((b'lib', b'f'), old, intermediate), ((b'lib', b'f'), intermediate, new))
        prov = self.provenance(result, self.b.transform_record(base, structural, edits))
        pool = self.protect(prov, base, Ref(1, intermediate[2:34]))
        checked = replay(prov, pool, (base,))
        self.assertEqual(checked.state, result)
        Verifier(checked.objects).state(result)
        self.assertNotIn(base, checked.objects)
        for _, runtime in commands('--objects-stdin'):
            self.assertEqual(invoke(runtime, corpus(checked.objects, [result])), [True])
        # Leaf witnesses are against the computed view, not the old src path.
        wrong = self.provenance(result, self.b.transform_record(base, structural,
                                (((b'src', b'f'), old, new),)))
        with self.assertRaises(Refused):
            replay(wrong, self.protect(wrong, base), (base,))
        # A later failed condition cannot publish the preceding valid move.
        stale = self.provenance(result, self.b.transform_record(base,
                                (structural[0], self.b.copy_op(subtree, (b'copy',), None))))
        before = dict(self.b.store)
        with self.assertRaises(Refused):
            replay(stale, self.protect(stale, base), (base,))
        self.assertEqual(self.b.store, before)

    def test_aliases_overlaps_swaps_and_missing_parents_refuse(self):
        value = self.file(b'x')
        sub = self.b.directory(self.b.map({b'f': value}))
        base = self.state({b'a': sub, b'b': sub, b'occupied': value})
        cases = [
            (self.b.move_op((b'a',), (b'a',), sub, sub),),
            (self.b.move_op((b'a',), (b'a', b'inner'), sub, None),),
            (self.b.move_op((b'a',), (b'b',), sub, sub),
             self.b.move_op((b'b',), (b'a',), sub, sub)),
            (self.b.copy_op(sub, (b'new',), None), self.b.copy_op(sub, (b'new', b'child'), None)),
            (self.b.move_op((b'a',), (b'absent', b'child'), sub, None),),
            (self.b.copy_op(sub, (b'occupied',), None),),
        ]
        for structural in cases:
            with self.subTest(structural=structural):
                prov = self.provenance(base, self.b.transform_record(base, structural))
                with self.assertRaises(Refused):
                    replay(prov, self.protect(prov, base), (base,))

    def test_relocated_descendant_bound_cannot_be_hidden_by_leaf_removal(self):
        name = b'x' * 240
        value = self.file(b'leaf')
        subtree = value
        for _ in range(16):
            subtree = self.b.directory(self.b.map({name: subtree}))
        parent = self.b.directory(self.b.map({}))
        base = self.state({b's': subtree, b'd' * 240: parent})
        destination = (b'd' * 240, b'e' * 240)
        structural = (self.b.move_op((b's',), destination, subtree, None),)
        # Final source would be valid after deletion, but the structural view is not.
        result = self.state({b'd' * 240: {}})
        prov = self.provenance(result, self.b.transform_record(base, structural,
                               ((destination, subtree, None),)))
        with self.assertRaisesRegex(Refused, 'path constraints'):
            replay(prov, self.protect(prov, base), (base,))

    def test_canonical_claim_is_not_base_binding_or_result_truth(self):
        a, b = self.file(b'a'), self.file(b'b')
        base, result = self.state({b'f': a}), self.state({b'f': b})
        record = self.b.transform_record(base, edits=(((b'f',), a, b),))
        forged = self.provenance(base, record)
        Verifier(self.b.store).generic(forged)  # legitimate stored *claim*
        with self.assertRaisesRegex(Refused, 'bound result'):
            replay(forged, self.protect(forged, base, result), (base,))
        valid = self.provenance(result, record)
        with self.assertRaisesRegex(Refused, 'selected input'):
            replay(valid, self.protect(valid, base), (result,))
        other_side = self.state({b'f': a, b'extra': b})
        with self.assertRaises(Refused):
            replay(self.source_merge(base, result, other_side, result),
                   self.protect(base, result, other_side,
                                self.source_merge(base, result, other_side, result)),
                   (base, other_side, result))

    def test_audit_copy_source_needs_read_grant_and_separate_protection(self):
        secret = self.file(b'private copy input')
        base = self.state({})
        # Intermediate copy is removed again: result ownership cannot retain it.
        structural = (self.b.copy_op(secret, (b'temporary',), None),)
        prov = self.provenance(base, self.b.transform_record(base, structural,
                               (((b'temporary',), secret, None),)))
        secret_ref = Ref(1, secret[2:34])
        with self.assertRaises(NotAuthorized):
            ProtectedInputs(self.b.store, {prov, base}, {prov, base, secret_ref})
        with self.assertRaises(NotAuthorized):
            replay(prov, self.protect(prov, base), (base,))
        with self.assertRaises(EvidenceUnavailable):
            replay(prov, self.protect(prov, base, authorized={prov, base, secret_ref}), (base,))
        self.assertEqual(replay(prov, self.protect(prov, base, secret_ref), (base,)).state, base)

    def test_evidence_expiry_does_not_break_source_or_existing_input_handle(self):
        old, new = self.file(b'old'), self.file(b'new')
        base, result = self.state({b'f': old}), self.state({b'f': new})
        prov = self.provenance(result, self.b.transform_record(base, edits=(((b'f',), old, new),)))
        held = self.protect(prov, base)
        v = Verifier(self.b.store)
        v.generic(prov)
        exported = {ref: self.b.store[ref] for ref in v.closure}
        self.assertNotIn(base, exported)
        Verifier(exported).generic(prov)
        with self.assertRaises(EvidenceUnavailable):
            ProtectedInputs(exported, {prov, base}, {prov, base})
        self.assertEqual(replay(prov, held, (base,)).state, result)

    def test_source_merge_types_text_and_unknown_versions(self):
        a, b, c = map(self.file, (b'a\nb\nc\n', b'A\nb\nc\n', b'a\nb\nC\n'))
        base, left, right = (self.state({b'f': value}) for value in (a, b, c))
        result = self.state({b'f': self.file(b'A\nb\nC\n')})
        prov = self.source_merge(base, left, right, result)
        self.assertEqual(replay(prov, self.protect(prov, base, left, right),
                                (base, left, right)).state, result)
        # Disabled text is equality/base only, not an implicit default driver.
        cf = self.b.conflict_entry(self.b.conflict(0, a, [b, c]))
        conflicted = self.state({b'f': cf})
        disabled = self.source_merge(base, left, right, conflicted, text_algorithm=(0, 0))
        self.assertEqual(replay(disabled, self.protect(disabled, base, left, right),
                                (base, left, right)).state, conflicted)
        for options in ({'source_algorithm': (9, 1)}, {'text_algorithm': (1, 2)}):
            # Unknown versions must refuse even when an equality shortcut could
            # accidentally produce the claimed result without running a driver.
            unknown = self.source_merge(base, left, left, left, **options)
            Verifier(self.b.store).generic(unknown)
            with self.assertRaises(UnsupportedRecipe):
                replay(unknown, self.protect(unknown, base, left), (base, left, left))
        before = dict(self.b.store)
        with patch('operation_reference.merge_text', side_effect=MemoryError('probe')):
            with self.assertRaises(MemoryError):
                replay(prov, self.protect(prov, base, left, right), (base, left, right))
        self.assertEqual(self.b.store, before)
        # New different directories do not union over an old non-directory.
        ldir, rdir = self.state({b'x': b}), self.state({b'y': c})
        dir_l, dir_r = self.b.store[ldir], self.b.store[rdir]
        collision = self.b.conflict_entry(self.b.conflict(0, a, [dir_l, dir_r]))
        bstate, lstate, rstate = (self.state({b'entry': value}) for value in (a, dir_l, dir_r))
        output = self.state({b'entry': collision})
        p = self.source_merge(bstate, lstate, rstate, output)
        self.assertEqual(replay(p, self.protect(p, bstate, lstate, rstate),
                                (bstate, lstate, rstate)).state, output)

    def test_nontext_types_and_nested_conflicts_remain_opaque_and_bounded(self):
        cases = [
            (self.file(b'a'), self.file(b'left', True), self.file(b'right')),
            tuple(self.b.symlink(self.b.blob(data)) for data in (b'a', b'b', b'c')),
            tuple(self.file(data) for data in (b'a\x00', b'b\x00', b'c\x00')),
        ]
        for old, left, right in cases:
            states = tuple(self.state({b'f': value}) for value in (old, left, right))
            cf = self.b.conflict_entry(self.b.conflict(0, old, [left, right]))
            result = self.state({b'f': cf})
            p = self.source_merge(*states, result)
            self.assertEqual(replay(p, self.protect(p, *states), states).state, result)
        old, a, b = map(self.file, (b'base', b'a', b'b'))
        nested = a
        for depth in range(1, 65):
            nested = self.b.conflict_entry(self.b.conflict(0, None, [nested, b]))
            if depth == 63:
                left63 = nested
        for left, accepted in ((left63, True), (nested, False)):
            states = tuple(self.state({b'f': value}) for value in (old, left, b))
            if accepted:
                expected_cf = self.b.conflict_entry(self.b.conflict(0, old, [left, b]))
                expected = self.state({b'f': expected_cf})
            else:
                expected = states[0]  # valid claim; execution must refuse BEFORE binding
            p = self.source_merge(*states, expected)
            pool = self.protect(p, *states)
            if accepted:
                self.assertEqual(replay(p, pool, states).state, expected)
            else:
                with self.assertRaisesRegex(MergeTooComplex, 'nesting >64'):
                    replay(p, pool, states)

    def test_move_evidence_requires_exact_replayed_base_and_side(self):
        old, new = self.file(b'old'), self.file(b'new')
        base, side = self.state({b's': old}), self.state({b'd': new})
        record = self.b.transform_record(base, (self.b.move_op((b's',), (b'd',), old, None),),
                                         (((b'd',), old, new),))
        prov = self.provenance(side, record)
        pool = self.protect(prov, base)
        self.assertEqual(check_move_evidence(prov, pool, base, side).state, side)
        for expected_base, expected_side in ((side, side), (base, base)):
            with self.assertRaises(Refused):
                check_move_evidence(prov, pool, expected_base, expected_side)
        lie = self.provenance(base, record)
        with self.assertRaises(Refused):
            check_move_evidence(lie, self.protect(lie, base, side), base, base)
        merge = self.source_merge(base, side, base, side, moves=(prov, None))
        with self.assertRaises(UnsupportedRecipe):
            replay(merge, self.protect(merge, prov, base), (base, side, base))

    def test_history_criss_cross_frontier_virtual_ancestry_and_unrelated_permission(self):
        a, b, c = map(self.file, (b'a', b'b', b'c'))
        empty, sa, sb, sab = self.state({}), self.state({b'a': a}), self.state({b'b': b}), self.state({b'a': a, b'b': b})
        root = self.revision(empty)
        ra, rb = self.revision(sa, (root,)), self.revision(sb, (root,))
        # Two incomparable common ancestors; their reduced base contains BOTH edits.
        rleft = self.revision(sab, (ra, rb))
        sright = self.state({b'a': a, b'b': b, b'c': c})
        rright = self.revision(sright, (rb, ra))
        p = self.provenance(sright, self.b.merge_record(history=(0, 1)), (rleft, rright))
        checked = replay(p, self.protect(p, rleft, rright), (rleft, rright))
        self.assertEqual(checked.state, sright)
        self.assertEqual(checked.frontiers[0], tuple(sorted((ra, rb))))
        self.assertIn((root,), checked.frontiers)
        self.assertFalse(any(ref.kind == K_REV for ref in checked.objects))
        # Even with common tips, reducing unrelated bases requires the same permission.
        ua, ub = self.revision(sa), self.revision(sb)
        ul, ur = self.revision(sab, (ua, ub)), self.revision(sright, (ub, ua))
        refused = self.provenance(sright, self.b.merge_record(history=(0, 1)), (ul, ur))
        with self.assertRaisesRegex(Refused, 'unrelated'):
            replay(refused, self.protect(refused, ul, ur), (ul, ur))
        permitted = self.provenance(sright, self.b.merge_record(history=(0, 1), allow_unrelated=True), (ul, ur))
        with self.assertRaises(Refused):
            replay(permitted, self.protect(permitted, ul, ur), (ul, ur))
        answer = replay(permitted, self.protect(permitted, ul, ur), (ul, ur), permit_unrelated=True)
        self.assertEqual(answer.state, sright)
        self.assertIn((), answer.frontiers)

    def test_history_frontier_three_bases_and_complexity_refusal(self):
        # A third frontier reduction must retain the ancestry of the first virtual pair.
        states = [self.state({bytes([name]): self.file(bytes([name]))}) for name in b'abc']
        root = self.revision(self.state({}))
        bases = [self.revision(s, (root,)) for s in states]
        combined = self.state({bytes([name]): self.file(bytes([name])) for name in b'abc'})
        left, right = self.revision(combined, bases), self.revision(combined, tuple(reversed(bases)))
        p = self.provenance(combined, self.b.merge_record(history=(0, 1)), (left, right))
        pool = self.protect(p, left, right)
        checked = replay(p, pool, (left, right))
        self.assertEqual(checked.state, combined)
        self.assertEqual(checked.frontiers[0], tuple(sorted(bases)))
        # Every recursive pair still shares the root; no silent empty base is needed.
        self.assertTrue(all(frontier for frontier in checked.frontiers))
        empty = self.state({})
        anchor = self.revision(empty)
        leaves = [self.revision(empty, (anchor,)) for _ in range(126)]
        wide_left = self.revision(empty, leaves[:63])
        wide_right = self.revision(empty, leaves[63:])
        wide = self.provenance(empty, self.b.merge_record(history=(0, 1)), (wide_left, wide_right))
        with self.assertRaisesRegex(MergeTooComplex, 'node budget'):
            replay(wide, self.protect(wide, wide_left, wide_right), (wide_left, wide_right))

    def test_required_observation_truth_and_base_revision_binding(self):
        old, new = self.file(b'old'), self.file(b'new')
        base, result = self.state({b'f': old}), self.state({b'f': new})
        revision = self.revision(base)
        observation = self.b.observation_record(base, base_revision=revision, scope=((b'f',),), covered=((b'f',),))
        record = self.b.transform_record(base, edits=(((b'f',), old, new),))
        p = self.provenance(result, record, observation=observation)
        facts = ObservationFacts(result, self.observation(p))
        self.assertEqual(self.admit(p, (base, revision), facts, (base,)), result)
        with self.assertRaisesRegex(UnsupportedRecipe, 'no filesystem replay'):
            only = self.provenance(result, observation)
            replay(only, self.protect(only, base, revision), (base,))
        for altered in (
            None,
            self.b.observation_record(base, base_revision=revision, consistency=2, scope=((b'f',),), covered=((b'f',),)),
            self.b.observation_record(base, base_revision=revision, scope=((b'f',),), coverage_mode=1),
            self.b.observation_record(base, base_revision=revision, partial=True, scope=((b'f',),), failed=((b'f',),)),
        ):
            forged = self.provenance(result, record, observation=altered)
            with self.assertRaises(Refused):
                self.admit(forged, (base, revision), facts, (base,))
        # Ordinary directory/type replacement is not a conflict resolution.
        directory_base = self.state({b'dir': {b'child': old}})
        replacement = self.state({b'dir': new})
        ordinary = self.provenance(replacement, self.b.observation_record(directory_base))
        self.assertEqual(self.admit(ordinary, (directory_base,),
                                   ObservationFacts(replacement, self.observation(ordinary)),
                                   (directory_base,)), replacement)
        # Matching a trusted label does not bypass the independent Revision->State check.
        wrong_revision = self.revision(result)
        wrong_observation = self.b.observation_record(base, base_revision=wrong_revision)
        bad = self.provenance(result, wrong_observation)
        with self.assertRaisesRegex(Refused, 'another State'):
            self.admit(bad, (base, wrong_revision), ObservationFacts(result, self.observation(bad)), (base,))

    def test_changed_conflicts_need_exact_explicit_whole_unit_choices(self):
        a, b, c = map(self.file, (b'a', b'b', b'c'))
        conflict = self.b.conflict(0, a, [b, c])
        base = self.state({b'f': self.b.conflict_entry(conflict)})
        marker = self.file(b'<<<<<<< a\nnot an implicit resolution\n=======\n>>>>>>> b\n')
        for choice, output in ((marker, {b'f': marker}), (None, {})):
            result = self.state(output)
            no_choice = self.provenance(result, self.b.observation_record(base, scope=((b'f',),), covered=((b'f',),)))
            with self.assertRaisesRegex(Refused, 'explicit whole-unit'):
                self.admit(no_choice, (base,), ObservationFacts(result, self.observation(no_choice)), (base,))
            for chosen in (choice, b if choice is None else None):
                p = self.provenance(result, self.b.observation_record(base, scope=((b'f',),), covered=((b'f',),),
                                    resolutions=(((b'f',), conflict, chosen),)))
                facts = ObservationFacts(result, self.observation(p))
                if chosen == choice:
                    self.assertEqual(self.admit(p, (base,), facts, (base,)), result)
                else:
                    with self.assertRaises(Refused):
                        self.admit(p, (base,), facts, (base,))
        left, right = self.state({b'left': a}), self.state({b'right': b})
        root_cf = self.b.conflict(1, None, [self.b.store[left], self.b.store[right]])
        root_base = self.b.state(self.b.conflict_entry(root_cf))
        for chosen in (self.b.store[left], None):
            p = self.provenance(left, self.b.observation_record(root_base, resolutions=(((), root_cf, chosen),)))
            facts = ObservationFacts(left, self.observation(p))
            if chosen is None:
                with self.assertRaises(Refused):
                    self.admit(p, (root_base,), facts, (root_base,))
            else:
                self.assertEqual(self.admit(p, (root_base,), facts, (root_base,)), left)

    def test_seeded_conditional_edits_match_independent_flat_values(self):
        rng = random.Random(711)
        values = {}
        base = self.state(values)
        for step in range(40):
            name = bytes([ord('a') + rng.randrange(8)])
            old = values.get(name)
            new = None if old is not None and rng.randrange(3) == 0 else self.file(step.to_bytes(2, 'little'))
            expected = dict(values)
            if new is None:
                del expected[name]
            else:
                expected[name] = new
            result = self.state(expected)
            prov = self.provenance(result, self.b.transform_record(base, edits=(((name,), old, new),)))
            checked = replay(prov, self.protect(prov, base), (base,))
            self.assertEqual(checked.state, result)
            # Build the same sorted map from a different insertion order.
            reversed_state = self.state(dict(reversed(list(expected.items()))))
            self.assertEqual(checked.state, reversed_state)
            values, base = expected, result


if __name__ == '__main__':
    unittest.main(verbosity=2)
