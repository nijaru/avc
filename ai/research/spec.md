# Agent-Native Git-Compatible VCS: Research Handoff and V1 Target Spec

**Prepared:** 2026-06-17
**Audience:** implementation/research agent, product/architecture handoff
**Working name:** `AgentVCS` / `agentvcs`
**Primary constraint:** maximize adoption by remaining compatible with existing Git repositories, Git remotes, GitHub/GitLab/Bitbucket workflows, and ordinary Git tooling.

---

## 0. Executive Summary

Build a **Git-compatible agent-native change-management layer**, not a new storage engine in V1.

The product should let humans and AI coding agents edit freely while making every edit recoverable, attributable, splittable, validateable, reviewable, and exportable to ordinary Git branches and pull requests.

The V1 architecture should be:

```text
Git object database:
  source snapshots, branches, exported commits, hidden snapshot commits

Hidden Git refs:
  keep-alive refs for internal snapshots and change states

Local SQLite database:
  operation log, lanes, changes, stacks, agent runs, validation records, export mappings

Commit trailers:
  minimal durable public metadata: stable Change-Id, Stack-Id, Agent-Assisted, Validation

Git notes / custom refs:
  optional team-shared structured metadata; not required for baseline compatibility

Committed .agentvcs/*.yaml:
  team policy, validation config, redaction policy, agent rules

External provenance/artifact store, later:
  full logs, large traces, attestations, generated artifacts
```

The core design bet:

```text
Git compatibility wins adoption.
Agent-native metadata wins UX.
Native storage comes later only if Git becomes the bottleneck.
```

V1 should copy ideas from **Jujutsu/jj**, **GitButler**, **Gerrit**, **Graphite/GitHub stacked PRs**, **Sapling**, and syntax-aware diff/merge tools, but should not make compatibility with jj/GitButler/Sapling a product requirement.

---

## 1. Problem Statement

Git and GitHub were designed primarily for human-driven editing and review. AI coding agents change the pressure points:

1. Agents can create many edits quickly.
2. Multiple agents may operate concurrently.
3. Agents produce edits that need stronger attribution, validation, and review boundaries.
4. Users need easy recovery from failed or unwanted agent attempts.
5. Large agent-authored PRs are hard to review and less likely to be merged.
6. Current branch/PR workflows do not preserve stable semantic identity across amend/rebase/squash cycles.
7. Prompts, tool traces, test runs, and provenance are not modeled by Git or GitHub as first-class objects.

Empirical work on agent-authored PRs supports these concerns. The AIDev dataset reports **932,791 agent-authored PRs** across **116,211 repositories** and **72,189 developers**.[^aidev] A separate study of 33k agent-authored GitHub PRs found that not-merged PRs tend to have larger code changes, touch more files, fail CI/CD more often, and suffer from duplicate PRs, unwanted implementations, weak reviewer engagement, and agent misalignment.[^agent-fail]

Therefore, V1 should optimize for:

```text
- isolation of concurrent agent work
- automatic snapshots and undo
- small intent-based changes
- stacked review
- stable change identity
- per-change validation
- provenance summaries
- no lock-in from existing Git/GitHub workflows
```

---

## 2. Hard Product Constraints

### 2.1 Must remain Git-compatible

The system must initialize inside existing Git repositories and continue to allow normal Git operations:

```bash
git status
git log
git branch
git checkout
git push
gh pr create
```

Rationale:

- Git is the de facto source-code substrate and has a huge ecosystem of hosting services, GUIs, command-line tools, CI systems, branch-protection systems, release workflows, and audit tooling.[^git]
- GitHub Copilot cloud agent, Claude Code, GitButler, jj, Graphite, Sapling, and other relevant systems still interoperate with Git/GitHub rather than asking teams to abandon them.[^copilot-agent][^claude-worktrees][^gitbutler-main][^jj-git-compat][^sapling-meta]
- Adoption is likely dramatically higher if teams can try the product on existing repos without migration or lock-in.

### 2.2 Do not require jj compatibility

Use jj as **prior art**, not a compatibility target.

Do copy:

```text
- stable change IDs distinct from Git commit IDs
- operation log
- working-copy-as-change mental model
- safe undo
- Git backend coexistence ideas
```

Do not require:

```text
- jj CLI dependency
- jj repo format compatibility
- jj-specific sync semantics
- users to adopt jj
```

jj is valuable because it proves that a better VCS UX can be layered over Git. Its docs distinguish change IDs, which remain stable when a commit is rewritten, from commit IDs, which may change.[^jj-tutorial] jj also records repo-mutating operations in an operation log containing snapshots of repo views.[^jj-oplog] Its Git compatibility docs note that Git-representable commit data can be stored in Git while non-Git metadata such as change IDs can be stored separately.[^jj-git-compat]

### 2.3 Do not replace Git storage in V1

Git's storage is not optimal for every workload, but for source code it is good enough and adoption-critical.

Git is strong for:

```text
- source code and text files
- distributed snapshots
- content-addressed commits
- cheap branch/merge operations
- broad hosting/CI/review support
- release and audit workflows
```

Git is weak for:

```text
- huge binary assets
- datasets and model checkpoints
- server-enforced path-level ACLs
- file locking
- very large sparse workspaces
- rich agent provenance
- native stack/review metadata
```

V1 should handle source-code workflows on Git. Native/chunked storage can be a later track informed by Epic Lore, Git LFS, DVC, and Git-Theta.[^epic-lore][^git-lfs][^dvc][^git-theta]

---

## 3. Research Inputs and Design Implications

### 3.1 Jujutsu / jj

**What it is:** A VCS with a Git-compatible backend and radically improved local UX. Relevant concepts include a working-copy commit, operation log, stable change IDs, easy history rewriting, and safe undo.[^jj-tutorial][^jj-oplog][^jj-git-compat]

