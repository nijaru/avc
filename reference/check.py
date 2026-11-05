#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["blake3==1.0.8"]
# ///
"""Independent AVC v1 codec. See README.md for the corpus and stdin contracts."""

from __future__ import annotations

import argparse
import json
import re
import struct
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path

from blake3 import blake3

DOMAIN = b"AVC-OBJECT\0v1\0"
KINDS = {1, 2, 3, 4, 5}
MAPS = {2, 3}
SPELLING = re.compile(r"avc1-k(0[1-5])-([0-9a-f]{64})\Z")
HEX = re.compile(r"(?:[0-9a-f]{2})*\Z")


class Invalid(ValueError):
    """Invalid canonical source or corpus; no authoritative result."""


class Limit(Invalid):
    """Configured resource budget exhausted, not a semantic conflict."""


@dataclass(frozen=True)
class Pending:
    reason: str


def require(ok, message):
    if not ok:
        raise Invalid(message)


def uint(value, width):
    require(isinstance(value, int) and not isinstance(value, bool), "integer required")
    require(0 <= value < 1 << (8 * width), "integer out of range")
    return value.to_bytes(width, "little")


def identify(kind, payload):
    require(kind in KINDS, "unknown object kind")
    return (
        bytes([kind])
        + blake3(DOMAIN + bytes([kind]) + uint(len(payload), 8) + payload).digest()
    )


def spelling(ref):
    require(len(ref) == 33 and ref[0] in KINDS, "invalid reference")
    return f"avc1-k{ref[0]:02x}-{ref[1:].hex()}"


def parse_id(text):
    require(isinstance(text, str), "ID must be a string")
    match = SPELLING.fullmatch(text)
    require(match is not None, "noncanonical typed ID spelling")
    return bytes.fromhex(match[1] + match[2])


def unhex(text):
    require(
        isinstance(text, str) and HEX.fullmatch(text) is not None, "noncanonical hex"
    )
    return bytes.fromhex(text)


def nibbles(key):
    return tuple(n for byte in key for n in (byte >> 4, byte & 15))


def packed(ns):
    require(len(ns) <= 510 and all(0 <= n < 16 for n in ns), "nibble bounds")
    return uint(len(ns), 2) + bytes(
        (ns[i] << 4) | (ns[i + 1] if i + 1 < len(ns) else 0)
        for i in range(0, len(ns), 2)
    )


def common(keys):
    first, last = keys[0], keys[-1]
    i = 0
    while i < min(len(first), len(last)) and first[i] == last[i]:
        i += 1
    return first[:i]


def basename(name):
    require(1 <= len(name) <= 255, "basename length")
    require(
        b"\0" not in name and b"/" not in name and name not in (b".", b".."),
        "invalid basename",
    )


def file(ref, executable=False):
    require(
        len(ref) == 33 and ref[0] == 1 and type(executable) is bool, "file descriptor"
    )
    return b"\x01" + ref + bytes([executable])


def symlink(ref):
    require(len(ref) == 33 and ref[0] == 1, "symlink descriptor")
    return b"\x02" + ref


def directory(ref):
    require(len(ref) == 33 and ref[0] in MAPS, "directory descriptor")
    return b"\x03" + ref


@dataclass(frozen=True)
class Limits:
    corpus_bytes: int = 64 * 1024 * 1024
    object_bytes: int = 16 * 1024 * 1024
    objects: int = 100_000
    visits: int = 1_000_000
    nesting: int = 1024


DEFAULT_LIMITS = Limits()


