//! Bounded, byte-preserving `avc.text.diff3/v1` content merge (format §E).
//!
//! A clean result is a textual composition, not evidence of program correctness.
//! Modes, typed conflicts, ancestry, and retaining pending inputs belong to callers.
//!
//! Algorithm choices made explicit here: trim equal prefixes/suffixes in every
//! patience region; find lines unique in *both* sides of that region; select a
//! strictly increasing LIS of side positions, replacing a tail with the smaller
//! position. Recurse into its gaps, using Myers only where no anchor exists.
//! Myers visits diagonals from negative to positive and prefers deletion when
//! predecessor reaches tie. There is no all-optimal-alignment search.
//!
//! Unequal insertions at the same gap conflict. An insertion at either endpoint
//! (or inside) a replacement/deletion also conflicts: its attachment is uncertain.
//! Adjacent nonempty base ranges, however, are disjoint. Exact same hunks coalesce.

use std::cmp::Ordering;

const TEXT_LIMIT: usize = 8 * 1024 * 1024;
const WORK_LIMIT: &str = "text merge work limit";
const BYTE_LIMIT: &str = "text merge byte limit";
const ALLOCATION: &str = "text merge allocation failed";

type Limited<T> = Result<T, &'static str>;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Clean(Vec<u8>),
    Conflict,
    Pending(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct Budget {
    /// Deterministic algorithmic units, not elapsed time: one per visited line,
    /// index/frontier/copy step, plus ceil(bytes / 64) per byte scan/comparison.
    /// Exact equality/base triage and fixed-size text eligibility scans precede
    /// this budget. Allocation failure never selects a different diff/result.
    pub work: usize,
    /// Per-input and output byte ceiling. Inputs above the fixed 8 MiB text
    /// eligibility limit are opaque (Conflict after equality/base triage).
    /// A lower configured ceiling or an oversized composed output is Pending.
    /// Auxiliary storage is bounded by input lines and charged work, not this
    /// byte ceiling; in particular Myers trace storage is O(charged work).
    pub max_bytes: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            work: 4_000_000,
            max_bytes: TEXT_LIMIT,
        }
    }
}

/// Merge exact file contents, without markers, encoding/newline normalization,
/// wall-clock cutoffs, or an external driver. Invalid UTF-8/NUL is opaque binary,
/// not malformed source. Equality/base fast paths apply even to opaque inputs
/// and bypass configured limits (apart from fallible result allocation).
pub fn merge(base: &[u8], left: &[u8], right: &[u8], budget: Budget) -> Outcome {
    let exact = if left == right {
        Some(left)
    } else if left == base {
        Some(right)
    } else if right == base {
        Some(left)
    } else {
        None
    };
    if let Some(bytes) = exact {
        return match copy_bytes(bytes) {
            Ok(bytes) => Outcome::Clean(bytes),
            Err(reason) => Outcome::Pending(reason),
        };
    }
    if [base, left, right].iter().any(|bytes| {
        bytes.len() > TEXT_LIMIT || bytes.contains(&0) || std::str::from_utf8(bytes).is_err()
    }) {
        return Outcome::Conflict;
    }
    if [base, left, right]
        .iter()
        .any(|bytes| bytes.len() > budget.max_bytes)
    {
        return Outcome::Pending(BYTE_LIMIT);
    }
    // Canonicalize side order, including work/refusal behavior under side swap.
    let (left, right) = if left < right {
        (left, right)
    } else {
        (right, left)
    };
    match merge_text(base, left, right, budget) {
        Ok(Some(bytes)) => Outcome::Clean(bytes),
        Ok(None) => Outcome::Conflict,
        Err(reason) => Outcome::Pending(reason),
    }
}

fn copy_bytes(bytes: &[u8]) -> Limited<Vec<u8>> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(bytes.len())
        .map_err(|_| ALLOCATION)?;
    result.extend_from_slice(bytes);
    Ok(result)
}

struct Work(usize);

impl Work {
    fn take(&mut self, units: usize) -> Limited<()> {
        self.0 = self.0.checked_sub(units).ok_or(WORK_LIMIT)?;
        Ok(())
    }
}

