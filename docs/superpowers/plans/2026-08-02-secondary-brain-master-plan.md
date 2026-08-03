# Cross-Agent Secondary Brain Master Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Deliver a production-grade Windows secondary brain shared by Claude Code, Codex, Hermes Agent, and future adapters, including reliable capture, temporal memory, bounded context, concurrency coordination, and guarded CodeGraph/LLM Wiki providers.

**Architecture:** A Rust workspace produces a persistent Windows service, a small compiled hook shim, a CLI, and an MCP server. Native transcripts and hooks enter a per-project append-only SQLite ledger; curated Markdown and indexes are derived. Coordination and optional code/document providers remain isolated behind contracts.

**Tech Stack:** Rust 1.88.0 (MSVC, edition 2024), Tokio, Serde, rusqlite with bundled SQLite, Notify plus polling reconciliation, Clap, Reqwest, Zstd, SHA-256, Windows named pipes, Markdown, Basic Memory v0.22.1 as an external projection.

## Global Constraints

- Target Windows 11; paths use Windows-safe canonicalization and never rely on WSL.
- Default BRAIN_HOME is %USERPROFILE%\AgentBrain and can be overridden by one environment variable.
- Canonical scope key is a generated project UUID; repository basenames are display-only.
- Project memory and the small global-preferences namespace are physically and logically separate. Only explicit user promotion can move a preference into global scope.
- Every query applies project_id before text, semantic, or graph ranking.
- Capture-only hooks must meet p95 ≤50 ms warm, ≤100 ms cold, and a 250 ms hard timeout.
- Hook capture is fail-open and performs no network, LLM, Python, uv, Node, Git Bash, Basic Memory, or indexer startup.
- Normal SessionStart context is ≤1,500 tokens; hard maximum is ≤3,000 tokens.
- LLM Wiki contributes 300–600 tokens and at most three cited results within the existing context budget.
- Current Git, working tree, tests, and deployments outrank memory.
- Live-state acquisition is deterministic and local: current Git is inspected directly, while test/deployment status is labeled with its last observed revision and time.
- Unknown transcript events are retained; parse failures never silently advance a cursor.
- Raw evidence is append-only; indexes and Basic Memory state are rebuildable.
- Basic Memory is pinned to released v0.22.1 and accessed only through public behavior.
- CodeGraph and LLM Wiki providers are implemented but runtime-optional and fail-open.
- No CodeGraph index is shared across divergent worktrees.
- Use TDD for every behavior and commit each independently testable task.

---

## Plan package and execution order

Execute these plans in order. Each plan ends with a working, testable increment.

1. [Foundation and Claude walking skeleton](./2026-08-02-foundation-walking-skeleton.md)
2. [Multi-agent capture adapters](./2026-08-02-multi-agent-capture.md)
3. [Temporal memory and context compiler](./2026-08-02-temporal-memory-context.md)
4. [Worktree coordination](./2026-08-02-worktree-coordination.md)
5. [Operations, backup, and scale](./2026-08-02-operations-scale.md)
6. [CodeGraph and LLM Wiki providers](./2026-08-02-optional-knowledge-providers.md)

Do not begin a later plan until the previous plan’s exit suite passes.

## Locked workspace structure

~~~text
Cargo.toml
Cargo.lock
rust-toolchain.toml
crates/
  brain-domain/       # IDs, events, project identity, shared contracts
  brain-store/        # SQLite ledger, migrations, cursors, FTS, segments
  brain-adapters/     # Claude, Codex, Hermes source adapters
  brain-context/      # retrieval, precedence, token budgeting, providers
  brain-coordination/ # tasks, leases, claims, worktrees, merge preflight
  brain-service/      # persistent capture/retrieval service and named pipe
  brain-hook/         # tiny compiled hook process
  brain-cli/          # registration, status, query, backup, diagnostics
  brain-mcp/          # stdio MCP server over brain-service
fixtures/
  claude/
  codex/
  hermes/
  providers/
tests/
  e2e/
  scale/
docs/
  operations/
  schemas/
~~~

## Stable cross-plan interfaces

The first plan defines these types; later plans extend implementations without renaming them.

~~~rust
pub trait SourceAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn discover(&self) -> anyhow::Result<Vec<SourceDescriptor>>;
    fn fingerprint(&self, source: &SourceDescriptor) -> anyhow::Result<SchemaFingerprint>;
    fn read_increment(
        &self,
        source: &SourceDescriptor,
        cursor: &SourceCursor,
    ) -> anyhow::Result<ReadOutcome>;
    fn normalize(
        &self,
        record: &RawRecord,
        context: &NormalizeContext,
    ) -> anyhow::Result<Vec<NormalizedEvent>>;
}

pub trait ContextProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn retrieve(&self, query: &ContextQuery) -> ProviderResult;
}

pub trait CodeTruthProvider: ContextProvider {
    async fn freshness(&self, worktree: &WorktreeIdentity) -> Freshness;
}

pub trait DocumentKnowledgeProvider: ContextProvider {
    async fn cached_startup(&self, query: &ContextQuery) -> ProviderResult;
    async fn first_prompt(&self, query: &ContextQuery) -> ProviderResult;
}
~~~

## Cross-plan release gates

| Gate | Required result |
|---|---|
| Foundation | Claude crash/replay produces zero duplicates and ≤1,500-token continuation |
| Multi-agent | Claude ↔ Codex ↔ Hermes handoff succeeds with hard project isolation |
| Temporal memory | Supersession, evidence precedence, citations, Basic Memory rebuild, and provider outage pass |
| Coordination | One-writer lease, overlap warning, and non-mutating merge preflight pass |
| Operations | Hourly backup, isolated restore, schema drift, disk pressure, 12k/6m primary corpus pass |
| Optional providers | CodeGraph and LLM Wiki fail-open; activation gates enforce correctness and token limits |

## Implementation completion definition

The overall goal is complete only when:

- every child-plan checkbox is complete;
- all unit, integration, end-to-end, property, and scale tests pass;
- the Windows service and hook installer work from a clean machine;
- Claude, Codex, and Hermes can share one registered project;
- a new session continues prior work without transcript export;
- last-week recall is evidence-cited and temporally correct;
- concurrent tasks use separate worktrees and visible merge preflight;
- optional providers can be enabled or removed without migrating canonical data;
- restore has been exercised from an actual backup;
- operational documentation matches the shipped CLI.
