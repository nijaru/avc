"""Small, unoptimized BLAKE3-256 reference for AVC format fixtures.

Derived independently from the BLAKE3 specification and cross-checked with
BLAKE3-team's published hash vectors (including chunk boundaries).
NOT a production or security-reviewed hash implementation.
Primary sources: https://github.com/BLAKE3-team/BLAKE3-specs
                 https://github.com/BLAKE3-team/BLAKE3/tree/master/test_vectors
"""
from __future__ import annotations
from dataclasses import dataclass
from struct import pack, unpack

IV = (0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
      0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19)
P = (2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8)
START, END, PARENT, ROOT = 1, 2, 4, 8
MASK = 0xffffffff


def _r(x: int, n: int) -> int:
    return ((x >> n) | (x << (32-n))) & MASK


def _mix(v: list[int], a: int, b: int, c: int, d: int, x: int, y: int) -> None:
    v[a] = (v[a] + v[b] + x) & MASK
    v[d] = _r(v[d] ^ v[a], 16)
    v[c] = (v[c] + v[d]) & MASK
    v[b] = _r(v[b] ^ v[c], 12)
    v[a] = (v[a] + v[b] + y) & MASK
    v[d] = _r(v[d] ^ v[a], 8)
    v[c] = (v[c] + v[d]) & MASK
    v[b] = _r(v[b] ^ v[c], 7)


def compress(cv: tuple[int, ...], block: bytes, counter: int, block_len: int, flags: int) -> tuple[int, ...]:
    assert len(cv) == 8 and len(block) == 64
    v = list(cv) + list(IV[:4]) + [counter & MASK, (counter >> 32) & MASK, block_len, flags]
    m = list(unpack('<16I', block))
    for _ in range(7):
        _mix(v, 0, 4, 8, 12, m[0], m[1])
        _mix(v, 1, 5, 9, 13, m[2], m[3])
        _mix(v, 2, 6, 10, 14, m[4], m[5])
        _mix(v, 3, 7, 11, 15, m[6], m[7])
        _mix(v, 0, 5, 10, 15, m[8], m[9])
        _mix(v, 1, 6, 11, 12, m[10], m[11])
        _mix(v, 2, 7, 8, 13, m[12], m[13])
        _mix(v, 3, 4, 9, 14, m[14], m[15])
        m = [m[p] for p in P]
    return tuple((v[i] ^ v[i+8]) & MASK for i in range(8)) + tuple((v[i+8] ^ cv[i]) & MASK for i in range(8))


@dataclass(frozen=True)
class Output:
    cv: tuple[int, ...]
    block: bytes
    counter: int
    block_len: int
    flags: int

    def words(self, root: bool = False) -> tuple[int, ...]:
        return compress(self.cv, self.block, self.counter if not root else 0, self.block_len,
                        self.flags | (ROOT if root else 0))

    def chaining(self) -> tuple[int, ...]:
        return self.words()[:8]


def _chunk(data: bytes, counter: int) -> Output:
    cv = IV
    blocks = [data[i:i+64] for i in range(0, len(data), 64)] or [b'']
    for i, raw in enumerate(blocks):
        flags = (START if i == 0 else 0) | (END if i == len(blocks)-1 else 0)
        out = Output(cv, raw.ljust(64, b'\0'), counter, len(raw), flags)
        if i < len(blocks)-1:
            cv = out.chaining()
    return out


def _parent(left: Output, right: Output) -> Output:
    return Output(IV, pack('<16I', *(left.chaining() + right.chaining())), 0, 64, PARENT)


def blake3(data: bytes) -> bytes:
    """Pure reference, one-shot. O(total input) memory; use only for fixtures."""
    chunks = [_chunk(data[i:i+1024], i // 1024) for i in range(0, len(data), 1024)] or [_chunk(b'', 0)]
    def reduce(nodes: list[Output]) -> Output:
        if len(nodes) == 1:
            return nodes[0]
        pivot = 1 << ((len(nodes)-1).bit_length()-1)
        return _parent(reduce(nodes[:pivot]), reduce(nodes[pivot:]))
    return pack('<16I', *reduce(chunks).words(root=True))[:32]


OFFICIAL_VECTORS = {
    0: 'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262',
    1: '2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213',
    2: '7b7015bb92cf0b318037702a6cdd81dee41224f734684c2c122cd6359cb1ee63',
    3: 'e1be4d7a8ab5560aa4199eea339849ba8e293d55ca0a81006726d184519e647f',
    64: '4eed7141ea4a5cd4b788606bd23f46e212af9cacebacdc7d1f4c6dc7f2511b98',
    65: 'de1e5fa0be70df6d2be8fffd0e99ceaa8eb6e8c93a63f2d8d1c30ecb6b263dee',
    1023: '10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11',
    1024: '42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7',
    1025: 'd00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444',
    2048: 'e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a',
    2049: '5f4d72f40d7a5f82b15ca2b2e44b1de3c2ef86c426c95c1af0b6879522563030',
}

if __name__ == '__main__':
    for n, want in OFFICIAL_VECTORS.items():
        found = blake3(bytes(i % 251 for i in range(n))).hex()
        assert found == want, (n, found, want)
        print(n, 'OK')