**Design implications:**

```text
Use stable internal Change IDs.
Store mutable workflow metadata outside normal Git commits.
Maintain an operation log for every repo mutation.
Protect internal snapshot commits from GC with hidden refs.
Expose undo/restore as first-class commands.
```

**Do not:** make jj compatibility a hard target unless later market evidence says jj has become a major platform boundary.

### 3.2 GitButler

**What it is:** A Git-backed change-management tool with parallel/virtual branches, branch lanes, stacked branches, undo, and AI coding workflow integrations.[^gitbutler-main]

GitButler's docs describe virtual branches as multiple branches applied to one working directory, each represented as a lane with its own staging area.[^gitbutler-virtual] Its Claude Code hooks are specifically designed to manage commits and branches for multiple simultaneous AI coding agent instances, isolating generated code into virtual or stacked branches automatically.[^gitbutler-hooks] Its blog explicitly frames this as "multiple Claude Code sessions without worktrees."[^gitbutler-parallel]

**Design implications:**

```text
Copy the lane concept.
Copy the visual/mental model of parallel streams of work.
Copy the agent hook integration pattern.
Copy stacked branch review UX.
```

**Caution:** same-worktree virtual branches require reliably tracking diff/hunk ownership. This is powerful but hard. V1 should choose correctness over elegance and use one Git worktree per lane.

### 3.3 Gerrit

**What it is:** A Git-based code review system centered on "changes" and patch sets, using stable `Change-Id` footers in commit messages to associate new revisions with the same review even across amends, rebases, and cherry-picks.[^gerrit-changeid][^gerrit-patchsets]

**Design implications:**

```text
Stable review/change identity must be separate from Git commit hash.
Use a Change-Id trailer in exported commits.
Treat amended commits as new versions of the same semantic change.
Preserve review identity across rewrite operations.
```

### 3.4 Stacked review: Graphite, GitHub stacked PRs, Sapling

Stacked review decomposes a large change into small dependent changes. Graphite describes the benefit as improving focus and speeding feedback by letting reviewers inspect smaller parts of the codebase.[^graphite-stacked] GitHub now documents native stacked PRs with the `gh stack` CLI, describing them as ordered chains of small, reviewable PRs that build on each other.[^github-gh-stack] Sapling emphasizes easier workflows around repository understanding, commit stacks, and recovery from mistakes.[^sapling-docs]

**Design implications:**

```text
Make stacks the default review/export format for agent output.
Warn or force split when diffs are too large.
Support export to normal PRs, stacked GitHub PRs, Graphite, or fallback branches.
Track stack relationships independently from Git branch names.
```

### 3.5 Claude Code and GitHub Copilot cloud agent

Claude Code recommends Git worktrees for parallel sessions so edits from different sessions do not collide.[^claude-worktrees] GitHub Copilot cloud agent can research a repo, plan, make changes on a branch, write commit messages, push, and create PRs.[^copilot-agent]

**Design implications:**

```text
V1 lane isolation should use Git worktrees.
Git branch/PR export remains the mainstream agent workflow boundary.
The product should operate as a local orchestrator/workbench, not as a replacement for GitHub initially.
```

### 3.6 Git metadata primitives

Git trailers provide structured key/value metadata at the end of commit messages.[^git-trailers] Git notes attach metadata to Git objects without changing their object IDs.[^git-notes]

**Design implications:**

```text
Use commit trailers for minimal, durable, portable metadata.
Use Git notes/custom refs only as optional sync channels because they are not always fetched/pushed by default.
Do not store raw prompts, tool logs, or secrets in ordinary commits.
```

A recent "Lore" paper, unrelated to Epic's Lore VCS, proposes repurposing Git commit messages and trailers as structured knowledge records for AI coding agents, carrying constraints, rejected alternatives, directives, and verification metadata.[^lore-paper]

### 3.7 Syntax-aware diff/merge

Difftastic compares files based on syntax rather than line-oriented diffs.[^difftastic] Mergiraf is a syntax-aware Git merge driver that can resolve conflicts using tree structure.[^mergiraf] Recent research on structured merge tools suggests generic structured merge can be practical and reduce certain classes of false positives/negatives compared with line-based or language-specific tools.[^lastmerge]

**Design implications:**

```text
V1 should shell out to existing syntax-aware diff/merge tools.
Do not write a custom AST diff/merge engine in V1.
Expose semantic diff/merge as optional review aids.
```

### 3.8 Epic Lore and large-asset VCS

Epic's Lore is a new VCS focused on large-scale, binary-heavy workflows. Its design docs describe a centralized content-addressed system with Merkle-tree repository states, immutable revision chains, chunked storage, sparse/on-demand hydration, local/remote mutable stores, compare-and-swap branch updates, and edge caching.[^epic-lore][^epic-lore-design]

**Design implications:**

```text
Do not copy Lore for V1 source-code workflows.
Study Lore for future native storage: chunking, sparse hydration, locks, permissions, edge cache.
If/when supporting assets/datasets/model checkpoints, Git object storage may be insufficient.
```

### 3.9 Data/model versioning: Git LFS, DVC, Git-Theta

Git LFS replaces large files in Git with text pointers while storing content on a remote server.[^git-lfs] DVC uses `.dvc` files so Git can version data alongside code without storing the data directly in Git.[^dvc] Git-Theta extends Git for fine-grained tracking of ML model parameters and model checkpoint changes.[^git-theta]

**Design implications:**

```text
Future native storage or integrations should target large binaries, datasets, generated assets, and model checkpoints.
Do not build this into V1 unless target users require it immediately.
```

### 3.10 Radicle and forge data as protocol

Radicle supplements Git with collaborative objects for issues, code reviews, and discussions.[^radicle] This is relevant if the product later needs a decentralized or Git-native alternative to GitHub/GitLab forge state.

**Design implication:**

