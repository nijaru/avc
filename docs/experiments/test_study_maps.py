"""Protect cost-accounting contracts, not measured byte/timing snapshots."""
from hashlib import sha256
import unittest

from map_access_study import Reader, build, files
from map_layout_study import Layout


def entry(data):
    return b'\x01\x01' + sha256(data).digest() + b'\x00'


class MapStudyContracts(unittest.TestCase):
    def test_byte_name_lookup_and_per_operation_object_reuse(self):
        tree = {b'a': entry(b'a'), b'ab': entry(b'ab'), b'\xff': entry(b'raw'),
                b'dir': {b'file': entry(b'nested')}, b'empty': {}}
        for limit in (None, 4096, 0):
            layout = Layout(limit)
            root = build(layout, tree)
            for path, value in files(tree):
                reader = Reader(layout.objects)
                self.assertEqual(reader.lookup(root, path), value)
                cost = reader.metrics()
                self.assertGreater(cost['objects_read'], 0)
                self.assertEqual(reader.lookup(root, path), value)
                self.assertEqual(reader.metrics(), cost)
            reader = Reader(layout.objects)
            for path in ((b'missing',), (b'dir', b'missing'), (b'a', b'not-a-directory')):
                self.assertIsNone(reader.lookup(root, path))

    def test_merkle_diff_matches_flat_values_across_shape_changes(self):
        before = {b'aa': entry(b'aa'), b'ab': entry(b'ab')}
        after = {**before, b'z': entry(b'z'),
                 **{f'long-name-{i:04d}'.encode(): entry(str(i).encode()) for i in range(180)}}
        after[b'aa'] = entry(b'changed')
        for limit in (None, 4096, 0):
            a, b = Layout(limit), Layout(limit)
            left, right = build(a, before), build(b, after)
            reader = Reader(a.objects | b.objects)
            actual = {path: (old, new) for path, old, new in reader.diff(left, right)}
            aa, bb = dict(files(before)), dict(files(after))
            expected = {path: (aa.get(path), bb.get(path)) for path in aa.keys() | bb.keys()
                        if aa.get(path) != bb.get(path)}
            self.assertEqual(actual, expected)
            if limit is not None:
                self.assertGreater(reader.metrics()['shape_fallbacks'], 0)
            equal = Reader(a.objects)
            self.assertEqual(list(equal.diff(left, left)), [])
            self.assertEqual(equal.metrics()['objects_read'], 0)

    def test_move_reuses_maps_but_checks_every_relocated_binding(self):
        subtree = {b'file': entry(b'data'), b'empty': {}}
        for _ in range(16):
            subtree = {b'x' * 240: subtree}
        for limit in (None, 4096, 0):
            layout = Layout(limit)
            root = build(layout, subtree)
            reader = Reader(layout.objects)
            self.assertEqual(reader.validate_relocation(root, (b's',)), 19)
            # The same immutable subtree becomes invalid under a longer prefix.
            with self.assertRaises(AssertionError):
                Reader(layout.objects).validate_relocation(root, (b'd' * 240, b'e' * 240))


if __name__ == '__main__':
    unittest.main()
