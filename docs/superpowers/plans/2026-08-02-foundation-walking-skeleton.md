# Foundation and Claude Walking Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Build the smallest end-to-end system that captures one Claude Code project, stores normalized evidence, and injects a bounded continuation into a fresh session.

**Architecture:** A Rust service tails Claude JSONL using committed cursors and writes a per-project SQLite ledger. A compiled hook shim communicates through a Windows named pipe and emits deterministic SessionStart context. This phase contains no LLM, Basic Memory, CodeGraph, LLM Wiki, or embeddings.

**Tech Stack:** Rust 1.88.0 MSVC, Tokio, Serde, rusqlite bundled SQLite, Notify, Clap, tracing, UUIDv7, SHA-256, Windows named pipes.

## Global Constraints

- Follow all constraints in the master plan.
- Walking-skeleton scope is exactly one registered Claude Code project.
- The service must recover from restart and idempotently replay an earlier cursor.
- No transcript content may be injected without project_id scope and evidence IDs.
- No hidden full-history scan is allowed during SessionStart.

---

### Task 1: Initialize the repository and Rust workspace

**Files:**
- Create: .gitignore
- Create: rust-toolchain.toml
- Create: Cargo.toml
- Create: crates/brain-domain/Cargo.toml
- Create: crates/brain-domain/src/lib.rs
- Create: crates/brain-store/Cargo.toml
- Create: crates/brain-store/src/lib.rs
- Create: crates/brain-adapters/Cargo.toml
- Create: crates/brain-adapters/src/lib.rs
- Create: crates/brain-context/Cargo.toml
- Create: crates/brain-context/src/lib.rs
- Create: crates/brain-service/Cargo.toml
- Create: crates/brain-service/src/main.rs
- Create: crates/brain-hook/Cargo.toml
- Create: crates/brain-hook/src/main.rs
- Create: crates/brain-cli/Cargo.toml
- Create: crates/brain-cli/src/main.rs

**Interfaces:**
- Consumes: none
- Produces: a compiling Rust workspace with all later crate names fixed

- [ ] **Step 1: Initialize Git and install the pinned toolchain after approval**

Run:

~~~powershell
git init
winget install --id Rustlang.Rustup -e --source winget
rustup toolchain install 1.88.0-x86_64-pc-windows-msvc
rustup default 1.88.0-x86_64-pc-windows-msvc
~~~

Expected: git status succeeds and rustc --version reports 1.88.0. Request approval before winget because this changes machine-level tooling.

- [ ] **Step 2: Write the workspace manifest and toolchain pin**

~~~toml
# rust-toolchain.toml
[toolchain]
channel = "1.88.0"
profile = "minimal"
components = ["rustfmt", "clippy"]
~~~

~~~toml
# Cargo.toml
[workspace]
resolver = "2"
members = [
  "crates/brain-domain",
  "crates/brain-store",
  "crates/brain-adapters",
  "crates/brain-context",
  "crates/brain-service",
  "crates/brain-hook",
  "crates/brain-cli",
]

[workspace.package]
edition = "2024"
license = "MIT"
rust-version = "1.88"