```text
Not V1.
Study later if the product moves from local GitHub workbench to portable collaboration protocol.
```

### 3.11 Supply-chain provenance

GitHub Artifact Attestations create cryptographically signed claims about where and how build artifacts were produced.[^github-attestations] SLSA provenance formalizes build provenance as an attestation over a build platform, build definition, and produced artifacts.[^slsa]

**Design implication:**

```text
AgentVCS provenance should be digest-addressable and optionally attestable.
Do not rely only on commit signatures.
V1 should record validation output digests and attach build/test provenance links where available.
```

---

## 4. Product Thesis

AgentVCS is a Git-compatible local workbench for AI-assisted development.

It makes each human or agent edit:

```text
- captured
- undoable
- attributable
- grouped by intent
- splittable
- stackable
- validateable
- exportable to normal Git/GitHub workflows
```

Primary product statement:

```text
Run many agents against a GitHub repo without chaos.
Every edit is saved.
Every change is attributable.
Every stack is reviewable.
Every PR is normal GitHub.
```

---

## 5. V1 User Model

### 5.1 Workspace

A Git repo with AgentVCS initialized.

```text
repo/
  .git/
    agentvcs/
      state.sqlite
      blob-cache/
      oplog/
      runs/
  .agentvcs/
    policy.yaml
    agents.yaml
    validation.yaml
    redaction.yaml
    review.yaml
```

### 5.2 Lane

A lane is an isolated stream of work.

Examples:

```text
human/main
agent/login-flaky-test
agent/docs-refresh
agent/refactor-attempt-2
```

V1 implementation:

```text
one lane = one Git worktree + one Git branch + local DB metadata
```

Why:

- Correct isolation.
- Already recommended by Claude Code for parallel agent sessions.[^claude-worktrees]
- Avoids GitButler's harder same-worktree virtual-branch hunk ownership problem.

V2/V3 may implement same-worktree virtual lanes.

### 5.3 Change

A change is the stable semantic unit of work.

```yaml
change_id: avc_ch_01JZ8Y3M4Q9FZK2B7A6H
title: Fix login retry behavior
status: draft | ready | exported | merged | abandoned
lane_id: agent/login-flaky-test
stack_id: avc_st_01JZ8Y2VYRCG7W4DPX2M
current_git_commit: abc123...
previous_git_commits:
  - def456...
  - 789abc...
```

Critical rule:

```text
stable internal change_id != Git commit hash
```

### 5.4 Stack

A stack is an ordered set of dependent changes.

```text
stack: login-reliability
  1. Add auth test helper
  2. Fix retry state refresh
  3. Add regression test
  4. Update docs
```

### 5.5 Agent Run

An agent run is one session/execution/task from an AI coding tool.

```yaml
run_id: avc_run_01JZ8Y5R9E6M3
task_id: avc_task_01JZ8Y5...
tool: claude-code | codex | copilot | cursor | custom
model: optional
prompt_digest: sha256:...
redacted_prompt_summary: "Fix flaky login retry test."
files_read:
  - src/auth/session.ts
files_written:
  - src/auth/session.ts
  - tests/auth/test_login.ts
commands_run:
  - npm test -- tests/auth/test_login.ts
linked_changes:
  - avc_ch_01JZ8Y3M4Q9FZK2B7A6H
```

### 5.6 Operation

An operation is any repo mutation.

```yaml
op_id: avc_op_01JZ...
actor: human | agent | hook | cli
timestamp: 2026-06-17T12:00:00-07:00
command: agentvcs change amend avc_ch_...
before_ref: refs/agentvcs/snapshots/...
after_ref: refs/agentvcs/snapshots/...
undo_pointer: avc_op_...
```

---

## 6. V1 Storage Design

### 6.1 Local private DB

Path:

```text
.git/agentvcs/state.sqlite
```

Properties:

```text
- local by default
- not committed
- can contain sensitive/private data
- stores mutable workflow state
- can be partly rebuilt from Git refs/commits where possible
```

Stores:

```text
- operation log
- lane registry
- change graph
- stack graph
- agent run metadata
- validation history
- local snapshots
- prompt digests and redacted summaries
- tool-call summaries
- files read/written
- semantic diff cache
- review export mappings
- GitHub/GitLab PR mappings
- undo/redo state
```

### 6.2 Hidden Git refs

Use Git object storage for durable snapshots and keep-alive state.

Suggested refs:

```text
refs/agentvcs/snapshots/*
refs/agentvcs/changes/*
refs/agentvcs/keep/*
```

Rationale:

- Avoid building a content store in V1.
- Internal snapshot commits can be retained by refs and protected from GC.
- The pattern is similar in spirit to jj using `refs/jj/` for Git-backed commits and external storage for non-Git metadata.[^jj-git-compat]

### 6.3 Committed config

Path:

```text
.agentvcs/policy.yaml
.agentvcs/agents.yaml
.agentvcs/validation.yaml
.agentvcs/redaction.yaml
.agentvcs/review.yaml
```

Committed config should contain policy, not logs.

Example:

```yaml
version: 1
review:
  max_files_per_change_warn: 8
  max_lines_per_change_warn: 500
  require_human_review: true
validation:
  required_before_export:
    - name: unit-tests
      command: npm test
    - name: typecheck
      command: npm run typecheck
agents:
  allow_agent_assisted_commits: true
  require_agent_assisted_trailer: true
redaction:
  never_commit:
    - prompts.full_text
    - tool_outputs.raw
    - env
```

### 6.4 Commit trailers

Every exported commit should include minimal durable metadata.

Example:

```text
Fix login retry after expired session

Why:
The retry path failed to refresh session state before reattempting login.

Validation:
- npm test -- tests/auth/test_login.ts

Change-Id: avc_ch_01JZ8Y3M4Q9FZK2B7A6H
Stack-Id: avc_st_01JZ8Y2VYRCG7W4DPX2M
Agent-Assisted: true
Agent-Run: avc_run_01JZ8Y5R9E6M3
Validation: unit-tests:passed
```

