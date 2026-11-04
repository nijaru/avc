# AVC — intended source-workflow contract

**November 3, 2026.** User-facing behavior to implement/qualify, not an existing CLI. The authoritative source model is [design.md](design.md); implementation and release gates are in [roadmap.md](roadmap.md). Commands below are examples, not irrevocable interface commitments.

## Everyday model

Users have **source work**, **changes**, and **remote sharing**.

- The working directory contains files external processes may change. It is not a trustworthy atomic snapshot.
- A **change** is an editable unit with a stable logical identity and an immutable revision every time it is authored/revised. A single user need not name every effort in advance.
- A **checkpoint** protects exact captured bytes and recorded base privately. It does not commit, track, approve, or publish those bytes.
- A **repository operation** (switch, amend, restack, ref move) alters local selections through a durable transaction with scoped undo. Restoring working files is separate and must be guarded.
- A **publish** chooses a candidate and conditionally updates a remote target after durable object transfer. Authored does not mean shared.

The CLI prioritizes a clear default for ordinary work; expert flags/API expose precise scope and IDs. Do not surface internal tree nodes, operation receipts, GC leases or foreign-ID tables by default.

## Clone, read, work

Normal source-first behavior:

\`\`\`sh
avc clone https://github.com/owner/project
cd project
avc status
avc diff
avc commit -m "Handle escaped input"
\`\`\`

Select the requested remote ref or the server-advertised default branch's **currently resolved tip**. For an ordinary Git source, download the selected tip's complete current source and compute a native source root; historical blobs need not all be local. Clearly mark any **foreign ancestry frontier** rather than claim a fully imported native DAG. For a native AVC remote, use its authenticated root and selectively fetch current objects.

A shallow history does not mean a sparse working directory:

- \`avc clone URL --ref feature/api\`: choose another exact ref.
- \`avc clone URL --full\`: request a completely verified, self-contained selected closure and ordinary materialization.
- \`avc clone native-URL --paths src/api/,Cargo.toml\`: obtain a path-scoped native thin client rooted at an authenticated full native source.
- \`avc clone URL --no-checkout\`: no materialized directory, explicit cached-source APIs.
- \`avc hydrate PATH\`, \`avc hydrate --history=all\`, \`avc hydrate --all\`: expand exact scope/history/offline custody.

**Do not pretend that a filtered Git tree is an already-verified native tree.** For a huge Git source, offer a clearly labeled *foreign partial view* that can inspect and make conditional Git-relative edits through the Git adapter, or ask to hydrate/convert complete current source before native authorship. The bridge remains a useful practical path; it must expose the conversion cost and foreign IDs honestly. Avoid hidden filesystem downloads or implicit fallback to full monorepo checkout when a scoped command fails.

For native thin clients, a missing promised object is not an absent file. \`show\`/\`diff\` may fetch missing objects when online, under explicit repository policy, and cache verified data; they are still **read-only in source/history terms**. \`--offline\` forbids fetches. \`status\` operates locally by default and may report incomplete coverage. All commands must distinguish absent, not hydrated, offline, unauthorized and corrupt.

A normal developer should get a usable directory without needing FUSE or a daemon. Agents can query and edit **by immutable source root** without any working copy. Full offline custody must be explicitly hydrated and independently exportable.

## Author exactly selected edits

\`\`\`sh
avc status
avc diff
avc commit -m "Implement parser"
avc commit --interactive -m "Separate validation fix"
\`\`\`

A simple commit includes selected tracked modifications and clearly disclosed deletions; new files and ignored paths are handled by visible inclusion policy, never via accidental past capture. File/hunk selection pins an immutable candidate and its exact base. Confirmation authors the displayed candidate; it must not silently rescan files to include later changes. Unselected edits remain in the normal workspace.

A failed/incomplete scan is **not** deletion evidence. If external writers race a capture, AVC can report what it observed but cannot claim all captured files existed together at one instant. An operation that risks deleting or overwriting source protects the affected observed bytes first, then refuses on stale destination preconditions. No blanket \`--force\` makes a corrupt/incomplete candidate safe.

Neither source reads nor import/commit/restore executes source-provided hooks, build commands, merge drivers, or agent tools without explicit authorization.

## Editable history and multiple changes

\`\`\`sh
avc amend
avc rebase --onto main
avc history undo
\`\`\`

Amend/rebase produce new immutable revisions while keeping an appropriate stable ChangeId; divergent versions, revisions with equal trees, and many-to-many split/fold mappings are explicit. Shared/publication status must be checked before moving remote refs or rewriting already published changes. Undoing an authored history operation may restore a local selection without altering working files; source recovery is a different verb.

A **change stack** groups exact ordered change versions with a base and dependencies. A developer can select several independent/stacked changes in one *composed* working view:

\`\`\`sh
avc change new api
avc change new docs
avc view api,docs
avc commit --change docs -m "Document tokens"
\`\`\`

The combined view is reproducible and reviewable but not an implicit permanent merge commit. Assignment of pending hunks to separate changes is explicit when ambiguous. Optional \`absorb\` may propose moving a later hunk into the earlier owned change, showing a **preview of all dependent restacks** and refusing ambiguous ownership. A conflict is not silently hidden by a different application order.

Human or agent reviewers can pin a combined candidate, run tests against its exact source, and separately publish representable stacks. Test results belong to the exact candidate and environment, not a floating stack name. Shared directories are optional for cooperative agents; concurrent agents with unknown write overlap should have independent writable working copies or checkout-free conditional source edits. AVC does not isolate their process state, package caches or credentials.

## Checkpoint, interrupt, restore

\`\`\`sh
avc checkpoint
avc recover --list
avc recover CHECKPOINT --to ../recovered-work
\`\`\`

An acknowledged checkpoint records exact captured bytes, base, included and omitted paths (including untracked), retention, and any observation limits. It is private, not a way to track or publish untracked files. Preserving an unfinished effort across a switch must retain its selected source until deliberate resumption/discard. Normal old checkpoints can expire under visible policy; neither operation may silently retire the only protected copy of stated work.

Recover into a new directory by default. In-place restore requires matching the recorded before-image/path preconditions. A failed or interrupted materialization retains both protected source and a recoverable operation receipt; stale editor writes are never blindly rolled back.

No command can promise unsaved editor buffer, running database, process, VM, ignored secret or environment recovery without an additional explicitly scoped integration.

## Typed conflict and conditional source edits

A merge has explicit inputs/base, returns either a clean exact source, a source with **typed unresolved conflicts**, or a pending/aborted result when data/resource checks fail. Text markers are only one possible editor presentation. Resolve selected binary/type/rename/delete cases as full source values, with source alternatives preserved until resolution is authored. A successful merge is not proof of semantic correctness.

Native structured API (illustrative):

\`\`\`text
read(root, path, fetch_policy) -> exact value | promised | absent | denied
apply(base_root, [put(path, expected=BlobId, new=BlobId), ...])
     -> new_root | stale | conflict | need_data
publish(target, expected_ref_version, new_revision)
     -> committed | stale | unauthorized | need_objects
\`\`\`

The agent does not need to open a shell, use a working copy or fetch all of a monorepo. An incomplete thin client may ask an **authorized full holder** to verify and construct the new source; it cannot announce a complete root from unresolved sibling promises. Concurrent updates to the same remote ref use CAS rather than implicit last-writer-wins.

## Git remote workflow

A native project with Git upstream supports import/current branch tracking, fetch/update, revision mapping and export/push of representable states. Foreign Git IDs remain distinct from native BLAKE3 objects. Unsupported first-class conflicts, empty directories, modes, incomplete ancestry, LFS/submodules and signatures must be explicitly understood before claiming lossless conversion. Git server partial-clone features may constrain availability or fetch performance; don't pretend they have AVC's native promises.

A Git mirror/colocated writable Git repository is an **optional separately qualified compatibility mode**; two writers must not race control of the same checkout or refs. Normal native users should be able to keep GitHub/GitLab for collaboration without living in a Git object store.

## UX validation cases

Exercise with the real CLI, not just model tests: clone→edit→push of a normal Git project; native full/offline recovery; sparse native agent read→edit without checkout; huge foreign Git source with incomplete current blobs; two independent and one dependent change; same-file hunk attribution; repeated rebase of unresolved conflict; stale path CAS; competing agent publication; crash while writing objects, switching files, and exporting/backup.

A successful task is a safer, faster, clearer **entire task**, not the fewest commands or the smallest download in isolation.
