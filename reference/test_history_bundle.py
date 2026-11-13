"""Independent AVCHIS01/02 framing, native closure, and offline Git origins."""

import hashlib
import io
import json
import struct
import subprocess
import sys
import unittest
from dataclasses import replace
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from blake3 import blake3
from bundle import (
    HISTORY_LIMITS,
    BundleLimits,
    encode_history,
    verify_history_file,
    verify_history_stream,
    verify_stream,
)
from check import (
    Invalid,
    Limit,
    Store,
    directory,
    file,
    identify,
    parse_id,
    spelling,
    symlink,
)
from test_bundle import ReadProbe, finish
from test_bundle import build as build_source


def build(
    root,
    records,
    head=None,
    *,
    tag=None,
    count=None,
    end=None,
    origins=None,
    origin_count=None,
):
    # No production framing helpers: fields and byte order are explicit here.
    v2 = origins is not None
    body = (b"AVCHIS02" if v2 else b"AVCHIS01") + root
    body += bytes([int(head is not None) if tag is None else tag])
    if head is not None:
        body += head
    body += struct.pack("<Q", len(records) if count is None else count)
    for ref, payload in records:
        body += ref + struct.pack("<Q", len(payload)) + payload
    if v2:
        body += struct.pack(
            "<Q", len(origins) if origin_count is None else origin_count
        )
        for ref, oid, raw in origins:
            body += ref + oid + struct.pack("<Q", len(raw)) + raw
    return finish(
        body + (end if end is not None else b"AVCHEND2" if v2 else b"AVCHEND1")
    )


def records(corpus):
    return [
        (parse_id(row["id"]), bytes.fromhex(row["payload"]))
        for row in corpus["objects"]
    ]


def revision(store, source, parents=()):
    # Independently laid out canonical v1 Revision, not Store.authored/encode_node.
    payload = (
        source
        + bytes([len(parents)])
        + b"".join(parents)
        + bytes(range(16))
        + b"\x01\0\0\0A"
        + b"\x02\0\0\0m\n"
        + struct.pack("<qh", -123, 60)
    )
    return store.add(5, payload)


