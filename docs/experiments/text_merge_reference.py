"""Bounded conservative text-merge reference, not a native engine.

Run: python -m unittest discover -s docs/experiments -p '*reference.py' -v
Exact LCS is a longest chain of byte-equal line pairs. Ambiguous alignments
may merge only when every optimal pairing gives the same clean byte result.
"""
from array import array
from bisect import bisect_left, bisect_right
from collections import Counter, defaultdict
from functools import lru_cache
from itertools import chain, combinations, product
import re
import unittest

MAX_MATCHES = 4_000_000
MAX_RANK_WORK = 64_000_000
MAX_ALIGNMENTS = 64
MAX_ENUM_WORK = 4_000_000
MAX_COMBINATIONS = 256
MAX_MERGE_BYTES = 32 * 1024 * 1024


class AlignmentBudget(ValueError):
    """A deterministic driver budget, not an allocation or execution failure."""


def lines(data: bytes) -> list[bytes]:
    return re.findall(rb'[^\r\n]*(?:\r\n|\r|\n)|[^\r\n]+$', data)


class AlignmentIndex:
    """Sparse suffix lengths and capped counts, without Python pair-state nodes.

    Rank one match (p,q) by 1 + LCS(base[p+1:], side[q+1:]). Store ranks in
    row-major packed integers; side positions and row offsets identify pairs.
    Fenwick nodes aggregate disjoint side ranges, counting distinct chains by
    their first pair. Counts saturate at 65, sufficient for every admitted cap.
    """
    def __init__(self, base, side, *, max_matches=MAX_MATCHES, max_rank_work=MAX_RANK_WORK):
        self.base, self.m = base, len(side)
        self.positions = defaultdict(lambda: array('I'))
        for j, token in enumerate(side):
            self.positions[token].append(j)
        matches = sum(count * len(self.positions.get(token, ()))
                      for token, count in Counter(base).items())
        if matches > max_matches or matches * self.m.bit_length() > max_rank_work:
            raise AlignmentBudget('equal-line match-pair budget exceeded')
        self.offsets = array('I', [0])
        for token in base:
            self.offsets.append(self.offsets[-1] + len(self.positions.get(token, ())))
        self.ranks = array('I', [0]) * matches
        lengths = array('I', [0]) * (self.m + 1)
        counts = array('I', [0]) * (self.m + 1)

        def prefix(end):
            length = count = 0
            while end:
                if lengths[end] > length:
                    length, count = lengths[end], counts[end]
                elif lengths[end] == length:
                    count = min(MAX_ALIGNMENTS + 1, count + counts[end])
                end -= end & -end
            return (length, count) if length else (0, 1)

        for p in range(len(base) - 1, -1, -1):
            # Descending base rows, ASCENDING side positions. Updates from this
            # row have smaller q and cannot enter a strict-greater-q query.
            for slot, q in enumerate(self.positions.get(base[p], ())):
                length, count = prefix(self.m - q - 1)
                length += 1
                self.ranks[self.offsets[p] + slot] = length
                index = self.m - q
                while index <= self.m:
                    if length > lengths[index]:
                        lengths[index], counts[index] = length, count
                    elif length == lengths[index]:
                        counts[index] = min(MAX_ALIGNMENTS + 1, counts[index] + count)
                    index += index & -index
        # Do not refuse on an intermediate saturated count: earlier rows can
        # supply a strictly longer, unique optimum (e.g. equal repeated files).
        self.length, self.count = prefix(self.m)

    def alignments(self, *, max_variants=MAX_ALIGNMENTS, max_work=MAX_ENUM_WORK):
        if not 1 <= max_variants <= MAX_ALIGNMENTS:
            raise ValueError('variant limit outside the candidate driver bound')
        if self.count > max_variants:
            raise AlignmentBudget('alignment variant budget exceeded')
        if not self.length:
            yield []
            return
        if self.count == 1:
            # A monotone scan reconstructs the unique chain in O(r+n), without
            # parent pointers, DFS frames or the ambiguous enumeration budget.
            pairs, j, remaining = [], 0, self.length
            for p, token in enumerate(self.base):
                positions = self.positions.get(token, ())
                for slot in range(bisect_left(positions, j), len(positions)):
                    if self.ranks[self.offsets[p] + slot] == remaining:
                        q = positions[slot]
                        pairs.append((p, q))
                        j, remaining = q + 1, remaining - 1
                        break
                if not remaining:
                    break
            assert not remaining
            yield pairs
            return
        work = 0
        def candidates(i, j, remaining):
            nonlocal work
            for p in range(i, len(self.base) - remaining + 1):
                work += 1
                if work > max_work:
                    raise AlignmentBudget('alignment enumeration work exceeded')
                positions = self.positions.get(self.base[p], ())
                lo, hi = bisect_left(positions, j), bisect_right(positions, self.m - remaining)
                for slot in range(lo, hi):
                    work += 1
                    if work > max_work:
                        raise AlignmentBudget('alignment enumeration work exceeded')
                    if self.ranks[self.offsets[p] + slot] == remaining:
                        yield p, positions[slot]
        # Keep one mutable path, sharing its immutable pairs across yielded
        # shallow copies. Copying the full prefix at every step is quadratic.
        pairs, stack = [], [candidates(0, 0, self.length)]
        while stack:
            try:
                p, q = next(stack[-1])
            except StopIteration:
                stack.pop()
                if pairs:
                    pairs.pop()
                continue
            pairs.append((p, q))
            remaining = self.length - len(pairs)
            if remaining:
                stack.append(candidates(p + 1, q + 1, remaining))
            else:
                yield list(pairs)
                pairs.pop()