class Reader:
    def __init__(self, payload):
        self.payload = payload
        self.pos = 0

    def take(self, count):
        require(0 <= count <= len(self.payload) - self.pos, "truncated field")
        result = self.payload[self.pos : self.pos + count]
        self.pos += count
        return result

    def number(self, width, signed=False):
        return int.from_bytes(self.take(width), "little", signed=signed)

    def boolean(self):
        value = self.number(1)
        require(value in (0, 1), "invalid boolean")
        return value

    def ref(self, allowed):
        value = self.take(33)
        require(value[0] in allowed, "wrong reference kind")
        return value

    def ns(self):
        count = self.number(2)
        require(count <= 510, "nibble count exceeds cap")
        raw = self.take((count + 1) // 2)
        require(not count % 2 or raw[-1] & 15 == 0, "nonzero nibble padding")
        return nibbles(raw)[:count]

    def entry(self, absent=False, conflict=True):
        start = self.pos
        tag = self.number(1)
        if tag == 0:
            require(absent, "Absent in directory map")
        elif tag == 1:
            self.ref({1})
            self.boolean()
        elif tag == 2:
            self.ref({1})
        elif tag == 3:
            self.ref(MAPS)
        elif tag == 4:
            require(conflict, "nested conflict term")
            self.ref({4})
        else:
            raise Invalid("unknown entry tag")
        return self.payload[start : self.pos]

    def text(self, cap):
        size = self.number(4)
        require(size <= cap, "UTF-8 field exceeds cap")
        raw = self.take(size)
        try:
            raw.decode("utf-8", errors="strict")
        except UnicodeDecodeError as exc:
            raise Invalid("invalid UTF-8") from exc
        return raw

    def end(self):
        require(self.pos == len(self.payload), "trailing bytes")


# Decoded tuples retain descriptor bytes, not filesystem or Unicode values.
# encode_node is separate from parsing and must reproduce the exact payload.
def decode_node(kind, payload):
    require(kind in KINDS, "unknown kind")
    r = Reader(payload)
    if kind == 1:
        return payload
    if kind == 2:
        count = r.number(1)
        require(count <= 32, "leaf threshold")
        node = tuple((r.ns(), r.entry()) for _ in range(count))
    elif kind == 3:
        count, prefix, terminal = r.number(4), r.ns(), None
        require(count >= 33, "branch threshold")
        if r.boolean():
            terminal = r.entry()
        bitmap = r.number(2)
        children = tuple((n, r.ref(MAPS)) for n in range(16) if bitmap & (1 << n))
        require(len(children) + (terminal is not None) >= 2, "redundant branch")
        node = count, prefix, terminal, children
    elif kind == 4:
        region = r.number(1)
        require(region in (0, 1), "unknown conflict region")
        signs = []
        for sign in range(2):
            count = r.number(1)
            require((1 if sign == 0 else 0) <= count <= 64, "conflict distinct count")
            terms = []
            for _ in range(count):
                multiplicity = r.number(1)
                require(1 <= multiplicity <= 64, "conflict multiplicity")
                term = r.entry(absent=True, conflict=False)
                require(
                    not terms or terms[-1][0] < term,
                    "unsorted/duplicate conflict terms",
                )
                require(region == 0 or term[0] == 3, "non-directory root conflict term")
                terms.append((term, multiplicity))
            signs.append(tuple(terms))
        positive, negative = signs
        p, n = sum(m for _, m in positive), sum(m for _, m in negative)
        require(p + n <= 64 and p - n == 1, "signed conflict balance/bound")
        require(
            not ({t for t, _ in positive} & {t for t, _ in negative}),
            "uncancelled conflict",
        )
        require(not (p == 1 and n == 0), "unnecessary resolved conflict")
        node = region, positive, negative
    else:
        root = r.ref(MAPS | {4})
        count = r.number(1)
        require(count <= 16, "revision parent count")
        parents = tuple(r.ref({5}) for _ in range(count))
        require(len(set(parents)) == len(parents), "duplicate revision parent")
        change = r.take(16)
        author, message = r.text(4096), r.text(1024 * 1024)
        timestamp, timezone = r.number(8, True), r.number(2, True)
        require(-1439 <= timezone <= 1439, "timezone range")
        node = root, parents, change, author, message, timestamp, timezone
    r.end()
    require(encode_node(kind, node) == payload, "noncanonical round trip")
    return node


def encode_node(kind, node):
    if kind == 1:
        return node
    if kind == 2:
        return uint(len(node), 1) + b"".join(packed(ns) + entry for ns, entry in node)
    if kind == 3:
        count, prefix, terminal, children = node
        return (
            uint(count, 4)
            + packed(prefix)
            + bytes([terminal is not None])
            + (terminal or b"")
            + uint(sum(1 << n for n, _ in children), 2)
            + b"".join(ref for _, ref in children)
        )
    if kind == 4:
        region, positive, negative = node
        return bytes([region]) + b"".join(
            uint(len(terms), 1) + b"".join(bytes([m]) + term for term, m in terms)
            for terms in (positive, negative)
        )
    if kind == 5:
        root, parents, change, author, message, timestamp, timezone = node
        return (
            root
            + uint(len(parents), 1)
            + b"".join(parents)
            + change
            + uint(len(author), 4)
            + author
            + uint(len(message), 4)
            + message
            + struct.pack("<qh", timestamp, timezone)
        )
    raise Invalid("unknown kind")


def dependencies(kind, node):
    if kind == 1:
        return ()
    if kind == 2:
        entries = [entry for _, entry in node]
    elif kind == 3:
        _, _, terminal, children = node
        return tuple(ref for _, ref in children) + (
            (terminal[1:34],) if terminal else ()
        )
    elif kind == 4:
        _, positive, negative = node
        entries = [term for terms in (positive, negative) for term, _ in terms]
    else:
        return (node[0],) + node[1]
    return tuple(entry[1:34] for entry in entries if entry[0] != 0)


class Store:
    def __init__(self, objects=None, limits=DEFAULT_LIMITS):
        self.objects = {} if objects is None else dict(objects)
        self.limits = limits
        self.nodes = {}
        self.visits = 0
        self.work_bytes = 0
        self.active = set()
        self.summaries = {}
        self.revisions = set()
        self.reached = set()

    def tick(self, depth):
        self.visits += 1
        if self.visits > self.limits.visits or depth > self.limits.nesting:
            raise Limit("closure traversal budget")

    def charge_key(self, key, depth):
        # Reused radix DAGs can expand exponentially into contextual names.
        # Charge each recovered tuple before retaining the flat subtree.
        self.tick(depth)
        self.work_bytes += 128 + 8 * len(key)
        if self.work_bytes > self.limits.corpus_bytes:
            raise Limit("recovered-key workspace budget")

    def add(self, kind, payload):
        ref = identify(kind, payload)
        if ref in self.objects:
            require(self.objects[ref] == payload, "hash collision")
        self.objects[ref] = payload
        return ref

    def node(self, ref):
        require(len(ref) == 33 and ref[0] in KINDS, "invalid reference")
        require(ref in self.objects, "missing owning dependency: " + spelling(ref))
        self.reached.add(ref)
        if ref not in self.nodes:
            payload = self.objects[ref]
            if len(payload) > self.limits.object_bytes:
                raise Limit("object byte budget")
            require(identify(ref[0], payload) == ref, "wrong object hash")
            self.nodes[ref] = decode_node(ref[0], payload)
        return self.nodes[ref]

    def map(self, bindings):
        """Full independent builder; sort raw byte names, then partition nibbles."""
        pairs = list(bindings.items()) if isinstance(bindings, dict) else list(bindings)
        require(len(pairs) <= 0xFFFFFFFF, "map count overflow")
        pairs.sort(key=lambda p: p[0])
        for i, (name, entry) in enumerate(pairs):
            basename(name)
            require(i == 0 or pairs[i - 1][0] != name, "duplicate map key")
            r = Reader(entry)
            r.entry()
            r.end()
        return self._build([(nibbles(name), entry) for name, entry in pairs])

    def _build(self, pairs):
        if len(pairs) <= 32:
            return self.add(2, encode_node(2, tuple(pairs)))
        prefix = common([key for key, _ in pairs])
        terminal, groups = None, {}
        for key, entry in pairs:
            suffix = key[len(prefix) :]
            if not suffix:
                terminal = entry
            else:
                groups.setdefault(suffix[0], []).append((suffix[1:], entry))
        children = tuple((n, self._build(group)) for n, group in sorted(groups.items()))
        return self.add(3, encode_node(3, (len(pairs), prefix, terminal, children)))

    def radix(self, ref, consumed=(), depth=0):
        """Validate context *on every edge*, even when an ID was seen elsewhere."""
        self.tick(depth)
        require(ref[0] in MAPS, "non-map radix child")
        require(ref not in self.active, "owning cycle")
        require(len(consumed) <= 510, "consumed prefix exceeds basename cap")
        self.active.add(ref)
        try:
            node = self.node(ref)
            if ref[0] == 2:
                pairs = []
                for suffix, entry in node:
                    key = consumed + suffix
                    self.charge_key(key, depth)
                    pairs.append((key, entry))
                require(
                    all(len(key) <= 510 and len(key) % 2 == 0 for key, _ in pairs),
                    "recovered nibble length",
                )
                require(
                    all(pairs[i - 1][0] < pairs[i][0] for i in range(1, len(pairs))),
                    "unsorted/duplicate leaf keys",
                )
            else:
                count, prefix, terminal, children = node
                full = consumed + prefix
                require(len(full) <= 510, "branch prefix length")
                require(terminal is None or len(full) % 2 == 0, "odd terminal boundary")
                if terminal is not None:
                    self.charge_key(full, depth)
                pairs = [(full, terminal)] if terminal is not None else []
                for nibble, child in children:
                    group = self.radix(child, full + (nibble,), depth + 1)
                    require(group, "empty branch child")
                    pairs.extend(group)
                require(len(pairs) == count, "incorrect authenticated count")
                require(
                    common([key for key, _ in pairs]) == full,
                    "nonmaximal branch prefix",
                )
            for key, _ in pairs:
                require(
                    len(key) % 2 == 0 and len(key) <= 510, "incomplete/overlong key"
                )
                basename(
                    bytes((key[i] << 4) | key[i + 1] for i in range(0, len(key), 2))
                )
            return pairs
        finally:
            self.active.remove(ref)

    def entry_summary(self, entry, depth):
        self.tick(depth)
        tag = entry[0]
        if tag == 0:
            return 0, 0
        ref = entry[1:34]
        if tag in (1, 2):
            self.node(ref)
            return 0, 0
        if tag == 3:
            return self.directory_summary(ref, depth + 1)
        require(tag == 4, "unknown entry")
        return self.conflict_summary(ref, 0, depth + 1)

    def conflict_summary(self, ref, region, depth):
        self.tick(depth)
        require(ref not in self.active, "owning cycle")
        self.active.add(ref)
        try:
            actual, positive, negative = self.node(ref)
            require(actual == region, "conflict region/context mismatch")
            summaries = [
                self.entry_summary(term, depth + 1)
                for terms in (positive, negative)
                for term, _ in terms
            ]
            return max(d for d, _ in summaries), max(b for _, b in summaries)
        finally:
            self.active.remove(ref)

    def directory_summary(self, ref, depth=0):
        self.tick(depth)
        require(ref not in self.active, "directory owning cycle")
        if ref in self.summaries:
            self.reached.add(ref)
            return self.summaries[ref]
        pairs = self.radix(ref, depth=depth + 1)
        self.active.add(ref)
        try:
            max_depth, max_bytes = 0, 0
            for key, entry in pairs:
                child_depth, child_bytes = self.entry_summary(entry, depth + 1)
                max_depth = max(max_depth, 1 + child_depth)
                max_bytes = max(
                    max_bytes, len(key) // 2 + (1 + child_bytes if child_depth else 0)
                )
            require(max_depth <= 256 and max_bytes <= 4096, "logical source path bound")
            self.summaries[ref] = max_depth, max_bytes
            return max_depth, max_bytes
        finally:
            self.active.remove(ref)

    def source(self, ref, depth=0):
        require(
            ref[0] in MAPS | {4}, "source root must be a directory or root conflict"
        )
        if ref[0] in MAPS:
            return self.directory_summary(ref, depth)
        result = self.conflict_summary(ref, 1, depth)
        require(result[0] <= 256 and result[1] <= 4096, "root conflict path bounds")
        return result

    def revision(self, ref, depth=0):
        self.tick(depth)
        require(ref not in self.active, "revision ancestry cycle")
        node = self.node(ref)
        if ref in self.revisions:
            return
        self.active.add(ref)
        try:
            self.source(node[0], depth + 1)
            for parent in node[1]:
                self.revision(parent, depth + 1)
            self.revisions.add(ref)
        finally:
            self.active.remove(ref)

    def verify(self, roots):
        self.visits = 0
        self.work_bytes = 0
        self.active.clear()
        self.summaries.clear()
        self.revisions.clear()
        self.nodes.clear()
        if len(self.objects) > self.limits.objects:
            raise Limit("object count budget")
        # Check all hashes/local grammar and dependency existence, including any
        # unrooted payloads. Map name semantics still require a root context.
        for ref in self.objects:
            node = self.node(ref)
            for dep in dependencies(ref[0], node):
                require(
                    dep in self.objects, "missing owning dependency: " + spelling(dep)
                )
        self.reached.clear()
        for ref in roots:
            self.tick(0)
            if ref[0] == 1:
                self.node(ref)
            elif ref[0] == 5:
                self.revision(ref)
            else:
                self.source(ref)
        require(
            set(self.objects) <= self.reached,
            "unrooted objects lack full contextual validation",
        )

    def signed(self, positive, negative=(), region=0):
        """Normalize only exact cancellation; return Entry (possibly Conflict)."""
        require(region in (0, 1), "conflict region")
        terms = Counter()

        def flatten(entry, sign, active):
            r = Reader(entry)
            r.entry(absent=True)
            r.end()
            if entry[0] != 4:
                require(region == 0 or entry[0] == 3, "root conflict type")
                terms[entry] += sign
                return
            ref = entry[1:34]
            require(ref not in active, "conflict flatten cycle")
            actual, pos, neg = self.node(ref)
            require(actual == region, "conflict flatten region")
            for term, mult in pos:
                flatten(term, sign * mult, active | {ref})
            for term, mult in neg:
                flatten(term, -sign * mult, active | {ref})

        for entries, sign in ((positive, 1), (negative, -1)):
            for entry in entries:
                flatten(entry, sign, set())
        terms = {term: mult for term, mult in terms.items() if mult}
        require(sum(terms.values()) == 1, "signed balance")
        require(sum(abs(m) for m in terms.values()) <= 64, "signed bound")
        if len(terms) == 1 and next(iter(terms.values())) == 1:
            return next(iter(terms))
        pos = tuple((t, m) for t, m in sorted(terms.items()) if m > 0)
        neg = tuple((t, -m) for t, m in sorted(terms.items()) if m < 0)
        return b"\x04" + self.add(4, encode_node(4, (region, pos, neg)))

    def merge_opaque(self, base, left, right, region=0):
        """Tiny semantic oracle for validated values, not a directory/text driver."""
        if left == right:
            return left
        if left == base:
            return right
        if right == base:
            return left
        # A signed expression does not retain which alternative an author
        # resolved. Subtracting that base can fabricate deletion or undo edits.
        if base[0] == 4:
            return Pending("UnresolvedBase")
        return self.signed([left, right], [base], region=region)

    def authored(
        self,
        root,
        parents=(),
        change=bytes(range(16)),
        author=b"A <a@example.test>",
        message=b"reference\n",
        timestamp=0,
        timezone=0,
    ):
        require(len(change) == 16, "ChangeId must have exactly 16 bytes")
        require(
            type(timestamp) is int and -(1 << 63) <= timestamp < (1 << 63),
            "timestamp range",
        )
        require(type(timezone) is int and -1439 <= timezone <= 1439, "timezone range")
        node = root, tuple(parents), change, author, message, timestamp, timezone
        payload = encode_node(5, node)
        require(decode_node(5, payload) == node, "revision encoder field boundaries")
        return self.add(5, payload)

    def corpus(self, roots):
        # Only include the owning closure: builder intermediate states are not
        # implicitly authoritative roots and may be discarded by the caller.
        pending, selected = list(roots), {}
        while pending:
            ref = pending.pop()
            if ref not in selected:
                payload = self.objects[ref]
                selected[ref] = payload
                pending.extend(dependencies(ref[0], decode_node(ref[0], payload)))
        return {
            "objects": [
                {"id": spelling(ref), "payload": payload.hex()}
                for ref, payload in sorted(selected.items())
            ],
            "roots": [spelling(ref) for ref in roots],
        }


def corpus_store(data, limits=DEFAULT_LIMITS):
    require(isinstance(data, dict), "corpus must be an object")
    require(
        isinstance(data.get("objects"), list) and isinstance(data.get("roots"), list),
        "corpus arrays",
    )
    if len(data["objects"]) > limits.objects:
        raise Limit("object count budget")
    objects = {}
    total = 0
    for item in data["objects"]:
        require(
            isinstance(item, dict) and set(item) == {"id", "payload"},
            "object record fields",
        )
        ref = parse_id(item["id"])
        require(ref not in objects, "duplicate object ID")
        text = item["payload"]
        require(isinstance(text, str), "payload must be hex string")
        if len(text) > limits.object_bytes * 2:
            raise Limit("object byte budget")
        payload = unhex(text)
        total += len(payload)
        if total > limits.corpus_bytes:
            raise Limit("corpus byte budget")
        objects[ref] = payload
    roots = [parse_id(ref) for ref in data["roots"]]
    return Store(objects, limits), roots


def verify_corpus(data, limits=DEFAULT_LIMITS):
    store, roots = corpus_store(data, limits)
    store.verify(roots)
    return store, roots


def load_json(path, limits):
    # Bound input *before* parsing unknown JSON lengths, not only after allocation.
    with open(path, "rb") as stream:
        raw = stream.read(limits.corpus_bytes + 1)
    if len(raw) > limits.corpus_bytes:
        raise Limit("JSON input byte budget")
    return json.loads(raw, object_pairs_hook=unique_json_fields)


def unique_json_fields(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON field")
        result[key] = value
    return result


def map_stdin(limits):
    """Flat oracle: [{name: lowercase hex, entry: descriptor hex}], plus objects."""
    data = load_json("/dev/stdin", limits)
    store, _roots = corpus_store(
        {"objects": data.get("objects", []), "roots": []}, limits
    )
    bindings = [(unhex(row["name"]), unhex(row["entry"])) for row in data["bindings"]]
    root = store.map(bindings)
    output = store.corpus([root])
    verify_corpus(output, limits)
    print(json.dumps(output, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--vectors",
        action="store_true",
        help="verify pinned vectors and unittest checks",
    )
    mode.add_argument(
        "--verify",
        metavar="FILE",
        help="verify an exported corpus (including negatives, if present)",
    )
    mode.add_argument(
        "--map", action="store_true", help="build a directory from stdin JSON"
    )
    parser.add_argument("--max-bytes", type=int, default=Limits.corpus_bytes)
    parser.add_argument("--max-object-bytes", type=int, default=Limits.object_bytes)
    parser.add_argument("--max-objects", type=int, default=Limits.objects)
    parser.add_argument("--max-visits", type=int, default=Limits.visits)
    parser.add_argument("--max-nesting", type=int, default=Limits.nesting)
    args = parser.parse_args()
    values = (
        args.max_bytes,
        args.max_object_bytes,
        args.max_objects,
        args.max_visits,
        args.max_nesting,
    )
    require(
        all(v > 0 for v in values) and args.max_nesting <= 2000,
        "invalid resource limits",
    )
    limits = Limits(*values)
    sys.setrecursionlimit(
        20_000
    )  # Explicit traversal budget controls recursion, not interpreter accident.
    if args.map:
        map_stdin(limits)
        return
    data = load_json(
        Path(__file__).with_name("vectors.json") if args.vectors else args.verify,
        limits,
    )
    verify_corpus(data, limits)
    for case in data.get("negative", []):
        try:
            verify_corpus(case, limits)
        except Limit:
            raise  # Exhaustion is not evidence that a negative is malformed.
        except Invalid:
            pass
        else:
            raise Invalid("negative accepted: " + case["name"])
    print(
        f"verified {len(data['objects'])} objects, {len(data['roots'])} roots, {len(data.get('negative', []))} negatives"
    )
    if args.vectors:
        import unittest

        sys.path.insert(0, str(Path(__file__).parent))
        # Avoid importing a second copy of this module when executed as a script.
        sys.modules["check"] = sys.modules[__name__]
        suite = unittest.defaultTestLoader.discover(
            str(Path(__file__).parent), pattern="test_*.py"
        )
        if not unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful():
            raise Invalid("self checks failed")


if __name__ == "__main__":
    try:
        main()
    except (
        Invalid,
        OSError,
        json.JSONDecodeError,
        KeyError,
        TypeError,
        OverflowError,
        RecursionError,
    ) as exc:
        print(f"refused: {exc}", file=sys.stderr)
        sys.exit(1)
