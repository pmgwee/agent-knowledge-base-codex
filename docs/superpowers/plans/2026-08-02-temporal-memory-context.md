# Temporal Memory and Context Compiler Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Convert captured evidence into durable, temporally correct project memory and retrieve only the smallest evidence-cited context needed by each agent session.

**Architecture:** Raw events remain canonical. An asynchronous consolidation pipeline produces versioned memory records and Markdown projections with explicit validity intervals and supersession links. Retrieval applies hard project scope, temporal/authority filters, FTS ranking, optional provider results, and deterministic token budgets.

**Tech Stack:** Rust, SQLite WAL and FTS5, Markdown with YAML front matter, Reqwest, GLM through an OpenAI-compatible endpoint, Basic Memory v0.22.1 as an external index, stdio MCP.

## Global Constraints

- Complete the multi-agent plan first.
- Current Git/test/deployment state outranks every stored memory.
- Raw evidence is never overwritten by summaries or corrections.
- Consolidation is asynchronous, idempotent, restartable, and project-scoped.
- LLM output is untrusted proposed structure until schema validation and evidence checks pass.
- Retrieval must work without GLM, embeddings, Basic Memory, CodeGraph, or LLM Wiki.
- Normal context remains at most 1,500 tokens and the hard ceiling is 3,000.
- Global preferences use a separate scope and store; project facts never auto-promote to it.

---

### Task 1: Define versioned memory records and projections

**Files:**
- Create: crates/brain-domain/src/memory.rs
- Modify: crates/brain-domain/src/lib.rs
- Create: crates/brain-store/src/memory.rs
- Modify: crates/brain-store/src/migrations.rs
- Modify: crates/brain-store/src/lib.rs
- Create: crates/brain-store/tests/memory_versions.rs
- Create: crates/brain-store/tests/global_preferences.rs
- Create: docs/schemas/memory-record.md

**Interfaces:**
- Consumes: evidence IDs and validated memory content in a project or explicit global-preference scope
- Produces: append-only `MemoryRecord`, versions, evidence links, and projection metadata

- [ ] **Step 1: Write failing versioning tests**

~~~rust
#[test]
fn correcting_memory_appends_a_version_and_preserves_the_old_value() {
    let store = TestStore::new();
    let first = store.append_memory(decision("use Redis", evidence_a())).unwrap();
    let second = store.correct_memory(first.id, "use SQLite", evidence_b()).unwrap();
    assert_eq!(store.memory_versions(first.id).unwrap().len(), 2);
    assert_eq!(store.current_memory(first.id).unwrap().content, "use SQLite");
}

#[test]
fn memory_cannot_link_evidence_from_another_project() {
    let result = TestStore::new().append_memory(cross_project_memory());
    assert!(matches!(result, Err(StoreError::ProjectScopeViolation)));
}