Why trailers:

- Native Git-compatible structured metadata.[^git-trailers]
- Survives ordinary fetch/push.
- Parseable by Git and external tools.
- Similar to Gerrit Change-Id and the commit-trailer-based Lore proposal.[^gerrit-changeid][^lore-paper]

### 6.5 Git notes and custom refs

Optional, tool-aware sync layer:

```text
refs/notes/agentvcs
refs/agentvcs/runs/*
refs/agentvcs/stacks/*
refs/agentvcs/reviews/*
```

Use for:

```text
- structured JSON metadata
- full validation summaries
- shared review state
- provenance digests
- stack relationships
```

Caveat:

```text
Do not depend on Git notes/custom refs for baseline compatibility because ordinary Git users may not fetch or push them by default.
```

### 6.6 External provenance/artifact store

Not V1, but design metadata to support it later.

Store externally:

```text
- full logs
- raw tool traces
- large CI artifacts
- sandbox recordings
- build artifacts
- signed attestations
- model/tool transcripts
```

Commit/local DB should store:

```text
- digest
- content type
- URI
- access policy
- redaction status
```

---

## 7. V1 CLI Target

### 7.1 Setup

```bash
agentvcs init
agentvcs doctor
```

`agentvcs init`:

```text
- verifies Git repo
- creates .git/agentvcs/state.sqlite
- creates .agentvcs/*.yaml if missing
- installs optional hooks only with explicit consent
```

### 7.2 Lanes

```bash
agentvcs lane create agent/login-fix
agentvcs lane list
agentvcs lane switch agent/login-fix
agentvcs lane archive agent/login-fix
```

V1 behavior:

```text
- create Git worktree + branch
- record lane in SQLite
- associate future agent runs with lane
```

### 7.3 Snapshots and undo

```bash
agentvcs snap "before agent run"
agentvcs history
agentvcs undo
agentvcs restore avc_op_...
```

Implementation:

```text
- create hidden Git snapshot commit under refs/agentvcs/snapshots/*
- write operation-log entry
- expose clean UX, not raw internal commits
```

### 7.4 Changes

```bash
agentvcs change new "Fix login retry behavior"
agentvcs change list
agentvcs change show avc_ch_...
agentvcs change amend avc_ch_...
agentvcs change split
agentvcs change absorb
agentvcs change abandon avc_ch_...
```

`change split` should begin as semi-automatic:

```text
- propose split points by file/module/test/provenance
- user confirms
- later: automate with LLM + diff heuristics
```

### 7.5 Stacks

```bash
agentvcs stack new login-reliability
agentvcs stack list
agentvcs stack add avc_ch_...
agentvcs stack restack
agentvcs stack validate
agentvcs stack submit --github
```

Export strategies:

```text
- --branch-only
- --github-pr
- --github-stack
- --graphite
- --one-pr-with-stack-sections fallback
```

### 7.6 Validation

```bash
agentvcs validate
agentvcs validate --change avc_ch_...
agentvcs validate --stack avc_st_...
```

Record:

```text
- command
- environment digest
- exit status
- output digest
- timestamp
- linked change(s)
```

### 7.7 Agent integration

```bash
agentvcs agent start "fix flaky login test"
agentvcs agent attach --lane agent/login-fix
agentvcs agent runs
agentvcs agent summarize avc_run_...
```

Initial integrations:

```text
- generic command wrapper: agentvcs agent run -- <command>
- Claude Code hooks adapter
- Codex/Copilot adapters later, depending on available integration surfaces
```

### 7.8 Review and PRs

```bash
agentvcs review
agentvcs pr create
agentvcs pr update
agentvcs pr status
agentvcs pr open
```

`agentvcs review` produces:

```text
- summary
- files changed
- risk warnings
- tests run
- unresolved validation failures
- suggested split points
- PR body draft
```

---

## 8. V1 SQLite Schema Draft

```sql
create table changes (
  id text primary key,
  title text not null,
  description text,
  status text not null,
  lane_id text,
  stack_id text,
  current_commit text,
  created_at text not null,
  updated_at text not null
);

create table change_commits (
  change_id text not null,
  git_commit text not null,
  role text not null,
  created_at text not null,
  primary key (change_id, git_commit)
);

create table lanes (
  id text primary key,
  name text not null,
  type text not null,
  git_branch text,
  git_worktree_path text,
  base_ref text,
  status text not null,
  created_at text not null,
  updated_at text not null
);

create table stacks (
  id text primary key,
  name text not null,
  target_ref text not null,
  export_strategy text,
  status text not null,
  created_at text not null,
  updated_at text not null
);

create table stack_items (
  stack_id text not null,
  change_id text not null,
  position integer not null,
  primary key (stack_id, change_id)
);

create table operations (
  id text primary key,
  actor text not null,
  command text,
  before_ref text,
  after_ref text,
  created_at text not null
);

create table agent_runs (
  id text primary key,
  lane_id text,
  task text,
  tool text,
  model text,
  prompt_digest text,
  redacted_summary text,
  status text not null,
  created_at text not null,
  completed_at text
);

create table agent_run_files (
  run_id text not null,
  path text not null,
  access_type text not null, -- read | write | delete | rename
  primary key (run_id, path, access_type)
);

create table validations (
  id text primary key,
  change_id text,
  stack_id text,
  command text not null,
  status text not null,
  output_digest text,
  environment_digest text,
  created_at text not null
);

create table exports (
  id text primary key,
  change_id text,
  stack_id text,
  provider text not null, -- github | gitlab | graphite | branch-only
  pr_url text,
  branch text,
  last_exported_commit text,
  created_at text not null,
  updated_at text not null
);
```

