# AVC user workflows and command/API contract

**Design baseline 2026-11-04; authored commit/log added 2026-11-09; selected-history transfer, basic amend/undo and single-change rebase added 2026-11-11; local Git translation added 2026-11-12.** The README lists today's experimental commands. Basic authoring, first-parent history, selected-history transfer, tip amend, scoped undo, single-change rebase and a local Git translation subset are implemented; advanced history editing, recursive capture/checkpoints, workspaces and synchronization below remain **target workflows**, not available commands. Commands below are **selected target spellings**, subject to usability adjustment *before public release*. The [architecture](design.md) owns invariants, [format](format.md) owns immutable data/merge, [storage](storage.md) owns recovery, [sync](sync.md) owns native remote behavior, and [git](git.md) owns the Git adapter. Do not implement duplicate state machines in the CLI.

## 1. User mental model

AVC has **source**, **changes**, **unfinished work**, and **sharing**.

- A **source root** is an exact immutable snapshot, with typed unresolved conflicts when necessary.
- A **change** is an authored unit (stable ChangeId) with an exact immutable RevisionId; changed source, parents or metadata produce a different ID, while a no-op rebase retains it. A stack is an ordered group of specific change versions.
- A **work** is one resumable local effort, including pre-first-commit edits, with a base, a current selection and protected checkpoints. Naming is optional; distinct unfinished work cannot be identified solely by equal file bytes.
- A **checkpoint** protects captured bytes and exact scope privately; it neither tracks/publishes new files nor creates an authored revision. Explicitly protected unfinished work survives ordinary checkpoint expiry.
- A **remote ref** points to an exact authored revision. A push conditionally changes it after required object transfer. Tests and reviews bind exact candidate versions, not floating labels.
- The **filesystem** is mutable by other tools and cannot be assumed to be the same as last captured source; editor unsaved buffers, processes, dependencies and ignored environments are outside this guarantee.

By default, `status`/`log` operate locally; `show`/`diff` can read-through-fetch promised immutable data under configured online policy. None of these read-only commands creates commits, checkpoints, moves refs, runs hooks or discloses private source. Use `--offline` to forbid fetches.

## 2. Starting and cloning

