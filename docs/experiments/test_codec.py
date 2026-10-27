"""Independent AVC canonical v0 qualification fixtures, NOT native tests.

Run: python -m unittest -v test_codec.py
Golden vectors are pinned in codec_vectors.json; regeneration is deliberate.
"""
from __future__ import annotations
import json
import hashlib
import random
import shutil
import subprocess
import unittest
from pathlib import Path
from codec_reference import (Builder,Ref,object_id,nibs,packed_nibbles,u16,u32,
                             K_LEAF,K_BRANCH,K_STATE,K_GROUP,K_CONFLICT)
from codec_verify import Verifier, Invalid, alternate_map
from blake3_reference import blake3, OFFICIAL_VECTORS

HERE=Path(__file__).resolve().parent


def build_samples():
    b=Builder()
    f=b.file(b.blob(b'hello\n'))
    f2=b.file(b.blob(b'hello\x00\xff\r\n'))
    run=b.file(b.blob(b'#!/bin/sh\necho hi\n'),True)
    sym=b.symlink(b.blob(b'../target\x00bytes'))
    empty=b.map({})
    empty_state=b.state(b.directory(empty))
    single=b.state(b.directory(b.map({b'foo':f})))
    mixed=b.state(b.directory(b.map({b'a':f,b'ab':run,b'foo':f2,b'link':sym,b'\xff':f})))
    names={b'a'+bytes([0x30+(i//16),0x30+(i%16)]):f for i in range(130)}
    # 130 children forces a branch whose maximal common prefix has odd nibble length.
    odd=b.map(names)
    odd_state=b.state(b.directory(odd))
    threshold={b'k'+f'{i:02d}'.encode():f for i in range(100)}
    threshold[b'long00002']=f
    near={}
    for length in (8,9,10):
        key=b'long'+b'0'*(length-5)+b'1'  # exactly length 8,9,10
        near[length]=b.map({**threshold,key:f})
        if near[length].kind==K_LEAF:
            assert len(b.store[near[length]])==4087+length
        else:
            assert 4087+length>4096
    terminal=b.map({b'a':f,**names})
    terminal_state=b.state(b.directory(terminal))
    child=b.map({b'file':run})
    src_old=b.state(b.directory(b.map({b'src':b.directory(child),b'untouched':f})))
    src_new=b.state(b.directory(b.map({b'lib':b.directory(child),b'untouched':f})))
    rev=b.revision(mixed,[],b'X'*16,b'author',b'change',123456789)
    rev_author=b.revision(mixed,[],b'X'*16,b'different',b'change',123456789)
    path_cf=b.conflict(0,f,[None,f2]);path_cf_state=b.state(b.directory(b.map({b'broken':b.conflict_entry(path_cf)})))
    left=b.directory(b.map({b'left':f,b'same':f}))
    right=b.directory(b.map({b'right':run,b'same':f}))
    root_cf=b.conflict(1,b.directory(empty),[left,right])
    conflicted_root=b.state(b.conflict_entry(root_cf))
    big_alts=[b.file(b.blob(f'file-{i:03}'.encode())) for i in range(65)]
    group_cf=b.conflict(0,None,big_alts)
    group_root=Ref(K_GROUP,b.store[group_cf][-32:])
    group_payload=b.store[group_root]
    group_leaf0=Ref(K_GROUP,group_payload[6:38])
    group_leaf1=Ref(K_GROUP,group_payload[39:71])
    group_state=b.state(b.directory(b.map({b'many':b.conflict_entry(group_cf)})))
    prov=b.provenance([rev,rev_author],[((b'broken',),path_cf,[0,1])],b'avc.text.lines',1,b'fixture')
    merge_rev=b.revision(path_cf_state,[rev,rev_author],b'Y'*16,b'author',b'merge',1234567890,prov)
    candidate=b.candidate(b'local',b'main',0,None,[merge_rev],b'merge-v0',1,b'fixture',
                          path_cf_state,merge_rev,b'P'*32,7)
    roots={
        'empty_map':empty,'empty_state':empty_state,'single_file':single,
        'mixed_types':mixed,'odd_nibble_branch':odd,'odd_nibble_state':odd_state,
        'terminal_branch':terminal,'terminal_state':terminal_state,
        'leaf_4095':near[8], 'leaf_4096':near[9], 'branch_4097':near[10],
        'moved_before':src_old,'moved_after':src_new,
        'revision':rev,'alternate_author_revision':rev_author,
        'path_conflict':path_cf,'path_conflict_state':path_cf_state,
        'root_coupled_conflict':root_cf,'root_conflict_state':conflicted_root,
        'overflow_group_conflict':group_cf,'overflow_group_state':group_state,
        'overflow_group_branch':group_root,'overflow_group_leaf0':group_leaf0,
        'overflow_group_leaf1':group_leaf1,
        'provenance':prov,'merge_revision':merge_rev,'publication_candidate':candidate
    }
    return b,roots


def render_vectors(b,roots):
    out={}
    for label,ref in roots.items():
        payload=b.store[ref]
        vector={'kind':ref.kind,'id':ref.display(),'payload_len':len(payload)}
        if len(payload)<=256:
            vector['payload_hex']=payload.hex()
        else:
            vector['payload_sha256']=hashlib.sha256(payload).hexdigest()
            vector['payload_prefix_hex']=payload[:24].hex()
        out[label]=vector
    return out


class CanonicalQualification(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.builder,cls.roots=build_samples()

    def test_official_blake3_known_answers(self):
        for n,want in OFFICIAL_VECTORS.items():
            with self.subTest(length=n):
                self.assertEqual(blake3(bytes(i%251 for i in range(n))).hex(),want)
        self.assertEqual(blake3(b'abc').hex(),
                         '6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85')

    def test_pinned_avc_golden_vectors(self):
        want=json.loads((HERE/'codec_vectors.json').read_text())
        self.assertEqual(render_vectors(self.builder,self.roots),want)

    def test_all_canonical_object_roots_parse(self):
        v=Verifier(self.builder.store)
        for name,ref in self.roots.items():
            with self.subTest(name=name):
                if ref.kind==K_STATE:v.state(ref)
                elif ref.kind in (K_LEAF,K_BRANCH):v.map(ref,(),0,0)
                else:v.generic(ref)

    def test_object_id_spelling(self):
        for ref in self.roots.values():
            self.assertEqual(Ref.parse(ref.display()),ref)
            with self.assertRaises(ValueError):Ref.parse(ref.display().upper())
        # 52 base32 chars encode 260 bits; the final four bits must be zero.
        s=self.roots['empty_map'].display()
        bad=s[:-1]+('b' if s[-1]!='b' else 'c')
        with self.assertRaises(ValueError):Ref.parse(bad)

    def test_flat_semantics_independent_radix(self):
        entry=self.builder.file(self.builder.blob(b'tiny'))
        rnd=random.Random(817)
        for size in (0,1,10,95,96,99,100,103,104,130,260):
            values={b'x'+f'{i:03d}'.encode():entry for i in range(size)}
            a=self.builder.map(values)
            alt,_=alternate_map(values)
            self.assertEqual(a,alt)
            Verifier(self.builder.store).map(a,(),0,0)
            for _ in range(3):
                names=list(values)
                rnd.shuffle(names)
                ordered={k:values[k] for k in names}
                self.assertEqual(self.builder.map(ordered),a)

    def test_threshold_and_prefix_cases(self):
        empty=self.roots['empty_map']
        self.assertEqual(self.builder.store[empty],b'\x00'*4)
        self.assertEqual(self.roots['odd_nibble_branch'].kind,K_BRANCH)
        self.assertEqual(self.roots['terminal_branch'].kind,K_BRANCH)
        self.assertEqual(self.builder.store[self.roots['odd_nibble_branch']][:2],u16(3))
        names=Verifier(self.builder.store).map(self.roots['terminal_branch'],(),0,0)
        self.assertIn(b'a',names)
        self.assertIn(b'a00',names)

    def test_exact_leaf_boundary(self):
        for size,label in ((4095,'leaf_4095'),(4096,'leaf_4096')):
            ref=self.roots[label]
            self.assertEqual(ref.kind,K_LEAF)
            self.assertEqual(len(self.builder.store[ref]),size)
        ref=self.roots['branch_4097']
        self.assertEqual(ref.kind,K_BRANCH)
        self.assertEqual(len(Verifier(self.builder.store).map(ref,(),0,0)),102)

    @unittest.skipUnless(shutil.which('node'), 'Node.js cross-language fixture unavailable')
    def test_independent_javascript_radix_randomized(self):
        rng=random.Random(20261016)
        groups=[]
        for count in (0,1,2,15,63,64,94,95,96,97,102,103,128,255,260,400):
            pool={}
            while len(pool)<count:
                # Include prefixes, byte-non-UTF8 names, varied lengths and
                # dense same-prefix keys near 4096-byte leaf split.
                prefix=rng.choice([b'a',b'abc',b'k',b'\xff',b'xyz',b'q'*40])
                suffix=bytes(rng.choice(list(range(1,47))+list(range(48,255)))
                             for _ in range(rng.randrange(0,6)))
                key=prefix+suffix
                if len(key)<=255 and key not in (b'.',b'..') and b'\x00' not in key and b'/' not in key:
                    pool[key]=None
            group=list(pool)
            groups.append(group)
            rng.shuffle(group)
            groups.append(group)
        b=Builder()
        value=b.file(b.blob(b'fixture'))
        expected=[b.map({name:value for name in group}).display() for group in groups]
        result=subprocess.run(['node',str(HERE/'codec_crosscheck.mjs'),'--maps-stdin'],
                              input=json.dumps([[name.hex() for name in group] for group in groups]),
                              text=True,capture_output=True,check=True,timeout=35)
        self.assertEqual(json.loads(result.stdout),expected)

    def test_coupled_deletion_and_root_constraint(self):
        b=Builder()
        empty=b.directory(b.map({}))
        sub=b.conflict(1,empty,[None,empty])
        Verifier(b.store).conflict(sub)
        bad_root=b.state(b.conflict_entry(sub))
        with self.assertRaisesRegex(Invalid,'absent directory alternative'):
            Verifier(b.store).state(bad_root)
        path=b.conflict(0,None,[None,empty])
        bad_path_root=b.state(b.conflict_entry(path))
        with self.assertRaisesRegex(Invalid,'root kind'):
            Verifier(b.store).state(bad_path_root)

    def test_nested_coupled_conflict_cannot_hide_deletion_or_file(self):
        b=Builder()
        directory=b.directory(b.map({}))
        other=b.directory(b.map({b'visible':b.file(b.blob(b'a'))}))
        hidden_deletion=b.conflict(1,directory,[None,directory])
        outer=b.conflict(1,directory,[b.conflict_entry(hidden_deletion),other])
        with self.assertRaisesRegex(Invalid,'absent directory alternative'):
            Verifier(b.store).state(b.state(b.conflict_entry(outer)))
        with self.assertRaisesRegex(Invalid,'absent directory alternative'):
            Verifier(b.store).conflict(outer)
        hidden_file=b.conflict(0,None,[b.file(b.blob(b'bad')),directory])
        outer_file=b.conflict(1,directory,[b.conflict_entry(hidden_file),other])
        with self.assertRaisesRegex(Invalid,'conflict region/root kind'):
            Verifier(b.store).state(b.state(b.conflict_entry(outer_file)))
        nested_base=b.conflict(1,b.conflict_entry(hidden_deletion),[directory,other])
        with self.assertRaisesRegex(Invalid,'absent directory alternative'):
            Verifier(b.store).state(b.state(b.conflict_entry(nested_base)))
        # A non-root coupled conflict may itself represent deletion.
        Verifier(b.store).conflict(hidden_deletion)

    def test_grouped_root_coupled_alternatives_must_all_be_directories(self):
        b=Builder()
        base=b.directory(b.map({}))
        file=b.file(b.blob(b'one'))
        dirs=[b.directory(b.map({f'file{i:03}'.encode():file})) for i in range(65)]
        invalid=b.conflict(1,base,[None,*dirs])
        with self.assertRaisesRegex(Invalid,'absent directory alternative'):
            Verifier(b.store).state(b.state(b.conflict_entry(invalid)))
        # Non-root deletion is an explicit valid semantic term even when grouped.
        Verifier(b.store).conflict(invalid)

    def test_mutated_hash_valid_input_never_crashes_decoder(self):
        rng=random.Random(643024)
        eligible=[(ref,payload) for ref,payload in self.builder.store.items()
                  if ref.kind not in (1,) and payload]
        for i in range(200):
            ref,payload=rng.choice(eligible)
            candidate=bytearray(payload)
            pos=rng.randrange(len(candidate))
            candidate[pos]^=1<<rng.randrange(8)
            candidate=bytes(candidate)
            corrupted=object_id(ref.kind,candidate)
            store={**self.builder.store,corrupted:candidate}
            try:
                Verifier(store).generic(corrupted)
            except Invalid:
                pass
            except Exception as e:
                self.fail(f'mutant {i} kind {ref.kind} caused {type(e).__name__}: {e}')

    def test_conflict_depth_limit(self):
        b=Builder()
        f=b.file(b.blob(b'constant'))
        current=b.conflict(0,None,[None,f])
        for depth in range(2,66):
            current=b.conflict(0,None,[f,b.conflict_entry(current)])
            if depth==64:Verifier(b.store).conflict(current)
        with self.assertRaisesRegex(Invalid,'nesting'):
            Verifier(b.store).conflict(current)

    def test_group_block_normalization(self):
        b=Builder()
        entries=[b.file(b.blob(f'{i:03d}'.encode())) for i in range(65)]
        terms=sorted(b.maybe(x) for x in entries)
        bad0=b.store_obj(K_GROUP,b'\0'+u32(63)+b''.join(terms[:63]))
        bad1=b.store_obj(K_GROUP,b'\0'+u32(2)+b''.join(terms[63:]))
        badroot=b.store_obj(K_GROUP,b'\x01'+u32(2)+bad0.encode()+bad1.encode())
        badconf=b.store_obj(K_CONFLICT,b'\0\0\x01'+badroot.encode())
        with self.assertRaisesRegex(Invalid,'group blocks'):
            Verifier(b.store).conflict(badconf)

    def check_admission(self, builder, cases):
        """The wire corpus lets the other runtime judge the same exact objects."""
        expected=[]
        for ref, accepted in cases:
            if accepted:
                Verifier(builder.store).generic(ref)
            else:
                with self.assertRaises(Invalid):
                    Verifier(builder.store).generic(ref)
            expected.append(accepted)
        if shutil.which('node'):
            corpus={
                'objects':[{'kind':ref.kind,'payload_hex':payload.hex()}
                           for ref,payload in builder.store.items()],
                'roots':[ref.encode().hex() for ref,_ in cases],
            }
            result=subprocess.run(['node',str(HERE/'codec_crosscheck.mjs'),'--objects-stdin'],
                                  input=json.dumps(corpus),text=True,capture_output=True,
                                  check=True,timeout=35)
            self.assertEqual(json.loads(result.stdout),expected)

    def test_conflict_depth_survives_directory_boundaries(self):
        b=Builder()
        empty=b.directory(b.map({}))
        value=empty
        cases=[]
        for depth in range(1,71):
            other=b.directory(b.map({b'x':value}))
            value=b.conflict_entry(b.conflict(1,None,[empty,other]))
            if depth in (64,65,70):
                cases.append((b.state(value),depth<=64))
        self.check_admission(b,cases)

    def test_group_height_boundary_collapses_singleton_branches(self):
        b=Builder()
        values=[b.file(b.blob(i.to_bytes(4,'little'))) for i in range(4097)]
        cases=[(b.conflict(0,None,values[:n]),True) for n in (4096,4097)]
        self.check_admission(b,cases)
        for ref,payload in b.store.items():
            if ref.kind==K_GROUP and payload[0]==1:
                self.assertGreaterEqual(int.from_bytes(payload[1:5],'little'),2)

    def test_provenance_order_uses_raw_path_components(self):
        b=Builder()
        empty=b.state(b.directory(b.map({})))
        rev=b.revision(empty,[],bytes(16),b'a',b'd',0)
        conflict=b.conflict(0,None,[b.file(b.blob(b'a')),b.file(b.blob(b'b'))])
        # Include a path prefix and non-UTF8 bytes, not length-prefixed order.
        paths=[(),(b'aa',),(b'aa',b'x'),(b'z',),(b'\xff',)]
        def provenance(paths):
            return b.provenance([rev],[(p,conflict,[0]) for p in paths],b'test',1,b'')
        self.check_admission(b,[(provenance(paths),True),
                                (provenance(list(reversed(paths))),False),
                                (provenance([(b'z',),(b'aa',)]),False)])

    def test_recipe_payload_size_limit(self):
        b=Builder()
        state=b.state(b.directory(b.map({})))
        rev=b.revision(state,[],bytes(16),b'a',b'd',0)
        oversized=b'x'*(1048576+1)
        provenance=b.provenance([rev],[],b'test',1,oversized)
        candidate=b.candidate(b'local',b'main',0,None,[rev],b'test',1,oversized,
                              state,rev,b'p'*32,0)
        self.check_admission(b,[(provenance,False),(candidate,False)])

    def test_deep_history_has_an_explicit_reference_resource_refusal(self):
        b=Builder()
        state=b.state(b.directory(b.map({})))
        parent=[]
        for _ in range(1200):
            rev=b.revision(state,parent,bytes(16),b'a',b'd',0)
            parent=[rev]
        with self.assertRaisesRegex(Invalid,'traversal depth budget'):
            Verifier(b.store).generic(rev)
        self.check_admission(b,[(rev,False)])

    def test_directory_subtree_reuse(self):
        # Copies/moves reuse identical nested object IDs but produce different State.
        self.assertNotEqual(self.roots['moved_before'],self.roots['moved_after'])
        src=self.builder.store[self.roots['moved_before']]
        dst=self.builder.store[self.roots['moved_after']]
        self.assertNotEqual(src,dst)

    def test_conflict_values_have_author_independent_ids(self):
        self.assertNotEqual(self.roots['revision'],self.roots['alternate_author_revision'])
        self.assertEqual(self.builder.store[self.roots['root_coupled_conflict']][0],1)
        self.assertEqual(self.builder.store[self.roots['overflow_group_conflict']][-33],K_GROUP)

    def test_reject_noncanonical_and_corrupt(self):
        base=self.builder.file(self.builder.blob(b'K'))
        valid=self.builder.map({b'a':base,b'b':base})
        payload=self.builder.store[valid]
        def invalid_node(kind,data,checker):
            ref=object_id(kind,data)
            store={**self.builder.store,ref:data}
            with self.assertRaises(Invalid):checker(Verifier(store),ref)
        invalid_node(K_LEAF,payload+b'\0',lambda v,r:v.map(r,(),0,0))
        # Duplicate and unsorted records are hash-valid yet noncanonical.
        rec1=u16(2)+b'a'+base
        rec2=u16(2)+b'b'+base
        invalid_node(K_LEAF,u32(2)+rec2+rec1,lambda v,r:v.map(r,(),0,0))
        invalid_node(K_LEAF,u32(2)+rec1+rec1,lambda v,r:v.map(r,(),0,0))
        # Alternate encoding of a small map as a branch must be refused.
        child_payload=u32(1)+u16(0)+base
        child=object_id(K_LEAF,child_payload)
        branch=u16(1)+b'\x60'+b'\x00'+u16(0x0006)+child.encode()*2
        branch_ref=object_id(K_BRANCH,branch)
        # Both children exist and verify; only noncanonical shape is wrong.
        underfull={**self.builder.store,child:child_payload,branch_ref:branch}
        with self.assertRaisesRegex(Invalid,'noncanonical map'):
            Verifier(underfull).map(branch_ref,(),0,0)
        # Invalid executable bool even with a *matching* outer object ID.
        invalid_node(K_LEAF,u32(1)+u16(2)+b'a'+base[:-1]+b'\x02',lambda v,r:v.map(r,(),0,0))
        # A file cannot be the State root.
        invalid_node(K_STATE,base,lambda v,r:v.state(r))
        # A correctly-hashed leaf which references missing content cannot be admitted.
        missing=b'\x01'+Ref(1,b'F'*32).encode()+b'\x00'
        invalid_node(K_LEAF,u32(1)+u16(2)+b'a'+missing,lambda v,r:v.map(r,(),0,0))
        # Odd trailing nibble must be zero even when a parent prefix would complete it.
        invalid_node(K_LEAF,u32(1)+u16(1)+b'\xf1'+base,
                     lambda v,r:v.map(r,(6,1,3),0,0))
        # Hash doesn't match content: reject before decoding.
        store={**self.builder.store,valid:payload+b'\xff'}
        with self.assertRaises(Invalid):Verifier(store).map(valid,(),0,0)


if __name__=='__main__':unittest.main()