---

## 9. V1 Non-Goals

Do not build in V1:

```text
- new object database
- new remote hosting service
- same-worktree virtual branches
- hunk ownership engine
- file locking
- chunked binary storage
- path-level server permissions
- decentralized forge
- custom review platform
- model checkpoint versioning
- full AST merge implementation
```

Reason: each is valid but distracts from the first adoption wedge: local Git-compatible agent workflow safety.

---

## 10. Architecture Decisions

### ADR-001: Use Git as source-code backing store

**Decision:** V1 stores source snapshots as normal Git objects and commits.

**Why:** compatibility, adoption, CI/review ecosystem, and ease of fallback.

**Tradeoff:** Git remains weak for large binary/data/model artifacts. Defer to future storage track.

### ADR-002: Use SQLite local metadata DB

**Decision:** store rich workflow state in `.git/agentvcs/state.sqlite`.

**Why:** local, fast, mutable, easy to query, not committed accidentally.

**Tradeoff:** not automatically shared; use trailers/notes/custom refs for portable subsets.

### ADR-003: Use stable Change IDs

**Decision:** every change has a stable `avc_ch_*` ID independent of Git commit hash.

**Why:** commits change after amend/rebase/squash. Review/task/provenance identity should not.

**Prior art:** jj change IDs and Gerrit Change-Id.[^jj-tutorial][^gerrit-changeid]

### ADR-004: Use worktree-backed lanes in V1

**Decision:** one lane maps to one Git worktree/branch.

**Why:** safer isolation for parallel agents. Aligns with Claude Code recommendation.[^claude-worktrees]

**Tradeoff:** more disk/env overhead than same-worktree virtual branches.

### ADR-005: Use commit trailers for portable metadata

**Decision:** exported commits include minimal trailers.

**Why:** trailers are Git-native, portable, parseable, and survive ordinary Git operations.[^git-trailers]

**Tradeoff:** commit messages can become noisy if overused. Keep small.

### ADR-006: Use notes/custom refs only as optional sync

**Decision:** Git notes/custom refs may carry structured metadata, but baseline product must work without them.

**Why:** notes do not alter object IDs, but ordinary workflows may not fetch/push them.[^git-notes]

### ADR-007: Stacked review by default

**Decision:** make stacks a first-class object and default export strategy for multi-change agent work.

**Why:** smaller PRs are easier to review, and agent failure research suggests larger/more complex PRs are higher risk.[^agent-fail][^graphite-stacked]

---

## 11. Build Plan

### Phase 0: CLI spike

Goal: prove Git-backed snapshots + SQLite metadata.

Build:

```text
- agentvcs init
- agentvcs snap
- agentvcs history
- agentvcs undo
- agentvcs change new/list/show
- commit trailer injection
- hidden refs for snapshots
```

Acceptance criteria:

```text
- existing Git repo remains normal
- snapshots are recoverable
- git log is not polluted by internal commits
- local DB maps changes to commits
```

### Phase 1: Worktree lanes and agent run records

Build:

```text
- agentvcs lane create/list/switch/archive
- generic agent command wrapper
- before/after snapshots around agent commands
- changed-file recording
- redacted run summary
```

Acceptance criteria:

```text
- two agents can edit in separate lanes without file collisions
- user can abandon one lane without affecting another
- operation log can restore before/after states
```

### Phase 2: Change and stack operations

Build:

```text
- change amend/split/absorb/abandon
- stack new/add/restack/validate
- suggested split warnings by size/files/modules
```

Acceptance criteria:

```text
- multi-file agent output can be converted into multiple changes
- each change has stable ID
- stack can be reordered/restacked
```

### Phase 3: GitHub export

Build:

```text
- branch export
- PR creation/update
- stack submit fallback
- GitHub stacked PR / gh-stack integration where available
- Graphite detection where available
```

Acceptance criteria:

```text
- exported branches and PRs are usable by ordinary teammates
- Change-Id trailers appear in commits
- PR URLs map back to local changes
```

### Phase 4: Review UX

Build:

```text
- agentvcs review
- PR body generator
- semantic diff integration via Difftastic
- merge driver recommendation via Mergiraf
- risk warnings
- validation dashboard
```

Acceptance criteria:

```text
- user sees what changed, why, risk level, tests run
- large changes get split suggestions
- semantic diff is available without replacing Git diff
```

### Phase 5: Optional shared metadata

Build:

```text
- git notes sync
- refs/agentvcs/* sync
- provenance digest export
- team-shared stack metadata
```

Acceptance criteria:

```text
- two AgentVCS users can share stack/change metadata beyond commit trailers
- ordinary Git users are unaffected
```

---

## 12. Research Backlog

Priority order:

1. **GitButler internals**: virtual branch hunk ownership, branch lanes, stacked branch export, undo, Claude hooks.
2. **jj internals**: operation log, Git backend, change ID storage, conflict state, GC protection.
3. **Gerrit Change-Id / patch sets**: stable review identity across amend/rebase/cherry-pick.
4. **Git metadata mechanisms**: trailers, notes, custom refs, fetch/push behavior.
5. **Stacked review systems**: Graphite, GitHub `gh stack`, Sapling stacks, Phabricator Differential.
6. **Agent PR empirical work**: failure modes, merge predictors, review burden, task categories.
7. **Semantic diff/merge**: Difftastic, Mergiraf, LastMerge, RefactoringMiner AST diff.
8. **Future storage**: Epic Lore, Git LFS, DVC, Git-Theta, Perforce, Unity Version Control.
9. **Security/provenance**: artifact attestations, SLSA, signed metadata, redaction, secret scanning.
10. **Agent configuration standards**: `AGENTS.md`, Claude hooks/skills/subagents, repo policy files.

---

## 13. Open Questions for Implementation Agent

### 13.1 Snapshot representation

