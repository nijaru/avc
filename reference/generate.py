#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["blake3==1.0.8"]
# ///
"""Deterministically regenerate independent vectors; no external implementation inputs."""

import json
from pathlib import Path

from check import (
    Store,
    directory,
    encode_node,
    file,
    identify,
    packed,
    spelling,
    symlink,
    uint,
)


def generate():
    s = Store()
    cases = []
    roots = []

    def case(name, root):
        cases.append({"name": name, "root": spelling(root)})
        if root not in roots:
            roots.append(root)
        return root

    empty = case("empty-map", s.map([]))
    raw = case("blob-exact-nul-crlf-nonutf8", s.add(1, b"a\0\r\n\xff"))
    a = case("blob-a", s.add(1, b"A\n"))
    b = case("blob-b", s.add(1, b"B\n"))
    c = case("blob-c", s.add(1, b"C\n"))
    fa, fb, fc = file(a), file(b), file(c)
    one = case("one-file", s.map([(b"file", fa)]))
    nested_map = case(
        "nested-file-map",
        s.map([(b"dir", directory(one)), (b"empty", directory(empty))]),
    )
    target = s.add(1, b"../opaque\0target")  # symlink payload is uninterpreted bytes
    case(
        "symlink-and-executable",
        s.map([(b"link", symlink(target)), (b"run", file(raw, True))]),
    )
    names = [(f"k{i:02}".encode(), fa) for i in range(33)]
    small = case("threshold-32", s.map(names[:32]))
    wide = case("threshold-33-split", s.map(names))
    case("collapse-back-to-32", s.map(list(reversed(names[:32]))))
    assert roots.count(small) == 1 and wide != small
    case("long-name-255", s.map([(b"z" * 255, fa)]))
    case(
        "long-common-prefix-254",
        s.map([(b"z" * 254 + bytes([i + 1]), fa) for i in range(33)]),
    )
    case(
        "prefix-key-terminal-odd-child-suffix",
        s.map([(b"a", fa)] + [(b"a" + bytes([i + 1]), fa) for i in range(32)]),
    )
    case(
        "odd-nibble-common-prefix",
        s.map([(bytes([0x40 + i // 16, 0x40 + i % 16]), fa) for i in range(33)]),
    )
    # All names share high nibble 4; some children consume a whole-byte prefix.
    case(
        "raw-byte-name-order",
        s.map(
            [
                (name, fa)
                for name in (b"\xff", b"\x80", b"a", b"A", b"e\xcc\x81", b"\xc3\xa9")
            ]
        ),
    )
    signed = s.signed([fa, fb], [fc])
    case("flat-signed-file-conflict", s.map([(b"f", signed)]))
    repeated = s.signed([fa, fa], [fb])
    case("signed-multiplicity-not-same-change", s.map([(b"f", repeated)]))
    typed = s.signed([directory(one), symlink(target), b"\0"], [file(a, True), fb])
    case("typed-absence-mode-symlink-directory-conflict", s.map([(b"f", typed)]))
    nested = s.signed([signed, fc], [fb])
    assert nested == fa
    case("flatten-exact-cancellation", s.map([(b"f", nested)]))
    root_conflict = s.signed(
        [directory(empty), directory(one)], [directory(nested_map)], region=1
    )
    root_ref = case("root-directory-conflict", root_conflict[1:])
    r0 = case(
        "revision-zero-parent", s.authored(empty, timestamp=-(1 << 63), timezone=-1439)
    )
    r1 = case(
        "revision-parent-and-root-conflict",
        s.authored(
            root_ref,
            [r0],
            message=b"conflict\r\n",
            timestamp=(1 << 63) - 1,
            timezone=1439,
        ),
    )
    case(
        "revision-two-ordered-parents",
        s.authored(one, [r1, r0], author="\u00e9 <x@y>".encode()),
    )

    corpus = s.corpus(roots)
    corpus["cases"] = cases
    negatives = []

    def bad(name, kind, payload, dependencies=()):
        base = s.corpus(dependencies)
        ref = identify(kind, payload)
        base["objects"].append({"id": spelling(ref), "payload": payload.hex()})
        base["roots"] = [spelling(ref)] + base["roots"]
        negatives.append({"name": name, **base})

    bad("trailing-leaf-byte", 2, b"\0\0")
    bad("leaf-count-over-32", 2, b"\x21")
    bad("huge-nibble-count", 2, b"\x01\xff\xff")
    bad("truncated-nibbles", 2, b"\x01\xfe\x01")
    bad("nonzero-nibble-padding", 2, b"\x01\x01\0\x61" + fa, [a])
    bad("odd-recovered-key", 2, b"\x01" + packed((6,)) + fa, [a])
    bad("empty-name", 2, b"\x01\0\0" + fa, [a])
    for name in (b"\0", b"/", b".", b".."):
        bad(
            "forbidden-name-" + name.hex(),
            2,
            b"\x01" + packed(tuple(n for v in name for n in (v >> 4, v & 15))) + fa,
            [a],
        )
    bad("absent-map-binding", 2, b"\x01" + packed((6, 1)) + b"\0")
    bad("unknown-entry-tag", 2, b"\x01" + packed((6, 1)) + b"\xff")
    bad("nonboolean-file-mode", 2, b"\x01" + packed((6, 1)) + fa[:-1] + b"\x02", [a])
    bad(
        "file-wrong-ref-kind",
        2,
        b"\x01" + packed((6, 1)) + b"\x01" + empty + b"\0",
        [empty],
    )
    bad("duplicate-leaf-keys", 2, b"\x02" + (packed((6, 1)) + fa) * 2, [a])
    bad(
        "unsorted-leaf-keys",
        2,
        b"\x02" + packed((6, 2)) + fa + packed((6, 1)) + fa,
        [a],
    )
    count, prefix, terminal, children = s.node(wide)
    bad(
        "incorrect-branch-count",
        3,
        encode_node(3, (count + 1, prefix, terminal, children)),
        [wide],
    )
    bad(
        "branch-below-threshold",
        3,
        encode_node(3, (32, prefix, terminal, children)),
        [wide],
    )
    bad("invalid-terminal-flag", 3, uint(33, 4) + packed(()) + b"\x02")
    bad(
        "redundant-one-child",
        3,
        uint(33, 4) + packed(()) + b"\0" + uint(1, 2) + empty,
        [empty],
    )
    bad(
        "empty-child",
        3,
        uint(33, 4) + packed(()) + b"\0" + uint(3, 2) + empty * 2,
        [empty],
    )
    # A syntactically valid 33-entry child under one nibble cannot be used as a
    # full root: the total recovered keys become odd; context must not be cached.
    bad(
        "map-parent-context-odd-length",
        3,
        encode_node(3, (34, (), fa, ((1, wide),))),
        [wide],
    )
    # Nonmaximal prefix with a terminal-free one-child chain fails even before
    # gathering names. A two-outcome branch may not claim a longer prefix either.
    bad(
        "overlong-prefix-recovered-name",
        3,
        encode_node(3, (33, (1,) * 510, None, children)),
        [wide],
    )
    bad("odd-terminal-boundary", 3, encode_node(3, (33, (6,), fa, children)), [wide])
    base = s.corpus([one])
    base["objects"] = [dict(row) for row in base["objects"]]
    base["objects"][0]["payload"] += "00"
    negatives.append({"name": "wrong-hash", **base})
    base = s.corpus([one])
    base["objects"] = [row for row in base["objects"] if row["id"] != spelling(a)]
    negatives.append({"name": "missing-owning-blob", **base})

    def bad_conflict(name, node):
        payload = encode_node(4, node)
        bad(name, 4, payload, [a, b, c, empty, one, root_ref])

    bad_conflict("unnecessary-conflict", (1, ((directory(empty), 1),), ()))
    bad_conflict(
        "opposite-sign-not-cancelled",
        (1, ((directory(empty), 2),), ((directory(empty), 1),)),
    )
    bad_conflict("unbalanced-conflict", (1, ((directory(empty), 2),), ()))
    bad_conflict("zero-multiplicity", (1, ((directory(empty), 0),), ()))
    bad_conflict("root-file-conflict", (1, ((fa, 2),), ((fb, 1),)))
    bad_conflict(
        "root-conflict-positive-absence",
        (1, ((b"\0", 1), (directory(one), 1)), ((directory(empty), 1),)),
    )
    bad_conflict(
        "root-conflict-negative-absence",
        (
            1,
            tuple((t, 1) for t in sorted((directory(empty), directory(one)))),
            ((b"\0", 1),),
        ),
    )
    bad_conflict(
        "path-conflict-as-source-root",
        (0, tuple(sorted(((fa, 1), (fb, 1)))), ((fc, 1),)),
    )
    bad_conflict(
        "same-sign-duplicate",
        (1, ((directory(empty), 1), (directory(empty), 1)), ((directory(one), 1),)),
    )
    ordered = sorted((directory(empty), directory(one)))
    bad_conflict(
        "unsorted-conflict",
        (1, ((ordered[1], 1), (ordered[0], 1)), ((directory(nested_map), 1),)),
    )
    bad_conflict(
        "multiplicity-bound", (1, ((directory(empty), 33),), ((directory(one), 32),))
    )
    bad_conflict(
        "nested-conflict-term", (1, ((root_conflict, 2),), ((directory(empty), 1),))
    )
    bad("unknown-conflict-region", 4, b"\x02\0\0")
    bad("huge-conflict-distinct-count", 4, b"\x01\xff")

    rev = s.node(r0)
    bad(
        "revision-invalid-utf8",
        5,
        encode_node(5, (*rev[:3], b"\xff", *rev[4:])),
        [empty],
    )
    bad("revision-timezone-range", 5, encode_node(5, (*rev[:-1], 1440)), [empty])
    bad(
        "revision-duplicate-parent",
        5,
        encode_node(5, (empty, (r0, r0), *rev[2:])),
        [r0],
    )
    bad(
        "revision-author-length-cap",
        5,
        empty + b"\0" + bytes(16) + uint(4097, 4),
        [empty],
    )
    bad(
        "revision-message-length-cap",
        5,
        empty + b"\0" + bytes(16) + uint(0, 4) + uint(1048577, 4),
        [empty],
    )
    bad("revision-too-many-parents", 5, empty + b"\x11", [empty])
    # Wrong typed roots are rejected at the revision boundary, not disguised as
    # a source map. Blob is otherwise a valid codec-corpus root.
    bad("revision-blob-source-root", 5, encode_node(5, (a, *rev[1:])), [a])
    base = s.corpus([empty])
    base["roots"][0] = base["roots"][0].upper()
    negatives.append({"name": "uppercase-id", **base})
    base = s.corpus([empty])
    base["objects"].append(dict(base["objects"][0]))
    negatives.append({"name": "duplicate-object-record", **base})
    corpus["negative"] = negatives
    return corpus


if __name__ == "__main__":
    output = Path(__file__).with_name("vectors.json")
    output.write_text(json.dumps(generate(), indent=2) + "\n")
    print(output)
