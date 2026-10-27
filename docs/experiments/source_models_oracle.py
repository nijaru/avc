#!/usr/bin/env python3
"""Executable source-state reference and two history representations for AVC.

Independent *semantic* oracle: intentionally does not depend on AVC's Rust code,
Git, SQLite, disk layout, Merkle page design, or network infrastructure.

Run: python docs/experiments/source_models_oracle.py

SnapshotHistory and DeltaHistory use the same operations. This verifies that a
representation can reproduce source values; it is NOT a performance benchmark,
a production codec, a complete causal patch graph, or a proof of crash safety.
"""

from __future__ import annotations

import hashlib
import random
import unittest
from dataclasses import dataclass
from typing import TypeAlias

Path: TypeAlias = tuple[bytes, ...]


@dataclass(frozen=True)
class File:
    data: bytes
    executable: bool = False


@dataclass(frozen=True)
class Symlink:
    target: bytes


@dataclass(frozen=True)
class Directory:
    entries: tuple[tuple[bytes, Entry], ...] = ()


@dataclass(frozen=True)
class Conflict:
    base: Entry | None
    alternatives: tuple[Entry | None, Entry | None]


Entry: TypeAlias = File | Symlink | Directory | Conflict
EMPTY = Directory()


def directory(entries: dict[bytes, Entry]) -> Directory:
    for name in entries:
        valid_component(name)
    return Directory(tuple(sorted(entries.items())))


def valid_component(component: bytes) -> None:
    if not isinstance(component, bytes) or component in (b"", b".", b"..") or b"/" in component or b"\0" in component:
        raise ValueError("invalid path component")


def valid_path(path: Path) -> None:
    if not isinstance(path, tuple) or not path:
        raise ValueError("nonempty path required")
    for part in path:
        valid_component(part)


def lookup(root: Directory, path: Path) -> Entry | None:
    node: Entry = root
    for part in path:
        valid_component(part)
        if not isinstance(node, Directory):
            raise ValueError("path traverses a non-directory")
        node = dict(node.entries).get(part)
        if node is None:
            return None
    return node


def update(root: Directory, path: Path, value: Entry | None, expected: Entry | None) -> Directory:
    """Conditional exact-source edit; no ambient filesystem state is consulted."""
    valid_path(path)
    if lookup(root, path) != expected:
        raise ValueError("stale source entry")

    def walk(node: Directory, remaining: Path) -> Directory:
        name, *tail = remaining
        children = dict(node.entries)
        if tail:
            child = children.get(name)
            if not isinstance(child, Directory):
                raise ValueError("missing parent directory")
            children[name] = walk(child, tuple(tail))
        elif value is None:
            if name not in children:
                raise ValueError("cannot remove absent path")
            del children[name]
        else:
            children[name] = value
        return directory(children)

    return walk(root, path)


def move(root: Directory, old: Path, new: Path) -> Directory:
    """Exact subtree move; no expansion into descendant file edits."""
    valid_path(old)
    valid_path(new)
    if new[: len(old)] == old or old == new:
        raise ValueError("cyclic move")
    source = lookup(root, old)
    if source is None:
        raise ValueError("missing move source")
    if lookup(root, new) is not None:
        raise ValueError("destination occupied")
    result = update(root, old, None, source)
    return update(result, new, source, None)


def graft(root: Directory, source_root: Directory, source: Path, dest: Path) -> Directory:
    """Copy an immutable subtree from a pinned source state, not a mutable path."""
    value = lookup(source_root, source)
    if value is None:
        raise ValueError("source missing")
    return update(root, dest, value, None)


def framed(data: bytes) -> bytes:
    return len(data).to_bytes(8, "big") + data


def digest(entry: Entry | None) -> bytes:
    """Illustrative deterministic source commitment, NOT a stable AVC format."""
    if entry is None:
        value = b"absent"
    elif isinstance(entry, File):
        value = b"file" + (b"1" if entry.executable else b"0") + framed(entry.data)
    elif isinstance(entry, Symlink):
        value = b"symlink" + framed(entry.target)
    elif isinstance(entry, Directory):
        if tuple(sorted(entry.entries)) != entry.entries or len(dict(entry.entries)) != len(entry.entries):
            raise ValueError("noncanonical directory entries")
        value = b"directory" + b"".join(framed(n) + digest(e) for n, e in entry.entries)
    elif isinstance(entry, Conflict):
        terms = sorted({digest(a) for a in entry.alternatives})
        if len(entry.alternatives) != 2 or len(terms) != 2:
            raise ValueError("conflict requires two distinct alternatives")
        value = b"conflict" + digest(entry.base) + b"".join(terms)
    else:
        raise TypeError(type(entry))
    return hashlib.sha256(framed(value)).digest()


