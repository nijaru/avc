# AVC — user experience and source-workflow contract

**Revised November 3, 2026. Working design, not an implemented CLI or a usability-validated interaction model.** The [architecture](design.md) owns semantics, the [roadmap](roadmap.md) owns sequencing and gates. The examples below are illustrative and may change after task trials.

## The everyday mental model

AVC works with **source, changes, and sharing**:

- A **workspace** is where a person or tool edits files. Local mutable files are not themselves a durable immutable source state.
- A **change** is a unit of work with a stable logical identity and one or more immutable authored revisions. A change may be amended, rebased, split, folded, stacked, or discarded under explicit policy.
- A **checkpoint** privately preserves explicitly observed bytes and base/scope for recovery, without making them an authored change or sharing them.
- **Publishing** transfers exact source/history and updates a shared target only if its expected version is still current.

Users should not have to understand tree layout, object references, database generations, or internal receipts to do normal work. The CLI should display exact IDs and scopes when necessary for disambiguation and automation.

`status`, `show`, `diff`, `log` and read-only source queries do not secretly author, checkpoint, fetch, execute code, or publish. An explicitly enabled background policy is separate and must report its actual scope and failures.

## Clone an existing project

The normal source-first flow should be as simple as:

```sh
avc clone https://github.com/owner/project
cd project
avc status
```

For a Git URL, AVC uses an **importing Git boundary** and records the remote, foreign identities and available history. It does **not** make Git its native content store. For an AVC native remote it negotiates native immutable identities and promised data.

By default AVC checks out the remote's designated default branch or a specifically requested ref at its **latest observed tip**. It should download only the history and content needed for the initial useful task; not all historical revisions or every old blob.

Illustrative explicit controls:

```sh
avc clone URL --ref feature/api
avc clone URL --paths src/api/,Cargo.toml
avc clone URL --no-checkout
avc clone URL --full
avc hydrate src/api/
avc hydrate --history=all
avc hydrate --all
```

A large monorepo may open with a sparse or virtualized working set. Explain the retained source scope, whether deeper history is local, and whether network fetches may occur. For ordinary local editors and build tools that expect files on disk, either materialize them or explain that they are absent from the selected scope; optional FUSE/virtual-filesystem integration must not be required for a normal clone.

`--full` has an unambiguous meaning for the selected scope: every object needed to reconstruct the claimed refs and source is local, checked and available without the remote. `--paths` is not equivalent to full offline repository custody. Missing **promised** objects are not "deleted" or "untracked". Failed authentication, unavailable remote, denied path and corruption are reported as distinct outcomes.

For source-only ephemeral jobs, a very shallow history may be sensible. For long-lived development, choose a policy that avoids repeated Git shallow-boundary performance penalties. Native history expansion is first-class, not just a wrapper around `git fetch --deepen`.

## Edit and commit

```sh
avc status
avc diff
avc commit -m "Handle escaped input"
avc commit --interactive -m "Split validation fix"
```

Proposed interaction default: show already-tracked modifications and deletions, make new-file inclusion understandable and deliberate, and provide path/hunk selection without a mandatory persistent staging index. Important directory removals and newly selected files need visible scope. No file or hunk is committed from bytes that changed after its confirmed preview: the candidate is built from **pinned observed bytes** and exact preconditions.

A commit makes immutable authored history. It does not prove the mutable directory was atomically snapshotted; the result records a meaningful observation scope. An unsuccessful capture/ref advance does not appear as a successful commit. Protect pinned prepared content until the operation is resolved under explicit retention rules.

An untracked file is not automatically published just because an earlier checkpoint happened to capture it. Ignore patterns are selection conveniences, not secret scanners. AVC must never execute repository hooks or arbitrary source programs during commit or checkout unless the user deliberately authorizes them.

## Keep several changes in flight

Borrow GitButler's useful **parallel and stacked changes**, without requiring Git's synthetic merge commit or a single shared agent filesystem.

A user can maintain independent changes `api` and `docs` based on the same source revision, plus a dependent `ui` change stacked above `api`. They may:

- Select which stacks are included in the current **composed view**.
- Assign chosen uncommitted file/hunk edits to a particular change when needed.
- Inspect, test and publish one change or an ordered stack without accidentally publishing the others.
- Move a change between dependency positions with an explicit rebase or conflict result.
- View the integrated candidate and its exact constituent change versions without silently making it canonical history.

Example interactions (names and commands **not selected**):

```sh
avc change new api
avc change new docs
avc change stack ui --on api
avc view api,docs
avc commit --change docs -m "Document token format"
avc diff --change api
```

