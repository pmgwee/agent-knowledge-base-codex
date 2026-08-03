# CodeGraph and LLM Wiki Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Add CodeGraph for current-code navigation and LLM Wiki for external document knowledge without making either provider canonical, mandatory, slow, or able to leak across projects.

**Architecture:** Both integrations sit behind stable provider contracts and runtime feature flags. CodeGraph is a worktree-specific code-truth candidate provider activated only after an A/B gate. LLM Wiki uses a separate document vault and two-stage retrieval: cached project/task knowledge at SessionStart, then one live query on the first user prompt.

**Tech Stack:** Existing Rust context/service/CLI crates, subprocess or HTTP adapters discovered from released provider behavior, Reqwest, SQLite provider cache, fixture servers, benchmark harness.

## Global Constraints

- Complete the operations plan first; optional providers cannot delay the production core.
- Providers never write canonical events, curated memory, tasks, leases, or project identity.
- Every provider call is hard-scoped to one project and, for code, one exact worktree/HEAD.
- Provider errors, timeouts, stale indexes, malformed responses, or uninstall must fail open.
- LLM Wiki owns a separate document vault; the brain may cache/read results but never co-own the vault.
- LLM Wiki contributes at most three cited results and 300–600 tokens inside the existing budget.
- Live LLM Wiki retrieval has a 300 ms provider deadline.
- No CodeGraph index is reused across divergent worktrees.

---

### Task 1: Add provider configuration, resilience, and feature flags

**Files:**
- Modify: crates/brain-context/src/providers.rs
- Create: crates/brain-context/src/provider_config.rs
- Create: crates/brain-context/src/provider_guard.rs
- Create: crates/brain-context/tests/provider_guard.rs
- Modify: crates/brain-domain/src/config.rs
- Create: docs/schemas/provider-configuration.md

**Interfaces:**
- Consumes: per-project provider configuration and `ContextQuery`
- Produces: guarded `ContextProvider`, `CodeTruthProvider`, and `DocumentKnowledgeProvider` calls

- [ ] **Step 1: Write failing timeout, scope, and disable tests**

~~~rust
#[tokio::test]
async fn disabled_provider_is_never_invoked() {
    let provider = CountingProvider::new();
    ProviderGuard::disabled(provider.clone()).retrieve(query()).await;
    assert_eq!(provider.calls(), 0);
}

#[tokio::test]
async fn timeout_returns_empty_degraded_result_without_failing_context() {
    let result = ProviderGuard::with_deadline(HangingProvider, millis(300))
        .retrieve(query()).await;
    assert!(result.items.is_empty());
    assert_eq!(result.status, ProviderStatus::TimedOut);
}