fn push<T>(items: &mut Vec<T>, item: T, work: &mut Work) -> Limited<()> {
    work.take(1)?;
    items.try_reserve(1).map_err(|_| ALLOCATION)?;
    items.push(item);
    Ok(())
}

fn compare(a: &[u8], b: &[u8], work: &mut Work) -> Limited<Ordering> {
    work.take(1 + a.len().min(b.len()).div_ceil(64))?;
    Ok(a.cmp(b))
}

fn equal(a: &[u8], b: &[u8], work: &mut Work) -> Limited<bool> {
    work.take(1)?;
    if a.len() != b.len() {
        return Ok(false);
    }
    work.take(a.len().div_ceil(64))?;
    Ok(a == b)
}

#[derive(Clone, Copy)]
struct Line<'a> {
    bytes: &'a [u8],
    offset: usize,
}

fn lines<'a>(bytes: &'a [u8], work: &mut Work) -> Limited<Vec<Line<'a>>> {
    work.take(bytes.len().div_ceil(64))?;
    let mut result = Vec::new();
    let mut start = 0;
    let mut end = 0;
    while end < bytes.len() {
        let byte = bytes[end];
        end += 1;
        if byte == b'\r' && bytes.get(end) == Some(&b'\n') {
            end += 1;
        }
        if byte == b'\r' || byte == b'\n' {
            push(
                &mut result,
                Line {
                    bytes: &bytes[start..end],
                    offset: start,
                },
                work,
            )?;
            start = end;
        }
    }
    if start < end {
        push(
            &mut result,
            Line {
                bytes: &bytes[start..end],
                offset: start,
            },
            work,
        )?;
    }
    Ok(result)
}

