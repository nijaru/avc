# AVC user workflows and CLI contract

**Selected behavior; candidate command spelling, not implemented.** This is the user-facing companion to [the architecture](design.md). It owns interaction defaults and workflow acceptance cases, not another object/storage specification. Examples describe the native design, **not** the existing Git-wrapper CLI. Names and presentation must survive task-based qualification before becoming stable.

## 1. The everyday model

Users work with **files, local commits, and shared work**. A private **checkpoint** is an acknowledged recovery capture, not a commit and not publication. Task labels are local navigation aids, not canonical Task objects. Serial work normally stays in one project directory; extra writable directories are an option for concurrent work, not a prerequisite for changing tasks. Ordinary editors/directories and offline operation are the default; no daemon, mount, IDE extension or server is required.

Use familiar words where their meaning survives: `status`, `diff`, `commit`, `amend`, `merge`, `log`. Use **checkpoint** consistently for private recovery. Do not carry forward the prototype's auto-commit/squash model or introduce a second `save` verb with an overlapping meaning.

Default status answers four questions without displaying internal object kinds or generations:

```text
Workspace: parser-cleanup
Files: 3 modified, 1 new file not included
Recovery: tracked edits captured 20 seconds ago; retained for 7 days
History: 2 local commits; no publish requested here
```

Those lines describe evidence, not a permanently clean filesystem. If current files differ from the last checkpoint, show that difference; do not call them saved merely because a checkpoint exists. A checkpoint has a scope and deadline. Untracked/ignored files and unsaved editor buffers are not protected unless explicitly captured. Unknown sharing or recovery status is **unknown**, not an optimistic success.

`status`, `diff`, `show`, `log` and listing checkpoints are read-only repository operations: no implicit capture, authoring, history rewrite, fetch or hook execution. Background recovery is an explicit workspace policy; it must report failures/gaps and never manufacture authored commits. Quote untrusted paths/descriptions safely, preserve non-UTF8 names losslessly, and bound default output.

## 2. Selection and preview

- `commit` with no path selection includes modifications and proven deletions of already tracked files. New files require explicit path scope or interactive selection; a directory path includes its eligible new files. No-op ordinary commits report “nothing selected,” not an empty authored revision. Explicit history/metadata operations may legitimately keep the same source State.
- Explicit directory paths include non-ignored new files under that scope and the parent directories needed to represent selected entries, not unrelated siblings. First-run help must say this plainly. Ignore rules are selection aids, not a secret detector or erasure mechanism. Ignored paths need a separate explicit inclusion choice.
- A failed/incomplete directory read is never a deletion. Show deletions separately in the preview; deleting an entire tracked directory subtree requires explicit scope/interactive selection. No arbitrary file-count threshold or generic `--force` bypass.
- Interactive file/hunk selection is supported without a mandatory mutable staging index. Selecting a new-file hunk explicitly selects that file's creation. Binary/type/mode changes use whole-entry choices where hunk selection has no defined meaning.

Checkpoint selection follows the same default tracked/new-file inclusion rules as commit; a bare checkpoint does not silently add untracked files. Capturing an observed deletion is not authoring consent: check whole-directory deletion choices when promoting the checkpoint. With no eligible changes, report that no checkpoint was created and explain explicit scope selection.

A filesystem authoring command uses the capture/admission path, not a weaker second scanner. Pin the observed bytes, build an **immutable selected candidate**, and show its actual diff against its exact base. Hunk choices refer to those pinned versions, never offsets into a subsequently reread live file. Keep required Observation provenance; a constructed partial-file candidate must not be described as a byte-for-byte filesystem snapshot.

Confirmation authors exactly the previewed candidate. Later editor changes remain uncommitted; do not rescan them into the commit or roll them back. Selection/base changes by another operation invalidate the plan and require re-review. The local commit and requested selection advance are one generation-checked transaction; on failure the prepared checkpoint remains recoverable, with its actual deadline. Read-only status never creates that checkpoint.

Cancellation creates no commit. Report any recovery checkpoint already retained. Unselected observed edits may be kept in a separate private recovery checkpoint; show its scope/deadline. They are **not** committed or retained forever just because another portion was committed. Do not pretend to infer which later live edits came from excluded hunks. The saved checkpoint remains the exact comparison point.

## 3. Concrete workflows

Syntax below is illustrative. `<change>` and `<checkpoint>` mean references shown by AVC, not additional objects users must construct.

### Start and make the first commit

