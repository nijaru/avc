"""Decoded closed provenance values; commitments here do not confer custody."""
from __future__ import annotations
from dataclasses import dataclass
from typing import ClassVar
from codec_reference import Ref

Path = tuple[bytes, ...]


@dataclass(frozen=True)
class Move:
    source: Path
    destination: Path
    expected_source: bytes
    expected_destination: bytes | None


@dataclass(frozen=True)
class Copy:
    value: bytes
    destination: Path
    expected_destination: bytes | None


@dataclass(frozen=True)
class Edit:
    path: Path
    expected: bytes | None
    value: bytes | None


@dataclass(frozen=True)
class Transform:
    tag: ClassVar[int] = 1
    base: Ref
    structural: tuple[Move | Copy, ...]
    edits: tuple[Edit, ...]


@dataclass(frozen=True)
class Merge:
    tag: ClassVar[int] = 2
    # Exactly one mode is populated by the decoder.
    history: tuple[int, int, bool] | None
    source: tuple[Ref, Ref, Ref] | None
    source_algorithm: tuple[int, int]
    text_algorithm: tuple[int, int]
    moves: tuple[Ref | None, Ref | None]


@dataclass(frozen=True)
class Observation:
    tag: ClassVar[int] = 3
    base: Ref
    revision: Ref | None
    generation: int
    consistency: int
    receipt: bytes | None
    partial: bool
    coverage_mode: int
    scope: tuple[Path, ...]
    covered: tuple[Path, ...]
    failed: tuple[Path, ...]
    skipped: tuple[Path, ...]
    resolutions: tuple[tuple[Path, Ref, bytes | None], ...]


@dataclass(frozen=True)
class Provenance:
    result: Ref
    inputs: tuple[Ref, ...]
    occurrences: tuple[tuple[Path, Ref, tuple[int, ...]], ...]
    records: tuple[Transform | Merge | Observation, ...]