def normalize_conflict(base: Entry | None, left: Entry | None, right: Entry | None) -> Entry | None:
    """Canonicalize one binary node, never flatten or cancel nested terms."""
    alternatives = {digest(s): s for s in (left, right)}
    if len(alternatives) == 1:
        return next(iter(alternatives.values()))
    return Conflict(base, tuple(alternatives[k] for k in sorted(alternatives)))


def merge(base: Entry | None, left: Entry | None, right: Entry | None) -> Entry | None:
    """Conservative 3-way value merge; no promise of semantic code correctness."""
    if left == right:
        return left
    if left == base:
        return right
    if right == base:
        return left
    if isinstance(left, Directory) and isinstance(right, Directory) and (base is None or isinstance(base, Directory)):
        old = dict(base.entries) if isinstance(base, Directory) else {}
        a = dict(left.entries)
        b = dict(right.entries)
        children = {}
        for k in sorted(old.keys() | a.keys() | b.keys()):
            v = merge(old.get(k), a.get(k), b.get(k))
            if v is not None:
                children[k] = v
        return directory(children)
    return normalize_conflict(base, left, right)


@dataclass(frozen=True)
class Put:
    path: Path
    value: Entry | None
    expected: Entry | None


@dataclass(frozen=True)
class Move:
    old: Path
    new: Path


@dataclass(frozen=True)
class Graft:
    source_root: Directory
    source: Path
    dest: Path


Edit: TypeAlias = Put | Move | Graft


def apply(root: Directory, edits: tuple[Edit, ...]) -> Directory:
    """Apply exact, sequenced test edits; a production batch needs atomic validation."""
    for edit in edits:
        if isinstance(edit, Put):
            root = update(root, edit.path, edit.value, edit.expected)
        elif isinstance(edit, Move):
            root = move(root, edit.old, edit.new)
        elif isinstance(edit, Graft):
            root = graft(root, edit.source_root, edit.source, edit.dest)
        else:
            raise TypeError(type(edit))
    return root


class SnapshotHistory:
    """Each immutable revision has an immediately stored source root."""

    def __init__(self) -> None:
        self.revisions = [EMPTY]

    def record(self, edits: tuple[Edit, ...]) -> int:
        self.revisions.append(apply(self.revisions[-1], edits))
        return len(self.revisions) - 1

    def read(self, index: int) -> Directory:
        return self.revisions[index]


class DeltaHistory:
    """Canonical parent-relative edits; source views are *derived* and evictable."""

    def __init__(self) -> None:
        self.changes: list[tuple[Edit, ...]] = []
        self.view_cache: dict[int, Directory] = {0: EMPTY}
        self.replayed_revisions = 0

    def record(self, edits: tuple[Edit, ...]) -> int:
        """Admit a VALID source revision, not merely persist an unchecked draft.

        Without reconstructing/verifying its parent, a minimal delta log can
        acknowledge edits whose preconditions or namespace effects are invalid.
        A source VCS must not call such a draft a complete revision. This toy
        uses the derived parent view to validate the exact resulting source;
        consequently it does tree work at admission even with delta truth.
        Production designs may use other verified witnesses/transformations.
        """
        next_state = apply(self.read(len(self.changes)), edits)
        self.changes.append(edits)
        self.view_cache[len(self.changes)] = next_state
        return len(self.changes)

    def read(self, index: int) -> Directory:
        if index in self.view_cache:
            return self.view_cache[index]
        ancestor = max(k for k in self.view_cache if k <= index)
        state = self.view_cache[ancestor]
        for i in range(ancestor, index):
            state = apply(state, self.changes[i])
            self.view_cache[i + 1] = state
            self.replayed_revisions += 1
        return state

    def evict_derived_views(self) -> None:
        self.view_cache = {0: EMPTY}
        self.replayed_revisions = 0


