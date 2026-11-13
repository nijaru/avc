#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11,<3.15"
# dependencies = ["blake3==1.0.8"]
# ///
"""Verify experimental AVC source/history streams, not full repository backups."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import re
import struct
import sys
from dataclasses import dataclass
from pathlib import Path

from blake3 import blake3
from check import (
    Invalid,
    Limit,
    encode_node,
    identify,
    parse_id,
    require,
    spelling,
    unhex,
    verify_corpus,
)
from check import (
    Limits as CodecLimits,
)

MAGIC = b"AVCEXP01"
END = b"AVCEND01"
HISTORY_MAGIC = b"AVCHIS01"
HISTORY_END = b"AVCHEND1"
ORIGIN_MAGIC = b"AVCHIS02"
ORIGIN_END = b"AVCHEND2"
MAX_ORIGINS = 100_000
GIT_ENTRIES = 100_000
GIT_DEPTH = 256
GIT_PATH = 4096
CHUNK = 64 * 1024


@dataclass(frozen=True)
class BundleLimits:
    object_bytes: int = 16 * 1024 * 1024
    closure_bytes: int = 256 * 1024 * 1024
    objects: int = 100_000
    # 49-byte header, 41 bytes per record, and 40-byte completion/checksum.
    file_bytes: int = 256 * 1024 * 1024 + 41 * 100_000 + 89

    def __post_init__(self):
        require(
            all(
                type(n) is int and n > 0
                for n in (
                    self.object_bytes,
                    self.closure_bytes,
                    self.objects,
                    self.file_bytes,
                )
            ),
            "bundle limits must be positive integers",
        )


DEFAULT_LIMITS = BundleLimits()
# V2 adds the head, origin count, and 62 bytes per origin (payload is shared).
HISTORY_LIMITS = BundleLimits(
    file_bytes=BundleLimits.file_bytes + 34 + 8 + 62 * MAX_ORIGINS
)


class Stream:
    def __init__(self, reader, limit):
        self.reader = reader
        self.limit = limit
        self.position = 0
        self.hash = blake3()

    def take(self, size, *, hashed=True):
        # This check precedes reads/allocations, including for u64 wire lengths.
        if size > self.limit - self.position:
            raise Limit("bundle whole-file byte budget")
        output = bytearray()
        while len(output) < size:
            piece = self.reader.read(min(CHUNK, size - len(output)))
            require(bool(piece), "truncated bundle")
            output.extend(piece)
            self.position += len(piece)
            if hashed:
                self.hash.update(piece)
        return bytes(output)

    def eof(self):
        # A one-byte probe is necessary even when a valid stream exactly fills
        # its budget; never consume or allocate arbitrary trailing input.
        require(self.reader.read(1) == b"", "trailing bundle bytes")


def _verify_stream(reader, limits, *, history):
    stream = Stream(reader, limits.file_bytes)
    magic = stream.take(8)
    require(
        magic in (HISTORY_MAGIC, ORIGIN_MAGIC) if history else magic == MAGIC,
        "bundle magic/version",
    )
    version2 = magic == ORIGIN_MAGIC
    root = stream.take(33)
    require(root[0] in (2, 3, 4), "bundle root must be a source map or root conflict")
    roots = [spelling(root)]
    if history:
        tag = stream.take(1)[0]
        require(tag in (0, 1), "bundle authored-head tag must be 0 or 1")
        if tag == 1:
            head = stream.take(33)
            require(head[0] == 5, "bundle authored head must be a Revision")
            roots.append(spelling(head))
    require(not version2 or len(roots) == 2, "v2 requires an authored head")
    count = int.from_bytes(stream.take(8), "little")
    require(count != 0, "bundle record count must be nonzero")
    if count > limits.objects:
        raise Limit("bundle object-count budget")

    records = []
    previous = None
    total = 0
    for _ in range(count):
        ref = stream.take(33)
        require(
            previous is None or previous < ref, "bundle IDs must be unique and sorted"
        )
        previous = ref
        length = int.from_bytes(stream.take(8), "little")
        if length > limits.object_bytes:
            raise Limit("bundle object-payload byte budget")
        if length > limits.closure_bytes - total:
            raise Limit("bundle closure-payload byte budget")
        total += length
        payload = stream.take(length)
        records.append({"id": spelling(ref), "payload": payload.hex()})

    origins = []
    if version2:
        count = int.from_bytes(stream.take(8), "little")
        require(count != 0, "bundle origin count must be nonzero")
        if count > MAX_ORIGINS:
            raise Limit("bundle origin-count budget")
        previous = None
        for _ in range(count):
            revision = stream.take(33)
            require(revision[0] == 5, "origin ID must be a Revision")
            require(
                previous is None or previous < revision,
                "origin IDs must be unique and sorted",
            )
            previous = revision
            oid = stream.take(21)
            require(oid[0] == 1, "origin requires SHA-1 algorithm tag")
            length = int.from_bytes(stream.take(8), "little")
            if length > limits.object_bytes:
                raise Limit("bundle origin-payload byte budget")
            if length > limits.closure_bytes - total:
                raise Limit("bundle closure-payload byte budget")
            total += length
            origins.append((revision, oid[1:], stream.take(length)))

    # The end marker participates in the outer hash; the digest does not.
    end = ORIGIN_END if version2 else HISTORY_END if history else END
    require(stream.take(8) == end, "bundle completion marker")
    digest = stream.take(32, hashed=False)
    require(digest == stream.hash.digest(), "bundle checksum")
    stream.eof()

    corpus = {"objects": records, "roots": roots}
    store, _ = verify_corpus(
        corpus,
        CodecLimits(
            corpus_bytes=limits.closure_bytes,
            object_bytes=limits.object_bytes,
            objects=limits.objects,
        ),
    )
    if version2:
        _verify_origins(store, origins)
        corpus["origins"] = [
            {
                "revision": spelling(ref),
                "commit": "sha1:" + oid.hex(),
                "raw_commit": raw.hex(),
            }
            for ref, oid, raw in origins
        ]
    return corpus


def git_hash(kind, raw):
    """Git SHA-1 framing/math only; not a collision-detection security oracle."""
    return hashlib.sha1(
        kind + b" " + str(len(raw)).encode("ascii") + b"\0" + raw
    ).digest()


def _header_oid(raw):
    require(re.fullmatch(rb"[0-9a-fA-F]{40}", raw) is not None, "Git header OID")
    return bytes.fromhex(raw.decode("ascii"))


def _identity(raw):
    name, sep, email = raw.rpartition(b" <")
    require(
        sep
        and name
        and email.endswith(b">")
        and len(email) > 1
        and not any(c in name + email[:-1] for c in (b"<", b">", b"\0", b"\r", b"\n")),
        "Git identity requires Name <email>",
    )


def _person(raw):
    fields = raw.rsplit(b" ", 2)
    require(len(fields) == 3, "Git person fields")
    identity, seconds, zone = fields
    _identity(identity)
    require(
        re.fullmatch(rb"(?:0|[1-9][0-9]*)", seconds) is not None,
        "invalid Git timestamp",
    )
    # Bound the conversion as well as its result (Python limits decimal parsing).
    require(len(seconds) <= 19, "Git timestamp out of range")
    seconds = int(seconds)
    require(seconds < 1 << 63, "Git timestamp out of range")
    require(re.fullmatch(rb"[+-][0-9]{4}", zone) is not None, "invalid Git timezone")
    hours, minutes = int(zone[1:3]), int(zone[3:5])
    require(hours <= 23 and minutes <= 59, "Git timezone outside +/-2359")
    return identity, seconds, (hours * 60 + minutes) * (-1 if zone[:1] == b"-" else 1)


def _commit(raw):
    # Match the import parser, including opaque extra headers/continuations and
    # the deliberately uninterpreted (possibly non-UTF-8) committer identity.
    require(b"\0" not in raw, "NUL-containing Git commit")
    headers, sep, message = raw.partition(b"\n\n")
    require(sep, "Git commit lacks header separator")
    tree = author = committer = None
    parents = []
    continuation = False
    for index, line in enumerate(headers.split(b"\n")):
        if line.startswith(b" "):
            require(continuation, "continuation on required Git header")
            continue
        key, space, value = line.partition(b" ")
        require(space and re.fullmatch(rb"[a-zA-Z0-9-]+", key), "invalid Git header")
        continuation = False
        if key == b"tree":
            require(tree is None and index == 0, "Git tree must be unique first header")
            tree = _header_oid(value)
        elif key == b"parent":
            require(tree is not None and author is None, "Git parent header order")
            if len(parents) >= 16:
                raise Limit("Git commit parents (maximum 16)")
            parent = _header_oid(value)
            require(parent not in parents, "duplicate Git parent")
            parents.append(parent)
        elif key == b"author":
            require(
                tree is not None and author is None and committer is None,
                "Git author header order",
            )
            author = _person(value)
        elif key == b"committer":
            require(
                author is not None and committer is None, "Git committer header order"
            )
            committer = _person(value)
        else:
            require(committer is not None, "extra Git headers must follow committer")
            continuation = True
    require(
        tree is not None and author is not None and committer is not None,
        "missing required Git header",
    )
    author = author[0]
    require(len(author) <= 4096, "Git author identity bound")
    if len(message) > 1024 * 1024:
        raise Limit("Git message bound")
    try:
        author.decode("utf-8")
        message.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise Invalid("Git author/message must be UTF-8") from exc
    time_ns = committer[1] * 1_000_000_000
    require(time_ns < 1 << 63, "Git committer timestamp outside native range")
    return tree, author, message, time_ns, committer[2]


class _GitTrees:
    """Reconstruct imported trees, not the stricter Git export projection.

    Cache unique native objects, but propagate expanded logical metrics on every
    directory edge. In particular, retained empty directory entries are legal.
    """

    def __init__(self, store):
        self.store = store
        self.trees = {}
        self.blobs = {}
        self.bytes = self.objects = self.bindings = 0

    def charge(self, size):
        if size > HISTORY_LIMITS.object_bytes:
            raise Limit("Git reconstructed object bytes")
        if (
            self.objects >= HISTORY_LIMITS.objects
            or size > HISTORY_LIMITS.closure_bytes - self.bytes
        ):
            raise Limit("Git reconstructed closure budget")
        self.objects += 1
        self.bytes += size

    def blob(self, ref):
        if ref not in self.blobs:
            raw = self.store.node(ref)
            self.charge(len(raw))
            self.blobs[ref] = git_hash(b"blob", raw)
        return self.blobs[ref]

    def tree(self, ref, depth=0):
        if depth > GIT_DEPTH:
            raise Limit("Git tree nesting")
        require(ref[0] in (2, 3), "Git origin source must be a resolved map")
        if ref in self.trees:
            return self.trees[ref]
        pairs = self.store.radix(ref)
        self.bindings += len(pairs)
        if self.bindings > GIT_ENTRIES:
            raise Limit("expanded Git conversion entries")
        entries = []
        count = max_depth = path = size = 0
        for ns, entry in pairs:
            name = bytes((ns[i] << 4) | ns[i + 1] for i in range(0, len(ns), 2))
            tag, child = entry[0], entry[1:34]
            child_count = child_depth = child_path = 0
            if tag == 3:
                oid, child_count, child_depth, child_path = self.tree(child, depth + 1)
                mode = b"40000"
            elif tag in (1, 2):
                oid = self.blob(child)
                mode = b"120000" if tag == 2 else b"100755" if entry[34] else b"100644"
            else:
                raise Invalid("Git origin source contains unresolved entries")
            count += 1 + child_count
            max_depth = max(max_depth, 1 + child_depth)
            path = max(path, len(name) + (1 + child_path if child_depth else 0))
            if count > GIT_ENTRIES or max_depth > GIT_DEPTH or path > GIT_PATH:
                raise Limit("expanded Git entries/depth/path")
            size += len(mode) + 1 + len(name) + 1 + 20
            if size > HISTORY_LIMITS.object_bytes:
                raise Limit("Git reconstructed tree bytes")
            entries.append((name + (b"/" if tag == 3 else b""), mode, name, oid))
        self.charge(size)
        raw = b"".join(
            mode + b" " + name + b"\0" + oid for _, mode, name, oid in sorted(entries)
        )
        result = git_hash(b"tree", raw), count, max_depth, path
        self.trees[ref] = result
        return result


def _verify_origins(store, origins):
    trees = _GitTrees(store)
    for ref, oid, raw in origins:
        require(
            ref in store.revisions,
            "origin Revision is not reachable from authored head",
        )
        source, parents, *_ = store.node(ref)
        require(not parents, "origin Revision must have zero native parents")
        require(git_hash(b"commit", raw) == oid, "Git origin commit hash")
        tree, author, message, timestamp, timezone = _commit(raw)
        change = blake3(b"AVC-GIT-ANCHOR\0v1\0" + bytes([1, 20]) + oid).digest()[:16]
        expected = encode_node(
            5, (source, (), change, author, message, timestamp, timezone)
        )
        require(identify(5, expected) == ref, "Git origin native anchor mismatch")
        require(trees.tree(source)[0] == tree, "Git origin source tree mismatch")


def encode_history(corpus, origins=None, limits=HISTORY_LIMITS):
    """Encode a verified corpus; no origins emits byte-identical AVCHIS01.

    Origins use the decoder's JSON records: revision, sha1: commit, raw_commit
    hex. If omitted, use corpus['origins'] (or none). Input order is normalized;
    every semantic check also runs before any bytes are returned.
    """
    limits = _history_limits(limits)
    roots = [parse_id(ref) for ref in corpus["roots"]]
    require(len(roots) in (1, 2), "history requires source and optional head")
    origins = corpus.get("origins", []) if origins is None else origins
    if len(origins) > MAX_ORIGINS:
        raise Limit("bundle origin-count budget")
    rows = corpus["objects"]
    if len(rows) > limits.objects:
        raise Limit("bundle object-count budget")
    output = bytearray()
    total = 0

    def put(raw):
        if len(raw) > limits.file_bytes - len(output):
            raise Limit("bundle whole-file byte budget")
        output.extend(raw)

    def payload(text):
        nonlocal total
        require(isinstance(text, str), "payload must be hex string")
        if len(text) > 2 * limits.object_bytes:
            raise Limit("bundle object/origin-payload byte budget")
        if len(text) > 2 * (limits.closure_bytes - total):
            raise Limit("bundle closure-payload byte budget")
        raw = unhex(text)
        total += len(raw)
        put(struct.pack("<Q", len(raw)))
        put(raw)

    put(ORIGIN_MAGIC if origins else HISTORY_MAGIC)
    put(roots[0])
    put(bytes([len(roots) - 1]))
    if len(roots) == 2:
        put(roots[1])
    put(struct.pack("<Q", len(rows)))
    for row in sorted(rows, key=lambda row: parse_id(row["id"])):
        require(set(row) == {"id", "payload"}, "object record fields")
        put(parse_id(row["id"]))
        payload(row["payload"])
    if origins:
        put(struct.pack("<Q", len(origins)))
        for origin in sorted(origins, key=lambda row: parse_id(row["revision"])):
            require(
                set(origin) == {"revision", "commit", "raw_commit"},
                "origin record fields",
            )
            put(parse_id(origin["revision"]))
            commit = origin["commit"]
            require(
                isinstance(commit, str) and re.fullmatch(r"sha1:[0-9a-f]{40}", commit),
                "origin commit spelling",
            )
            put(b"\1" + bytes.fromhex(commit[5:]))
            payload(origin["raw_commit"])
    put(ORIGIN_END if origins else HISTORY_END)
    put(blake3(output).digest())
    result = bytes(output)
    verify_history_stream(io.BytesIO(result), limits)
    return result


def verify_stream(reader, limits=DEFAULT_LIMITS):
    """Verify AVCEXP01 source-only framing and exact owning closure."""
    return _verify_stream(reader, limits, history=False)


def _history_limits(limits):
    return BundleLimits(
        object_bytes=min(limits.object_bytes, HISTORY_LIMITS.object_bytes),
        closure_bytes=min(limits.closure_bytes, HISTORY_LIMITS.closure_bytes),
        objects=min(limits.objects, HISTORY_LIMITS.objects),
        file_bytes=limits.file_bytes,
    )


def verify_history_stream(reader, limits=HISTORY_LIMITS):
    """Verify AVCHIS01/02; v2 adds verified origins to the canonical corpus.

    Roots retain selected source, then optional authored head. Resource limits
    may tighten, but cannot relax, the history payload/object wire caps.
    """
    return _verify_stream(reader, _history_limits(limits), history=True)


def verify_file(path, limits=DEFAULT_LIMITS):
    with open(path, "rb") as reader:
        return verify_stream(reader, limits)


def verify_history_file(path, limits=HISTORY_LIMITS):
    with open(path, "rb") as reader:
        return verify_history_stream(reader, limits)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("file", type=Path)
    parser.add_argument(
        "--history",
        action="store_true",
        help="parse AVCHIS01/02 selected authored history",
    )
    parser.add_argument(
        "--json", action="store_true", help="emit the verified canonical corpus"
    )
    parser.add_argument(
        "--max-object-bytes", type=int, default=BundleLimits.object_bytes
    )
    parser.add_argument(
        "--max-closure-bytes", type=int, default=BundleLimits.closure_bytes
    )
    parser.add_argument("--max-objects", type=int, default=BundleLimits.objects)
    parser.add_argument("--max-file-bytes", type=int)
    args = parser.parse_args()
    defaults = HISTORY_LIMITS if args.history else DEFAULT_LIMITS
    limits = BundleLimits(
        args.max_object_bytes,
        args.max_closure_bytes,
        args.max_objects,
        defaults.file_bytes if args.max_file_bytes is None else args.max_file_bytes,
    )
    # The codec enforces its own traversal limits; allow those to take effect
    # rather than depending on the interpreter's smaller default recursion cap.
    sys.setrecursionlimit(20_000)
    verify = verify_history_file if args.history else verify_file
    corpus = verify(args.file, limits)
    if args.json:
        print(json.dumps(corpus, indent=2))
    else:
        print(
            f"verified {len(corpus['objects'])} objects; source root {corpus['roots'][0]}"
        )


if __name__ == "__main__":
    try:
        main()
    except (Invalid, OSError, RecursionError) as exc:
        print(f"refused: {exc}", file=sys.stderr)
        sys.exit(1)