Should `agentvcs snap` create:

```text
A. hidden Git commit under refs/agentvcs/snapshots/*
B. stash-like object
C. patch file in local DB/blob-cache
```

Recommendation: A for V1.

### 13.2 Prompt storage policy

Default should be privacy-preserving:

```text
- store prompt digest
- store redacted summary
- do not store full prompt unless user opts in
```

Need policy file support:

```yaml
provenance:
  store_full_prompts: false
  store_tool_outputs: false
  export_public_summaries: true
```

### 13.3 Virtual lanes

Should V2 implement GitButler-style same-worktree virtual branches?

Recommendation: only after V1 validates demand. Hunk ownership and conflict attribution are hard.

### 13.4 GitHub stacked PR status

GitHub stacked PR support is emerging; support should be capability-detected, not assumed.

Implementation:

```text
- check gh stack availability
- check repo/account support
- fallback to branch chain or one PR with stack sections
```

### 13.5 Shared metadata sync

Should teams use Git notes/custom refs, external service, or both?

Recommendation:

```text
V1: commit trailers only
V1.5: optional notes/custom refs
V2: external provenance service if enterprise use cases demand it
```

---

## 14. Reference PR Body Template

```markdown
## Summary

<One-paragraph summary.>

## Stack position

Change <N> of <M> in stack `<stack name>`.

Depends on:
- <previous PR / change>

## Why

<Intent and rationale.>

## What changed

- <bullet>
- <bullet>

## Validation

- [x] <command>: passed
- [ ] <command>: not run / failed

## Agent provenance

Agent-assisted: yes/no
Agent run: `<avc_run_...>`
Prompt: redacted summary only
Files read/written: see AgentVCS metadata

## Risk notes

<Generated risk notes: touched auth/payment/migration/etc.>

---

Change-Id: `<avc_ch_...>`
Stack-Id: `<avc_st_...>`
```

---

## 15. Reference Commit Message Template

```text
<imperative title>

Why:
<why this change exists>

What:
<short implementation summary>

Validation:
- <command>: <passed|failed|not-run>

Agent Notes:
- Agent-assisted: <true|false>
- Human reviewed before export: <true|false>

Change-Id: avc_ch_...
Stack-Id: avc_st_...
Agent-Assisted: true
Agent-Run: avc_run_...
Validation: unit-tests:passed
```

---

## 16. Risk Register

| Risk | Impact | Mitigation |
|---|---:|---|
| Git metadata pollution | Medium | Keep trailers minimal; full logs stay local/external. |
| Local DB corruption | High | Regular SQLite backups; rebuild from Git refs where possible. |
| Snapshot ref leaks to remote | Medium | Clear namespace; require explicit `agentvcs sync-metadata`. |
| User confusion with hidden commits | Medium | Never show internal commits in normal UX; docs explain hidden refs. |
| Worktree disk overhead | Medium | V1 accepts; V2 investigate virtual lanes. |
| Agent prompts contain secrets | High | Do not store full prompts by default; redaction policy. |
| Stacked PR support varies | Medium | Capability detection and fallbacks. |
| GitHub API/permissions complexity | Medium | Start branch-only; add PR integration second. |
| Semantic merge false confidence | Medium | Present semantic merge as assistive, not authoritative. |
| Native storage scope creep | High | Explicitly non-goal until V1 adoption validated. |

---

## 17. Initial Implementation Pseudocode

### 17.1 Snapshot

```python
def snap(message: str):
    before = read_current_head_and_index()
    tree = write_worktree_tree_to_git()
    commit = create_internal_commit(tree=tree, message=message)
    ref = f"refs/agentvcs/snapshots/{new_id()}"
    git_update_ref(ref, commit)
    db.insert_operation(
        id=new_id(),
        actor=current_actor(),
        command=f"snap {message}",
        before_ref=before,
        after_ref=ref,
    )
    return ref
```

### 17.2 Change creation

```python
def change_new(title: str):
    change_id = new_change_id()
    commit = create_or_amend_git_commit_with_trailer(
        title=title,
        trailers={"Change-Id": change_id},
    )
    db.insert_change(id=change_id, title=title, current_commit=commit)
    git_update_ref(f"refs/agentvcs/changes/{change_id}", commit)
    return change_id
```

### 17.3 Agent wrapper

```python
def agent_run(task: str, command: list[str], lane: str):
    run_id = new_run_id()
    before_ref = snap(f"before agent run {run_id}")
    db.insert_agent_run(run_id, task, command, lane, status="running")
    result = run_command(command, cwd=lane_worktree_path(lane))
    after_ref = snap(f"after agent run {run_id}")
    files = git_diff_name_status(before_ref, after_ref)
    db.update_agent_run(run_id, status=result.status, files=files)
    return run_id
```

---

## 18. Final Recommendation

Proceed with a V1 that is:

```text
- Git-backed
- worktree-lane based
- SQLite-metadata backed
- Change-Id/trailer compatible
- stack-first
- provenance-aware
- validation-first
- GitHub-export oriented
```

Do not attempt to build a new VCS object store, forge, or same-worktree virtual branch engine in V1. The first useful product is a local, Git-compatible safety and review layer for agent-generated work.

---

# References

[^git]: Git SCM, "Git." https://git-scm.com/

[^git-partial]: Git SCM, "Partial Clone." https://git-scm.com/docs/partial-clone

[^git-lfs]: Git Large File Storage. https://git-lfs.com/

[^dvc]: DVC Docs, "Get Started with DVC." https://doc.dvc.org/start

[^git-theta]: Nikhil Kandpal et al., "Git-Theta: A Git Extension for Collaborative Development of Machine Learning Models," arXiv:2306.04529. https://arxiv.org/abs/2306.04529

[^jj-tutorial]: Jujutsu Docs, "Tutorial and bird's eye view." https://docs.jj-vcs.dev/latest/tutorial/