#[tokio::test]
async fn result_with_wrong_project_scope_is_discarded_and_audited() {
    let result = guarded(CrossProjectProvider).retrieve(project_a_query()).await;
    assert!(result.items.is_empty());
    assert_eq!(result.status, ProviderStatus::ScopeViolation);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-context provider_guard`

Expected: FAIL because provider guards and config are absent.

- [ ] **Step 3: Define explicit runtime configuration**

~~~rust
pub struct ProviderConfig {
    pub codegraph: CodeGraphConfig,
    pub llm_wiki: LlmWikiConfig,
}

pub struct CodeGraphConfig {
    pub enabled: bool,
    pub executable: Option<std::path::PathBuf>,
    pub per_worktree_indexes: bool,
    pub deadline_ms: u64,
}

pub struct LlmWikiConfig {
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub vault: Option<std::path::PathBuf>,
    pub deadline_ms: u64,
    pub max_results: usize,
    pub max_tokens: usize,
}
~~~

Defaults disable both providers. Enforce LLM Wiki `max_results <= 3`, `max_tokens <= 600`, and deadline at most 300 ms for hook retrieval.

- [ ] **Step 4: Implement circuit breaker and diagnostics**

After three consecutive failures, stop live calls for five minutes while serving a valid cache. A half-open probe cannot run in a hook request. Emit status, latency, freshness, last success, cache age, and reason without secrets.

- [ ] **Step 5: Run tests**

Run: `cargo test -p brain-context provider_guard`

Expected: disabled, timeout, malformed, scope violation, circuit open/half-open, stale cache, cancellation, and canonical-context preservation tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-context crates/brain-domain docs/schemas/provider-configuration.md
git commit -m "feat: guard optional knowledge providers"
~~~

### Task 2: Implement the worktree-specific CodeGraph adapter

**Files:**
- Create: crates/brain-context/src/codegraph.rs
- Modify: crates/brain-context/src/lib.rs
- Create: crates/brain-context/tests/codegraph_contract.rs
- Create: fixtures/providers/codegraph-status.json
- Create: fixtures/providers/codegraph-search.json
- Create: fixtures/providers/codegraph-malformed.json
- Create: crates/brain-cli/src/providers.rs
- Modify: crates/brain-cli/src/main.rs
- Create: docs/operations/codegraph-provider.md

**Interfaces:**
- Consumes: exact project/worktree path, HEAD, query paths/symbols
- Produces: `CodeGraphProvider: CodeTruthProvider`

- [ ] **Step 1: Write failing freshness and worktree tests**

~~~rust
#[tokio::test]
async fn index_for_another_head_or_worktree_is_rejected() {
    let provider = FixtureCodeGraph::indexed_for(worktree_a(), head_a());
    let result = provider.retrieve(query_for(worktree_b(), head_b())).await;
    assert!(result.items.is_empty());
    assert_eq!(result.status, ProviderStatus::Stale);
}

#[tokio::test]
async fn code_hits_include_current_file_symbol_and_revision_provenance() {
    let result = fixture_provider().retrieve(code_query("OAuth callback")).await;
    assert!(result.items[0].citation.file.is_some());
    assert!(result.items[0].citation.symbol.is_some());
    assert_eq!(result.items[0].citation.git_head, Some(current_head()));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-context codegraph_contract`

Expected: FAIL because the adapter is absent.

- [ ] **Step 3: Implement released-behavior discovery and adapter**

Detect an explicitly configured executable/API and validate its version/capabilities through public commands. Invoke with an exact worktree path and a bounded result count. Parse structured output into file/symbol/range/relationship hits. If the provider lacks structured output or a reliable index-to-HEAD identity, keep it disabled and report why.

~~~rust
pub struct CodeGraphProvider {
    client: CodeGraphClient,
    indexes: WorktreeIndexRegistry,
}

#[async_trait::async_trait]
impl CodeTruthProvider for CodeGraphProvider {
    async fn freshness(&self, worktree: &WorktreeIdentity) -> Freshness;
}
~~~

- [ ] **Step 4: Add index lifecycle commands**

~~~text
brain providers codegraph status --project <id> --worktree <id>
brain providers codegraph index --project <id> --worktree <id>
brain providers codegraph disable --project <id>
~~~

Index refresh runs outside hooks and records provider/version/worktree path/HEAD/config hash. Never auto-delete indexes belonging to unknown paths.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-context codegraph_contract
cargo test -p brain-cli providers
~~~

Expected: version discovery, exact path, HEAD freshness, divergent worktrees, malformed output, timeout, disable/uninstall, and provenance tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-context crates/brain-cli fixtures/providers docs/operations/codegraph-provider.md
git commit -m "feat: add guarded worktree-specific CodeGraph provider"
~~~

### Task 3: Enforce the CodeGraph activation benchmark

**Files:**
- Create: tests/scale/codegraph_ab.rs
- Create: tests/scale/codegraph_queries.json
- Modify: crates/brain-cli/src/benchmark.rs
- Create: crates/brain-context/tests/codegraph_activation.rs
- Modify: docs/operations/codegraph-provider.md

**Interfaces:**
- Consumes: fixed code-navigation task set with/without CodeGraph
- Produces: signed/hashed A/B report and activation decision

- [ ] **Step 1: Write failing gate tests**

~~~rust
#[test]
fn provider_stays_disabled_when_token_reduction_is_below_twenty_percent() {
    let report = report_with(0.19, AccuracyDelta::Zero, sessions_to_amortize(10));
    assert_eq!(activation_decision(report), ActivationDecision::KeepDisabled);
}

#[test]
fn accuracy_regression_blocks_activation_even_with_large_token_savings() {
    let report = report_with(0.50, AccuracyDelta::Regression, sessions_to_amortize(2));
    assert_eq!(activation_decision(report), ActivationDecision::KeepDisabled);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-context codegraph_activation`

Expected: FAIL because the gate is absent.

- [ ] **Step 3: Define the representative task corpus and metrics**

Include symbol location, callers/callees, change impact, implementation tracing, test ownership, worktree-different code, deleted/renamed symbol, and ambiguous-name queries. Record targeted-read tokens, total model input tokens, answer accuracy, citation validity, cold index time, refresh time, retrieval latency, and false confidence.

- [ ] **Step 4: Implement the locked decision rule**

Activate per project only when median targeted-read token use drops at least 20%, accuracy has no regression, all citations match current files/HEAD, and cold indexing cost amortizes within 20 sessions. Store the report hash and provider version. Any later version or repository-size class change requires revalidation.

- [ ] **Step 5: Run the A/B gate**

Run: `cargo test --test codegraph_ab --release -- --ignored --nocapture`

Expected: the report records both arms and produces an explainable activation/disabled decision. A failed gate is a successful safe outcome, not a test infrastructure failure.

- [ ] **Step 6: Commit**

~~~powershell
git add tests/scale crates/brain-context crates/brain-cli docs/operations/codegraph-provider.md
git commit -m "test: gate CodeGraph activation on measured value"
~~~

### Task 4: Implement the separate-vault LLM Wiki client and cache

**Files:**
- Create: crates/brain-context/src/llm_wiki.rs
- Create: crates/brain-context/src/document_cache.rs
- Modify: crates/brain-context/src/lib.rs
- Create: crates/brain-context/tests/llm_wiki_contract.rs
- Create: fixtures/providers/llm-wiki-search.json
- Create: fixtures/providers/llm-wiki-malformed.json
- Create: crates/brain-store/src/provider_cache.rs
- Modify: crates/brain-store/src/migrations.rs
- Create: docs/operations/llm-wiki-provider.md

**Interfaces:**
- Consumes: project-scoped document query and separately owned LLM Wiki vault/API
- Produces: `LlmWikiProvider: DocumentKnowledgeProvider` and non-canonical cache entries

- [ ] **Step 1: Write failing ownership, scope, and citation tests**

~~~rust
#[tokio::test]
async fn llm_wiki_results_require_source_date_and_trust_label() {
    let result = fixture_provider().first_prompt(&query()).await;
    assert!(result.items.iter().all(|i| i.citation.uri.is_some()));
    assert!(result.items.iter().all(|i| i.citation.source_date.is_some()));
    assert!(result.items.iter().all(|i| i.trust == Trust::ExternalDocument));
}

#[test]
fn canonical_memory_paths_cannot_be_configured_as_llm_wiki_vault() {
    let result = validate_vault(brain_home().join("projects"));
    assert!(matches!(result, Err(ConfigError::ProviderOwnsCanonicalPath)));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-context llm_wiki_contract`

Expected: FAIL because the client/cache are absent.

- [ ] **Step 3: Implement capability discovery and strict mapping**

Use a configured public API/CLI capability. If installed LLM Wiki exposes no stable machine-readable search behavior, keep live search disabled while allowing a reviewed Markdown export reader. Map document ID, title, excerpt, canonical URI/path, source date, ingestion date, and relevance score. Do not import raw session transcripts into LLM Wiki.

- [ ] **Step 4: Implement a disposable project/task cache**

Cache only provider result IDs/excerpts/provenance under a provider namespace with query hash, project ID, optional task ID, source version, fetched time, and expiry. Cache loss has no effect on canonical memory. Reject cache records from another project or provider configuration hash.

- [ ] **Step 5: Run tests**

Run:

~~~powershell
cargo test -p brain-context llm_wiki_contract
cargo test -p brain-store provider_cache
~~~

Expected: separate vault, one-way ownership, malformed response, citations, dates, trust labels, cache expiry, config/version invalidation, and project isolation tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-context crates/brain-store fixtures/providers docs/operations/llm-wiki-provider.md
git commit -m "feat: add separate-vault LLM Wiki provider"
~~~

### Task 5: Implement two-stage LLM Wiki retrieval

**Files:**
- Modify: crates/brain-context/src/llm_wiki.rs
- Modify: crates/brain-context/src/compiler.rs
- Modify: crates/brain-context/src/token_budget.rs
- Modify: crates/brain-service/src/hook_handler.rs
- Create: crates/brain-service/src/prompt_state.rs
- Create: crates/brain-service/src/retrieval_trigger.rs
- Create: crates/brain-context/tests/document_dedup.rs
- Create: crates/brain-service/tests/llm_wiki_two_stage.rs
- Create: fixtures/providers/llm-wiki-session-start.json
- Create: fixtures/providers/llm-wiki-first-prompt.json

**Interfaces:**
- Consumes: SessionStart identity/task/checkpoint and first actual user prompt through a verified harness capability
- Produces: cached startup document block plus at most one live first-prompt block or MCP prefetch result

- [ ] **Step 1: Write failing two-stage and once-only tests**

~~~rust
#[tokio::test]
async fn session_start_uses_cache_and_never_waits_for_live_wiki() {
    let fixture = WikiHookFixture::with_hanging_live_provider_and_valid_cache();
    let context = fixture.session_start().await;
    assert!(context.contains("Cached project knowledge"));
    assert_eq!(fixture.live_calls(), 0);
}

#[tokio::test]
async fn only_the_first_user_prompt_runs_live_task_specific_search() {
    let fixture = WikiHookFixture::new();
    fixture.session_start().await;
    fixture.user_prompt("implement OAuth PKCE").await;
    fixture.user_prompt("continue").await;
    assert_eq!(fixture.live_queries(), vec!["implement OAuth PKCE"]);
}

#[tokio::test]
async fn document_results_stay_within_three_items_and_six_hundred_tokens() {
    let context = WikiHookFixture::oversized_results().first_prompt().await;
    assert!(context.document_item_count() <= 3);
    assert!(context.document_token_count() <= 600);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-service llm_wiki_two_stage; cargo test -p brain-context document_dedup`

Expected: FAIL because first-prompt state and budgets are absent.

- [ ] **Step 3: Implement SessionStart cached orientation**

Build the cache key from project ID, optional task ID, checkpoint version, provider config hash, and document source version. Use only unexpired cache. Render at most two startup items within the 300–600-token provider allocation and label them external/cached with age.

- [ ] **Step 4: Verify and register each harness retrieval trigger**

Represent support as `InlinePromptHook`, `McpPrefetch`, or `StartupOnly`. Claude uses its verified prompt-submit hook when its fixture contract passes. Codex and Hermes use their supported native prompt hook when verified; otherwise the SessionStart context instructs the harness adapter to invoke the idempotent `brain_context_for_prompt` MCP tool once after receiving the first prompt. Do not claim inline injection for a harness whose public hook cannot return context. Status reports the active tier per harness.

- [ ] **Step 5: Implement first-prompt live retrieval**

Track `(harness, native_session_id, project_id)` first-prompt state durably enough to survive hook/MCP retries. On the first real user prompt only, query LLM Wiki using the prompt plus compact task/title/paths—never the full transcript. Enforce 300 ms; max three results; relevance threshold; citations/dates/trust; deduplicate against canonical memory and startup cache.

- [ ] **Step 6: Add no-injection conditions**

Inject nothing when the provider is unavailable, slow, weakly relevant, stale beyond policy, cross-scoped, uncited, entirely duplicate, or would displace higher-authority live/task context. The normal 1,500-token and hard 3,000-token ceilings remain unchanged.

- [ ] **Step 7: Run tests**

Run:

~~~powershell
cargo test -p brain-service llm_wiki_two_stage
cargo test -p brain-context document_dedup context_quality
cargo test -p brain-hook --test latency -- --ignored --nocapture
~~~

Expected: cached startup, verified per-harness trigger tier, inline/MCP first-only behavior, retry idempotency, task query, timeout, relevance, dedup, citation, 3-result, 600-token, global-budget, and latency tests pass.

- [ ] **Step 8: Commit**

~~~powershell
git add crates/brain-context crates/brain-service fixtures/providers
git commit -m "feat: retrieve LLM Wiki knowledge in two guarded stages"
~~~

### Task 6: Prove provider removability and complete the optional release gate

**Files:**
- Create: tests/e2e/optional_providers.rs
- Create: tests/e2e/provider_removal.rs
- Modify: tests/e2e/fixtures.rs
- Modify: docs/operations/codegraph-provider.md
- Modify: docs/operations/llm-wiki-provider.md
- Create: docs/operations/optional-provider-runbook.md

**Interfaces:**
- Consumes: enabled, disabled, stale, unavailable, and uninstalled provider states
- Produces: optional-provider release evidence and operator procedures

- [ ] **Step 1: Write failing removal and combined-budget tests**

~~~rust
#[tokio::test]
async fn removing_both_providers_requires_no_canonical_data_migration() {
    let fixture = ProviderE2e::enabled_and_populated();
    let canonical_before = fixture.canonical_hashes();
    fixture.uninstall_optional_providers();
    let context = fixture.query("OAuth callback").await;
    assert_eq!(fixture.canonical_hashes(), canonical_before);
    assert!(context.contains_canonical_memory());
}

#[tokio::test]
async fn combined_providers_never_overrun_context_or_outrank_live_state() {
    let context = ProviderE2e::all_enabled_with_stale_docs().compile().await;
    assert!(context.token_count <= 1500);
    assert!(context.live_state_precedes_external_results());
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test --test optional_providers; cargo test --test provider_removal`

Expected: FAIL until configuration, adapters, budgets, and uninstall behavior are connected.

- [ ] **Step 3: Implement status and clean disable/remove commands**

`brain providers status` shows configuration, activation gate, worktree freshness, cache age, circuit state, last latency, and reason for exclusion. Disable only changes configuration. Remove can delete brain-owned disposable caches/index registry after listing exact paths and confirmation; it never deletes an LLM Wiki vault or unknown CodeGraph data.

- [ ] **Step 4: Run the optional provider release gate**

Run:

~~~powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --test optional_providers -- --nocapture
cargo test --test provider_removal -- --nocapture
cargo test --test codegraph_ab --release -- --ignored --nocapture
~~~

Expected: failure and removal are fail-open; canonical hashes remain unchanged; LLM Wiki obeys two-stage limits; CodeGraph activates only after its measured gate; no context exceeds its budget.

- [ ] **Step 5: Complete the runbook**

Document install/discovery, separate LLM Wiki vault, per-project opt-in, cache/index refresh, CodeGraph A/B interpretation, stale state, provider outage, safe disable, safe removal, and proof that canonical retrieval continues.

- [ ] **Step 6: Commit**

~~~powershell
git add crates tests/e2e docs/operations
git commit -m "feat: complete optional provider integrations"
~~~

## Optional-provider exit criteria

- Both providers are off by default, per-project scoped, observable, and fail-open.
- CodeGraph indexes and verifies each worktree separately and cannot activate without the 20%/accuracy/amortization gate.
- LLM Wiki remains a separate document system, contributes only cited external knowledge, and never ingests the raw session firehose.
- SessionStart performs no live LLM Wiki request; the first real prompt performs at most one 300 ms query.
- LLM Wiki contributes no more than three results and 600 tokens without displacing higher-authority context.
- Disabling or uninstalling either provider changes no canonical event, memory, task, lease, or project record.
