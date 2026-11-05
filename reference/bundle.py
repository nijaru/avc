#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11,<3.15"
# dependencies = ["blake3==1.0.8"]
# ///
"""Verify an experimental AVCEXP01 object-only source export, not a full backup."""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path

from blake3 import blake3
from check import (
    Invalid,
    Limit,
    require,
    spelling,
    verify_corpus,
)
from check import (
    Limits as CodecLimits,
)

MAGIC = b"AVCEXP01"
END = b"AVCEND01"
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


def verify_stream(reader, limits=DEFAULT_LIMITS):
    """Return a canonical corpus only after framing, checksum, and closure pass."""
    stream = Stream(reader, limits.file_bytes)
    require(stream.take(8) == MAGIC, "bundle magic/version")
    root = stream.take(33)
    require(root[0] in (2, 3, 4), "bundle root must be a source map or root conflict")
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

    # The end marker participates in the outer hash; the digest does not.
    require(stream.take(8) == END, "bundle completion marker")
    digest = stream.take(32, hashed=False)
    require(digest == stream.hash.digest(), "bundle checksum")
    stream.eof()

    corpus = {"objects": records, "roots": [spelling(root)]}
    verify_corpus(
        corpus,
        CodecLimits(
            corpus_bytes=limits.closure_bytes,
            object_bytes=limits.object_bytes,
            objects=limits.objects,
        ),
    )
    return corpus


def verify_file(path, limits=DEFAULT_LIMITS):
    with open(path, "rb") as reader:
        return verify_stream(reader, limits)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("file", type=Path)
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
    parser.add_argument("--max-file-bytes", type=int, default=BundleLimits.file_bytes)
    args = parser.parse_args()
    limits = BundleLimits(
        args.max_object_bytes,
        args.max_closure_bytes,
        args.max_objects,
        args.max_file_bytes,
    )
    # The codec enforces its own traversal limits; allow those to take effect
    # rather than depending on the interpreter's smaller default recursion cap.
    sys.setrecursionlimit(20_000)
    corpus = verify_file(args.file, limits)
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
