"""Independent strict decoder and alternate radix encoder for AVC v0 fixtures.

Uses different control flow than codec_reference.Builder: decodes untrusted
payloads, checks all transitively referenced objects, then re-encodes a fully
reconstructed flat name map using a separate min/max-prefix split algorithm.
Both use the independently officially-vectored slow BLAKE3 fixture backend.
"""
from __future__ import annotations
from collections import defaultdict
from struct import unpack
from codec_reference import (Ref, object_id, u16, u32, nibs, packed_nibbles,
                             name_valid, K_BLOB,K_LEAF,K_BRANCH,K_STATE,K_REV,
                             K_CONFLICT,K_PROV)

# Reference-parser budgets, not canonical graph/path limits.
MAX_EDGES=30000
MAX_ACTIVE=256

class Invalid(ValueError):pass

class Cursor:
    def __init__(self, data: bytes):self.data,self.pos=data,0
    def take(self,n:int)->bytes:
        if n<0 or n>len(self.data)-self.pos:raise Invalid('truncated/oversized field')
        v=self.data[self.pos:self.pos+n];self.pos+=n;return v
    def number(self,n:int)->int:return int.from_bytes(self.take(n),'little')
    def flag(self)->bool:
        i=self.number(1)
        if i not in (0,1):raise Invalid('nonbinary flag')
        return bool(i)
    def str(self,limit:int|None=None)->bytes:
        size=self.number(4)
        if limit is not None and size>limit:raise Invalid('byte-string size limit')
        return self.take(size)
    def ref(self,allowed:set[int])->Ref:
        k=self.number(1)
        if k not in allowed:raise Invalid(f'wrong kind {k}')
        return Ref(k,self.take(32))
    def nibbles(self,n:int)->tuple[int,...]:
        raw=self.take((n+1)//2)
        if n%2 and raw[-1]&15:raise Invalid('nonzero trailing nibble pad')
        out=tuple(i for b in raw for i in (b>>4,b&15))
        return out[:n]
    def done(self)->None:
        if self.pos!=len(self.data):raise Invalid('trailing data')


def byte_name(ns: tuple[int,...])->bytes:
    if len(ns)%2:raise Invalid('key ends mid-byte')
    b=packed_nibbles(ns)
    if not name_valid(b):raise Invalid('invalid full byte name')
    return b


def alternate_map(flat:dict[bytes,bytes])->tuple[Ref, dict[Ref,bytes]]:
    """Alternate key-range partitioner: common prefix of sorted min/max.
    Unlike Builder, lengths are computed from bytes, not the Builder's nibble
    vector list. Child IDs are reconstructed only from each child range.
    """
    objects:dict[Ref,bytes]={}
    items=sorted(flat.items())
    def split(items:list[tuple[tuple[int,...],bytes]])->Ref:
        # A raw prefix-radix node is uniquely decided by its suffix set.
        leaf_parts=[len(items).to_bytes(4,'little')]
        for key,val in items:
            leaf_parts.extend((len(key).to_bytes(2,'little'),packed_nibbles(key),val))
        leaf=b''.join(leaf_parts)
        if len(leaf)<=4096:
            kind,payload=K_LEAF,leaf
        else:
            first,last=items[0][0],items[-1][0]
            k=0
            while k<min(len(first),len(last)) and first[k]==last[k]:k+=1
            same=first[:k]
            groups:dict[int,list[tuple[tuple[int,...],bytes]]]=defaultdict(list)
            terminal=None
            for full,val in items:
                if len(full)==k:terminal=val
                else:groups[full[k]].append((full[k+1:],val))
            if (terminal is not None)+len(groups)<2:raise Invalid('no branch split')
            bitmap=sum(1<<i for i in groups)
            out=(len(same).to_bytes(2,'little')+packed_nibbles(same)+bytes([int(terminal is not None)])+
                 (terminal if terminal is not None else b'')+bitmap.to_bytes(2,'little'))
            for slot in sorted(groups):
                out+=split(groups[slot]).encode()
            kind,payload=K_BRANCH,out
        ref=object_id(kind,payload)
        old=objects.setdefault(ref,payload)
        assert old==payload
        return ref
    return split([(nibs(key),val) for key,val in items]),objects


class Verifier:
    def __init__(self,objects:dict[Ref,bytes]):
        self.objects=objects
        self.active:set[Ref]=set()
        self.validated:set[Ref]=set()
        self.edges=0
        self.provenance_variants={}

    def fetch(self,ref:Ref)->bytes:
        if ref not in self.objects:raise Invalid('missing required object')
        payload=self.objects[ref]
        if object_id(ref.kind,payload)!=ref:raise Invalid('content hash mismatch')
        return payload

    def enter(self,ref:Ref)->bytes:
        self.edges+=1
        if self.edges>MAX_EDGES:raise Invalid('expansion budget exceeded')
        if ref in self.active:raise Invalid('cycle')
        if len(self.active)>=MAX_ACTIVE:raise Invalid('reference traversal depth budget exceeded')
        self.active.add(ref)
        return self.fetch(ref)

    def exit(self,ref:Ref):self.active.remove(ref)

    def entry(self,c:Cursor,depth:int=0, pathlen:int=0,conf_depth:int=0,require_dir:bool=False)->bytes:
        if depth>256 or pathlen>4096:raise Invalid('path constraints')
        start=c.pos
        tag=c.number(1)
        if require_dir and tag not in (3,4):raise Invalid('directory-valued conflict requires directory')
        if tag in (1,2):
            r=c.ref({K_BLOB});self.generic(r)
            if tag==1:c.flag()
        elif tag==3:
            r=c.ref({K_LEAF,K_BRANCH})
            self.map(r,(),depth,pathlen,conf_depth)
        elif tag==4:
            self.conflict(c.ref({K_CONFLICT}),depth,pathlen,conf_depth+1,require_dir=require_dir)
        else:raise Invalid('unsupported entry type')
        return c.data[start:c.pos]

    def maybe(self,c:Cursor,depth:int=0,pathlen:int=0,conf_depth:int=0,require_dir:bool=False,allow_absent:bool=True)->bytes:
        p=c.pos
        if c.flag():self.entry(c,depth,pathlen,conf_depth,require_dir)
        elif require_dir and not allow_absent:raise Invalid('absent directory alternative')
        return c.data[p:c.pos]

    def map(self,ref:Ref,prefix:tuple[int,...],depth:int,pathlen:int,conf_depth:int=0)->dict[bytes,bytes]:
        if depth>256 or pathlen>4096:raise Invalid('path constraints')
        data=self.enter(ref)
        c=Cursor(data)
        got:dict[bytes,bytes]={}
        if ref.kind==K_LEAF:
            if len(data)>4096:raise Invalid('oversize canonical leaf')
            count=c.number(4)
            if count>256:raise Invalid('impossible leaf count')
            for _ in range(count):
                tail=c.nibbles(c.number(2))
                name=byte_name(prefix+tail)
                if name in got:raise Invalid('duplicate name')
                v=self.entry(c,depth+1,pathlen+(1 if depth else 0)+len(name),conf_depth)
                got[name]=v
        elif ref.kind==K_BRANCH:
            common=c.nibbles(c.number(2));full=prefix+common
            if c.flag():
                name=byte_name(full)
                v=self.entry(c,depth+1,pathlen+(1 if depth else 0)+len(name),conf_depth)
                got[name]=v
            bitmap=c.number(2)
            if bitmap==0:raise Invalid('branch without children')
            if (bool(got)+bitmap.bit_count())<2:raise Invalid('uncompressed branch')
            for slot in range(16):
                if (bitmap>>slot)&1:
                    child=c.ref({K_LEAF,K_BRANCH})
                    partial=self.map(child,full+(slot,),depth,pathlen,conf_depth)
                    if set(got).intersection(partial):raise Invalid('overlapping names')
                    got.update(partial)
        else:raise Invalid('nonmap reference')
        c.done()
        self.exit(ref)
        if not prefix:
            alt,_=alternate_map(got)
            if alt!=ref:raise Invalid('noncanonical map shape / order')
        # Note: non-root suffix map shapes are checked via the independent
        # builder of the *root* to avoid losing their prefix parity context.
        return got

    def conflict(self,ref:Ref,depth:int=0,pathlen:int=0,conf_depth:int=1,root:bool=False,require_dir:bool=False)->None:
        if conf_depth>64:raise Invalid('conflict nesting >64')
        c=Cursor(self.enter(ref))
        region=c.number(1)
        if region not in (0,1) or ((root or require_dir) and region!=1):raise Invalid('conflict region/root kind')
        base=self.maybe(c,depth,pathlen,conf_depth,region==1,True)
        terms=[self.maybe(c,depth,pathlen,conf_depth,region==1,not(root or require_dir)) for _ in range(2)]
        c.done();self.exit(ref)
        if terms!=sorted(set(terms)):raise Invalid('noncanonical alternatives')
        # Parsing every present coupled alternative as a strictly directory-valued
        # Entry, including nested conflict alternatives, is stronger than checking
        # only the nested conflict's region byte. This prevents root states whose
        # apparent directory alternatives secretly include a nested deletion.
        return region

    def state(self,ref:Ref)->None:
        if ref.kind!=K_STATE:raise Invalid('not State')
        c=Cursor(self.enter(ref))
        start=c.pos
        tag=c.number(1)
        if tag==3:self.map(c.ref({K_LEAF,K_BRANCH}),(),0,0)
        elif tag==4:self.conflict(c.ref({K_CONFLICT}),0,0,1,root=True)
        else:raise Invalid('not valid state root')
        c.done();self.exit(ref)

    @staticmethod
    def path(c:Cursor):
        n=c.number(4)
        if n>256:raise Invalid('long path')
        parts=tuple(c.str(255) for _ in range(n))
        if any(not name_valid(p) for p in parts):raise Invalid('path part')
        if sum(map(len,parts))+max(0,len(parts)-1)>4096:raise Invalid('path length')
        return parts

    def paths(self,c:Cursor):
        n=c.number(4)
        if n>1000000:raise Invalid('path list budget')
        paths=[]
        for _ in range(n):
            path=self.path(c)
            if paths and path<=paths[-1]:raise Invalid('path list order')
            paths.append(path)
        return paths

    def record(self,c:Cursor,inputs:list[Ref]):
        tag=c.number(1)
        if tag==1:  # Transform: the structural batch precedes ordered leaf edits.
            self.state(c.ref({K_STATE}))
            n=c.number(4)
            if n>1000000:raise Invalid('structural count')
            for _ in range(n):
                op=c.number(1)
                if op==1:
                    if not self.path(c) or not self.path(c):raise Invalid('empty structural path')
                    self.entry(c);self.maybe(c)
                elif op==2:
                    self.entry(c)
                    if not self.path(c):raise Invalid('empty structural path')
                    self.maybe(c)
                else:raise Invalid('structural operation')
            n=c.number(4)
            if n>1000000:raise Invalid('leaf edit count')
            for _ in range(n):
                if not self.path(c):raise Invalid('empty leaf path')
                self.maybe(c);self.maybe(c)
        elif tag==2:  # Merge: reduction traces are derived, not another object graph.
            mode=c.number(1)
            if mode==0:
                if c.number(4)>=len(inputs) or c.number(4)>=len(inputs):raise Invalid('merge input index')
                c.flag()  # explicit permission for synthetic bases throughout reduction
            elif mode==1:
                for _ in range(3):self.state(c.ref({K_STATE}))
            else:raise Invalid('merge mode')
            for index in range(2):
                algorithm,version=c.number(2),c.number(4)
                if (algorithm==0 and (index==0 or version!=0)) or (algorithm!=0 and version==0):
                    raise Invalid('algorithm version')
            for _ in range(2):
                if c.flag():
                    evidence=c.ref({K_PROV});self.generic(evidence)
                    if 1 not in self.provenance_variants[evidence]:raise Invalid('move evidence must be Transform')
        elif tag==3:  # Observation is descriptive, never a replayable FS snapshot.
            base=c.ref({K_STATE});self.state(base)
            if c.flag():
                base_rev=c.ref({K_REV});self.generic(base_rev)
                if Cursor(self.fetch(base_rev)).ref({K_STATE})!=base:raise Invalid('observation base mismatch')
            c.number(8)  # non-owning selection generation
            if c.number(1)>2:raise Invalid('consistency label')
            if c.flag():c.take(16)  # non-owning capture/receipt identity
            partial=c.flag()
            coverage_mode=c.number(1)
            if coverage_mode not in (0,1):raise Invalid('coverage mode')
            scope=self.paths(c)
            if not scope:raise Invalid('empty observation scope')
            if any(b[:len(a)]==a for a,b in zip(scope,scope[1:])):raise Invalid('redundant scope prefix')
            scope_set=set(scope)
            def within(path):return any(path[:i] in scope_set for i in range(len(path)+1))
            coverage=[self.paths(c) for _ in range(3)]
            if coverage_mode==1 and any(coverage):raise Invalid('noncanonical completed scope coverage')
            if any(set(coverage[i]) & set(coverage[j]) for i in range(3) for j in range(i)):
                raise Invalid('overlapping coverage labels')
            if (coverage[1] or coverage[2]) and not partial:raise Invalid('unconfirmed partial observation')
            for paths in coverage:
                if any(not within(p) for p in paths):raise Invalid('coverage outside scope')
            n=c.number(4)
            if n>1000000:raise Invalid('resolution count')
            previous=None
            for _ in range(n):
                path=self.path(c)
                if previous is not None and path<=previous:raise Invalid('resolution order')
                if not within(path):raise Invalid('resolution outside scope')
                previous=path
                self.conflict(c.ref({K_CONFLICT}));self.maybe(c)
        else:raise Invalid('unknown provenance record')
        return tag

    def generic(self,ref:Ref)->None:
        if ref.kind==K_BLOB:
            self.fetch(ref)
            return
        if ref.kind==K_STATE:
            self.state(ref)
            return
        if ref.kind in (K_LEAF,K_BRANCH):
            self.map(ref,(),0,0)
            return
        if ref.kind==K_CONFLICT:
            self.conflict(ref)
            return
        c=Cursor(self.enter(ref))
        if ref.kind==K_REV:
            state=c.ref({K_STATE});self.state(state)
            n=c.number(4)
            if n>64:raise Invalid('revision parent limit')
            parents=[c.ref({K_REV}) for _ in range(n)]
            if len(set(parents))!=n:raise Invalid('duplicate parents')
            c.take(16)
            c.str(4096);c.str(1048576)
            c.take(8)
            if c.flag():
                prov=c.ref({K_PROV});self.generic(prov)
                if Cursor(self.fetch(prov)).ref({K_STATE})!=state:raise Invalid('provenance result mismatch')
            for p in parents:self.generic(p)
        elif ref.kind==K_PROV:
            if len(c.data)>1048576:raise Invalid('provenance payload size limit')
            self.state(c.ref({K_STATE}))
            n=c.number(4)
            if n>64:raise Invalid('provenance input budget')
            inputs=[c.ref({K_REV}) for _ in range(n)]
            if len(set(inputs))!=n:raise Invalid('duplicate provenance inputs')
            occurrences=c.number(4)
            if occurrences>1000000:raise Invalid('provenance count')
            previous=None
            for _ in range(occurrences):
                path=self.path(c)
                cf=c.ref({K_CONFLICT})
                if previous is not None and (path,cf.encode())<=previous:
                    raise Invalid('noncanonical provenance occurrence ordering')
                previous=(path,cf.encode())
                self.conflict(cf)
                roles=c.number(4)
                for _ in range(roles):
                    if c.number(4)>=len(inputs):raise Invalid('bad role')
            count=c.number(1)
            if count not in (1,2):raise Invalid('provenance record count')
            tags=[self.record(c,inputs) for _ in range(count)]
            if tags!=sorted(set(tags)) or (1 in tags and 2 in tags):raise Invalid('provenance record combination')
            self.provenance_variants[ref]=set(tags)
            for inp in inputs:self.generic(inp)
        else:raise Invalid('unknown kind')
        c.done();self.exit(ref)
