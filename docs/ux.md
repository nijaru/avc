# AVC user workflows and CLI contract

**Required effects and proposed interactions; not implemented or usability-validated.** This is the user-facing companion to [the architecture](design.md). It owns interaction defaults and workflow acceptance cases, not another object/storage specification. Examples describe the native design, **not** the existing Git-wrapper CLI. The source architecture is selected; the everyday navigation, command spelling and presentation still need task-based qualification. A safe mechanism alone does not establish a good user model.

## 1. The everyday model

Users edit **files**, deliberately organize **commits**, and separately choose what to **share**. A private **checkpoint** protects observed work without making it a commit or publishing it. Recovery scope and commit selection are different decisions. Do not carry forward the prototype's automatic authored-commit/squash model or require users to understand object storage.

Unfinished work must be addressable **before its first commit**. Two efforts starting at the same commit need distinct local references; a base commit or directory name cannot identify both. The proposed interaction gives each a stable local reference, always shown as a selectable short handle, and an optional descriptive name. No naming ceremony is required. Names are labels, not identity; renaming does not reattribute captures, and closed handles are not reused. That reference identifies current work across captures and first authoring; an individual checkpoint identifies one immutable observation. Repository-level navigation lists current and preserved unfinished work, including work from closed workspace registrations, not every recovery snapshot. Show its base, current/saved location, protection and unavailable data. Clean work can remain named for continued editing; completing a commit does not silently retire its reference or turn it into a rewrite-following change alias. Closing a local reference is explicit. An active reference must first be left through navigation or workspace closure; those effects may be combined in one confirmed action. Closing inactive unfinished work refuses unless the user separately chooses discard. Neither closure nor discard cancels other acknowledged checkpoint deadlines. These are local navigation records, not canonical Task objects or another history graph.

Serial work can stay in one project directory; extra writable directories are an option for concurrency, not a prerequisite for changing work. Ordinary editors/directories and offline operation are the default; no daemon, mount, IDE extension or server is required. Use familiar verbs where their meaning survives: `status`, `diff`, `commit`, `amend`, `merge`, `log`.

Default status answers what work is selected, what differs, what is recoverable and what is shared, without displaying internal object kinds or generations:

```text
Work: parser-cleanup (work:7b4c)
Base: Handle tokens (change:c82d, version 2)
Files: 3 modified, 1 new file not selected for a commit
Recovery: none; background capture off
History: 2 local commits; remote sharing not checked
```

This example has no checkpoint. After an explicit capture or an enabled recovery policy succeeds, report the actual captured scope/base, age and remaining retention instead; do not imply that recovery is enabled by default. Those lines describe evidence, not a permanently clean filesystem. If current files differ from the last checkpoint, show that difference; do not call them saved merely because a checkpoint exists. Identify checkpoints at an earlier history base separately, including after a selective commit. The same work reference can have historical protection without current bytes being captured; showing that relation does not retarget the old checkpoint. Report uncaptured paths, exclusions and failed coverage separately. Unsaved editor buffers are outside filesystem recovery. Unknown sharing or recovery status is **unknown**, not an optimistic success.

`status`, `diff`, `show`, `log` and listing checkpoints are read-only repository operations: no implicit capture, authoring, history rewrite, fetch or hook execution. Background recovery is an explicit workspace policy; it must report failures/gaps and never manufacture authored commits. Quote untrusted paths/descriptions safely, preserve non-UTF8 names losslessly, and bound default output.

### Proposed overview and navigation

Use the current work as the entry point: its history base (or “no commit yet”), current file differences, recovery coverage and known sharing status. Work is the effort to resume; a commit is authored history; a checkpoint is an earlier observation to recover. Exact object IDs, receipts and generations belong in details or automation, not a prerequisite tutorial. This is a presentation candidate for task trials, not a selected CLI layout.

The interactive switch picker distinguishes **resume work** from **visit a version**. Visiting a version moves the current clean work to that base; it does not create another effort or resume saved bytes that happen to share the base. Starting independent work is a separate explicit action. For example:

```text
Resume work
  parser-cleanup  work:7b4c  here; 3 modified, 1 new
  hotfix          work:9a12  preserved; base: Handle tokens; return protected
  docs            work:4e61  active in ../parser-docs; cannot resume here

Visit a version — requires clean work
  Handle tokens   change:c82d  version 2
  Add lexer       change:a63f  version 1

Start independent work…  choose a version or use the current base
```

