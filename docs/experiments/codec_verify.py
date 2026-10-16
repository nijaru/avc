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
                             K_CONFLICT,K_PROV,K_GROUP,K_CANDIDATE)

MAX_EDGES=30000

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
    def str(self)->bytes:return self.take(self.number(4))
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

    def fetch(self,ref:Ref)->bytes:
        if ref not in self.objects:raise Invalid('missing required object')
        payload=self.objects[ref]
        if object_id(ref.kind,payload)!=ref:raise Invalid('content hash mismatch')
        return payload

    def enter(self,ref:Ref)->bytes:
        self.edges+=1
        if self.edges>MAX_EDGES:raise Invalid('expansion budget exceeded')
        if ref in self.active:raise Invalid('cycle')
        self.active.add(ref)
        return self.fetch(ref)

    def exit(self,ref:Ref):self.active.remove(ref)

    def entry(self,c:Cursor,depth:int=0, pathlen:int=0,conf_depth:int=0)->bytes:
        if depth>256 or pathlen>4096:raise Invalid('path constraints')
        start=c.pos
        tag=c.number(1)
        if tag in (1,2):
            r=c.ref({K_BLOB});self.generic(r)
            if tag==1:c.flag()
        elif tag==3:
            r=c.ref({K_LEAF,K_BRANCH})
            self.map(r,(),depth,pathlen)
        elif tag==4:
            self.conflict(c.ref({K_CONFLICT}),depth,pathlen,conf_depth+1)
        else:raise Invalid('unsupported entry type')
        return c.data[start:c.pos]

    def maybe(self,c:Cursor,depth:int=0,pathlen:int=0,conf_depth:int=0)->bytes:
        p=c.pos
        if c.flag():self.entry(c,depth,pathlen,conf_depth)
        return c.data[p:c.pos]

    def map(self,ref:Ref,prefix:tuple[int,...],depth:int,pathlen:int)->dict[bytes,bytes]:
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
                v=self.entry(c,depth+1,pathlen+(1 if depth else 0)+len(name))
                got[name]=v
        elif ref.kind==K_BRANCH:
            common=c.nibbles(c.number(2));full=prefix+common
            if c.flag():
                name=byte_name(full)
                v=self.entry(c,depth+1,pathlen+(1 if depth else 0)+len(name))
                got[name]=v
            bitmap=c.number(2)
            if bitmap==0:raise Invalid('branch without children')
            if (bool(got)+bitmap.bit_count())<2:raise Invalid('uncompressed branch')
            for slot in range(16):
                if (bitmap>>slot)&1:
                    child=c.ref({K_LEAF,K_BRANCH})
                    partial=self.map(child,full+(slot,),depth,pathlen)
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

    def group(self,ref:Ref,depth:int=0,pathlen:int=0,conf_depth:int=0)->list[bytes]:
        if ref.kind!=K_GROUP:raise Invalid('not group')
        c=Cursor(self.enter(ref))
        tag=c.number(1);count=c.number(4)
        if tag==0:
            if not 1<=count<=64:raise Invalid('leaf width')
            terms=[self.maybe(c,depth,pathlen,conf_depth) for _ in range(count)]
        elif tag==1:
            if not 2<=count<=64:raise Invalid('branch width')
            terms=[]
            for _ in range(count):terms+=self.group(c.ref({K_GROUP}),depth,pathlen,conf_depth)
        else:raise Invalid('group variant')
        c.done();self.exit(ref)
        if terms!=sorted(set(terms)):raise Invalid('group order or duplicate')
        return terms

    def conflict(self,ref:Ref,depth:int=0,pathlen:int=0,conf_depth:int=1,root:bool=False)->None:
        if conf_depth>64:raise Invalid('conflict nesting >64')
        c=Cursor(self.enter(ref))
        region=c.number(1)
        if region not in (0,1) or (root and region!=1):raise Invalid('conflict region/root kind')
        base=self.maybe(c,depth,pathlen,conf_depth)
        mode=c.number(1)
        if mode==0:
            n=c.number(4)
            if not 2<=n<=64:raise Invalid('conflict inline width')
            terms=[self.maybe(c,depth,pathlen,conf_depth) for _ in range(n)]
        elif mode==1:
            group_ref=c.ref({K_GROUP})
            terms=self.group(group_ref,depth,pathlen,conf_depth)
            if self._canonical_group(terms)!=group_ref:raise Invalid('noncanonical group blocks')
            if len(terms)<=64:raise Invalid('noncanonical conflict grouping')
        else:raise Invalid('conflict storage variant')
        c.done();self.exit(ref)
        if terms!=sorted(set(terms)):raise Invalid('noncanonical alternatives')
        if region==1:
            # Absence is a valid *non-root* coupled-subtree alternative.
            # A root conflict cannot contain absent alternatives.
            for t in terms:
                if t==b'\0' and not root:continue
                if not self._directory_term(t):
                    raise Invalid('coupled alternative not directory-valued')
            if base!=b'\0' and not self._directory_term(base):
                raise Invalid('coupled base not a directory')
        return region

    def _directory_term(self,t:bytes)->bool:
        if len(t)<2 or t[0]!=1:return False
        if t[1]==3:return True
        if t[1]==4 and len(t)==35:
            ref=Ref(K_CONFLICT,t[3:35])
            return self.fetch(ref)[:1]==b'\x01'
        return False

    @staticmethod
    def _canonical_group(terms:list[bytes])->Ref:
        # Independent canonical wide-alternative packing from decoded terms.
        level=[object_id(K_GROUP,b'\0'+u32(len(terms[j:j+64]))+b''.join(terms[j:j+64]))
               for j in range(0,len(terms),64)]
        while len(level)>1:
            level=[object_id(K_GROUP,b'\1'+u32(len(level[j:j+64]))+
                             b''.join(ref.encode() for ref in level[j:j+64]))
                   for j in range(0,len(level),64)]
        return level[0]

    def state(self,ref:Ref)->None:
        if ref.kind!=K_STATE:raise Invalid('not State')
        c=Cursor(self.enter(ref))
        start=c.pos
        tag=c.number(1)
        if tag==3:self.map(c.ref({K_LEAF,K_BRANCH}),(),0,0)
        elif tag==4:self.conflict(c.ref({K_CONFLICT}),0,0,1,root=True)
        else:raise Invalid('not valid state root')
        c.done();self.exit(ref)

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
        if ref.kind==K_GROUP:
            self.group(ref)
            return
        c=Cursor(self.enter(ref))
        if ref.kind==K_REV:
            self.state(c.ref({K_STATE}))
            n=c.number(4)
            if n>64:raise Invalid('revision parent limit')
            parents=[c.ref({K_REV}) for _ in range(n)]
            if len(set(parents))!=n:raise Invalid('duplicate parents')
            c.take(16)
            if len(c.str())>4096 or len(c.str())>1048576:raise Invalid('revision metadata bound')
            c.take(8)
            if c.flag():self.generic(c.ref({K_PROV}))
            for p in parents:self.generic(p)
        elif ref.kind==K_PROV:
            n=c.number(4)
            if n>64:raise Invalid('provenance input budget')
            inputs=[c.ref({K_REV}) for _ in range(n)]
            occurrences=c.number(4)
            if occurrences>1000000:raise Invalid('provenance count')
            previous=None
            for _ in range(occurrences):
                components=c.number(4)
                if components>256:raise Invalid('long path')
                path=tuple(c.str() for _ in range(components))
                if any(not name_valid(p) for p in path):raise Invalid('path part')
                if sum(map(len,path))+max(0,len(path)-1)>4096:raise Invalid('path length')
                cf=c.ref({K_CONFLICT})
                if previous is not None and (path,cf.encode())<=previous:
                    raise Invalid('noncanonical provenance occurrence ordering')
                previous=(path,cf.encode())
                self.conflict(cf)
                roles=c.number(4)
                for _ in range(roles):
                    if c.number(4)>=len(inputs):raise Invalid('bad role')
            c.str();c.number(4);c.str()
            for inp in inputs:self.generic(inp)
        elif ref.kind==K_CANDIDATE:
            c.str();c.str();gen=c.number(8)
            present=c.flag()
            obs=c.ref({K_REV}) if present else None
            if (gen==0)!= (obs is None):raise Invalid('generation/observed pair')
            n=c.number(4)
            if not 1<=n<=64:raise Invalid('candidate selection width')
            selected=[c.ref({K_REV}) for _ in range(n)]
            if len(set(selected))!=n:raise Invalid('duplicate selected')
            c.str();c.number(4);c.str()
            state=c.ref({K_STATE});rev=c.ref({K_REV})
            c.take(32);c.number(8)
            self.state(state);self.generic(rev)
            if obs:self.generic(obs)
            for x in selected:self.generic(x)
            # Source-state reference equality is checked after revision decode.
            rv=Cursor(self.fetch(rev))
            if rv.ref({K_STATE})!=state:raise Invalid('candidate result state mismatch')
        else:raise Invalid('unknown kind')
        c.done();self.exit(ref)
