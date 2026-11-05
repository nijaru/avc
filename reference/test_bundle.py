"""Independent physical stream construction and adversarial AVCEXP01 inputs."""

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
from bundle import BundleLimits, verify_file, verify_stream
from check import (
    Invalid,
    Limit,
    Store,
    directory,
    file,
    identify,
    parse_id,
    verify_corpus,
)


def finish(body):
    return body + blake3(body).digest()


def build(root, records, *, count=None, end=b"AVCEND01"):
    # Preserve caller order and use literal physical fields, not parser helpers.
    body = (
        b"AVCEXP01" + root + struct.pack("<Q", len(records) if count is None else count)
    )
    for ref, payload in records:
        body += ref + struct.pack("<Q", len(payload)) + payload
    return finish(body + end)


def from_corpus(corpus):
    return build(
        parse_id(corpus["roots"][0]),
        [
            (parse_id(row["id"]), bytes.fromhex(row["payload"]))
            for row in corpus["objects"]
        ],
    )


class ReadProbe(io.BytesIO):
    def __init__(self, data, fragment=None):
        super().__init__(data)
        self.requests = []
        self.fragment = fragment

    def read(self, count=-1):
        if not 0 <= count <= 64 * 1024:
            raise AssertionError(f"unbounded read: {count}")
        self.requests.append(count)
        return super().read(min(count, self.fragment) if self.fragment else count)


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.store = Store()
        self.blob = self.store.add(1, b"payload\0\r\n")
        self.root = self.store.map([(b"f", file(self.blob))])
        self.corpus = self.store.corpus([self.root])
        self.records = [
            (parse_id(row["id"]), bytes.fromhex(row["payload"]))
            for row in self.corpus["objects"]
        ]
        self.wire = build(self.root, self.records)

    def test_empty_map_literal_stream_and_file_boundary(self):
        root = identify(2, b"\0")
        body = (
            b"AVCEXP01"
            + root
            + b"\x01"
            + bytes(7)
            + root
            + b"\x01"
            + bytes(7)
            + b"\0AVCEND01"
        )
        wire = finish(body)
        expected = Store({root: b"\0"}).corpus([root])
        self.assertEqual(verify_stream(io.BytesIO(wire)), expected)
        limits = replace(BundleLimits(), object_bytes=1, file_bytes=len(wire))
        self.assertEqual(verify_stream(io.BytesIO(wire), limits), expected)
        with self.assertRaisesRegex(Limit, "whole-file"):
            verify_stream(io.BytesIO(wire), replace(limits, file_bytes=len(wire) - 1))

    def test_file_map_round_trip_and_short_reads(self):
        self.assertEqual(verify_stream(io.BytesIO(self.wire)), self.corpus)
        probe = ReadProbe(self.wire, fragment=3)
        self.assertEqual(verify_stream(probe), self.corpus)
        self.assertTrue(probe.requests)
        self.assertEqual(probe.tell(), len(self.wire))
        verify_corpus(self.corpus)

    def test_zero_length_blob_payload(self):
        blob = self.store.add(1, b"")
        root = self.store.map([(b"empty", file(blob))])
        corpus = self.store.corpus([root])
        self.assertEqual(verify_stream(io.BytesIO(from_corpus(corpus))), corpus)

    def test_large_payload_uses_bounded_stream_reads(self):
        blob = self.store.add(1, b"x" * (2 * 64 * 1024 + 7))
        root = self.store.map([(b"large", file(blob))])
        corpus = self.store.corpus([root])
        probe = ReadProbe(from_corpus(corpus))
        self.assertEqual(verify_stream(probe), corpus)
        self.assertEqual(max(probe.requests), 64 * 1024)

    def test_root_conflict_and_negative_directory_closure(self):
        empty = self.store.map([])
        other = self.store.map([(b"other", file(self.blob, True))])
        root = self.store.signed(
            [directory(empty), directory(self.root)], [directory(other)], region=1
        )[1:]
        corpus = self.store.corpus([root])
        self.assertEqual(verify_stream(io.BytesIO(from_corpus(corpus))), corpus)
        # Negative alternatives own dependencies just as positive ones do.
        records = [
            (parse_id(row["id"]), bytes.fromhex(row["payload"]))
            for row in corpus["objects"]
            if parse_id(row["id"]) != other
        ]
        with self.assertRaisesRegex(Invalid, "missing owning"):
            verify_stream(io.BytesIO(build(root, records)))

    def test_blob_revision_and_path_conflict_are_not_source_roots(self):
        revision = self.store.authored(self.root)
        path_conflict = self.store.signed(
            [file(self.blob), b"\0"], [directory(self.root)]
        )[1:]
        for root in (self.blob, revision, path_conflict):
            corpus = self.store.corpus([root])
            with self.subTest(kind=root[0]), self.assertRaises(Invalid):
                verify_stream(io.BytesIO(from_corpus(corpus)))

    def test_records_must_be_unique_and_typed_wire_sorted(self):
        for records in (list(reversed(self.records)), [self.records[0], *self.records]):
            with (
                self.subTest(records=len(records)),
                self.assertRaisesRegex(Invalid, "unique and sorted"),
            ):
                verify_stream(io.BytesIO(build(self.root, records)))

    def test_repaired_checksum_does_not_authorize_wrong_object_id(self):
        wrong = self.blob[:1] + bytes([self.blob[1] ^ 1]) + self.blob[2:]
        records = [
            (wrong if ref == self.blob else ref, payload)
            for ref, payload in self.records
        ]
        wire = build(self.root, sorted(records))
        self.assertEqual(wire[-32:], blake3(wire[:-32]).digest())
        with self.assertRaisesRegex(Invalid, "wrong object hash"):
            verify_stream(io.BytesIO(wire))

    def test_repaired_checksum_does_not_authorize_changed_payload(self):
        records = [
            (ref, payload + b"x" if ref == self.blob else payload)
            for ref, payload in self.records
        ]
        with self.assertRaisesRegex(Invalid, "wrong object hash"):
            verify_stream(io.BytesIO(build(self.root, records)))

    def test_valid_hashes_do_not_authorize_noncanonical_object(self):
        payload = b"\0\0"  # Empty leaf plus forbidden trailing byte.
        root = identify(2, payload)
        with self.assertRaisesRegex(Invalid, "trailing bytes"):
            verify_stream(io.BytesIO(build(root, [(root, payload)])))

    def test_missing_root_missing_blob_and_surplus_objects(self):
        for missing in (self.root, self.blob):
            records = [
                (ref, payload) for ref, payload in self.records if ref != missing
            ]
            with (
                self.subTest(missing=missing.hex()),
                self.assertRaisesRegex(Invalid, "missing owning"),
            ):
                verify_stream(io.BytesIO(build(self.root, records)))
        extra = (identify(1, b"not owned"), b"not owned")
        with self.assertRaisesRegex(Invalid, "unrooted"):
            verify_stream(io.BytesIO(build(self.root, sorted([*self.records, extra]))))

    def test_unknown_kind_is_not_an_object(self):
        ref = b"\x06" + bytes(32)
        with self.assertRaisesRegex(Invalid, "invalid reference"):
            verify_stream(io.BytesIO(build(self.root, [(ref, b"")])))

    def test_each_truncated_prefix_is_refused(self):
        for length in range(len(self.wire)):
            with self.subTest(length=length), self.assertRaises(Invalid):
                verify_stream(io.BytesIO(self.wire[:length]))

    def test_magic_completion_checksum_and_exact_eof(self):
        bad_magic = finish(b"AVCEXP02" + self.wire[8:-32])
        bad_end = build(self.root, self.records, end=b"AVCEND02")
        missing_end_from_hash = self.wire[:-32] + blake3(self.wire[:-40]).digest()
        corrupt_checksum = self.wire[:-1] + bytes([self.wire[-1] ^ 1])
        for wire, message in (
            (bad_magic, "magic/version"),
            (bad_end, "completion marker"),
            (missing_end_from_hash, "checksum"),
            (corrupt_checksum, "checksum"),
            (self.wire + b"\0", "trailing bundle"),
            (self.wire * 2, "trailing bundle"),
        ):
            with (
                self.subTest(message=message),
                self.assertRaisesRegex(Invalid, message),
            ):
                verify_stream(io.BytesIO(wire))

    def test_declared_record_counts(self):
        for count in (0, 1, 3):
            with self.subTest(count=count), self.assertRaises(Invalid):
                verify_stream(io.BytesIO(build(self.root, self.records, count=count)))
        header = b"AVCEXP01" + self.root + struct.pack("<Q", (1 << 64) - 1)
        probe = ReadProbe(header)
        with self.assertRaisesRegex(Limit, "object-count"):
            verify_stream(probe)
        self.assertEqual(probe.tell(), 49)
        with self.assertRaisesRegex(Limit, "object-count"):
            verify_stream(io.BytesIO(self.wire), replace(BundleLimits(), objects=1))

    def test_u64_object_length_is_bounded_before_payload_read(self):
        header = (
            b"AVCEXP01"
            + self.root
            + struct.pack("<Q", 1)
            + self.root
            + struct.pack("<Q", (1 << 64) - 1)
        )
        probe = ReadProbe(header)
        with self.assertRaisesRegex(Limit, "object-payload"):
            verify_stream(probe)
        self.assertEqual(probe.tell(), len(header))
        self.assertEqual(max(probe.requests), 33)

    def test_closure_and_whole_file_limits_precede_payload_read(self):
        total = sum(len(payload) for _, payload in self.records)
        probe = ReadProbe(self.wire)
        with self.assertRaisesRegex(Limit, "closure-payload"):
            verify_stream(probe, replace(BundleLimits(), closure_bytes=total - 1))
        # The last record's payload has not been read, nor has its tail.
        self.assertEqual(
            probe.tell(), 49 + 41 * len(self.records) + len(self.records[0][1])
        )
        header = (
            b"AVCEXP01"
            + self.root
            + struct.pack("<Q", 1)
            + self.root
            + struct.pack("<Q", 100)
        )
        probe = ReadProbe(header)
        with self.assertRaisesRegex(Limit, "whole-file"):
            verify_stream(probe, replace(BundleLimits(), file_bytes=150))
        self.assertEqual(probe.tell(), len(header))
        self.assertEqual(max(probe.requests), 33)

    def test_invalid_limits(self):
        for field in ("object_bytes", "closure_bytes", "objects", "file_bytes"):
            for value in (0, -1, True, 1.5):
                with self.subTest(field=field, value=value), self.assertRaises(Invalid):
                    replace(BundleLimits(), **{field: value})

    def test_real_cli_json_and_failure_emit_no_unverified_corpus(self):
        command = [sys.executable, str(Path(__file__).with_name("bundle.py"))]
        with TemporaryDirectory() as scratch:
            path = Path(scratch) / "source.avcexp"
            path.write_bytes(self.wire)
            self.assertEqual(verify_file(path), self.corpus)
            plain = subprocess.run(
                [*command, str(path)], capture_output=True, text=True, check=False
            )
            self.assertEqual(plain.returncode, 0, plain.stderr)
            self.assertIn("verified 2 objects", plain.stdout)
            result = subprocess.run(
                [*command, str(path), "--json"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout), self.corpus)
            verify_corpus(json.loads(result.stdout))
            bounded = subprocess.run(
                [*command, str(path), "--max-file-bytes", "50"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(bounded.returncode, 1)
            self.assertEqual(bounded.stdout, "")
            self.assertIn("whole-file", bounded.stderr)
            path.write_bytes(self.wire + b"unexpected")
            corrupt = subprocess.run(
                [*command, str(path), "--json"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(corrupt.returncode, 1)
            self.assertEqual(corrupt.stdout, "")
            self.assertIn("refused: trailing bundle", corrupt.stderr)


if __name__ == "__main__":
    unittest.main()