The example's edits block a clean visit. Selecting a version must offer explicit preserve-and-start-independent-work or refuse, not silently move `parser-cleanup` with its edits.

Before the first commit, show the base as **no commit yet** and allow independent work at the initial base. Existing efforts still appear under resume, with their distinct handles and saved bytes. Neither navigation nor private preservation requires a synthetic first commit.

Rows use kind-distinct displayed references (`work:…`, `change:…`, `revision:…` or `checkpoint:…`) rather than indistinguishable bare hex, equal bases or labels. Prefixes are UI reference syntax, not new wire IDs; abbreviations still require collision checking and change references still resolve to one exact version before mutation. A preserved row exposes the exact saved scope, exclusions and protection in its preview; “return protected” is not a promise about every historical checkpoint. Work detached from a closed workspace remains discoverable. Another workspace's active/reserved work is identified, not silently stolen. Large lists need bounded output, search and an explicit way to reveal omitted rows; historical checkpoints stay under their work's recovery view rather than competing with work destinations.

The switch preview brings departure and destination into one decision: what is preserved, which source/base is restored, what local ignored/excluded files remain, and any obstacle that blocks writing. Do not add a separate acknowledgement for every internal capture/root transition. Unknown coverage and overwrite decisions still require their own resolution or refusal; a concise flow cannot imply consent to them. Noninteractive operation keeps explicit policy and machine-readable outcomes.

