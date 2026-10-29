"""Bounded selective-authoring reference over the real candidate AVC codec.

Models regular-file observations, immutable previews, conditional authoring and
selected-root export. Inputs explicitly identify successfully observed bindings
(including proven absence); this is NOT a filesystem scanner, safe materializer,
durable authority or hunk-selection UI. No directories-as-values, symlinks,
conflicts, ignored/admin-path policy, quotas or retention deadlines are modeled.
Transactions and root pins are in-memory; other fixtures qualify their ordering.
ChangeIds are unique deterministic fixture counters, not native random allocation.
"""
from __future__ import annotations

from dataclasses import dataclass
from difflib import SequenceMatcher

from codec_reference import Builder, Ref, K_BLOB, K_LEAF, K_BRANCH, K_STATE
from codec_verify import Cursor, Verifier

Path = tuple[bytes, ...]


class Refused(ValueError):
    pass


@dataclass(frozen=True)
class File:
    content: bytes
    executable: bool = False


@dataclass(frozen=True)
class Base:
    state: Ref
    revision: Ref | None = None


@dataclass(frozen=True)
class Capture:
    receipt: bytes
    base: Base
    generation: int
    state: Ref
    covered: tuple[Path, ...]
    failed: tuple[Path, ...]
    skipped: tuple[Path, ...]


@dataclass(frozen=True)
class Choice:
    path: Path
    # Indices into the pinned base-to-observed line edits, not live-file offsets.
    hunks: tuple[int, ...] | None = None


@dataclass(frozen=True)
class Candidate:
    key: int
    base: Base
    generation: int
    state: Ref
    provenance: Ref
    change: bytes
    author: bytes
    description: bytes
    unix_ns: int


def write_state(builder: Builder, files: dict[Path, File]) -> Ref:
    tree = {}
    for path, value in sorted(files.items()):
        if not path:
            raise Refused('regular file cannot be the root')
        node = tree
        for part in path[:-1]:
            node = node.setdefault(part, {})
            if not isinstance(node, dict):
                raise Refused('file/directory collision')
        if path[-1] in node:
            raise Refused('file/directory collision')
        node[path[-1]] = value

    def directory(node):
        return builder.directory(builder.map({
            name: directory(value) if isinstance(value, dict)
            else builder.file(builder.blob(value.content), value.executable)
            for name, value in node.items()
        }))

    state = builder.state(directory(tree))
    Verifier(builder.store).state(state)
    return state


def read_state(objects: dict[Ref, bytes], state: Ref) -> dict[Path, File]:
    verifier = Verifier(objects)
    verifier.state(state)
    files = {}

    def walk(entry, path):
        cursor = Cursor(entry)
        tag = cursor.number(1)
        if tag == 3:
            entries = verifier.map(cursor.ref({K_LEAF, K_BRANCH}), (), 0, 0)
            if not entries and path:
                raise Refused('empty directory outside regular-file fixture')
            for name, child in entries.items():
                walk(child, (*path, name))
        elif tag == 1:
            content = objects[cursor.ref({K_BLOB})]
            files[path] = File(content, cursor.flag())
        else:
            raise Refused('non-regular source outside fixture')
        cursor.done()

    walk(objects[state], ())
    return files


def within(path: Path, prefix: Path) -> bool:
    return path[:len(prefix)] == prefix


def selected_hunks(base: File | None, observed: File, indices: tuple[int, ...]) -> File:
    if base is not None and base.executable != observed.executable:
        raise Refused('mode change needs whole-entry selection')
    old, new = base.content if base else b'', observed.content
    try:
        old.decode('utf-8'); new.decode('utf-8')
    except UnicodeDecodeError as exc:
        raise Refused('binary file needs whole-entry selection') from exc
    if b'\0' in old + new:
        raise Refused('binary file needs whole-entry selection')
    before, after = old.splitlines(keepends=True), new.splitlines(keepends=True)
    edits = [op for op in SequenceMatcher(None, before, after, autojunk=False).get_opcodes()
             if op[0] != 'equal']
    if not indices or indices != tuple(sorted(set(indices))) or any(i < 0 or i >= len(edits) for i in indices):
        raise Refused('invalid pinned hunk selection')
    result, end = [], 0
    for index in indices:
        _, start, stop, new_start, new_stop = edits[index]
        result.extend(before[end:start])
        result.extend(after[new_start:new_stop])
        end = stop
    result.extend(before[end:])
    return File(b''.join(result), observed.executable)


