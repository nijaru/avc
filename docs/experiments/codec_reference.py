"""AVC v0 canonical-object *fixture builder*, not a native engine.

Minimal independent executable interpretation of docs/design.md sections 3, 4,
12. Uses the local slow BLAKE3 reference so no packages/network are required.
Consumers must not use this as a production parser or source store.
"""
from __future__ import annotations
from dataclasses import dataclass
import base64
from struct import pack
from typing import Mapping
from blake3_reference import blake3

MAGIC = b'AVC-OBJECT\x00v1\x00'
LEAF_LIMIT = 4096
K_BLOB, K_LEAF, K_BRANCH, K_STATE, K_REV, K_CONFLICT, K_PROV = range(1,8)


def u8(v: int) -> bytes:
    return pack('<B', v)


def u16(v: int) -> bytes:
    return pack('<H', v)


def u32(v: int) -> bytes:
    return pack('<I', v)


def u64(v: int) -> bytes:
    return pack('<Q', v)


def i64(v: int) -> bytes:
    return pack('<q', v)


def string(v: bytes) -> bytes:
    return u32(len(v)) + v


def nibs(b: bytes) -> tuple[int, ...]:
    return tuple(x for v in b for x in (v >> 4, v & 0x0f))


def packed_nibbles(v: tuple[int, ...]) -> bytes:
    return bytes(((v[i] << 4) | (v[i+1] if i+1 < len(v) else 0)) for i in range(0,len(v),2))


def name_valid(name: bytes) -> bool:
    return bool(name) and len(name) <= 255 and name not in (b'.',b'..') and not any(c in name for c in (0,47))


@dataclass(frozen=True, order=True)
class Ref:
    kind: int
    digest: bytes

    def __post_init__(self) -> None:
        if not 1 <= self.kind <= 255 or len(self.digest) != 32:
            raise ValueError('bad object ref')

    def encode(self) -> bytes:
        return bytes([self.kind]) + self.digest

    def display(self) -> str:
        return f'avc1-b3-k{self.kind:02x}-' + base64.b32encode(self.digest).decode('ascii').rstrip('=').lower()

    @classmethod
    def parse(cls, s: str) -> 'Ref':
        import re
        if not re.fullmatch(r'avc1-b3-k[0-9a-f]{2}-[a-z2-7]{52}', s):
            raise ValueError('malformed ObjectID')
        kind = int(s[9:11], 16)
        digest_str = s[12:]
        raw = base64.b32decode(digest_str.upper()+'====')
        val = cls(kind,raw)
        if val.display() != s:
            raise ValueError('noncanonical base32')
        return val


def object_id(kind: int, payload: bytes) -> Ref:
    return Ref(kind, blake3(MAGIC + u8(kind) + u64(len(payload)) + payload))


