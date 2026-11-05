"""Contract checks, flat oracles, deterministic random cases, and adversarial bytes."""

import json
import random
import subprocess
import sys
import unittest
from dataclasses import replace
from pathlib import Path
from tempfile import TemporaryDirectory

from blake3 import blake3
from check import (
    Invalid,
    Limits,
    Pending,
    Store,
    corpus_store,
    decode_node,
    directory,
    encode_node,
    file,
    identify,
    nibbles,
    packed,
    parse_id,
    spelling,
    uint,
    verify_corpus,
)
from generate import generate


class CodecTests(unittest.TestCase):
    def setUp(self):
        self.s = Store()
        self.a = self.s.add(1, b"a")
        self.b = self.s.add(1, b"b")
        self.c = self.s.add(1, b"c")
        self.fa, self.fb, self.fc = map(file, (self.a, self.b, self.c))

    def verify(self, root):
        return verify_corpus(self.s.corpus([root]))

    def invalid_map(self, payload, message=None):
        root = self.s.add(2, payload)
        with self.assertRaisesRegex(Invalid, message or "."):
            self.s.source(root)

    def test_empty_and_literal_one_file_payload(self):
        empty = self.s.map([])
        self.assertEqual(empty[0], 2)
        self.assertEqual(self.s.objects[empty], b"\0")
        one = self.s.map([(b"a", self.fa)])
        self.assertEqual(self.s.objects[one], b"\x01\x02\0a\x01" + self.a + b"\0")
        self.verify(one)

    def test_pinned_generation_and_corpus(self):
        pinned = json.loads(Path(__file__).with_name("vectors.json").read_text())
        self.assertEqual(generate(), pinned)
        verify_corpus(pinned)
        for case in pinned["negative"]:
            with self.subTest(case=case["name"]), self.assertRaises(Invalid):
                verify_corpus(case)

    def test_blob_identity_is_exact_and_typed(self):
        self.assertNotEqual(identify(1, b"a\r\n"), identify(1, b"a\n"))
        self.assertNotEqual(identify(1, b"\0"), identify(2, b"\0"))
        self.assertEqual(
            identify(1, b""),
            bytes.fromhex(
                "01" + blake3(b"AVC-OBJECT\0v1\0\x01" + bytes(8)).hexdigest()
            ),
        )
        self.assertEqual(parse_id(spelling(self.a)), self.a)
        for bad in (
            spelling(self.a).upper(),
            spelling(self.a) + " ",
            spelling(self.a)[:-1],
            spelling(self.a).replace("k01", "k06"),
            self.a.hex(),
            "avc1-k1-" + self.a[1:].hex(),
        ):
            with self.subTest(value=bad), self.assertRaises(Invalid):
                parse_id(bad)

    def test_random_order_flat_oracle_and_threshold_edits(self):
        rng = random.Random(0x41564301)
        for size in (0, 1, 31, 32, 33, 34, 65, 257, 1000):
            pairs = [
                (b"prefix-" + uint(i, 2).hex().encode(), self.fa if i % 2 else self.fb)
                for i in range(size)
            ]
            root = self.s.map(pairs)
            for _ in range(3):
                rng.shuffle(pairs)
                self.assertEqual(self.s.map(pairs), root)
            self.verify(root)
            # Flat oracle uses raw names/entries, independent of radix boundaries.
            recovered = self.s.radix(root)
            expected = sorted((nibbles(name), entry) for name, entry in pairs)
            self.assertEqual(recovered, expected)
            self.assertEqual(root[0], 2 if size <= 32 else 3)
        pairs = [(f"n{i:02}".encode(), self.fa) for i in range(34)]
        original = self.s.map(pairs[:32])
        self.assertEqual(self.s.map(pairs[:33])[0], 3)
        self.assertEqual(self.s.map(pairs[:32]), original)

    def test_literal_33_entry_branch_and_child_bytes(self):
        # Independently hand-layout the wire representation, without the node
        # encoder, nibble packer, or trie builder generating expected payloads.
        children = []
        for high, lows in ((4, range(16)), (5, range(16)), (6, range(1))):
            payload = bytes([len(lows)]) + b"".join(
                b"\x01\0" + bytes([low << 4]) + self.fa for low in lows
            )
            children.append(identify(2, payload))
        expected = b"\x21\0\0\0\x02\0a\0\x70\0" + b"".join(children)
        root = self.s.map([(b"a" + bytes([n]), self.fa) for n in range(0x40, 0x61)])
        self.assertEqual(self.s.objects[root], expected)
        self.assertEqual(root, identify(3, expected))
        self.verify(root)

    def test_deterministic_random_payloads_fail_without_accidental_exceptions(self):
        rng = random.Random(20261104)
        for _ in range(2000):
            kind = rng.randrange(8)
            payload = rng.randbytes(rng.randrange(256))
            try:
                node = decode_node(kind, payload)
            except Invalid:
                continue
            self.assertIn(kind, (1, 2, 3, 4, 5))
            self.assertEqual(encode_node(kind, node), payload)

    def test_random_binary_names_and_flat_oracle(self):
        rng = random.Random(78)
        alphabet = [n for n in range(1, 256) if n != 47]
        for _ in range(30):
            names = set()
            while len(names) < rng.randrange(33, 200):
                name = bytes(rng.choices(alphabet, k=rng.randrange(1, 40)))
                if name not in (b".", b".."):
                    names.add(name)
            pairs = [(key, self.fa) for key in names]
            root = self.s.map(pairs)
            flat = self.s.radix(root)
            self.assertEqual(flat, [(nibbles(key), self.fa) for key in sorted(names)])
            self.verify(root)

    def test_duplicate_builder_input_is_not_silently_overwritten(self):
        with self.assertRaisesRegex(Invalid, "duplicate"):
            self.s.map([(b"a", self.fa), (b"a", self.fb)])

    def test_leaf_unsorted_duplicates_absent_mode_bounds(self):
        for payload, match in (
            (b"\x02" + packed((6, 2)) + self.fa + packed((6, 1)) + self.fb, "unsorted"),
            (b"\x02" + (packed((6, 1)) + self.fa) * 2, "duplicate"),
            (b"\x01" + packed((6, 1)) + b"\0", "Absent"),
            (b"\x01" + packed((6, 1)) + self.fa[:-1] + b"\x02", "boolean"),
            (b"\x01\xff\xff", "nibble count"),
            (b"\x01\x01\0\x61" + self.fa, "padding"),
            (b"\x01" + packed((6,)) + self.fa, "nibble length"),
            (b"\x21", "threshold"),
            (b"\0\0", "trailing"),
        ):
            with self.subTest(match=match):
                self.invalid_map(payload, match)

    def test_parent_context_rechecked_for_shared_leaf(self):
        # The same 17-entry leaf is legal under nibble 1 (names 1f,1f61,...)
        # but illegal under nibble 2, where its first name is slash (2f).
        suffixes = [((15,), self.fa)] + [((15, 6, i), self.fa) for i in range(16)]
        leaf = self.s.add(2, encode_node(2, suffixes))
        self.assertEqual(len(self.s.radix(leaf, (1,))), 17)
        branch = self.s.add(3, encode_node(3, (34, (), None, ((1, leaf), (2, leaf)))))
        with self.assertRaisesRegex(Invalid, "basename"):
            self.s.source(branch)
        # Standalone MapId must not validate the relative suffix as a full name.
        with self.assertRaisesRegex(Invalid, "nibble length"):
            self.s.source(leaf)

    def test_terminal_and_odd_prefixes(self):
        bindings = [(b"a", self.fa)] + [
            (b"a" + bytes([i]), self.fb) for i in range(1, 34)
        ]
        root = self.s.map(bindings)
        count, prefix, terminal, _ = self.s.node(root)
        self.assertEqual((count, prefix, terminal), (34, (6, 1), self.fa))
        self.verify(root)
        bindings = [
            (bytes([0x40 + i // 16, 0x40 + i % 16]), self.fa) for i in range(33)
        ]
        root = self.s.map(bindings)
        self.assertEqual(self.s.node(root)[1], (4,))
        self.verify(root)

    def test_wrong_counts_empty_children_and_nonmaximal_branch(self):
        root = self.s.map([(f"n{i}".encode(), self.fa) for i in range(33)])
        count, prefix, term, children = self.s.node(root)
        wrong = self.s.add(3, encode_node(3, (count + 1, prefix, term, children)))
        with self.assertRaisesRegex(Invalid, "count"):
            self.s.source(wrong)
        redundant = self.s.add(3, encode_node(3, (33, (), None, ((1, root),))))
        with self.assertRaisesRegex(Invalid, "redundant"):
            self.s.source(redundant)
        empty = self.s.map([])
        wrong = self.s.add(3, encode_node(3, (33, (), None, ((1, empty), (2, empty)))))
        with self.assertRaisesRegex(Invalid, "empty branch child"):
            self.s.source(wrong)

    def test_full_owning_closure_including_negative_terms(self):
        deep = self.s.map([(b"deep", self.fa)])
        conflict = self.s.signed([self.fb, self.fc], [directory(deep)])
        root = self.s.map([(b"conflict", conflict)])
        corpus = self.s.corpus([root])
        verify_corpus(corpus)
        for missing in (self.a, deep, conflict[1:]):
            bad = {
                **corpus,
                "objects": [
                    o for o in corpus["objects"] if o["id"] != spelling(missing)
                ],
            }
            with (
                self.subTest(missing=spelling(missing)),
                self.assertRaisesRegex(Invalid, "missing owning"),
            ):
                verify_corpus(bad)

    def test_signed_cancellation_flatten_sign_inversion_and_multiplicity(self):
        conflict = self.s.signed([self.fa, self.fb], [self.fc])
        self.assertEqual(self.s.signed([conflict, self.fc], [self.fb]), self.fa)
        self.assertEqual(self.s.signed([self.fa, self.fb], [conflict]), self.fc)
        repeated = self.s.signed([self.fa, self.fa], [self.fb])
        self.assertEqual(
            repeated[0], 4
        )  # Do not apply jj's lossy same-change heuristic.
        _, positive, negative = self.s.node(repeated[1:])
        self.assertEqual(positive, ((self.fa, 2),))
        self.assertEqual(negative, ((self.fb, 1),))
        self.assertEqual(self.s.signed([repeated, self.fb], [self.fa]), self.fa)
        # B rebase from A onto C then onto D: D + (C+B-A) - C = D+B-A.
        d = file(self.s.add(1, b"d"))
        first = self.s.signed([self.fc, self.fb], [self.fa])
        self.assertEqual(
            self.s.signed([d, first], [self.fc]), self.s.signed([d, self.fb], [self.fa])
        )
        # Revert the unresolved change C+B-A relative to C.
        self.assertEqual(self.s.signed([first, self.fc], [first]), self.fc)

    def test_signed_term_limit_exact_boundary(self):
        c = self.s.signed([self.fa] * 32, [self.fb] * 31)
        self.assertEqual(self.s.node(c[1:])[1], ((self.fa, 32),))
        with self.assertRaisesRegex(Invalid, "signed bound"):
            self.s.signed([self.fa] * 33, [self.fb] * 32)
        # Cancellation occurs before the resulting expression's bound is checked.
        self.assertEqual(self.s.signed([self.fa] * 100, [self.fa] * 99), self.fa)

    def test_root_and_path_conflicts_are_typed(self):
        empty = directory(self.s.map([]))
        one = directory(self.s.map([(b"a", self.fa)]))
        other = directory(self.s.map([(b"b", self.fb)]))
        root = self.s.signed([empty, one], [other], region=1)[1:]
        self.verify(root)
        with self.assertRaisesRegex(Invalid, "region/context"):
            self.verify(self.s.map([(b"x", b"\x04" + root)]))
        path = self.s.signed([self.fa, self.fb], [self.fc])[1:]
        with self.assertRaisesRegex(Invalid, "region/context"):
            self.verify(path)
        with self.assertRaisesRegex(Invalid, "root conflict type"):
            self.s.signed([self.fa, empty], [one], region=1)
        with self.assertRaisesRegex(Invalid, "root conflict type"):
            self.s.signed([b"\0"], region=1)
        # A source with no bindings is an empty map, not an absent root.
        self.verify(self.s.signed([empty], region=1)[1:])

    def test_opaque_merge_refuses_differing_resolutions_of_unresolved_base(self):
        conflict = self.s.signed([self.fa, self.fb], [b"\0"])
        # Algebra alone would turn two resolutions of an add/add conflict into
        # deletion. It is not a semantic merge of authored resolved values.
        self.assertEqual(self.s.signed([self.fa, self.fb], [conflict]), b"\0")
        before = dict(self.s.objects)
        self.assertEqual(
            self.s.merge_opaque(conflict, self.fa, self.fb), Pending("UnresolvedBase")
        )
        self.assertEqual(
            self.s.merge_opaque(conflict, self.fb, self.fa), Pending("UnresolvedBase")
        )
        self.assertEqual(self.s.objects, before)

    def test_opaque_merge_fast_paths_precede_unresolved_base_guard(self):
        conflict = self.s.signed([self.fa, self.fb], [b"\0"])
        self.assertEqual(self.s.merge_opaque(conflict, self.fa, self.fa), self.fa)
        self.assertEqual(self.s.merge_opaque(conflict, conflict, self.fb), self.fb)
        self.assertEqual(self.s.merge_opaque(conflict, self.fa, conflict), self.fa)
        self.assertEqual(self.s.merge_opaque(conflict, conflict, conflict), conflict)

    def test_opaque_merge_with_resolved_base_keeps_exact_cancellation(self):
        conflict = self.s.signed([self.fa, self.fb], [self.fc])
        # Base A, left A+B-C, right C cancels exactly to B; the base itself is
        # resolved, so positive-side conflicts can still be flattened safely.
        self.assertEqual(self.s.merge_opaque(self.fa, conflict, self.fc), self.fb)
        added = self.s.merge_opaque(b"\0", self.fa, self.fb)
        self.assertEqual(added, self.s.signed([self.fa, self.fb], [b"\0"]))
        self.verify(self.s.map([(b"added", added)]))

    def test_root_conflicts_reject_absent_in_either_sign(self):
        empty = directory(self.s.map([]))
        one = directory(self.s.map([(b"a", self.fa)]))
        for name, positive, negative in (
            ("positive-absence", [b"\0", one], [empty]),
            ("negative-absence", [empty, one], [b"\0"]),
        ):
            node = (
                1,
                tuple((t, 1) for t in sorted(positive)),
                tuple((t, 1) for t in sorted(negative)),
            )
            payload = encode_node(4, node)
            with (
                self.subTest(name=name, operation="decode"),
                self.assertRaisesRegex(Invalid, "root conflict term"),
            ):
                decode_node(4, payload)
            with (
                self.subTest(name=name, operation="normalize"),
                self.assertRaisesRegex(Invalid, "root conflict type"),
            ):
                self.s.signed(positive, negative, region=1)
            # The same exact alternatives remain valid at an ordinary path.
            path = self.s.signed(positive, negative, region=0)
            self.verify(self.s.map([(b"path", path)]))

    def chain(self, count, name=b"x", terminal=None):
        root = self.s.map([]) if terminal is None else terminal
        for _ in range(count):
            root = self.s.map([(name, directory(root))])
        return root

    def test_depth_limit_including_empty_directory_bindings(self):
        valid = self.chain(256)
        self.verify(valid)
        invalid = self.s.map([(b"x", directory(valid))])
        with self.assertRaisesRegex(Invalid, "path bound"):
            self.verify(invalid)

    def test_path_byte_limits_and_negative_conflict_alternative(self):
        # 16 * 255 + 15 separators = 4095 bytes. One nested one-byte
        # component brings it to 4097 and is invalid, not silently truncated.
        valid = self.chain(16, b"x" * 255)
        self.verify(valid)
        invalid = self.s.map([(b"x", directory(valid))])
        with self.assertRaisesRegex(Invalid, "path bound"):
            self.verify(invalid)
        # Exactly 4096: 16 long names and a final zero-length separator cannot
        # be used; instead extend one of sixteen shorter components by 1.
        names = [b"x" * 255] * 15 + [b"y" * 254, b"z"]
        self.assertEqual(sum(map(len, names)) + len(names) - 1, 4096)
        root = self.s.map([])
        for name in reversed(names):
            root = self.s.map([(name, directory(root))])
        self.verify(root)
        # A negative directory term is just as owning/bounded as a positive one.
        alt = self.s.signed([self.fa, self.fb], [directory(valid)])
        root = self.s.map([(b"xx", alt)])
        with self.assertRaisesRegex(Invalid, "path bound"):
            self.verify(root)

    def test_revision_order_utf8_and_exact_fields(self):
        root = self.s.map([])
        first = self.s.authored(root)
        second = self.s.authored(root, [first], timestamp=-1)
        a = self.s.authored(root, [first, second])
        b = self.s.authored(root, [second, first])
        self.assertNotEqual(a, b)
        self.verify(a)
        self.verify(b)
        for kwargs in (
            {"timezone": 1440},
            {"timezone": -1440},
            {"timestamp": 1 << 63},
            {"timestamp": -(1 << 63) - 1},
            {"author": b"\xff"},
            {"message": b"\xc0\x80"},
            {"parents": [first, first]},
            {"change": b"short"},
            {"author": b"a" * 4097},
        ):
            with self.subTest(kwargs=str(kwargs)[:80]), self.assertRaises(Invalid):
                self.s.authored(root, **kwargs)
        valid = self.s.authored(root, author=b"a" * 4096, message=b"m" * 1048576)
        self.verify(valid)

    def test_bounded_unknown_lengths_and_resource_failures(self):
        with self.assertRaisesRegex(Invalid, "cap"):
            decode_node(5, self.s.map([]) + b"\0" + bytes(16) + uint(0xFFFFFFFF, 4))
        with self.assertRaisesRegex(Invalid, "truncated"):
            decode_node(5, self.s.map([]) + b"\0" + bytes(16) + uint(4096, 4))
        child = self.s.map([(b"a", self.fa)])
        root = self.s.map([(b"dir", directory(child))])
        corpus = self.s.corpus([root])
        for limits in (
            replace(Limits(), object_bytes=1),
            replace(Limits(), objects=1),
            replace(Limits(), visits=1),
            replace(Limits(), nesting=1),
            replace(Limits(), corpus_bytes=1),
        ):
            with self.subTest(limits=limits), self.assertRaises(Invalid):
                verify_corpus(corpus, limits)

    def test_recovered_key_workspace_is_bounded_before_retaining_large_map(self):
        root = self.s.map([(b"x" * 20, self.fa)])
        corpus = self.s.corpus([root])
        self.assertLess(
            sum(len(bytes.fromhex(o["payload"])) for o in corpus["objects"]), 200
        )
        with self.assertRaisesRegex(Invalid, "recovered-key workspace budget"):
            verify_corpus(corpus, replace(Limits(), corpus_bytes=200))

    def test_wrong_hash_unknown_kind_missing_root_and_duplicate_records(self):
        root = self.s.map([(b"a", self.fa)])
        corpus = self.s.corpus([root])
        bad = json.loads(json.dumps(corpus))
        bad["objects"][0]["payload"] += "00"
        with self.assertRaisesRegex(Invalid, "hash"):
            verify_corpus(bad)
        with self.assertRaisesRegex(Invalid, "unknown kind"):
            decode_node(6, b"")
        with self.assertRaisesRegex(Invalid, "missing owning"):
            Store().source(root)
        with self.assertRaisesRegex(Invalid, "duplicate object"):
            verify_corpus({**corpus, "objects": corpus["objects"] * 2})
        with self.assertRaisesRegex(Invalid, "unrooted"):
            verify_corpus({**corpus, "roots": []})

    def test_verify_is_repeatable_and_derived_cache_is_not_external_authority(self):
        root = self.s.map([(b"nested", directory(self.s.map([(b"a", self.fa)])))])
        store, roots = corpus_store(self.s.corpus([root]))
        store.verify(roots)
        store.verify(roots)
        # Verification invalidates derived summaries before using untrusted input.
        store.summaries[root] = (0, 0)
        store.verify(roots)
        self.assertEqual(store.summaries[root], (2, 8))
        store.objects[self.a] = b"changed without changing ID"
        with self.assertRaisesRegex(Invalid, "hash"):
            store.verify(roots)

    def test_truncation_of_each_structured_kind(self):
        root = self.s.map([(b"a", self.fa)])
        wide = self.s.map([(f"a{i}".encode(), self.fa) for i in range(33)])
        conflict = self.s.signed([self.fa, self.fb], [self.fc])[1:]
        revision = self.s.authored(root)
        for ref in (root, wide, conflict, revision):
            raw = self.s.objects[ref]
            for n in range(len(raw)):
                with self.subTest(kind=ref[0], length=n), self.assertRaises(Invalid):
                    decode_node(ref[0], raw[:n])

    def test_real_verify_cli_and_fail_closed_input_limits(self):
        corpus = self.s.corpus([self.a])
        command = [
            sys.executable,
            str(Path(__file__).with_name("check.py")),
            "--verify",
        ]
        with TemporaryDirectory() as scratch:
            path = Path(scratch) / "corpus.json"
            path.write_text(json.dumps(corpus))
            valid = subprocess.run(
                command + [str(path)], capture_output=True, text=True, check=False
            )
            self.assertEqual(valid.returncode, 0, valid.stderr)
            bounded = subprocess.run(
                command + [str(path), "--max-bytes", "1"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(bounded.returncode, 1)
            self.assertIn("JSON input byte budget", bounded.stderr)
            # Duplicate JSON fields cannot overwrite an earlier authoritative
            # value, even if the final parsed value would have been valid.
            path.write_text('{"objects": [], ' + json.dumps(corpus)[1:])
            duplicate = subprocess.run(
                command + [str(path)], capture_output=True, text=True, check=False
            )
            self.assertEqual(duplicate.returncode, 1)
            self.assertIn("duplicate JSON field", duplicate.stderr)
            # A valid expression in the negative set must not be considered
            # malformed merely because its traversal exceeded a caller limit.
            corpus["negative"] = [
                {"name": "valid-empty-map", **self.s.corpus([self.s.map([])])}
            ]
            path.write_text(json.dumps(corpus))
            pending = subprocess.run(
                command + [str(path), "--max-visits", "1"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(pending.returncode, 1)
            self.assertIn("closure traversal budget", pending.stderr)

    def test_real_map_cli(self):
        source = {
            "objects": [{"id": spelling(self.a), "payload": "61"}],
            "bindings": [{"name": "ff", "entry": self.fa.hex()}],
        }
        result = subprocess.run(
            [sys.executable, str(Path(__file__).with_name("check.py")), "--map"],
            input=json.dumps(source),
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        corpus = json.loads(result.stdout)
        verify_corpus(corpus)
        self.assertEqual(corpus["roots"], [spelling(self.s.map([(b"\xff", self.fa)]))])


if __name__ == "__main__":
    unittest.main()
