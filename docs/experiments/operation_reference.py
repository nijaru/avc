"""Bounded, in-memory replay/admission reference over actual canonical objects.

Read authorization and input protection are separate from untrusted audit IDs.
No target writes, filesystem observations, GC leases or durable receipts occur.
Recorded-move *evidence* is replayed here; merge remapping remains unqualified
and is explicitly refused, not silently executed as a source-only merge.
"""
from __future__ import annotations
from dataclasses import dataclass
from types import MappingProxyType
from typing import TypeAlias
from codec_reference import Builder, Ref, K_STATE, K_REV
from codec_records import Merge, Move, Observation, Path, Transform
from codec_verify import Cursor, Invalid, Verifier, entry_ref
from protected_inputs import ProtectedInputs, Refused
from text_merge_reference import merge_text

Tree: TypeAlias = bytes | dict[bytes, 'Tree']  # directories expanded; other Entries opaque


class UnsupportedRecipe(Refused):
    pass


class MergeTooComplex(Refused):
    pass


def lookup(tree: Tree | None, path: Path) -> Tree | None:
    for part in path:
        if tree is None:
            return None
        if not isinstance(tree, dict):
            raise Refused('path traverses an opaque/non-directory Entry')
        tree = tree.get(part)
    return tree


def replace(tree: Tree, path: Path, value: Tree | None) -> Tree:
    if not path or not isinstance(tree, dict):
        raise Refused('nonempty path and directory parent required')
    result = dict(tree)
    name, *rest = path
    if rest:
        if name not in result:
            raise Refused('missing parent directory')
        result[name] = replace(result[name], tuple(rest), value)
    elif value is None:
        if name not in result:
            raise Refused('cannot remove absent binding')
        del result[name]
    else:
        result[name] = value
    return result


class Evaluation:
    def __init__(self, inputs: ProtectedInputs):
        self.inputs = inputs
        self.builder = Builder()
        self.builder.store.update(inputs.objects)
        self.maps = {}

    def value(self, entry: bytes | None) -> Tree | None:
        if entry is None:
            return None
        ref = entry_ref(entry)
        self.inputs.payload(ref)  # check protection even for a leaf/opaque Conflict
        if entry[0] != 3:
            return entry
        if ref not in self.maps:
            flat = Verifier(self.inputs.objects).map(ref, (), 0, 0)
            self.maps[ref] = {name: self.value(child) for name, child in flat.items()}
        return self.maps[ref]

    def state(self, ref: Ref) -> Tree:
        if ref.kind != K_STATE:
            raise Refused('expected State')
        return self.value(self.inputs.payload(ref))

    def descriptor(self, value: Tree | None) -> bytes | None:
        if isinstance(value, dict):
            children = {name: self.descriptor(child) for name, child in value.items()}
            return self.builder.directory(self.builder.map(children))
        return value

    def complete(self, value: Tree) -> Ref:
        state = self.builder.state(self.descriptor(value))
        try:
            Verifier(self.builder.store).state(state)
        except Invalid as error:
            if 'budget' in str(error) or 'nesting >64' in str(error):
                raise MergeTooComplex(str(error)) from error
            raise Refused('computed source violates bounds: ' + str(error)) from error
        return state

    def transform(self, record: Transform) -> Tree:
        base = self.state(record.base)
        # Every structural endpoint is checked in the BASE, before any removal.
        endpoints = []
        installs = []
        for operation in record.structural:
            destination = operation.destination
            if lookup(base, destination) != self.value(operation.expected_destination):
                raise Refused('stale destination witness')
            endpoints.append(destination)
            if isinstance(operation, Move):
                source = lookup(base, operation.source)
                if source is None or source != self.value(operation.expected_source):
                    raise Refused('stale source witness')
                endpoints.append(operation.source)
                installs.append((destination, source))
            else:
                installs.append((destination, self.value(operation.value)))
        # Reject aliasing, overlapping destinations/sources, chains and swaps.
        ordered = sorted(endpoints)
        if any(b[:len(a)] == a for a, b in zip(ordered, ordered[1:])):
            raise Refused('overlapping structural paths')
        result = base
        for operation in record.structural:
            if isinstance(operation, Move):
                result = replace(result, operation.source, None)
        for destination, value in installs:
            result = replace(result, destination, value)
        # Prove relocated descendants BEFORE leaf edits could hide an overflow.
        self.complete(result)
        for edit in record.edits:
            if lookup(result, edit.path) != self.value(edit.expected):
                raise Refused('stale post-structural leaf witness')
            result = replace(result, edit.path, self.value(edit.value))
        return result

    def merge(self, base: Tree | None, left: Tree | None, right: Tree | None,
              text: tuple[int, int], *, root=False) -> Tree | None:
        if left == right:
            return left
        if left == base:
            return right
        if right == base:
            return left
        if isinstance(left, dict) and isinstance(right, dict) and (base is None or isinstance(base, dict)):
            old = {} if base is None else base
            result = {}
            for name in sorted(old.keys() | left.keys() | right.keys()):
                value = self.merge(old.get(name), left.get(name), right.get(name), text)
                if value is not None:
                    result[name] = value
            return result
        values = (base, left, right)
        if (text == (1, 1) and all(isinstance(v, bytes) and v[0] == 1 for v in values)
                and len({v[-1] for v in values}) == 1):
            data = [self.builder.store[entry_ref(v)] for v in values]
            merged = merge_text(*data)  # MemoryError/cancellation never become conflicts
            if merged is not None:
                return self.builder.file(self.builder.blob(merged), bool(left[-1]))
        alternatives = [self.descriptor(left), self.descriptor(right)]
        conflict = self.builder.conflict(int(root), self.descriptor(base), alternatives)
        return self.builder.conflict_entry(conflict)