#[test]
fn project_fact_cannot_be_written_to_global_preferences() {
    let result = TestStore::new().promote_to_global(project_fact("production URL"));
    assert!(matches!(result, Err(StoreError::InvalidGlobalMemoryKind)));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-store memory_versions`

Expected: FAIL because memory tables and types are absent.

- [ ] **Step 3: Define memory kinds and temporal metadata**

~~~rust
pub enum MemoryKind {
    Checkpoint,
    Decision,
    Fact,
    Investigation,
    Procedure,
    Deployment,
    Timeline,
    Preference,
    Task,
}

pub enum MemoryScope {
    Project(ProjectId),
    GlobalPreferences,
}

pub struct MemoryRecord {
    pub id: uuid::Uuid,
    pub version_id: uuid::Uuid,
    pub scope: MemoryScope,
    pub worktree_id: Option<WorktreeId>,
    pub task_id: Option<uuid::Uuid>,
    pub kind: MemoryKind,
    pub title: String,
    pub content: String,
    pub valid_from: time::OffsetDateTime,
    pub valid_to: Option<time::OffsetDateTime>,
    pub recorded_at: time::OffsetDateTime,
    pub confidence: f32,
    pub authority: Authority,
    pub evidence_ids: Vec<uuid::Uuid>,
    pub supersedes: Vec<uuid::Uuid>,
    pub status: MemoryStatus,
}
~~~

- [ ] **Step 4: Implement append-only migrations and Markdown projection IDs**

Create `memory_records`, `memory_versions`, `memory_evidence`, `memory_supersession`, `global_preferences`, and `projection_state`. Use immutable version rows and a stable logical memory ID. Project Markdown paths derive from project UUID, kind, year/month, and memory ID; global preferences live under a separate root. The database rejects non-preference kinds in global scope. Promotion requires an explicit audited user action.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-store memory
cargo test -p brain-domain memory
~~~

Expected: append, correction, evidence scope, time interval, explicit promotion, global-kind restriction, and stable projection-path tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-domain crates/brain-store docs/schemas/memory-record.md
git commit -m "feat: add versioned temporal memory records"
~~~

### Task 2: Implement authority, supersession, and conflict resolution

**Files:**
- Create: crates/brain-context/src/authority.rs
- Create: crates/brain-context/src/supersession.rs
- Modify: crates/brain-context/src/lib.rs
- Create: crates/brain-context/tests/authority.rs
- Create: crates/brain-context/tests/supersession.rs
- Create: docs/schemas/memory-precedence.md

**Interfaces:**
- Consumes: live facts, evidence events, human corrections, checkpoints, derived memory
- Produces: `ResolvedMemorySet` with winning records and visible conflicts

- [ ] **Step 1: Write failing precedence and reversal tests**

~~~rust
#[test]
fn live_test_failure_overrides_an_old_passing_checkpoint() {
    let resolved = resolve(vec![old_checkpoint_passed(), live_test_failed()]);
    assert_eq!(resolved.winner().authority, Authority::LiveState);
    assert!(resolved.rendered_warning().contains("memory said passing"));
}

#[test]
fn later_decision_supersedes_the_old_decision_without_erasing_history() {
    let resolved = resolve(vec![decision_redis(), decision_sqlite_superseding_redis()]);
    assert_eq!(resolved.current().content, "SQLite");
    assert_eq!(resolved.historical().len(), 2);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-context authority; cargo test -p brain-context supersession`

Expected: FAIL because the resolver does not exist.

- [ ] **Step 3: Encode the locked precedence order**

~~~rust
pub enum Authority {
    LiveState,
    RawMechanicalEvidence,
    HumanCorrection,
    AgentCheckpoint,
    DerivedMemory,
    ExternalDocument,
}

pub fn resolve_candidates(
    as_of: time::OffsetDateTime,
    candidates: Vec<MemoryCandidate>,
) -> ResolvedMemorySet;
~~~

Apply project scope first, validity interval second, explicit supersession third, authority fourth, recency fifth, and confidence last. Human corrections outrank agents for intent/preferences, but do not override current test/Git/deployment facts.

- [ ] **Step 4: Preserve unresolved conflicts**

When two active records have equal scope/authority and incompatible values, return both under a conflict label. Never guess a winner from vector similarity.

- [ ] **Step 5: Run tests**

Run: `cargo test -p brain-context authority supersession`

Expected: reversal, validity-window, human-intent, live-state, equal-authority conflict, and clock-skew tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-context docs/schemas/memory-precedence.md
git commit -m "feat: resolve temporal memory by evidence authority"
~~~

### Task 3: Build the restartable consolidation queue

**Files:**
- Create: crates/brain-store/src/jobs.rs
- Modify: crates/brain-store/src/migrations.rs
- Create: crates/brain-service/src/consolidation.rs
- Create: crates/brain-service/tests/consolidation_replay.rs
- Create: crates/brain-service/tests/consolidation_isolation.rs

**Interfaces:**
- Consumes: committed event ranges per project and session/task boundary
- Produces: idempotent `ConsolidationJob` and proposed memory batches

- [ ] **Step 1: Write failing crash/replay tests**

~~~rust
#[tokio::test]
async fn crash_after_memory_write_before_job_ack_does_not_duplicate_memory() {
    let fixture = ConsolidationFixture::new();
    fixture.crash_at(CrashPoint::BeforeJobAck).await;
    fixture.restart().await;
    assert_eq!(fixture.logical_memory_count(), fixture.expected_memory_count());
}

#[tokio::test]
async fn jobs_never_mix_projects() {
    let fixture = ConsolidationFixture::two_projects();
    fixture.run().await;
    assert!(fixture.every_job_has_one_project());
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-service consolidation`

Expected: FAIL because the queue and worker are absent.

- [ ] **Step 3: Implement durable jobs and deterministic batching**

~~~rust
pub struct ConsolidationJob {
    pub id: uuid::Uuid,
    pub project_id: ProjectId,
    pub first_event_id: uuid::Uuid,
    pub last_event_id: uuid::Uuid,
    pub reason: ConsolidationReason,
    pub attempt: u32,
}
~~~

Enqueue on session stop, compaction, explicit checkpoint, 30 minutes of inactivity, or 200 new events. Generate the job idempotency key from project and event range. Use leases for workers and exponential retry with a dead-letter state after five failed attempts.

- [ ] **Step 4: Add deterministic secret redaction before external LLM calls**

Redact configured patterns, API key formats, `.env` values, and high-entropy tokens. Store a redaction manifest containing only hashes and categories. Raw local evidence remains unchanged.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-service consolidation
cargo test -p brain-store jobs
~~~

Expected: crash/replay, retry, dead-letter, event-range, redaction, and isolation tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-store crates/brain-service
git commit -m "feat: add restartable memory consolidation jobs"
~~~

### Task 4: Add provider-neutral consolidation and the GLM adapter

**Files:**
- Create: crates/brain-context/src/llm.rs
- Create: crates/brain-context/src/glm.rs
- Modify: crates/brain-context/src/lib.rs
- Modify: crates/brain-service/src/consolidation.rs
- Create: crates/brain-context/tests/glm_contract.rs
- Create: fixtures/providers/glm-memory-response.json
- Create: fixtures/providers/glm-invalid-response.json
- Create: docs/operations/glm-configuration.md

**Interfaces:**
- Consumes: redacted evidence packet and configured GLM endpoint/model/key reference
- Produces: schema-validated `ProposedMemoryBatch`

- [ ] **Step 1: Write failing provider contract tests**

~~~rust
#[tokio::test]
async fn invalid_llm_evidence_ids_are_rejected() {
    let provider = FixtureLlm::response("glm-invalid-response.json");
    let result = consolidate_with(provider, evidence_packet()).await;
    assert!(matches!(result, Err(ConsolidationError::UnknownEvidenceId(_))));
}

#[tokio::test]
async fn unavailable_llm_leaves_job_retryable_and_capture_healthy() {
    let result = consolidate_with(UnavailableLlm, evidence_packet()).await;
    assert!(result.is_retryable());
    assert!(capture_health().is_healthy());
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-context glm_contract`

Expected: FAIL because the provider contract is absent.

- [ ] **Step 3: Define the provider and strict response schema**

~~~rust
#[async_trait::async_trait]
pub trait ConsolidationLlm: Send + Sync {
    async fn propose(&self, packet: &EvidencePacket) -> anyhow::Result<ProposedMemoryBatch>;
}

pub struct ProposedMemory {
    pub kind: MemoryKind,
    pub title: String,
    pub content: String,
    pub valid_from: time::OffsetDateTime,
    pub confidence: f32,
    pub evidence_ids: Vec<uuid::Uuid>,
    pub supersedes: Vec<uuid::Uuid>,
}
~~~

Reject unknown evidence IDs, missing citations, invalid timestamps, oversized fields, cross-project links, and unsupported kinds.

- [ ] **Step 4: Implement the OpenAI-compatible GLM client**

Use configuration for base URL, model name, timeout, and environment-variable key name. Never persist or log the key. Require JSON schema output when the endpoint supports it; otherwise validate strict JSON locally. Use bounded retries for 429/5xx only.

- [ ] **Step 5: Run contract tests**

Run: `cargo test -p brain-context glm_contract`

Expected: success, invalid JSON, hallucinated evidence, timeout, 429 retry, secret logging, and provider-outage tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-context crates/brain-service fixtures/providers docs/operations/glm-configuration.md
git commit -m "feat: consolidate evidence through validated GLM output"
~~~

### Task 5: Implement FTS retrieval and temporal queries

**Files:**
- Create: crates/brain-store/src/search.rs
- Modify: crates/brain-store/src/migrations.rs
- Modify: crates/brain-store/src/lib.rs
- Create: crates/brain-store/tests/search_scope.rs
- Create: crates/brain-store/tests/temporal_query.rs
- Create: crates/brain-context/src/retrieval.rs
- Create: crates/brain-context/tests/retrieval.rs

**Interfaces:**
- Consumes: `ContextQuery`, project ID, optional as-of/range/task/worktree filters
- Produces: ranked evidence and memory candidates with scores and reasons

- [ ] **Step 1: Write failing search and last-week tests**

~~~rust
#[test]
fn last_week_uses_occurred_at_and_reports_late_observation() {
    let result = fixture().search(Query::last_week(project_id(), fixed_now()));
    assert!(result.contains_event("worked_last_week_ingested_today"));
    assert!(result.item(0).late_observation);
}

#[test]
fn text_ranking_cannot_escape_project_scope() {
    let result = mixed_fixture().search(Query::text(project_a(), "unique sentinel"));
    assert!(result.items.iter().all(|item| item.project_id == project_a()));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-store search; cargo test -p brain-context retrieval`

Expected: FAIL because FTS and retrieval are absent.

- [ ] **Step 3: Add contentless FTS5 tables and scoped queries**

Index memory titles/content, selected event text, normalized paths, task labels, and aliases. Always obtain candidate row IDs through a `project_id`-scoped query path. Never accept project scope as an optional post-filter.

- [ ] **Step 4: Implement hybrid deterministic ranking**

Rank exact task/session continuity, explicit paths/symbols, authority, current validity, recency decay, BM25, evidence completeness, and worktree match. Return score components for diagnostics.

- [ ] **Step 5: Run tests and query-plan checks**

Run:

~~~powershell
cargo test -p brain-store search temporal_query
cargo test -p brain-context retrieval
~~~

Expected: keyword, path, exact task, last-day/week/month, as-of, superseded exclusion, conflict, and isolation tests pass; query plans use scoped indexes.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-store crates/brain-context
git commit -m "feat: add scoped temporal memory retrieval"
~~~

### Task 6: Project Markdown and Basic Memory v0.22.1 safely

**Files:**
- Create: crates/brain-store/src/markdown.rs
- Create: crates/brain-store/src/basic_memory.rs
- Modify: crates/brain-store/src/lib.rs
- Create: crates/brain-store/tests/markdown_projection.rs
- Create: crates/brain-store/tests/basic_memory_rebuild.rs
- Create: crates/brain-service/src/note_watcher.rs
- Create: crates/brain-service/tests/note_watcher.rs
- Create: crates/brain-cli/src/rebuild.rs
- Modify: crates/brain-cli/src/main.rs
- Create: docs/operations/obsidian-basic-memory.md

**Interfaces:**
- Consumes: current memory versions
- Produces: atomic Markdown vault projection and rebuildable Basic Memory index

- [ ] **Step 1: Write failing projection and rebuild tests**

~~~rust
#[test]
fn projection_is_deterministic_and_preserves_user_notes() {
    let vault = ProjectionFixture::with_user_note();
    vault.project_twice().unwrap();
    assert_eq!(vault.generated_hash_before(), vault.generated_hash_after());
    assert!(vault.user_note_exists());
}

#[test]
fn deleting_basic_memory_state_does_not_delete_canonical_memory() {
    let fixture = BasicMemoryFixture::new();
    fixture.delete_external_index();
    fixture.rebuild().unwrap();
    assert_eq!(fixture.retrievable_memory_count(), fixture.canonical_memory_count());
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-store markdown_projection basic_memory_rebuild`

Expected: FAIL because projection is absent.

- [ ] **Step 3: Implement atomic generated Markdown**

Write generated files under `vault/projects/<project-uuid>/generated/` with YAML metadata for IDs, version, kind, validity, authority, evidence, and supersession. Obsidian may edit only `notes/`; generated content is service-owned. Stage a complete generation, validate links/hashes, then atomically switch a manifest pointer.

- [ ] **Step 4: Integrate Basic Memory only through released public behavior**

Pin v0.22.1 in install documentation and diagnostics. Point it at the generated project vault or invoke its public CLI/MCP behavior. Never open or copy its internal database. If unavailable, mark the projection degraded and continue using SQLite FTS.

- [ ] **Step 5: Validate human Obsidian edits through a watcher**

Watch only `notes/` and the separate global-preferences notes root. Parse front matter, enforce scope/kind/evidence rules, append a human-correction or explicit-promotion audit event, then create a new memory version. Invalid or cross-project links enter a visible review queue; generated files remain read-only service output.

- [ ] **Step 6: Add rebuild and consistency commands**

~~~text
brain rebuild markdown --project <id>
brain rebuild basic-memory --project <id>
brain verify projections --project <id>
~~~

- [ ] **Step 7: Run tests**

Run:

~~~powershell
cargo test -p brain-store markdown_projection basic_memory_rebuild
cargo test -p brain-service note_watcher
cargo test -p brain-cli rebuild
~~~

Expected: deterministic generation, crash-safe swap, user-note validation/audit, global promotion controls, unavailable-index fallback, deletion/rebuild, and checksum tests pass.

- [ ] **Step 8: Commit**

~~~powershell
git add crates/brain-store crates/brain-service crates/brain-cli docs/operations/obsidian-basic-memory.md
git commit -m "feat: project memory to Obsidian and Basic Memory"
~~~

### Task 7: Complete the bounded context compiler

**Files:**
- Modify: crates/brain-context/src/query.rs
- Modify: crates/brain-context/src/compiler.rs
- Modify: crates/brain-context/src/token_budget.rs
- Create: crates/brain-context/src/citations.rs
- Create: crates/brain-context/src/providers.rs
- Create: crates/brain-context/src/live_state.rs
- Create: crates/brain-context/tests/context_quality.rs
- Create: crates/brain-context/tests/abstention.rs
- Create: crates/brain-context/tests/live_state.rs
- Modify: crates/brain-service/src/hook_handler.rs

**Interfaces:**
- Consumes: live state, resolved memory, raw evidence, provider results
- Produces: evidence-cited context blocks under explicit per-section budgets

- [ ] **Step 1: Write failing quality, budget, and abstention tests**

~~~rust
#[test]
fn compiler_prefers_current_failure_over_superseded_success() {
    let context = fixture_with_reversed_test_state().compile();
    assert!(context.text.contains("currently failing"));
    assert!(!context.text.contains("Current status: passing"));
}

#[test]
fn weak_evidence_produces_an_explicit_unknown_not_a_guess() {
    let context = empty_fixture().compile_task("why was cache removed?");
    assert!(context.text.contains("No reliable project evidence found"));
}

#[test]
fn hard_budget_is_never_exceeded_even_with_all_providers_enabled() {
    let context = oversized_fixture().compile_with_hard_budget(3000);
    assert!(context.token_count <= 3000);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-context context_quality abstention`

Expected: FAIL because the compiler only supports the foundation orientation.

- [ ] **Step 3: Implement fixed priority sections**

Render in this order: identity/live state; active task and lease; current checkpoint; unresolved failures/risks; relevant decisions/procedures; recent timeline; optional document/code provider results; evidence citations. Allocate 1,500 normal tokens and allow an explicit caller ceiling up to 3,000. Drop whole low-priority items, never truncate citation IDs or negate sentences.

- [ ] **Step 4: Implement deterministic live-state acquisition**

Inspect the exact registered worktree with read-only Git commands for HEAD, branch, upstream/divergence, dirty paths, conflicts, and submodule state. Join the latest captured test/deployment events only when their Git revision is compatible, and label their observation time and staleness. A failing or dirty current state outranks any stored success. Live-state failure is visible and never replaced by a confident memory claim.

- [ ] **Step 5: Add citations and provider deadlines**

Every claim links to event or memory IDs and includes source harness/time when useful. Query providers concurrently after canonical retrieval, with per-provider deadlines and total hook deadline. A provider timeout cannot remove canonical results or fail the hook.

- [ ] **Step 6: Run quality tests**

Run:

~~~powershell
cargo test -p brain-context context_quality abstention
cargo test -p brain-context live_state
cargo test -p brain-service hook_handler
~~~

Expected: precedence, reversal, contradictions, citation, Unicode, empty history, provider timeout, 1,500 normal, and 3,000 hard-limit tests pass.

- [ ] **Step 7: Commit**

~~~powershell
git add crates/brain-context crates/brain-service
git commit -m "feat: compile authoritative evidence-cited context"
~~~

### Task 8: Expose shared retrieval through MCP and prove temporal recall

**Files:**
- Create: crates/brain-mcp/Cargo.toml
- Create: crates/brain-mcp/src/main.rs
- Create: crates/brain-mcp/src/protocol.rs
- Create: crates/brain-mcp/src/tools.rs
- Modify: Cargo.toml
- Create: crates/brain-mcp/tests/protocol.rs
- Create: tests/e2e/temporal_recall.rs
- Create: tests/e2e/provider_outage.rs
- Create: docs/operations/mcp-and-query.md

**Interfaces:**
- Consumes: stdio MCP requests or `brain query`
- Produces: project-scoped search, timeline, checkpoint, evidence, and correction tools

- [ ] **Step 1: Write failing MCP and temporal E2E tests**

~~~rust
#[tokio::test]
async fn last_week_returns_cited_cross_agent_work_and_current_status() {
    let e2e = TemporalFixture::fixed_clock();
    e2e.seed_week_of_cross_agent_work();
    let result = e2e.mcp_query("What did I do last week?").await;
    assert!(result.has_citations_for_every_item());
    assert!(result.distinguishes_historical_from_current_state());
}

#[tokio::test]
async fn retrieval_survives_llm_and_basic_memory_outage() {
    let result = OutageFixture::all_optional_services_down().query("OAuth").await;
    assert!(result.contains_canonical_fts_match());
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-mcp; cargo test --test temporal_recall`

Expected: FAIL because the MCP crate is absent.

- [ ] **Step 3: Implement minimal MCP tools**

Expose `brain_search`, `brain_timeline`, `brain_checkpoint`, `brain_evidence`, `brain_correct`, and `brain_status`. Require project identity in every request; resolve aliases centrally; return structured citations and truncation metadata. Mutating correction calls append versions and audit events.

- [ ] **Step 4: Register the crate and add CLI parity**

Both MCP and CLI call the service protocol rather than open ledgers independently. Add `brain query`, `brain timeline`, and `brain checkpoint` commands with JSON output.

- [ ] **Step 5: Run the temporal release gate**

Run:

~~~powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --test temporal_recall -- --nocapture
cargo test --test provider_outage -- --nocapture
~~~

Expected: decision reversal, last-day/week/month, current-vs-historical, citations, correction audit, Basic Memory rebuild, and complete optional-provider outage tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add Cargo.toml Cargo.lock crates/brain-mcp crates/brain-cli tests/e2e docs/operations/mcp-and-query.md
git commit -m "feat: expose temporal brain retrieval through MCP"
~~~

## Temporal-memory exit criteria

- Every curated memory version has project scope, validity, authority, and evidence citations.
- Reversed decisions and stale status never present as current truth.
- GLM failures delay consolidation without affecting capture or canonical retrieval.
- Obsidian Markdown is deterministic; user notes are separated; Basic Memory is disposable and rebuildable.
- FTS-only retrieval remains useful with every optional provider offline.
- “What did I do last week?” returns a correct, evidence-cited cross-agent timeline.
- Session context stays within 1,500 normal and 3,000 hard tokens.