class Builder:
    def __init__(self) -> None:
        self.store: dict[Ref, bytes] = {}

    def store_obj(self, kind: int, payload: bytes) -> Ref:
        ref = object_id(kind,payload)
        other = self.store.setdefault(ref, payload)
        assert other == payload
        return ref

    def blob(self, content: bytes) -> Ref:
        return self.store_obj(K_BLOB, content)

    @staticmethod
    def file(blob: Ref, executable: bool=False) -> bytes:
        assert blob.kind == K_BLOB
        return b'\x01'+blob.encode()+u8(int(executable))

    @staticmethod
    def symlink(blob: Ref) -> bytes:
        assert blob.kind == K_BLOB
        return b'\x02'+blob.encode()

    @staticmethod
    def directory(map_ref: Ref) -> bytes:
        assert map_ref.kind in (K_LEAF,K_BRANCH)
        return b'\x03'+map_ref.encode()

    @staticmethod
    def conflict_entry(ref: Ref) -> bytes:
        assert ref.kind==K_CONFLICT
        return b'\x04'+ref.encode()

    @staticmethod
    def maybe(entry: bytes | None) -> bytes:
        return b'\x00' if entry is None else b'\x01' + entry

    def map(self, entries: Mapping[bytes, bytes]) -> Ref:
        for key in entries:
            if not name_valid(key):
                raise ValueError('bad name')
        ordered = [(nibs(name), entry) for name,entry in sorted(entries.items())]
        return self._radix(ordered)

    def _radix(self, terms: list[tuple[tuple[int, ...], bytes]]) -> Ref:
        payload = u32(len(terms)) + b''.join(u16(len(key)) + packed_nibbles(key) + entry for key,entry in terms)
        if len(payload) <= LEAF_LIMIT:
            return self.store_obj(K_LEAF, payload)
        if not terms:
            raise AssertionError('empty map should fit a leaf')
        prefix = list(terms[0][0])
        for key,_ in terms[1:]:
            n = next((i for i in range(min(len(prefix),len(key))) if prefix[i]!=key[i]),min(len(prefix),len(key)))
            prefix = prefix[:n]
        p = tuple(prefix)
        terminal: bytes|None = None
        buckets: dict[int,list[tuple[tuple[int,...],bytes]]] = {}
        for key,entry in terms:
            if len(key)==len(p):
                if terminal is not None: raise ValueError('duplicate')
                terminal=entry
            else:
                buckets.setdefault(key[len(p)],[]).append((key[len(p)+1:],entry))
        if (terminal is not None) + len(buckets) < 2:
            raise ValueError('non-progressing radix branch')
        payload = u16(len(p))+packed_nibbles(p)+u8(int(terminal is not None))
        if terminal is not None:
            payload += terminal
        bitmap = sum(1<<slot for slot in buckets)
        payload += u16(bitmap)
        for slot in sorted(buckets):
            payload += self._radix(buckets[slot]).encode()
        return self.store_obj(K_BRANCH,payload)

    def state(self, root: bytes) -> Ref:
        if not root or root[0] not in (3,4): raise ValueError('root must be directory or conflict')
        return self.store_obj(K_STATE,root)

    def revision(self, state: Ref, parents: list[Ref], change: bytes, author: bytes,
                 description: bytes, unix_ns: int, prov: Ref|None=None) -> Ref:
        if state.kind != K_STATE or len(change)!=16 or not (len(parents)<=64 and len(parents)==len(set(parents))):
            raise ValueError('revision field')
        if any(r.kind!=K_REV for r in parents) or (prov is not None and prov.kind!=K_PROV):
            raise ValueError('revision reference kind')
        payload=(state.encode()+u32(len(parents))+b''.join(p.encode() for p in parents)+change+
                 string(author)+string(description)+i64(unix_ns)+u8(prov is not None))
        if prov is not None:payload+=prov.encode()
        return self.store_obj(K_REV,payload)

    def conflict(self, region_kind: int, base: bytes|None, alternatives: list[bytes|None]) -> Ref:
        if region_kind not in (0,1):raise ValueError('region kind')
        vals=sorted({self.maybe(v) for v in alternatives})
        if len(alternatives)!=2 or len(vals)!=2: raise ValueError('conflict requires two distinct alternatives')
        if region_kind==1:
            for entry in ([base] if base is not None else [])+ [v for v in alternatives if v is not None]:
                if entry is None or entry[0] not in (3,4):
                    raise ValueError('non-directory coupled alternative')
            # A non-root coupled subtree may have a deletion alternative.
            # State-root admission separately requires all alternatives present.
        payload=u8(region_kind)+self.maybe(base)+b''.join(vals)
        return self.store_obj(K_CONFLICT,payload)

    @staticmethod
    def path(parts: tuple[bytes,...]) -> bytes:
        return u32(len(parts))+b''.join(string(p) for p in parts)

    @classmethod
    def move_op(cls, source, destination, expected_source, expected_destination):
        return b'\x01'+cls.path(source)+cls.path(destination)+expected_source+cls.maybe(expected_destination)

    @classmethod
    def copy_op(cls, source_value, destination, expected_destination):
        return b'\x02'+source_value+cls.path(destination)+cls.maybe(expected_destination)

    @classmethod
    def transform_record(cls, base, structural=(), edits=()):
        return (b'\x01'+base.encode()+u32(len(structural))+b''.join(structural)+
                u32(len(edits))+b''.join(cls.path(p)+cls.maybe(old)+cls.maybe(new)
                                       for p,old,new in edits))

    @classmethod
    def observation_record(cls, base, base_revision=None, selection_generation=0,
                           consistency=0, capture=None, partial=False, scope=((),),
                           covered=(), failed=(), skipped=(), resolutions=(), coverage_mode=0):
        payload=(b'\x03'+base.encode()+u8(base_revision is not None)+
                 (base_revision.encode() if base_revision is not None else b'')+
                 u64(selection_generation)+u8(consistency)+u8(capture is not None))
        if capture is not None:
            assert len(capture)==16
            payload+=capture
        payload+=u8(partial)+u8(coverage_mode)
        for paths in (scope,covered,failed,skipped):
            payload+=u32(len(paths))+b''.join(cls.path(p) for p in paths)
        payload+=u32(len(resolutions))
        for path,conflict,value in resolutions:
            payload+=cls.path(path)+conflict.encode()+cls.maybe(value)
        return payload

    @staticmethod
    def merge_record(*, history=None, source=None, allow_unrelated=False,
                     source_algorithm=(1,1), text_algorithm=(1,1), moves=(None,None)):
        assert (history is None)!=(source is None)
        payload=b'\x02'+u8(history is None)
        if history is not None:
            payload+=u32(history[0])+u32(history[1])+u8(allow_unrelated)
        else:
            payload+=b''.join(r.encode() for r in source)
        for tag,version in (source_algorithm,text_algorithm):
            payload+=u16(tag)+u32(version)
        for ref in moves:
            payload+=u8(ref is not None)+(ref.encode() if ref is not None else b'')
        return payload

    def provenance(self, result: Ref, inputs: list[Ref],
                   occurrences: list[tuple[tuple[bytes,...],Ref,list[int]]], record: bytes,
                   observation: bytes|None=None) -> Ref:
        payload = result.encode()+u32(len(inputs))+b''.join(v.encode() for v in inputs)
        payload += u32(len(occurrences))
        for path, conflict, roles in occurrences:
            payload += self.path(path)+conflict.encode()
            payload += u32(len(roles))+b''.join(u32(i) for i in roles)
        records=[record] if observation is None else [record,observation]
        return self.store_obj(K_PROV,payload+u8(len(records))+b''.join(records))