def unique_alignment(base, side, *, max_matches=MAX_MATCHES, max_rank_work=MAX_RANK_WORK):
    """Return the unique optimal index-pair sequence, or None if ambiguous."""
    index = AlignmentIndex(base, side, max_matches=max_matches, max_rank_work=max_rank_work)
    return next(index.alignments()) if index.count == 1 else None


def hunks(base, side, anchors):
    result = []
    old = new = 0
    for i, j in chain(anchors, ((len(base), len(side)),)):
        if old != i or new != j:
            result.append((old, i, tuple(side[new:j])))
        old, new = i + 1, j + 1
    return result


def optimal_alignments(base, side, *, max_variants=MAX_ALIGNMENTS, max_work=MAX_ENUM_WORK):
    """Materialize bounded complete pair sets for independent qualification."""
    return list(AlignmentIndex(base, side).alignments(max_variants=max_variants, max_work=max_work))


def merge_hunks(base, lh, rh):
    i = j = 0
    while i < len(lh) and j < len(rh):
        a, c = lh[i], rh[j]
        if a == c:
            i, j = i + 1, j + 1
            continue
        a0, a1, _ = a
        c0, c1, _ = c
        if a0 == a1:
            overlap = c0 <= a0 <= c1
        elif c0 == c1:
            overlap = a0 <= c0 <= a1
        else:
            overlap = max(a0, c0) < min(a1, c1)
        if overlap:
            return None
        if a0 < c0:
            i += 1
        else:
            j += 1
    output = []
    previous = 0
    for start, end, replacement in sorted(set(lh + rh)):
        output.extend(base[previous:start])
        output.extend(replacement)
        previous = end
    output.extend(base[previous:])
    return b''.join(output)


def merge_text(base: bytes, left: bytes, right: bytes) -> bytes | None:
    """None denotes a path conflict. Execution failures deliberately propagate.

    Only content-derived budget refusal is a source conflict; allocation failure,
    cancellation and other execution failures cannot invent an admitted result.
    Mode/type checks belong to the enclosing source merger, not this byte probe.
    """
    if left == right:
        return left
    if left == base:
        return right
    if right == base:
        return left
    for data in (base, left, right):
        if len(data) > 1_048_576 or b'\0' in data:
            return None
        try:
            data.decode('utf-8')
        except UnicodeDecodeError:
            return None
    b, l, r = map(lines, (base, left, right))
    try:
        left_index = AlignmentIndex(b, l)
        if left_index.count > MAX_ALIGNMENTS:
            return None
        right_index = AlignmentIndex(b, r)
        combinations = left_index.count * right_index.count
        if (right_index.count > MAX_ALIGNMENTS or combinations > MAX_COMBINATIONS or
                combinations * (len(base) + len(left) + len(right)) > MAX_MERGE_BYTES):
            return None
        # Check cardinality/byte work BEFORE expanding alternatives. Retain only
        # hunks, not a full mostly-identical anchor list for every alignment.
        left_options = [hunks(b, l, a) for a in left_index.alignments()]
        right_options = [hunks(b, r, a) for a in right_index.alignments()]
    except AlignmentBudget:
        return None
    result = None
    for lh in left_options:
        for rh in right_options:
            candidate = merge_hunks(b, lh, rh)
            if candidate is None or (result is not None and candidate != result):
                return None
            result = candidate
    return result


