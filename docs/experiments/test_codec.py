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
                             K_LEAF,K_BRANCH,K_STATE,K_CONFLICT)
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
    prov=b.provenance(path_cf_state,[rev,rev_author],[((b'broken',),path_cf,[0,1])],
                      b.transform_record(empty_state,edits=[((b'broken',),None,b.conflict_entry(path_cf))]))
    move_prov=b.provenance(src_new,[],[],b.transform_record(src_old,[
        b.move_op((b'src',),(b'lib',),b.directory(child),None)]))
    observation=b.provenance(mixed,[rev],[],b.observation_record(
        mixed,rev,7,capture=b'C'*16,covered=[(p,) for p in (b'a',b'ab',b'foo',b'link',b'\xff')]))
    history_merge=b.provenance(mixed,[rev,rev_author],[],
                               b.merge_record(history=(0,1),allow_unrelated=True))
    source_merge=b.provenance(mixed,[],[],b.merge_record(source=(empty_state,mixed,mixed)))
    merge_rev=b.revision(path_cf_state,[rev,rev_author],b'Y'*16,b'author',b'merge',1234567890,prov)
    roots={
        'empty_map':empty,'empty_state':empty_state,'single_file':single,
        'mixed_types':mixed,'odd_nibble_branch':odd,'odd_nibble_state':odd_state,
        'terminal_branch':terminal,'terminal_state':terminal_state,
        'leaf_4095':near[8], 'leaf_4096':near[9], 'branch_4097':near[10],
        'moved_before':src_old,'moved_after':src_new,
        'revision':rev,'alternate_author_revision':rev_author,
        'path_conflict':path_cf,'path_conflict_state':path_cf_state,
        'root_coupled_conflict':root_cf,'root_conflict_state':conflicted_root,
        'provenance':prov,'merge_revision':merge_rev,'move_provenance':move_prov,
        'observation_provenance':observation,'history_merge_provenance':history_merge,
        'source_merge_provenance':source_merge
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

    def test_binary_conflicts_and_retired_tags(self):
        b=Builder()
        values=[b.file(b.blob(bytes([i]))) for i in range(3)]
        for alternatives in ([],values[:1],values,[values[0],values[0]]):
            with self.assertRaisesRegex(ValueError,'two distinct'):
                b.conflict(0,None,alternatives)
        ref=b.conflict(0,None,values[:2])
        self.assertEqual(ref,b.conflict(0,None,list(reversed(values[:2]))))
        # Truncation, a third term and retired experimental kinds are not extensions.
        cases=[(ref,True),
               (b.store_obj(K_CONFLICT,b.store[ref][:-1]),False),
               (b.store_obj(K_CONFLICT,b.store[ref]+b.maybe(values[2])),False),
               (b.store_obj(8,b'\x00'),False),(b.store_obj(9,b'\x00'),False)]
        self.check_admission(b,cases)

    def test_provenance_order_uses_raw_path_components(self):
        b=Builder()
        empty=b.state(b.directory(b.map({})))
        rev=b.revision(empty,[],bytes(16),b'a',b'd',0)
        conflict=b.conflict(0,None,[b.file(b.blob(b'a')),b.file(b.blob(b'b'))])
        # Include a path prefix and non-UTF8 bytes, not length-prefixed order.
        paths=[(),(b'aa',),(b'aa',b'x'),(b'z',),(b'\xff',)]
        def provenance(paths):
            return b.provenance(empty,[rev],[(p,conflict,[0]) for p in paths],b.transform_record(empty))
        self.check_admission(b,[(provenance(paths),True),
                                (provenance(list(reversed(paths))),False),
                                (provenance([(b'z',),(b'aa',)]),False)])

    def test_provenance_payload_size_limit(self):
        b=Builder()
        state=b.state(b.directory(b.map({})))
        rev=b.revision(state,[],bytes(16),b'a',b'd',0)
        oversized=b'x'*(1048576+1)
        provenance=b.provenance(state,[rev],[],b.transform_record(state)+oversized)
        self.check_admission(b,[(provenance,False)])

    def test_typed_provenance_admission_and_result_binding(self):
        b=Builder()
        state=b.state(b.directory(b.map({})))
        value=b.file(b.blob(b'a'))
        other=b.state(b.directory(b.map({b'a':value})))
        rev=b.revision(state,[],bytes(16),b'a',b'd',0)
        transform=b.provenance(other,[rev],[],b.transform_record(state,[
            b.copy_op(value,(b'a',),None)],[]))
        observe=b.provenance(state,[rev],[],b.observation_record(state,rev,covered=[()]))
        def obs(**kwargs):return b.provenance(state,[rev],[],b.observation_record(state,**kwargs))
        merge=b.provenance(other,[rev],[],b.merge_record(source=(state,state,other),moves=(transform,None)))
        unknown_algorithm=b.provenance(state,[rev],[],b.merge_record(
            history=(0,0),source_algorithm=(65535,99)))
        combined=b.provenance(state,[rev],[],b.transform_record(state),b.observation_record(state,rev))
        combined_merge=b.provenance(state,[rev],[],b.merge_record(history=(0,0)),b.observation_record(state,rev))
        # Audit IDs are not fetched: mismatched/unavailable claims are checked
        # only when explicit evidence is consumed, not by canonical decoding.
        invalid_binding=b.store_obj(7,b.store[transform][:33]+b'\0'+b.store[transform][34:])
        cases=[(invalid_binding,False),(transform,True),(observe,True),(merge,True),(combined,True),
               (combined_merge,True),(unknown_algorithm,True),
               (b.provenance(state,[],[],b.transform_record(state),b.merge_record(source=(state,state,state))),False),
               (b.provenance(state,[],[],b.observation_record(state),b.observation_record(state)),False),
               (b.provenance(state,[],[],b'\x04'),False),
               (b.provenance(state,[rev],[],b.merge_record(history=(0,1))),False),
               (b.provenance(state,[rev],[],b.merge_record(history=(0,0),moves=(observe,None))),True),
               (b.provenance(state,[rev],[],b.merge_record(history=(0,0),text_algorithm=(0,1))),False),
               (obs(failed=[(b'a',)]),False),(obs(failed=[(b'a',)],partial=True),True),
               (obs(coverage_mode=1),True),(obs(coverage_mode=1,covered=[()]),False),
               (obs(coverage_mode=1,failed=[(b'a',)],partial=True),False),
               (obs(coverage_mode=2),False),
               (obs(covered=[(b'a',)],failed=[(b'a',)],partial=True),False),
               (obs(scope=[(b'a',)],covered=[(b'b',)]),False),
               (obs(scope=[(),(b'a',)]),False),(obs(covered=[(b'z',),(b'aa',)]),False),
               (b.provenance(state,[],[],b.observation_record(other,rev)),True),
               (b.revision(state,[rev],b'B'*16,b'a',b'd',0,transform),False)]
        self.check_admission(b,cases)

    def test_private_amend_inputs_do_not_own_source_closure(self):
        b=Builder()
        empty=b.state(b.directory(b.map({})))
        parent=b.revision(empty,[],b'P'*16,b'a',b'base',0)
        secret=b.blob(b'private discarded bytes')
        old_map=b.map({b'credentials':b.file(secret)})
        old_state=b.state(b.directory(old_map))
        old=b.revision(old_state,[parent],b'C'*16,b'a',b'private draft',1)
        clean=b.state(b.directory(b.map({b'app':b.file(b.blob(b'public bytes'))})))
        prov=b.provenance(clean,[old],[],b.observation_record(old_state,old,coverage_mode=1))
        amended=b.revision(clean,[parent],b'C'*16,b'a',b'cleaned draft',2,prov)
        private={secret,old_map,old_state,old}
        source_only={ref:data for ref,data in b.store.items() if ref not in private}
        v=Verifier(source_only)
        v.generic(amended)
        self.assertTrue(v.closure.isdisjoint(private))
        # Keeping private evidence is a separate, explicit custody choice.
        audit=Verifier(b.store)
        audit.generic(amended)
        self.assertTrue(audit.closure.isdisjoint(private))
        audit.generic(old)
        self.assertIn(secret,audit.closure)
        self.check_admission(b,[(amended,True)])
        without=Builder();without.store=source_only
        self.check_admission(without,[(amended,True)])
        # Present but corrupt audit inputs must not affect source admission
        # either: decoding cannot depend on locally available private objects.
        corrupt=Builder();corrupt.store=dict(b.store)
        for ref in private:corrupt.store[ref]=b'not an audit dependency'
        self.check_admission(corrupt,[(amended,True)])

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