If a workspace has several applicable destinations, a commit must request or infer an unambiguous selected change; it must **not guess** from timestamps or claim file/hunk ownership from a generic watcher. Changes touching the same file can be independent if their exact transformations compose; overlapping incompatible transformations and path/type/delete collisions remain explicit conflicts.

One combined workspace is convenient for a human. Agents writing concurrently ordinarily use independent writable directories or immutable conditional source-edit APIs, and can compose/review without copying an entire historical monorepo. AVC cannot promise safe concurrent writes by several external processes to the same file without an actual controlled-writer boundary.

## Checkpoint, switch, and recover

AVC should preserve affected observed work automatically before **its own** potentially destructive source-materialization actions, not manufacture commits for every interruption. An explicit checkpoint remains useful:

```sh
avc checkpoint
avc recover --list
avc recover CHECKPOINT --to ../recovered-work
```

Checkpoint output must report **what was captured**, any excluded/failed paths, source base, age and retention. When outside writers race the scan, there may be no provable single filesystem instant. Unsaved editor buffers, ignored secrets and process environment are not covered unless separately captured and explicitly reported.

Returning to a prior effort is not the same as visiting an old authored version. A dirty switch must protect the departing source and check destination obstacles before overwriting anything. Recovery normally defaults to a new directory; in-place restore requires exact before-image/path checks and informed user intent. An interrupted file update must not blindly roll back over a later editor write.

A durable local operation log supports **history undo** with exact scope. Undoing an authored history operation does not necessarily revert current filesystem bytes; recovering a checkpoint does not change the shared branch. This distinction belongs in command help and result summaries.

## Merge, rebase, and conflict UX

```sh
avc rebase --onto main api
avc merge api docs
avc resolve
```

Fixed source values must produce deterministic results under a specified merge driver version. A normal text merge should handle common disjoint edits efficiently; when the source or intent is ambiguous, AVC carries **typed unresolved alternatives**, rather than recording accidental marker text as clean source. Repeated rebases must not silently erase earlier alternatives.

The CLI should show conflict paths, kinds, side/source identities and a useful text presentation. Resolution commits the exact selected result after review. Binary, symlink, file-vs-directory, delete-vs-modify and rename/move cases must have meaningful whole-entry choices. If merge work cannot finish due to timeout, memory, missing promised input or I/O, preserve all candidates as **pending/aborted**; this is not a semantic conflict.

Tests, external reviews and agent evaluations should bind an **exact revision or candidate tree and scope**, not a mutable change name. A successful test of one composed view is not proof that publishing a different stack is safe.

## Native source APIs and agent behavior

A coding tool should be able to perform the following without mounting or copying a full repo:

1. Resolve an authorized ref/version and list available paths at a specific immutable tree.
2. Read exactly selected file/subtree objects with explicit remote-fetch policy and cache status.
3. Compute diff, affected paths, ancestry/merge-base or a source search using derived indexes.
4. Propose conditional edits such as `put(path, expected: BlobId, new: BlobId)` against a particular source revision.
5. Obtain a new immutable candidate or structured stale/conflict error; publish it only with guarded target/version permissions.

No agent receives implicit permission to execute source hooks, fetch private remote content, cross path-scoped ACL boundaries, mutate another agent's workspace, or force publication because it can read an object. Typed API outcomes must make incomplete/unknown state as explicit as data conflict.

## Git interoperability in routine development

Native AVC users should be able to keep GitHub/GitLab as upstream: clone/import, update upstream tracking, author native changes, and export/push representable commits. If a remote was rewritten, if the source cannot be represented in Git, or if the requested update would overwrite another writer, **stop with a precise diagnostic**. Keep original foreign IDs and native IDs distinct.

Colocated Git and AVC working directories are not a default: two unrelated CLIs writing the same refs/files create avoidable races. Evaluate an optional Git compatibility mirror when user workflows prove its benefit, with one explicit source of truth.

## UX qualification

Test these real journeys before selecting commands and default prompts:

- Small GitHub clone, edit, commit, push, update/rebase, repeat.
- Monorepo checkout limited to a path, agent reads a previously absent file, later expands scope and fetches old history.
- Offline work on a fully hydrated repo; partial repo fails honestly when missing data is needed.
- Human working on two independent and one stacked change, including same-file hunks, selective publish, conflict/rebase.
- Two agents in separate workspaces racing a shared destination, then a composed review.
- Crash or interruption during capture, commit, materialization, GC and Git publication; restore without data loss.

Preferences about prompt count, command spelling, automatic file assignment and directory reuse are **product hypotheses** until observed. Preserve correctness and clear disclosure while selecting the simplest interface that actually works.