Clean visits do not accumulate work references. Their preview says **“Move work parser-cleanup to this version”**: subsequent edits, checkpoints and commits belong to that same work at its new base. Its former tip is not a second current head. For read-only inspection, use `show`, `diff` or `log` instead. Explicitly created efforts can still accumulate; offer explicit closure of clean departing work in the navigation flow without requiring it or cancelling historical recovery promises. Cleanliness alone does not mean an effort is abandoned. Visiting older history can also leave a newer authored tip outside selected ancestry once the back slot changes. Show whether other roots retain that tip and its applicable retention; offer explicit retention without creating another work head automatically. A repository-wide retained-history view must make such authored versions discoverable, with anchors/availability and an explicit keep action using existing root pins. Default selected-ancestry history and alias resolution remain scoped; broad discovery must not silently choose among divergent versions or include private recovery inventories in export. These are [research-driven defaults](research.md#29-clean-navigation-without-synthetic-efforts--october-30-2026), not usability findings.

## 2. Selection and preview

- `commit` with no path selection includes modifications and proven deletions of already tracked files, **except deletion of a tracked directory subtree**, which requires explicit scope/interactive selection. Name those directory removals separately rather than burying them in a long file list. New files require explicit path scope or interactive selection; a directory path includes its eligible new files. No-op ordinary commits report “nothing selected,” not an empty authored revision. Explicit history/metadata operations may legitimately keep the same source State.
- Explicit directory paths include eligible tracked changes/proven deletions and non-ignored new files under that scope, including deliberately selected subtree removals and the parent directories needed to represent selected entries, not unrelated siblings. First-run help must say this plainly. Ignore rules are selection aids, not a secret detector or erasure mechanism. New ignored paths need a separate explicit inclusion choice; ignore patterns do not untrack already tracked files.
- A failed/incomplete directory read is never a deletion. Show deletions separately in the preview; deleting an entire tracked directory subtree requires explicit scope/interactive selection. No arbitrary file-count threshold or generic `--force` bypass. If a bare commit authors other modifications while deferring a subtree removal, report the omitted removal and its explicit selection path in the result; successful authoring is not a claim that every observed edit was included.
- Interactive file/hunk selection is supported without a mandatory mutable staging index. Selecting a new-file hunk explicitly selects that file's creation. Binary/type/mode changes use whole-entry choices where hunk selection has no defined meaning.

**Private recovery includes tracked edits and non-ignored new files by default**, within the requested workspace/path scope. This applies to an explicit checkpoint or preserve-and-switch, not to read-only inspection or an unrequested background service. It does not track, commit or share new files. AVC's live administration/store/journal files are never source inputs; explicit scope cannot override that boundary. New ignored paths need separate consent; already tracked files remain tracked even if an ignore pattern matches. Explicit exclusions, unsupported entries, size/resource limits and scan failures must be visible. Do not silently skip an oversized file and then report all work protected. Offer deliberate scope/limit changes or refuse; partial capture needs an explicit partial-scope decision. Background capture is separately enabled policy with the same honest coverage obligations. This is a reasoned default to qualify, not an empirically proven preference.

Authoring from a checkpoint still applies deliberate commit selection to its **pinned bytes and recorded base**. A broad recovery checkpoint is not blanket consent to commit every captured new file or deletion. Captures record observed tracked/new origin, not reusable authoring consent; a cancelled preview cannot make a later bare commit include a new file, and an old exclusion cannot override a later explicit path selection. Construct and preview the selected candidate before promotion; do not filter an already confirmed candidate afterward. With no eligible changes, explain that no new checkpoint/commit was created.

A filesystem authoring command uses the capture/admission path, not a weaker second scanner. Pin the observed bytes, build an **immutable selected candidate**, and show its actual diff against its exact base. Hunk choices refer to those pinned versions, never offsets into a subsequently reread live file. Keep required Observation provenance; a constructed partial-file candidate must not be described as a byte-for-byte filesystem snapshot.

Confirmation authors exactly the previewed candidate. Later editor changes remain uncommitted; do not rescan them into the commit or roll them back. Selection/base changes by another operation invalidate the plan and require re-review. The local commit and requested selection advance are one generation-checked transaction; on failure the prepared checkpoint remains recoverable, with its actual deadline. Read-only status never creates that checkpoint.

Cancellation creates no commit. Report any recovery checkpoint already retained. Unselected observed edits may be kept in a separate private recovery checkpoint; show its scope/deadline. They are **not** committed or retained forever just because another portion was committed. Do not pretend to infer which later live edits came from excluded hunks. The saved checkpoint remains the exact comparison point.

## 3. Concrete workflows

Syntax below is illustrative. `<work>`, `<change>` and `<checkpoint>` mean references shown by AVC, not additional objects users must construct. Work references resume an effort; change references select authored history. They are not interchangeable even at equal source or base.

### Start and make the first commit

```sh
avc init
avc status
avc commit README.md src/ -m "Start parser"
```

`init` creates a native repository and empty unauthored workspace base, **not a synthetic authored commit**. Existing files remain unchanged and initially untracked. A bare first `commit` reports that nothing was selected, the observed number of eligible new files (or incomplete coverage), and an explicit include-scope example; it must not appear to succeed while recording nothing. The first real commit has no parent. Included new files then become tracked in that selected history; omitted new files remain untracked, with only their acknowledged private recovery protection. Earlier checkpoints still represent their original observations. For example: privately checkpoint `README.md`, `src/` and new `notes.txt`; author only the first two; then edit `notes.txt` again. The same work remains selected, `notes.txt` remains new, and its older bytes are found under that work's checkpoints at the initial base. The later edit is not protected until another capture. Recovering the older checkpoint restores its original base/origins, not today's tracking status. Do not silently wrap/import a Git or prototype repository; refuse before changes and explain that conversion is a separate, deferred adapter.

### Make a selective commit and keep working

```sh
avc diff
avc commit --interactive -m "Handle escaped input"
avc status
```

The interactive flow selects files/hunks, shows the exact resulting source and omitted changes, then confirms once. It leaves working files untouched and reports remaining differences against the new commit. No separate stage/add command is required.

For review or tests before authoring, keep that same immutable preview open and offer materialization into a separate review directory. Select once, inspect/test the selected source, then confirm the same candidate; do not require a recovery-checkpoint ritual or select the hunks again. The exact CLI interaction remains to qualify. A separately chosen checkpoint can also supply older captured bytes, but is not itself the commit selection.

The review directory starts from the candidate, not later edits in the original directory. It is still an ordinary writable directory, not a test-isolation boundary. AVC does not become a build runner; evidence can bind that candidate only when execution actually consumes it under its declared input/writer boundary. Before/after hashes alone cannot rule out transient writes or a stale server. Promotion uses the recorded history base, not a search for an equal State. Default promotion must match the current workspace base before advancing it; otherwise offer a workspace at the recorded base, not a silent reparent. State-only recovery with no recorded revision requires an explicit parent/root-authoring choice, except the initial empty repository's first commit.

### Interrupt a task and return in the same directory

```sh
avc switch --new --preserve-current
# Start another effort at the selected base, without the departing edits.
# Edit and optionally commit using ordinary tools.
avc switch --back --preserve-current
```

`switch --new` explicitly allocates another local reference at the selected history base (including the initial empty base), or an explicitly chosen authored version, even if the base is unchanged. Do not copy the departing unfinished bytes or create an authored WIP commit. To resume existing work, select its work handle instead; an equal base never identifies which effort to resume.

By contrast, `switch <change>` visits the exact selected authored version within the current work when observed source matches its base, coverage is complete and no pending resolution or incomplete materialization blocks departure. It moves that work's base and files without allocating a reference. Selecting the already selected exact version does nothing and leaves useful back navigation unchanged; different versions with identical files remain different destinations. Dirty, pending or unknown work cannot take this clean-visit path. Offer explicit preserve-and-start-independent-work, or refuse; never carry edits along implicitly.

This is one preserve-and-switch flow, not a checkpoint/workspace/`cd` ritual. Its preview identifies whether the destination is saved unfinished work or authored source, the exact base and which captured bytes will be restored. It captures the departing observed tracked work **and eligible new files**, with the exact history base, before materializing the destination without those edits. Return preserves the second effort's unfinished work and restores the original base and captured files. A restored new file stays untracked until deliberately authored. No WIP commit is manufactured; later authoring uses the original base even if it was amended elsewhere.

`--back` returns to the destination left by this workspace's previous **completed switch**, not the latest commit, timestamp or history undo. After a work switch it resumes that work; after a clean visit it returns to the previous exact version within the same work. Show that distinction before confirmation. It is a one-slot toggle, not a stack: A → B → A leaves B as previous. A previous-version target protects that exact base until another completed switch replaces the slot or the workspace is unregistered; it does not pin every visited version or extend historical checkpoint deadlines. Older unfinished work remains reachable through its handle.

If A clean-visits C2 → C1 and is then edited, bare `--back` cannot move A to C2. The explicit preserve-and-start choice keeps A and its edits at C1, starts independent B at C2 without those edits, and makes A the previous work. Preservation alone must not silently authorize creating B or reusing A in two places. Returning to a version never substitutes saved work with an equal base.

Resolve and pin the exact destination before confirmation. One work reference has at most one active workspace; a pending or interrupted resume reserves it too. A second resume refuses and identifies that location/operation. Closed or unavailable work is reported, not replaced by a recent checkpoint or equal-base effort. For concurrent editing, create independent work or recover a pinned checkpoint into a new work reference; do not steal a binding or share a mutable work identity. Different work references can still select the same immutable revision. Do not silently follow rewrite successors. The spelling and presentation of work references remain candidates.

Explicitly preserved unfinished work stays protected **until successful resume or explicit discard**, independently of the ordinary seven-day checkpoint deadline. Status distinguishes that protection from expiring historical recovery. It covers the acknowledged scope, not every past checkpoint or excluded path. After verified resume, disclose the handoff to ordinary recovery retention; failed/interrupted resume keeps both protections. Quota failure refuses before filesystem writes. Discarding a pin does not erase other captures, authored history or backups.

Bare interactive `switch` offers preserve-and-resume for a work destination when it finds dirt; an authored-version destination instead requires the explicit preserve-and-start-independent-work choice. Noninteractive use requires an explicit policy. A remembered policy may be explicitly configured per workspace, never inferred from one override. Show which tracked/new files will be preserved and removed from the destination and which local files will remain. Refuse unknown coverage, unsafe names and unpreserved obstacles. Ignored/excluded paths remain directory-local unless explicitly included; an obstacle requiring overwrite needs separate consent or refusal.

Source switching is **not environment switching**. Non-preserved local `.env`, dependencies, caches and databases are not restored to earlier values. Unsaved editor buffers, open descriptors and running processes are not checkpointed or restarted. Lockfile changes may require project setup. AVC neither runs setup hooks nor copies ignored secrets, installs dependencies or shares mutable dependency directories implicitly. Ignore rules are not a secret detector: even private recovery can contain sensitive content and has a disclosed retention scope. Interrupted writes use guarded recovery, not blind rollback over later editor changes.

### Move edits started in the wrong place

Leaving work behind and taking it to another base are different intents. Offer an explicit checked transfer of selected files/hunks, including new files, without requiring a temporary authored commit. Pin the original base, source observations, destination base/current work, selected edits and exact leftovers before preview. Show what will remain at the origin and what will appear at the destination; moving is not merely changing a checkpoint's recorded parent.

Combine the selected edits with the destination under the normal checked source/merge rules. Preserve unrelated work at both ends. Conflict, cancellation or stale references must keep both inputs recoverable and must not claim a completed move. Later editor writes are not silently treated as the previously selected hunks. Directory updates still obey guarded materialization; no implicit writes to another active workspace. The exact transfer interaction is unselected and must pass this workflow before it ships.

### Use another directory when work is actually concurrent

```sh
avc workspace new hotfix --from <change> --path ../parser-hotfix
```

Choose this when another writer, long-running test or watcher needs the original source directory to stay unchanged. The new workspace starts from the named immutable change, not the original directory's unfinished bytes. `workspace new` neither saves nor scans an unrelated writer's directory; checkpoint there separately if needed. Independent workspaces may start at the same immutable revision without competing for an exclusive branch checkout.

A separate directory isolates source-file writes, **not** ports, services, databases, secrets or remote APIs. Dependencies and local configuration need deliberate setup; tests must identify the server/source they actually exercise. Do not silently reuse a server just because its URL answers. This is not a promise of a sandbox or automatic environment provisioning.

Closing workspace registration leaves its directory intact. Preview the active work and coverage: dirty work needs explicit preserve-and-close, with the same capture defaults and refusal on unknown coverage as switching. A clean reference retains its exact base. Detach the active binding only after establishing repository-owned return protection; do not make directory-local edits disappear from navigation merely by removing registration. An interrupted materialization must be deliberately recovered before normal closure. Directory deletion is a separate explicit action with tracked, new and ignored-file dispositions shown; no generic force fallback. Preserved work remains discoverable after closure and recoverable into a fresh directory. Extra workspaces must earn their setup, navigation, disk and cleanup costs rather than becoming a directory per small task.

### Amend a local commit

```sh
avc amend --interactive
```

The default target is the workspace's selected commit, not a commit chosen by timestamp; file/hunk selection follows the same deliberate rules and exact candidate preview. A StateOnly selection has nothing to amend: explain how to author a first commit instead. Keep its ChangeId and parents; create a replacement Revision rather than a second unrelated change. Do not rewrite descendants/other workspaces automatically. For an older change or stack rewrite, preview the explicitly chosen affected stack first; unsupported merge-commit rebases fail before changing history.

For a lower-stack fix, identify affected descendants and offer an explicitly scoped rewrite, leaving unrelated tip edits and other workspaces alone. Preview the entire chosen scope and its conflicts; a failed preparation cannot be described as a successfully restacked history. Split/fold must show explicit identity mappings rather than choose one universal successor from many-to-many lineage. Detailed split/fold and edit-assignment interactions remain unselected, not implied by the existence of evolution records.

Show known sharing status before rewriting. Private replacement does not change a remote target or another workspace's selection; do not claim “unpublished” from an unavailable/stale remote view. Prefer a follow-up commit/revert for shared fixes. No automatic force publication.

Review views identify the exact version **and comparison base/scope**: this change, a candidate amendment, a whole stack, or changes since a previous review. A stable change reference is not the reviewed version. Another amendment or equal source with different parents must not silently inherit approval. A future integration may reuse approval only under an explicit policy, with its original scope visible; this does not require a native forge or review object.

### Undo history versus recover files

```sh
avc history undo
avc recover <checkpoint> --to ../parser-recovered
```

There is **no bare magic `undo`**. `history undo` previews the latest undoable history edit for the current local work reference in this workspace, identifies the effect in ordinary words and confirms before restoring its private selection. Evolution records remain append-only; undo does not erase lineage. Users need not type operation IDs; exact IDs remain available to automation/diagnostics. Switching does not authorize crossing contexts: if the recorded work identity or resulting selection does not match the current work, refuse rather than jump to another effort's history. Equal selected revisions in two efforts do not make their undo scopes interchangeable. Files stay unchanged, prominently stated both before and after. The post-action status shows differences against the restored selection; do not imply that running tests in the unchanged directory tests the restored commit.

| Action | History undo / recovery behavior |
| --- | --- |
| Commit, amend, explicitly scoped rebase | Restore prior private history selection conditionally; keep current files and retained replacement revisions |
| Checkpoint creation | Not an undoable history edit; do not erase an acknowledged recovery receipt or jump past it without explaining the selected history action |
| Completed switch | Return with `switch --back`; history undo does not restore files |
| Interrupted file materialization | Keep preserved work and before-image protection; recover into a fresh directory or deliberately finish guarded recovery before switching again |
| Shared acceptance, another workspace's operation, arbitrary editor write | Never undone by a local history command; shared source fixes use an explicit new commit/revert |

`recover` defaults to a newly allocated directory/workspace and never overwrites the original. Recovery from a checkpoint retains its recorded base and observed tracked/new origins, not a new base that accidentally treats every privately captured file as tracked. Say which paths come from the checkpoint and which are inherited base values; a partial checkpoint is not a whole-directory time machine. Known failed/skipped coverage requires an explicit partial-recovery decision. An in-place recovery is a separate guarded action with preservation policy, not the normal recovery path. Expired/unavailable data is reported honestly; an audit ID is not a promise that bytes remain.

### Merge and resolve without losing the task

```sh
avc merge <change>
avc resolve
avc amend
```

An explicit merge can create/select a **local authored merge containing typed conflicts**; it does not publish it. There is no hidden index-based “merge in progress.” Show the new change, exact conflict units and recovery point. Outside a conflicted subtree, source edits can continue; agreed descendants inside it remain readable but cannot be edited through that conflict under the current conditional-transform contract. A coupled conflict at the repository root therefore blocks ordinary source edits until its whole unit is resolved. The merge preview must disclose that limitation and the scratch/resolution path rather than imply unrestricted continued editing. Resolution may amend that merge while preserving its parents, or be recorded in a deliberate follow-up commit. Clean source is not proof that tests pass.

For text: edit the generated presentation, review the proposed source replacement, and confirm it. `commit --interactive` may combine that review/confirmation with authoring; a separate resolve ritual is not mandatory. Confirmation binds exact replacement bytes. If those bytes change before authoring, re-review them rather than silently refreshing the accepted resolution. Untouched bound markers remain conflicts; ordinary marker-like file contents are not parsed as conflict identity.

Required conflict alternatives and bound presentation/pending data stay protected while the active workspace needs them, independently of an old checkpoint's seven-day deadline. Status distinguishes that protection from historical byte recovery; it does not mean every live edit is captured. Switching or closing registration transfers the needed interpretation data with the preserved work.

For binary/type/directory conflicts: show typed alternatives and materialize a user-chosen alternative in a scratch workspace for inspection/construction. External tools require explicit authorization. Confirm the resulting typed value, not an inferred filename/content match. Use input descriptions/identities, not context-dependent “ours/theirs” labels alone; unavailable origins are labeled as such, not invented.

A coupled rename/directory region is **one resolution unit**, even if its canonical location is the root. Show every affected binding together and explain the decision, e.g. “choose a destination for src/parser; keep lib/parser or pkg/parser.” Path rows are previews, not permission to resolve one coupled binding independently. Validate the whole replacement unit and preserve agreed siblings. Never show merely “entire repository conflicted.”

If a resource limit prevents a representable result, show “could not safely combine; inputs retained until …,” with the pending receipt and next choices. Do not manufacture a conflict State. If materialization fails, separately report the saved source, unchanged/changed paths, incomplete workspace and the fresh-directory recovery command; history undo is not crash repair.

### Share deliberately (later protocol stage)

Local commit/checkpoint commands never publish, run CI or execute repository hooks. A future publish preview distinguishes intended source/history, public provenance metadata, explicitly supplied audit evidence, and remote target/validation status. Acceptance, validation and deployment are different outcomes.

Newly authored public Observation metadata describes the selected candidate's scope, not the broader private capture's inventory. Keep unrelated captured paths, excluded/failed path lists and local work identifiers private by default; narrow public coverage claims honestly rather than copy them wholesale. Any extra audit disclosure is explicit at authoring, not a secret metadata rewrite at publication.

Source export follows **owning history/source references**, not non-owning audit commitments. Amending away a non-parent private predecessor must not transfer that predecessor's bytes merely to retain an explanation. Real parent history is still included. Public provenance can reveal paths/fingerprints and is not metadata anonymity; explicit audit-evidence export can include discarded versions/secrets and needs a separate disclosure choice. Default authoring keeps private capture receipt IDs in local audit metadata, not in public provenance. Do not strip metadata or change already-created RevisionIds secretly during publication.

A full physical repository backup can contain private recovery, discarded versions and evidence even when a selected-source export omits them. Preview that broader disclosure; removing a secret from the current commit does not erase it from backups.

On an unknown remote result, query the same durable receipt. Never tell an agent to delete its workspace until the receiving authority confirms complete pinned source and the recorded base/pending data needed for the promised recovery, with a retention/durability domain. Private handoff must not require manufacturing an authored commit.

Offline readiness is relative to named retained roots and requested operations. A sparse directory may have full local custody; a populated directory may lack historical objects. Show missing/promised dependencies before claiming readiness, separately from build dependencies or external services. A successful sparse fetch is not a promise that merge, history and independent export all work disconnected.

Git hosting/IDE compatibility is not provided by native source objects alone. Qualify a complete contribution/review/CI journey, including Git-dependent version/build tools, before selecting an interoperability interface. Conversion, sustained mixed-tool participation and protocol adaptation remain alternatives, not promised features. Each must declare representable subsets, native/Git ID mappings and losses, separately from full-fidelity native exchange. Do not silently install extensions, hooks or substitute Git commands.

## 4. References, errors and automation

Default history displays stable short change references and a version distinction when needed, not State/Revision/generation machinery. Resolve a change to its unique visible rewrite successor in the requested workspace/root scope (by default its selected authored ancestry, not every retained recovery/orphan root); retained superseded versions do not make every amend ambiguous. Divergent live successors and short-prefix collisions require disambiguation. Never choose by timestamp. An explicit immutable revision reference addresses an older exact version. A prepared mutation pins its resolved references before confirmation, so later alias movement cannot retarget it. Local unfinished-work references and authored-change references have different effects; render their kinds explicitly and disambiguate name collisions rather than silently choosing resume versus history navigation. Exact immutable source/history IDs and local work/checkpoint handles are distinct API types, not one guess-at-everything reference string. The [local API contract](design.md#local-api-names-and-effects) separates source computation from workspace changes and storage internals.

Errors state **what happened, what was saved, what did not change, and one concrete next action**. Do not bury successful source custody inside “command failed,” or describe an incomplete checkout as success. Separate canonical source availability, selection update, materialization and remote acceptance in machine output. JSON/versioned outcomes, meaningful nonzero statuses for blocked/incomplete/unknown operations, exact receipt lookup and caller-scoped retry keys are qualification requirements. Noninteractive commands never guess at interactive choices, silently broaden scope or retry a changed request under the same key.

## 5. Qualification, not UX claims

These workflows are paper contracts, not usability-test results or a running CLI. Compare representative tasks against **well-configured Git and Jujutsu**, in both ordinary single-directory and optional multi-workspace workflows, not only raw Git's worst commands. Preserve Git's deliberate selection, offline operation and reviewability; do not count fewer verbs as success. For agent-thread tasks, [Delta's documented workflows](research.md#deltadb-causal-collaboration-versus-source-custody) add a relevant comparison: find uncommitted work independently of its branch, fork without changing the parent, and bring back selected file changes without copying the conversation. These are documented behaviors, not measured advantages or a requirement for AVC to own an agent harness.

Before enabling each workflow, exercise its complete normal path and the failure that makes it risky:

| Workflow | What must be demonstrated |
| --- | --- |
| Start before any commit | New files are privately recoverable; author only an explicitly selected subset; find omitted work after further edits |
| Interrupt and return | Distinguish two unfinished efforts at the same base, even before either has a commit; restore exact source and original parent without checkpoint archaeology |
| Visit history | Visit several versions without new work handles; predict which base later edits belong to, exact-version back versus work resume, and dirty-back refusal or explicit creation of another effort |
| Preserve work over time | Ordinary capture expiry, quota/coverage failure, interrupted return plus external edits, and workspace closure do not violate the unresumed-work obligation |
| Repair wrong-place edits | Transfer selected hunks/new files, keep unrelated work at both ends, and explain conflict/cancellation outcomes |
| Revise a stack | Fix a lower change, show descendants and identity mappings, retain tip edits, and leave another workspace's selection alone |
| Review while work changes | Identify pinned base/version/scope, distinguish later amendments and divergent successors, and do not approve unseen versions by alias |
| Undo and recover | Find the intended prior work; refuse another effort's history even at an equal selected revision; predict restored history, unchanged files and the bytes tests will actually read |
| Collaborate or hand off | Explain source/export/private-backup scope, stale requests and unknown receipts; reconstruct a private worker result without forced authoring |
| Work disconnected or through Git tooling | Identify complete versus promised roots; qualify the chosen mixed-tool journey without pretending conversion preserves CI/review identity |

The [selected-authoring reference checks](research.md#17-selective-authoring-and-private-recovery--october-29-2026) exercise part of the first-commit/recovery/privacy case with canonical objects, not an implemented interaction. These cases supplement deletion versus failed scan, edit-after-preview/stale selection, excluded-work expiry, coupled conflict confirmation, partial recovery and interrupted writes. The [architecture](design.md#8-workspace-state-recovery-and-materialization) owns custody and guarded writes; none of these workflows is implemented or proven by the generic root/SQLite tests. The [bounded work-lifecycle model](experiments/work_lifecycle_model.py) checks part of the authority/pin contract, not complete entry points or filesystem safety. Use paper task trials to compare mental models before fixing navigation, then real entry-point trials before enabling features. Observe whether users predict included bytes, unchanged files and recovery without learning Merkle objects. Do not delay initial local codec qualification on designing a forge or every deferred integration.

### First interaction trial: predict the outcome

Use the [proposed overview and navigation](#proposed-overview-and-navigation) with these concrete cases before adding more command vocabulary. These are paper trial scenarios and expected effects derived from the contract, not recorded user results or executable mock-output tests.

| Scenario | What the person must be able to predict | Comparison and unresolved cost |
| --- | --- | --- |
| A and B explicitly start at commit C. A has a modified tracked file and new `scratch.txt`; preserve A, start B without those edits, then return to A. Repeat before the first commit, with only new files at the initial base. | Resume A restores its exact recorded base and acknowledged files; `scratch.txt` is still new. `--new C` starts another effort even at the same base; selecting C alone never infers resume A. At the initial base neither effort requires an authored commit; included files become tracked only through deliberate first authoring. Ignored `.env` remains directory-local. | Compare explicit new-file-inclusive Git stashes/branches and Jujutsu's separate working-copy changes. The picker exposes the intents on paper; understanding is not established. Explicitly retained efforts still have discovery/closure costs. |
| Clean A at C2 visits C1 and then C0, including versions with identical files. Return once, then edit and try to return again. | The visits keep A and allocate no other work. Back first returns to exact C1, not saved work with an equal base; later edits belong to A at C1. Dirty back refuses or explicitly preserves A and starts B at C0. Re-selecting the exact current version does not erase the back slot. | Compare clean historical checkout separately from new branch/change creation in Git/Jujutsu. Test whether “move this work” is understood, not mistaken for read-only inspection or a hidden original tip. |
| File `parser.py` contains independent edits X and Y. Preview a commit containing X only, then the editor adds Z before confirmation. | Confirmation commits pinned X only; live files keep X, Y and Z. Status reports the remaining difference against the new commit. The preview's checkpoint protects its observed bytes, not later Z; a separate capture would be needed to protect Z. | Compare Git's partial staging/index and Jujutsu's selective commit flow. AVC must earn an advantage through one understandable selection/preview, not by claiming immutable selection is novel. Selecting/testing the candidate still differs from testing live files. |
| A checkpoint protects `notes.txt` at bytes N1. The editor changes it to N2; recover the older checkpoint. | Default recovery creates another directory containing N1 with the recorded base/origins; the original N2 is not overwritten. History undo instead changes selected history and leaves files alone. | Compare explicit restore-to-another-directory workflows and Jujutsu recovery facilities. Predictable scope matters more than one magic undo command. Earlier-base labels and partial coverage must not look like complete current protection. |

A walkthrough fails if the participant must learn StateId/RevisionId, search raw checkpoint timestamps to find an effort, or infer whether a row means resume work, move current work or start independent work. It also fails if “saved” causes them to believe later bytes are protected, or if they expect history undo to change test inputs. Record predictions and mistakes before explaining the model. Git/Jujutsu baselines need equivalent source, recovery scope and task goals; count setup and cleanup rather than assuming the proposed AVC transcript is fewer steps.

Measure task completion, mistakes/omissions, recovery success, comprehension and responsiveness, including unnecessary conflict burden on real repeated-line/refactoring histories. Count environment setup, dependency/cache cost, directory navigation and cleanup, stale editor/process context and wrong-server test risks; source-command time alone is not the complete workflow. A conservative merge policy that repeatedly interrupts ordinary disjoint edits is a product failure to investigate before driver freeze, not evidence that users should learn more machinery. The [controlled text study](experiments/text_merge_study.py) exposed and reduced harmless blank-line conflicts; eight synthetic cases are not evidence that real merge/review burden is solved. Native correctness, privacy and durability gates remain mandatory even if a workflow tests well.
