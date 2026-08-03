# Multi-Agent Capture Adapters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Add production capture and continuation parity for Codex and Hermes Agent while proving that Claude, Codex, and Hermes share one project ledger without cross-project leakage.

**Architecture:** Every harness implements the same fixture-driven `SourceAdapter` contract. Native records remain immutable evidence; adapters normalize only understood fields and quarantine malformed data. Hook formats are harness-specific, but project identity, ledger storage, and context compilation stay shared.

**Tech Stack:** Existing Rust workspace, Serde JSON, rusqlite, Windows file identity, SHA-256 fixtures, named-pipe hook bridge.

## Global Constraints

- Complete the foundation plan first and keep its public interfaces stable.
- Never infer a project solely from a repository basename or native session directory name.
- Preserve unknown native events and encrypted/unreadable payloads without fabricating content.
- A schema mismatch stops cursor advancement for the affected source and raises visible diagnostics.
- Hermes production activation is blocked until its installed schema fingerprint matches a reviewed fixture.
- Hook installation is additive, backed up, reversible, and fail-open.

---

### Task 1: Build the adapter conformance harness

**Files:**
- Create: crates/brain-adapters/src/conformance.rs
- Modify: crates/brain-adapters/src/traits.rs
- Modify: crates/brain-adapters/src/lib.rs
- Create: crates/brain-adapters/tests/conformance.rs
- Create: docs/schemas/source-adapter-contract.md

**Interfaces:**
- Consumes: any `SourceAdapter`, fixture source, starting cursor
- Produces: reusable conformance assertions for replay, partial records, rotation, unknown shapes, and isolation

- [ ] **Step 1: Write a failing conformance test around Claude**

~~~rust
#[test]
fn claude_satisfies_source_adapter_contract() {
    let subject = AdapterSubject::claude_fixture("session.jsonl");
    assert_adapter_conformance(subject);
}
~~~

The shared assertions verify stable fingerprinting, monotonic cursors, deterministic normalization, raw hash retention, idempotent replay, partial-record handling, and project scoping.

- [ ] **Step 2: Run the test and confirm failure**

Run: `cargo test -p brain-adapters --test conformance`

Expected: FAIL because the conformance harness is absent.

- [ ] **Step 3: Enforce the foundation adapter outcomes in the shared conformance suite**

~~~rust
pub struct SchemaDrift {
    pub source_id: String,
    pub expected: SchemaFingerprint,
    pub observed: SchemaFingerprint,
    pub sample_hash: [u8; 32],
}
~~~

Conformance must require that drift and malformed records never return a cursor beyond the last proven record.

- [ ] **Step 4: Run tests**

Run:

~~~powershell
cargo test -p brain-adapters --test conformance
cargo test -p brain-adapters
~~~

Expected: Claude passes the shared contract and all foundation adapter tests remain green.

- [ ] **Step 5: Commit**

~~~powershell
git add crates/brain-adapters docs/schemas/source-adapter-contract.md
git commit -m "test: add source adapter conformance contract"
~~~

### Task 2: Implement the Codex rollout adapter

**Files:**
- Create: crates/brain-adapters/src/codex.rs
- Modify: crates/brain-adapters/src/lib.rs
- Create: crates/brain-adapters/tests/codex_fixture.rs
- Create: fixtures/codex/rollout.jsonl
- Create: fixtures/codex/compacted.jsonl
- Create: fixtures/codex/encrypted-reasoning.jsonl
- Create: fixtures/codex/unknown-event.jsonl
- Create: docs/schemas/codex-rollout-map.md

**Interfaces:**
- Consumes: local Codex rollout JSONL plus byte cursor
- Produces: `CodexAdapter: SourceAdapter`

- [ ] **Step 1: Create redacted fixtures and failing tests**

~~~rust
#[test]
fn codex_maps_session_turn_and_compaction_events() {
    let events = normalize_codex_fixture("compacted.jsonl");
    assert!(events.iter().any(|e| e.event_type == EventType::SessionStarted));
    assert!(events.iter().any(|e| e.event_type == EventType::UserPrompt));
    assert!(events.iter().any(|e| e.event_type == EventType::Compaction));
}

