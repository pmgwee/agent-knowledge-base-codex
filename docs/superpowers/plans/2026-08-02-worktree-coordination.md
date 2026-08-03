# Worktree Coordination Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Prevent silent concurrent-edit collisions by giving each task an explicit identity, worktree, writer lease, path claims, and non-mutating merge preflight visible to every agent.

**Architecture:** Coordination metadata lives beside each project ledger and is exposed through the service, CLI, MCP, and session-start context. Git worktrees provide file isolation; leases expose ownership; path claims warn early; merge preflight predicts conflicts without modifying user branches.

**Tech Stack:** Rust, SQLite transactions, Git CLI plumbing, Windows process/session metadata, existing named-pipe and MCP interfaces.

## Global Constraints

- Complete the temporal-memory plan first.
- One task maps to one worktree and at most one active writer lease.
- Default lease duration is 30 minutes; active sessions renew every 5 minutes.
- Expired leases are never silently stolen: the new owner records a takeover audit event.
- Path claims are advisory warnings; they do not claim that Git conflicts are impossible.
- Merge preflight must not check out, merge, reset, stage, or edit user files.
- Never create, remove, or prune a worktree without an exact resolved path and project match.

---

### Task 1: Add tasks and transactional writer leases

**Files:**
- Create: crates/brain-coordination/Cargo.toml
- Create: crates/brain-coordination/src/lib.rs
- Create: crates/brain-coordination/src/task.rs
- Create: crates/brain-coordination/src/lease.rs
- Create: crates/brain-coordination/tests/leases.rs
- Modify: Cargo.toml
- Modify: crates/brain-store/src/migrations.rs
- Create: docs/schemas/task-and-lease.md

**Interfaces:**
- Consumes: project/worktree/session identity and task title
- Produces: `TaskRecord`, `WriterLease`, acquire/renew/release/takeover operations

- [ ] **Step 1: Write failing exclusivity and expiry tests**

~~~rust
#[test]
fn only_one_session_can_hold_the_writer_lease_for_a_worktree() {
    let store = CoordinationFixture::new();
    store.acquire(worktree(), session_a()).unwrap();
    let second = store.acquire(worktree(), session_b());
    assert!(matches!(second, Err(LeaseError::AlreadyHeld { .. })));
}