class TextMergeQualification(unittest.TestCase):
    def test_sparse_alignment_matches_exhaustive_pair_sequences(self):
        sequences = [p for n in range(5) for p in product((b'a', b'b'), repeat=n)]
        for base in sequences:
            for side in sequences:
                @lru_cache(None)
                def align(i, j):
                    results = {()}
                    for p in range(i, len(base)):
                        for q in range(j, len(side)):
                            if base[p] == side[q]:
                                results.update(((p, q),) + tail for tail in align(p + 1, q + 1))
                    length = max(map(len, results))
                    return frozenset(a for a in results if len(a) == length)
                expected = align(0, 0)
                actual = unique_alignment(base, side)
                self.assertEqual(actual, list(next(iter(expected))) if len(expected) == 1 else None,
                                 (base, side))
                self.assertEqual(AlignmentIndex(base, side).count, len(expected))
                options = optimal_alignments(base, side)
                self.assertEqual(len(options), len(expected))
                self.assertEqual({tuple(a) for a in options}, expected)

    def test_large_unique_file_combines_distant_edits(self):
        base = [f'line-{i:04d}\n'.encode() for i in range(2001)]
        left, right = base.copy(), base.copy()
        left[0], right[-1] = b'first\n', b'last\n'
        expected = [left[0], *base[1:-1], right[-1]]
        self.assertEqual(merge_text(b''.join(base), b''.join(left), b''.join(right)),
                         b''.join(expected))

    def test_large_harmless_ambiguity_without_a_dense_matrix(self):
        base = [f'line-{i:04d}\n'.encode() for i in range(2001)]
        base[1000:1000] = [b'\n', b'\n']
        left, right = base.copy(), base.copy()
        del left[1000]
        right[-1] = b'last\n'
        expected = left.copy()
        expected[-1] = right[-1]
        self.assertEqual(merge_text(*map(b''.join, (base, left, right))), b''.join(expected))

    def test_partial_ambiguity_does_not_refuse_a_later_unique_optimum(self):
        base = [b'a'] * 20
        index = AlignmentIndex(base, base)
        self.assertEqual(index.count, 1)
        self.assertEqual(list(index.alignments()), [list(zip(range(20), range(20)))])

    def test_exact_lines_and_conservative_hunks(self):
        cases = [
            (b'a\nb\n', b'A\nb\n', b'a\nB\n', b'A\nB\n'),
            (b'a\nb\n', b'A\nb\n', b'a\nx\nb\n', None),  # insertion at end boundary
            (b'a\nb\n', b'l\na\nb\n', b'a\nb\nr\n', b'l\na\nb\nr\n'),
            (b'a\nb\n', b'x\na\nb\n', b'y\na\nb\n', None),
            # Both possible deletions give exactly the same clean output.
            (b'a\na\nb\n', b'a\nb\n', b'a\na\nB\n', b'a\nB\n'),
            (b'a\na\n', b'a\n', b'A\na\n', None),
            (b'a\nb\nc\n', b'A\nb\nc\n', b'A\nb\nC\n', b'A\nb\nC\n'),
            (b'a\r\nb\rc', b'A\r\nb\rc', b'a\r\nb\rC', b'A\r\nb\rC'),
        ]
        for base, left, right, expected in cases:
            with self.subTest(base=base, left=left, right=right):
                self.assertEqual(merge_text(base, left, right), expected)
                self.assertEqual(merge_text(base, right, left), expected)
        self.assertEqual(lines(b'\na\r\nb\rc'), [b'\n', b'a\r\n', b'b\r', b'c'])

    def test_match_budgets_are_content_derived(self):
        # Four match pairs, with a unique two-pair maximum chain.
        self.assertEqual(unique_alignment([b'a'] * 2, [b'a'] * 2,
                                         max_matches=4, max_rank_work=8), [(0, 0), (1, 1)])
        for limits in ({'max_matches': 3}, {'max_rank_work': 7}):
            with self.assertRaises(AlignmentBudget):
                unique_alignment([b'a'] * 2, [b'a'] * 2, **limits)
        self.assertIsNone(unique_alignment([b'a'], [b'a', b'a']))
        self.assertEqual(len(optimal_alignments([b'a'] * 2, [b'a'], max_work=4)), 2)
        for limits in ({'max_variants': 1}, {'max_work': 3}):
            with self.assertRaises(AlignmentBudget):
                optimal_alignments([b'a'] * 2, [b'a'], **limits)
        base = b'a\n' + b'\n' * 20 + b'b\n' + b'\n' * 20 + b'c\n'
        left = b'a\n' + b'\n' * 19 + b'b\n' + b'\n' * 20 + b'c\n'
        right = b'a\n' + b'\n' * 20 + b'b\n' + b'\n' * 19 + b'c\n'
        self.assertIsNone(merge_text(base, left, right))  # 20 * 20 > 256 pairs
        self.assertEqual(len(optimal_alignments([b'a'] * 64, [b'a'])), 64)
        with self.assertRaises(AlignmentBudget):
            optimal_alignments([b'a'] * 65, [b'a'])
        base = b'a\n' + b'\n' * 4 + b'b\n' + b'\n' * 4 + b'c\n'
        left = b'a\n' + b'\n' * 3 + b'b\n' + b'\n' * 4 + b'c\n'
        right = b'a\n' + b'\n' * 4 + b'b\n' + b'\n' * 3 + b'c\n'
        self.assertEqual(merge_text(base, left, right),
                         b'a\n' + b'\n' * 3 + b'b\n' + b'\n' * 3 + b'c\n')
        prefix = b'x' * 800_000 + b'\n'
        self.assertIsNone(merge_text(prefix + base, prefix + left, prefix + right))
        # Same 16 pairings; the declared byte-work limit now refuses them.

    def test_three_way_confluence_matches_exhaustive_reference(self):
        sequences = [p for n in range(4) for p in product((b'a\n', b'b\n'), repeat=n)]
        # Independent subset enumeration, not either production LCS algorithm.
        @lru_cache(None)
        def edits(base, side):
            for size in range(min(len(base), len(side)), -1, -1):
                choices = []
                for bi, si in product(combinations(range(len(base)), size),
                                      combinations(range(len(side)), size)):
                    if any(base[i] != side[j] for i, j in zip(bi, si)):
                        continue
                    changes, old, new = [], 0, 0
                    for i, j in [*zip(bi, si), (len(base), len(side))]:
                        if (old, new) != (i, j):
                            changes.append((old, i, side[new:j]))
                        old, new = i + 1, j + 1
                    choices.append(changes)
                if choices:
                    return choices
        def outcome(base, left, right):
            if left == right or right == base:
                return b''.join(left)
            if left == base:
                return b''.join(right)
            outputs = set()
            for lh, rh in product(edits(base, left), edits(base, right)):
                for a, b in product(lh, rh):
                    if a == b:
                        continue
                    ap, bp = set(range(a[0], a[1])), set(range(b[0], b[1]))
                    if (ap & bp or (not ap and b[0] <= a[0] <= b[1]) or
                            (not bp and a[0] <= b[0] <= a[1])):
                        return None
                output, position = [], 0
                for start, end, replacement in sorted(set(lh + rh)):
                    output.extend(base[position:start]); output.extend(replacement)
                    position = end
                outputs.add(b''.join(output + list(base[position:])))
            return outputs.pop() if len(outputs) == 1 else None
        for base, left, right in product(sequences, repeat=3):
            self.assertEqual(merge_text(*map(b''.join, (base, left, right))),
                             outcome(base, left, right), (base, left, right))


if __name__ == '__main__':
    unittest.main()