@dataclass(eq=False)
class Node:
    state: Tree
    parents: tuple[Node, ...]
    revision: Ref | None  # virtual nodes have ancestry, never an authored ID


class History:
    """Augmented finite DAG; explicit fixture work limits refuse without output."""
    def __init__(self, evaluation: Evaluation, text, permit_unrelated: bool):
        self.evaluation, self.text, self.permit = evaluation, text, permit_unrelated
        self.nodes = {}
        self.pairs = {}
        self.frontiers = []
        self.work = 0

    def tick(self):
        self.work += 1
        if self.work > 10000:
            raise MergeTooComplex('history reference work budget')

    def read(self, ref: Ref) -> Node:
        if ref in self.nodes:
            return self.nodes[ref]
        if len(self.nodes) >= 128:
            raise MergeTooComplex('history reference node budget')
        self.tick()
        c = Cursor(self.evaluation.inputs.payload(ref))
        state = c.ref({K_STATE})
        parents = tuple(c.ref({K_REV}) for _ in range(c.number(4)))
        # Closure/cycle validation was completed when these roots were protected.
        node = Node(self.evaluation.state(state), (), ref)
        self.nodes[ref] = node
        node.parents = tuple(self.read(p) for p in parents)
        return node

    def ancestors(self, node: Node) -> set[Node]:
        seen, pending = set(), [node]
        while pending:
            current = pending.pop()
            self.tick()
            if current not in seen:
                seen.add(current)
                pending.extend(current.parents)
        return seen

    def pair(self, left: Node, right: Node) -> Node:
        if (left, right) in self.pairs:
            return self.pairs[left, right]
        common = self.ancestors(left) & self.ancestors(right)
        # A common node is maximal iff it is not a strict ancestor of another.
        older = set()
        for node in common:
            older.update(self.ancestors(node) - {node})
        # The public tips are original nodes; every reduction pairs an
        # accumulator with an original base. Common ancestors are therefore
        # original revisions even when the accumulator itself is virtual.
        frontier = sorted(common - older, key=lambda n: n.revision.encode())
        self.frontiers.append(tuple(n.revision for n in frontier))
        if not frontier:
            if not self.permit:
                raise Refused('unrelated histories need explicit permission')
            base = {}
        else:
            reduced = frontier[0]
            for other in frontier[1:]:
                reduced = self.pair(reduced, other)
            base = reduced.state
        result = self.evaluation.merge(base, left.state, right.state, self.text, root=True)
        self.evaluation.complete(result)  # invalid virtual States cannot be bases
        virtual = Node(result, (left, right), None)
        self.pairs[left, right] = virtual
        return virtual


@dataclass(frozen=True)
class Replay:
    state: Ref
    objects: MappingProxyType  # only the computed State's owning closure
    frontiers: tuple[tuple[Ref, ...], ...] = ()