```sh
avc init
avc status
avc commit README.md src/ -m "Start parser"
```

`init` creates a native repository and empty unauthored workspace base, **not a synthetic authored commit**. Existing files remain unchanged and initially untracked. A bare first `commit` reports that nothing was selected, the observed number of eligible new files (or incomplete coverage), and an explicit include-scope example; it must not appear to succeed while recording nothing. The first real commit has no parent. Do not silently wrap/import a Git or prototype repository; refuse before changes and explain that conversion is a separate, deferred adapter.

### Make a selective commit and keep working

```sh
avc diff
avc commit --interactive -m "Handle escaped input"
avc status
```

The interactive flow selects files/hunks, shows the exact resulting source and omitted changes, then confirms once. It leaves working files untouched and reports remaining differences against the new commit. No separate stage/add command is required.

For review or tests before authoring:

```sh
avc checkpoint --interactive
avc workspace new review --from <checkpoint> --path ../parser-review
# Run the project's tests there using ordinary tools.
avc commit --from <checkpoint> -m "Handle escaped input"
```

The review workspace is initially materialized from the candidate, not later edits in the original directory. It is still an ordinary writable directory, not a test-isolation boundary. AVC does not become a build runner; test evidence can bind that candidate only when execution actually consumes it under its declared input/writer boundary. Editing the review workspace invalidates that claim. Promotion uses the checkpoint's recorded history base, not a search for an equal State. Default promotion must match the current workspace base before advancing it; otherwise offer a workspace at the checkpoint's base, not a silent reparent. State-only recovery with no recorded revision requires an explicit parent/root-authoring choice, except the initial empty repository's first commit.

### Interrupt a task and return in the same directory

```sh
avc switch <hotfix-change> --preserve-current
# Edit and optionally commit the hotfix using ordinary tools.
avc switch --back --preserve-current
```

This is one preserve-and-switch flow, not a checkpoint/workspace/`cd` ritual. The first command privately parks the observed tracked work and its exact history base, then materializes the requested change **without carrying those edits onto it**. The return parks any new unfinished hotfix work, restores the original history base and materializes the original observed edits. No WIP commit is manufactured. A later commit still has the original base as its parent, even if that change has since been amended elsewhere.

`--back` means the context departed in this workspace's previous **completed switch**, not the latest commit, a timestamp or history undo. Repeating it toggles between the two contexts; older parked work remains listed with checkpoints and can be selected using `switch --resume <checkpoint>`. Resume restores the checkpoint's recorded base, unlike authoring from a checkpoint. Resolve and pin the exact context before confirmation; never silently follow a rewrite successor.

Parked work is retained **until successful resume or explicit discard**, independently of the ordinary seven-day checkpoint deadline. Status and checkpoint listings distinguish parked work from expiring recovery. This protects the acknowledged scope, not every past checkpoint, excluded hunk or local file. After successful resume, report the handoff to ordinary checkpoint retention; failed/interrupted resume keeps the parked protection. Quota failure refuses before AVC changes files, rather than expiring another parked context. Discarding a pin does not erase copies in other captures, authored history or backups.

Bare interactive `switch` offers preserve-and-switch within the same flow when it finds dirt; noninteractive use requires an explicit policy. A remembered preservation policy may be explicitly configured per workspace, never inferred from one previous override. Preflight still refuses unknown coverage, unsafe names and unpreserved obstacles. New/ignored files are not silently included: show their disposition, explicitly preserve any at risk, or refuse. Preserving a file privately does not track or author it. Carrying edits onto a different base is a separate checked transform, not switching.

Source switching is **not environment switching**. Non-overwritten untracked/ignored files stay directory-local across tasks: `.env`, dependencies, caches and databases are not restored to an earlier task's values. Unsaved editor buffers, open descriptors and running processes are not checkpointed or restarted. Lockfile changes may require project setup. AVC neither runs setup hooks nor copies secrets, installs dependencies or shares mutable dependency directories implicitly. Interrupted writes use guarded recovery, not blind rollback over later editor changes.

### Use another directory when work is actually concurrent

```sh
avc workspace new hotfix --from <change> --path ../parser-hotfix
```

Choose this when another writer, long-running test or watcher needs the original source directory to stay unchanged. The new workspace starts from the named immutable change, not the original directory's unfinished bytes. `workspace new` neither saves nor scans an unrelated writer's directory; checkpoint there separately if needed. Independent workspaces may start at the same immutable revision without competing for an exclusive branch checkout.

