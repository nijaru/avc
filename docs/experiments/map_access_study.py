"""Logical lookup/diff/move costs on pinned public names and controlled trees.

Run: python docs/experiments/map_access_study.py --repository /path/to/git.git
Uses Layout's SHA-256 surrogate, not AVC IDs. Whole-object cold reads, no derived
index or range I/O; no Blob reads, physical storage, native latency or RSS claim.
"""
import argparse
from hashlib import sha256
import json
from math import ceil
import platform
from statistics import median
import tracemalloc

import git_study_inputs
from git_study_inputs import PublicGit, fingerprints
import map_layout_study
from map_layout_study import Layout, nibbles
import text_merge_study


class Reader:
    """Instrument trusted, locally generated maps; NOT a canonical verifier."""
    def __init__(self, objects):
        self.objects, self.cache = objects, {}
        self.reads = self.bytes = self.skipped = self.fallbacks = 0

    def node(self, ref):
        if ref in self.cache:
            return self.cache[ref]
        data = self.objects[ref]
        self.reads += 1
        self.bytes += len(data)
        offset = 0
        def take(n):
            nonlocal offset
            value = data[offset:offset + n]
            assert len(value) == n
            offset += n
            return value
        def number(n):
            return int.from_bytes(take(n), 'little')
        def suffix():
            count = number(2)
            return nibbles(take((count + 1) // 2))[:count]
        def entry():
            size = 35 if data[offset] == 1 else 34
            assert data[offset] in (1, 2, 3)
            return take(size)
        if ref[0] == 2:
            node = {suffix(): entry() for _ in range(number(4))}
        else:
            assert ref[0] == 3
            prefix = suffix()
            terminal = entry() if number(1) else None
            bitmap = number(2)
            node = (prefix, terminal, {slot: take(33) for slot in range(16) if bitmap & (1 << slot)})
        assert offset == len(data)
        self.cache[ref] = node
        return node

    def items(self, ref, prefix=()):
        node = self.node(ref)
        if ref[0] == 2:
            yield from ((prefix + key, entry) for key, entry in node.items())
        else:
            common, terminal, children = node
            prefix += common
            if terminal is not None:
                yield prefix, terminal
            for slot, child in children.items():
                yield from self.items(child, prefix + (slot,))

    def binding(self, ref, name):
        key = nibbles(name)
        while True:
            node = self.node(ref)
            if ref[0] == 2:
                return node.get(key)
            prefix, terminal, children = node
            if key[:len(prefix)] != prefix:
                return None
            key = key[len(prefix):]
            if not key:
                return terminal
            ref = children.get(key[0])
            if ref is None:
                return None
            key = key[1:]

    def lookup(self, root, path):
        for index, name in enumerate(path):
            entry = self.binding(root, name)
            if entry is None:
                return None
            if index == len(path) - 1:
                return entry
            if entry[0] != 3:
                return None
            root = entry[1:]
        raise ValueError('nonempty lookup path required')

    def changed(self, left, right, prefix=()):
        if left == right:
            self.skipped += 1
            return
        a, b = self.node(left), self.node(right)
        if left[0] == right[0] == 3 and a[0] == b[0]:
            full = prefix + a[0]
            if a[1] != b[1]:
                yield full, a[1], b[1]
            for slot in sorted(a[2].keys() | b[2].keys()):
                x, y = a[2].get(slot), b[2].get(slot)
                key = full + (slot,)
                if x is not None and y is not None:
                    yield from self.changed(x, y, key)
                elif x is None:
                    yield from ((k, None, v) for k, v in self.items(y, key))
                else:
                    yield from ((k, v, None) for k, v in self.items(x, key))
        else:
            if left[0] != right[0] or (left[0] == 3 and a[0] != b[0]):
                self.fallbacks += 1
            aa, bb = dict(self.items(left, prefix)), dict(self.items(right, prefix))
            for key in sorted(aa.keys() | bb.keys()):
                if aa.get(key) != bb.get(key):
                    yield key, aa.get(key), bb.get(key)

    @staticmethod
    def name(key):
        assert len(key) % 2 == 0
        return bytes((key[i] << 4) | key[i + 1] for i in range(0, len(key), 2))

    def diff(self, left, right, path=()):
        for key, a, b in self.changed(left, right):
            location = path + (self.name(key),)
            if a is not None and b is not None and a[0] == b[0] == 3:
                yield from self.diff(a[1:], b[1:], location)
            else:
                yield location, a, b

    def validate_relocation(self, root, destination):
        def walk(ref, path):
            checked = 1
            assert len(path) <= 256 and len(b'/'.join(path)) <= 4096
            if ref is not None:
                for key, value in self.items(ref):
                    checked += walk(value[1:] if value[0] == 3 else None,
                                    path + (self.name(key),))
            return checked
        return walk(root, destination)

    def metrics(self):
        return {'objects_read': self.reads, 'payload_bytes_read': self.bytes,
                'equal_map_pairs_skipped': self.skipped, 'shape_fallbacks': self.fallbacks}


def build(layout, tree):
    return layout.directory({name: b'\x03' + build(layout, value) if isinstance(value, dict) else value
                             for name, value in tree.items()})


def files(tree, prefix=()):
    for name, value in sorted(tree.items()):
        if isinstance(value, dict):
            yield from files(value, prefix + (name,))
        else:
            yield prefix + (name,), value


def replace(tree, path, value):
    result = dict(tree)
    if len(path) > 1:
        result[path[0]] = replace(result[path[0]], path[1:], value)
    elif value is None:
        del result[path[0]]
    else:
        result[path[0]] = value
    return result


def source_tree(rows):
    tree, excluded = {}, {}
    for path, (mode, kind, oid) in rows.items():
        if mode not in (b'100644', b'100755', b'120000') or kind != b'blob':
            key = mode.decode()
            excluded[key] = excluded.get(key, 0) + 1
            continue
        components = path.split(b'/')
        assert (len(path) <= 4096 and len(components) <= 256 and
                all(0 < len(n) <= 255 and n not in (b'.', b'..') for n in components))
        parent = tree
        for name in components[:-1]:
            parent = parent.setdefault(name, {})
        blob = b'\x01' + sha256(b'git-blob\0' + oid).digest()
        parent[components[-1]] = (b'\x02' + blob if mode == b'120000' else
                                   b'\x01' + blob + bytes([mode == b'100755']))
    return tree, excluded


def distribution(values):
    values = sorted(values)
    return {'min': values[0], 'median': median(values),
            'p95_nearest_rank': values[ceil(len(values) * .95) - 1], 'max': values[-1]}


def measure(tree, limit):
    paths = list(files(tree))
    tracemalloc.start()
    before = Layout(limit)
    root = build(before, tree)
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    queries = paths[::max(1, ceil(len(paths) / 64))]
    lookups = {}
    for kind in ('present', 'missing'):
        costs = []
        for path, entry in queries:
            query = path if kind == 'present' else (*path[:-1], path[-1] + b'.avc-absent')
            reader = Reader(before.objects)
            found = reader.lookup(root, query)
            assert found == (entry if kind == 'present' else None)
            costs.append(reader.metrics())
        lookups[kind] = {'queries': len(costs), **{
            metric: distribution([cost[metric] for cost in costs])
            for metric in ('objects_read', 'payload_bytes_read')}}
    edited_path, old = paths[len(paths) // 2]
    new = b'\x01\x01' + sha256(b'controlled-edit').digest() + b'\x00'
    after = Layout(limit)
    edited_root = build(after, replace(tree, edited_path, new))
    reader = Reader(before.objects | after.objects)
    assert list(reader.diff(root, edited_root)) == [(edited_path, old, new)]
    changed = after.objects.keys() - before.objects.keys()
    edit = {'path_hex': b'/'.join(edited_path).hex(), **reader.metrics(),
            'new_map_objects': len(changed),
            'new_map_payload_bytes': sum(len(after.objects[r]) for r in changed)}
    # Explicitly recorded move of the largest top-level directory, raw-name tie.
    directories = [(sum(1 for _ in files(value)), name) for name, value in tree.items()
                   if isinstance(value, dict)]
    _, name = sorted(directories, key=lambda pair: (-pair[0], pair[1]))[0]
    destination = b'relocated-' + name
    assert destination not in tree and len(destination) <= 255
    moved = dict(tree)
    moved[destination] = moved.pop(name)
    moved_layout = Layout(limit)
    moved_root = build(moved_layout, moved)
    reader = Reader(before.objects | moved_layout.objects)
    diff = list(reader.diff(root, moved_root))
    assert {p for p, _, _ in diff} == {(name,), (destination,)}
    assert len(diff) == 2
    subtree = Reader(before.objects).lookup(root, (name,))
    assert subtree[0] == 3
    proof = Reader(before.objects)
    visited = proof.validate_relocation(subtree[1:], (destination,))
    delta = moved_layout.objects.keys() - before.objects.keys()
    move = {'source_hex': name.hex(), 'destination_hex': destination.hex(),
            'subtree_ref_reused': Reader(moved_layout.objects).lookup(moved_root, (destination,)) == subtree,
            'new_map_objects': len(delta),
            'new_map_payload_bytes': sum(len(moved_layout.objects[r]) for r in delta),
            'structural_diff': reader.metrics(),
            'uncached_bound_validation': {'bindings_visited': visited, **proof.metrics()}}
    return {'map_objects': len(before.objects),
            'map_payload_bytes': sum(map(len, before.objects.values())),
            'max_node_bytes': max(map(len, before.objects.values())),
            'incremental_python_build_peak_bytes': peak,
            'lookups': lookups, 'one_file_edit': edit, 'recorded_move': move}


def study(repository):
    source = PublicGit(repository)
    public, excluded = source_tree(source.snapshot())
    def entry(name):
        return b'\x01\x01' + sha256(name).digest() + b'\x00'
    wide = {f'file-{i:05d}.txt'.encode(): entry(str(i).encode()) for i in range(10000)}
    distributed = {sha256(str(i).encode()).hexdigest()[:32].encode(): value
                   for i, value in enumerate(wide.values())}
    deep = {b'file': entry(b'deep')}
    for _ in range(32):
        deep = {b'directory': deep}
    workloads = {'git-v2.48.1': public, 'wide-shared-prefix-10000': {b'pkg': wide},
                 'wide-distributed-10000': {b'pkg': distributed}, 'deep-32': deep}
    return {'source': source.metadata(), 'excluded_snapshot_entries': excluded,
            'python_version': platform.python_version(), 'machine': platform.machine(),
            'study_sources': fingerprints(__file__, map_layout_study.__file__,
                                          git_study_inputs.__file__, text_merge_study.__file__),
            'scope': 'SHA-256 surrogate IDs, same Entry widths; no Blob payloads. Cold '
                     'whole-object reads with per-operation reuse, no range I/O or derived '
                     'indexes. Merkle diff falls back to full maps on shape mismatch. '
                     'Build peaks trace incremental Python allocations with input trees '
                     'already resident; not native RSS, persistence or throughput.',
            'workloads': {name: {'leaf_bindings': sum(1 for _ in files(tree)),
                                'layouts': {label: measure(tree, limit) for label, limit in
                                            (('flat', None), ('packed-4096', 4096), ('one-key-radix', 0))}}
                          for name, tree in workloads.items()}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', required=True)
    args = parser.parse_args()
    print(json.dumps(study(args.repository), indent=2))
