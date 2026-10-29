/**
 * Independent Node.js v0 canonical-format qualification, NOT a runtime codec.
 * No Python modules, third-party dependencies, Git objects, or Rust engine.
 * Run: node docs/experiments/codec_crosscheck.mjs
 * BLAKE3 independently implemented from https://github.com/BLAKE3-team/BLAKE3-specs
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

const IV = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
  0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
const PERM = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];
const rot = (v, n) => (v >>> n) | (v << (32 - n));

function compress(cv, block, counter, size, flags) {
  let s = [...cv, ...IV.slice(0, 4), counter >>> 0,
    Math.floor(counter / 4294967296) >>> 0, size, flags];
  let m = Array.from({ length: 16 }, (_, i) => block.readUInt32LE(i * 4));
  const mix = (a, b, c, d, x, y) => {
    s[a] = (s[a] + s[b] + x) >>> 0;
    s[d] = rot(s[d] ^ s[a], 16);
    s[c] = (s[c] + s[d]) >>> 0;
    s[b] = rot(s[b] ^ s[c], 12);
    s[a] = (s[a] + s[b] + y) >>> 0;
    s[d] = rot(s[d] ^ s[a], 8);
    s[c] = (s[c] + s[d]) >>> 0;
    s[b] = rot(s[b] ^ s[c], 7);
  };
  for (let r = 0; r < 7; r++) {
    mix(0, 4, 8, 12, m[0], m[1]);
    mix(1, 5, 9, 13, m[2], m[3]);
    mix(2, 6, 10, 14, m[4], m[5]);
    mix(3, 7, 11, 15, m[6], m[7]);
    mix(0, 5, 10, 15, m[8], m[9]);
    mix(1, 6, 11, 12, m[10], m[11]);
    mix(2, 7, 8, 13, m[12], m[13]);
    mix(3, 4, 9, 14, m[14], m[15]);
    m = PERM.map(i => m[i]);
  }
  return [...Array.from({ length: 8 }, (_, i) => (s[i] ^ s[i + 8]) >>> 0),
    ...Array.from({ length: 8 }, (_, i) => (s[i + 8] ^ cv[i]) >>> 0)];
}
function output(cv, block, counter, len, flags) {
  return {
    chaining: () => compress(cv, block, counter, len, flags).slice(0, 8),
    root: () => {
      const words = compress(cv, block, 0, len, flags | 8);
      const b = Buffer.alloc(32);
      words.slice(0, 8).forEach((w, i) => b.writeUInt32LE(w >>> 0, 4 * i));
      return b;
    },
  };
}
function chunk(bytes, position) {
  let cv = IV;
  let last;
  const blocks = Math.max(1, Math.ceil(bytes.length / 64));
  for (let i = 0; i < blocks; i++) {
    const data = bytes.subarray(i * 64, (i + 1) * 64);
    const block = Buffer.alloc(64);
    data.copy(block);
    last = output(cv, block, position, data.length,
      (i === 0 ? 1 : 0) | (i === blocks - 1 ? 2 : 0));
    if (i < blocks - 1) cv = last.chaining();
  }
  return last;
}
function parent(a, b) {
  const words = [...a.chaining(), ...b.chaining()];
  const block = Buffer.alloc(64);
  words.forEach((v, i) => block.writeUInt32LE(v >>> 0, i * 4));
  return output(IV, block, 0, 64, 4);
}
export function blake3(bytes) {
  const nodes = [];
  for (let i = 0; i < bytes.length; i += 1024) nodes.push(chunk(bytes.subarray(i, i + 1024), i / 1024));
  if (!nodes.length) nodes.push(chunk(Buffer.alloc(0), 0));
  const reduce = list => {
    if (list.length === 1) return list[0];
    const split = 2 ** Math.floor(Math.log2(list.length - 1));
    return parent(reduce(list.slice(0, split)), reduce(list.slice(split)));
  };
  return reduce(nodes).root();
}
const bytes = (...v) => Buffer.concat(v);
const b8 = n => Buffer.from([n]);
const b16 = n => { const b = Buffer.alloc(2); b.writeUInt16LE(n); return b; };
const b32 = n => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const b64 = n => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
const i64 = n => { const b = Buffer.alloc(8); b.writeBigInt64LE(BigInt(n)); return b; };
const string = b => bytes(b32(b.length), b);
const magic = Buffer.from('AVC-OBJECT\x00v1\x00', 'binary');
const wire = r => bytes(b8(r.kind), r.digest);
function identify(kind, data) {
  assert(Number.isInteger(kind) && kind >= 1 && kind <= 255);
  const digest = blake3(bytes(magic, b8(kind), b64(data.length), data));
  return { kind, digest, key: `${kind}:${digest.toString('hex')}` };
}
function display(r) {
  const alphabet = 'abcdefghijklmnopqrstuvwxyz234567';
  let bits = 0, pending = 0, text = '';
  for (const v of r.digest) {
    pending = (pending << 8) | v;
    bits += 8;
    while (bits >= 5) { bits -= 5; text += alphabet[(pending >>> bits) & 31]; }
    pending &= (1 << bits) - 1;
  }
  if (bits) text += alphabet[(pending << (5 - bits)) & 31];
  return `avc1-b3-k${r.kind.toString(16).padStart(2, '0')}-${text}`;
}
function nibs(b) { return Array.from(b, x => [x >>> 4, x & 15]).flat(); }
function packed(n) {
  const b = Buffer.alloc(Math.ceil(n.length / 2));
  for (let i = 0; i < n.length; i++) b[i >>> 1] |= n[i] << (i % 2 ? 0 : 4);
  return b;
}
function validName(b) {
  return b.length > 0 && b.length <= 255 && ![Buffer.from('.'), Buffer.from('..')].some(x => x.equals(b))
    && !b.includes(0) && !b.includes(47);
}
const compare = (a, b) => Buffer.compare(a, b);
function comparePaths(a, b) {
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    const order = compare(a[i], b[i]);
    if (order) return order;
  }
  return a.length - b.length;
}
class Builder {
  constructor() { this.store = new Map(); }
  put(k, data) {
    const ref = identify(k, data);
    if (this.store.has(ref.key)) assert(this.store.get(ref.key).data.equals(data));
    else this.store.set(ref.key, { ref, data });
    return ref;
  }
  blob(v) { return this.put(1, v); }
  file(ref, x = false) { assert.equal(ref.kind, 1); return bytes(b8(1), wire(ref), b8(+x)); }
  symlink(ref) { assert.equal(ref.kind, 1); return bytes(b8(2), wire(ref)); }
  directory(ref) { assert([2, 3].includes(ref.kind)); return bytes(b8(3), wire(ref)); }
  conflictEntry(ref) { assert.equal(ref.kind, 6); return bytes(b8(4), wire(ref)); }
  maybe(entry) { return entry === null ? b8(0) : bytes(b8(1), entry); }
  map(pairs) {
    const ordered = [...pairs].sort(([a], [b]) => compare(a, b));
    assert(ordered.every(([n]) => validName(n)));
    assert(ordered.every(([n], i) => i === 0 || !n.equals(ordered[i - 1][0])));
    return this.radix(ordered.map(([n, v]) => [nibs(n), v]));
  }
  radix(rows) {
    const leaf = bytes(b32(rows.length), ...rows.map(([key, v]) => bytes(b16(key.length), packed(key), v)));
    if (leaf.length <= 4096) return this.put(2, leaf);
    assert(rows.length >= 2);
    const first = rows[0][0], last = rows[rows.length - 1][0];
    let shared = 0;
    while (shared < Math.min(first.length, last.length) && first[shared] === last[shared]) shared++;
    const groups = new Map();
    let terminal = null;
    for (const [key, entry] of rows) {
      if (key.length === shared) terminal = entry;
      else {
        const slot = key[shared];
        if (!groups.has(slot)) groups.set(slot, []);
        groups.get(slot).push([key.slice(shared + 1), entry]);
      }
    }
    assert(groups.size + +Boolean(terminal) >= 2);
    const bitmap = [...groups.keys()].reduce((x, slot) => x | (1 << slot), 0);
    return this.put(3, bytes(b16(shared), packed(first.slice(0, shared)), b8(+Boolean(terminal)),
      ...(terminal ? [terminal] : []), b16(bitmap),
      ...[...groups.keys()].sort((a, b) => a - b).map(i => wire(this.radix(groups.get(i))))));
  }
  state(root) { assert([3, 4].includes(root[0])); return this.put(4, root); }
  revision(state, parents, change, author, description, ns, prov = null) {
    assert.equal(change.length, 16);
    return this.put(5, bytes(wire(state), b32(parents.length), ...parents.map(wire), change,
      string(author), string(description), i64(ns), b8(+(prov !== null)), ...(prov ? [wire(prov)] : [])));
  }
  conflict(kind, base, alternatives) {
    const uniq = new Map(alternatives.map(x => [this.maybe(x).toString('hex'), this.maybe(x)]));
    const terms = [...uniq.values()].sort(compare);
    assert.equal(alternatives.length, 2);
    assert.equal(terms.length, 2);
    return this.put(6, bytes(b8(kind), this.maybe(base), ...terms));
  }
  path(parts) { return bytes(b32(parts.length), ...parts.map(string)); }
  moveOp(src, dst, expected, destination) {
    return bytes(b8(1), this.path(src), this.path(dst), expected, this.maybe(destination));
  }
  copyOp(value, dst, destination) {
    return bytes(b8(2), value, this.path(dst), this.maybe(destination));
  }
  transform(base, structural = [], edits = []) {
    return bytes(b8(1), wire(base), b32(structural.length), ...structural, b32(edits.length),
      ...edits.map(([p, old, value]) => bytes(this.path(p), this.maybe(old), this.maybe(value))));
  }
  observation(base, revision = null, generation = 0, consistency = 0, capture = null,
    partial = false, scope = [[]], covered = [], failed = [], skipped = [], resolutions = [], coverageMode = 0) {
    return bytes(b8(3), wire(base), b8(+(revision !== null)), ...(revision ? [wire(revision)] : []),
      b64(generation), b8(consistency), b8(+(capture !== null)), ...(capture ? [capture] : []), b8(+partial), b8(coverageMode),
      ...[scope, covered, failed, skipped].map(paths => bytes(b32(paths.length), ...paths.map(p => this.path(p)))),
      b32(resolutions.length), ...resolutions.map(([p, conflict, value]) =>
        bytes(this.path(p), wire(conflict), this.maybe(value))));
  }
  merge({ history = null, source = null, unrelated = false, sourceAlgorithm = [1, 1],
    textAlgorithm = [1, 1], moves = [null, null] }) {
    assert((history === null) !== (source === null));
    return bytes(b8(2), b8(+(history === null)),
      ...(history ? [b32(history[0]), b32(history[1]), b8(+unrelated)] : source.map(wire)),
      ...[sourceAlgorithm, textAlgorithm].map(([tag, version]) => bytes(b16(tag), b32(version))),
      ...moves.map(ref => bytes(b8(+(ref !== null)), ...(ref ? [wire(ref)] : []))));
  }
  provenance(result, inputs, occurrences, record, observation = null) {
    return this.put(7, bytes(wire(result), b8(1), b32(inputs.length), ...inputs.map(wire), b32(occurrences.length),
      ...occurrences.map(([path, ref, roles]) => bytes(this.path(path), wire(ref),
        b32(roles.length), ...roles.map(b32))), b8(observation ? 2 : 1), record,
      ...(observation ? [observation] : [])));
  }
}
const B = (s) => Buffer.from(s, 'utf8');
function samples() {
  const b = new Builder();
  const f = b.file(b.blob(B('hello\n')));
  const f2 = b.file(b.blob(Buffer.from('68656c6c6f00ff0d0a', 'hex')));
  const run = b.file(b.blob(B('#!/bin/sh\necho hi\n')), true);
  const sym = b.symlink(b.blob(Buffer.from('2e2e2f746172676574006279746573', 'hex')));
  const empty = b.map([]), emptyState = b.state(b.directory(empty));
  const single = b.state(b.directory(b.map([[B('foo'), f]])));
  const mixed = b.state(b.directory(b.map([[B('a'), f], [B('ab'), run], [B('foo'), f2],
    [B('link'), sym], [Buffer.from('ff', 'hex'), f]])));
  const names = [];
  for (let i = 0; i < 130; i++) names.push([Buffer.from([0x61, 0x30 + Math.floor(i / 16), 0x30 + i % 16]), f]);
  const odd = b.map(names), oddState = b.state(b.directory(odd));
  const threshold = [];
  for (let i = 0; i < 100; i++) threshold.push([B('k' + String(i).padStart(2, '0')), f]);
  threshold.push([B('long00002'), f]);
  const near = {};
  for (const len of [8, 9, 10]) {
    const key = B('long' + '0'.repeat(len - 5) + '1');
    near[len] = b.map([...threshold, [key, f]]);
  }
  const terminal = b.map([[B('a'), f], ...names]);
  const terminalState = b.state(b.directory(terminal));
  const child = b.map([[B('file'), run]]);
  const old = b.state(b.directory(b.map([[B('src'), b.directory(child)], [B('untouched'), f]])));
  const now = b.state(b.directory(b.map([[B('lib'), b.directory(child)], [B('untouched'), f]])));
  const rev = b.revision(mixed, [], B('X'.repeat(16)), B('author'), B('change'), 123456789);
  const revAuthor = b.revision(mixed, [], B('X'.repeat(16)), B('different'), B('change'), 123456789);
  const pathCf = b.conflict(0, f, [null, f2]);
  const pathCfState = b.state(b.directory(b.map([[B('broken'), b.conflictEntry(pathCf)]])));
  const left = b.directory(b.map([[B('left'), f], [B('same'), f]]));
  const right = b.directory(b.map([[B('right'), run], [B('same'), f]]));
  const rootCf = b.conflict(1, b.directory(empty), [left, right]);
  const rootState = b.state(b.conflictEntry(rootCf));
  const prov = b.provenance(pathCfState, [rev, revAuthor], [[[B('broken')], pathCf, [0, 1]]],
    b.transform(emptyState, [], [[[B('broken')], null, b.conflictEntry(pathCf)]]));
  const moveProv = b.provenance(now, [], [], b.transform(old, [
    b.moveOp([B('src')], [B('lib')], b.directory(child), null)]));
  const observation = b.provenance(mixed, [rev], [], b.observation(mixed, rev, 7, 0, B('C'.repeat(16)),
    false, [[]], ['a', 'ab', 'foo', 'link'].map(x => [B(x)]).concat([[Buffer.from('ff', 'hex')]])));
  const historyMerge = b.provenance(mixed, [rev, revAuthor], [], b.merge({ history: [0, 1], unrelated: true }));
  const sourceMerge = b.provenance(mixed, [], [], b.merge({ source: [emptyState, mixed, mixed] }));
  const mergeRev = b.revision(pathCfState, [rev, revAuthor], B('Y'.repeat(16)), B('author'), B('merge'), 1234567890, prov);
  return [b, {
    empty_map: empty, empty_state: emptyState, single_file: single, mixed_types: mixed,
    odd_nibble_branch: odd, odd_nibble_state: oddState, terminal_branch: terminal,
    terminal_state: terminalState, leaf_4095: near[8], leaf_4096: near[9], branch_4097: near[10],
    moved_before: old, moved_after: now, revision: rev, alternate_author_revision: revAuthor,
    path_conflict: pathCf, path_conflict_state: pathCfState, root_coupled_conflict: rootCf,
    root_conflict_state: rootState, provenance: prov, merge_revision: mergeRev,
    move_provenance: moveProv, observation_provenance: observation,
    history_merge_provenance: historyMerge, source_merge_provenance: sourceMerge,
  }];
}

// Separately structured decoder: untrusted data must be checked beyond its hash.
class Reader {
  constructor(data) { this.data = data; this.pos = 0; this.commitments = new Set(); }
  take(n) {
    if (!Number.isSafeInteger(n) || n < 0 || n > this.data.length - this.pos)
      throw new Error('invalid/truncated length');
    const b = this.data.subarray(this.pos, this.pos + n);
    this.pos += n;
    return b;
  }
  u8() { return this.take(1)[0]; }
  u16() { return this.take(2).readUInt16LE(); }
  u32() { return this.take(4).readUInt32LE(); }
  u64() { return this.take(8).readBigUInt64LE(); }
  flag() { const n = this.u8(); if (n > 1) throw new Error('invalid bool'); return n === 1; }
  str(limit = Infinity) {
    const size = this.u32();
    if (size > limit) throw new Error('byte-string size limit');
    return this.take(size);
  }
  ref(kinds) {
    const kind = this.u8(), digest = this.take(32);
    if (!kinds.includes(kind)) throw new Error('invalid reference kind');
    return { kind, digest, key: `${kind}:${digest.toString('hex')}` };
  }
  commitment(kinds) {
    const ref = this.ref(kinds); this.commitments.add(ref.key); return ref;
  }
  nibbles() {
    const n = this.u16(), b = this.take(Math.ceil(n / 2));
    if (n % 2 && (b[b.length - 1] & 15)) throw new Error('nibble padding');
    return nibs(b).slice(0, n);
  }
  finish() { if (this.pos !== this.data.length) throw new Error('trailing bytes'); }
}
class Inspector {
  constructor(store) { this.store = store; this.active = new Set(); this.edges = 0; this.commitments = new Map(); }
  fetch(ref) {
    const item = this.store.get(ref.key);
    if (!item) throw new Error('missing closure');
    if (identify(ref.kind, item.data).key !== ref.key) throw new Error('wrong hash');
    return item.data;
  }
  open(ref) {
    if (++this.edges > 30000) throw new Error('expansion limit');
    if (this.active.has(ref.key)) throw new Error('source/provenance cycle');
    // Reference traversal budget, not a canonical history-depth restriction.
    if (this.active.size >= 256) throw new Error('reference traversal depth budget exceeded');
    this.active.add(ref.key);
    return new Reader(this.fetch(ref));
  }
  close(ref, c) { c.finish(); this.active.delete(ref.key); }
  entry(c, depth = 0, pathSize = 0, cfDepth = 0, strictDirectory = false) {
    const start = c.pos, tag = c.u8();
    if (strictDirectory && tag !== 3 && tag !== 4) throw new Error('root alternative is not directory');
    if (tag === 1 || tag === 2) {
      this.generic(c.ref([1]));
      if (tag === 1) c.flag();
    } else if (tag === 3) {
      this.map(c.ref([2, 3]), [], depth, pathSize, cfDepth);
    } else if (tag === 4) {
      this.conflict(c.ref([6]), depth, pathSize, cfDepth + 1, strictDirectory);
    } else throw new Error('invalid Entry tag');
    return c.data.subarray(start, c.pos);
  }
  maybe(c, depth = 0, pathSize = 0, cfDepth = 0, strictDirectory = false, allowAbsent = false) {
    const start = c.pos;
    if (c.flag()) this.entry(c, depth, pathSize, cfDepth, strictDirectory);
    else if (strictDirectory && !allowAbsent) throw new Error('absent directory alternative');
    return c.data.subarray(start, c.pos);
  }
  map(ref, prefix = [], depth = 0, pathSize = 0, cfDepth = 0) {
    if (depth > 256 || pathSize > 4096) throw new Error('path bound');
    const c = this.open(ref), full = new Map();
    const add = (ns) => {
      if (ns.length % 2) throw new Error('mid-byte key');
      const name = packed(ns);
      if (!validName(name)) throw new Error('invalid child name');
      const key = name.toString('hex');
      if (full.has(key)) throw new Error('duplicate map name');
      const nextSize = pathSize + (depth ? 1 : 0) + name.length;
      if (depth + 1 > 256 || nextSize > 4096) throw new Error('path bound');
      full.set(key, [name, this.entry(c, depth + 1, nextSize, cfDepth)]);
    };
    if (ref.kind === 2) {
      if (c.data.length > 4096) throw new Error('large leaf');
      const n = c.u32();
      if (n > 256) throw new Error('impossible leaf count');
      for (let i = 0; i < n; i++) add([...prefix, ...c.nibbles()]);
    } else if (ref.kind === 3) {
      const pre = [...prefix, ...c.nibbles()];
      if (c.flag()) add(pre);
      const bitmap = c.u16();
      const n = Array.from({ length: 16 }, (_, i) => i).filter(i => (bitmap >> i) & 1);
      if (n.length === 0 || n.length + +(full.size > 0) < 2) throw new Error('uncompressed branch');
      for (const slot of n) {
        const child = c.ref([2, 3]);
        for (const [key, pair] of this.map(child, [...pre, slot], depth, pathSize, cfDepth)) {
          if (full.has(key)) throw new Error('overlapping branch');
          full.set(key, pair);
        }
      }
    } else throw new Error('not a map');
    this.close(ref, c);
    if (prefix.length === 0) {
      const independent = new Builder().map([...full.values()]);
      if (independent.key !== ref.key) throw new Error('noncanonical map');
    }
    return full;
  }
  conflict(ref, depth = 0, pathSize = 0, cfDepth = 1, strictDirectory = false) {
    if (cfDepth > 64) throw new Error('conflict nesting');
    const c = this.open(ref), region = c.u8();
    if (region !== 0 && region !== 1 || strictDirectory && region !== 1) throw new Error('invalid conflict region');
    const base = this.maybe(c, depth, pathSize, cfDepth, region === 1, true);
    const terms = Array.from({ length: 2 }, () =>
      this.maybe(c, depth, pathSize, cfDepth, region === 1, !strictDirectory));
    this.close(ref, c);
    for (let i = 1; i < terms.length; i++) if (compare(terms[i - 1], terms[i]) >= 0) throw new Error('noncanonical alternatives');
    if (region === 1) {
      const directory = t => t[0] === 1 && [3, 4].includes(t[1]);
      if (base[0] === 1 && !directory(base)) throw new Error('coupled base non-directory');
      for (const t of terms) if (t[0] === 1 && !directory(t)) throw new Error('coupled alternative non-directory');
    }
  }
  state(ref) {
    const c = this.open(ref), tag = c.u8();
    if (tag === 3) this.map(c.ref([2, 3]));
    else if (tag === 4) this.conflict(c.ref([6]), 0, 0, 1, true);
    else throw new Error('invalid State root');
    this.close(ref, c);
  }
  path(c) {
    const n = c.u32();
    if (n > 256) throw new Error('path depth');
    const parts = Array.from({ length: n }, () => c.str(255));
    if (parts.some(p => !validName(p)) ||
      parts.reduce((sum, p) => sum + p.length, Math.max(0, n - 1)) > 4096) throw new Error('path bounds');
    return parts;
  }
  paths(c) {
    const n = c.u32(), out = [];
    if (n > 1e6) throw new Error('path count');
    for (let i = 0; i < n; i++) {
      const p = this.path(c);
      if (i && comparePaths(out[i - 1], p) >= 0) throw new Error('path order');
      out.push(p);
    }
    return out;
  }
  auditEntry(c) {
    const tag = c.u8();
    if (tag === 1 || tag === 2) {
      c.commitment([1]); if (tag === 1) c.flag();
    } else if (tag === 3) c.commitment([2, 3]);
    else if (tag === 4) c.commitment([6]);
    else throw new Error('audit entry type');
  }
  auditMaybe(c) { if (c.flag()) this.auditEntry(c); }
  record(c, inputs) {
    const tag = c.u8();
    if (tag === 1) {
      c.commitment([4]);
      let n = c.u32();
      if (n > 1e6) throw new Error('structural count');
      for (let i = 0; i < n; i++) {
        const op = c.u8();
        if (op === 1) {
          if (!this.path(c).length || !this.path(c).length) throw new Error('empty structural path');
          this.auditEntry(c); this.auditMaybe(c);
        } else if (op === 2) {
          this.auditEntry(c);
          if (!this.path(c).length) throw new Error('empty structural path');
          this.auditMaybe(c);
        } else throw new Error('structural operation');
      }
      n = c.u32();
      if (n > 1e6) throw new Error('leaf count');
      for (let i = 0; i < n; i++) {
        if (!this.path(c).length) throw new Error('empty leaf path');
        this.auditMaybe(c); this.auditMaybe(c);
      }
    } else if (tag === 2) {
      const mode = c.u8();
      if (mode === 0) {
        if (c.u32() >= inputs.length || c.u32() >= inputs.length) throw new Error('merge input index');
        c.flag();
      } else if (mode === 1) {
        for (let i = 0; i < 3; i++) c.commitment([4]);
      } else throw new Error('merge mode');
      for (let i = 0; i < 2; i++) {
        const algorithm = c.u16(), version = c.u32();
        if (algorithm === 0 ? i === 0 || version !== 0 : version === 0) throw new Error('algorithm version');
      }
      for (let i = 0; i < 2; i++) if (c.flag()) {
        c.commitment([7]);
      }
    } else if (tag === 3) {
      c.commitment([4]);
      if (c.flag()) c.commitment([5]);
      c.u64();
      if (c.u8() > 2) throw new Error('consistency label');
      if (c.flag()) c.take(16);
      const partial = c.flag(), coverageMode = c.u8(), scope = this.paths(c);
      if (coverageMode !== 0 && coverageMode !== 1) throw new Error('coverage mode');
      if (!scope.length) throw new Error('empty observation scope');
      const key = p => p.map(x => x.toString('hex')).join('/');
      const scopes = new Set(scope.map(key));
      for (let i = 1; i < scope.length; i++) {
        const previous = scope[i - 1], current = scope[i];
        if (previous.length <= current.length && previous.every((x, j) => x.equals(current[j])))
          throw new Error('redundant scope prefix');
      }
      const within = p => Array.from({ length: p.length + 1 }, (_, i) => key(p.slice(0, i))).some(k => scopes.has(k));
      const coverage = [this.paths(c), this.paths(c), this.paths(c)], seen = new Set();
      if (coverageMode === 1 && coverage.some(paths => paths.length)) throw new Error('completed scope coverage');
      for (const paths of coverage) for (const p of paths) {
        if (seen.has(key(p))) throw new Error('overlapping coverage labels');
        if (!within(p)) throw new Error('coverage outside scope');
        seen.add(key(p));
      }
      if (!partial && (coverage[1].length || coverage[2].length)) throw new Error('unconfirmed partial observation');
      const n = c.u32();
      if (n > 1e6) throw new Error('resolution count');
      let previous = null;
      for (let i = 0; i < n; i++) {
        const p = this.path(c);
        if (previous && comparePaths(previous, p) >= 0) throw new Error('resolution order');
        if (!within(p)) throw new Error('resolution outside scope');
        previous = p; c.commitment([6]); this.auditMaybe(c);
      }
    } else throw new Error('unknown provenance record');
    return tag;
  }
  generic(ref) {
    if (ref.kind === 1) { this.fetch(ref); return; }
    if (ref.kind === 2 || ref.kind === 3) { this.map(ref); return; }
    if (ref.kind === 4) { this.state(ref); return; }
    if (ref.kind === 6) { this.conflict(ref); return; }
    const c = this.open(ref);
    if (ref.kind === 5) {
      const state = c.ref([4]); this.state(state);
      const n = c.u32();
      if (n > 64) throw new Error('revision parents');
      const parents = Array.from({ length: n }, () => c.ref([5]));
      const uniq = new Set(parents.map(x => x.key));
      if (uniq.size !== n) throw new Error('duplicate parents');
      c.take(16);
      c.str(4096); c.str(1048576);
      c.take(8);
      if (c.flag()) {
        const prov = c.ref([7]); this.generic(prov);
        if (new Reader(this.fetch(prov)).ref([4]).key !== state.key) throw new Error('provenance result mismatch');
        if (this.commitments.get(prov.key).has(ref.key)) throw new Error('output revision in audit inputs');
      }
      for (const p of parents) this.generic(p);
    } else if (ref.kind === 7) {
      if (c.data.length > 1048576) throw new Error('provenance payload size limit');
      this.state(c.ref([4]));
      if (c.u8() !== 1) throw new Error('provenance binding');
      const n = c.u32();
      if (n > 64) throw new Error('provenance inputs');
      const inputs = Array.from({ length: n }, () => c.commitment([5]));
      if (new Set(inputs.map(r => r.key)).size !== n) throw new Error('duplicate provenance inputs');
      const count = c.u32();
      if (count > 1e6) throw new Error('provenance list');
      let previous = null;
      for (let i = 0; i < count; i++) {
        const path = this.path(c);
        const conflict = c.commitment([6]);
        if (previous) {
          const order = comparePaths(previous.path, path) || compare(wire(previous.conflict), wire(conflict));
          if (order >= 0) throw new Error('provenance order');
        }
        previous = { path, conflict };
        const roles = c.u32();
        for (let j = 0; j < roles; j++) if (c.u32() >= n) throw new Error('invalid role');
      }
      const records = c.u8();
      if (records !== 1 && records !== 2) throw new Error('provenance record count');
      const tags = Array.from({ length: records }, () => this.record(c, inputs));
      if (tags.some((tag, i) => i > 0 && tag <= tags[i - 1]) || tags.includes(1) && tags.includes(2))
        throw new Error('provenance record combination');
      if (c.commitments.has(ref.key)) throw new Error('self audit commitment');
      this.commitments.set(ref.key, c.commitments);
    } else throw new Error('unsupported kind');
    this.close(ref, c);
  }
}
function inspect(b, refs) {
  for (const [name, ref] of Object.entries(refs)) {
    const v = new Inspector(b.store);
    try { v.generic(ref); } catch (err) { throw new Error(`decoding ${name}: ${err.message}`); }
  }
  // Hash-valid but noncanonical leaf ordering; checks must not rely on hashes alone.
  const f = b.file(b.blob(B('negative'))), raw = bytes(b32(2), b16(2), B('b'), f, b16(2), B('a'), f);
  const forged = identify(2, raw), store = new Map(b.store);
  store.set(forged.key, { ref: forged, data: raw });
  assert.throws(() => new Inspector(store).map(forged), /noncanonical/);
  // Root conflict through nested coupled conflict cannot include a deletion.
  const empty = b.directory(b.map([]));
  const inner = b.conflict(1, empty, [null, empty]);
  const other = b.directory(b.map([[B('a'), f]]));
  const outer = b.conflict(1, empty, [b.conflictEntry(inner), other]);
  const invalidRoot = b.state(b.conflictEntry(outer));
  assert.throws(() => new Inspector(b.store).state(invalidRoot), /absent directory alternative/);
  assert.throws(() => new Inspector(b.store).conflict(outer), /absent directory alternative/);
  const badBase = b.conflict(1, b.conflictEntry(inner), [empty, other]);
  assert.throws(() => new Inspector(b.store).state(b.state(b.conflictEntry(badBase))), /absent directory alternative/);

  // Correctly hashed invalid executable flag and bad packed-nibble padding.
  const invalid = (kind, payload, method) => {
    const ref = identify(kind, payload), store = new Map(b.store);
    store.set(ref.key, { ref, data: payload });
    assert.throws(() => method(new Inspector(store), ref));
  };
  const file = b.file(b.blob(B('contents')));
  invalid(2, bytes(b32(1), b16(2), B('a'), file.subarray(0, -1), b8(2)), (v, r) => v.map(r));
  invalid(2, bytes(b32(1), b16(1), b8(0xf1), file), (v, r) => v.map(r));
  const missingFile = bytes(b8(1), b8(1), Buffer.alloc(32, 0x5a), b8(0));
  invalid(2, bytes(b32(1), b16(2), B('a'), missingFile), (v, r) => v.map(r));
  // Hash mismatch is rejected before its potentially plausible payload is parsed.
  const valid = b.map([[B('a'), file]]);
  const corrupt = new Map(b.store);
  corrupt.set(valid.key, { ref: valid, data: bytes(b.store.get(valid.key).data, b8(0)) });
  assert.throws(() => new Inspector(corrupt).map(valid), /wrong hash/);

}

function checkOfficialHash() {
  const vectors = {
    0: 'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262',
    1: '2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213',
    64: '4eed7141ea4a5cd4b788606bd23f46e212af9cacebacdc7d1f4c6dc7f2511b98',
    65: 'de1e5fa0be70df6d2be8fffd0e99ceaa8eb6e8c93a63f2d8d1c30ecb6b263dee',
    1023: '10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11',
    1024: '42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7',
    1025: 'd00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444',
    2048: 'e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a',
    2049: '5f4d72f40d7a5f82b15ca2b2e44b1de3c2ef86c426c95c1af0b6879522563030',
  };
  for (const [size, wanted] of Object.entries(vectors)) {
    const data = Buffer.alloc(Number(size));
    for (let i = 0; i < data.length; i++) data[i] = i % 251;
    assert.equal(blake3(data).toString('hex'), wanted, `upstream BLAKE3 length ${size}`);
  }
}
function golden() {
  checkOfficialHash();
  const [b, refs] = samples();
  const expected = JSON.parse(readFileSync(new URL('./codec_vectors.json', import.meta.url), 'utf8'));
  assert.deepEqual(Object.keys(refs).sort(), Object.keys(expected).sort());
  for (const [name, ref] of Object.entries(refs)) {
    const { data } = b.store.get(ref.key);
    const want = expected[name];
    assert.equal(ref.kind, want.kind, name + ' kind');
    assert.equal(display(ref), want.id, name + ' ObjectID');
    assert.equal(data.length, want.payload_len, name + ' length');
    if (want.payload_hex) assert.equal(data.toString('hex'), want.payload_hex, name + ' bytes');
    else {
      assert.equal(createHash('sha256').update(data).digest('hex'), want.payload_sha256, name + ' full payload');
      assert.equal(data.subarray(0, 24).toString('hex'), want.payload_prefix_hex, name + ' prefix');
    }
  }
  const constructed = b.store.size;
  inspect(b, refs);
  return { b, refs, constructed };
}

if (process.argv[1] && new URL(`file://${process.argv[1]}`).href === import.meta.url) {
  if (process.argv.includes('--maps-stdin')) {
    // Small JSON-lines-free subprocess interface for independently generated cases.
    // Input: [ [hex-name,...], ... ]. Output: [canonical full MapId,...].
    const rows = JSON.parse(readFileSync(0, 'utf8'));
    const b = new Builder(), file = b.file(b.blob(B('fixture')));
    const ids = rows.map(row => {
      const m = b.map(row.map(hex => [Buffer.from(hex, 'hex'), file]));
      new Inspector(b.store).map(m);
      return display(m);
    });
    console.log(JSON.stringify(ids));
  } else if (process.argv.includes('--objects-stdin')) {
    // Explicit addresses preserve corrupt-at-address cases, including private
    // audit objects that must not be fetched. No Python verdicts are supplied.
    const corpus = JSON.parse(readFileSync(0, 'utf8')), b = new Builder();
    for (const item of corpus.objects) {
      const data = Buffer.from(item.payload_hex, 'hex');
      const c = new Reader(Buffer.from(item.id_hex, 'hex'));
      const ref = c.ref(Array.from({ length: 255 }, (_, i) => i + 1));
      c.finish();
      assert(!b.store.has(ref.key), 'duplicate corpus object');
      b.store.set(ref.key, { ref, data });
    }
    const results = corpus.roots.map(hex => {
      const c = new Reader(Buffer.from(hex, 'hex')), ref = c.ref(Array.from({ length: 255 }, (_, i) => i + 1));
      c.finish();
      try { new Inspector(b.store).generic(ref); return true; }
      catch (err) {
        // A runtime stack/allocation error is a decoder crash, not safe rejection.
        if (err.constructor !== Error) throw err;
        return false;
      }
    });
    console.log(JSON.stringify(results));
  } else {
    const { refs, constructed } = golden();
    console.log(`Cross-language AVC v0 vectors: ${Object.keys(refs).length} PASS; objects ${constructed}; independent BLAKE3 PASS`);
  }
}
