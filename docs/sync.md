# AVC native synchronization and partial repositories

**Implementation contract (2026-11-04).** This document owns native object availability, remote requests and publication. Canonical source and proof rules live in [format](format.md), durable local ownership in [storage](storage.md), and [foreign Git interoperability](git.md) has its own adapter boundary. Native source control does **not** require a server.

## 1. The four independent axes of a clone

A clone chooses (1) exact requested ref/revision, (2) history horizon, (3) which immutable tree/blob/revision objects are locally present, and (4) which files are materialized in a directory. These must never be conflated.

Native default `avc clone URL`: resolve and pin the remote's advertised default ref (or requested ref) at one exact version; obtain its current logical tree and **full ordinary working directory** for typical repositories. Initially fetch **up to 64 first-parent revision metadata records**, not every old blob or every side-branch history. Ancestors outside the fetched horizon remain *promised*, not fabricated roots. A path-scoped or `--no-checkout` clone may skip unrelated current blobs; `--history=all` deepens the requested ancestry; `--full` hydrates **the complete owning closure of explicitly selected refs** and proves offline reconstructability.

For large monorepos, prioritize requested source paths and metadata-first queries; do not require a virtual filesystem, daemon, or whole checkout. For small projects, first useful source and normal editor compatibility matter more than minimizing transferred bytes. An exact pinned ref version never silently changes mid-fetch; an explicit later pull can choose a newer tip.

## 2. Availability and proof states

| State | Interpretation |
| --- | --- |
| `PresentVerified` | Canonical bytes locally present and validated against exact typed ID |
| `Promised` | Referenced by an authenticated native root but not locally held; an authorized holder is declared |
| `Unavailable` | Promisor offline, timed out or unable to serve now |
| `Unauthorized` | Caller cannot access the requested path/object; do not leak neighbors through proofs |
| `AbsentProven` | Complete authenticated membership/absence witness proves the path missing |
| `MissingUnexpected` | Required object missing without valid promise (possible corruption/incomplete custody) |
| `Corrupt` | Hash, kind, canonical decoding, length or proof disagrees |

Authentication of a remote root is **not** local availability of its closure. A thin native client may derive a *conditional candidate* root using an authenticated complete base root, validated path membership/absence proofs, expected-value witnesses and sibling hashes, even though sibling blobs are not local. It must not call the result a fully hydrated local repository. An authorized holder must verify changed dependencies and retain the full closure before publishing the candidate.

A path-restricted principal must **not** receive raw Merkle parent nodes that reveal unauthorized sibling names or values. The holder can instead perform the path-scoped conditional update and return only permitted output. Without authenticated root/proofs, return `NeedObjects` or an explicitly pending edit; never invent a locally complete new root.

## 3. Native transport

**Initial protocol:** TLS-authenticated HTTP/2, versioned control requests, bounded JSON control schema and streamed binary object records. HTTP/3, gRPC or other transports can be added behind the same semantics. The transport does not determine canonical source identities. A record includes protocol version, exact typed ObjectId, canonical uncompressed length, transfer codec, encoded length, checksum and payload; physical batching/packaging is not identity. Verify final decompressed bytes against the ID before admitting them. Partial ranges are not independently whole-Blob-verified unless an authorized proof or complete stream establishes that fact.

Proposed semantic endpoints:

```text
GET  /v1/capabilities                 -> protocol version, object kinds, negotiated bounds
GET  /v1/refs/{name}                  -> exact target RevisionId, generation, auth scope
POST /v1/objects/plan                 -> wanted IDs, known IDs, priorities, bounded batch plan
POST /v1/objects/batch                -> immutable verified frames or typed missing/denied
POST /v1/objects/upload               -> stage/verify new objects, report dependencies
POST /v1/source/read                  -> exact path/range queries without checkout
POST /v1/source/apply                 -> conditional base root plus path witnesses/edits
POST /v1/refs/{name}/compare-and-swap -> expected version, new revision, request ID
```

Clients should batch and prefetch related objects, prioritize the current working set, use bounded parallelism and backpressure, deduplicate requests, and support verified resumable transfers. Reusable transport packs, cache tiers and geometric compaction may improve hosted scale (Pando/Scion principles), but need not become mandatory in local VCS or canonical format. Untrusted length, overlapping range, truncated stream, decompression bomb, hash mismatch or unrecognized object version never yields success.

## 4. Publication and remote consistency

Native objects can upload independently and be cached forever by immutable ID. A remote accepts a ref update only after **it** validates and durably protects the full owning closure, then atomically compare-and-swaps the named ref from `expected_generation` to a new generation. A stale ref returns its current target/version; it does not silently rebase the caller or use last-writer-wins. A destructive rewrite requires explicit permission plus the expected ref lease.

Every publish has an idempotency key tied to its exact principal, target and request; a retry after lost response returns the accepted/rejected prior result, not another mutation. Independent refs may advance concurrently. A read of several refs is **not automatically one coherent repository-wide snapshot**: clients requiring that must request and pin an explicit multi-ref server snapshot token. This consistency guarantee must be documented at each API boundary.

A remote ref's durability/retention policy protects full source and ancestry dependencies. Draft roots may have explicit deadlines. GC/compaction cannot race protected refs, in-flight publications, active readers, remote replicas, or backup leases. Content IDs alone give integrity, not origin authenticity, access permission, encryption or remote availability.

## 5. Checkout-free source APIs

The same Rust source library underlies local CLI, eventual server and agent integrations:

```text
resolve(ref) -> revision+version
read(root,path,fetch_policy) -> Known | AbsentProven | Promised | Denied | Corrupt
diff(left,right,fetch_policy) -> exact changes | NeedObjects
edit(root,[expected old entry -> new entry]) -> candidate root | Stale | NeedObjects
merge(base,left,right,driver_version) -> exact root | Unresolved | Pending
publish(target,expected_version,candidate) -> Accepted | StaleRef | Denied | NeedObjects
```

Nothing requires a checkout or shell. A remote may compute a derived path query, blame or merge base lazily; derived data never authorizes a different canonical source or hides unknown ancestry. Read operations may populate immutable caches when online policy permits, but must not silently author commits, execute source hooks or move refs.

**Failure checks:** disconnect mid-upload, lost response after successful CAS, remote object promised but unavailable, stale expected path while editing, two agents racing a ref, corrupted compressed object, path-scope access denial, and signed/proven source no longer available offline. Each has a typed outcome and retains any already acknowledged input protection.

## 6. Adoption boundary

Git is **not** a native promisor. A Git partial clone may know foreign Git IDs for blobs whose native BLAKE3 hashes cannot be computed without content. The two supported integration modes, import-anchor semantics, foreign ancestry, Git-native partial edits and safe representable export are specified in [git.md](git.md). Do not introduce Git OIDs as hidden native Map IDs.

## 7. Qualification

Release checks include independent native object/proof verification; ordinary and path-scoped clones; a long-lived client deepening its history; full offline hydration/export; two concurrent publishers and retries; a huge current tree accessed without checkout; and a path-restricted caller who must not learn sibling names. Benchmark **first useful operation + later fetch**, not initial bytes alone.
