"""Read-only agreed descendant queries over a protected, complete State.

Known contains a typed Entry descriptor or proven absence, not Blob bytes.
An endpoint Conflict remains that exact opaque Entry. Traversing it compares
live alternatives, never its historical base or flattened nested terms.
EvidenceUnavailable denotes an unknown/unacquired input, never source absence.
Full-closure acquisition precedes queries: sparse promises/hydration are NOT
implemented, and an incomplete closure refuses before a reader is created.
"""
from dataclasses import dataclass
from codec_reference import Ref, K_STATE
from codec_records import Path
from codec_verify import Verifier, entry_ref
from protected_inputs import ProtectedInputs, Refused


@dataclass(frozen=True)
class Known:
    value: bytes | None


@dataclass(frozen=True)
class Obstructed:
    entry: bytes  # a File/Symlink cannot be traversed as a directory
    remaining: Path


@dataclass(frozen=True)
class Unresolved:
    """An ephemeral query result, never a persistent Entry or chosen branch."""


Result = Known | Obstructed | Unresolved


class SourceQueries:
    def __init__(self, state: Ref, inputs: ProtectedInputs):
        if state.kind != K_STATE:
            raise Refused('expected State')
        self._inputs = inputs  # retain the protected snapshot for this reader
        self._root = inputs.payload(state)
        verifier = Verifier(inputs.objects)
        verifier.state(state)
        self._maps, self._conflicts = verifier.maps, verifier.conflicts

    def read(self, path: Path) -> Result:
        path = Verifier.checked_path(path)  # validate even an unreachable suffix
        # Per-request memoization, not an unbounded reader-lifetime query cache.
        return self._read(self._root, path, {})

    def _read(self, entry: bytes | None, path: Path, memo: dict) -> Result:
        key = entry, path
        if key in memo:
            return memo[key]
        if entry is None or not path:
            result = Known(entry)
        elif entry[0] == 3:
            child = self._maps[entry_ref(entry)].get(path[0])
            result = self._read(child, path[1:], memo)
        elif entry[0] == 4:
            conflict = self._conflicts[entry_ref(entry)]
            # The base is an owned historical comparison value, not another
            # live choice. Every alternative, including absence, participates.
            left, right = (self._read(term, path, memo) for term in conflict.alternatives)
            result = left if left == right else Unresolved()
        else:
            result = Obstructed(entry, path)
        memo[key] = result
        return result
