# Cross-Agent Secondary Brain Production Design

**Status:** Final design candidate, pending user review  
**Date:** 2026-08-01  
**Target environment:** One Windows machine, local Claude Code and Codex histories, with Hermes Agent and future coding agents supported through adapters  
**Implementation status:** No implementation has started

## 1. Outcome

Build one durable, cross-agent secondary brain that automatically captures work from Claude Code, Codex, and Hermes; preserves authoritative session evidence; produces compact, evidence-linked long-term memory; supplies bounded context to every new or compacted session; and coordinates concurrent work without treating memory as a substitute for Git isolation.

The design is intentionally not “LLM Wiki + Obsidian + CodeGraph.” Those products cover useful roles but not the complete problem.

The core is:

1. a central Windows brain with hard per-project namespaces;
2. an append-only evidence ledger fed by native session adapters and fail-open hooks;
3. a temporal consolidation layer that preserves provenance and supersession;
4. a bounded context compiler exposed through deterministic hooks and MCP;
5. a separate coordination plane using worktrees, task claims, and merge preflight.

Basic Memory is a replaceable curated-memory index. Obsidian is the human viewer. CodeGraph and NashSu LLM Wiki are optional sidecars that must prove value.

## 2. Goals

### 2.1 Required user outcomes

1. Concurrent sessions do not silently overwrite one another’s working files.
2. A new session can continue a prior task without manually exporting or rereading a full transcript.
3. A new session understands the latest relevant project state without scanning the complete repository.
4. Claude Code, Codex, Hermes, and future agents share the same durable memory.
5. “What did I do yesterday, last week, or last month?” returns a complete timeline with evidence.
6. Superseded decisions are not served as current truth.
7. Context remains precise and bounded as the historical corpus grows for years.
8. Capture failures are visible and recoverable; no component may silently drop history.

### 2.2 Non-goals

- Replacing Git, tests, deployments, or direct source inspection as current truth.
- Injecting the entire memory store or transcript into every session.
- Preventing all merge conflicts. Worktrees isolate working directories; merge conflicts remain possible and must be detected.
- Making CodeGraph, embeddings, a cloud LLM, LLM Wiki, or Obsidian mandatory for core correctness.
- Sharing project-specific facts across projects through semantic similarity.
- Implementing a multi-user or hosted SaaS platform in the first production version.

## 3. Locked design principles

### 3.1 Evidence before summaries

Native transcripts and mechanically observed events are retained before any LLM transforms them. A summary can be regenerated; missing raw evidence cannot.

### 3.2 Hard scope before ranking

Every query resolves one project_id before retrieval. Project filtering occurs in the database query, never as a vector-search preference. Global preferences are stored in a physically separate namespace and are never mixed with project memories.

### 3.3 Current code outranks memory

For present-state questions, authority is:

1. current working tree, Git, tests, and deployments;
2. an optional live code index whose freshness matches the requesting worktree;
3. timestamped raw session evidence;
4. curated memory and historical narrative.

### 3.4 Bounded active context, unbounded logical history

Historical storage may grow continuously. Startup context has a fixed token budget and retrieves only the relevant slice. No startup path scans all sessions, all notes, or the repository.

### 3.5 Fail open for agents, fail loud for operators

A broken brain must never prevent Claude Code, Codex, or Hermes from starting or stopping. The hook exits successfully, while health state, backlog, and data gaps become visible in diagnostics.

### 3.6 Replaceable projections

Raw evidence and curated Markdown belong to the brain. Basic Memory, FTS indexes, embeddings, CodeGraph, LLM Wiki, and Obsidian are replaceable projections or viewers.

## 4. Canonical architecture