#[test]
fn encrypted_reasoning_is_retained_but_not_decrypted_or_summarized() {
    let events = normalize_codex_fixture("encrypted-reasoning.jsonl");
    assert_eq!(events[0].event_type, EventType::OpaqueEvidence);
    assert!(events[0].payload.get("reasoning_text").is_none());
    assert!(events[0].raw_hash.iter().any(|byte| *byte != 0));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-adapters codex`

Expected: FAIL because `CodexAdapter` does not exist.

- [ ] **Step 3: Implement discovery and normalization**

Discover only configured Codex roots. Map `session_meta`, `turn_context`, `response_item`, `event_msg`, `world_state`, and `compacted` records. Within response items, map user messages, assistant messages, function calls, function outputs, and file/tool evidence. Keep unknown types as `SchemaUnknown`.

~~~rust
pub struct CodexAdapter {
    roots: Vec<std::path::PathBuf>,
}

impl SourceAdapter for CodexAdapter {
    fn name(&self) -> &'static str { "codex-rollout" }
    // discover, fingerprint, read_increment, normalize
}
~~~

Project resolution uses explicit registration plus rollout `cwd`; ambiguous or unregistered paths enter quarantine.

- [ ] **Step 4: Run conformance and fixture tests**

Run:

~~~powershell
cargo test -p brain-adapters codex
cargo test -p brain-adapters --test conformance
~~~

Expected: Codex passes byte replay, compaction, unknown-event, encrypted-payload, and project-isolation cases.

- [ ] **Step 5: Commit**

~~~powershell
git add crates/brain-adapters fixtures/codex docs/schemas/codex-rollout-map.md
git commit -m "feat: capture Codex rollout evidence"
~~~

### Task 3: Add Codex hook integration and output formatting

**Files:**
- Create: crates/brain-service/src/harness_output.rs
- Modify: crates/brain-service/src/hook_handler.rs
- Create: crates/brain-service/tests/codex_hook.rs
- Modify: crates/brain-cli/src/install_hooks.rs
- Create: crates/brain-cli/tests/codex_hook_install.rs
- Create: fixtures/codex/hook-input.json
- Create: docs/operations/codex-integration.md

**Interfaces:**
- Consumes: Codex notification payload and `CompiledContext`
- Produces: valid Codex hook response plus reversible hook configuration

- [ ] **Step 1: Write failing formatter and installer tests**

~~~rust
#[test]
fn codex_output_contains_only_bounded_additional_context() {
    let output = format_codex_context(compiled_context());
    assert!(output.is_valid_for_fixture("hook-input.json"));
    assert!(output.token_count() <= 1500);
}

#[test]
fn reinstall_changes_only_brain_owned_hook_entries() {
    let settings = fixture_with_unrelated_hooks();
    install_codex_hook(&settings).unwrap();
    install_codex_hook(&settings).unwrap();
    assert_eq!(settings.unrelated_hooks(), fixture_unrelated_hooks());
    assert_eq!(settings.brain_hook_count(), 1);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-service codex_hook; cargo test -p brain-cli codex_hook_install`

Expected: FAIL because Codex formatting and installation are absent.

- [ ] **Step 3: Implement harness output routing**

Create one formatter per harness. The shared hook handler resolves `project_id`, asks the same compiler for context, then applies the Codex-specific output envelope. Capture-only notifications return immediately.

- [ ] **Step 4: Implement backed-up install and uninstall**

Detect the supported local Codex configuration shape, write a timestamped backup, merge a brain-owned command pointing to `brain-hook.exe`, validate the result, then atomically replace it. Refuse unknown configuration schemas with remediation text.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-service codex_hook
cargo test -p brain-cli codex_hook_install
~~~

Expected: formatter, idempotent install, backup, uninstall, fail-open, and unrelated-setting preservation tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-service crates/brain-cli fixtures/codex docs/operations/codex-integration.md
git commit -m "feat: integrate bounded context with Codex"
~~~

### Task 4: Implement a guarded Hermes SQLite adapter

**Files:**
- Create: crates/brain-adapters/src/hermes.rs
- Modify: crates/brain-adapters/src/lib.rs
- Create: crates/brain-adapters/tests/hermes_fixture.rs
- Create: fixtures/hermes/state.db
- Create: fixtures/hermes/schema.sql
- Create: fixtures/hermes/schema-drift.sql
- Create: docs/schemas/hermes-state-map.md

**Interfaces:**
- Consumes: reviewed Hermes `state.db` snapshot plus composite cursor
- Produces: `HermesAdapter: SourceAdapter` or explicit `SchemaDrift`

- [ ] **Step 1: Create a synthetic reviewed schema and failing tests**

~~~rust
#[test]
fn hermes_reads_new_messages_after_composite_cursor() {
    let adapter = HermesAdapter::fixture("state.db");
    let batch = adapter.read_increment(&source(), &cursor("s1", 10)).unwrap();
    assert_eq!(batch.records.len(), 2);
    assert_eq!(batch.next_cursor, cursor("s1", 12));
}

#[test]
fn unknown_hermes_schema_blocks_cursor_advance() {
    let outcome = HermesAdapter::fixture_schema("schema-drift.sql").read();
    assert!(matches!(outcome, ReadOutcome::SchemaDrift(_)));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-adapters hermes`

Expected: FAIL because the adapter is absent.

- [ ] **Step 3: Implement read-only WAL-safe access**

Open the native database read-only with an immutable snapshot strategy that includes WAL-visible rows. Never migrate, checkpoint, vacuum, or write the Hermes database. Cursor by stable session/message identifiers, not `rowid` alone.

~~~rust
pub struct HermesSchemaProfile {
    pub fingerprint: SchemaFingerprint,
    pub session_table: String,
    pub message_table: String,
    pub message_order_columns: Vec<String>,
}
~~~

Map user/assistant/tool messages, task/session metadata, timestamps, and working directory when present. Unknown columns are retained in raw evidence.

- [ ] **Step 4: Add the production activation guard**

`brain status --harness hermes` reports `fixture_only` until the user points the CLI at a local Hermes database and its fingerprint matches a reviewed profile. A mismatch reports the hash and exports only a redacted schema description for review.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-adapters hermes
cargo test -p brain-adapters --test conformance
~~~

Expected: snapshot, WAL, replay, composite cursor, drift, and conformance tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-adapters fixtures/hermes docs/schemas/hermes-state-map.md
git commit -m "feat: add guarded Hermes state adapter"
~~~

### Task 5: Add schema drift quarantine and diagnostics

**Files:**
- Create: crates/brain-store/src/quarantine.rs
- Modify: crates/brain-store/src/lib.rs
- Modify: crates/brain-service/src/capture.rs
- Modify: crates/brain-service/src/health.rs
- Create: crates/brain-service/tests/schema_drift.rs
- Modify: crates/brain-cli/src/status.rs
- Create: crates/brain-cli/src/diagnose.rs

**Interfaces:**
- Consumes: `SchemaDrift`, parse failure, malformed native record
- Produces: durable quarantine record, source pause, health status, diagnostics bundle

- [ ] **Step 1: Write failing source-local pause tests**

~~~rust
#[tokio::test]
async fn drift_pauses_only_the_affected_source() {
    let fixture = MultiSourceFixture::with_one_drifted_codex_source();
    fixture.capture().await;
    assert_eq!(fixture.codex_cursor(), fixture.original_codex_cursor());
    assert!(fixture.claude_event_count() > 0);
    assert_eq!(fixture.health().drifted_sources, 1);
}
~~~

- [ ] **Step 2: Run test and confirm failure**

Run: `cargo test -p brain-service schema_drift`

Expected: FAIL because drift is not persisted or source-local.

- [ ] **Step 3: Implement durable quarantine and status**

Persist project, source, cursor, expected/observed fingerprint, raw hash, timestamp, and reason. Never store secrets in the diagnostic export. `brain diagnose --source <id>` emits a redacted JSON bundle and exact cursor state.

- [ ] **Step 4: Run tests**

Run:

~~~powershell
cargo test -p brain-service schema_drift
cargo test -p brain-cli diagnose
~~~

Expected: the bad source pauses, healthy sources continue, cursor stays fixed, and status is actionable.

- [ ] **Step 5: Commit**

~~~powershell
git add crates/brain-store crates/brain-service crates/brain-cli
git commit -m "feat: surface native schema drift safely"
~~~

### Task 6: Prove cross-agent handoff and project isolation

**Files:**
- Create: tests/e2e/multi_agent_handoff.rs
- Create: tests/e2e/multi_project_isolation.rs
- Modify: tests/e2e/fixtures.rs
- Create: docs/operations/multi-agent-handoff.md

**Interfaces:**
- Consumes: Claude, Codex, and Hermes fixture sessions for two projects
- Produces: release-gate evidence for shared memory and hard isolation

- [ ] **Step 1: Write failing handoff tests**

~~~rust
#[tokio::test]
async fn codex_continues_a_task_started_in_claude_and_updated_in_hermes() {
    let e2e = E2eFixture::three_harnesses();
    e2e.capture_claude("Project A", "add OAuth callback", "test is failing");
    e2e.capture_hermes("Project A", "fixed redirect URI", "test now passes");
    let codex_context = e2e.session_start("Project A", Harness::Codex).await;
    assert!(codex_context.contains("OAuth callback"));
    assert!(codex_context.contains("redirect URI"));
    assert!(codex_context.contains("passes"));
}

#[tokio::test]
async fn identical_secret_sentinels_in_another_project_are_never_returned() {
    let e2e = E2eFixture::two_projects();
    e2e.capture_sentinel_in_project_b("PROJECT_B_ONLY");
    assert!(!e2e.context_for_project_a().await.contains("PROJECT_B_ONLY"));
}

#[tokio::test]
async fn post_compaction_reorients_to_the_same_active_task_and_checkpoint() {
    let e2e = E2eFixture::three_harnesses();
    e2e.capture_task_and_checkpoint("Project A", "OAuth callback", "next: add PKCE");
    let context = e2e.compact_and_resume("Project A", Harness::Codex).await;
    assert!(context.contains("OAuth callback"));
    assert!(context.contains("next: add PKCE"));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test --test multi_agent_handoff; cargo test --test multi_project_isolation`

Expected: FAIL until all adapters and hook formats are wired into the service.

- [ ] **Step 3: Register all adapters in the service**

Load enabled adapters from configuration, expose per-harness/source health, and use the shared project registry. No adapter-specific branching is permitted below normalization.

- [ ] **Step 4: Run the multi-agent release gate**

Run:

~~~powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --test multi_agent_handoff -- --nocapture
cargo test --test multi_project_isolation -- --nocapture
~~~

Expected: Claude to Codex to Hermes handoffs work in every direction; post-compaction orientation preserves task/checkpoint; the project leakage count is zero; all contexts remain within 1,500 tokens.

- [ ] **Step 5: Commit**

~~~powershell
git add crates tests/e2e docs/operations/multi-agent-handoff.md
git commit -m "feat: complete cross-agent evidence handoff"
~~~

## Multi-agent exit criteria

- Claude and Codex production adapters pass the shared conformance suite.
- Hermes passes fixture conformance and cannot activate against an unreviewed schema.
- Native schema drift pauses only the affected source without silent cursor advancement.
- All three harnesses read project-scoped context from one canonical ledger.
- Bidirectional handoff tests pass without transcript export or full-codebase rereads.
- Post-compaction sessions receive the same scoped active task and current checkpoint.
- Same-named and deliberately adversarial projects have zero memory leakage.
