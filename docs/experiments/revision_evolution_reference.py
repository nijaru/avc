"""Canonical-object revision evolution, not native history/authoring authority.

All reads use separately authorized, protected closures. Computation is private;
only a completed output Revision closure is returned. No roots, filesystem bytes,
receipts or caller stores change. Caller-supplied fresh ChangeIds are fixture
identities, not a qualified random allocator. Moves are not inferred.

Amend adopts an explicitly selected State with a checked identity Transform;
it does not claim that State was observed in a filesystem or edited from the
predecessor. Rebase/revert/reapply compute actual source merges. Their audit input
table starts with (subject Revision, selected old/mainline parent), followed by
onto when distinct. Thus even equal-State merge parents remain distinguishable.
Evolution metadata below is separate from canonical ancestry and owns no bytes.
"""
from __future__ import annotations

from dataclasses import dataclass
from types import MappingProxyType

from codec_reference import Builder, Ref, K_STATE, K_REV, K_PROV
from codec_verify import Cursor, Verifier
from operation_reference import Evaluation, MergeTooComplex, UnsupportedRecipe
from protected_inputs import ProtectedInputs, Refused


@dataclass(frozen=True)
class Metadata:
    author: bytes
    description: bytes
    unix_ns: int


@dataclass(frozen=True)
class Revision:
    state: Ref
    parents: tuple[Ref, ...]
    change: bytes
    metadata: Metadata
    provenance: Ref | None


def read_revision(inputs: ProtectedInputs, ref: Ref) -> Revision:
    if ref.kind != K_REV:
        raise Refused('expected Revision')
    # Acquiring the protected root already verified its entire owning closure.
    c = Cursor(inputs.payload(ref))
    state = c.ref({K_STATE})
    parents = tuple(c.ref({K_REV}) for _ in range(c.number(4)))
    change = c.take(16)
    metadata = Metadata(c.str(), c.str(), int.from_bytes(c.take(8), 'little', signed=True))
    provenance = c.ref({K_PROV}) if c.flag() else None
    c.done()
    return Revision(state, parents, change, metadata, provenance)


@dataclass(frozen=True)
class Evolution:
    predecessors: tuple[Ref, ...]
    successors: tuple[Ref, ...]
    kind: str


@dataclass(frozen=True)
class Authored:
    revision: Ref
    objects: MappingProxyType
    evolution: Evolution | None = None


def finish(builder: Builder, revision: Ref, predecessor=None, kind=None) -> Authored:
    verifier = Verifier(builder.store)
    verifier.generic(revision)
    objects = MappingProxyType({ref: builder.store[ref] for ref in verifier.closure})
    evolution = (Evolution((predecessor,), (revision,), kind)
                 if predecessor is not None and predecessor != revision else None)
    return Authored(revision, objects, evolution)


def construct(builder, state, parents, change, metadata, provenance):
    return builder.revision(state, list(parents), change, metadata.author,
                            metadata.description, metadata.unix_ns, provenance)


def amend(inputs: ProtectedInputs, subject: Ref, state: Ref, *, metadata=None) -> Authored:
    old = read_revision(inputs, subject)
    metadata = old.metadata if metadata is None else metadata
    evaluation = Evaluation(inputs)
    selected = evaluation.complete(evaluation.state(state))
    if selected == old.state and metadata == old.metadata:
        return finish(evaluation.builder, subject)  # no-op needs no lineage edge
    builder = evaluation.builder
    provenance = builder.provenance(selected, [subject], [], builder.transform_record(selected))
    revision = construct(builder, selected, old.parents, old.change, metadata, provenance)
    return finish(builder, revision, subject, 'amend')


def _merge_revision(inputs, subject, onto, *, kind, metadata, new_change=None,
                   mainline=None, text=(1, 1)):
    old, destination = (read_revision(inputs, ref) for ref in (subject, onto))
    if text not in ((0, 0), (1, 1)):
        raise UnsupportedRecipe('unknown text algorithm/version')
    if not old.parents:
        raise UnsupportedRecipe('root has no implicit old parent or inverse')
    if kind != 'revert' and len(old.parents) != 1:
        raise UnsupportedRecipe('multi-parent rebase/reapply needs a defined recipe')
    if mainline is None:
        if len(old.parents) != 1:
            raise Refused('merge revert requires an explicit mainline Revision')
        mainline = old.parents[0]
    if mainline not in old.parents:
        raise Refused('mainline is not a parent of the subject')
    parent = read_revision(inputs, mainline)
    metadata = old.metadata if metadata is None else metadata
    if kind == 'rebase':
        change = old.change
        if onto == mainline and metadata == old.metadata:
            return finish(Evaluation(inputs).builder, subject)
    else:
        # Fresh allocation is a caller obligation, not a global seen-change set.
        if new_change is None or len(new_change) != 16 or new_change in (old.change, parent.change, destination.change):
            raise Refused('new authored change requires a fresh 16-byte ChangeId')
        change = new_change
    source = ((old.state, destination.state, parent.state) if kind == 'revert'
              else (parent.state, destination.state, old.state))
    evaluation = Evaluation(inputs)
    state = evaluation.complete(evaluation.merge_sources(source, (None, None), text))
    builder = evaluation.builder
    audit = list(dict.fromkeys((subject, mainline, onto)))
    provenance = builder.provenance(state, audit, [], builder.merge_record(source=source, text_algorithm=text))
    revision = construct(builder, state, (onto,), change, metadata, provenance)
    return finish(builder, revision, subject if kind == 'rebase' else None, kind)