#[test]
fn takeover_after_expiry_is_audited() {
    let clock = FakeClock::new();
    let store = CoordinationFixture::with_clock(clock.clone());
    store.acquire(worktree(), session_a()).unwrap();
    clock.advance(std::time::Duration::from_secs(31 * 60));
    store.acquire(worktree(), session_b()).unwrap();
    assert_eq!(store.takeover_events().len(), 1);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-coordination leases`

Expected: FAIL because the crate and tables are absent.

- [ ] **Step 3: Define task and lease records**

~~~rust
pub struct TaskRecord {
    pub id: uuid::Uuid,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub title: String,
    pub status: TaskStatus,
    pub created_at: time::OffsetDateTime,
}

pub struct WriterLease {
    pub task_id: uuid::Uuid,
    pub worktree_id: WorktreeId,
    pub owner: SessionIdentity,
    pub acquired_at: time::OffsetDateTime,
    pub renewed_at: time::OffsetDateTime,
    pub expires_at: time::OffsetDateTime,
    pub generation: u64,
}
~~~

- [ ] **Step 4: Implement compare-and-swap lease transactions**

Acquire, renew, release, and takeover use `BEGIN IMMEDIATE` and generation checks. Reject renewal by a different native session. Record every transition in append-only `coordination_events` linked to normalized evidence.

- [ ] **Step 5: Run tests**

Run: `cargo test -p brain-coordination leases`

Expected: exclusivity, idempotent renewal, wrong-owner release, expiry boundary, clock skew, and takeover audit tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add Cargo.toml Cargo.lock crates/brain-coordination crates/brain-store docs/schemas/task-and-lease.md
git commit -m "feat: add transactional task writer leases"
~~~

### Task 2: Register and validate Git worktrees per task

**Files:**
- Create: crates/brain-coordination/src/worktree.rs
- Create: crates/brain-coordination/tests/worktrees.rs
- Create: crates/brain-cli/src/task.rs
- Modify: crates/brain-cli/src/main.rs
- Create: crates/brain-cli/tests/task_commands.rs
- Create: docs/operations/task-worktrees.md

**Interfaces:**
- Consumes: registered project, task title, optional branch/base
- Produces: validated worktree record and `brain task create/list/close`

- [ ] **Step 1: Write failing worktree identity tests**

~~~rust
#[test]
fn task_create_reuses_project_identity_but_gets_unique_worktree_identity() {
    let repo = TestRepo::new();
    let task = create_task_worktree(&repo, "oauth callback").unwrap();
    assert_eq!(task.project_id, repo.project_id());
    assert_ne!(task.worktree_id, repo.main_worktree_id());
    assert!(task.path.starts_with(repo.approved_worktree_parent()));
}

#[test]
fn path_outside_registered_project_is_rejected() {
    assert!(matches!(register_foreign_worktree(), Err(WorktreeError::ProjectMismatch)));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-coordination worktrees; cargo test -p brain-cli task_commands`

Expected: FAIL because worktree management is absent.

- [ ] **Step 3: Implement read-only discovery and safe creation**

Parse `git worktree list --porcelain`, resolve `git common-dir`, HEAD, branch, lock/prunable state, and canonical path. `brain task create` chooses a collision-free slug below the configured worktree parent and invokes `git worktree add -b <branch> <exact-path> <base>` only after validation.

- [ ] **Step 4: Implement close without automatic deletion**

`brain task close` marks the task complete and releases its lease. It reports dirty/unpushed state and the exact optional cleanup command; it does not delete the worktree or branch.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-coordination worktrees
cargo test -p brain-cli task_commands
~~~

Expected: Git and non-Git errors, duplicate branches, spaces in paths, linked worktree identity, dirty close, and same-project validation tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-coordination crates/brain-cli docs/operations/task-worktrees.md
git commit -m "feat: map tasks to validated Git worktrees"
~~~

### Task 3: Add normalized path claims and overlap warnings

**Files:**
- Create: crates/brain-coordination/src/claims.rs
- Create: crates/brain-coordination/tests/claims.rs
- Modify: crates/brain-service/src/hook_handler.rs
- Create: crates/brain-service/tests/path_claim_context.rs
- Modify: crates/brain-mcp/src/tools.rs

**Interfaces:**
- Consumes: task/worktree plus file, directory, glob, or symbol claims
- Produces: normalized `PathClaim`, overlap severity, and context warnings

- [ ] **Step 1: Write failing overlap tests**

~~~rust
#[test]
fn directory_and_child_file_claims_overlap_case_insensitively_on_windows() {
    let a = claim(r"src\Auth", ClaimKind::Directory);
    let b = claim(r"SRC\auth\callback.rs", ClaimKind::File);
    assert_eq!(overlap(a, b), Overlap::Definite);
}

#[test]
fn generated_and_dependency_paths_are_ignored_by_default() {
    assert_eq!(overlap(claim("target", ClaimKind::Directory), source_claim()), Overlap::Ignored);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-coordination claims`

Expected: FAIL because claims do not exist.

- [ ] **Step 3: Implement Windows-safe normalization and overlap levels**

Resolve repository-relative paths; reject `..` escape and alternate data streams. Compare case-insensitively while preserving display spelling. Return `Definite`, `Probable`, `Possible`, `None`, or `Ignored` for file, directory, conservative glob, and symbol claims.

- [ ] **Step 4: Expose claim tools and context warnings**

Add MCP/CLI operations to claim, list, and release. Session-start context shows other active tasks, worktrees, owners, expiry, and overlapping claims before task memory.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-coordination claims
cargo test -p brain-service path_claim_context
~~~

Expected: case, separators, parent/child, glob, symbol, ignore rules, project isolation, and warning budget tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-coordination crates/brain-service crates/brain-mcp
git commit -m "feat: warn when active tasks claim overlapping paths"
~~~

### Task 4: Wire leases into agent lifecycle hooks

**Files:**
- Create: crates/brain-service/src/coordination_handler.rs
- Modify: crates/brain-service/src/hook_handler.rs
- Modify: crates/brain-service/src/main.rs
- Create: crates/brain-service/tests/lease_lifecycle.rs
- Modify: crates/brain-cli/src/install_hooks.rs
- Create: docs/operations/lease-lifecycle.md

**Interfaces:**
- Consumes: SessionStart, prompt/turn activity, stop/end events
- Produces: lease acquire/renew/release attempts and visible conflict responses

- [ ] **Step 1: Write failing lifecycle tests**

~~~rust
#[tokio::test]
async fn active_session_renews_every_five_minutes_without_hook_blocking() {
    let fixture = LeaseLifecycleFixture::new();
    fixture.session_start().await;
    fixture.advance_minutes(16).await;
    assert!(fixture.lease_renewal_count() >= 3);
    assert!(fixture.hook_p95() < std::time::Duration::from_millis(50));
}

#[tokio::test]
async fn second_writer_receives_owner_and_worktree_guidance() {
    let context = LeaseLifecycleFixture::with_existing_owner().second_start().await;
    assert!(context.contains("writer lease is held"));
    assert!(context.contains("create or switch to a separate worktree"));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-service lease_lifecycle`

Expected: FAIL because lifecycle events do not manage leases.

- [ ] **Step 3: Implement service-side lifecycle management**

The hook only enqueues activity. The persistent service acquires/renews/releases leases. If no task is selected, startup context asks the agent to select or create one but does not invent ownership. Unexpected session death lets the lease expire naturally.

- [ ] **Step 4: Add explicit handoff**

`brain task handoff <task> --to <harness/session>` writes a checkpoint, releases the current generation, and acquires the next generation in one transaction when possible. A failed acquire leaves the original lease intact.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-service lease_lifecycle
cargo test -p brain-hook --test latency -- --ignored --nocapture
~~~

Expected: acquisition, renewal, graceful end, crash expiry, conflict guidance, handoff rollback, and hook latency tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-service crates/brain-cli docs/operations/lease-lifecycle.md
git commit -m "feat: coordinate writer leases across agent sessions"
~~~

### Task 5: Implement non-mutating merge preflight

**Files:**
- Create: crates/brain-coordination/src/preflight.rs
- Create: crates/brain-coordination/tests/preflight.rs
- Create: crates/brain-cli/src/preflight.rs
- Modify: crates/brain-cli/src/main.rs
- Modify: crates/brain-mcp/src/tools.rs
- Create: docs/operations/merge-preflight.md

**Interfaces:**
- Consumes: source task/worktree and target ref
- Produces: merge base, changed paths, predicted conflicts, semantic warnings, and readiness result

- [ ] **Step 1: Write failing clean/conflict tests**

~~~rust
#[test]
fn preflight_finds_text_conflict_without_changing_index_or_worktree() {
    let repo = ConflictFixture::new();
    let before = repo.snapshot_state();
    let result = preflight(repo.source(), repo.target()).unwrap();
    assert!(result.conflicts.contains(&PathBuf::from("src/auth.rs")));
    assert_eq!(repo.snapshot_state(), before);
}

#[test]
fn dirty_worktree_is_reported_but_never_stashed() {
    let repo = DirtyFixture::new();
    let result = preflight(repo.source(), repo.target()).unwrap();
    assert!(result.blockers.contains(&Blocker::DirtyWorktree));
    assert!(repo.local_edit_exists());
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-coordination preflight`

Expected: FAIL because merge preflight is absent.

- [ ] **Step 3: Implement Git plumbing analysis**

Use `git merge-base`, `git diff --name-status`, `git diff --check`, and supported `git merge-tree` forms. Capture command version and exit status. Do not call `git merge`, `checkout`, `reset`, `stash`, `add`, or write an index.

- [ ] **Step 4: Add semantic warnings**

Combine path claims, delete/modify pairs, generated migration ordering, lockfile changes, and overlapping symbols from captured tool/file evidence. Label these warnings as heuristic, distinct from Git-proven conflicts.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-coordination preflight
cargo test -p brain-cli preflight
~~~

Expected: clean merge, text conflict, rename/delete, dirty tree, detached HEAD, missing base, spaces, non-mutation, and heuristic-label tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-coordination crates/brain-cli crates/brain-mcp docs/operations/merge-preflight.md
git commit -m "feat: add non-mutating merge conflict preflight"
~~~

### Task 6: Prove concurrent-session safety end to end

**Files:**
- Create: tests/e2e/concurrent_tasks.rs
- Create: tests/e2e/lease_expiry.rs
- Modify: tests/e2e/fixtures.rs
- Create: docs/operations/concurrent-agent-workflow.md

**Interfaces:**
- Consumes: simulated Claude/Codex/Hermes sessions editing related paths
- Produces: coordination release-gate evidence

- [ ] **Step 1: Write failing concurrent workflow test**

~~~rust
#[tokio::test]
async fn three_agents_get_isolated_worktrees_and_early_overlap_warning() {
    let e2e = CoordinationE2e::new();
    let claude = e2e.start_task(Harness::Claude, "oauth callback").await;
    let codex = e2e.start_task(Harness::Codex, "oauth tests").await;
    let hermes = e2e.start_task(Harness::Hermes, "docs").await;
    claude.claim("src/auth").await;
    let warning = codex.claim("src/auth/callback.rs").await;
    assert!(warning.is_definite_overlap());
    assert_ne!(claude.worktree(), codex.worktree());
    assert_ne!(codex.worktree(), hermes.worktree());
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test --test concurrent_tasks; cargo test --test lease_expiry`

Expected: FAIL until all coordination layers are connected.

- [ ] **Step 3: Run the coordination release gate**

Run:

~~~powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --test concurrent_tasks -- --nocapture
cargo test --test lease_expiry -- --nocapture
~~~

Expected: one-writer enforcement, separate worktrees, overlap warnings, takeover audit, handoff, and merge preflight pass without modifying fixture branches.

- [ ] **Step 4: Document the daily workflow**

Document task creation, launching each agent in its assigned worktree, path claims, handoff, status, preflight, merge by the user/agent, task close, and optional reviewed cleanup.

- [ ] **Step 5: Commit**

~~~powershell
git add tests/e2e docs/operations/concurrent-agent-workflow.md
git commit -m "feat: complete concurrent agent coordination gate"
~~~

## Coordination exit criteria

- A worktree has at most one non-expired writer lease.
- Separate tasks receive separate validated Git worktrees.
- All sessions see active owners, tasks, expiries, and path overlaps in bounded context.
- Session crashes recover through lease expiry with an auditable takeover.
- Merge preflight predicts Git conflicts without changing worktrees, branches, or indexes.
- Three-agent concurrency tests pass with early warnings and no silent same-worktree writes.