[workspace.dependencies]
anyhow = "1"
async-trait = "0.1"
clap = { version = "4.5", features = ["derive"] }
hex = "0.4"
notify = "7"
regex = "1"
rusqlite = { version = "0.32", features = ["bundled", "uuid"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
tempfile = "3"
thiserror = "2"
time = { version = "0.3", features = ["serde", "formatting"] }
tokio = { version = "1", features = ["macros", "net", "rt-multi-thread", "signal", "sync", "time"] }
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
uuid = { version = "1", features = ["serde", "v7"] }
~~~

- [ ] **Step 3: Add minimal crate entry points**

Each library src/lib.rs contains:

~~~rust
#![forbid(unsafe_code)]
~~~

Each binary main contains:

~~~rust
fn main() -> anyhow::Result<()> {
    Ok(())
}
~~~

- [ ] **Step 4: Verify formatting, linting, and tests**

Run:

~~~powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
~~~

Expected: all commands pass with zero warnings.

- [ ] **Step 5: Commit**

~~~powershell
git add .gitignore rust-toolchain.toml Cargo.toml Cargo.lock crates
git commit -m "chore: initialize secondary brain workspace"
~~~

### Task 2: Define configuration and stable project identity

**Files:**
- Create: crates/brain-domain/src/config.rs
- Create: crates/brain-domain/src/ids.rs
- Create: crates/brain-domain/src/project.rs
- Modify: crates/brain-domain/src/lib.rs
- Create: crates/brain-domain/tests/project_identity.rs
- Create: docs/schemas/project-identity.md

**Interfaces:**
- Consumes: BRAIN_HOME and a filesystem/Git checkout path
- Produces: BrainConfig, ProjectId, WorktreeId, ProjectIdentity, ProjectRegistry

- [ ] **Step 1: Write failing identity tests**

~~~rust
#[test]
fn same_git_common_dir_maps_worktrees_to_one_project() {
    let fixture = TestRepo::with_two_worktrees();
    let a = ProjectIdentity::inspect(fixture.main()).unwrap();
    let b = ProjectIdentity::inspect(fixture.worktree()).unwrap();
    assert_eq!(a.project_key, b.project_key);
    assert_ne!(a.worktree_key, b.worktree_key);
}

#[test]
fn equal_basenames_do_not_collide() {
    let a = ProjectIdentity::for_non_git(r"C:\one\api").unwrap();
    let b = ProjectIdentity::for_non_git(r"C:\two\api").unwrap();
    assert_ne!(a.project_key, b.project_key);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: cargo test -p brain-domain --test project_identity

Expected: FAIL because ProjectIdentity is undefined.

- [ ] **Step 3: Implement IDs, configuration, and identity**

~~~rust
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProjectId(pub uuid::Uuid);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorktreeId(pub uuid::Uuid);

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ProjectIdentity {
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub project_key: String,
    pub worktree_key: String,
    pub root: std::path::PathBuf,
    pub git_common_dir: Option<std::path::PathBuf>,
    pub branch: Option<String>,
    pub head: Option<String>,
}

impl BrainConfig {
    pub fn load() -> anyhow::Result<Self>;
    pub fn brain_home() -> anyhow::Result<std::path::PathBuf>;
}
~~~

Resolve BRAIN_HOME first, otherwise %USERPROFILE%\AgentBrain. Normalize case and separators for lookup, but preserve the display path. Git common-directory identity uses git rev-parse --git-common-dir; non-Git roots require explicit registration and receive a UUID.

- [ ] **Step 4: Implement an atomic projects.json registry**

Write to projects.json.tmp, flush, and rename. Store project UUID, aliases, common Git directory, remote, and worktrees. Reject ambiguous path matches instead of selecting one.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-domain
cargo clippy -p brain-domain --all-targets -- -D warnings
~~~

Expected: identity, alias, moved-path, and collision tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-domain docs/schemas/project-identity.md
git commit -m "feat: add stable project and worktree identity"
~~~

### Task 3: Create the append-only SQLite evidence ledger

**Files:**
- Create: crates/brain-domain/src/event.rs
- Modify: crates/brain-domain/src/lib.rs
- Create: crates/brain-store/src/migrations.rs
- Create: crates/brain-store/src/ledger.rs
- Create: crates/brain-store/src/cursor.rs
- Modify: crates/brain-store/src/lib.rs
- Create: crates/brain-store/tests/ledger_idempotency.rs
- Create: crates/brain-store/tests/cursor_atomicity.rs
- Create: docs/schemas/normalized-event.md

**Interfaces:**
- Consumes: NormalizedEvent, RawRecord, SourceCursor
- Produces: EventLedger::append_batch and committed cursor transactions

- [ ] **Step 1: Write failing idempotency and cursor tests**

~~~rust
#[test]
fn replaying_one_batch_creates_no_duplicate_events() {
    let store = TestStore::new();
    let batch = fixture_batch();
    assert_eq!(store.append_batch(&batch).unwrap().inserted, 3);
    assert_eq!(store.append_batch(&batch).unwrap().inserted, 0);
    assert_eq!(store.event_count().unwrap(), 3);
}

#[test]
fn cursor_does_not_advance_when_event_insert_fails() {
    let store = TestStore::new();
    let result = store.append_batch(&invalid_batch());
    assert!(result.is_err());
    assert_eq!(store.cursor("claude:test").unwrap(), SourceCursor::start());
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: cargo test -p brain-store

Expected: FAIL because the ledger and migrations are absent.

- [ ] **Step 3: Define the normalized event**

~~~rust
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct NormalizedEvent {
    pub event_id: uuid::Uuid,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub task_id: Option<uuid::Uuid>,
    pub harness: Harness,
    pub native_session_id: String,
    pub native_turn_id: Option<String>,
    pub event_type: EventType,
    pub occurred_at: time::OffsetDateTime,
    pub observed_at: time::OffsetDateTime,
    pub source_locator: String,
    pub source_offset: i64,
    pub source_schema: String,
    pub raw_hash: [u8; 32],
    pub idempotency_key: [u8; 32],
    pub git_head: Option<String>,
    pub git_branch: Option<String>,
    pub payload: serde_json::Value,
    pub raw: serde_json::Value,
}
~~~

- [ ] **Step 4: Implement migrations and transactions**

Create tables schema_migrations, events, source_cursors, quarantine, and capture_gaps. Enable WAL, foreign_keys, busy_timeout=1000, and synchronous=NORMAL. Create UNIQUE(idempotency_key), project/time, session/sequence, and event_type/time indexes.

Event insert and cursor update occur in one IMMEDIATE transaction:

~~~rust
pub fn append_batch(&mut self, batch: &EventBatch) -> anyhow::Result<AppendResult> {
    let tx = self.connection.transaction_with_behavior(
        rusqlite::TransactionBehavior::Immediate,
    )?;
    let inserted = insert_events(&tx, &batch.events)?;
    save_cursor(&tx, &batch.source_id, &batch.next_cursor)?;
    tx.commit()?;
    Ok(AppendResult { inserted })
}
~~~

- [ ] **Step 5: Run tests and inspect the query plan**

Run:

~~~powershell
cargo test -p brain-store
cargo test -p brain-store --test ledger_idempotency
~~~

Expected: all tests pass; EXPLAIN QUERY PLAN for project_id plus occurred_at uses the composite index.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-domain crates/brain-store docs/schemas/normalized-event.md
git commit -m "feat: add append-only evidence ledger"
~~~

### Task 4: Implement the Claude JSONL adapter

**Files:**
- Create: crates/brain-adapters/src/traits.rs
- Create: crates/brain-adapters/src/claude.rs
- Create: crates/brain-adapters/src/jsonl.rs
- Modify: crates/brain-adapters/src/lib.rs
- Create: crates/brain-adapters/tests/claude_fixture.rs
- Create: fixtures/claude/session.jsonl
- Create: fixtures/claude/truncated.jsonl
- Create: fixtures/claude/unknown-event.jsonl

**Interfaces:**
- Consumes: Claude JSONL source plus SourceCursor
- Produces: SourceAdapter implementation and `ReadOutcome` containing byte-offset batches, no-change, drift, or unavailable state

- [ ] **Step 1: Create redacted fixtures and failing adapter tests**

~~~rust
#[test]
fn claude_adapter_preserves_unknown_events() {
    let adapter = ClaudeAdapter::new(fixture_projects_root());
    let batch = adapter.read_fixture("unknown-event.jsonl").unwrap();
    let events = normalize_all(&adapter, batch);
    assert_eq!(events[0].event_type, EventType::SchemaUnknown);
    assert_eq!(events[0].raw["type"], "future-event");
}

#[test]
fn partial_final_line_is_not_committed() {
    let batch = ClaudeAdapter::new(fixture_projects_root())
        .read_fixture("truncated.jsonl")
        .unwrap();
    assert_eq!(batch.next_cursor.byte_offset, batch.last_complete_newline);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: cargo test -p brain-adapters --test claude_fixture

Expected: FAIL because ClaudeAdapter is undefined.

- [ ] **Step 3: Implement SourceAdapter and byte-safe JSONL reading**

~~~rust
pub struct RawRecord {
    pub source_id: String,
    pub byte_offset: u64,
    pub next_byte_offset: u64,
    pub value: serde_json::Value,
    pub raw_hash: [u8; 32],
}

pub struct RawRecordBatch {
    pub records: Vec<RawRecord>,
    pub next_cursor: SourceCursor,
    pub file_identity: FileIdentity,
}

pub enum ReadOutcome {
    Batch(RawRecordBatch),
    NoChange,
    SchemaDrift(SchemaDrift),
    SourceUnavailable(SourceUnavailable),
}
~~~

Use a BufReader seeked to the committed byte offset. Commit only newline-terminated records. Detect replacement using Windows file ID plus size/mtime. If size shrinks or identity changes, record a rotation event and restart safely.

- [ ] **Step 4: Map Claude types without dropping raw content**

Map user, assistant, system, attachment, queue-operation, relocated, mode, and known tool results. Derive sessionId, cwd, gitBranch, timestamp, parentUuid, and requestId when present. Unknown and malformed shapes produce SchemaUnknown or quarantine records with the raw hash.

- [ ] **Step 5: Run tests and property replay**

Run:

~~~powershell
cargo test -p brain-adapters
cargo test -p brain-adapters claude
~~~

Expected: fixtures, partial-line, rotation, malformed-line, and unknown-event tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-adapters fixtures/claude
git commit -m "feat: capture Claude session JSONL"
~~~

### Task 5: Build the persistent capture service

**Files:**
- Create: crates/brain-service/src/config.rs
- Create: crates/brain-service/src/capture.rs
- Create: crates/brain-service/src/reconcile.rs
- Create: crates/brain-service/src/health.rs
- Modify: crates/brain-service/src/main.rs
- Create: crates/brain-service/tests/restart_replay.rs
- Create: crates/brain-service/tests/project_isolation.rs

**Interfaces:**
- Consumes: ProjectRegistry, SourceAdapter, EventLedger
- Produces: CaptureSupervisor::run and ServiceHealth

- [ ] **Step 1: Write failing restart and isolation tests**

~~~rust
#[tokio::test]
async fn restart_replays_from_committed_cursor_without_duplicates() {
    let fixture = ServiceFixture::claude();
    fixture.run_until_events(10).await;
    fixture.stop_unclean().await;
    fixture.append_events(5);
    fixture.restart().await;
    assert_eq!(fixture.event_count(), 15);
}

#[tokio::test]
async fn two_same_named_projects_never_share_ledgers() {
    let fixture = ServiceFixture::two_projects_named_api();
    fixture.capture_both().await;
    assert_eq!(fixture.query_project(0).len(), 1);
    assert_eq!(fixture.query_project(1).len(), 1);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: cargo test -p brain-service

Expected: FAIL because CaptureSupervisor does not exist.

- [ ] **Step 3: Implement watcher plus periodic reconciliation**

Use notify for low latency and a two-second periodic scan for correctness. Debounce source paths for 50 ms. A per-source mutex serializes reads; batches across distinct sources may run concurrently.

~~~rust
pub struct CaptureSupervisor {
    registry: ProjectRegistry,
    adapters: Vec<std::sync::Arc<dyn SourceAdapter>>,
    stores: StorePool,
}

impl CaptureSupervisor {
    pub async fn run(self, shutdown: tokio::sync::watch::Receiver<bool>)
        -> anyhow::Result<()>;
}
~~~

- [ ] **Step 4: Add health and visible gaps**

Health reports service start time, last event per project, source cursor, parse failures, quarantined count, schema fingerprint changes, and backlog. Do not report healthy if any source has an unaccounted cursor gap.

- [ ] **Step 5: Run restart and isolation tests**

Run:

~~~powershell
cargo test -p brain-service
cargo test -p brain-service --test restart_replay
cargo test -p brain-service --test project_isolation
~~~

Expected: all pass deterministically across ten repeated runs.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-service
git commit -m "feat: add restart-safe capture service"
~~~

### Task 6: Implement the named-pipe protocol and compiled hook shim

**Files:**
- Create: crates/brain-domain/src/hook.rs
- Create: crates/brain-service/src/pipe.rs
- Create: crates/brain-hook/src/protocol.rs
- Modify: crates/brain-hook/src/main.rs
- Create: crates/brain-hook/tests/fail_open.rs
- Create: crates/brain-hook/tests/latency.rs
- Create: crates/brain-service/tests/pipe_roundtrip.rs

**Interfaces:**
- Consumes: hook JSON on stdin
- Produces: HookEnvelope over \\.\pipe\agent-brain-v1 and harness-compatible JSON on stdout

- [ ] **Step 1: Write failing protocol, fail-open, and latency tests**

~~~rust
#[test]
fn missing_service_exits_zero_and_spools() {
    let result = run_hook_without_service(session_start_payload());
    assert_eq!(result.exit_code, 0);
    assert!(result.spool_file.exists());
}

#[test]
fn warm_enqueue_p95_is_under_50_ms() {
    let samples = benchmark_hook(200, RunningService::new());
    assert!(percentile(&samples, 95) <= std::time::Duration::from_millis(50));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: cargo test -p brain-hook -p brain-service pipe

Expected: FAIL because the protocol is absent.

- [ ] **Step 3: Define a length-prefixed protocol**

~~~rust
#[derive(serde::Serialize, serde::Deserialize)]
pub struct HookEnvelope {
    pub protocol: u16,
    pub harness: Harness,
    pub event_name: String,
    pub received_at: time::OffsetDateTime,
    pub nonce: uuid::Uuid,
    pub payload: serde_json::Value,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct HookReply {
    pub additional_context: Option<String>,
    pub diagnostics_id: Option<String>,
}
~~~

Frame each JSON document with a four-byte little-endian length. Reject frames over 1 MiB. The service creates an access-controlled current-user named pipe.

- [ ] **Step 4: Implement fail-open spooling**

When connection or response exceeds 250 ms, append one newline-delimited envelope under BRAIN_HOME\runtime\spool using CreateNew plus atomic rename. Print an empty valid hook response and exit zero.

- [ ] **Step 5: Run protocol and latency tests**

Run:

~~~powershell
cargo test -p brain-hook
cargo test -p brain-service --test pipe_roundtrip
cargo test -p brain-hook --test latency -- --ignored --nocapture
~~~

Expected: correctness tests pass. The ignored hardware benchmark records warm/cold p50/p95/p99 and fails the release gate if p95 exceeds the contract.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-domain crates/brain-service crates/brain-hook
git commit -m "feat: add fail-open Windows hook bridge"
~~~

### Task 7: Compile bounded Claude SessionStart context

**Files:**
- Create: crates/brain-context/src/query.rs
- Create: crates/brain-context/src/compiler.rs
- Create: crates/brain-context/src/token_budget.rs
- Modify: crates/brain-context/src/lib.rs
- Create: crates/brain-context/tests/bounded_orientation.rs
- Create: crates/brain-service/src/hook_handler.rs
- Modify: crates/brain-service/src/main.rs
- Create: fixtures/claude/session-start.json

**Interfaces:**
- Consumes: ContextQuery and scoped EventLedger
- Produces: CompiledContext and Claude HookReply

- [ ] **Step 1: Write failing bounded-context tests**

~~~rust
#[test]
fn orientation_never_crosses_1500_normal_tokens() {
    let compiler = fixture_compiler_with_large_history();
    let result = compiler.compile(ContextQuery::startup(project_id())).unwrap();
    assert!(result.token_count <= 1500);
    assert!(result.text.contains("Evidence:"));
}

#[test]
fn another_project_cannot_enter_orientation() {
    let result = mixed_project_fixture().compile(project_a_query()).unwrap();
    assert!(!result.text.contains("PROJECT_B_SENTINEL"));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: cargo test -p brain-context

Expected: FAIL because ContextCompiler is undefined.

- [ ] **Step 3: Implement deterministic event selection**

~~~rust
pub struct ContextQuery {
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub native_session_id: Option<String>,
    pub max_tokens: usize,
}

pub struct CompiledContext {
    pub text: String,
    pub token_count: usize,
    pub evidence_ids: Vec<uuid::Uuid>,
}
~~~

Select latest task-like prompt, latest completed commands/tests, current branch/head, recent file changes, and last assistant response from the same project. Apply authority/order rules before rendering. Use a deterministic tokenizer and drop whole low-priority blocks until within budget.

- [ ] **Step 4: Format Claude hook output**

For SessionStart return:

~~~json
{
  "hookSpecificOutput": {
    "hookEventName": "SessionStart",
    "additionalContext": "<compiled context>"
  }
}
~~~

For capture-only events, return an empty object.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-context
cargo test -p brain-service hook_handler
~~~

Expected: budget, citation, no-history, corrupt-event, and project-isolation tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-context crates/brain-service fixtures/claude/session-start.json
git commit -m "feat: compile bounded Claude startup context"
~~~

### Task 8: Add CLI registration and the end-to-end walking-skeleton gate

**Files:**
- Create: crates/brain-cli/src/register.rs
- Create: crates/brain-cli/src/status.rs
- Create: crates/brain-cli/src/install_hooks.rs
- Modify: crates/brain-cli/src/main.rs
- Create: tests/e2e/claude_walking_skeleton.rs
- Create: tests/e2e/fixtures.rs
- Create: docs/operations/claude-walking-skeleton.md

**Interfaces:**
- Consumes: a project path and Claude settings location
- Produces: brain register, brain status, brain install-hooks claude, and a reproducible E2E test

- [ ] **Step 1: Write the failing CLI/E2E test**

~~~rust
#[tokio::test]
async fn fresh_session_receives_prior_task_without_export() {
    let e2e = E2eFixture::new();
    e2e.register_project();
    e2e.capture_claude_session("implement auth callback", "tests failed");
    let context = e2e.start_fresh_claude_session().await;
    assert!(context.contains("implement auth callback"));
    assert!(context.contains("tests failed"));
    assert!(token_count(&context) <= 1500);
}
~~~

- [ ] **Step 2: Run test and confirm failure**

Run: cargo test --test claude_walking_skeleton

Expected: FAIL because CLI registration and hook installation are absent.

- [ ] **Step 3: Implement CLI commands**

~~~rust
#[derive(clap::Subcommand)]
enum Command {
    Register { path: std::path::PathBuf },
    Status { project: Option<String>, json: bool },
    InstallHooks { harness: Harness },
    Query { project: String, text: String },
}
~~~

Hook installation creates a timestamped backup, merges only the brain hook entry, validates JSON, and atomically replaces the destination. Uninstall restores only brain-owned entries.

- [ ] **Step 4: Run the full foundation gate**

Run:

~~~powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --test claude_walking_skeleton -- --nocapture
cargo test -p brain-hook --test latency -- --ignored --nocapture
~~~

Expected: all functional tests pass; replay has zero duplicates; orientation is ≤1,500 tokens; project leakage is zero; measured hook p95 meets the release contract.

- [ ] **Step 5: Document operator verification**

Document exact register, service start, status, hook install, simulated restart, query, and uninstall commands. Include expected JSON health fields and the backup path.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-cli tests/e2e docs/operations/claude-walking-skeleton.md
git commit -m "feat: complete Claude walking skeleton"
~~~

## Foundation exit criteria

- cargo test --workspace passes.
- Claude adapter captures complete fixture events and preserves unknown records.
- Killing the service before cursor commit causes safe replay and zero duplicates.
- Same-named projects remain isolated.
- Hook capture benchmarks meet the latency contract on this Windows machine.
- A fresh Claude SessionStart receives evidence-linked context under 1,500 tokens.
- No LLM, Basic Memory, CodeGraph, LLM Wiki, or embedding dependency is installed by this phase.