class IndependentFlatOracle:
    """Separate map-of-paths semantics to cross-check tree transformations."""

    def __init__(self) -> None:
        self.nodes: dict[Path, Entry] = {(): EMPTY}

    def from_tree(self, root: Directory) -> None:
        self.nodes = {(): EMPTY}

        def walk(path: Path, d: Directory) -> None:
            for name, value in d.entries:
                here = path + (name,)
                self.nodes[here] = value if not isinstance(value, Directory) else EMPTY
                if isinstance(value, Directory):
                    walk(here, value)
        walk((), root)

    def put(self, path: Path, value: Entry | None, expected: Entry | None) -> None:
        valid_path(path)
        if self.nodes.get(path) != expected:
            raise ValueError("flat stale source")
        if path[:-1] not in self.nodes or not isinstance(self.nodes[path[:-1]], Directory):
            raise ValueError("flat path parent missing")
        if value is None:
            if path not in self.nodes:
                raise ValueError("flat cannot remove absent")
            for key in list(self.nodes):
                if key[:len(path)] == path:
                    del self.nodes[key]
        else:
            self.nodes[path] = value if not isinstance(value, Directory) else EMPTY
            if isinstance(value, Directory):
                # A subtree can be installed in one edit, without changing all
                # of its children through the public API.
                def descend(prefix: Path, d: Directory) -> None:
                    for name, child in d.entries:
                        key = prefix + (name,)
                        self.nodes[key] = child if not isinstance(child, Directory) else EMPTY
                        if isinstance(child, Directory):
                            descend(key, child)
                descend(path, value)

    def move(self, old: Path, new: Path) -> None:
        if new[:len(old)] == old or new in self.nodes or old not in self.nodes:
            raise ValueError("invalid flat move")
        before = {p: e for p, e in self.nodes.items() if p[:len(old)] == old}
        if new[:-1] not in self.nodes or not isinstance(self.nodes[new[:-1]], Directory):
            raise ValueError("invalid destination parent")
        for p in before:
            del self.nodes[p]
        for p, e in before.items():
            self.nodes[new + p[len(old):]] = e

    def flatten(self, root: Directory) -> dict[Path, Entry]:
        self.from_tree(root)
        return dict(self.nodes)


