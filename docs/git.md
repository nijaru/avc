# AVC ↔ Git interoperability contract

**Design baseline 2026-11-04; first local translation implemented 2026-11-12.** Git is an **adoption and publication boundary**, not AVC's source storage, object format, conflict algebra, or native protocol. This document owns Git translation; native clone/partial semantics live in [sync](sync.md), canonical source in [format](format.md), and observable user behavior in [UX](ux.md).

## Available local round trip

The experimental CLI implements a **local SHA-1 subset**, using an explicit Git administration directory and exact commit OID. It does not resolve refs, fetch, push, update a Git index/ref/worktree, run hooks or execute filters. SHA-256 repositories refuse. Git must support the `--no-lazy-fetch` and raw `cat-file` options used by the adapter; unavailable capabilities fail closed. Source bytes, executable modes, symlinks and LFS pointer bytes are preserved; gitlinks, malformed objects and unsupported metadata refuse. Git author/message must fit native UTF-8 metadata. The subset excludes pre-epoch dates and NUL-bearing commit metadata. Git and canonical source closures each have 16 MiB payload, 256 MiB aggregate and 100,000-object limits; Git conversion also bounds expanded entries at 100,000, depth at 256 components, paths at 4096 bytes and foreign parents at 16. Original commit bytes, tree and ordered foreign parents are retained in SQLite, atomically with the native anchor and import receipt. No historical Git ancestry is converted: the recorded foreign parents are a boundary, even if Git happens to have those objects locally. There is no remote or tracking-ref snapshot because this slice accepts exact OIDs only.

- `avc --experimental git import --git-dir DIR --commit OID --request-key KEY` requires an untouched initialized native store. An exact retry returns its receipt even after native edits or removal of the Git store; the path is not part of identity.
- `avc git origins [--revision NATIVE_ID]` inspects retained boundaries without contacting Git. Ordinary `status` does not traverse ancestry or certify Git completeness.
- `avc --experimental git export --git-dir DIR [--revision NATIVE_ID]` prepares a complete representable candidate, writes immutable Git objects and records derived mappings. Without `--revision`, selected source must match the authored head. Selection/generation remain unchanged. A parentless native revision without a foreign boundary exports as a Git root, including after native history transfer. Known import anchors instead require retained origin metadata and their exact original Git commits at the target; missing or damaged origins refuse rather than fall back to native-root translation. New translated commits use mapped parents in native parent order.

Empty top-level source is representable; standalone empty directory entries and unresolved source conflicts refuse. Export policy **`avc.git.export/v1`** uses the exact native author as both Git author and committer, with the native timezone. The identity must be a single-line representable `Name <email>`; no email is invented. Git time is `time_ns.div_euclid(1_000_000_000)` seconds. Exact nanoseconds and the native RevisionId remain in `avc-time-ns` and `avc-revision` audit headers. This is a declared Git precision projection, not a lossless native identity round trip. Reimport treats those headers as untrusted foreign data and creates the ordinary deterministic Git anchor; it does not recreate native parents from a claimed ID.

All metadata/source/mapping checks precede Git writes. An I/O failure afterward may leave unreferenced candidate objects; retry is deterministic. Git object writes and SQLite mapping publication are **not one transaction**, and no Git ref has been published. Candidate objects may be pruned by Git until the user deliberately protects/publishes them. Repeated export validates native data and target objects again; the mapping is not an availability certificate or a second source authority.

**Current portability limit:** `AVCHIS01` cannot carry foreign-origin records. Selected-history export refuses an owning graph containing a known Git anchor *before creating output*, rather than silently dropping its boundary. Source-only export still works, but does not back up authored history or Git origins. Preserve the original native store using a SQLite-consistent backup; copying only an active main database is unsafe. Portable Git-origin metadata is follow-up work. Amending/rebasing an import anchor itself refuses; author a native child first. Rewriting that child remains native, using exact known source comparisons rather than an invented old Git merge base.

With a built `./target/debug/avc`, this local example needs only stock Git:

```sh
mkdir -p tmp
demo=$(mktemp -d tmp/git-roundtrip.XXXXXX)
git init --bare "$demo/foreign.git"
blob=$(printf 'original\n' | git --git-dir "$demo/foreign.git" hash-object -w --stdin)
tree=$(printf '100644 blob %s\thello\n' "$blob" | git --git-dir "$demo/foreign.git" mktree)
tip=$(GIT_AUTHOR_NAME=Original GIT_AUTHOR_EMAIL=original@example.test \
  GIT_COMMITTER_NAME=Original GIT_COMMITTER_EMAIL=original@example.test \
  git --git-dir "$demo/foreign.git" commit-tree "$tree" -m original)
mkdir "$demo/native"
./target/debug/avc --repository "$demo/native" --experimental init
./target/debug/avc --repository "$demo/native" --experimental git import \
  --git-dir "$demo/foreign.git" --commit "$tip" --request-key import
printf 'native edit\r\n' > "$demo/bytes"
./target/debug/avc --repository "$demo/native" --experimental source put hello \
  --file "$demo/bytes" --expected-generation 1 --request-key edit
./target/debug/avc --repository "$demo/native" --experimental commit \
  --author 'Native <native@example.test>' -m 'Native child' \
  --expected-generation 2 --request-key child
result=$(./target/debug/avc --repository "$demo/native" --experimental git export \
  --git-dir "$demo/foreign.git")
oid=${result#commit=sha1:}; oid=${oid%% *}
test "$(git --git-dir "$demo/foreign.git" rev-parse "$oid^")" = "$tip"
git --git-dir "$demo/foreign.git" cat-file -p "$oid"
./target/debug/avc --repository "$demo/native" git origins
```

A repository started in AVC needs no prior Git import. This example captures and authors a native root, then exports it into a fresh bare Git store:

```sh
mkdir -p tmp
demo=$(mktemp -d tmp/git-native.XXXXXX)
mkdir "$demo/native"
./target/debug/avc --repository "$demo/native" --experimental init
printf 'native source\n' > "$demo/hello"
./target/debug/avc --repository "$demo/native" --experimental capture \
  --from "$demo" --expected-generation 0 --request-key capture hello
./target/debug/avc --repository "$demo/native" --experimental commit \
  --author 'Native <native@example.test>' -m 'Native root' \
  --expected-generation 1 --request-key root
git init --bare "$demo/target.git"
result=$(./target/debug/avc --repository "$demo/native" --experimental git export \
  --git-dir "$demo/target.git")
oid=${result#commit=sha1:}; oid=${oid%% *}
test "$(git --git-dir "$demo/target.git" rev-list --count "$oid")" = 1
test -z "$(git --git-dir "$demo/target.git" show -s --format=%P "$oid")"
git --git-dir "$demo/target.git" show "$oid:hello"
```

Both examples leave candidate objects unreferenced; no Git branch was created. The sections below describe the broader interoperability target; refs, remote synchronization, foreign partial views and hosting are not implemented by this subset.

## The fundamental identity incompatibility

Git tree/blob identities are SHA-1 or SHA-256 of Git object framing; AVC uses typed BLAKE3 IDs of a **different canonical representation**. There is no way to know the AVC BlobId of an unfetched Git blob, and no safe general SHA→BLAKE3 translation. A Git partially filtered current tree must not be passed to the AVC native codec as a complete authenticated tree.

**Two clearly labeled modes are selected.**

## A. Normal native-writable import

Used for ordinary existing repositories and GitHub/GitLab onboarding.