def replay(provenance: Ref, inputs: ProtectedInputs, selected: tuple[Ref, ...], *,
           permit_unrelated=False) -> Replay:
    """Check a requested recipe against independently selected exact inputs.

    Successful replay proves source computation, not permission to publish,
    observation truth, current selection freshness or durable admission.
    """
    claim = inputs.provenance(provenance)
    record = claim.records[0]
    evaluation = Evaluation(inputs)
    frontiers = ()
    if isinstance(record, Transform):
        if selected != (record.base,):
            raise Refused('Transform base differs from selected input')
        result = evaluation.transform(record)
    elif isinstance(record, Merge):
        if record.source_algorithm != (1, 1) or record.text_algorithm not in ((0, 0), (1, 1)):
            raise UnsupportedRecipe('unknown algorithm/version')
        if any(record.moves):
            raise UnsupportedRecipe('move remapping not qualified by this reference')
        if record.source is not None:
            if selected != record.source:
                raise Refused('Source Merge inputs differ from selected inputs')
            result = evaluation.merge(*(evaluation.state(s) for s in selected), record.text_algorithm, root=True)
        else:
            left, right, permission = record.history
            tips = (claim.inputs[left], claim.inputs[right])
            if selected != tips or permission != permit_unrelated:
                raise Refused('History Merge inputs/permission differ from request')
            history = History(evaluation, record.text_algorithm, permission)
            result = history.pair(*(history.read(tip) for tip in tips)).state
            frontiers = tuple(history.frontiers)
    else:
        raise UnsupportedRecipe('Observation has no filesystem replay')
    state = evaluation.complete(result)
    if state != claim.result:
        raise Refused('recipe does not reproduce bound result')
    verifier = Verifier(evaluation.builder.store)
    verifier.state(state)
    closure = MappingProxyType({ref: evaluation.builder.store[ref] for ref in verifier.closure})
    return Replay(state, closure, frontiers)


def check_move_evidence(provenance: Ref, inputs: ProtectedInputs, base: Ref, side: Ref) -> Replay:
    claim = inputs.provenance(provenance)
    record = claim.records[0]
    if (not isinstance(record, Transform) or claim.result != side or record.base != base
            or not any(isinstance(op, Move) for op in record.structural)):
        raise Refused('move evidence must bind the exact base and side')
    return replay(provenance, inputs, (base,))


@dataclass(frozen=True)
class ObservationFacts:
    """Trusted observing-boundary output, NOT reconstructed from an audit claim.

    Tests supply this separately. A native scanner/promotion authority must
    establish it; a caller echoing an untrusted record has proved nothing.
    """
    result: Ref
    observation: Observation


def qualify_authoring(provenance: Ref, inputs: ProtectedInputs, facts: ObservationFacts,
                      selected: tuple[Ref, ...], *, authoring_base: Ref,
                      permit_unrelated=False) -> Ref:
    """Check observation and optional construction obligations, not full authoring.

    Deliberate subset consent and Selection CAS belong to the separate authoring
    authority; this check neither updates a target nor issues a custody receipt.
    """
    claim = inputs.provenance(provenance)
    observations = [r for r in claim.records if isinstance(r, Observation)]
    if observations != [facts.observation] or claim.result != facts.result:
        raise Refused('required observation differs from observing authority')
    observation = facts.observation
    evaluation = Evaluation(inputs)
    if observation.revision is not None:
        revision_state = Cursor(inputs.payload(observation.revision)).ref({K_STATE})
        if revision_state != observation.base:
            raise Refused('Observation base Revision names another State')
    base, result = evaluation.state(observation.base), evaluation.state(claim.result)
    if observation.base != authoring_base:
        raise Refused('Observation base differs from selected authoring base')
    resolutions = {path: (conflict, chosen) for path, conflict, chosen in observation.resolutions}
    for path, (conflict, chosen) in resolutions.items():
        before = lookup(base, path)
        if before != Builder.conflict_entry(conflict):
            raise Refused('resolution does not name the base Conflict')
        if lookup(result, path) != evaluation.value(chosen):
            raise Refused('resolution choice differs from admitted source')
    def require_choices(before, path=()):
        if isinstance(before, dict):
            for name, child in before.items():
                require_choices(child, path + (name,))
        elif before[0] == 4 and before != lookup(result, path) and path not in resolutions:
            raise Refused('changed conflict needs an explicit whole-unit choice')
    require_choices(base)
    if not isinstance(claim.records[0], Observation):
        replay(provenance, inputs, selected, permit_unrelated=permit_unrelated)
    return claim.result