class Authority:
    """Abstract atomic authority; live files are separate from object custody."""

    def __init__(self):
        self.builder = Builder()
        self.base = Base(write_state(self.builder, {}))
        self.generation = 0
        self.initial_selection = True
        self.live: dict[Path, File] = {}
        self.captures: dict[bytes, Capture] = {}
        self.prepared: dict[int, Candidate] = {}
        self.receipts: dict[int, Ref] = {}
        self.authored: set[Ref] = set()
        self.next_capture = self.next_candidate = 0

    def select(self, base: Base):
        files = read_state(self.builder.store, base.state)
        if base.revision is not None:
            verifier = Verifier(self.builder.store)
            verifier.generic(base.revision)
            if Cursor(self.builder.store[base.revision]).ref({K_STATE}) != base.state:
                raise Refused('revision/base mismatch')
        self.base = base
        self.initial_selection = False
        self.generation += 1
        return files

    def checkpoint(self, covered, *, failed=(), skipped=(), partial=False) -> Capture:
        covered, failed, skipped = (tuple(sorted(set(paths))) for paths in (covered, failed, skipped))
        if (set(covered) & set(failed) or set(covered) & set(skipped) or set(failed) & set(skipped)):
            raise Refused('overlapping observation labels')
        if (failed or skipped) and not partial:
            raise Refused('partial checkpoint requires consent')
        files = read_state(self.builder.store, self.base.state)
        for path in covered:
            if path in self.live:
                files[path] = self.live[path]
            else:
                files.pop(path, None)  # Only explicitly observed absence removes.
        self.next_capture += 1
        capture = Capture(f'private-{self.next_capture:08d}'.encode(), self.base,
                          self.generation, write_state(self.builder, files), covered, failed, skipped)
        self.captures[capture.receipt] = capture
        return capture

    def recover(self, receipt: bytes):
        capture = self.captures[receipt]
        original = read_state(self.builder.store, capture.base.state)
        files = read_state(self.builder.store, capture.state)
        origins = {path: 'tracked' if path in original else 'new' for path in capture.covered}
        # Return a fresh view, never adopt the captured State as its tracking base.
        return capture.base, files, origins

    def prepare(self, receipt: bytes, choices: tuple[Choice, ...] | None = None, *,
                partial=False, zero_parent=False, author=b'fixture', description=b'selected', unix_ns=0):
        capture = self.captures[receipt]
        # Historical observation generations are audit data, not today's CAS
        # token. Re-review can reuse old bytes at their exact recorded base.
        if self.base != capture.base:
            raise Refused('checkpoint base differs; select its recorded base first')
        before = read_state(self.builder.store, capture.base.state)
        observed = read_state(self.builder.store, capture.state)
        if choices is None:
            # Bare commit: tracked modifications/proven deletions only. Whole
            # tracked subtree removal is deferred to explicit selection.
            removals = {p for p in capture.covered if p in before and p not in observed}
            removed_dirs = {p[:i] for p in removals for i in range(1, len(p))
                            if all(q in removals for q in before if within(q, p[:i]))}
            choices = tuple(Choice(p) for p in capture.covered
                            if p in before and before.get(p) != observed.get(p)
                            and not any(within(p, directory) for directory in removed_dirs))
        choices = tuple(sorted(choices, key=lambda c: c.path))
        scope = tuple(c.path for c in choices)
        if not scope:
            raise Refused('nothing selected')
        if any(within(b, a) for a, b in zip(scope, scope[1:])):
            raise Refused('overlapping selection scopes')
        # Clip an ancestor traversal gap to the selected scope, never disclose
        # unrelated private siblings. Gaps never become deletion observations.
        def project(paths):
            return tuple(sorted({p if within(p, s) else s for p in paths for s in scope
                                 if within(p, s) or within(s, p)}))
        failed, skipped = project(capture.failed), project(capture.skipped)
        if (failed or skipped) and not partial:
            raise Refused('selected scope has gaps; partial consent required')
        covered = tuple(p for p in capture.covered if any(within(p, s) for s in scope))
        if set(covered) & (set(failed) | set(skipped)) or set(failed) & set(skipped):
            raise Refused('selected scope has overlapping coverage labels')
        result = dict(before)
        constructed = False
        for choice in choices:
            matches = [p for p in covered if within(p, choice.path)]
            if choice.hunks is not None:
                if matches != [choice.path] or choice.path not in observed:
                    raise Refused('hunks require an observed regular file')
                result[choice.path] = selected_hunks(before.get(choice.path), observed[choice.path], choice.hunks)
                constructed = True
            else:
                for path in matches:
                    if path in observed:
                        result[path] = observed[path]
                    else:
                        result.pop(path, None)
        changes = [p for p in sorted(before.keys() | result.keys()) if before.get(p) != result.get(p)]
        if not changes:
            raise Refused('nothing selected')
        # Equal empty source does not make a recovery selection initial. The
        # first init selection supplies root intent; other StateOnly selections
        # require it explicitly. Choosing a matching parent is outside fixture.
        if capture.base.revision is None:
            if not zero_parent and (not self.initial_selection or before):
                raise Refused('StateOnly authoring needs explicit root intent')
        elif zero_parent:
            raise Refused('zero-parent intent cannot silently drop the recorded parent')
        state = write_state(self.builder, result)
        observation = self.builder.observation_record(
            capture.base.state, capture.base.revision, capture.generation,
            scope=scope, covered=covered, failed=failed, skipped=skipped,
            partial=bool(failed or skipped))
        if constructed:
            def entry(value):
                return None if value is None else self.builder.file(
                    self.builder.blob(value.content), value.executable)
            transform = self.builder.transform_record(capture.base.state, edits=[
                (p, entry(before.get(p)), entry(result.get(p))) for p in changes])
            provenance = self.builder.provenance(state, [], [], transform, observation)
        else:
            provenance = self.builder.provenance(state, [], [], observation)
        Verifier(self.builder.store).generic(provenance)
        self.next_candidate += 1
        candidate = Candidate(self.next_candidate, capture.base, self.generation, state,
                              provenance, self.next_candidate.to_bytes(16, 'little'),
                              author, description, unix_ns)
        self.prepared[candidate.key] = candidate
        return candidate

    def confirm(self, key: int) -> Ref:
        if key in self.receipts:
            return self.receipts[key]
        if key not in self.prepared:
            raise Refused('unknown or cancelled candidate')
        candidate = self.prepared[key]
        if self.base != candidate.base or self.generation != candidate.generation:
            raise Refused('selection changed; review again')
        # All failures precede the abstract authored-root/Selection transaction.
        parents = [] if candidate.base.revision is None else [candidate.base.revision]
        revision = self.builder.revision(candidate.state, parents, candidate.change,
                                         candidate.author, candidate.description,
                                         candidate.unix_ns, candidate.provenance)
        Verifier(self.builder.store).generic(revision)
        self.authored.add(revision)
        self.base = Base(candidate.state, revision)
        self.initial_selection = False
        self.generation += 1
        self.receipts[key] = revision
        del self.prepared[key]
        return revision

    def cancel(self, key: int):
        if key not in self.prepared:
            raise Refused('unknown or completed candidate')
        del self.prepared[key]

    def export(self, root: Ref) -> dict[Ref, bytes]:
        verifier = Verifier(self.builder.store)
        verifier.generic(root)
        return {ref: self.builder.store[ref] for ref in verifier.closure}

    def collect(self):
        roots = {self.base.state, *self.authored}
        if self.base.revision is not None:
            roots.add(self.base.revision)
        for item in (*self.captures.values(), *self.prepared.values()):
            roots.update((item.base.state, item.state))
            if item.base.revision is not None:
                roots.add(item.base.revision)
            if isinstance(item, Candidate):
                roots.add(item.provenance)
        resident = set()
        for root in roots:
            resident.update(self.export(root))
        self.builder.store = {ref: data for ref, data in self.builder.store.items() if ref in resident}
