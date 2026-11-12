"""Literal AVCHIS01 framing, selected-source preservation, and owning history."""

import io
import json
import struct
import subprocess
import sys
import unittest
from dataclasses import replace
from pathlib import Path
from tempfile import TemporaryDirectory

from blake3 import blake3
from bundle import (
    HISTORY_LIMITS,
    BundleLimits,
    verify_history_file,
    verify_history_stream,
    verify_stream,
)
from check import Invalid, Limit, Store, directory, file, identify, parse_id, spelling
from test_bundle import ReadProbe, finish
from test_bundle import build as build_source


def build(root, records, head=None, *, tag=None, count=None, end=b"AVCHEND1"):
    # No production framing helpers: fields and byte order are explicit here.
    body = b"AVCHIS01" + root + bytes([int(head is not None) if tag is None else tag])
    if head is not None:
        body += head
    body += struct.pack("<Q", len(records) if count is None else count)
    for ref, payload in records:
        body += ref + struct.pack("<Q", len(payload)) + payload
    return finish(body + end)


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
        self.assertEqual(HISTORY_LIMITS.file_bytes, BundleLimits.file_bytes + 34)
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
            (finish(b"AVCHIS02" + self.wire[8:-32]), "magic/version"),
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


if __name__ == "__main__":
    unittest.main()
