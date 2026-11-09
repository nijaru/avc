# AVC user workflows and command/API contract

**Design baseline 2026-11-04; authored commit/log slice added 2026-11-09.** The README lists today's experimental commands. Basic authoring and first-parent history are implemented; history rewriting, capture/checkpoints, workspaces and synchronization below remain **target workflows**, not available commands. Commands below are **selected target spellings**, subject to usability adjustment *before public release*. The [architecture](design.md) owns invariants, [format](format.md) owns immutable data/merge, [storage](storage.md) owns recovery, [sync](sync.md) owns native remote behavior, and [git](git.md) owns the Git adapter. Do not implement duplicate state machines in the CLI.

## 1. User mental model

AVC has **source**, **changes**, **unfinished work**, and **sharing**.

- A **source root** is an exact immutable snapshot, with typed unresolved conflicts when necessary.
- A **change** is an authored unit (stable ChangeId) with an exact immutable RevisionId that changes on amend/rebase. A stack is an ordered group of specific change versions.
- A **work** is one resumable local effort, including pre-first-commit edits, with a base, a current selection and protected checkpoints. Naming is optional; distinct unfinished work cannot be identified solely by equal file bytes.
- A **checkpoint** protects captured bytes and exact scope privately; it neither tracks/publishes new files nor creates an authored revision. Explicitly protected unfinished work survives ordinary checkpoint expiry.
- A **remote ref** points to an exact authored revision. A push conditionally changes it after required object transfer. Tests and reviews bind exact candidate versions, not floating labels.
- The **filesystem** is mutable by other tools and cannot be assumed to be the same as last captured source; editor unsaved buffers, processes, dependencies and ignored environments are outside this guarantee.

By default, `status`/`log` operate locally; `show`/`diff` can read-through-fetch promised immutable data under configured online policy. None of these read-only commands creates commits, checkpoints, moves refs, runs hooks or discloses private source. Use `--offline` to forbid fetches.

## 2. Starting and cloning

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

**Available now:** `avc --experimental commit --author 'Name <email>' -m MESSAGE --expected-generation N --request-key KEY` authors the already-selected source, without scanning or changing files. `AVC_AUTHOR` can supply author identity. Init authors nothing; explicit empty/unchanged-source commits are allowed. Source edits preserve the last authored head, while commit advances it under the same generation. Identical retries return the original Revision/ChangeId/time; changed identity or cross-operation key reuse refuses. `avc log [--revision ID] [--limit N]` shows newest-first first-parent metadata (default 20); `status` shows root, authored head/unborn and generation, never a fabricated filesystem-clean claim. Metadata output escapes terminal controls. Source `export`/`import` does not transfer this history.

**Target capture/editing workflow (not implemented):**

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

Once capture exists, the target default `commit` selects changed **tracked** files and proven deletions, but requires explicit path/interactive inclusion for **new** files; ignore rules are selection aids, not secret detection. Recursive directory deletion is conspicuously previewed and requires explicit selection. An unreadable path is not proof of deletion. Interactive file/hunk selection constructs and **pins an exact candidate** from captured bytes and recorded expected base; confirmation never rereads the working directory to silently include later edits. Cancellation authors nothing. Unselected edits remain live; unselected observed data may be privately checkpointed under explicit policy but isn't an unbounded permanent commit.

History rewrite is an immutable new revision. A stable logical ChangeId can follow amend/rebase but `revision` references are exact. A change may become **empty** when its result is already present on the new base: report this explicitly, retain the old immutable revision under scoped undo protection, and do not promise that a later rebase will automatically reconstruct latent edit intent from an empty source diff. Split/fold creates explicit new identities and mappings. Shared refs never automatically follow a local rewrite. `history undo` conditionally returns the scoped *local history selection*; it does not claim to revert current filesystem files or remote publication.

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