A separate directory isolates source-file writes, **not** ports, services, databases, secrets or remote APIs. Dependencies and local configuration need deliberate setup; tests must identify the server/source they actually exercise. Do not silently reuse a server just because its URL answers. This is not a promise of a sandbox or automatic environment provisioning.

Closing workspace registration leaves its directory and parked recovery roots intact. Directory deletion is a separate explicit action with tracked, new and ignored-file dispositions shown; no generic force fallback. Parked work remains discoverable after closure and recoverable into a fresh directory. Extra workspaces must earn their setup, navigation, disk and cleanup costs rather than becoming a directory per small task.

### Amend a local commit

```sh
avc amend --interactive
```

The default target is the workspace's selected commit, not a commit chosen by timestamp; file/hunk selection follows the same deliberate rules and exact candidate preview. A StateOnly selection has nothing to amend: explain how to author a first commit instead. Keep its ChangeId and parents; create a replacement Revision rather than a second unrelated change. Do not rewrite descendants/other workspaces automatically. For an older change or stack rewrite, preview the explicitly chosen affected stack first; unsupported merge-commit rebases fail before changing history.

Show known sharing status before rewriting. Private replacement does not change a remote target or another workspace's selection; do not claim “unpublished” from an unavailable/stale remote view. Prefer a follow-up commit/revert for shared fixes. No automatic force publication.

### Undo history versus recover files

```sh
avc history undo
avc recover <checkpoint> --to ../parser-recovered
```

There is **no bare magic `undo`**. `history undo` previews the latest undoable history edit in this workspace, identifies the effect in ordinary words and confirms before restoring its private selection. Evolution records remain append-only; undo does not erase lineage. Users need not type operation IDs; exact IDs remain available to automation/diagnostics. Switching does not authorize crossing contexts: if a history action's recorded resulting selection no longer matches the current selection, refuse rather than jump to another task's base. Files stay unchanged, prominently stated both before and after. The post-action status shows differences against the restored selection; do not imply that running tests in the unchanged directory tests the restored commit.

| Action | History undo / recovery behavior |
| --- | --- |
| Commit, amend, explicitly scoped rebase | Restore prior private history selection conditionally; keep current files and retained replacement revisions |
| Checkpoint creation | Not an undoable history edit; do not erase an acknowledged recovery receipt or jump past it without explaining the selected history action |
| Completed switch | Return with `switch --back`; history undo does not restore files |
| Interrupted file materialization | Keep parked/before-image protection; recover into a fresh directory or deliberately finish guarded recovery before switching again |
| Shared acceptance, another workspace's operation, arbitrary editor write | Never undone by a local history command; shared source fixes use an explicit new commit/revert |

`recover` defaults to a newly allocated directory/workspace and never overwrites the original. Say which paths come from the checkpoint and which are inherited base values; a partial checkpoint is not a whole-directory time machine. Known failed/skipped coverage requires an explicit partial-recovery decision. An in-place recovery is a separate guarded action with preservation policy, not the normal recovery path. Expired/unavailable data is reported honestly; an audit ID is not a promise that bytes remain.

### Merge and resolve without losing the task

```sh
avc merge <change>
avc resolve
avc amend
```

An explicit merge can create/select a **local authored merge containing typed conflicts**; it does not publish it. There is no hidden index-based “merge in progress.” Show the new change, exact conflict units and recovery point. Work can continue and agreed paths remain readable. Resolution may amend that merge while preserving its parents, or be recorded in a deliberate follow-up commit. Clean source is not proof that tests pass.

For text: edit the generated presentation, review the proposed source replacement, and confirm it. `commit --interactive` may combine that review/confirmation with authoring; a separate resolve ritual is not mandatory. Confirmation binds exact replacement bytes. If those bytes change before authoring, re-review them rather than silently refreshing the accepted resolution. Untouched bound markers remain conflicts; ordinary marker-like file contents are not parsed as conflict identity.

For binary/type/directory conflicts: show typed alternatives and materialize a user-chosen alternative in a scratch workspace for inspection/construction. External tools require explicit authorization. Confirm the resulting typed value, not an inferred filename/content match. Use input descriptions/identities, not context-dependent “ours/theirs” labels alone; unavailable origins are labeled as such, not invented.

A coupled rename/directory region is **one resolution unit**, even if its canonical location is the root. Show every affected binding together and explain the decision, e.g. “choose a destination for src/parser; keep lib/parser or pkg/parser.” Path rows are previews, not permission to resolve one coupled binding independently. Validate the whole replacement unit and preserve agreed siblings. Never show merely “entire repository conflicted.”

