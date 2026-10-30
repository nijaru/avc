"""Bounded owning-closure custody model, not a store or durability proof.

Six immutable fixture labels stand for already written objects. Atomic closure
reservation abstracts a known complete manifest, NOT traversal/decoding inside a
writer transaction. Verification, tracing and per-object export are separate
steps. One writer, reader, exporter and GC run once; GC may restart after the
finite actors' protection mutations. Retirement is irreversible here; physical
segment cleanup is exercised separately by the SQLite probes.
"""
from collections import deque
from dataclasses import dataclass, replace
from functools import cache


OWNING = {'A': ('shared', 'a'), 'B': ('shared', 'b'),
          'shared': (), 'a': (), 'b': (), 'audit': ()}
AUDIT = {'A': ('audit',)}  # Commitments are deliberately NOT owning edges.
OBJECTS = tuple(sorted(OWNING))


@cache
def closure(root):
    return frozenset((root,)).union(*(closure(child) for child in OWNING[root]))


def closures(roots):
    return frozenset().union(*(closure(root) for root in roots))


@dataclass(frozen=True, slots=True)
class Custody:
    live: frozenset = frozenset(OBJECTS)
    root: str = 'A'
    epoch: int = 0
    writer_pc: int = 0  # reserve B; verify; publish; done (or refused)
    reader_pc: int = 0  # acquire current root; renew; expire/stop; done
    reader_root: str = ''
    export_pc: int = 0  # select+lease; copy/finish; done
    export_root: str = ''  # Immutable selected descriptor, retained after finish.
    copied: frozenset = frozenset()
    gc_pc: int = 0  # snapshot+lease; trace; retire; done
    scan_roots: frozenset = frozenset()
    scan_epoch: int = -1
    marked: frozenset = frozenset()
    cursor: int = 0

    def roots(self):
        roots = {self.root}
        if self.writer_pc in (1, 2):
            roots.add('B')
        if self.reader_pc in (1, 2):
            roots.add(self.reader_root)
        if self.export_pc == 1:
            roots.add(self.export_root)
        return frozenset(roots)


def violation(s):
    if not closures(s.roots() | s.scan_roots) <= s.live:
        return 'protected owning closure retired'
    if s.export_root:
        selected = closure(s.export_root)
        if not s.copied <= selected or (s.export_pc == 2 and s.copied != selected):
            return 'export differs from selected descriptor'
    return None


def steps(s, *, fault=None):
    """Linearizable authority steps plus outside-writer verify/trace/copy.

    Faults are explicit negative controls, never alternate supported protocols.
    No current-closure oracle is used by retirement: safety depends on the traced
    mark and the generation barrier, just as in the intended collector.
    """
    def bump():
        return (s.epoch + 1) % 2 if fault == 'wrapping-epoch' else s.epoch + 1

    if s.writer_pc == 0:
        if closure('B') <= s.live:
            epoch = s.epoch if fault == 'unversioned-stage' else bump()
            yield 'writer-reserve', replace(s, writer_pc=1, epoch=epoch)
        else:
            yield 'writer-refuse', replace(s, writer_pc=3)
    elif s.writer_pc == 1:
        # Real verification must bind exact bytes/closure; this fixture already
        # fixes both. The stage stays protected while verification runs.
        yield 'writer-verify', replace(s, writer_pc=2)
    elif s.writer_pc == 2:
        yield 'writer-publish', replace(s, writer_pc=3, root='B', epoch=bump())

    if s.reader_pc == 0:
        yield 'reader-acquire', replace(s, reader_pc=1, reader_root=s.root, epoch=bump())
    elif s.reader_pc == 1:
        yield 'reader-renew', replace(s, reader_pc=2, epoch=bump())
    elif s.reader_pc == 2:
        # Expiry is an explicit versioned authority action. This reader stops;
        # no wall-clock filter can silently remove its protection mid-read.
        yield 'reader-expire', replace(s, reader_pc=3, epoch=bump())

    if s.export_pc == 0:
        yield 'export-select-and-lease', replace(s, export_pc=1,
                                                export_root=s.root, epoch=bump())
    elif s.export_pc == 1:
        remaining = closure(s.export_root) - s.copied
        if remaining:
            obj = min(remaining)
            yield f'export-copy:{obj}', replace(s, copied=s.copied | {obj})
        else:
            # Abstract successful verification/flush; actual I/O is not modeled.
            yield 'export-finish', replace(s, export_pc=2, epoch=bump())

    if s.gc_pc == 0:
        # The scan lease duplicates current roots, then preserves them even if
        # those roots move/expire. Capture the epoch AFTER acquiring this lease.
        yield 'gc-snapshot-and-lease', replace(s, gc_pc=1, scan_roots=s.roots(),
                                              epoch=bump(), scan_epoch=bump())
    elif s.gc_pc == 1:
        marked = s.scan_roots if fault == 'roots-only' else closures(s.scan_roots)
        yield 'gc-trace', replace(s, gc_pc=2, marked=marked)
    elif s.gc_pc == 2:
        if s.epoch != s.scan_epoch:
            yield 'gc-restart', replace(s, gc_pc=0, scan_roots=frozenset(),
                                        marked=frozenset(), scan_epoch=-1,
                                        cursor=0, epoch=bump())
        elif s.cursor == len(OBJECTS):
            yield 'gc-finish', replace(s, gc_pc=3, scan_roots=frozenset(), epoch=bump())
        else:
            obj = OBJECTS[s.cursor]
            live = s.live if obj in s.marked else s.live - {obj}
            yield f'gc-check:{obj}', replace(s, live=live, cursor=s.cursor + 1)


def reachable(start=Custody(), *, fault=None):
    """Coalesce equal states, retaining one shortest schedule per state."""
    queue = deque([(start, ())])
    seen = {start}
    while queue:
        state, trace = queue.popleft()
        yield state, trace
        for action, nxt in steps(state, fault=fault):
            if nxt not in seen:
                seen.add(nxt)
                queue.append((nxt, (*trace, action)))