**Available local Git subset:** `git import --git-dir DIR --commit OID --request-key KEY` imports one exact SHA-1 tip into an untouched store; `git export --git-dir DIR [--revision ID]` writes candidate objects, not refs. Both require `--experimental`. Native-born parentless history exports as Git roots without a prior import; known foreign anchors still require exact retained origins. Without an exact `--revision`, export requires selected source equal to the authored head. `git origins` reports retained foreign boundaries without contacting Git. These do not implement the clone/fetch/push workflows below. See [Git](git.md#available-local-round-trip) for examples, metadata projection and portable Git-origin history. Selected-history restore needs neither Git nor network; subsequent export can reconstruct exact original current-source objects in an empty Git target without fetching foreign ancestors.

```sh
avc init
avc clone https://github.com/org/repo
avc clone avc+https://source.example/org/repo --ref main
avc clone avc+https://source.example/org/mono --paths src/api/,Cargo.toml
avc clone avc+https://source.example/org/repo --no-checkout
avc clone URL --full
avc hydrate src/api/
avc hydrate --history=all
avc hydrate --all
```

`init` refuses an occupied or incompatible `.avc` directory; never silently migrate old recovery data. Existing unrelated project files remain untouched and untracked until selected. No fake initial authored commit.

`clone` pins the selected branch/ref's exact current version. **Native default:** current source fully materialized for normal repos, up to 64 first-parent revision metadata records, older history/blobs fetched when needed. `--paths` and `--no-checkout` use native authenticated sparse promises; missing promised objects are not deletions. `--full` obtains independently verified native closure for the explicitly selected refs, not every remote ref in existence.

**Git modes are distinct.** A normal Git URL converts *all current selected-tip source bytes* to native, with a clearly labeled **foreign ancestry frontier** where older Git history is not yet imported. A huge filtered Git checkout uses `avc clone --foreign URL --paths ...` to stay explicitly in **Foreign Git view** and permit Git-relative source operations. It does *not* claim an AVC BLAKE3 root; `avc convert-to-native` requires full selected current-source hydration, warns about scope and may refuse. `--full` on Git must resolve and retain the requested complete supported Git ancestry and native source or report an explicitly incomplete import; it cannot falsely claim full native history. A failed selective fetch never silently becomes a full monorepo download.

Default status includes mode, current work/base, modified/new/deleted count and coverage, checkpoint protection, missing/promised objects, and remote sharing *as last known*:

```text
Mode: Native (Git ancestry frontier at abc1234)
Work: parser-cleanup   Base: change:7bc, rev:2af
Files: 3 modified, 1 new (not selected)
Recovery: none; no background checkpoint policy enabled
Availability: current source complete, earlier history foreign/pending
Remote: origin/main last seen at abc1234 (not checked now)
```

An unknown remote/recovery state is shown as unknown, never as safely synchronized. Unreadable source/changed-during-scan is shown as incomplete, never clean.

## 3. Exact authoring and change editing

**Available now:** `avc --experimental commit --author 'Name <email>' -m MESSAGE --expected-generation N --request-key KEY` authors the already-selected source, without scanning or changing files. `AVC_AUTHOR` can supply author identity. Init authors nothing; explicit empty/unchanged-source commits are allowed. Source edits preserve the last authored head, while commit advances it under the same generation. Identical retries return the original Revision/ChangeId/time; changed identity or cross-operation key reuse refuses. `avc log [--revision ID] [--limit N]` shows newest-first first-parent metadata (default 20); `status` shows root, authored head/unborn and generation, never a fabricated filesystem-clean claim. Metadata output escapes terminal controls. Default source `export`/`import` does not transfer this history. Explicit `export --history --output FILE` and `--experimental import --history --input FILE --request-key KEY` preserve the selected source and optional authored head with all native owning ancestry. Import requires an untouched initialized store; generations, old receipts, instance identity, workspaces and Git metadata are excluded. Selected-but-uncommitted source is preserved separately, not silently authored. See [storage §8](storage.md#8-export-backup-and-disaster-recovery) for wire scope and limits.

**Available tip amend:** `avc --experimental amend --author AUTHOR -m MESSAGE --expected-generation N --request-key KEY` authors selected source into a new Revision with the current change's ChangeId and exact ordered parent list; it does not add the old tip as an extra parent. Author/message are explicit, as for commit; the timestamp is new. Unborn heads refuse. Unchanged/empty source outcomes are allowed and reported; comparison-diagnostic failure warns without hiding a committed receipt.

**Available metadata undo:** `avc --experimental history undo OPERATION_KEY --expected-generation N --request-key KEY` restores that receipt's before-source/head values only if its after-selection and generation still match current state. Undo advances generation, retains objects and has its own target-bound retry receipt; it does not rewind the database or touch filesystem/remote state. Undo may itself be undone. Selected-history export does not transfer private undo receipts or superseded revisions outside its owning graph.

**Available single-change rebase:** `avc --experimental rebase --onto REVISION_ID --expected-generation N --request-key KEY` replays a clean selected change onto an exact known native revision, preserving its ChangeId, author/message, authored time and timezone. Its comparison source is the exact original parent's source, or empty source if parentless; the target becomes its sole parent. The original revision may have at most one parent. Selected source must equal the authored head's source; otherwise refuse without changing either selection. Do not silently drop selected-but-uncommitted source, auto-stash it or include it in a replay. Target ancestry containing the same ChangeId refuses, including self/descendant targets. No-op rebase onto the existing parent retains the exact RevisionId but advances generation and records a receipt. Both input owning history graphs and the result have explicit aggregate bounds; this is not incremental history traversal. Named-ref rebase, descendant restacking and interactive capture are later capabilities.

A rewrite reports an unchanged/empty result explicitly. A differing conflicted comparison base may return `UnresolvedBase` without publishing an output; do not imply that arbitrary conflicted histories can already be rebased. Scoped history undo restores recorded selection values under a new generation, not old generation counters, filesystem bytes or remote refs ([storage §7](storage.md#7-operation-journal-and-retryundo)).

**Available explicit capture:** `avc --experimental capture [--from DIRECTORY] --expected-generation N --request-key KEY PATH...` observes existing regular files or symlink leaves and atomically selects their pinned bytes, preserving unselected source and the authored head. Paths are relative to `--from` (default project directory), explicit and raw-byte-preserving on Linux and macOS. Add `--recursive` to include directory descendants, including new files and empty directories; `.` then selects the whole root. Symlinks remain leaves and are never followed. Review `diff BEFORE_ROOT AFTER_ROOT`, then `commit`; commit never rescans the filesystem. Cancel with scoped `history undo CAPTURE_KEY` before a later operation takes ownership. Cancellation authors nothing and leaves all live edits untouched. Exact retries recover the original capture even after live files disappear. Receipts list the exact requested scope and inclusion/removal policy; capture reports observed counts, removed bindings and unobserved administration paths. The library exposes retained versioned observation witnesses separately. A damaged optional report warns without hiding a committed/recovered receipt.

To capture deletions, explicitly add **`--recursive --remove-missing`**. You may name a tracked missing path directly, for example `capture --recursive --remove-missing --expected-generation N --request-key KEY src/deleted.rs`: its existing parent's complete name listing proves absence without capturing modified or new siblings. An existing directory scope such as `src` still includes its full descendants. Each removal is reported, including that removing a directory binding drops its selected descendants. Ordinary `--recursive` still refuses missing selected bindings. Removal policy is part of retry identity, even if a particular capture removes nothing. A later file recreation cannot silently enter a commit of the pinned result. Scoped undo restores the old selected source, not deleted live files.

All modes require 1–1024 explicit scopes and at most 64 KiB of requested path bytes. Without `--recursive`, directories refuse. Untracked missing requested paths, missing filesystem parents, unreadable selected paths, hardlinked regular files, unsupported selected types, detected races, explicit administration paths and symlink parents refuse without selecting partial results. With `--remove-missing`, a missing tracked file, symlink or directory requires an existing parent that can be fully enumerated; if that parent is also missing, select the tracked missing ancestor whose parent exists. Unselected sibling payloads and types do not enter a direct absence capture. Replacing a selected directory with a file/symlink still refuses rather than implicitly dropping children. Recursive scans preserve excluded administration source bindings, even if absent live, and refuse to prune an ancestor containing such protected data or unresolved conflicts. Default tracked-file scans, ignore rules and hunk selection remain unimplemented. File permissions project to the owner executable bit; symlink targets remain raw leaf bytes. Witness checks are race indicators, not a claim that all files coexisted at one instant.

**Later interactive workflow (not implemented):**

```sh
avc status
avc diff
avc commit -m "Implement parser"
avc commit --interactive -m "Split validation"
avc commit src/ README.md -m "Initial source"
avc amend --interactive
avc rebase --onto main
avc history undo
```

In the broader target workflow, default `commit` selects changed **tracked** files and proven deletions, but requires explicit path/interactive inclusion for **new** files; ignore rules are selection aids, not secret detection. Recursive directory deletion is conspicuously previewed and requires explicit selection. An unreadable path is not proof of deletion. Interactive file/hunk selection constructs and **pins an exact candidate** from captured bytes and recorded expected base; confirmation never rereads the working directory to silently include later edits. Cancellation authors nothing. Unselected edits remain live; unselected observed data may be privately checkpointed under explicit policy but isn't an unbounded permanent commit.

History rewrites create immutable new revisions; a no-op rebase can retain the existing one. Amend/rebase preserves the logical ChangeId while `revision` references remain exact. A change may become **empty** when its result is already present on the new base: report this explicitly, retain the old immutable revision under scoped undo protection, and do not promise that a later rebase will automatically reconstruct latent edit intent from an empty source diff. Split/fold creates explicit new identities and mappings. Shared refs never automatically follow a local rewrite. `history undo` conditionally returns the scoped *local history selection*; it does not claim to revert current filesystem files or remote publication.

## 4. Switching, checkpointing and recovering

```sh
avc checkpoint
avc work list
avc work new hotfix --from main
avc work resume hotfix
avc switch main
avc recover --list
avc recover CHECKPOINT --to ../recovered
```

Serial work may reuse one directory. `work new` creates an independent effort, not merely an alternate name for an equal base. For simultaneous ordinary filesystem writers use distinct workspaces/destinations; checkout-free agents can skip materialization. A clean `switch` selects an exact authored revision within the current work. A dirty/pending `switch` must capture and pin affected bytes **before** clearing files; refusal is mandatory if coverage, quota or obstacle handling cannot be made safe. Never silently carry dirty edits to another base or discard an untracked/ignored collision.

Explicit `checkpoint` records included tracked/nonignored-new scope, unknown/ignored omissions, exact captured bytes, and current base. Manually pinned checkpoints remain protected until explicit discard. Routine auto-protection (if separately enabled) can expire after **seven days** only with expiry disclosed at acknowledgement; unfinished work deliberately preserved for return stays protected until verified resume or explicit discard. `status` alone is never a recovery checkpoint.

`recover` defaults to a **fresh directory**; in-place restore requires exact before-image/path checks and explicit consent. A crashed or raced restore retains protected old/new values and a pending operation receipt; it does not overwrite a later editor write. Source switching doesn't restore services, dependencies, env, unsaved editor buffers, build outputs or VM state.

## 5. Merge and conflicts

```sh
avc merge feature
avc rebase --onto main
avc resolve
avc show REV:path
avc diff REV1 REV2
```

Only a complete, verified merge-base context permits a completed merge. With multiple maximal common ancestors, **v1 returns `AmbiguousMergeBase`** and requires an explicit verified `--base REV` selection with preview; it does not invent a recursive virtual base. See [format](format.md). An unavailable promised/foreign ancestor leads to `NeedObjects`/`Pending`, **not a fabricated root**. Different resolutions of an unresolved comparison base return `UnresolvedBase` with no new source; choose an explicit resolved base or explicit replacement rather than accepting algebraic deletion/resurrection. Recorded relocation candidates that interact with another candidate currently retain whole-source alternatives; resolving such a root is an explicit whole-source choice, not independent rename/delete path rows. Trivial equal/base cases and safe disjoint text/mode/directory edits combine; ambiguous binary/type/delete/overlap cases retain *normalized signed source conflict terms*. The CLI renders conflicts with side/base labels for inspection, but marker text in the ordinary filesystem is a view, not the canonical conflict value. Resolution explicitly binds a selected result to the original conflict version. Git push refuses unresolved native conflicts unless deliberate conversion semantics are selected.

A failed computation due to deadline/memory/I/O is `Pending` and protects inputs, not a semantic conflict. An automatically clean merge is **not** a claim of program correctness; optional test/review evidence binds the exact candidate and environment.

## 6. Parallel changes and composed views

```sh
avc change new api
avc change new docs
avc change stack ui --on api
avc view api,docs
avc commit --change docs -m "Document token format"
avc absorb --preview
```

A stack selects exact revision versions and dependencies. A combined view is a **derived, reproducible candidate source**, not a synthetic authored merge commit or a second source history. Selected hunks belong to the explicitly chosen change when ambiguous; don't assign by timestamp or inferred rename intent. `absorb` may suggest moving a later hunk to an earlier change that unambiguously owns its original lines and previews every dependent restack; ambiguous or collided edits remain where they are.

One composed directory is convenient for humans and cooperative agents with known write scope. Other agents should use isolated mutable directories or conditional source APIs. Merely labeling changes as separate stacks does **not** isolate generated files, open descriptors, ports, databases or service state.

## 7. Native source API and agent outputs

Minimum typed library/API behavior:

```text
read(root,path,fetch_policy) -> Value | AbsentProven | Promised | Denied | Corrupt
apply(root, [Put/Remove/Move with exact expected prior value])
      -> CandidateRoot | Stale | Conflict | NeedObjects | Refused
merge(base,left,right,driver_version)
      -> CleanRoot | UnresolvedRoot | Pending
publish(ref,expected_ref_version,revision,request_id)
      -> Accepted | StaleRef | Unauthorized | NeedObjects
```

Agents can operate without Git, checkout, shell, or local file copy. An authenticated native path-proof-derived source candidate may be **conditionally valid** despite missing untouched sibling bytes; only a full holder may acknowledge publication of a complete closure. A foreign filtered Git view does **not** have native IDs for its missing blobs and must stay in the Git adapter. Explicit caller permissions and path-scoped disclosure still apply.

## 8. Fetch, push, verify, export

```sh
avc fetch origin
avc push origin main
avc verify --full
avc export --full --output backup.avc
```

`fetch` updates explicit remote tracking refs, not automatically the current work/files. `push` uses exact expected remote version/lease and rejects concurrent movement, incomplete source, unauthorized refs or lossy Git conversion; a lost result is reconciled by operation ID/actual remote state, not assumed failed. `verify` distinguishes complete, promised, unexpectedly missing and corrupt; `export --full` must pin an exact closure and independently restore without remote. A foreign Git partial view cannot claim native full export.

## 9. User and safety acceptance

Test the actual CLI (not just isolated model scripts): standard small Git import/edit/push; native offline work/backup; native path-scoped query/edit under a remote authorized holder; incomplete foreign Git partial fetch; multiple independent/stacked edits; repeated rebase of unresolved conflict; competing agent writes; a changed source during scan; a crashed multistep switch; retention/GC race once deletion/GC exists; and stale publication retry.

**Command failures must preserve already acknowledged source and report concrete next action.** No `--force` suppresses hash/closure/path safety requirements, and no import/checkout silently runs untrusted repository hooks.
