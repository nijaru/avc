"""Bounded conservative text-merge reference, not a native engine.

Run: python -m unittest discover -s docs/experiments -p '*reference.py' -v
Exact LCS is a longest chain of byte-equal line pairs. Ambiguous alignments
may merge only when every optimal pairing gives the same clean byte result.
"""
from array import array
from bisect import bisect_left, bisect_right
from collections import Counter, defaultdict
from functools import lru_cache
from itertools import combinations, product
import re
import unittest

MAX_MATCHES = 4_000_000
MAX_RANK_WORK = 64_000_000
MAX_DENSE_CELLS = 1_000_000
MAX_ALIGNMENTS = 64
MAX_ENUM_WORK = 4_000_000
MAX_COMBINATIONS = 256
MAX_MERGE_BYTES = 32 * 1024 * 1024


class AlignmentBudget(ValueError):
    """A deterministic driver budget, not an allocation or execution failure."""


def lines(data: bytes) -> list[bytes]:
    return re.findall(rb'[^\r\n]*(?:\r\n|\r|\n)|[^\r\n]+$', data)


def unique_alignment(base, side, *, max_matches=MAX_MATCHES, max_rank_work=MAX_RANK_WORK):
    """Return the unique optimal index-pair sequence, or None if ambiguous.

    Each state counts chains ending at one particular match. Fenwick nodes
    aggregate disjoint endpoint ranges; empty prefixes contain one empty chain.
    All queries for a base row precede its updates, preventing same-row chains.
    """
    positions = defaultdict(list)
    for j, token in enumerate(side):
        positions[token].append(j)
    matches = sum(count * len(positions[token]) for token, count in Counter(base).items())
    if matches > max_matches or matches * len(side).bit_length() > max_rank_work:
        raise AlignmentBudget('equal-line match-pair budget exceeded')
    # (length, number of maximum chains capped at 2, representative endpoint)
    empty = (0, 0, None)
    tree = [empty] * (len(side) + 1)
    states = []

    def combine(a, b):
        if a[0] != b[0]:
            return a if a[0] > b[0] else b
        return (a[0], min(2, a[1] + b[1]), a[2] if a[1] else b[2])

    def prefix(end):
        best = empty
        while end:
            best = combine(best, tree[end])
            end -= end & -end
        return best if best[0] else (0, 1, None)

    for i, token in enumerate(base):
        row = []
        for j in positions[token]:
            length, count, previous = prefix(j)  # side indices strictly < j
            state = len(states)
            states.append((i, j, previous))
            row.append((j + 1, (length + 1, count, state)))
        for index, value in row:
            while index < len(tree):
                tree[index] = combine(tree[index], value)
                index += index & -index
    _, count, endpoint = prefix(len(side))
    if count != 1:
        return None
    result = []
    while endpoint is not None:
        i, j, endpoint = states[endpoint]
        result.append((i, j))
    return list(reversed(result))


def hunks(base, side, anchors):
    result = []
    old = new = 0
    for i, j in [*anchors, (len(base), len(side))]:
        if old != i or new != j:
            result.append((old, i, tuple(side[new:j])))
        old, new = i + 1, j + 1
    return result


def optimal_alignments(base, side, *, max_cells=MAX_DENSE_CELLS,
                       max_variants=MAX_ALIGNMENTS, max_work=MAX_ENUM_WORK):
    """All optimal pair sequences, with a sparse fast path for unique input.

    The dense fallback is bounded before allocation. Enumerating first matched
    pairs avoids duplicate paths caused by interleaved skips in a DP traceback.
    """
    unique = unique_alignment(base, side)
    if unique is not None:
        return [unique]
    n, m = len(base), len(side)
    if (n + 1) * (m + 1) > max_cells:
        raise AlignmentBudget('ambiguous alignment matrix budget exceeded')
    table = [array('I', [0]) * (m + 1) for _ in range(n + 1)]
    for i in range(n - 1, -1, -1):
        row, following = table[i], table[i + 1]
        for j in range(m - 1, -1, -1):
            row[j] = (1 + following[j + 1] if base[i] == side[j]
                      else max(following[j], row[j + 1]))
    positions = defaultdict(list)
    for j, token in enumerate(side):
        positions[token].append(j)
    result, stack, work = [], [(0, 0, table[0][0], ())], 0
    while stack:
        i, j, remaining, pairs = stack.pop()
        if not remaining:
            result.append(list(pairs))
            continue
        for p in range(i, n - remaining + 1):
            work += 1
            if work > max_work:
                raise AlignmentBudget('alignment enumeration work exceeded')
            matches = positions[base[p]]
            lo, hi = bisect_left(matches, j), bisect_right(matches, m - remaining)
            for index in range(lo, hi):
                q = matches[index]
                work += 1
                if work > max_work:
                    raise AlignmentBudget('alignment enumeration work exceeded')
                if table[p + 1][q + 1] == remaining - 1:
                    stack.append((p + 1, q + 1, remaining - 1, pairs + ((p, q),)))
                    # Every pending state has at least one distinct completion.
                    if len(result) + len(stack) > max_variants:
                        raise AlignmentBudget('alignment variant budget exceeded')
    return result


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
        left_options, right_options = optimal_alignments(b, l), optimal_alignments(b, r)
    except AlignmentBudget:
        return None
    combinations = len(left_options) * len(right_options)
    if (combinations > MAX_COMBINATIONS or
            combinations * (len(base) + len(left) + len(right)) > MAX_MERGE_BYTES):
        return None
    result = None
    for la in left_options:
        for ra in right_options:
            candidate = merge_hunks(b, hunks(b, l, la), hunks(b, r, ra))
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
                self.assertEqual({tuple(a) for a in optimal_alignments(base, side)}, expected)

    def test_large_unique_file_combines_distant_edits(self):
        base = [f'line-{i:04d}\n'.encode() for i in range(2001)]
        left, right = base.copy(), base.copy()
        left[0], right[-1] = b'first\n', b'last\n'
        expected = [left[0], *base[1:-1], right[-1]]
        self.assertEqual(merge_text(b''.join(base), b''.join(left), b''.join(right)),
                         b''.join(expected))

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
        self.assertEqual(len(optimal_alignments([b'a'] * 2, [b'a'],
                                                max_cells=6, max_work=4)), 2)
        for limits in ({'max_cells': 5}, {'max_variants': 1}, {'max_work': 3}):
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