~~~mermaid
flowchart TD
    CC["Claude Code<br/>native JSONL + hooks"] --> AD["Agent adapter contract"]
    CX["Codex<br/>rollout JSONL + hooks"] --> AD
    HA["Hermes Agent<br/>state.db + hooks"] --> AD
    FA["Future agents<br/>tiered adapters"] --> AD

    AD --> CAP["Persistent capture service<br/>cursoring, normalization, deduplication"]
    CAP --> EV["Canonical evidence<br/>append-only, hard project scope"]
    EV --> CONS["Temporal consolidation<br/>typed memories, provenance, supersession"]
    CONS --> CUR["Curated Markdown<br/>project-owned"]
    CUR --> BM["Basic Memory v0.22.1<br/>replaceable index/MCP projection"]

    LIVE["Live authority<br/>Git, working tree, tests, deployments"] --> CTX["Bounded context compiler"]
    EV --> CTX
    CUR --> CTX
    BM --> CTX
    CG["Optional CodeGraph<br/>same-worktree only after benchmark"] --> CTX
    WIKI -->|"cached startup + first-prompt retrieval"| CTX
    CTX --> INJ["Compiled orientation<br/>hooks + MCP progressive depth"]
    INJ --> CC
    INJ --> CX
    INJ --> HA
    INJ --> FA

    DOCS["External documents and research"] --> WIKI["Optional NashSu LLM Wiki<br/>separate vault and owner"]
    CUR -. "selected one-way exports only" .-> WIKI
    CUR --> OBS["Obsidian<br/>human audit and editing"]
~~~

Coordination is independent of memory:

~~~mermaid
flowchart LR
    WT["One worktree and branch<br/>per independent task"] --> CL["Task owner, renewable lease<br/>optional path claims"]
    CL --> PF["Merge preflight<br/>diff overlap, conflicts, tests"]
    PF --> MG["Human/agent-reviewed integration"]
~~~

Git, test, and deployment data have two paths:

- current snapshots feed the context compiler as live authority;
- observed commits, test runs, and deployments are appended to evidence so historical questions remain answerable.

## 5. Central Windows layout

The default brain root is %USERPROFILE%\AgentBrain and is configurable through BRAIN_HOME. Canonical data never lives inside a code repository.

~~~text
AgentBrain/
  config/
    brain.toml
    projects.json
    providers.json
  global/
    preferences/
    procedures/
  projects/
    <project_uuid>/
      identity/
        project.json
        path-aliases.json
      evidence/
        hot/
          events.sqlite
        segments/
          YYYY/
            MM/
              events-<sequence>.jsonl.zst
        blobs/
          <sha256-prefix>/<sha256>
        quarantine/
      curated/
        checkpoints/
        decisions/
        facts/
        investigations/
        procedures/
        deployments/
        timelines/
      indexes/
        fts/
        basic-memory/
      coordination/
        tasks.sqlite
      snapshots/
  runtime/
    brain.pipe
    service.lock
    spool/
    logs/
  backups/
    manifests/
~~~

### 5.1 Project identity

Each project receives a generated UUID. Identity is resolved using:

1. an existing path alias;
2. Git common-directory identity plus normalized remote URL when available;
3. a user-approved registration for repositories without a stable remote.

Repository basenames are display labels only and never scope keys. Every worktree maps to one project_id and has its own worktree_id, path, branch, and head SHA.

Renames and moved checkouts add path aliases without changing project_id. Project merges require an explicit administrative operation and audit record.

### 5.2 Global namespace

Only durable cross-project items belong in global:

- response and coding preferences;
- tool usage preferences;
- universally applicable personal procedures;
- explicitly promoted reusable learnings.

Promotion from a project requires an explicit user action or a reviewed consolidation proposal. Project code facts, credentials, deployment details, and task history never auto-promote.

## 6. Capture plane

### 6.1 Two complementary capture paths

Native transcript adapters provide completeness and crash recovery. Hooks provide lifecycle timing, immediate context injection, and low-latency coordination signals.

Neither path is sufficient alone:

- transcript watching can lag and must understand format changes;
- hooks can be skipped, crash, or contain only bounded payloads.

The normalizer deduplicates both into one canonical event stream.

### 6.2 Persistent service and hook shim

brain-service is a persistent local process. A compiled brain-hook.exe:

1. reads the hook JSON from stdin;
2. adds harness, receive time, and a nonce;
3. sends the envelope over a Windows named pipe;
4. falls back to an append-only spool if the service is unavailable;
5. exits zero.