// Fallible, deterministic bottom-up mergesort. Comparator work can abort, unlike
// a std sort callback; all auxiliary allocations are fallible as well.
fn sort<T: Copy>(
    values: &mut [T],
    work: &mut Work,
    mut cmp: impl FnMut(&T, &T, &mut Work) -> Limited<Ordering>,
) -> Limited<()> {
    if values.len() < 2 {
        return Ok(());
    }
    work.take(values.len())?;
    let mut scratch = Vec::new();
    scratch
        .try_reserve_exact(values.len())
        .map_err(|_| ALLOCATION)?;
    scratch.extend_from_slice(values);
    let mut width = 1;
    while width < values.len() {
        for start in (0..values.len()).step_by(width * 2) {
            let mid = (start + width).min(values.len());
            let end = (mid + width).min(values.len());
            let (mut a, mut b) = (start, mid);
            for dest in &mut scratch[start..end] {
                work.take(1)?;
                if a < mid && (b == end || cmp(&values[a], &values[b], work)? != Ordering::Greater)
                {
                    *dest = values[a];
                    a += 1;
                } else {
                    *dest = values[b];
                    b += 1;
                }
            }
        }
        work.take(values.len())?;
        values.copy_from_slice(&scratch);
        width *= 2;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Position {
    side: bool,
    index: usize,
}

fn anchors(a: &[Line<'_>], b: &[Line<'_>], work: &mut Work) -> Limited<Vec<(usize, usize)>> {
    let mut positions = Vec::new();
    for (side, count) in [(false, a.len()), (true, b.len())] {
        for index in 0..count {
            push(&mut positions, Position { side, index }, work)?;
        }
    }
    let bytes = |p: &Position| {
        if p.side {
            b[p.index].bytes
        } else {
            a[p.index].bytes
        }
    };
    sort(&mut positions, work, |p, q, work| {
        compare(bytes(p), bytes(q), work)
    })?;
    let mut candidates = Vec::new();
    let mut start = 0;
    while start < positions.len() {
        work.take(1)?;
        let mut end = start + 1;
        while end < positions.len()
            && equal(bytes(&positions[start]), bytes(&positions[end]), work)?
        {
            end += 1;
        }
        if end - start == 2 && positions[start].side != positions[start + 1].side {
            let (p, q) = (positions[start], positions[start + 1]);
            let pair = if p.side {
                (q.index, p.index)
            } else {
                (p.index, q.index)
            };
            push(&mut candidates, pair, work)?;
        }
        start = end;
    }
    sort(&mut candidates, work, |p, q, work| {
        work.take(1)?;
        Ok(p.0.cmp(&q.0))
    })?;
    // Tails contain candidate indices; predecessor links recover a concrete LIS.
    let mut tails: Vec<usize> = Vec::new();
    let mut predecessors = Vec::new();
    for (index, &(_, side_pos)) in candidates.iter().enumerate() {
        work.take(1)?;
        let (mut low, mut high) = (0, tails.len());
        while low < high {
            work.take(1)?;
            let mid = low + (high - low) / 2;
            if candidates[tails[mid]].1 < side_pos {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        push(
            &mut predecessors,
            low.checked_sub(1).map(|i| tails[i]),
            work,
        )?;
        if low == tails.len() {
            push(&mut tails, index, work)?;
        } else {
            tails[low] = index;
        }
    }
    let mut result = Vec::new();
    let mut next = tails.last().copied();
    while let Some(index) = next {
        push(&mut result, candidates[index], work)?;
        next = predecessors[index];
    }
    work.take(result.len())?;
    result.reverse();
    Ok(result)
}

// Sparse Myers frontiers: at edit distance d, slot i denotes k = 2*i - d.
// Keep trace slots only after charging them, so quadratic trace growth cannot
// bypass the work budget. No heuristic changes the chosen shortest path.
fn myers(a: &[Line<'_>], b: &[Line<'_>], work: &mut Work) -> Limited<Vec<(usize, usize)>> {
    let mut trace: Vec<Vec<usize>> = Vec::new();
    for d in 0..=a.len() + b.len() {
        work.take(d + 1)?;
        let mut frontier = Vec::new();
        frontier.try_reserve_exact(d + 1).map_err(|_| ALLOCATION)?;
        let mut finished = false;
        for i in 0..=d {
            work.take(1)?;
            let mut x = if d == 0 {
                0
            } else {
                let prev = &trace[d - 1];
                if deletion(prev, d, i) {
                    prev[i - 1] + 1
                } else {
                    prev[i]
                }
            };
            let mut y = x + d - 2 * i;
            while x < a.len() && y < b.len() && equal(a[x].bytes, b[y].bytes, work)? {
                work.take(1)?;
                x += 1;
                y += 1;
            }
            frontier.push(x);
            if x == a.len() && y == b.len() {
                finished = true;
                break;
            }
        }
        push(&mut trace, frontier, work)?;
        if finished {
            let (mut x, mut y) = (a.len(), b.len());
            let mut matches = Vec::new();
            for d in (1..trace.len()).rev() {
                work.take(1)?;
                let i = (x + d - y) / 2;
                let prev = &trace[d - 1];
                let prev_i = if deletion(prev, d, i) { i - 1 } else { i };
                let prev_x = prev[prev_i];
                let prev_y = prev_x + d - 1 - 2 * prev_i;
                while x > prev_x && y > prev_y {
                    x -= 1;
                    y -= 1;
                    push(&mut matches, (x, y), work)?;
                }
                x = prev_x;
                y = prev_y;
            }
            // The d=0 snake starts at the origin.
            while x > 0 && y > 0 {
                x -= 1;
                y -= 1;
                push(&mut matches, (x, y), work)?;
            }
            work.take(matches.len())?;
            matches.reverse();
            return Ok(matches);
        }
    }
    unreachable!("Myers reaches the endpoint by deleting and inserting every line")
}

fn deletion(prev: &[usize], d: usize, i: usize) -> bool {
    i != 0 && (i == d || prev[i - 1] >= prev[i])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hunk<'a> {
    start: usize,
    end: usize,
    replacement: &'a [u8],
}

fn diff<'a>(
    base: &[Line<'_>],
    side: &[Line<'_>],
    side_bytes: &'a [u8],
    work: &mut Work,
) -> Limited<Vec<Hunk<'a>>> {
    let mut regions = Vec::new();
    push(&mut regions, (0, base.len(), 0, side.len()), work)?;
    let mut matches = Vec::new();
    while let Some((mut a0, mut a1, mut b0, mut b1)) = regions.pop() {
        work.take(1)?;
        while a0 < a1 && b0 < b1 && equal(base[a0].bytes, side[b0].bytes, work)? {
            push(&mut matches, (a0, b0), work)?;
            a0 += 1;
            b0 += 1;
        }
        while a0 < a1 && b0 < b1 && equal(base[a1 - 1].bytes, side[b1 - 1].bytes, work)? {
            a1 -= 1;
            b1 -= 1;
            push(&mut matches, (a1, b1), work)?;
        }
        if a0 == a1 || b0 == b1 {
            continue;
        }
        let selected = anchors(&base[a0..a1], &side[b0..b1], work)?;
        if selected.is_empty() {
            for (a, b) in myers(&base[a0..a1], &side[b0..b1], work)? {
                push(&mut matches, (a0 + a, b0 + b), work)?;
            }
        } else {
            let (mut prev_a, mut prev_b) = (a0, b0);
            for (a, b) in selected {
                let (a, b) = (a0 + a, b0 + b);
                push(&mut regions, (prev_a, a, prev_b, b), work)?;
                push(&mut matches, (a, b), work)?;
                prev_a = a + 1;
                prev_b = b + 1;
            }
            push(&mut regions, (prev_a, a1, prev_b, b1), work)?;
        }
    }
    sort(&mut matches, work, |p, q, work| {
        work.take(1)?;
        Ok(p.cmp(q))
    })?;
    let mut hunks = Vec::new();
    let (mut prev_a, mut prev_b) = (0, 0);
    for (a, b) in matches
        .into_iter()
        .chain(std::iter::once((base.len(), side.len())))
    {
        work.take(1)?;
        if a != prev_a || b != prev_b {
            push(
                &mut hunks,
                Hunk {
                    start: prev_a,
                    end: a,
                    replacement: &side_bytes
                        [offset(side, prev_b, side_bytes.len())..offset(side, b, side_bytes.len())],
                },
                work,
            )?;
        }
        prev_a = a + 1;
        prev_b = b + 1;
    }
    Ok(hunks)
}

fn offset(lines: &[Line<'_>], index: usize, byte_len: usize) -> usize {
    if index == lines.len() {
        byte_len
    } else {
        lines[index].offset
    }
}

fn overlaps(a: Hunk<'_>, b: Hunk<'_>) -> bool {
    if a.start == a.end {
        b.start <= a.start && a.start <= b.end
    } else if b.start == b.end {
        a.start <= b.start && b.start <= a.end
    } else {
        a.start < b.end && b.start < a.end
    }
}

fn append(out: &mut Vec<u8>, bytes: &[u8], max_bytes: usize, work: &mut Work) -> Limited<()> {
    if bytes.len() > max_bytes.saturating_sub(out.len()) {
        return Err(BYTE_LIMIT);
    }
    work.take(1 + bytes.len().div_ceil(64))?;
    out.try_reserve(bytes.len()).map_err(|_| ALLOCATION)?;
    out.extend_from_slice(bytes);
    Ok(())
}

fn merge_text(base: &[u8], left: &[u8], right: &[u8], budget: Budget) -> Limited<Option<Vec<u8>>> {
    let mut work = Work(budget.work);
    let base_lines = lines(base, &mut work)?;
    let left_lines = lines(left, &mut work)?;
    let right_lines = lines(right, &mut work)?;
    let left_hunks = diff(&base_lines, &left_lines, left, &mut work)?;
    let right_hunks = diff(&base_lines, &right_lines, right, &mut work)?;
    let (mut l, mut r, mut cursor) = (0, 0, 0);
    let mut result = Vec::new();
    while l < left_hunks.len() || r < right_hunks.len() {
        work.take(1)?;
        let next = match (left_hunks.get(l), right_hunks.get(r)) {
            (Some(&a), Some(&b)) => {
                if a.start == b.start
                    && a.end == b.end
                    && equal(a.replacement, b.replacement, &mut work)?
                {
                    l += 1;
                    r += 1;
                    a
                } else if overlaps(a, b) {
                    return Ok(None);
                } else if a.start < b.start {
                    l += 1;
                    a
                } else {
                    r += 1;
                    b
                }
            }
            (Some(&a), None) => {
                l += 1;
                a
            }
            (None, Some(&b)) => {
                r += 1;
                b
            }
            (None, None) => unreachable!("at least one hunk remains"),
        };
        append(
            &mut result,
            &base[offset(&base_lines, cursor, base.len())
                ..offset(&base_lines, next.start, base.len())],
            budget.max_bytes,
            &mut work,
        )?;
        append(&mut result, next.replacement, budget.max_bytes, &mut work)?;
        cursor = next.end;
    }
    append(
        &mut result,
        &base[offset(&base_lines, cursor, base.len())..],
        budget.max_bytes,
        &mut work,
    )?;
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(base: &[u8], left: &[u8], right: &[u8], expected: &[u8]) {
        for (l, r) in [(left, right), (right, left)] {
            assert_eq!(
                merge(base, l, r, Budget::default()),
                Outcome::Clean(expected.to_vec())
            );
        }
    }

    fn conflict(base: &[u8], left: &[u8], right: &[u8]) {
        for (l, r) in [(left, right), (right, left)] {
            assert_eq!(merge(base, l, r, Budget::default()), Outcome::Conflict);
        }
    }

    fn clean_bytes(outcome: Outcome) -> Vec<u8> {
        match outcome {
            Outcome::Clean(bytes) => bytes,
            other => panic!("expected a clean textual composition, got {other:?}"),
        }
    }

    #[test]
    fn exact_paths_precede_binary_and_resource_limits() {
        let none = Budget {
            work: 0,
            max_bytes: 0,
        };
        for (base, left, right, expected) in [
            (
                &b"\0base"[..],
                &b"\xffsame"[..],
                &b"\xffsame"[..],
                &b"\xffsame"[..],
            ),
            (
                &b"\0base"[..],
                &b"\0base"[..],
                &b"\xffright"[..],
                &b"\xffright"[..],
            ),
            (
                &b"\0base"[..],
                &b"\xffleft"[..],
                &b"\0base"[..],
                &b"\xffleft"[..],
            ),
            (&b""[..], &b""[..], &b""[..], &b""[..]),
        ] {
            assert_eq!(
                merge(base, left, right, none),
                Outcome::Clean(expected.to_vec())
            );
        }
        let oversized = vec![b'x'; TEXT_LIMIT + 1];
        assert_eq!(
            merge(b"base", &oversized, &oversized, none),
            Outcome::Clean(oversized)
        );
    }

    #[test]
    fn unequal_binary_or_oversized_inputs_are_opaque() {
        for bytes in [&b"a\0b"[..], &b"\xff"[..], &b"\xf0\x90"[..]] {
            conflict(b"base\n", bytes, b"right\n");
            conflict(bytes, b"left\n", b"right\n");
            assert_eq!(
                merge(
                    b"base\n",
                    bytes,
                    b"right\n",
                    Budget {
                        work: 0,
                        max_bytes: 0
                    }
                ),
                Outcome::Conflict
            );
        }
        let oversized = vec![b'a'; TEXT_LIMIT + 1];
        conflict(b"base", &oversized, b"right");
    }

    #[test]
    fn tokenizer_retains_terminators_and_no_phantom_final_line() {
        for (input, expected) in [
            (
                &b"a\r\nb\rc\nlast"[..],
                vec![&b"a\r\n"[..], &b"b\r"[..], &b"c\n"[..], &b"last"[..]],
            ),
            (
                &b"\r\n\r\n\n\r"[..],
                vec![&b"\r\n"[..], &b"\r\n"[..], &b"\n"[..], &b"\r"[..]],
            ),
            (&b"no newline"[..], vec![&b"no newline"[..]]),
            (&b""[..], vec![]),
        ] {
            let parsed = lines(input, &mut Work(10_000)).unwrap();
            assert_eq!(
                parsed.iter().map(|line| line.bytes).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                parsed
                    .iter()
                    .flat_map(|line| line.bytes.iter().copied())
                    .collect::<Vec<_>>(),
                input
            );
            let mut offset = 0;
            for line in parsed {
                assert_eq!(line.offset, offset);
                offset += line.bytes.len();
            }
        }
    }

    #[test]
    fn mixed_newlines_and_unterminated_tail_are_preserved() {
        clean(
            b"a\r\nb\rc\nlast",
            b"A\r\nb\rc\nlast",
            b"a\r\nb\rc\nLAST",
            b"A\r\nb\rc\nLAST",
        );
        clean(b"a\rb\rc\r", b"A\rb\rc\r", b"a\rb\rC\r", b"A\rb\rC\r");
        clean(
            b"a\r\nb\r\nc",
            b"A\r\nb\r\nc",
            b"a\r\nb\r\nc\r\n",
            b"A\r\nb\r\nc\r\n",
        );
        // Terminators are part of the exact line: changing a line's content on
        // one side and its terminator on the other is an overlapping edit.
        conflict(b"a\nb\n", b"A\nb\n", b"a\r\nb\n");
    }

    #[test]
    fn utf8_is_neither_normalized_nor_split_into_characters() {
        clean(
            "é\nkeep\nend\n".as_bytes(),
            "e\u{301}\nkeep\nend\n".as_bytes(),
            "é\nkeep\n終\n".as_bytes(),
            "e\u{301}\nkeep\n終\n".as_bytes(),
        );
    }

    #[test]
    fn independent_changes_compose_in_every_order() {
        let base = b"a\nb\nc\nd\ne\n";
        let sides: [&[u8]; 3] = [b"A\nb\nc\nd\ne\n", b"a\nB\nc\nd\ne\n", b"a\nb\nc\nd\nE\n"];
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let first = clean_bytes(merge(
                base,
                sides[order[0]],
                sides[order[1]],
                Budget::default(),
            ));
            clean(base, &first, sides[order[2]], b"A\nB\nc\nd\nE\n");
        }
    }

    #[test]
    fn identical_hunks_coalesce_alongside_distinct_changes() {
        clean(
            b"a\nb\nc\nd\ne\nf\ng\n",
            b"A\nb\nC\nd\ne\nf\ng\n",
            b"a\nb\nC\nd\ne\nf\nG\n",
            b"A\nb\nC\nd\ne\nf\nG\n",
        );
        clean(
            b"a\nb\nc\nd\n",
            b"x\na\nb\nC\nd\n",
            b"x\na\nb\nc\nD\n",
            b"x\na\nb\nC\nD\n",
        );
        clean(
            b"a\nb\nc\nd\ne\nf\ng\n",
            b"A\nb\nd\ne\nf\ng\n",
            b"a\nb\nd\ne\nf\nG\n",
            b"A\nb\nd\ne\nf\nG\n",
        );
    }

    #[test]
    fn distinct_gaps_compose_but_competing_insertions_do_not() {
        clean(
            b"a\nb\nc\n",
            b"x\na\nb\nc\n",
            b"a\nb\ny\nc\n",
            b"x\na\nb\ny\nc\n",
        );
        conflict(b"a\nb\n", b"a\nx\nb\n", b"a\ny\nb\n");
        conflict(b"", b"left\n", b"right\n");
    }

    #[test]
    fn insertion_attachment_is_conservative_at_both_boundaries() {
        for inserted in [&b"a\nx\nb\nc\n"[..], &b"a\nb\nx\nc\n"[..]] {
            conflict(b"a\nb\nc\n", b"a\nB\nc\n", inserted);
            conflict(b"a\nb\nc\n", b"a\nc\n", inserted);
        }
        conflict(b"a\nb\nc\nd\n", b"a\nD\n", b"a\nb\nx\nc\nd\n");
        clean(b"a\nb\nc\n", b"a\nB\nc\n", b"x\na\nb\nc\n", b"x\na\nB\nc\n");
    }

    #[test]
    fn unequal_overlap_and_edit_delete_conflict() {
        conflict(b"a\nb\nc\nd\n", b"a\nB\nc\nd\n", b"a\nOTHER\nc\nd\n");
        conflict(b"a\nb\nc\nd\n", b"a\nB\nc\nd\n", b"a\nc\nd\n");
        conflict(b"a\nb\nc\nd\n", b"a\nd\n", b"a\nb\n");
        // Adjacent replacement and deletion ranges do not overlap.
        clean(b"a\nb\nc\nd\n", b"a\nB\nc\nd\n", b"a\nb\nd\n", b"a\nB\nd\n");
    }

    #[test]
    fn repeated_lines_use_deterministic_gap_diff() {
        clean(
            b"x\nx\nx\nx\nx\n",
            b"A\nx\nx\nx\nx\n",
            b"x\nx\nx\nx\nB\n",
            b"A\nx\nx\nx\nB\n",
        );
        conflict(b"x\nx\nx\n", b"x\nA\nx\n", b"x\nB\nx\n");
        let a = lines(b"a\nb\n", &mut Work(1000)).unwrap();
        let b = lines(b"b\na\n", &mut Work(1000)).unwrap();
        assert_eq!(myers(&a, &b, &mut Work(1000)).unwrap(), vec![(1, 0)]);
    }

    #[test]
    fn unique_anchor_lis_has_a_fixed_tail_tie_break() {
        let a = lines(b"a\nb\nc\nd\nx\nx\n", &mut Work(1000)).unwrap();
        let b = lines(b"b\na\nd\nc\nx\nx\n", &mut Work(1000)).unwrap();
        assert_eq!(
            anchors(&a, &b, &mut Work(10_000)).unwrap(),
            vec![(1, 0), (3, 2)]
        );
        let b = lines(b"a\na\nb\nb\nc\nc\nd\nd\nx\n", &mut Work(1000)).unwrap();
        assert!(anchors(&a, &b, &mut Work(10_000)).unwrap().is_empty());
    }

    fn words(max_len: usize) -> Vec<Vec<u8>> {
        let mut words = Vec::new();
        for len in 0..=max_len {
            for bits in 0..(1 << len) {
                words.push(
                    (0..len)
                        .flat_map(|i| [b'a' + ((bits >> i) & 1) as u8, b'\n'])
                        .collect(),
                );
            }
        }
        words
    }

    // Independent dense LCS oracle is deliberately tiny and test-only. It
    // verifies Myers' shortest path, not a superseded merge/ambiguity rule.
    fn lcs(a: &[Line<'_>], b: &[Line<'_>]) -> usize {
        let mut table = vec![vec![0; b.len() + 1]; a.len() + 1];
        for x in 0..a.len() {
            for y in 0..b.len() {
                table[x + 1][y + 1] = if a[x].bytes == b[y].bytes {
                    table[x][y] + 1
                } else {
                    table[x][y + 1].max(table[x + 1][y])
                };
            }
        }
        table[a.len()][b.len()]
    }

    #[test]
    fn myers_matches_independent_shortest_path_oracle_on_small_repeats() {
        let words = words(5);
        for a in &words {
            for b in &words {
                let a = lines(a, &mut Work(1000)).unwrap();
                let b = lines(b, &mut Work(1000)).unwrap();
                let matches = myers(&a, &b, &mut Work(100_000)).unwrap();
                assert_eq!(matches.len(), lcs(&a, &b));
                for &(x, y) in &matches {
                    assert_eq!(a[x].bytes, b[y].bytes);
                }
                assert!(
                    matches
                        .windows(2)
                        .all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1)
                );
            }
        }
    }

    fn assert_diff_reconstructs(base: &[u8], side: &[u8]) {
        let mut work = Work(100_000);
        let base_lines = lines(base, &mut work).unwrap();
        let side_lines = lines(side, &mut work).unwrap();
        let hunks = diff(&base_lines, &side_lines, side, &mut work).unwrap();
        let mut rebuilt = Vec::new();
        let mut cursor = 0;
        for hunk in hunks {
            assert!(cursor <= hunk.start && hunk.start <= hunk.end && hunk.end <= base_lines.len());
            rebuilt.extend_from_slice(
                &base[offset(&base_lines, cursor, base.len())
                    ..offset(&base_lines, hunk.start, base.len())],
            );
            rebuilt.extend_from_slice(hunk.replacement);
            cursor = hunk.end;
        }
        rebuilt.extend_from_slice(&base[offset(&base_lines, cursor, base.len())..]);
        assert_eq!(rebuilt, side);
    }

    #[test]
    fn patience_hunks_reconstruct_every_small_input_pair() {
        let words = words(5);
        for base in &words {
            for side in &words {
                assert_diff_reconstructs(base, side);
            }
        }
    }

    #[test]
    fn patience_regions_reconstruct_mixed_unique_and_repeated_lines() {
        let mut state = 0x717e_87c3_05e4_9b2d_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as usize
        };
        let vocabulary = ["a\n", "b\r\n", "c\r", "d\n", "é\n", "終\r\n", "x\n", "y\r"];
        for _ in 0..512 {
            let mut texts = [Vec::new(), Vec::new()];
            for text in &mut texts {
                for _ in 0..next() % 24 {
                    text.extend_from_slice(vocabulary[next() % vocabulary.len()].as_bytes());
                }
                if next() % 2 == 0 {
                    text.extend_from_slice(b"unterminated");
                }
            }
            assert_diff_reconstructs(&texts[0], &texts[1]);
            assert_diff_reconstructs(&texts[1], &texts[0]);
        }
    }

    #[test]
    fn myers_trace_exhaustion_refuses_instead_of_selecting_a_coarse_diff() {
        let a_bytes = b"a\n".repeat(64);
        let b_bytes = b"b\n".repeat(64);
        let a = lines(&a_bytes, &mut Work(1000)).unwrap();
        let b = lines(&b_bytes, &mut Work(1000)).unwrap();
        for _ in 0..4 {
            assert_eq!(myers(&a, &b, &mut Work(1000)), Err(WORK_LIMIT));
        }
        assert_eq!(myers(&a, &b, &mut Work(100_000)).unwrap(), vec![]);
    }

    #[test]
    fn all_small_triples_are_side_symmetric_even_when_budget_refuses() {
        let words = words(3);
        for base in &words {
            for left in &words {
                for right in &words {
                    for work in [0, 100, 10_000] {
                        let budget = Budget {
                            work,
                            max_bytes: TEXT_LIMIT,
                        };
                        assert_eq!(
                            merge(base, left, right, budget),
                            merge(base, right, left, budget)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn work_limit_has_a_repeatable_exact_boundary() {
        let (base, left, right) = (
            &b"a\nb\nc\nd\ne\n"[..],
            &b"a\nB\nc\nd\ne\n"[..],
            &b"a\nb\nc\nD\ne\n"[..],
        );
        let expected = Outcome::Clean(b"a\nB\nc\nD\ne\n".to_vec());
        let run = |work| {
            merge(
                base,
                left,
                right,
                Budget {
                    work,
                    max_bytes: TEXT_LIMIT,
                },
            )
        };
        assert_eq!(run(0), Outcome::Pending(WORK_LIMIT));
        assert_eq!(run(100_000), expected);
        let (mut low, mut high) = (0, 100_000);
        while low < high {
            let mid = low + (high - low) / 2;
            match run(mid) {
                Outcome::Pending(WORK_LIMIT) => low = mid + 1,
                Outcome::Clean(_) => high = mid,
                other => panic!("unexpected boundary result: {other:?}"),
            }
        }
        for _ in 0..4 {
            assert_eq!(run(low - 1), Outcome::Pending(WORK_LIMIT));
            assert_eq!(run(low), expected);
        }
        let noisy_base = b"a\na\na\na\na\na\n";
        assert_eq!(
            merge(
                noisy_base,
                b"b\nb\nb\nb\nb\nb\n",
                b"c\nc\nc\nc\nc\nc\n",
                Budget {
                    work: 200,
                    max_bytes: TEXT_LIMIT
                }
            ),
            Outcome::Pending(WORK_LIMIT)
        );
    }

    #[test]
    fn configured_input_and_output_limits_return_pending_not_conflict() {
        let base = b"a\nb\n";
        let left = b"x\na\nb\n";
        let right = b"a\nb\ny\n";
        for max_bytes in [0, 5, 6, 7] {
            assert_eq!(
                merge(
                    base,
                    left,
                    right,
                    Budget {
                        max_bytes,
                        ..Budget::default()
                    }
                ),
                Outcome::Pending(BYTE_LIMIT)
            );
        }
        assert_eq!(
            merge(
                base,
                left,
                right,
                Budget {
                    max_bytes: 8,
                    ..Budget::default()
                }
            ),
            Outcome::Clean(b"x\na\nb\ny\n".to_vec())
        );
        // Configuring a larger resource ceiling does not relax text eligibility.
        let oversized = vec![b'a'; TEXT_LIMIT + 1];
        assert_eq!(
            merge(
                b"base",
                &oversized,
                b"right",
                Budget {
                    max_bytes: usize::MAX,
                    ..Budget::default()
                }
            ),
            Outcome::Conflict
        );
    }
}
