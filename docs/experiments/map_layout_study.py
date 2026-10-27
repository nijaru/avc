"""Deterministic canonical-map cost study; NOT an AVC codec or benchmark.

Compare the same Entry bytes with flat sorted maps, the packed radix candidate,
and a one-key-leaf radix. Count logical map payload bytes, not SQLite/compressed
storage, latency, resident memory or network performance. SHA-256 surrogate
references preserve reference width and sharing; they are NOT native ObjectIDs.
Run: python docs/experiments/map_layout_study.py
"""
from dataclasses import dataclass
from hashlib import sha256
import json
from struct import pack


def nibbles(key):
    return tuple(n for byte in key for n in (byte >> 4, byte & 15))


def packed(key):
    return bytes((key[i] << 4) | (key[i + 1] if i + 1 < len(key) else 0)
                 for i in range(0, len(key), 2))


class Layout:
    def __init__(self, leaf_limit):
        self.limit = leaf_limit  # None = flat; 0 = one-key leaves
        self.objects = {}

    def put(self, kind, payload):
        ref = bytes([kind]) + sha256(b'layout-study\0' + bytes([kind]) + payload).digest()
        self.objects[ref] = payload
        return ref

    def directory(self, entries):
        return self.radix([(nibbles(k), v) for k, v in sorted(entries.items())])

    def radix(self, entries):
        leaf = pack('<I', len(entries)) + b''.join(
            pack('<H', len(k)) + packed(k) + v for k, v in entries)
        if self.limit is None or len(entries) <= 1 or len(leaf) <= self.limit:
            return self.put(2, leaf)
        first, last = entries[0][0], entries[-1][0]
        common = 0
        while common < min(len(first), len(last)) and first[common] == last[common]:
            common += 1
        terminal = b''
        children = {}
        for key, value in entries:
            if len(key) == common:
                terminal = value
            else:
                children.setdefault(key[common], []).append((key[common + 1:], value))
        payload = (pack('<H', common) + packed(first[:common]) + bytes([bool(terminal)]) +
                   terminal + pack('<H', sum(1 << slot for slot in children)))
        payload += b''.join(self.radix(children[slot]) for slot in sorted(children))
        return self.put(3, payload)


@dataclass(frozen=True)
class Workload:
    name: str
    names: tuple[bytes, ...]
    depth: int = 0
    shared_content: bool = False


def measure(workload, limit):
    entries = {name: b'\x01\x01' + sha256(b'old' + (b'' if workload.shared_content else name)).digest() + b'\x00'
               for name in workload.names}
    new_entry = b'\x01\x01' + sha256(b'new').digest() + b'\x00'
    objects = []
    roots = []
    for change in (False, True):
        current = dict(entries)
        if change:
            current[workload.names[len(workload.names) // 2]] = new_entry
        layout = Layout(limit)
        root = layout.directory(current)
        for _ in range(workload.depth):
            root = layout.directory({b'd': b'\x03' + root})
        roots.append(root)
        objects.append(layout.objects)
    before, after = objects
    assert roots[0] != roots[1]
    return {
        'map_objects': len(before),
        'map_payload_bytes': sum(map(len, before.values())),
        'max_node_bytes': max(map(len, before.values())),
        'new_map_objects_on_edit': len(after.keys() - before.keys()),
        'new_map_payload_bytes_on_edit': sum(len(after[r]) for r in after.keys() - before.keys()),
    }


def study():
    workloads = [
        Workload('small-32', tuple(f'f{i:03d}'.encode() for i in range(32))),
        Workload('near-leaf-102', tuple(f'k{i:02d}'.encode() for i in range(102))),
        Workload('shared-prefix-10000', tuple(f'file-{i:05d}.txt'.encode() for i in range(10000))),
        Workload('shared-content-10000', tuple(f'file-{i:05d}.txt'.encode() for i in range(10000)),
                 shared_content=True),
        Workload('distributed-10000', tuple(sha256(str(i).encode()).hexdigest()[:32].encode()
                                            for i in range(10000))),
        Workload('deep-128', (b'file',), 128),
    ]
    return {w.name: {name: measure(w, limit) for name, limit in
                    (('flat', None), ('packed-4096', 4096), ('one-key-radix', 0))}
            for w in workloads}


if __name__ == '__main__':
    print(json.dumps(study(), indent=2, sort_keys=True))