The hook hot path must not start Python, uv, Node, Git Bash, an LLM, Basic Memory, or an indexer.

Performance contract:

- warm hook p95 at most 50 ms;
- cold hook p95 at most 100 ms;
- hard timeout 250 ms;
- no network access;
- zero agent-blocking failures.

### 6.3 Transcript adapters

Every adapter implements:

~~~text
discover_sources() -> list<SourceDescriptor>
fingerprint(source) -> SchemaFingerprint
read_increment(source, cursor) -> RawRecordBatch
normalize(raw_record, context) -> list<NormalizedEvent>
checkpoint_cursor(source, cursor) -> void
health() -> AdapterHealth
compile_injection(request) -> HookResponse
~~~

Adapters must:

- persist a cursor only after the event transaction commits;
- detect truncation, file replacement, and rotation;
- preserve an unknown event as raw evidence instead of discarding it;
- quarantine malformed records with source path, offset, hash, and error;
- report schema fingerprints that changed after an agent update;
- replay idempotently from an earlier cursor;
- never infer project identity from a basename.

### 6.4 Initial adapters

#### Claude Code

✅ This machine currently has native histories under %USERPROFILE%\.claude\projects and session JSONL containing sessionId, timestamp, cwd, gitBranch, message/tool data, and multiple event shapes.

Use the native JSONL as authoritative session evidence. Hooks supply SessionStart, PreCompact, Stop, tool lifecycle, and coordination events when enabled.

#### Codex

✅ This machine currently has rollout JSONL under %USERPROFILE%\.codex\sessions\YYYY\MM\DD. Records contain session metadata, turn context, user and agent messages, tool calls and outputs, compaction, task lifecycle, and token events.

Use rollout JSONL as authoritative evidence. Hooks provide lifecycle timing and context injection. The adapter ignores encrypted reasoning content for memory extraction and never requires chain-of-thought.

#### Hermes Agent

✅ Current Hermes source documents %USERPROFILE%\.hermes\state.db as a WAL-mode SQLite store containing full message history, FTS5, session lineage, cwd, Git branch, and repository root. Hermes also exposes lifecycle hooks and MCP. Hermes is not currently present at %USERPROFILE%\.hermes on this machine, so its adapter will first be implemented and tested against fixtures before activation.

The Hermes adapter reads committed rows incrementally by stable row/session identifiers using a read-only SQLite connection. It does not copy a live database file.

### 6.5 Future-agent capability tiers

- Tier 1: native transcript or database, hooks, and MCP/injection support.
- Tier 2: native transcript watcher plus MCP or a startup skill.
- Tier 3: launcher wrapper or polling plus a generated orientation file.

Tier affects latency and completeness reporting, not the canonical schema.

## 7. Canonical evidence model

### 7.1 Event fields

Every normalized event includes:

| Field | Purpose |
|---|---|
| event_id | UUIDv7 generated by the brain |
| project_id | hard scope key |
| worktree_id | physical checkout identity |
| task_id | nullable coordination link |
| harness | claude-code, codex, hermes, or registered adapter |
| native_session_id | original session identity |
| native_turn_id | original turn identity when available |
| event_type | typed normalized event |
| occurred_at | source event time |
| observed_at | capture time |
| source_locator | file/database plus stable source identity |
| source_offset | byte offset or row identifier |
| source_schema | adapter fingerprint/version |
| raw_hash | SHA-256 of canonicalized source record |
| idempotency_key | hash of harness, source identity, offset/row, and raw hash |
| git_head | observed commit SHA when available |
| git_branch | observed branch |
| payload | normalized JSON payload |
| raw_ref | pointer to raw record/blob |
| redaction_status | none, redacted, quarantined |

The event table rejects duplicate idempotency_key values. Raw records smaller than 64 KiB may be stored inline; larger records and binary attachments go to the content-addressed blob directory.

### 7.2 Event types

Initial normalized types:

- session.started, session.resumed, session.compacted, session.ended;
- user.prompted, agent.responded;
- tool.requested, tool.completed, tool.failed;
- file.read, file.created, file.modified, file.deleted;
- command.started, command.completed;
- test.completed;
- git.commit_observed, git.branch_changed;
- deployment.observed;
- task.claimed, task.released, task.completed;
- checkpoint.authored;
- schema.unknown and capture.gap.

Raw harness-specific payloads remain available even when no normalized type exists.

### 7.3 Storage lifecycle

- Hot events are appended transactionally to a per-project SQLite database in WAL mode.
- At 256 MiB or month end, committed ranges are sealed into compressed immutable JSONL segments with a manifest and checksum.
- SQLite retains the hot range and searchable catalog. Cold payloads can be removed from the hot database only after segment verification and backup.
- FTS, Basic Memory, and embedding indexes are derived and rebuildable.
- Corrections create new events or redaction overlays; sealed evidence is not silently rewritten.

## 8. Curated temporal memory

### 8.1 Memory types

- checkpoint: resumable task state and next action;
- decision: chosen approach, rationale, alternatives, consequences;
- fact: verified project or environment fact;
- investigation: hypothesis, evidence, failed attempts, conclusion;
- procedure: repeatable workflow;
- deployment: environment, version/SHA, result, rollback information;
- timeline: dated aggregation of events;
- preference: user behavior instruction, normally global;
- task: objective, ownership, status, blockers, worktree.

### 8.2 Required fields

Every curated memory contains:

- memory_id and project_id;
- type and status;
- title and compact body;
- valid_from and nullable valid_to;
- observed_at and consolidated_at;
- confidence;
- source event IDs;
- source sessions and agents;
- Git SHA/worktree when relevant;
- supersedes and superseded_by links;
- last_verified_at and verification method;
- content hash and schema version.

### 8.3 Precedence and contradiction

Mechanically verifiable outcomes come from raw evidence and live state. Agent-authored checkpoints supply intent and rationale. LLM consolidation never upgrades a claim beyond its sources.

When two memories disagree:

1. determine whether the claim is mechanical or interpretive;
2. compare source authority and event time;
3. preserve both records;
4. close valid_to on a superseded claim when justified;
5. create a conflict record when no safe resolution exists;
6. exclude unresolved contested claims from automatic orientation unless the conflict itself is relevant.

Human corrections take precedence for preferences, intent, and approval but do not rewrite mechanical history.

### 8.4 Consolidation

Consolidation is asynchronous and cursor-based. It processes only new evidence plus the small set of possibly affected memories.

Pipeline:

1. deterministic event grouping by session/task/time;
2. secret and prompt-injection screening;
3. extraction through a provider-neutral LLM interface when enabled;
4. evidence validation against raw events;
5. deduplication and supersession analysis;
6. write proposed memory;
7. index and publish after validation.

GLM may be the initial LLM provider. No cloud LLM or embedding service is required for the walking skeleton. If the provider is unavailable, evidence capture continues and consolidation backlog grows visibly.

## 9. Basic Memory and Obsidian boundary

Basic Memory v0.22.1 is pinned as a separate process or CLI/MCP integration. The unreleased main branch is not consumed.

The brain:

- owns curated Markdown;
- writes through documented public behavior;
- never reads or modifies Basic Memory’s internal database;
- treats its indexes as disposable;
- maintains SQLite FTS as a degraded fallback.

Upgrade procedure:

1. copy curated Markdown and configuration into a staging brain project;
2. install the candidate exact version and artifact hash;
3. rebuild its index;
4. run retrieval, temporal, project-isolation, and token-budget conformance tests;
5. back up the current index/config;
6. switch the version pin;
7. roll back by restoring the prior pin and rebuilding if any gate fails.

Obsidian opens the curated Markdown tree for human inspection and controlled editing. Human edits pass through a watcher that validates schema and creates an audit event; generated indexes are not hand-edited.

## 10. Context compiler

### 10.1 Deterministic startup orientation

SessionStart and post-compaction injection compile:

1. project and worktree identity;
2. active task, owner, branch, and lease;
3. live Git SHA and dirty-state summary;
4. drift since the latest checkpoint;
5. latest relevant checkpoint and next action;
6. accepted decisions relevant to the task;
7. active conflicts, path claims, and blockers;
8. cached LLM Wiki document knowledge related to the active task when configured;
9. evidence/MCP identifiers for deeper retrieval.

Normal target is 1,000–1,500 tokens. The hard maximum is 3,000 tokens. If the candidate set exceeds the budget, lower-authority and older items are omitted before any item is truncated into ambiguity.

### 10.2 Progressive retrieval

Agents retrieve in levels:

short orientation → scoped timeline → selected memory → exact raw evidence.

MCP supplies optional depth. Deterministic hooks supply the minimum ambient state even when the model never calls MCP.

### 10.3 Query policy

Every query:

1. resolves project_id and optional worktree/task;
2. identifies present-state versus historical intent;
3. applies hard scope and temporal filters;
4. searches FTS and typed metadata;
5. optionally uses semantic retrieval only within the scoped candidates;
6. reranks by authority, temporal validity, task relation, and recency;
7. compiles within the caller’s token budget;
8. cites memory IDs and raw evidence IDs;
9. abstains when coverage is insufficient.

Embeddings are introduced only if a measured recall suite shows that FTS plus metadata cannot meet the target.

### 10.4 Two-stage LLM Wiki retrieval

LLM Wiki is a first-class optional DocumentKnowledgeProvider, not a memory store. It participates in context compilation only when a separate vault is configured.

Stage one occurs at SessionStart. The compiler uses project_id, the active task, and the latest checkpoint to select a cached LLM Wiki digest. It does not perform an uncached document search in the hook shim.

Stage two occurs on the first UserPromptSubmit of a session. The persistent service searches LLM Wiki using the actual prompt plus project/task metadata, hard-scopes results to the configured document collection, and returns at most three results. Results consume 300–600 tokens inside the existing 1,500-token normal orientation budget. A provider deadline of 300 ms applies; timeout, weak relevance, or unavailability produces no injection and never blocks the core context.

Every result includes its source document, update date, and a trust label stating that document knowledge must be verified against current source before code changes. Results are deduplicated against canonical memory. Later prompts use cached results unless the task topic changes materially or the agent explicitly requests deeper MCP retrieval.

## 11. Coordination plane

### 11.1 Structural isolation

- One worktree and branch per independent task.
- One active writer lease per worktree.
- Multiple sessions may inspect one worktree, but only the lease owner may write.
- Worktrees convert working-directory corruption into reviewable branch integration; they do not eliminate merge conflicts.

### 11.2 Task claims

Each active task records:

- task_id, project_id, worktree_id, branch;
- objective and acceptance criteria;
- owning agent/session;
- status and blocker;
- optional path globs;
- acquired_at, renewed_at, and expires_at.

Default lease is 30 minutes and renews every 5 minutes while the session is active. An expired lease becomes reclaimable, but its abandoned worktree remains intact.

Overlapping path claims warn before work starts. They are advisory because two logically conflicting changes may touch different files.

### 11.3 Merge preflight

Before integration:

1. update the target branch reference;
2. detect textual conflicts without mutating the worktree;
3. compare changed paths with active tasks;
4. run the task’s required tests;
5. surface semantic risks such as migrations, lockfiles, schemas, and shared configuration;
6. require a reviewed merge result.

No automated force resolution is permitted.

## 12. Optional components

### 12.1 CodeGraph