If a resource limit prevents a representable result, show “could not safely combine; inputs retained until …,” with the pending receipt and next choices. Do not manufacture a conflict State. If materialization fails, separately report the saved source, unchanged/changed paths, incomplete workspace and the fresh-directory recovery command; history undo is not crash repair.

### Share deliberately (later protocol stage)

Local commit/checkpoint commands never publish, run CI or execute repository hooks. A future publish preview distinguishes intended source/history, public provenance metadata, explicitly supplied audit evidence, and remote target/validation status. Acceptance, validation and deployment are different outcomes.

Source export follows **owning history/source references**, not non-owning audit commitments. Amending away a non-parent private predecessor must not transfer that predecessor's bytes merely to retain an explanation. Real parent history is still included. Public provenance can reveal paths/fingerprints and is not metadata anonymity; explicit audit-evidence export can include discarded versions/secrets and needs a separate disclosure choice. Default authoring keeps private capture receipt IDs in local audit metadata, not in public provenance. Do not strip metadata or change already-created RevisionIds secretly during publication.

On an unknown remote result, query the same durable receipt. Never tell an agent to delete its workspace until the receiving authority confirms pinned source closure and its retention/durability domain. Git hosting/IDE compatibility is not provided by the native design. The Git interoperability interface remains open; it must declare representable subsets and losses, separately from full-fidelity native exchange.

## 4. References, errors and automation

Default history displays stable short change references and a version distinction when needed, not State/Revision/generation machinery. Resolve a change to its unique visible rewrite successor in the requested workspace/root scope (by default its selected authored ancestry, not every retained recovery/orphan root); retained superseded versions do not make every amend ambiguous. Divergent live successors and short-prefix collisions require disambiguation. Never choose by timestamp. An explicit immutable revision reference addresses an older exact version. A prepared mutation pins its resolved references before confirmation, so later alias movement cannot retarget it.

Errors state **what happened, what was saved, what did not change, and one concrete next action**. Do not bury successful source custody inside “command failed,” or describe an incomplete checkout as success. Separate canonical source availability, selection update, materialization and remote acceptance in machine output. JSON/versioned outcomes, meaningful nonzero statuses for blocked/incomplete/unknown operations, exact receipt lookup and caller-scoped retry keys are qualification requirements. Noninteractive commands never guess at interactive choices, silently broaden scope or retry a changed request under the same key.

## 5. Qualification, not UX claims

These workflows are paper contracts, not usability-test results or a running CLI. Compare representative tasks against **well-configured Git and Jujutsu**, in both ordinary single-directory and optional multi-workspace workflows, not only raw Git's worst commands. Preserve Git's deliberate selection, offline operation and reviewability; do not count fewer verbs as success.

Before enabling a workflow, exercise its normal path and the specific failure that makes it risky: ignored/new-file selection; deletion versus failed scan; edit after preview; stale selection; excluded-work retention expiry; divergent rewrite references; amend/export disclosure; partial recovery; coupled conflict confirmation; interrupted in-place writes; and unknown publication results. For switching, qualify dirty A → B → edits on B → return to A with exact observed bytes and original parent; ordinary capture expiry while A is parked; quota/coverage failure before writes; interrupted return followed by external edits; and workspace closure with parked work. Check that new/ignored files keep their inclusion dispositions, non-overwritten local files remain shared across serial contexts, and two independent directories may select the same revision. The [architecture](design.md#8-workspace-state-recovery-and-materialization) owns the pin/handoff and guarded-write contracts; these are required tests, not completed ones. Observe whether users can predict included bytes, unchanged files and the recovery action without learning Merkle objects.

Measure task completion, mistakes/omissions, recovery success, comprehension and responsiveness, including unnecessary conflict burden on real repeated-line/refactoring histories. Count environment setup, dependency/cache cost, directory navigation and cleanup, stale editor/process context and wrong-server test risks; source-command time alone is not the complete workflow. A conservative merge policy that repeatedly interrupts ordinary disjoint edits is a product failure to investigate before driver freeze, not evidence that users should learn more machinery. The [controlled text study](experiments/text_merge_study.py) exposed and reduced harmless blank-line conflicts; eight synthetic cases are not evidence that real merge/review burden is solved. Native correctness, privacy and durability gates remain mandatory even if a workflow tests well.