def rebase(inputs, subject, onto, *, metadata=None, text=(1, 1)) -> Authored:
    return _merge_revision(inputs, subject, onto, kind='rebase', metadata=metadata, text=text)


def revert(inputs, subject, onto, *, new_change, metadata, mainline=None, text=(1, 1)) -> Authored:
    return _merge_revision(inputs, subject, onto, kind='revert', metadata=metadata,
                          new_change=new_change, mainline=mainline, text=text)


def reapply(inputs, subject, onto, *, new_change, metadata, text=(1, 1)) -> Authored:
    return _merge_revision(inputs, subject, onto, kind='reapply', metadata=metadata,
                          new_change=new_change, text=text)


@dataclass(frozen=True)
class Stack:
    replacements: MappingProxyType
    objects: MappingProxyType
    evolution: tuple[Evolution, ...]


def rebase_stack(inputs: ProtectedInputs, selected: tuple[Ref, ...], onto: Ref, *, text=(1, 1)) -> Stack:
    """Only an explicit, contiguous, parent-first single-parent stack is supported."""
    if not selected or len(set(selected)) != len(selected) or len(selected) > 128:
        raise Refused('select 1..128 distinct stack Revisions')
    read_revision(inputs, onto)
    originals = [read_revision(inputs, ref) for ref in selected]
    for index, old in enumerate(originals):
        if len(old.parents) != 1 or (index and old.parents != (selected[index - 1],)):
            raise UnsupportedRecipe('stack must be contiguous, single-parent and parent-first')
    store = dict(inputs.objects)
    protected = inputs
    replacements, events = {}, []
    result = None
    for subject in selected:
        result = rebase(protected, subject, onto, text=text)
        replacements[subject] = result.revision
        if result.evolution is not None:
            events.append(result.evolution)
        store.update(result.objects)
        onto = result.revision
        # Acquire only the computed output, never unrelated authorized inputs.
        protected = ProtectedInputs(store, protected.authorized | {onto}, protected.pins | {onto})
    return Stack(MappingProxyType(replacements), result.objects, tuple(events))


@dataclass(frozen=True)
class Lineage:
    """Immutable candidate metadata snapshot; appending publishes all or nothing.

    Edges are the explicit cross-product of each event's endpoint sets, NOT
    canonical object edges. Split/fold events record relationships only: they do
    not prove source decomposition, fresh allocation or endpoint availability.
    The 128-event/1024-edge limits are fixture budgets, not wire-format limits.
    """
    events: tuple[Evolution, ...] = ()

    def append(self, *events: Evolution) -> Lineage:
        return Lineage(self.events + events)

    def __post_init__(self):
        if len(self.events) > 128:
            raise MergeTooComplex('lineage reference event budget')
        graph = {}
        edges = 0
        for event in self.events:
            if (event.kind not in ('amend', 'rebase', 'split', 'fold')
                    or not event.predecessors or not event.successors
                    or len(set(event.predecessors)) != len(event.predecessors)
                    or len(set(event.successors)) != len(event.successors)
                    or any(ref.kind != K_REV for ref in event.predecessors + event.successors)):
                raise Refused('invalid evolution event')
            if event.kind in ('amend', 'rebase') and (len(event.predecessors), len(event.successors)) != (1, 1):
                raise Refused('rewrite requires one predecessor and one successor')
            for predecessor in event.predecessors:
                targets = graph.setdefault(predecessor, set())
                for successor in event.successors:
                    if predecessor == successor:
                        raise Refused('self evolution edge')
                    if successor not in targets:
                        targets.add(successor)
                        edges += 1
                        if edges > 1024:
                            raise MergeTooComplex('lineage reference edge budget')
        indegree = {ref: 0 for ref in graph}
        for targets in graph.values():
            for target in targets:
                indegree[target] = indegree.get(target, 0) + 1
        ready = [ref for ref, count in indegree.items() if not count]
        visited = 0
        while ready:
            ref = ready.pop()
            visited += 1
            for target in graph.get(ref, ()):
                indegree[target] -= 1
                if indegree[target] == 0:
                    ready.append(target)
        if visited != len(indegree):
            raise Refused('evolution cycle')