class SourceValueTests(unittest.TestCase):
    def test_source_equality_independent_of_history(self) -> None:
        source = update(EMPTY, (b"a",), File(b"\0\xff", True), None)
        reverted = update(source, (b"a",), None, File(b"\0\xff", True))
        self.assertEqual(digest(reverted), digest(EMPTY))
        self.assertNotEqual(digest(source), digest(EMPTY))
        self.assertNotEqual(digest(update(EMPTY, (b"a",), Symlink(b"target"), None)), digest(source))

    def test_directory_move_reuses_subtree_and_keeps_empty_dirs(self) -> None:
        src = update(EMPTY, (b"src",), EMPTY, None)
        src = update(src, (b"src", b"empty"), EMPTY, None)
        src = update(src, (b"src", b"binary"), File(b"\0\xfe"), None)
        moved = move(src, (b"src",), (b"renamed",))
        self.assertEqual(lookup(moved, (b"renamed",)), lookup(src, (b"src",)))
        self.assertIsNone(lookup(moved, (b"src",)))
        self.assertEqual(lookup(moved, (b"renamed", b"empty")), EMPTY)
        flat = IndependentFlatOracle()
        flat.from_tree(src)
        flat.move((b"src",), (b"renamed",))
        expected_flat = dict(flat.nodes)
        self.assertEqual(expected_flat, flat.flatten(moved))

    def test_graft_pins_source_value_not_path_identity(self) -> None:
        src = update(EMPTY, (b"source",), File(b"old"), None)
        grafted = graft(EMPTY, src, (b"source",), (b"copy",))
        newer_source = update(src, (b"source",), File(b"new"), File(b"old"))
        self.assertEqual(lookup(grafted, (b"copy",)), File(b"old"))
        self.assertEqual(lookup(newer_source, (b"source",)), File(b"new"))

    def test_preconditions_are_exact_and_paths_validated(self) -> None:
        root = update(EMPTY, (b"f",), File(b"before"), None)
        with self.assertRaises(ValueError):
            update(root, (b"f",), File(b"after"), None)
        for invalid in ((b"..",), (b"a/b",), (b"\0",), (b"",)):
            with self.assertRaises(ValueError):
                update(root, invalid, File(b"x"), None)
        with self.assertRaises(ValueError):
            move(update(root, (b"dir",), EMPTY, None), (b"dir",), (b"dir", b"inner"))

    def test_conflict_value_independent_of_origin(self) -> None:
        base, a, b = File(b"one"), File(b"left"), File(b"right")
        from_ab = normalize_conflict(base, a, b)
        from_ba = normalize_conflict(base, b, a)
        self.assertEqual(digest(from_ab), digest(from_ba))
        self.assertNotEqual(digest(from_ab), digest(normalize_conflict(File(b"other"), a, b)))
        # These distinct side labels are metadata, not source bytes.
        origin_a = ("author A", "revision A")
        origin_b = ("author B", "revision B")
        self.assertNotEqual(origin_a, origin_b)
        self.assertEqual(digest(from_ab), digest(from_ba))

    def test_nested_conflicts_preserve_shape_and_do_not_cancel(self) -> None:
        base, a, b, c = (File(x) for x in (b"base",b"a",b"b",b"c"))
        ab = normalize_conflict(base,a,b)
        left_grouped = normalize_conflict(base,ab,c)
        right_grouped = normalize_conflict(base,a,normalize_conflict(base,b,c))
        self.assertNotEqual(digest(left_grouped),digest(right_grouped))
        result = merge(base,left_grouped,c)
        self.assertEqual(result,normalize_conflict(base,left_grouped,c))
        self.assertIn(left_grouped,result.alternatives)
        self.assertIn(c,result.alternatives)
        # Only whole-value equality/base rules may resolve an existing node.
        self.assertEqual(merge(left_grouped,left_grouped,c),c)

    def test_merge_preserves_delete_modify_conflict(self) -> None:
        base = update(EMPTY, (b"f",), File(b"before"), None)
        left = update(base, (b"f",), None, File(b"before"))
        right = update(base, (b"f",), File(b"changed"), File(b"before"))
        result = merge(base, left, right)
        self.assertIsInstance(result, Directory)
        self.assertIsInstance(lookup(result, (b"f",)), Conflict)
        self.assertEqual(digest(result), digest(merge(base, right, left)))

    def test_history_models_have_identical_source_values(self) -> None:
        snap, delta = SnapshotHistory(), DeltaHistory()
        edits = [
            (Put((b"folder",), EMPTY, None),),
            (Put((b"folder", b"f"), File(b"first"), None),),
            (Put((b"folder", b"f"), File(b"second"), File(b"first")),),
            (Move((b"folder",), (b"renamed",)),),
            (Put((b"renamed", b"f"), File(b"third"), File(b"second")),),
        ]
        for index, e in enumerate(edits, 1):
            self.assertEqual(snap.record(e), index)
            self.assertEqual(delta.record(e), index)
            self.assertEqual(digest(snap.read(index)), digest(delta.read(index)))
        delta.evict_derived_views()
        self.assertEqual(digest(snap.read(len(edits))), digest(delta.read(len(edits))))
        self.assertEqual(delta.replayed_revisions, len(edits))
        self.assertEqual(delta.read(len(edits)), snap.read(len(edits)))
        self.assertEqual(delta.replayed_revisions, len(edits))  # warm read

    def test_independent_flat_model_random_edit_traces(self) -> None:
        rng = random.Random(1337)
        root = EMPTY
        flat = IndependentFlatOracle()
        paths = [(prefix, name) for prefix in (b"a", b"b", b"c")
                 for name in (b"0", b"1", b"2", b"3", b"4")]
        for top in (b"a", b"b", b"c"):
            root = update(root, (top,), EMPTY, None)
            flat.put((top,), EMPTY, None)
        for step in range(250):
            path = rng.choice(paths)
            expected = lookup(root, path)
            value = None if expected is not None and rng.randrange(4) == 0 else File(
                step.to_bytes(4, "big") + bytes([rng.randrange(256)]))
            root = update(root, path, value, expected)
            flat.put(path, value, expected)
            rendered = IndependentFlatOracle()
            self.assertEqual(rendered.flatten(root), flat.nodes, f"diverged at step {step}")
        # Explicit subtree move is a single logical change in either model.
        root = move(root, (b"a",), (b"moved",))
        flat.move((b"a",), (b"moved",))
        rendered = IndependentFlatOracle()
        self.assertEqual(rendered.flatten(root), flat.nodes)

    def test_delta_revision_must_validate_before_acknowledgement(self) -> None:
        snap, delta = SnapshotHistory(), DeltaHistory()
        create = (Put((b"f",), File(b"original"), None),)
        self.assertEqual(snap.record(create), delta.record(create))
        # An unchecked append could store this bogus operation and report a
        # revision that fails source reconstruction only after cache eviction.
        stale = (Put((b"f",), File(b"replacement"), None),)
        with self.assertRaises(ValueError):
            delta.record(stale)
        with self.assertRaises(ValueError):
            snap.record(stale)
        self.assertEqual(len(delta.changes), 1)
        self.assertEqual(len(snap.revisions), 2)
        delta.evict_derived_views()
        valid = (Put((b"f",), File(b"replacement"), File(b"original")),)
        self.assertEqual(snap.record(valid), delta.record(valid))
        self.assertEqual(delta.replayed_revisions, 1)
        self.assertEqual(digest(delta.read(2)), digest(snap.read(2)))

    def test_unmaterialized_draft_is_not_absent_source(self) -> None:
        delta = DeltaHistory()
        delta.record((Put((b"f",), File(b"data"), None),))
        delta.evict_derived_views()
        self.assertNotIn(1, delta.view_cache)
        self.assertIsNone(lookup(delta.view_cache[0], (b"f",)))
        self.assertEqual(lookup(delta.read(1), (b"f",)), File(b"data"))