class HistoryBundleTests(unittest.TestCase):
    def setUp(self):
        self.store = Store()
        self.empty = self.store.map([])
        self.blob = self.store.add(1, b"authored\0\r\n")
        self.authored_source = self.store.map([(b"f", file(self.blob))])
        self.parent = revision(self.store, self.empty)
        self.head = revision(self.store, self.authored_source, (self.parent,))
        self.selected_blob = self.store.add(1, b"uncommitted")
        self.selected = self.store.map([(b"working", file(self.selected_blob))])
        self.corpus = self.store.corpus([self.selected, self.head])
        self.records = records(self.corpus)
        self.wire = build(self.selected, self.records, self.head)

    def test_two_revisions_and_distinct_selected_source(self):
        self.assertNotEqual(self.selected, self.authored_source)
        self.assertEqual(verify_history_stream(io.BytesIO(self.wire)), self.corpus)
        self.assertEqual(
            self.corpus["roots"], [spelling(self.selected), spelling(self.head)]
        )
        self.assertEqual(
            {ref for ref, _ in self.records},
            {
                self.selected,
                self.selected_blob,
                self.head,
                self.parent,
                self.authored_source,
                self.blob,
                self.empty,
            },
        )
        probe = ReadProbe(self.wire, fragment=3)
        self.assertEqual(verify_history_stream(probe), self.corpus)
        self.assertEqual(probe.tell(), len(self.wire))

    def test_unborn_empty_source_literal_and_exact_file_budget(self):
        root = identify(2, b"\0")
        body = (
            b"AVCHIS01"
            + root
            + b"\0"
            + b"\x01"
            + bytes(7)
            + root
            + b"\x01"
            + bytes(7)
            + b"\0AVCHEND1"
        )
        wire = finish(body)
        expected = Store({root: b"\0"}).corpus([root])
        limits = replace(HISTORY_LIMITS, object_bytes=1, file_bytes=len(wire))
        self.assertEqual(verify_history_stream(io.BytesIO(wire), limits), expected)
        with self.assertRaisesRegex(Limit, "whole-file"):
            verify_history_stream(
                io.BytesIO(wire), replace(limits, file_bytes=len(wire) - 1)
            )
        self.assertEqual(
            HISTORY_LIMITS.file_bytes, BundleLimits.file_bytes + 34 + 8 + 62 * 100_000
        )
        # A present head really consumes the maximum 34-byte header addition.
        source_wire = build_source(self.selected, self.records)
        self.assertEqual(len(self.wire), len(source_wire) + 34)

    def test_unborn_nonempty_source_and_head_source_selection(self):
        for root, head in ((self.selected, None), (self.authored_source, self.head)):
            with self.subTest(head=head):
                corpus = self.store.corpus([root] + ([] if head is None else [head]))
                self.assertEqual(
                    verify_history_stream(
                        io.BytesIO(build(root, records(corpus), head))
                    ),
                    corpus,
                )

    def test_explicit_format_selection_no_autodetection(self):
        source_corpus = self.store.corpus([self.selected])
        source_wire = build_source(self.selected, records(source_corpus))
        self.assertEqual(verify_stream(io.BytesIO(source_wire)), source_corpus)
        for verify, wire in (
            (verify_stream, self.wire),
            (verify_history_stream, source_wire),
        ):
            with (
                self.subTest(verify=verify.__name__),
                self.assertRaisesRegex(Invalid, "magic/version"),
            ):
                verify(io.BytesIO(wire))

    def test_head_tags_and_kinds_rejected_at_header(self):
        for tag in (2, 127, 255):
            probe = ReadProbe(b"AVCHIS01" + self.selected + bytes([tag]))
            with (
                self.subTest(tag=tag),
                self.assertRaisesRegex(Invalid, "authored-head tag"),
            ):
                verify_history_stream(probe)
            self.assertEqual(probe.tell(), 42)
        for kind in (0, 1, 2, 3, 4, 6, 255):
            probe = ReadProbe(
                b"AVCHIS01" + self.selected + b"\1" + bytes([kind]) + bytes(32)
            )
            with (
                self.subTest(kind=kind),
                self.assertRaisesRegex(Invalid, "head must be a Revision"),
            ):
                verify_history_stream(probe)
            self.assertEqual(probe.tell(), 75)

    def test_blob_revision_and_unknown_kind_not_selected_source(self):
        for root in (self.blob, self.head, b"\x06" + bytes(32)):
            with (
                self.subTest(root=root),
                self.assertRaisesRegex(Invalid, "root must be a source"),
            ):
                verify_history_stream(io.BytesIO(build(root, self.records, self.head)))

    def test_root_conflict_owns_both_signs_with_history(self):
        root = self.store.signed(
            [directory(self.selected), directory(self.authored_source)],
            [directory(self.empty)],
            region=1,
        )[1:]
        corpus = self.store.corpus([root, self.head])
        self.assertEqual(
            verify_history_stream(io.BytesIO(build(root, records(corpus), self.head))),
            corpus,
        )
        # The negative term's source is required even without any Revision owning it.
        corpus = self.store.corpus([root])
        missing_negative = [
            (ref, payload) for ref, payload in records(corpus) if ref != self.empty
        ]
        with self.assertRaisesRegex(Invalid, "missing owning"):
            verify_history_stream(io.BytesIO(build(root, missing_negative)))

    def test_path_conflict_rejected_in_selected_and_revision_source_contexts(self):
        path = self.store.signed([file(self.blob), b"\0"], [directory(self.empty)])[1:]
        head = revision(self.store, path)
        for roots in ([path], [self.selected, head]):
            corpus = self.store.corpus(roots)
            with (
                self.subTest(roots=roots),
                self.assertRaisesRegex(Invalid, "region/context"),
            ):
                verify_history_stream(
                    io.BytesIO(
                        build(
                            roots[0], records(corpus), None if len(roots) == 1 else head
                        )
                    )
                )

    def test_negative_absence_rejected_in_selected_and_revision_source_contexts(self):
        positives = sorted([directory(self.selected), directory(self.empty)])
        payload = b"\1\2" + b"".join(b"\1" + term for term in positives) + b"\1\1\0"
        root = self.store.add(4, payload)
        head = revision(self.store, root)
        support = records(self.store.corpus([self.selected, self.empty]))
        for selected, authored in ((root, None), (self.selected, head)):
            rows = [*support, (root, payload)]
            if authored is not None:
                rows.append((head, self.store.objects[head]))
            with (
                self.subTest(selected=selected),
                self.assertRaisesRegex(Invalid, "non-directory root conflict"),
            ):
                verify_history_stream(
                    io.BytesIO(build(selected, sorted(rows), authored))
                )

    def test_missing_selected_head_ancestor_and_source_dependencies(self):
        # Every missing object is tested with an otherwise valid, repaired checksum.
        for missing, _ in self.records:
            remaining = [
                (ref, payload) for ref, payload in self.records if ref != missing
            ]
            with (
                self.subTest(missing=spelling(missing)),
                self.assertRaisesRegex(Invalid, "missing owning"),
            ):
                verify_history_stream(
                    io.BytesIO(build(self.selected, remaining, self.head))
                )

    def test_surplus_revision_and_source_records_are_not_transferred(self):
        extra_head = revision(self.store, self.empty, (self.parent,))
        extra_blob = self.store.add(1, b"not selected or authored")
        extra_source = self.store.map([(b"extra", file(extra_blob))])
        for extra in (extra_head, extra_source):
            extra_records = dict(self.records)
            extra_records.update(records(self.store.corpus([extra])))
            with self.subTest(extra=extra), self.assertRaisesRegex(Invalid, "unrooted"):
                verify_history_stream(
                    io.BytesIO(
                        build(self.selected, sorted(extra_records.items()), self.head)
                    )
                )
        # Removing the head header cannot turn its records into source-only closure.
        with self.assertRaisesRegex(Invalid, "unrooted"):
            verify_history_stream(io.BytesIO(build(self.selected, self.records)))

    def test_shared_closure_charged_once_and_duplicate_or_unsorted_records_refused(
        self,
    ):
        head = revision(self.store, self.authored_source, (self.head,))
        corpus = self.store.corpus([self.authored_source, head])
        rows = records(corpus)
        limits = replace(
            HISTORY_LIMITS, closure_bytes=sum(len(payload) for _, payload in rows)
        )
        self.assertEqual(
            verify_history_stream(
                io.BytesIO(build(self.authored_source, rows, head)), limits
            ),
            corpus,
        )
        for rows in (list(reversed(self.records)), [self.records[0], *self.records]):
            with (
                self.subTest(rows=len(rows)),
                self.assertRaisesRegex(Invalid, "unique and sorted"),
            ):
                verify_history_stream(io.BytesIO(build(self.selected, rows, self.head)))

    def test_object_hash_and_canonical_payload_checked_after_outer_checksum(self):
        changed = [
            (ref, payload + b"x" if ref == self.head else payload)
            for ref, payload in self.records
        ]
        with self.assertRaisesRegex(Invalid, "wrong object hash"):
            verify_history_stream(io.BytesIO(build(self.selected, changed, self.head)))
        payload = (
            b"\0\0"  # Rehashing cannot authorize trailing canonical payload bytes.
        )
        root = identify(2, payload)
        with self.assertRaisesRegex(Invalid, "trailing bytes"):
            verify_history_stream(io.BytesIO(build(root, [(root, payload)])))

    def test_every_truncated_prefix_including_optional_head_and_completion(self):
        for length in range(len(self.wire)):
            with self.subTest(length=length), self.assertRaises(Invalid):
                verify_history_stream(io.BytesIO(self.wire[:length]))

    def test_magic_completion_checksum_and_eof(self):
        for wire, message in (
            (finish(b"AVCHIS03" + self.wire[8:-32]), "magic/version"),
            (
                build(self.selected, self.records, self.head, end=b"AVCHEND2"),
                "completion marker",
            ),
            (self.wire[:-32] + blake3(self.wire[:-40]).digest(), "checksum"),
            (self.wire[:-1] + bytes([self.wire[-1] ^ 1]), "checksum"),
            (self.wire + b"\0", "trailing bundle"),
            (self.wire * 2, "trailing bundle"),
        ):
            probe = ReadProbe(wire)
            with (
                self.subTest(message=message),
                self.assertRaisesRegex(Invalid, message),
            ):
                verify_history_stream(probe)
            if message == "trailing bundle":
                self.assertEqual(probe.tell(), len(self.wire) + 1)

    def test_record_counts_and_wire_caps_before_record_reads(self):
        for count in (0, 1, len(self.records) + 1):
            with self.subTest(count=count), self.assertRaises(Invalid):
                verify_history_stream(
                    io.BytesIO(
                        build(self.selected, self.records, self.head, count=count)
                    )
                )
        for count in (100_001, (1 << 64) - 1):
            probe = ReadProbe(
                b"AVCHIS01"
                + self.selected
                + b"\1"
                + self.head
                + struct.pack("<Q", count)
            )
            with (
                self.subTest(count=count),
                self.assertRaisesRegex(Limit, "object-count"),
            ):
                verify_history_stream(probe, replace(HISTORY_LIMITS, objects=count))
            self.assertEqual(probe.tell(), 83)
        with self.assertRaisesRegex(Limit, "object-count"):
            verify_history_stream(
                io.BytesIO(self.wire),
                replace(HISTORY_LIMITS, objects=len(self.records) - 1),
            )

    def test_payload_lengths_and_file_budget_before_payload_read(self):
        header = (
            b"AVCHIS01"
            + self.selected
            + b"\1"
            + self.head
            + struct.pack("<Q", 1)
            + self.selected
        )
        for length in (16 * 1024 * 1024 + 1, (1 << 64) - 1):
            probe = ReadProbe(header + struct.pack("<Q", length))
            with (
                self.subTest(length=length),
                self.assertRaisesRegex(Limit, "object-payload"),
            ):
                verify_history_stream(
                    probe, replace(HISTORY_LIMITS, object_bytes=length)
                )
            self.assertEqual(probe.tell(), len(header) + 8)
            self.assertEqual(max(probe.requests), 33)
        for limits, message in (
            (replace(HISTORY_LIMITS, object_bytes=99), "object-payload"),
            (replace(HISTORY_LIMITS, closure_bytes=99), "closure-payload"),
            (replace(HISTORY_LIMITS, file_bytes=len(header) + 8 + 99), "whole-file"),
        ):
            probe = ReadProbe(header + struct.pack("<Q", 100))
            with self.subTest(message=message), self.assertRaisesRegex(Limit, message):
                verify_history_stream(probe, limits)
            self.assertEqual(probe.tell(), len(header) + 8)

    def test_aggregate_budget_accumulates_unique_records_before_last_payload(self):
        total = sum(len(payload) for _, payload in self.records)
        probe = ReadProbe(self.wire)
        with self.assertRaisesRegex(Limit, "closure-payload"):
            verify_history_stream(
                probe, replace(HISTORY_LIMITS, closure_bytes=total - 1)
            )
        self.assertEqual(
            probe.tell(), 83 + 41 * len(self.records) + total - len(self.records[-1][1])
        )

    def test_large_payload_bounded_reads(self):
        blob = self.store.add(1, b"x" * (2 * 64 * 1024 + 7))
        root = self.store.map([(b"large", file(blob))])
        corpus = self.store.corpus([root, self.head])
        probe = ReadProbe(build(root, records(corpus), self.head))
        self.assertEqual(verify_history_stream(probe), corpus)
        self.assertEqual(max(probe.requests), 64 * 1024)

    def test_file_and_cli_json_usable_by_checker_without_local_metadata(self):
        command = [sys.executable, str(Path(__file__).with_name("bundle.py"))]
        with TemporaryDirectory() as scratch:
            path = Path(scratch) / "history.avchis"
            path.write_bytes(self.wire)
            self.assertEqual(verify_history_file(path), self.corpus)
            plain = subprocess.run(
                [*command, str(path), "--history"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(plain.returncode, 0, plain.stderr)
            self.assertIn(f"verified {len(self.records)} objects", plain.stdout)
            result = subprocess.run(
                [*command, str(path), "--history", "--json"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            corpus = json.loads(result.stdout)
            self.assertEqual(corpus, self.corpus)
            self.assertEqual(set(corpus), {"objects", "roots"})
            corpus_path = Path(scratch) / "corpus.json"
            corpus_path.write_text(result.stdout)
            checked = subprocess.run(
                [
                    sys.executable,
                    str(Path(__file__).with_name("check.py")),
                    "--verify",
                    str(corpus_path),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(checked.returncode, 0, checked.stderr)
            for flags in ([], ["--history", "--max-file-bytes", "83"]):
                refused = subprocess.run(
                    [*command, str(path), "--json", *flags],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertEqual(refused.returncode, 1)
                self.assertEqual(refused.stdout, "")
                self.assertIn("refused:", refused.stderr)
            path.write_bytes(self.wire + b"local metadata is not a wire field")
            refused = subprocess.run(
                [*command, str(path), "--history", "--json"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(refused.returncode, 1)
            self.assertEqual(refused.stdout, "")
            self.assertIn("trailing bundle", refused.stderr)


def foreign_hash(kind, raw):
    # Direct Git framing, independent of bundle.git_hash.
    return hashlib.sha1(b"%s %d\0" % (kind, len(raw)) + raw).digest()


def raw_commit(
    tree,
    parents=(),
    *,
    author=b"A <a@test> 7 +0130",
    committer=b"C <c@test> 11 -0230",
    message=b"original\n",
    extra=b"",
):
    return (
        b"tree "
        + tree.hex().encode()
        + b"".join(b"\nparent " + parent.hex().encode() for parent in parents)
        + b"\nauthor "
        + author
        + b"\ncommitter "
        + committer
        + extra
        + b"\n\n"
        + message
    )


def anchor(
    store,
    source,
    raw,
    *,
    parents=(),
    author=b"A <a@test>",
    message=b"original\n",
    timestamp=11_000_000_000,
    timezone=-150,
    change=None,
):
    oid = foreign_hash(b"commit", raw)
    change = (
        blake3(b"AVC-GIT-ANCHOR\0v1\0\x01\x14" + oid).digest()[:16]
        if change is None
        else change
    )
    payload = (
        source
        + bytes([len(parents)])
        + b"".join(parents)
        + change
        + struct.pack("<I", len(author))
        + author
        + struct.pack("<I", len(message))
        + message
        + struct.pack("<qh", timestamp, timezone)
    )
    ref = store.add(5, payload)
    return ref, b"\1" + oid, raw


def origin_json(origin):
    ref, oid, raw = origin
    return {
        "revision": spelling(ref),
        "commit": "sha1:" + oid[1:].hex(),
        "raw_commit": raw.hex(),
    }


class GitHistoryBundleTests(unittest.TestCase):
    def setUp(self):
        self.store = Store()
        self.empty = self.store.map([])
        # A well-known Git empty tree pin, not produced by the verifier.
        self.empty_tree = bytes.fromhex("4b825dc642cb6eb9a060e54bf8d69288fbee4904")
        self.raw = raw_commit(self.empty_tree, (b"p" * 20,))
        self.origin = anchor(self.store, self.empty, self.raw)
        self.head = self.origin[0]
        self.selected = self.store.map(
            [(b"uncommitted", file(self.store.add(1, b"new")))]
        )

    def wire(self, origins=None, head=None, selected=None, rows=None):
        head = self.head if head is None else head
        selected = self.selected if selected is None else selected
        corpus = self.store.corpus([selected, head])
        return build(
            selected,
            records(corpus) if rows is None else rows,
            head,
            origins=[self.origin] if origins is None else origins,
        )

    def test_empty_anchor_foreign_parents_and_native_descendant(self):
        self.assertEqual(
            self.origin[1].hex(), "0105f9b000c5fe32dad17e2d90012101ede1d68cde"
        )
        self.assertEqual(
            spelling(self.head),
            "avc1-k05-7476cbde3192fb74c7d7fa1b2745544b5c501655e99d0a3f6ee58cced843f52d",
        )
        head = revision(self.store, self.selected, (self.head,))
        wire = self.wire(head=head)
        expected = self.store.corpus([self.selected, head])
        expected["origins"] = [origin_json(self.origin)]
        self.assertEqual(verify_history_stream(io.BytesIO(wire)), expected)
        probe = ReadProbe(wire, fragment=2)
        self.assertEqual(verify_history_stream(probe), expected)
        self.assertEqual(probe.tell(), len(wire))
        self.assertEqual(encode_history(expected), wire)
        # Opaque foreign parent has neither a native object nor an origin record.
        self.assertNotIn(b"p" * 20, self.store.objects)

    def test_all_modes_raw_names_git_order_and_nested_empty_directories(self):
        blob_raw, link_raw = b"data\0\xff\r\n", b"../target\xff"
        blob = self.store.add(1, blob_raw)
        link = self.store.add(1, link_raw)
        source = self.store.map(
            [
                (b"a", directory(self.empty)),
                (b"a.", file(blob)),
                (b"a0", file(blob, True)),
                (b"\xff\xfe", symlink(link)),
            ]
        )
        blob_oid = foreign_hash(b"blob", blob_raw)
        link_oid = foreign_hash(b"blob", link_raw)
        # Git compares a directory as name+'/', so a. sorts BEFORE directory a.
        tree_raw = (
            b"100644 a.\0"
            + blob_oid
            + b"40000 a\0"
            + self.empty_tree
            + b"100755 a0\0"
            + blob_oid
            + b"120000 \xff\xfe\0"
            + link_oid
        )
        tree = foreign_hash(b"tree", tree_raw)
        origin = anchor(self.store, source, raw_commit(tree))
        wire = self.wire([origin], head=origin[0])
        self.assertEqual(
            verify_history_stream(io.BytesIO(wire))["origins"], [origin_json(origin)]
        )
        # Omitting the empty directory (the export projection's restriction) is wrong.
        wrong_tree = foreign_hash(
            b"tree", tree_raw.replace(b"40000 a\0" + self.empty_tree, b"")
        )
        bad = anchor(self.store, source, raw_commit(wrong_tree))
        with self.assertRaisesRegex(Invalid, "source tree mismatch"):
            verify_history_stream(io.BytesIO(self.wire([bad], head=bad[0])))

    def test_radix_branch_tree_and_shared_directory_reconstruction(self):
        blob = self.store.add(1, b"shared")
        child = self.store.map([(b"x%02d" % i, file(blob)) for i in range(40)])
        child_raw = b"".join(
            b"100644 x%02d\0" % i + foreign_hash(b"blob", b"shared") for i in range(40)
        )
        child_oid = foreign_hash(b"tree", child_raw)
        source = self.store.map(
            [(b"one", directory(child)), (b"two", directory(child))]
        )
        tree = foreign_hash(
            b"tree", b"40000 one\0" + child_oid + b"40000 two\0" + child_oid
        )
        origin = anchor(self.store, source, raw_commit(tree))
        self.assertEqual(
            len(
                verify_history_stream(io.BytesIO(self.wire([origin], head=origin[0])))[
                    "origins"
                ]
            ),
            1,
        )

    def test_multiple_sorted_origins_with_merge_head(self):
        other = anchor(
            self.store,
            self.empty,
            raw_commit(self.empty_tree, message=b"two"),
            message=b"two",
        )
        head = revision(self.store, self.selected, (other[0], self.head))
        origins = sorted([other, self.origin])
        wire = self.wire(origins, head=head)
        corpus = verify_history_stream(io.BytesIO(wire))
        self.assertEqual(corpus["origins"], list(map(origin_json, origins)))
        self.assertEqual(
            encode_history(corpus, list(reversed(corpus["origins"]))), wire
        )
        for bad in (list(reversed(origins)), [origins[0], origins[0]]):
            with (
                self.subTest(bad=bad),
                self.assertRaisesRegex(Invalid, "unique and sorted"),
            ):
                verify_history_stream(io.BytesIO(self.wire(bad, head=head)))

    def test_encoder_v1_identity_and_source_parser_is_unchanged(self):
        corpus = self.store.corpus([self.selected, self.head])
        expected = build(self.selected, records(corpus), self.head)
        self.assertEqual(encode_history(corpus), expected)
        unborn = self.store.corpus([self.empty])
        self.assertEqual(encode_history(unborn), build(self.empty, records(unborn)))
        with self.assertRaisesRegex(Invalid, "magic/version"):
            verify_stream(io.BytesIO(self.wire()))

    def test_origin_requires_head_nonempty_count_and_revision_tag(self):
        for wire, message in (
            (
                build(self.empty, records(self.store.corpus([self.empty])), origins=[]),
                "authored head",
            ),
            (self.wire([]), "origin count must be nonzero"),
            (self.wire([(self.empty, self.origin[1], self.raw)]), "must be a Revision"),
            (
                self.wire([(self.head, b"\2" + self.origin[1][1:], self.raw)]),
                "algorithm tag",
            ),
        ):
            with (
                self.subTest(message=message),
                self.assertRaisesRegex(Invalid, message),
            ):
                verify_history_stream(io.BytesIO(wire))

    def test_origins_reachable_only_from_head_not_supplied_object_records(self):
        absent = b"\5" + b"x" * 32
        with self.assertRaisesRegex(Invalid, "not reachable from authored head"):
            verify_history_stream(
                io.BytesIO(self.wire([(absent, self.origin[1], self.raw)]))
            )
        # An origin from another valid closure cannot authorize surplus objects.
        other = anchor(
            self.store,
            self.empty,
            raw_commit(self.empty_tree, message=b"other"),
            message=b"other",
        )
        rows = records(self.store.corpus([self.selected, self.head, other[0]]))
        with self.assertRaisesRegex(Invalid, "unrooted"):
            verify_history_stream(
                io.BytesIO(self.wire(sorted([self.origin, other]), rows=rows))
            )
        child = anchor(self.store, self.empty, self.raw, parents=(self.head,))
        with self.assertRaisesRegex(Invalid, "zero native parents"):
            verify_history_stream(io.BytesIO(self.wire([child], head=child[0])))

    def test_complete_anchor_identity_not_just_change_id(self):
        for fields in (
            {"author": b"Wrong <w@test>"},
            {"message": b"different"},
            {"timestamp": 7_000_000_000},
            {"timezone": 90},
            {"change": b"x" * 16},
        ):
            bad = anchor(self.store, self.empty, self.raw, **fields)
            with (
                self.subTest(fields=fields),
                self.assertRaisesRegex(Invalid, "anchor mismatch"),
            ):
                verify_history_stream(io.BytesIO(self.wire([bad], head=bad[0])))
        blob = self.store.add(1, b"wrong source")
        source = self.store.map([(b"file", file(blob))])
        bad = anchor(self.store, source, self.raw)
        with self.assertRaisesRegex(Invalid, "source tree mismatch"):
            verify_history_stream(io.BytesIO(self.wire([bad], head=bad[0])))
        conflict = self.store.signed(
            [directory(self.empty), directory(source)],
            [directory(self.selected)],
            region=1,
        )[1:]
        bad = anchor(self.store, conflict, self.raw)
        with self.assertRaisesRegex(Invalid, "resolved map"):
            verify_history_stream(io.BytesIO(self.wire([bad], head=bad[0])))
        path = self.store.signed([file(blob), b"\0"], [directory(self.empty)])
        bad = anchor(self.store, self.store.map([(b"c", path)]), self.raw)
        with self.assertRaisesRegex(Invalid, "unresolved entries"):
            verify_history_stream(io.BytesIO(self.wire([bad], head=bad[0])))

    def test_git_hash_and_canonical_object_hash_independently_checked(self):
        for origin in (
            (self.head, self.origin[1], self.raw + b"x"),
            (self.head, b"\1" + bytes(20), self.raw),
        ):
            with (
                self.subTest(origin=origin),
                self.assertRaisesRegex(Invalid, "commit hash"),
            ):
                verify_history_stream(io.BytesIO(self.wire([origin])))
        rows = records(self.store.corpus([self.selected, self.head]))
        changed = [(ref, raw + b"x" if ref == self.head else raw) for ref, raw in rows]
        with self.assertRaisesRegex(Invalid, "wrong object hash"):
            verify_history_stream(io.BytesIO(self.wire(rows=changed)))
        with self.assertRaisesRegex(Invalid, "missing owning"):
            verify_history_stream(
                io.BytesIO(self.wire(rows=[r for r in rows if r[0] != self.empty]))
            )

    def test_commit_import_parser_rejections_even_with_rehashed_origins(self):
        tree_line = b"tree " + self.empty_tree.hex().encode()
        invalid = [
            b"",
            self.raw.replace(b"\n\n", b"\n", 1),
            self.raw + b"\0",
            self.raw.replace(tree_line, tree_line + b"\n" + tree_line),
            self.raw.replace(tree_line, b"tree " + b"z" * 40),
            self.raw.replace(b"\nauthor", b"\nx-extra value\nauthor"),
            self.raw.replace(b"\nauthor", b"\n continuation\nauthor"),
            self.raw.replace(b"\ncommitter", b"\nauthor A <a@test> 7 +0000\ncommitter"),
            self.raw.replace(b"\ncommitter", b"\nparent " + b"a" * 40 + b"\ncommitter"),
            self.raw.replace(b"A <a@test>", b"bad identity"),
            self.raw.replace(b"A <a@test>", b"A <>"),
            self.raw.replace(b"A <a@test>", b"\xff <a@test>"),
            self.raw.replace(b"C <c@test>", b"C\r <c@test>"),
            self.raw.replace(b" 7 +0130", b" -7 +0130"),
            self.raw.replace(b" 7 +0130", b" 07 +0130"),
            self.raw.replace(b" 7 +0130", b" 9223372036854775808 +0130"),
            self.raw.replace(b" 11 -0230", b" 9223372037 -0230"),
            self.raw.replace(b"-0230", b"+2400"),
            self.raw.replace(b"-0230", b"-0060"),
            self.raw.replace(b"-0230", b"0230"),
            self.raw.replace(b"original\n", b"\xff"),
            self.raw.replace(
                b"\nparent " + b"70" * 20,
                b"\nparent " + b"70" * 20 + b"\nparent " + b"70" * 20,
            ),
            raw_commit(self.empty_tree, [bytes([i]) * 20 for i in range(17)]),
        ]
        for raw in invalid:
            bad = anchor(self.store, self.empty, raw)
            with self.subTest(raw=raw), self.assertRaises(Invalid):
                verify_history_stream(io.BytesIO(self.wire([bad], head=bad[0])))

    def test_import_parser_supported_opaque_headers_and_person_boundaries(self):
        raw = raw_commit(
            self.empty_tree,
            [bytes([i]) * 20 for i in range(16)],
            author=b"A <a@test> 9223372036854775807 +2359",
            committer=b"\xff <c@test> 0 -0000",
            message=b"",
            extra=b"\ngpgsig arbitrary\n continuation\navc-revision forged\navc-author forged",
        ).replace(
            self.empty_tree.hex().encode(), self.empty_tree.hex().upper().encode()
        )
        origin = anchor(
            self.store, self.empty, raw, message=b"", timestamp=0, timezone=0
        )
        self.assertEqual(
            verify_history_stream(io.BytesIO(self.wire([origin], head=origin[0])))[
                "origins"
            ],
            [origin_json(origin)],
        )
        # Required committer is not replaceable by provenance/audit headers.
        raw = self.raw.replace(b"committer ", b"avc-committer ")
        bad = anchor(self.store, self.empty, raw)
        with self.assertRaises(Invalid):
            verify_history_stream(io.BytesIO(self.wire([bad], head=bad[0])))

    def test_origin_count_lengths_and_shared_payload_budget_before_read(self):
        corpus = self.store.corpus([self.empty, self.head])
        rows = records(corpus)
        # Strip v1 footer/checksum to get the identical v2 object section.
        prefix = b"AVCHIS02" + build(self.empty, rows, self.head)[8:-40]
        for count in (100_001, (1 << 64) - 1):
            probe = ReadProbe(prefix + struct.pack("<Q", count))
            with (
                self.subTest(count=count),
                self.assertRaisesRegex(Limit, "origin-count"),
            ):
                verify_history_stream(probe)
            self.assertEqual(probe.tell(), len(prefix) + 8)
        header = prefix + struct.pack("<Q", 1) + self.head + self.origin[1]
        for length in (16 * 1024 * 1024 + 1, (1 << 64) - 1):
            probe = ReadProbe(header + struct.pack("<Q", length))
            with (
                self.subTest(length=length),
                self.assertRaisesRegex(Limit, "origin-payload"),
            ):
                verify_history_stream(
                    probe, replace(HISTORY_LIMITS, object_bytes=length)
                )
            self.assertEqual(probe.tell(), len(header) + 8)
        wire = self.wire(selected=self.empty)
        total = sum(len(raw) for _, raw in rows) + len(self.raw)
        limits = replace(HISTORY_LIMITS, closure_bytes=total, file_bytes=len(wire))
        self.assertEqual(
            verify_history_stream(io.BytesIO(wire), limits)["origins"],
            [origin_json(self.origin)],
        )
        probe = ReadProbe(wire)
        with self.assertRaisesRegex(Limit, "closure-payload"):
            verify_history_stream(probe, replace(limits, closure_bytes=total - 1))
        self.assertEqual(probe.tell(), len(header) + 8)
        with self.assertRaisesRegex(Limit, "whole-file"):
            verify_history_stream(
                io.BytesIO(wire), replace(limits, file_bytes=len(wire) - 1)
            )

    def test_all_v2_truncations_footer_checksum_and_eof(self):
        wire = self.wire()
        for length in range(len(wire)):
            with self.subTest(length=length), self.assertRaises(Invalid):
                verify_history_stream(io.BytesIO(wire[:length]))
        for bad, message in (
            (self.wire().replace(b"AVCHEND2", b"AVCHEND1"), "completion marker"),
            (wire[:-32] + blake3(wire[:-40]).digest(), "checksum"),
            (wire + b"x", "trailing bundle"),
            (wire * 2, "trailing bundle"),
        ):
            with (
                self.subTest(message=message),
                self.assertRaisesRegex(Invalid, message),
            ):
                verify_history_stream(io.BytesIO(bad))

    def test_expanded_shared_tree_budget_not_only_unique_maps(self):
        source, tree = self.empty, self.empty_tree
        # 17 tiny unique maps describe >100k expanded entries; no huge allocation.
        for _ in range(17):
            source = self.store.map(
                [(b"a", directory(source)), (b"b", directory(source))]
            )
            tree = foreign_hash(b"tree", b"40000 a\0" + tree + b"40000 b\0" + tree)
        origin = anchor(self.store, source, raw_commit(tree))
        with self.assertRaisesRegex(Limit, "expanded Git"):
            verify_history_stream(io.BytesIO(self.wire([origin], head=origin[0])))

    def test_conversion_entry_budget_is_shared_across_distinct_origin_roots(self):
        blob = self.store.add(1, b"shared blob")
        oid = foreign_hash(b"blob", b"shared blob")
        roots = [self.store.map([(name, file(blob))]) for name in (b"a", b"b")]
        trees = [
            foreign_hash(b"tree", b"100644 " + name + b"\0" + oid)
            for name in (b"a", b"b")
        ]
        origins = [
            anchor(self.store, root, raw_commit(tree))
            for root, tree in zip(roots, trees)
        ]
        head = anchor(
            self.store,
            roots[0],
            raw_commit(trees[0]),
            parents=tuple(origin[0] for origin in origins),
            change=b"m" * 16,
        )[0]
        origins.sort(key=lambda origin: origin[0])
        wire = self.wire(origins, head=head)
        with patch("bundle.GIT_ENTRIES", 2):
            self.assertEqual(len(verify_history_stream(io.BytesIO(wire))["origins"]), 2)
        with (
            patch("bundle.GIT_ENTRIES", 1),
            self.assertRaisesRegex(Limit, "expanded Git"),
        ):
            verify_history_stream(io.BytesIO(wire))

    def test_reconstruction_path_depth_and_entry_budgets(self):
        source, tree = self.empty, self.empty_tree
        for _ in range(4):
            source = self.store.map([(b"long", directory(source))])
            tree = foreign_hash(b"tree", b"40000 long\0" + tree)
        origin = anchor(self.store, source, raw_commit(tree))
        wire = self.wire([origin], head=origin[0])
        for constant, value, message in (
            ("GIT_DEPTH", 2, "nesting"),
            ("GIT_PATH", 10, "expanded Git"),
            ("GIT_ENTRIES", 2, "expanded Git"),
        ):
            with (
                patch("bundle." + constant, value),
                self.subTest(constant=constant),
                self.assertRaisesRegex(Limit, message),
            ):
                verify_history_stream(io.BytesIO(wire))

    def test_history_aggregate_hard_cap_cannot_be_relaxed(self):
        # Shrink the fixed cap to exercise its clamp without allocating 256 MiB.
        corpus = self.store.corpus([self.empty, self.head])
        total = sum(len(raw) for _, raw in records(corpus)) + len(self.raw)
        wire = self.wire(selected=self.empty)
        with (
            patch(
                "bundle.HISTORY_LIMITS",
                replace(HISTORY_LIMITS, closure_bytes=total - 1),
            ),
            self.assertRaisesRegex(Limit, "closure-payload"),
        ):
            verify_history_stream(
                io.BytesIO(wire), replace(HISTORY_LIMITS, closure_bytes=1 << 40)
            )

    def test_message_identity_caps_and_multichunk_raw_commit(self):
        for raw in (
            raw_commit(self.empty_tree, author=b"a" * 4096 + b" <a@test> 0 +0000"),
            raw_commit(self.empty_tree, message=b"x" * (1024 * 1024 + 1)),
        ):
            bad = anchor(self.store, self.empty, raw)
            with self.assertRaises(Invalid):
                verify_history_stream(io.BytesIO(self.wire([bad], head=bad[0])))
        message = b"x" * (2 * 64 * 1024 + 7)
        origin = anchor(
            self.store,
            self.empty,
            raw_commit(self.empty_tree, message=message),
            message=message,
        )
        probe = ReadProbe(self.wire([origin], head=origin[0]))
        self.assertEqual(verify_history_stream(probe)["origins"], [origin_json(origin)])
        self.assertEqual(max(probe.requests), 64 * 1024)

    def test_v2_selected_root_conflict_and_source_context_validation(self):
        conflict = self.store.signed(
            [directory(self.empty), directory(self.selected)],
            [directory(self.store.map([(b"other", directory(self.empty))]))],
            region=1,
        )[1:]
        wire = self.wire(selected=conflict)
        self.assertEqual(
            verify_history_stream(io.BytesIO(wire))["roots"][0], spelling(conflict)
        )
        path_conflict = self.store.signed(
            [b"\0", directory(self.selected)], [directory(self.empty)]
        )[1:]
        with self.assertRaisesRegex(Invalid, "region/context"):
            verify_history_stream(io.BytesIO(self.wire(selected=path_conflict)))

    def test_encoder_rejects_invalid_origins_and_tight_budgets(self):
        corpus = self.store.corpus([self.empty, self.head])
        row = origin_json(self.origin)
        for bad in (
            {**row, "commit": "sha256:" + "00" * 32},
            {**row, "raw_commit": "zz"},
            {**row, "extra": 1},
            {**row, "revision": spelling(self.empty)},
        ):
            with self.subTest(bad=bad), self.assertRaises(Invalid):
                encode_history(corpus, [bad])
        with self.assertRaisesRegex(Invalid, "unique and sorted"):
            encode_history(corpus, [row, row])
        total = sum(len(raw) for _, raw in records(corpus)) + len(self.raw)
        with self.assertRaisesRegex(Limit, "closure-payload"):
            encode_history(
                corpus, [row], replace(HISTORY_LIMITS, closure_bytes=total - 1)
            )
        with self.assertRaisesRegex(Limit, "whole-file"):
            encode_history(corpus, [row], replace(HISTORY_LIMITS, file_bytes=80))

    def test_v2_cli_json_preserves_verified_origins(self):
        with TemporaryDirectory() as scratch:
            path = Path(scratch) / "git-history.avchis"
            path.write_bytes(self.wire())
            result = subprocess.run(
                [
                    sys.executable,
                    str(Path(__file__).with_name("bundle.py")),
                    str(path),
                    "--history",
                    "--json",
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            corpus = json.loads(result.stdout)
            self.assertEqual(corpus["origins"], [origin_json(self.origin)])
            self.assertEqual(encode_history(corpus), path.read_bytes())


if __name__ == "__main__":
    unittest.main()
