"""Multiple recorded mappings: shared coordinates and closed original scopes."""
import unittest

from codec_runtime_checks import commands, corpus, invoke
from move_test_support import MoveFixture
from operation_reference import MergeTooComplex, replay
from protected_inputs import Refused


class MoveCompositionTests(MoveFixture, unittest.TestCase):
    def test_independent_mappings_merge_both_edits_and_keep_fresh_old_names(self):
        old, first, last, merged = map(self.file, (
            b'a\nb\nc\n', b'A\nb\nc\n', b'a\nb\nC\n', b'A\nb\nC\n'))
        mode, executable = self.file(b'mode'), self.file(b'mode', True)
        fresh_left, fresh_right = {b'left': first}, {b'right': last}
        original = {b'a': {b'f': old, b'g': mode}, b'b': {b'f': old}}
        base = self.state(original)
        left = self.state({b'x': {b'f': first, b'g': mode}, b'b': {b'f': first}, b'a': fresh_left})
        right = self.state({b'a': {b'f': last, b'g': executable}, b'y': {b'f': last}, b'b': fresh_right})
        le = self.evidence(base, left, (b'a',), (b'x',), self.directory(original[b'a']), edits=(
            ((b'x', b'f'), old, first), ((b'b', b'f'), old, first),
            ((b'a',), None, self.directory(fresh_left))))
        re = self.evidence(base, right, (b'b',), (b'y',), self.directory(original[b'b']), edits=(
            ((b'y', b'f'), old, last), ((b'a', b'f'), old, last),
            ((b'a', b'g'), mode, executable), ((b'b',), None, self.directory(fresh_right))))
        expected = self.state({b'x': {b'f': merged, b'g': executable}, b'y': {b'f': merged},
                               b'a': fresh_left, b'b': fresh_right})
        checked = self.check(base, left, right, expected, (le, re))
        for _, command in commands('--objects-stdin'):
            self.assertEqual(invoke(command, corpus(checked.objects, [expected])), [True])

    def test_partially_shared_batches_are_independent_of_structural_array_order(self):
        old, first, last, merged = map(self.file, (
            b'a\nb\nc\n', b'A\nb\nc\n', b'a\nb\nC\n', b'A\nb\nC\n'))
        new_b, new_c = self.file(b'new b'), self.file(b'new c')
        original = {b'a': {b'f': old}, b'b': old, b'c': old}
        base = self.state(original)
        fresh_left, fresh_right = {b'l': first}, {b'r': last}
        left = self.state({b'x': {b'f': first}, b'y': old, b'c': new_c, b'a': fresh_left})
        right = self.state({b'x': {b'f': last}, b'b': new_b, b'z': old, b'a': fresh_right})
        common = ((b'a',), (b'x',), self.directory(original[b'a']), None)
        lmoves = (common, ((b'b',), (b'y',), old, None))
        rmoves = (common, ((b'c',), (b'z',), old, None))
        ledits = (((b'x', b'f'), old, first), ((b'c',), old, new_c),
                  ((b'a',), None, self.directory(fresh_left)))
        redits = (((b'x', b'f'), old, last), ((b'b',), old, new_b),
                  ((b'a',), None, self.directory(fresh_right)))
        expected = self.state({b'x': {b'f': merged}, b'y': new_b, b'z': new_c,
                               b'a': {**fresh_left, **fresh_right}})
        for lorder, rorder in ((lmoves, rmoves), (lmoves[::-1], rmoves),
                               (lmoves, rmoves[::-1]), (lmoves[::-1], rmoves[::-1])):
            le = self.batch_evidence(base, left, lorder, ledits)
            re = self.batch_evidence(base, right, rorder, redits)
            self.check(base, left, right, expected, (le, re))

    def test_separate_unsafe_scopes_preserve_original_siblings_and_compose_outside(self):
        old, new, a, b = map(self.file, (b'old\x00', b'new\x00', b'left', b'right'))
        names = (b'p', b'q', b'r')
        original = {name: {b's': {b'f': old}, b'inside_left': old, b'inside_right': old}
                    for name in names}
        moving = {name: {b'd': {b'f': old}, b'inside_left': a, b'inside_right': old}
                  for name in names}
        other = {name: {b's': {b'f': new, **({b'added': new} if name != b'q' else {})},
                        b'inside_left': old, b'inside_right': b} for name in names}
        base = self.state(original)
        left, right = self.state({**moving, b'left': a}), self.state({**other, b'right': b})
        moves = tuple(((name, b's'), (name, b'd'), self.directory({b'f': old}), None)
                      for name in names)
        edits = tuple(((name, b'inside_left'), old, a) for name in names) + (((b'left',), None, a),)
        evidence = self.batch_evidence(base, left, moves, edits)
        expected = {b'q': {b'd': {b'f': new}, b'inside_left': a, b'inside_right': b},
                    b'left': a, b'right': b}
        for name in (b'p', b'r'):
            expected[name] = self.conflict(*(self.directory(tree[name])
                                             for tree in (original, moving, other)))
        self.check(base, left, right, self.state(expected), (evidence, None))

    def test_scope_closure_reaches_crossing_mappings_until_a_fixed_point(self):
        old, new = self.file(b'old\x00'), self.file(b'new\x00')
        source = {b'f': old}
        base = self.state({b'pkg': {b'p': {b's': source, b'b': old}, b'q': {}, b'c': old}, b'outer': {}})
        left = self.state({b'pkg': {b'p': {b'd': source}, b'q': {b'd': old}}, b'outer': {b'y': old}})
        right = self.state({b'pkg': {b'p': {b's': {b'f': new, b'added': new}, b'b': new},
                                    b'q': {}, b'c': new}, b'outer': {}})
        evidence = self.batch_evidence(base, left, (
            ((b'pkg', b'p', b's'), (b'pkg', b'p', b'd'), self.directory(source), None),
            ((b'pkg', b'p', b'b'), (b'pkg', b'q', b'd'), old, None),
            ((b'pkg', b'c'), (b'outer', b'y'), old, None)))
        self.check(base, left, right, self.root_conflict(base, left, right), (evidence, None))

    def test_colliding_mapping_group_does_not_absorb_disjoint_safe_mapping(self):
        a, b, old, new = map(self.file, (b'a\x00', b'b\x00', b'old\x00', b'new\x00'))
        original = {b'pkg': {b'src': a, b'mid': b}, b'other': {b'src': old}}
        moving = {b'pkg': {b'mid': a}, b'other': {b'dst': old}}
        other = {b'pkg': {b'src': a, b'dst': b}, b'other': {b'src': new}}
        base, left, right = (self.state(tree) for tree in (original, moving, other))
        le = self.batch_evidence(base, left, (
            ((b'pkg', b'src'), (b'pkg', b'mid'), a, b),
            ((b'other', b'src'), (b'other', b'dst'), old, None)))
        re = self.evidence(base, right, (b'pkg', b'mid'), (b'pkg', b'dst'), b,
                           edits=(((b'other', b'src'), old, new),))
        conflict = self.conflict(*(self.directory(tree[b'pkg']) for tree in (original, moving, other)))
        expected = self.state({b'pkg': conflict, b'other': {b'dst': new}})
        self.check(base, left, right, expected, (le, re))

    def test_every_batch_member_is_replayed_before_source_equality(self):
        old, wrong = self.file(b'old'), self.file(b'wrong')
        source = {b'f': old}
        base = self.state({b'a': source, b'b': old})
        side = self.state({b'x': source, b'y': old})
        first = ((b'a',), (b'x',), self.directory(source), None)
        for second, message in (
                (((b'b',), (b'y',), wrong, None), 'stale source'),
                (((b'b',), (b'y',), old, wrong), 'stale destination'),
                (((b'a', b'f'), (b'y',), old, None), 'overlapping structural')):
            with self.subTest(message=message):
                evidence = self.batch_evidence(base, side, (first, second))
                claim = self.claim(base, side, base, side, (evidence, None))
                with self.assertRaisesRegex(Refused, message):
                    replay(claim, self.protect(claim, base, evidence, self.b.blob(b'wrong')),
                           (base, side, base))

    def test_pair_and_closure_budgets_refuse_without_using_available_claimed_results(self):
        old = self.file(b'old')
        left_names = [(f'ls{i:03}'.encode(), f'ld{i:03}'.encode()) for i in range(100)]
        right_names = [(f'rs{i:03}'.encode(), f'rd{i:03}'.encode()) for i in range(101)]
        original = {source: old for source, _ in left_names + right_names}
        moving = {**{name: old for name, _ in right_names}, **{name: old for _, name in left_names}}
        other = {**{name: old for name, _ in left_names}, **{name: old for _, name in right_names}}
        base, left, right = (self.state(tree) for tree in (original, moving, other))
        expected = self.state({name: old for _, name in left_names + right_names})
        evidence = tuple(self.batch_evidence(base, side,
                         tuple(((s,), (d,), old, None) for s, d in names))
                         for side, names in ((left, left_names), (right, right_names)))
        cases = [(base, left, right, expected, evidence)]
        # No opposite-side pairs: 100 separate unsafe scopes exhaust closure
        # comparisons alone (100 safety checks + 100 * 100 closure checks).
        names = [f'pkg{i:03}'.encode() for i in range(100)]
        original = {name: {b's': {b'f': old}} for name in names}
        moving = {name: {b'd': {b'f': old}} for name in names}
        other = {name: {b's': {b'f': old, b'added': old}} for name in names}
        base, left, right = (self.state(tree) for tree in (original, moving, other))
        evidence = self.batch_evidence(base, left, tuple(
            ((name, b's'), (name, b'd'), self.directory({b'f': old}), None) for name in names))
        expected = self.state({name: self.conflict(*(self.directory(tree[name])
                              for tree in (original, moving, other))) for name in names})
        cases.append((base, left, right, expected, (evidence, None)))
        for base, left, right, expected, evidence in cases:
            for a, b, moves in ((left, right, evidence), (right, left, evidence[::-1])):
                claim = self.claim(base, a, b, expected, moves)
                before = dict(self.b.store)
                with self.assertRaisesRegex(MergeTooComplex, 'mapping reference planning budget'):
                    replay(claim, self.protect(claim, base, a, b, *(m for m in moves if m is not None)),
                           (base, a, b))
                self.assertEqual(self.b.store, before)

    def test_all_three_unit_mapping_ownership_masks_match_a_flat_binary_oracle(self):
        sources, destinations = (b'a', b'b', b'c'), (b'x', b'y', b'z')
        old = [self.file(b'old\x00' + bytes([i])) for i in range(3)]
        changed = [[self.file(side + b'\x00' + bytes([i])) for i in range(3)]
                   for side in (b'left', b'right')]
        base = self.state(dict(zip(sources, old)))
        conflicts = [self.conflict(old[i], changed[0][i], changed[1][i], region=0) for i in range(3)]
        for lmask in range(8):
            for rmask in range(8):
                with self.subTest(left=lmask, right=rmask):
                    sides, evidence = [], []
                    for mask, values in zip((lmask, rmask), changed):
                        names = [destinations[i] if mask & (1 << i) else sources[i] for i in range(3)]
                        side = self.state(dict(zip(names, values)))
                        moves = tuple(((sources[i],), (destinations[i],), old[i], None)
                                      for i in range(3) if mask & (1 << i))
                        edits = tuple(((names[i],), old[i], values[i]) for i in range(3))
                        sides.append(side)
                        evidence.append(self.batch_evidence(base, side, moves, edits) if mask else None)
                    names = [destinations[i] if (lmask | rmask) & (1 << i) else sources[i] for i in range(3)]
                    expected = self.state(dict(zip(names, conflicts)))
                    self.check(base, *sides, expected, tuple(evidence))


if __name__ == '__main__':
    unittest.main()