class WorkspaceResourceOracle:
    """A tiny scoped-authority model. It does not emulate filesystem writes."""

    def __init__(self) -> None:
        self.selected = (0, b"revision-0")
        self.capture_head = (0, None)
        self.pinned_captures: dict[bytes, bytes] = {}
        self.outcomes: dict[bytes, tuple[str, bytes | None]] = {}

    def select(self, generation: int, new_revision: bytes, request: bytes) -> tuple[str, bytes | None]:
        if request in self.outcomes:
            return self.outcomes[request]
        if generation != self.selected[0]:
            answer: tuple[str, bytes | None] = ("stale-selection", None)
        else:
            self.selected = (generation + 1, new_revision)
            answer = ("selected", new_revision)
        self.outcomes[request] = answer
        return answer

    def capture(self, generation: int, capture_id: bytes, data: bytes, request: bytes) -> tuple[str, bytes | None]:
        if request in self.outcomes:
            return self.outcomes[request]
        if capture_id in self.pinned_captures and self.pinned_captures[capture_id] != data:
            raise ValueError("same capture identity used for other data")
        # Admission and durable pinning precede acknowledgement and pointer CAS.
        self.pinned_captures[capture_id] = data
        if generation == self.capture_head[0]:
            self.capture_head = (generation + 1, capture_id)
            answer = ("current-capture", capture_id)
        else:
            answer = ("retained-historical-capture", capture_id)
        self.outcomes[request] = answer
        return answer


class WorkspaceResourceTests(unittest.TestCase):
    def test_capture_and_selection_are_independent_authorities(self) -> None:
        w = WorkspaceResourceOracle()
        first = w.capture(0, b"capture-1", b"some bytes", b"req-1")
        selected = w.select(0, b"rev-new", b"req-2")
        second = w.capture(1, b"capture-2", b"new bytes", b"req-3")
        self.assertEqual((first[0], selected[0], second[0]),
                         ("current-capture", "selected", "current-capture"))
        self.assertEqual(w.selected, (1, b"rev-new"))
        self.assertEqual(w.capture_head, (2, b"capture-2"))
        self.assertEqual(w.pinned_captures[b"capture-1"], b"some bytes")

    def test_losing_capture_head_cas_does_not_erase_acknowledged_capture(self) -> None:
        w = WorkspaceResourceOracle()
        w.capture(0, b"a", b"A", b"r1")
        conflict = w.capture(0, b"b", b"B", b"r2")
        self.assertEqual(conflict, ("retained-historical-capture", b"b"))
        self.assertEqual(w.pinned_captures, {b"a": b"A", b"b": b"B"})
        self.assertEqual(w.capture_head, (1, b"a"))
        self.assertEqual(w.capture(0, b"b", b"B", b"r2"), conflict)
        self.assertEqual(len(w.pinned_captures), 2)

    def test_stale_selection_cannot_overwrite_another_revision(self) -> None:
        w = WorkspaceResourceOracle()
        w.select(0, b"new-head", b"r1")
        rejected = w.select(0, b"old-request", b"r2")
        self.assertEqual(rejected[0], "stale-selection")
        self.assertEqual(w.selected, (1, b"new-head"))
        self.assertEqual(w.select(0, b"old-request", b"r2"), rejected)

    def test_source_state_is_not_proof_of_atomic_filesystem_observation(self) -> None:
        # Valid source data assembled from per-file observations is still not a
        # point-in-time snapshot of an uncontrolled external writer's directory.
        state = update(EMPTY, (b"f1",), File(b"observed at t1"), None)
        state = update(state, (b"f2",), File(b"observed at t2"), None)
        self.assertEqual(lookup(state, (b"f1",)), File(b"observed at t1"))
        self.assertEqual(lookup(state, (b"f2",)), File(b"observed at t2"))
        # The digest intentionally records values, not filesystem consistency.
        self.assertEqual(len(digest(state)), 32)


if __name__ == "__main__":
    unittest.main(verbosity=2)
