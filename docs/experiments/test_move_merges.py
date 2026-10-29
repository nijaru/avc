"""Recorded source mappings: exact replay, lossless collisions and bounded scope."""
import unittest
from unittest.mock import patch

from codec_reference import Builder
from codec_runtime_checks import commands, corpus, invoke
from codec_verify import Verifier, entry_ref
from operation_reference import MergeTooComplex, UnsupportedRecipe, replay
from protected_inputs import EvidenceUnavailable, NotAuthorized, ProtectedInputs, Refused
from source_query_reference import Known, SourceQueries


class MoveMergeTests(unittest.TestCase):
    def setUp(self):
        self.b = Builder()

    def file(self, data, executable=False):
        return self.b.file(self.b.blob(data), executable)

    def directory(self, tree):
        return self.b.directory(self.b.map({name: self.directory(value) if isinstance(value, dict)
                                          else value for name, value in tree.items()}))

    def state(self, tree):
        return self.b.state(self.directory(tree))

    def protect(self, *roots, authorized=None):
        return ProtectedInputs(self.b.store, set(roots) if authorized is None else authorized, set(roots))

    def evidence(self, base, side, source, destination, old, occupied=None, edits=(), copies=()):
        move = self.b.move_op(source, destination, old, occupied)
        record = self.b.transform_record(base, (move, *copies), edits)
        return self.b.provenance(side, [], [], record)

    def claim(self, base, left, right, result, moves, **options):
        return self.b.provenance(result, [], [], self.b.merge_record(
            source=(base, left, right), moves=moves, **options))

    def conflict(self, base, left, right, region=1):
        return self.b.conflict_entry(self.b.conflict(region, base, [left, right]))

    def root_conflict(self, base, left, right):
        return self.b.state(self.conflict(*(self.b.store[s] for s in (base, left, right))))

    def check(self, base, left, right, expected, moves, extra=(), **options):
        # Source results, including canonical conflict grouping, must be symmetric.
        for a, b, evidence in ((left, right, moves), (right, left, moves[::-1])):
            claim = self.claim(base, a, b, expected, evidence, **options)
            roots = (claim, base, a, b, *(e for e in evidence if e is not None), *extra)
            before = dict(self.b.store)
            result = replay(claim, self.protect(*roots), (base, a, b))
            self.assertEqual(result.state, expected)
            self.assertEqual(self.b.store, before)
            verifier = Verifier(result.objects)
            verifier.state(expected)
            self.assertEqual(set(result.objects), verifier.closure)
        return result

    def test_existing_descendant_edits_deletions_modes_and_symlinks_follow(self):
        old, a, b, merged = map(self.file, (b'a\nb\nc\n', b'A\nb\nc\n', b'a\nb\nC\n', b'A\nb\nC\n'))
        mode, executable = self.file(b'mode'), self.file(b'mode', True)
        link, new_link = (self.b.symlink(self.b.blob(s)) for s in (b'old', b'new'))
        gone, outside_a, outside_b = map(self.file, (b'gone', b'left', b'right'))
        original = {b'f': old, b'm': mode, b'link': link, b'gone': gone, b'empty': {}}
        moving = {**original, b'f': a}
        other = {b'f': b, b'm': executable, b'link': new_link, b'empty': {}}
        base = self.state({b's': original})
        left = self.state({b'd': moving, b'left': outside_a})
        right = self.state({b's': other, b'right': outside_b})
        evidence = self.evidence(base, left, (b's',), (b'd',), self.directory(original),
                                 edits=(((b'd', b'f'), old, a), ((b'left',), None, outside_a)))
        expected = self.state({b'd': {**other, b'f': merged}, b'left': outside_a, b'right': outside_b})
        result = self.check(base, left, right, expected, (evidence, None))
        unresolved = self.conflict(old, a, b, region=0)
        disabled = self.state({b'd': {**other, b'f': unresolved}, b'left': outside_a, b'right': outside_b})
        self.check(base, left, right, disabled, (evidence, None), text_algorithm=(0, 0))
        for _, command in commands('--objects-stdin'):
            self.assertEqual(invoke(command, corpus(result.objects, [expected])), [True])
        with patch('operation_reference.merge_text', side_effect=MemoryError('probe')):
            with self.assertRaises(MemoryError):
                self.check(base, left, right, expected, (evidence, None))

    def test_witnessed_overwrite_and_fresh_source_name_do_not_lose_an_edit(self):
        old, new, occupied, fresh = map(self.file, (b'old', b'new', b'occupied', b'fresh'))
        base = self.state({b's': old, b'd': occupied})
        left = self.state({b'd': old, b's': fresh})
        right = self.state({b's': new, b'd': occupied})
        evidence = self.evidence(base, left, (b's',), (b'd',), old, occupied,
                                 (((b's',), None, fresh),))
        expected = self.state({b'd': new, b's': fresh})
        result = self.check(base, left, right, expected, (evidence, None))
        for ref in (evidence, base, entry_ref(occupied)):
            self.assertNotIn(ref, result.objects)
        # Symlink contents follow an explicit move too; no link is traversed.
        old_link, new_link = (self.b.symlink(self.b.blob(s)) for s in (b'old-link', b'new-link'))
        base, left, right, expected = (self.state(tree) for tree in (
            {b's': old_link}, {b'd': old_link}, {b's': new_link}, {b'd': new_link}))
        evidence = self.evidence(base, left, (b's',), (b'd',), old_link)
        self.check(base, left, right, expected, (evidence, None))

    def test_checked_destination_removal_or_type_change_stays_a_live_alternative(self):
        old, new = self.file(b'old'), self.file(b'new')
        link = self.b.symlink(self.b.blob(b'target'))
        base, right = self.state({b's': old}), self.state({b's': new})
        for replacement in (None, link):
            left = self.state({} if replacement is None else {b'd': replacement})
            evidence = self.evidence(base, left, (b's',), (b'd',), old,
                                     edits=(((b'd',), old, replacement),))
            expected = self.state({b'd': self.conflict(old, replacement, new, region=0)})
            self.check(base, left, right, expected, (evidence, None))

    def test_populated_destination_overwrite_requires_unchanged_descendants(self):
        old, new, occupied, changed = map(self.file, (b'old', b'new', b'occupied', b'changed'))
        source, destination = {b'f': old, b'deleted': old}, {b'keep': occupied}
        base = self.state({b's': source, b'd': destination})
        left = self.state({b'd': source})
        evidence = self.evidence(base, left, (b's',), (b'd',), self.directory(source),
                                 self.directory(destination))
        right = self.state({b's': {b'f': new}, b'd': destination})
        self.check(base, left, right, self.state({b'd': {b'f': new}}), (evidence, None))
        right = self.state({b's': {b'f': new}, b'd': {b'keep': changed}})
        self.check(base, left, right, self.root_conflict(base, left, right), (evidence, None))

    def test_ordered_put_overflow_cannot_be_hidden_before_move_evidence_is_consumed(self):
        value = self.file(b'leaf')
        name = b'x' * 240
        deep = value
        for _ in range(16):
            deep = self.directory({name: deep})
        base, side = self.state({b's': value, name: {}}), self.state({b'd': value, name: {}})
        evidence = self.evidence(base, side, (b's',), (b'd',), value,
                                 edits=(((name, name), None, deep), ((name, name), deep, None)))
        claim = self.claim(base, side, base, side, (evidence, None))
        pool = self.protect(claim, evidence, base, entry_ref(deep))
        # Both direct replay and the merge equality path must validate every view.
        for provenance, selected in ((evidence, (base,)), (claim, (base, side, base))):
            with self.assertRaisesRegex(Refused, 'path constraints'):
                replay(provenance, pool, selected)

    def test_both_sides_same_mapping_merge_edits_and_explicit_name_recreation(self):
        old, a, b, merged = map(self.file, (b'a\nb\nc\n', b'A\nb\nc\n', b'a\nb\nC\n', b'A\nb\nC\n'))
        base = self.state({b's': {b'f': old}})
        left = self.state({b'd': {b'f': a}, b's': {b'left': a}})
        right = self.state({b'd': {b'f': b}, b's': {b'right': b}})
        evidence = tuple(self.evidence(base, side, (b's',), (b'd',), self.directory({b'f': old}),
                                      edits=(((b'd', b'f'), old, value),
                                             ((b's',), None, self.directory({name: value}))))
                         for side, value, name in ((left, a, b'left'), (right, b, b'right')))
        expected = self.state({b'd': {b'f': merged}, b's': {b'left': a, b'right': b}})
        self.check(base, left, right, expected, evidence)

    def test_unsafe_mapping_retains_original_scope_and_merges_outside_it(self):
        old, new, keep, x, y = map(self.file, (b'old', b'new', b'keep', b'x', b'y'))
        original = {b's': {b'f': old}, b'keep': keep, b'inside_left': old, b'inside_right': old}
        moving = {b'd': {b'f': old}, b'keep': keep, b'inside_left': x, b'inside_right': old}
        base = self.state({b'pkg': original, b'x': old, b'y': old})
        left = self.state({b'pkg': moving, b'x': x, b'y': old})
        evidence = self.evidence(base, left, (b'pkg', b's'), (b'pkg', b'd'),
                                 self.directory({b'f': old}),
                                 edits=(((b'x',), old, x), ((b'pkg', b'inside_left'), old, x)))
        cases = [
            {b's': {b'f': new, b'new': new}, b'keep': keep},  # new old-name addition
            {b's': {b'f': {b'child': new}}, b'keep': keep},   # children of a replaced leaf
            {b'keep': keep},                               # whole-source deletion
            {b's': new, b'keep': keep},                     # source type replacement
            {b's': {b'f': new}, b'd': new, b'keep': keep},   # concurrent destination occupation
            None,                                         # enclosing directory deletion
            new,                                          # ancestor type obstruction
        ]
        for other in cases:
            with self.subTest(other=other):
                if isinstance(other, dict):
                    other = {**other, b'inside_left': old, b'inside_right': y}
                tree = {b'x': old, b'y': y}
                if other is not None:
                    tree[b'pkg'] = other
                right = self.state(tree)
                descriptor = self.directory(other) if isinstance(other, dict) else other
                region = 0 if isinstance(other, bytes) else 1
                conflict = self.conflict(self.directory(original), self.directory(moving), descriptor, region)
                expected = self.state({b'pkg': conflict, b'x': x, b'y': y})
                self.check(base, left, right, expected, (evidence, None))

    def test_changed_existing_destination_and_missing_destination_parent_couple(self):
        a, b, c = map(self.file, (b'a', b'b', b'c'))
        for occupied in (None, b):
            original = {b's': a, b'parent': {} if occupied is None else {b'd': occupied}}
            base = self.state(original)
            left = self.state({b'parent': {b'd': a}})
            evidence = self.evidence(base, left, (b's',), (b'parent', b'd'), a, occupied)
            for other in ({b's': c}, {b's': c, b'parent': {b'd': c}}):
                right = self.state(other)
                self.check(base, left, right, self.root_conflict(base, left, right), (evidence, None))

    def test_different_colliding_mappings_preserve_all_paths(self):
        a, b = self.file(b'a'), self.file(b'b')
        cases = [
            ({b's': a}, {b'd': a}, {b'x': a}, ((b's',), (b'd',), a, None), ((b's',), (b'x',), a, None)),
            ({b'a': a, b'b': b}, {b'b': b, b'd': a}, {b'a': a, b'd': b},
             ((b'a',), (b'd',), a, None), ((b'b',), (b'd',), b, None)),
            ({b'a': a, b'b': b}, {b'b': a}, {b'a': a, b'c': b},
             ((b'a',), (b'b',), a, b), ((b'b',), (b'c',), b, None)),
            ({b'a': {b'f': a}, b'b': b}, {b'x': {b'f': a}, b'b': b}, {b'a': {b'f': b}},
             ((b'a',), (b'x',), self.directory({b'f': a}), None), ((b'b',), (b'a', b'f'), b, a)),
        ]
        for original, moving, other, lmap, rmap in cases:
            base, left, right = map(self.state, (original, moving, other))
            evidence = tuple(self.evidence(base, side, s, d, value, occupied)
                             for side, (s, d, value, occupied) in ((left, lmap), (right, rmap)))
            self.check(base, left, right, self.root_conflict(base, left, right), evidence)

    def test_unsupported_composition_and_history_never_fall_back(self):
        a, b = self.file(b'a'), self.file(b'b')
        base = self.state({b'a': a, b'b': b})
        left, right = self.state({b'x': a, b'b': b}), self.state({b'a': a, b'y': b})
        le = self.evidence(base, left, (b'a',), (b'x',), a)
        re = self.evidence(base, right, (b'b',), (b'y',), b)
        for l, r, moves in ((left, right, (le, re)), (right, left, (re, le))):
            claim = self.claim(base, l, r, base, moves)
            with self.assertRaisesRegex(UnsupportedRecipe, 'independent'):
                replay(claim, self.protect(claim, base, l, r, le, re), (base, l, r))
        combined = self.state({b'x': a, b'y': b})
        record = self.b.transform_record(base, (self.b.move_op((b'a',), (b'x',), a, None),
                                              self.b.move_op((b'b',), (b'y',), b, None)))
        evidence = self.b.provenance(combined, [], [], record)
        claim = self.claim(base, combined, base, combined, (evidence, None))
        with self.assertRaisesRegex(UnsupportedRecipe, 'multiple moves'):
            replay(claim, self.protect(claim, evidence, base), (base, combined, base))
        root = self.b.revision(base, [], b'0' * 16, b'fixture', b'', 0)
        tip = self.b.revision(left, [root], b'1' * 16, b'fixture', b'', 1)
        history = self.b.provenance(left, [root, tip], [], self.b.merge_record(history=(0, 1), moves=(None, le)))
        with self.assertRaisesRegex(UnsupportedRecipe, 'history'):
            replay(history, self.protect(history, root, tip, le), (root, tip))

    def test_invalid_evidence_cannot_hide_behind_equal_inputs_or_wrong_roles(self):
        old, new = self.file(b'old'), self.file(b'new')
        base, side = self.state({b's': old}), self.state({b'd': old})
        valid = self.evidence(base, side, (b's',), (b'd',), old)
        claim = self.claim(base, side, base, side, (valid, None))
        for authorized, error in ((None, NotAuthorized), ({claim, base, side, valid}, EvidenceUnavailable)):
            with self.assertRaises(error):
                replay(claim, self.protect(claim, base, side, authorized=authorized), (base, side, base))
        false = self.evidence(base, side, (b's',), (b'd',), old, edits=(((b'd',), old, new),))
        wrong_base = self.evidence(side, side, (b's',), (b'd',), old)
        wrong_side = self.evidence(base, base, (b's',), (b'd',), old)
        for evidence in (false, wrong_base, wrong_side):
            bad = self.claim(base, side, base, side, (evidence, None))
            with self.assertRaises(Refused):
                replay(bad, self.protect(bad, evidence, base, side, entry_ref(new)), (base, side, base))
        with self.assertRaisesRegex(Refused, 'selected inputs'):
            replay(claim, self.protect(claim, valid, base), (base, base, side))
        wrong_result = self.claim(base, side, base, base, (valid, None))
        with self.assertRaisesRegex(Refused, 'bound result'):
            replay(wrong_result, self.protect(wrong_result, valid, base), (base, side, base))
        wrong_role = self.claim(base, base, side, side, (valid, None))
        with self.assertRaisesRegex(Refused, 'exact base and side'):
            replay(wrong_role, self.protect(wrong_role, valid, base), (base, base, side))

    def test_private_copy_intermediate_is_required_but_never_exported(self):
        old, new, secret = map(self.file, (b'old', b'new', b'private intermediate'))
        base, left, right, expected = (self.state(t) for t in (
            {b's': old}, {b'd': old}, {b's': new}, {b'd': new}))
        evidence = self.evidence(base, left, (b's',), (b'd',), old,
                                 copies=(self.b.copy_op(secret, (b'tmp',), None),),
                                 edits=(((b'tmp',), secret, None),))
        claim = self.claim(base, left, right, expected, (evidence, None))
        roots = (claim, evidence, base, left, right)
        with self.assertRaises(NotAuthorized):
            replay(claim, self.protect(*roots), (base, left, right))
        secret_ref = entry_ref(secret)
        with self.assertRaises(EvidenceUnavailable):
            replay(claim, self.protect(*roots, authorized={*roots, secret_ref}), (base, left, right))
        result = self.check(base, left, right, expected, (evidence, None), (secret_ref,))
        self.assertNotIn(secret_ref, result.objects)
        self.assertNotIn(evidence, result.objects)

    def test_opaque_agreement_does_not_authorize_projection_or_flatten_depth(self):
        old, new, a, b = map(self.file, (b'old', b'new', b'a', b'b'))
        base, left = self.state({b's': old}), self.state({b'd': old})
        evidence = self.evidence(base, left, (b's',), (b'd',), old)
        root = self.directory({b's': new, b'x': a})
        other = self.directory({b's': new, b'x': b})
        for depth in range(1, 65):
            root = self.conflict(None, root, other)
            if depth == 63:
                right = self.b.state(root)
                self.assertEqual(SourceQueries(right, self.protect(right)).read((b's',)), Known(new))
                self.check(base, left, right, self.root_conflict(base, left, right), (evidence, None))
        right = self.b.state(root)
        claim = self.claim(base, left, right, base, (evidence, None))
        with self.assertRaisesRegex(MergeTooComplex, 'nesting >64'):
            replay(claim, self.protect(claim, evidence, base, right), (base, left, right))

    def test_old_name_addition_is_not_projected_into_an_invalid_long_path(self):
        old, new = self.file(b'old'), self.file(b'new')
        name = b'x' * 240
        original = {b'f': old}
        base = self.state({b's': original, name: {}})
        left = self.state({name: {name: original}})
        evidence = self.evidence(base, left, (b's',), (name, name), self.directory(original))
        deep = new
        for _ in range(16):
            deep = {name: deep}
        right = self.state({b's': {b'f': new, b'added': deep}, name: {}})
        self.check(base, left, right, self.root_conflict(base, left, right), (evidence, None))


if __name__ == '__main__':
    unittest.main()
