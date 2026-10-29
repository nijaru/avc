"""Agreed reads do not resolve conflicts or grant access to missing evidence."""
import unittest

from codec_reference import Builder
from codec_verify import Invalid, entry_ref
from operation_reference import replay
from protected_inputs import EvidenceUnavailable, NotAuthorized, ProtectedInputs, Refused
from source_query_reference import Known, Obstructed, SourceQueries, Unresolved


class SourceQueryTests(unittest.TestCase):
    def setUp(self):
        self.b = Builder()

    def file(self, content, executable=False):
        return self.b.file(self.b.blob(content), executable)

    def directory(self, values):
        return self.b.directory(self.b.map(values))

    def protect(self, *roots):
        return ProtectedInputs(self.b.store, set(roots), set(roots))

    def reader(self, entry):
        state = self.b.state(entry)
        return SourceQueries(state, self.protect(state))

    def conflict(self, left, right, base=None, region=1):
        return self.b.conflict_entry(self.b.conflict(region, base, [left, right]))

    def test_nested_coupling_agrees_on_live_choices_not_historical_base(self):
        old, shared, a, b, c = map(self.file, (b'old', b'shared', b'a', b'b', b'c'))
        empty = self.directory({})
        link = self.b.symlink(self.b.blob(b'common'))
        sides = [self.directory({b'common': shared, b'varies': v, b'empty': empty, b'link': link})
                 for v in (a, b, c)]
        historical = self.directory({b'common': old})
        inner = self.conflict(*sides[:2], historical)
        root = self.conflict(inner, sides[2], historical)
        state = self.b.state(root)
        before = dict(self.b.store)
        reader = SourceQueries(state, self.protect(state))
        self.assertEqual(reader.read((b'common',)), Known(shared))
        self.assertEqual(reader.read((b'empty',)), Known(empty))
        self.assertEqual(reader.read((b'link',)), Known(link))
        self.assertEqual(reader.read((b'missing', b'child')), Known(None))
        self.assertEqual(reader.read((b'varies',)), Unresolved())
        self.assertEqual(reader.read(()), Known(root))
        self.assertEqual(self.b.store, before)

    def test_absence_is_not_a_file_or_symlink_obstruction(self):
        file = self.file(b'value')
        directory = self.directory({})
        mixed = self.conflict(directory, file, region=0)
        absent = self.conflict(directory, None)
        link = self.b.symlink(self.b.blob(b'target'))
        root = self.directory({b'mixed': mixed, b'absent': absent, b'link': link,
                               b'target': self.directory({b'f': file})})
        reader = self.reader(root)
        self.assertEqual(reader.read((b'mixed', b'nope')), Unresolved())
        self.assertEqual(reader.read((b'absent', b'nope')), Known(None))
        self.assertEqual(reader.read((b'link',)), Known(link))
        self.assertEqual(reader.read((b'link', b'f')), Obstructed(link, (b'f',)))
        same_link = self.conflict(self.directory({b'p': link, b'x': file}),
                                  self.directory({b'p': link}))
        self.assertEqual(self.reader(same_link).read((b'p', b'f')), Obstructed(link, (b'f',)))
        modes = self.conflict(self.directory({b'p': file}),
                              self.directory({b'p': self.file(b'value', True)}))
        self.assertEqual(self.reader(modes).read((b'p',)), Unresolved())

    def test_endpoint_conflicts_keep_opaque_binary_group_identity(self):
        a, b, c = map(self.file, (b'a', b'b', b'c'))
        ab, bc = self.conflict(a, b, region=0), self.conflict(b, c, region=0)
        left, right = self.conflict(ab, c, region=0), self.conflict(a, bc, region=0)
        grouped = self.conflict(self.directory({b'p': left}), self.directory({b'p': right}))
        self.assertEqual(self.reader(grouped).read((b'p',)), Unresolved())
        common = self.conflict(self.directory({b'p': left, b'other': a}),
                               self.directory({b'p': left, b'other': b}))
        self.assertEqual(self.reader(common).read((b'p',)), Known(left))

    def test_full_protection_precedes_answers_and_snapshots_retain_bytes(self):
        a, b = self.file(b'a'), self.file(b'b')
        root = self.conflict(self.directory({b'p': a}), self.directory({b'p': b}))
        state = self.b.state(root)
        objects = dict(self.b.store)
        held = SourceQueries(state, self.protect(state))
        with self.assertRaises(NotAuthorized):
            SourceQueries(state, ProtectedInputs(objects, set(), set()))
        with self.assertRaises(EvidenceUnavailable):
            SourceQueries(state, ProtectedInputs(objects, {state}, set()))
        missing = dict(objects)
        del missing[entry_ref(b)]
        with self.assertRaises(EvidenceUnavailable):
            ProtectedInputs(missing, {state}, {state})
        corrupt = dict(objects)
        corrupt[state] += b'corrupt'
        with self.assertRaisesRegex(Invalid, 'hash mismatch'):
            ProtectedInputs(corrupt, {state}, {state})
        self.b.store.clear()
        self.assertEqual(held.read((b'p',)), Unresolved())
        self.assertEqual(held.read((b'missing',)), Known(None))

    def test_query_authority_follows_owning_edges_not_audit_commitments(self):
        source = self.b.state(self.directory({b'f': self.file(b'public')}))
        audit = self.b.state(self.directory({b'secret': self.file(b'private')}))
        provenance = self.b.provenance(source, [], [], self.b.observation_record(audit))
        pool = self.protect(provenance)
        self.assertIsInstance(SourceQueries(source, pool).read((b'f',)), Known)
        with self.assertRaises(NotAuthorized):
            SourceQueries(audit, pool)

    def test_raw_names_radix_maps_and_complete_request_path_validation(self):
        value = self.file(b'v')
        names = [f'prefix-{i:04d}'.encode() for i in range(150)] + [b'a', b'ab', b'\xff']
        reader = self.reader(self.directory(dict.fromkeys(names, value)))
        for name in names:
            self.assertEqual(reader.read((name,)), Known(value))
        empty = self.reader(self.directory({}))
        longest = (b'x' * 255,) * 15 + (b'y' * 254, b'z')  # 4,096 bytes with slashes
        self.assertEqual(empty.read(longest), Known(None))
        self.assertEqual(empty.read((b'x',) * 256), Known(None))
        for path in ((b'',), (b'.',), (b'..',), (b'a/b',), (b'\0',),
                     (b'x' * 256,), (b'x',) * 257, longest[:-1] + (b'zz',)):
            with self.assertRaises(Invalid):
                empty.read(path)

    def test_deep_agreement_respects_canonical_conflict_depth(self):
        common, a, b = map(self.file, (b'common', b'a', b'b'))
        root = self.directory({b'f': common, b'x': a})
        other = self.directory({b'f': common, b'x': b})
        for _ in range(64):
            root = self.conflict(root, other)
        reader = self.reader(root)
        self.assertEqual(reader.read((b'f',)), Known(common))
        self.assertEqual(reader.read((b'x',)), Unresolved())
        too_deep = self.b.state(self.conflict(root, other))
        with self.assertRaisesRegex(Invalid, 'nesting >64'):
            self.protect(too_deep)

    def test_agreed_query_does_not_authorize_an_edit_through_a_conflict(self):
        old, new, a, b = map(self.file, (b'old', b'new', b'a', b'b'))
        conflict = self.conflict(self.directory({b'f': old, b'x': a}),
                                 self.directory({b'f': old, b'x': b}))
        state = self.b.state(self.directory({b'p': conflict}))
        reader = SourceQueries(state, self.protect(state))
        self.assertEqual(reader.read((b'p', b'f')), Known(old))
        result = self.b.state(self.directory({b'p': self.directory({b'f': new})}))
        record = self.b.transform_record(state, edits=(((b'p', b'f'), old, new),))
        claim = self.b.provenance(result, [], [], record)
        before = dict(self.b.store)
        with self.assertRaisesRegex(Refused, 'opaque'):
            replay(claim, self.protect(claim, state), (state,))
        self.assertEqual(self.b.store, before)


if __name__ == '__main__':
    unittest.main()