[^jj-oplog]: Jujutsu Docs, "Operation log." https://docs.jj-vcs.dev/latest/operation-log/

[^jj-git-compat]: Jujutsu Docs, "Git compatibility." https://docs.jj-vcs.dev/latest/git-compatibility/

[^gitbutler-main]: GitButler. https://gitbutler.com/

[^gitbutler-virtual]: GitButler Docs, "Virtual Branches." https://docs.gitbutler.com/features/branch-management/virtual-branches

[^gitbutler-hooks]: GitButler Docs, "Claude Code Hooks." https://docs.gitbutler.com/features/ai-integration/claude-code-hooks

[^gitbutler-parallel]: GitButler Blog, "Managing Multiple Claude Code Sessions Without Worktrees." https://blog.gitbutler.com/parallel-claude-code

[^gerrit-changeid]: Gerrit Code Review Docs, "Change-Ids." https://gerrit-review.googlesource.com/Documentation/user-changeid.html

[^gerrit-patchsets]: Gerrit Code Review Docs, "Patch Sets." https://gerrit-review.googlesource.com/Documentation/concept-patch-sets.html

[^sapling-docs]: Sapling SCM Docs, "Introduction." https://sapling-scm.com/docs/introduction/

[^sapling-meta]: Meta Engineering, "Sapling: Source control that's user-friendly and scalable." https://engineering.fb.com/2022/11/15/open-source/sapling-source-control-scalable/

[^graphite-stacked]: Graphite, "Stacked diffs." https://graphite.com/guides/stacked-diffs

[^github-gh-stack]: GitHub, "GitHub Stacked PRs." https://github.github.com/gh-stack/

[^copilot-agent]: GitHub Docs, "About Copilot cloud agent." https://docs.github.com/en/enterprise-cloud@latest/copilot/concepts/agents/cloud-agent/about-cloud-agent

[^claude-worktrees]: Claude Code Docs, "Run parallel sessions with worktrees." https://code.claude.com/docs/en/worktrees

[^git-trailers]: Git SCM, "git-interpret-trailers." https://git-scm.com/docs/git-interpret-trailers

[^git-notes]: Git SCM, "git-notes." https://git-scm.com/docs/git-notes

[^lore-paper]: Ivan Stetsenko, "Lore: Repurposing Git Commit Messages as a Structured Knowledge Protocol for AI Coding Agents," arXiv:2603.15566. https://arxiv.org/abs/2603.15566

[^epic-lore]: Epic Games, "Lore." https://lore.org/

[^epic-lore-design]: Epic Games Lore Docs, "System design." https://epicgames.github.io/lore/explanation/system-design/

[^difftastic]: Wilfred Hughes, "Difftastic: a structural diff that understands syntax." https://github.com/Wilfred/difftastic

[^mergiraf]: Mergiraf, "Mergiraf: syntax-aware git merge driver." https://mergiraf.org/

[^lastmerge]: João Pedro Duarte, Paulo Borba, Guilherme Cavalcanti, "LastMerge: A language-agnostic structured tool for code integration," arXiv:2507.19687. https://arxiv.org/abs/2507.19687

[^aidev]: Hao Li, Haoxiang Zhang, Ahmed E. Hassan, "AIDev: Studying AI Coding Agents on GitHub," arXiv:2602.09185. https://arxiv.org/abs/2602.09185

[^agent-fail]: Ramtin Ehsani et al., "Where Do AI Coding Agents Fail? An Empirical Study of Failed Agentic Pull Requests in GitHub," arXiv:2601.15195. https://arxiv.org/abs/2601.15195

[^agent-security]: Mohammed Latif Siddiq et al., "Security in the Age of AI Teammates: An Empirical Study of Agentic Pull Requests on GitHub," arXiv:2601.00477. https://arxiv.org/abs/2601.00477

[^git-of-thoughts]: Pavan C. Shekar, Abhishek H. S., Aswanth Krishnan, "GitOfThoughts: Version-Controlled Reasoning and Agent Memory You Can Replay, Diff, and Merge," arXiv:2606.14470. https://arxiv.org/abs/2606.14470

[^radicle]: Radicle Docs, "Radicle Protocol Guide." https://radicle.dev/guides/protocol

[^github-attestations]: GitHub Docs, "Artifact attestations." https://docs.github.com/en/actions/concepts/security/artifact-attestations

[^slsa]: SLSA, "Build: Provenance." https://slsa.dev/spec/draft/build-provenance

---

## 19. Review Notes (Added 2026-06-17)

### 19.1 gitoxide worktree limitation

gitoxide cannot create linked worktrees (`git worktree add` equivalent) natively as of v0.53. The maintainer confirmed this in 2023:

> "The Worktree doesn't have any capability to mutate itself yet, nor is there a way to create an official worktree like `git worktree add` does." — Byron, [gitoxide#1002](https://github.com/GitoxideLabs/gitoxide/discussions/1002)

**Implication for V1:** Lane creation must shell out to `git worktree add`. This is acceptable — the spec's worktree-per-lane approach remains correct. gitoxide handles everything else (reading commits, writing objects, managing refs, diffing). The shell-out is a one-time setup cost per lane, not a hot path.

**Future:** gitoxide may add native worktree creation. Track [gitoxide#301](https://github.com/GitoxideLabs/gitoxide/issues/301) and related issues.

### 19.2 SQLite library choice

For a synchronous CLI tool, `rusqlite` is the better choice over `sqlx`:

| Consideration | rusqlite | sqlx |
|---|---|---|
| Async runtime required | No | Yes (tokio/async-std) |
| Compile-time query checking | No | Yes (via `sqlx::query!` macro) |
| Complexity for CLI | Low | Higher |
| SQLite version bundled | 3.51.3 (via `bundled` feature) | Same (via `bundled` feature) |
| Mixing with sqlx in same project | Problematic (libsqlite3-sys conflicts) | N/A |

**Recommendation:** `rusqlite` with `features = ["bundled"]`. Simpler, no async overhead, sufficient for local CLI operations.

### 19.3 Hidden ref push protection

The spec defines `refs/agentvcs/*` as local-only but doesn't specify how to prevent accidental push. Options:

1. **Push hook:** Install a `pre-push` hook that blocks `refs/agentvcs/*` unless `agentvcs sync-metadata` is used.
2. **Remote refspec config:** Set `remote.origin.push` to exclude `refs/agentvcs/*` explicitly.
3. **Push default:** Use `push.default = current` or `push.default = simple` to limit what gets pushed.

**Recommendation:** Option 1 (hook) for V1. Install during `agentvcs init`, warn during `agentvcs doctor` if missing.

### 19.4 Agent hook interface (needs design)

The spec references Claude Code hooks and GitButler's integration but doesn't define the hook interface. For Phase 1, need to specify:

- What events fire (pre-agent-run, post-agent-run, pre-commit, post-commit)
- What data is passed to hooks (run_id, lane_id, changed files, etc.)
- How `agentvcs agent attach --lane` observes agent actions in real-time
- Hook installation mechanism (git hooks wrapper? agent config? environment variable?)

### 19.5 Market context (2026-06-17)

Recent developments relevant to positioning:

- **Cursor Origin** (announced 2026-06-17): agent-native git hosting by Graphite team. Waitlist, launching fall 2026. MCP-extensible. Direct competitor on the hosting side, but doesn't address the local CLI workbench problem.
- **Zed DeltaDB**: CRDT-based version control tracking every edit as a stream. Complementary to git, not a replacement. Beta late June 2026.
- **Weave**: entity-level semantic merge driver using tree-sitter. CRDT coordination layer for agent conflict prevention. Directly applicable to multi-agent lane conflicts.
- **Gitdot**: Rust-based, anti-AI GitHub alternative. Early stage. Proves Rust works for this domain.
- **GitHub reliability**: 10 incidents in April, 9 in May 2026. AI agent traffic overwhelming infrastructure. Real opening for alternatives.

**Positioning:** AgentVCS is the local workbench layer. It works with any hosting platform (GitHub, GitLab, Forgejo, Cursor Origin). The spec's "no lock-in" constraint is a strength.

---

## 20. Implementation Notes (Added 2026-06-17)

### 20.1 Suggested Rust dependencies

```toml
[dependencies]
# CLI
clap = { version = "4", features = ["derive"] }

# Git operations
gix = "0.72"  # or latest stable

# SQLite
rusqlite = { version = "0.39", features = ["bundled"] }

# Error handling
anyhow = "1"
thiserror = "2"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"  # for .agentvcs/*.yaml config files
serde_json = "1"

# IDs
uuid = { version = "1", features = ["v4"] }

# Hashing (for prompt digests, content digests)
sha2 = "0.10"

# Time
chrono = { version = "0.4", features = ["serde"] }
```

### 20.2 Phase 0 implementation order

1. `agentvcs init` — create `.git/agentvcs/` dir, `.git/agentvcs/state.sqlite` with schema, `.agentvcs/*.yaml` defaults
2. `agentvcs snap` — create hidden commit under `refs/agentvcs/snapshots/`, write operation log entry
3. `agentvcs history` — read operation log from SQLite
4. `agentvcs undo` — restore to previous snapshot ref
5. `agentvcs change new` — create change record, inject Change-Id trailer into current commit
6. `agentvcs change list/show` — query SQLite
7. `agentvcs doctor` — verify git hooks installed, refs namespace clean, DB schema current

### 20.3 ID format

The spec uses `avc_ch_01JZ8Y3M4Q9FZK2B7A6H` format. This appears to be ULID-inspired. Options:

- **ULID:** sortable by time, 26 chars, Crockford base32
- **UUID v7:** time-ordered, 36 chars with hyphens, standard
- **Custom prefix + UUID v7:** `avc_ch_` + UUID v7 = readable + sortable

**Recommendation:** Custom prefix + UUID v7 for changes (`avc_ch_`), stacks (`avc_st_`), runs (`avc_run_`), operations (`avc_op_`). Sortable, standard, debuggable.

### 20.4 Repo name suggestion

`agentvcs` or `avc` for the CLI binary. Short, typeable, no conflicts with existing tools (checked: no `avc` in Homebrew, no `agentvcs` in common package managers).

---

## 21. Open Design Questions (Added 2026-06-17)

### 21.1 Multi-repo support

Should AgentVCS work across multiple repos simultaneously? Agents may span repos (monorepo services, shared libraries). V1 probably single-repo, but worth noting.

### 21.2 Concurrent agent conflict resolution

With worktree-backed lanes, agents can't collide on files. But what happens when two lanes modify the same function differently? The spec's `change split` and stack restack handle this at export time, but real-time conflict detection would be useful.

**Weave** (tree-sitter-based entity-level merge) could be integrated as a merge driver recommendation in Phase 4.

### 21.3 Operation log size

The operation log will grow unbounded. Need:

- Compaction strategy (keep last N operations, or operations within time window)
- Snapshot pruning (keep snapshots referenced by active changes/stacks, prune orphaned ones)
- SQLite WAL mode for concurrent reads during long operations

### 21.4 Git LFS interaction

If the repo uses Git LFS, AgentVCS snapshots should not duplicate LFS objects. The hidden snapshot commits should reference the same LFS pointers as the working tree. This needs testing in Phase 0.

### 21.5 Config file versioning

The `.agentvcs/*.yaml` files are committed. What happens when the schema evolves? Need:

- `version` field in each config file
- Migration logic in `agentvcs init` and `agentvcs doctor`
- Backwards-compatible defaults when fields are missing