1. Resolve the **exact Git commit/ref** with its hash algorithm and configured remote, using an established Git subprocess or maintained library. Reject unknown/untrusted remotes and conflicting local/remote refs.
2. Retrieve **every blob and tree necessary to reconstruct the selected current Git tip**. Old historical blobs and every ancestor commit are not required initially; missing historical ancestors become an explicit **foreign Git ancestry frontier**.
3. Verify the Git object IDs and type/mode/path fidelity, including source at Git LFS *pointer* level rather than pretending to own LFS payloads. Refuse unsupported submodules or deliberately offer an opaque Git-link adapter view; do not mislabel a Git submodule as a regular native directory. Reject paths unrepresentable on the target filesystem while preserving source names independently when feasible.
4. Convert exact representable current-source bytes into native Blob/Map IDs. Create a **deterministic native import-anchor revision** with that source root and **no native parents**. Use the exact import ChangeId and normalized author/message/committer-time grammar in [format](format.md#f-revision-0x05); two independently importing clients should obtain the **same** anchor RevisionId for the same Git commit/source bytes. Record the original foreign tip, parent OIDs, original raw metadata, available ancestry frontier, tracking ref snapshot and Git hash algorithm in a separate durable adapter record. A zero-parent native anchor is not a claim that Git commit had zero parents.
5. Author changes natively on top of that anchor. **Normal authored** ChangeIds are native random values and source roots never depend on Git OIDs; only the **special deterministic import-anchor ChangeId** uses a foreign Git OID to ensure cross-client convergence. Native source and foreign origin are separately inspectable.
6. Incremental Git fetch updates the *foreign tracking ref* under guarded expected versions. Update/merge native work using exact source roots, and fetch necessary Git merge bases through the adapter when cross-boundary history is otherwise unknown. Never treat the import boundary as an actual ancient Git root or silently follow a force-pushed ref.
7. Git export builds representable Git trees and commits, using the **original exact Git tip** as the parent of the first exported native child, then mapped Git commits for subsequent native parents. It must not invent unknown old Git ancestry. Preserve a stable native↔Git revision mapping for each completed translation.
8. Git push uses the remote's exact expected ref with lease/conditional comparison. If upstream advanced, refuse and offer fetch/rebase/merge; don't silently force. A lost push response requires reconciliation by reading the actual remote before repeating.

**What "preserve Git history" means:** imported current source can be exact while Git ancestry remains foreign. Exported new Git commits can link to the exact original Git parent if that parent remains available at the target remote. Converting older Git commits into native revisions *after* authoring would change native roots/parents if forced into old revision IDs, so don't silently graft them. Keep foreign ancestry as explicit adapter edges and use Git for cross-boundary history queries; a deliberate migration may produce new native revisions with traceable new IDs, never retroactively alter existing hashes.

## B. Foreign Git partial view

For enormous monorepos, intermittent agents and situations where fetching all current blobs defeats the intended UX:

- Keep the selected Git commit and its tree/blob OIDs as **foreign**. A filtered/shallow Git repository and promisor semantics may provide path listing, Git-native diff, sparse checkout, Git-relative conditional patching, ref inspection, and on-demand Git object retrieval.
- A new Git blob for an edited path has its Git hash computed from its actual bytes. Git tree objects can reuse *existing Git OIDs of unchanged paths*, allowing a partial view to form a foreign Git commit without downloading every unchanged blob. Use real Git's protocol and safety checks for this workflow.
- A foreign partial change may be published to Git with Git's commit/ref semantics, subject to representability, integrity, permissions, and expected-ref lease. **It is not a native AVC authored revision** and must not claim a native full source root, native offline custody, or a native content-verified backup.
- Do not silently download the whole monorepo or reinterpret foreign IDs to manufacture a "native" sparse root. Provide a distinct `foreign` status and an explicit `convert-to-native` hydration operation that obtains the **entire selected current source closure** before native authoring.
- Git remote support for `blob:none`, treeless or shallow fetch, promisor missing objects and pack negotiation varies; adapt to actual capabilities. A missing Git object is not necessarily corruption if it is promised; never turn unknown ancestry into a fake merge base.

A future native overlay whose canonical entries reference both Git and AVC IDs is **not selected**: it complicates content equality, full backup, identity stability, and trust. Only reconsider with demonstrated complete-task gains and a properly versioned native format design.

## Optional shared Git-to-native conversion cache

**Long-term adoption option, not part of the first local Git adapter.** The current rule that native BLAKE3 identities require the **complete selected current Git source bytes** is a mathematical constraint on conversion, **not** a requirement that every client download those bytes. An authorized shared conversion service could:

1. Resolve an exact Git commit/hash-algorithm/remote identity and read+verify its full required Git tree and file bytes **once per unique source/conversion policy**.
2. Compute/cache native AVC BlobIds, the deterministic source root and the Git↔AVC import mapping, keyed by exact Git commit, hash algorithm, AVC codec version and conversion policy. Verify that cache entries bind to the exact converted byte closure; repeated clients cannot simply trust an unrelated root claim.
3. Authenticate the **native source-root attestation and holder** to selective AVC clients, then serve only their requested BLAKE3 tree nodes/blob objects and path proofs. Clients can validate fetched native bytes/proofs against the trusted root and work as *native thin clients*, without each client hydrating all current Git blobs.

This amortizes conversion over many users or agents and can bypass a per-client full-current-tip download. It does **not** eliminate source-byte ingestion somewhere, and does **not** give a thin client independent proof that an untrusted gateway honestly converted a Git SHA tree without reading the foreign content or using separately specified/verifiable conversion proofs. Trust, authorization, privacy, stale/force-pushed Git tips, object retention, and service operation all require explicit contracts. If the service cannot provide adequate trust or upstream bytes are unavailable, retain **Foreign Git view** and disclose its limitations.

Meta Mononoke's [canonical data and derived Git mappings](https://github.com/facebook/sapling/blob/main/eden/mononoke/docs/2.3-derived-data.md) demonstrate the pattern of centrally computed, cached cross-VCS representations, **not an AVC-specific proven gateway**. This option does **not** require a second Git-backed AVC engine or a new canonical mixed-hash source kind. Only implement after measured repeated full-tip conversion pressure or native hosted demand; it is not a dependency of first local AVC use.

## Lossless subset and explicit refusals

| Input or feature | Contract |
| --- | --- |
| Regular files and executable bit | Exact bytes and executable status when representable |
| Symbolic links | Exact target bytes, no dereference; refuse unsafe local materialization |
| Git file timestamps/owners | Not source identity; no claim of preserving them |
| Git submodules (gitlink) | Explicit unsupported native v1 source kind; retain as foreign mapping or refuse conversion |
| Git LFS | Git source identity represents pointer bytes; external LFS payloads require independently authorized retention/hydration |
| Native empty directory | Cannot be represented in Git tree history as a standalone empty directory; exporting must report lossy and require explicit approval or refuse |
| Native unresolved conflict | Cannot export as a clean Git file; must resolve explicitly or refuse |
| Non-UTF8 messages, exotic headers | Native Revision author/message require UTF-8; preserve foreign data in adapter or refuse true lossless metadata conversion |
| Signed Git objects/tags | New Git objects have different IDs/signatures; never pretend signatures survive rewriting |
| Git SHA-1 or SHA-256 | Store algorithm with every foreign OID; never mix untyped digests |
| Git merge commits | Preserve known parent order when all representable and mapped; incomplete ancestry yields a typed refusal or foreign boundary |
| Case/Unicode/path limitations | Source names may remain exact in native logical tree; refuse unsafe or lossy materialization |

**Loss disclosure occurs before effectful import/export**, not after a partial irreversible migration. The adapter cannot silently auto-resolve native conflicts, invoke Git hooks, trust repository config as permission to execute arbitrary processes, or push without explicit authorization.

## Concurrency and authority

A native AVC repo and a colocated writable Git index/ref database must not independently control the same filesystem paths and history. The normal bridge uses **separate stores and a single active source authority**. Git integration reads remote or isolated foreign repositories and exports a deliberate candidate. An optional interoperable colocated mode would need a separately designed lock, authoritative selection, external Git mutation detection and recovery protocol; it is not an assumption in v1.

The bridge may use `git` subprocesses because that delegates object/pack/negotiation complexity to mature software. Disable hooks, replacement/graft surprises, unsafe custom filters and automatic external command execution for untrusted remotes. Treat Git config/env as untrusted at source boundaries and explicitly select safe settings for each subprocess.

## Long-term Git compatibility: one engine, multiple protocols

**Permanent product direction:** Retain Git import/export and remote synchronization for as long as AVC supports existing Git-hosted users. When AVC hosts its own native repositories, add an **optional Git smart-protocol frontend** that serves ordinary Git clients from **the same native source and ref authority**. **Do not build or maintain a second Git-backed AVC storage/revision/working-copy backend.** The native source/history core is partially implemented; the initial local Git adapter is implemented; remote synchronization and the hosting frontend are not.

### Trust and representation boundaries

- The canonical source, parent DAG, ChangeIds, conflicts, mutable refs and retention live in AVC. Generated Git blobs/trees/commits/packs and native↔Git mappings form a **derived, rebuildable, versioned compatibility index**, pinned for readers during transfer. Cache invalidation follows exact source revision, export policy and Git hash algorithm.
- A Git-visible native ref is one **logical ref with one versioned authority**, not two branches synchronized eventually. Publication through either native API or Git frontend checks the same native expected generation. Its advertised Git OID must correspond to the **current representable native revision**. If conversion fails, the ref update is **not accepted** as Git-visible; a user may instead publish an explicitly native-only ref.
- For refs offered to both native and Git clients, every accepted tip and the ancestry visible to Git must be representable. Source conflicts, native empty directories, unsupported metadata, missing ancestors, or lossy conversion require a documented refusal or an explicit separately named projection; a silently projected Git tip is forbidden. An annotated/signed Git tag or signature cannot be fabricated merely because native source bytes match.
- The Git-facing object format (SHA-1 or SHA-256) is negotiated/declared per repository. Deterministic canonical translation must produce identical Git OIDs for the same native revision and same conversion policy; preserve genuine original Git objects when they remain exactly valid. Native ChangeIds and revisions are **not** Git OIDs.
- The frontend implements Git protocol authentication/authorization, old-OID ref validation, object/pack negotiation, reachable-object access rules, upload/receive limits and replay safety. Git push becomes **validate Git objects → map/verify all supported source/history → prepare native candidate → CAS authoritative native ref → acknowledge**. If accepted but client loses the reply, reconcile exact request and ref state before retry. Do not automatically execute remote hooks, filters or repository-provided programs.
- Derived pack caches may use Git-format objects because **Git clients require them**, but that cache is not a writable second VCS engine. The native store may be BLAKE3/Zstandard; it need not adopt Git's physical packfiles, ref layout or SHA identity internally.

**Git visibility is a policy on refs, not a weaker definition of AVC source.** A native-only ref may intentionally contain conflicts or source constructs that Git cannot express. The user can resolve/transform them into a new representable candidate for the Git-visible ref. Advancing that shared ref must never silently strip native data or claim fidelity it lacks. This costs an extra qualification boundary and sometimes restricts the shared branch; the alternative is divergent, misleading parallel histories.

### Delivery sequence

The [roadmap](roadmap.md#stage-4--git-onboarding-early) owns cross-project ordering. Git delivery is incremental, not an all-features gate:

1. **Small local round trip (first):** import a verified complete Git tip into its deterministic native anchor, retain foreign-origin mapping, author a native child and export it with the exact original Git parent. Verify independently with stock Git. Declare the supported metadata/hash/source subset and refuse unsupported cases before effects. Existing native authoring is sufficient; advanced rewrite UX, filesystem capture, remote hosting and foreign partial mode are not prerequisites.
2. **Ordinary remote collaboration:** guarded remote tracking/fetch/push, idempotent translation and lost-response reconciliation; then upstream advancement and safe fetch/rebase/push. Use isolated Git stores, not competing Git/AVC ownership of one working tree.
3. **Foreign partial views and compatibility expansion:** support huge filtered Git sources separately, with explicit foreign identities and hydrate/convert boundaries. Add additional hash-algorithm, metadata and ancestry cases as supported; never present the initial subset as full Git parity.
4. **Native hosting foundation:** native object/proof API, authenticated refs, durable owning closure and exact CAS. No Git smart protocol is required for native-only users.
5. **Git fetch frontend (later):** necessary Git smart HTTP protocol v2 fetch/ls-refs, deterministic Git objects/packs, stable mappings, ref visibility and cache correctness. Validate with unmodified Git clients.
6. **Git push frontend:** verify received Git objects, convert supported source/history and enforce native CAS plus Git old-OID/lease checks. Extend concurrency, response-loss and retention checks with this lifecycle.
7. **Optimize on demand:** protocol partial-clone/shallow capabilities, tags and caching where actual adoption needs justify them.

**Compatibility statement:** Users should eventually be able to collaborate with Git clients and Git hosts around representable history, not assume `git` commands can directly manipulate a local `.avc` database. GitHub/GitLab PRs and CI continue to operate on **exported Git commits**, not native-only objects. A Git frontend on AVC hosting does not automatically make AVC-hosted native-only revisions meaningful to ordinary Git tooling.

## Testable adoption contract

- First slice: import a small complete Git tip, author natively and export locally; stock Git verifies exact source and the original Git parent. Re-importing the same supported tip converges to the same native anchor.
- Remote collaboration slice: add push, upstream advancement and native amend/rebase, independently verifying source and ancestry. This is not required to declare the first local translation slice complete.
- Repeat import/export without duplicating native commits or advancing the wrong remote ref; handle an upstream force push and a lost push response.
- Test merge parents, executable/symlink semantics, non-UTF8 metadata, Git LFS pointers, submodules, empty directories, signed objects, and both Git hash algorithms.
- Clone a blobless/shallow *large* Git repository and perform a foreign patch/push without claiming native custody. Later convert to native only after full selected current source hydration.
- Offline foreign partial fetch must return promised/unavailable, not a clean missing-path result. Native full export must remain self-contained independently of Git.

The public product should distinguish **Native**, **Foreign Git view**, **Native with foreign ancestry frontier**, and **Complete native offline** in status/API output. These are real capability distinctions, not marketing synonyms.