✅ Issue [#1236](https://github.com/colbymchenry/codegraph/issues/1236) remains open. CodeGraph is worktree-aware enough to warn about index mismatches, but cannot provide one correct shared index across divergent worktrees.

Policy:

- disabled in the walking skeleton and feature worktrees;
- implemented behind the CodeTruthProvider interface and a per-project feature flag;
- activated only after an optional benchmark in the primary checkout;
- never blocks SessionStart for indexing;
- kept only if it reduces targeted-read tokens by at least 20%, causes no answer-accuracy regression, and amortizes its cold index cost within 20 sessions;
- no query is trusted when index freshness does not match the requesting checkout.

### 12.2 NashSu LLM Wiki

NashSu LLM Wiki is an optional document-knowledge sidecar and DocumentKnowledgeProvider for specifications, external documentation, articles, PDFs, and selected finalized summaries. It is not part of session capture, coordination, or canonical evidence.

It uses a separate vault with a single generated-content owner. Selected exports from curated memory are one-way. LLM Wiki never writes back into evidence, and Basic Memory and LLM Wiki never own the same generated Markdown directory. Automatic retrieval follows the two-stage policy in section 10.4 and remains feature-flagged, bounded, cited, deduplicated, and fail-open.

### 12.3 Embeddings and temporal graph engines

No embedding service or Graphiti-style graph engine is in the initial core. Add one only after measured failures demonstrate that typed metadata, temporal fields, FTS, and evidence links are insufficient.

## 13. Failure handling

| Failure | Required behavior |
|---|---|
| brain-service unavailable | Hook spools minimal envelope and exits zero; watcher catches up |
| hook never fires | Native transcript watcher captures the session |
| transcript malformed | Preserve raw record in quarantine; do not advance past an unaccounted gap |
| agent format changes | Raise schema-fingerprint health failure; retain unknown events |
| duplicate replay | Unique idempotency key prevents duplicate canonical events |
| LLM/provider unavailable | Capture continues; consolidation backlog remains queryable |
| Basic Memory unavailable | Context compiler uses curated Markdown plus SQLite FTS |
| CodeGraph unavailable/stale | Use Git, ripgrep, language tooling, and direct reads |
| disk pressure | Stop optional indexing and consolidation first; preserve capture and alert |
| project resolution ambiguous | Quarantine until explicitly mapped; never guess by basename |
| conflicting memory | Mark contested, preserve provenance, omit unsafe claim from orientation |
| service restart | Resume from committed cursors and replay the spool idempotently |

## 14. Security and trust

Although the user does not currently require strong privacy, coding sessions can contain credentials and untrusted text.

- credential files and known secret stores are excluded;
- common token/key patterns are redacted before curated memory;
- raw evidence records redaction status;
- recovered text is data, never executable instructions;
- memory cannot override repository rules, current user requests, or agent safety policy;
- tool commands from old transcripts are never replayed automatically;
- every external-content ingest is labeled by source and trust class.

## 15. Scalability and service objectives

### 15.1 Capacity model

- Expected rate: 10 sessions/day.
- Primary production qualification: 12,000 sessions and 6 million events, representing about three years plus margin.
- Stress qualification: 120,000 sessions and 60 million events.
- Logical history has no fixed retention ceiling; active indexes and injected context remain bounded.

### 15.2 Performance objectives

| Operation | Objective |
|---|---|
| Hook enqueue | p95 ≤50 ms warm, ≤100 ms cold, 250 ms hard timeout |
| Normal capture lag | p95 ≤2 seconds after transcript flush |
| Recovery after ordinary restart | spool replay begins within 5 seconds |
| Startup orientation | p95 ≤1.5 seconds normal, ≤3 seconds degraded |
| Normal orientation size | ≤1,500 tokens |
| Hard orientation size | ≤3,000 tokens |
| Project leakage | zero in the isolation suite |
| Duplicate canonical events | zero after replay/restart tests |
| Evidence citation | every factual historical answer cites at least one event or memory source |

Scoped retrieval at the primary corpus must hold warm p95 within 25 ms and cold, uncached p95 within one second. Stress qualification may take longer to build, but scoped query latency and context size must remain within two times the primary target.

> **Superseded 2026-08-03.** This criterion originally read: *"Performance at the primary corpus may degrade by no more than 20% relative to a 1,000-session corpus for the same scoped query."* The bounded scoped-query cache introduced in `e10cd59` made warm retrieval tens of microseconds, at which point the ratio compared two noise-dominated numbers — five identical smoke runs measured 0%, 2.8%, 33.6%, 0% and 0% degradation, failing one of them, and the same coin-flip sat inside the twelve-hour stress gate. The relative threshold is retained in the report as a diagnostic and replaced as a gate by the absolute ceilings above. The cold ceiling preserves the original intent: it is measured against a freshly opened ledger, so the cache cannot mask retrieval scaling with history.

### 15.3 Data lifecycle

- hot: active month plus active tasks in SQLite;
- warm: recent twelve months with FTS and curated memory;
- cold: sealed compressed segments with catalog metadata;
- indexes are rebuilt incrementally from manifests and cursors;
- no query scans cold segments unless the user explicitly requests deep historical evidence.

## 16. Backup, restore, and upgrades

- Hourly SQLite online backup for active project databases.
- Daily backup of config, curated Markdown, sealed manifests, and new segments.
- Retain 30 daily and 12 monthly recovery points.
- Monthly automated restore into an isolated path followed by checksum, schema, and query smoke tests.
- Recovery point objective: at most one hour for brain-owned data.
- Recovery time objective: at most two hours for a primary-size corpus; native transcripts can replay any missing captured events.
- Schema migrations are versioned, transactional, resumable, and tested on a restored copy before production.
- Adapter compatibility is pinned by supported agent version and schema fingerprint, not assumed indefinitely.

## 17. Evaluation and acceptance

### 17.1 Core scenarios

1. Claude session continues in a fresh Claude session without transcript export.
2. Claude work continues in Codex and then Hermes with the same task context.
3. A post-compaction continuation receives the same active task and checkpoint.
4. A session crashes before Stop and is recovered from its transcript.
5. Duplicate hook/transcript events create one canonical event.
6. Two repositories with the same basename never share results.
7. A moved repository retains project identity.
8. A worktree task sees its branch state, not the primary checkout’s state.
9. An earlier passing test followed by a failing test is reported as currently failing.
10. A reversed decision returns only the current decision by default and preserves history.
11. “What did I do last week?” matches Git/test/deployment evidence.
12. A false-premise historical question causes abstention.
13. Basic Memory outage falls back without losing evidence.
14. LLM outage accumulates consolidation backlog without affecting capture.
15. Agent format drift creates an alert and preserves unknown records.
16. Overlapping worktree/path claims warn before edits.
17. Merge preflight detects textual conflict and test failure.
18. Startup orientation remains within token limits at primary and stress corpus sizes.

### 17.2 Quality gates

- Capture completeness: at least 99.9% of parseable native records, with every gap explicitly reported.
- Current-state correctness: no memory claim may override contradictory live Git/test/deployment evidence.
- Historical precision: at least 95% of benchmark assertions supported by correct cited evidence.
- Temporal correctness: 100% of supersession fixtures exclude invalidated claims from default retrieval.
- Project isolation: 100% pass.
- Token reduction: new-session orientation uses at least 80% fewer tokens than loading the equivalent exported transcript on the benchmark tasks.

## 18. Staged delivery

### Phase 0: Measurement and contract fixtures

Define project identity, event schema, adapter fixtures, hook benchmark harness, token counter, and evaluation corpus. Measure the compiled hook mechanism before feature work.

Exit: hook latency contract passes; Claude and Codex fixture parsers preserve unknown records and replay idempotently.

### Phase 1: One-project Claude walking skeleton

Claude JSONL watcher → normalized append-only SQLite → deterministic recent-task query → SessionStart injection.

Exit: crash recovery, zero duplicates, project isolation, ≤1,500-token orientation, and fresh-session continuation pass without LLMs or Basic Memory.

### Phase 2: Codex and Hermes adapters

Add Codex rollout parsing and Hermes read-only state.db ingestion. All three agents query the same project ledger.

Exit: cross-agent handoff and time-window history pass in both directions.

### Phase 3: Temporal consolidation and curated Markdown

Add typed memories, provenance, supersession, GLM provider adapter, validation, and SQLite FTS. Add Basic Memory v0.22.1 and Obsidian as projections.

Exit: decision reversal, checkpoint conflict, provider outage, and rebuild tests pass.

### Phase 4: Context compiler and MCP depth

Add task-aware retrieval, Git drift verification, strict token budgets, citations, abstention, and progressive MCP tools.

Exit: startup and query SLOs pass at the primary corpus.

### Phase 5: Coordination

Add worktree registration, writer leases, path claims, overlap warnings, and merge preflight.

Exit: concurrent-write and integration suites pass without claiming merge conflicts are impossible.

### Phase 6: Scale and operational hardening

Add segment sealing, hot/warm/cold lifecycle, backup/restore, format-drift monitoring, corruption recovery, disk-pressure behavior, primary and 10× stress tests.

Exit: production SLOs, RPO/RTO, and stress invariants pass.

### Phase 7: First-class optional provider integrations

Implement the guarded CodeGraph CodeTruthProvider and the two-stage LLM Wiki DocumentKnowledgeProvider. Benchmark CodeGraph in a primary checkout and validate LLM Wiki relevance, provenance, latency, and token budgets. Evaluate embeddings and temporal graph engines only after these providers and the core retrieval suite establish a baseline.

Exit: both provider adapters pass fail-open and isolation tests. CodeGraph activation still requires its predeclared quality/cost gate; LLM Wiki activation requires a configured separate vault and passes the first-prompt relevance suite. Failure leaves the core unchanged.

## 19. Research status

✅ Verified 2026-08-01:

| Repository | Stars | Created | pushedAt | State |
|---|---:|---|---|---|
| [basicmachines-co/basic-memory](https://github.com/basicmachines-co/basic-memory) | 3,542 | 2024-12-02 | 2026-07-30 | Active, original, AGPL-3.0 |
| [colbymchenry/codegraph](https://github.com/colbymchenry/codegraph) | 63,877 | 2026-01-18 | 2026-08-01 | Active, original, MIT |
| [thedotmack/claude-mem](https://github.com/thedotmack/claude-mem) | 89,189 | 2025-08-31 | 2026-07-31 | Active, original, Apache-2.0 |
| [NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent) | 223,448 | 2025-07-22 | 2026-08-01 | Active, original, MIT |
| [openai/codex](https://github.com/openai/codex) | 102,944 | 2025-04-13 | 2026-08-01 | Active, original, Apache-2.0 |
| [nashsu/llm_wiki](https://github.com/nashsu/llm_wiki) | 15,684 | 2026-04-08 | 2026-07-27 | Active, original |

✅ Basic Memory sidecar issue #669 and CodeGraph worktree issue #1236 are open.  
✅ Basic Memory latest released application version is v0.22.1 from 2026-06-13; newer lifecycle work exists on main.  
✅ Local Claude and Codex transcript surfaces were inspected by schema/key only; conversation contents and credentials were not exported.  
📄 claude-mem issue performance numbers are user reports, not reproduced benchmarks.  
❓ Hermes is not currently installed at the default Windows path on this machine; its activation path remains fixture-tested until its actual location is registered.

## 20. Final locked decisions

1. Central brain outside repositories with physical per-project namespaces.
2. Native transcripts plus lightweight fail-open hooks.
3. Append-only canonical evidence with cursor-based replay.
4. Typed temporal memories with evidence links and supersession.
5. Git/test/deployment live state outranks memory.
6. Deterministic ≤1,500-token normal startup context and ≤3,000-token hard maximum.
7. Claude Code, Codex, and Hermes are the first supported adapters.
8. Basic Memory v0.22.1 is a replaceable projection, not the canonical store.
9. Obsidian is the human viewer.
10. Worktrees, leases, claims, and merge preflight form a separate coordination plane.
11. CodeGraph is implemented as a guarded CodeTruthProvider but remains runtime-optional and disabled across feature worktrees until compatibility and value are proven.
12. NashSu LLM Wiki is implemented as a runtime-optional, separate-vault DocumentKnowledgeProvider using cached SessionStart context and first-prompt retrieval.
13. FTS and metadata precede embeddings; measured retrieval gaps justify later complexity.
14. Three-year primary and 10× stress capacity tiers replace a single oversized launch gate.
15. The week-one Claude walking skeleton must pass before horizontal expansion.
