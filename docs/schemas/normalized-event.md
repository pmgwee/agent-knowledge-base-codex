# Normalized Event and Ledger Contract

Raw Claude, Codex, Hermes, and future-agent records normalize into one append-only event envelope. The original JSON remains attached so unknown fields and future schemas are not lost.

## Required fields

| Field | Meaning |
|---|---|
| `event_id` | Brain-generated UUIDv7 primary key |
| `project_id` | Canonical hard-scope UUID |
| `worktree_id` | Physical checkout UUID |
| `task_id` | Optional coordination task |
| `harness` | Source coding agent |
| `native_session_id` | Source session identity |
| `native_turn_id` | Source turn identity when present |
| `event_type` | Normalized dotted event name |
| `occurred_at` | Time reported by the source |
| `observed_at` | Time captured by the brain |
| `source_locator` | Native file or database locator |
| `source_offset` | Byte offset or stable source sequence |
| `source_schema` | Adapter schema fingerprint/version |
| `raw_hash` | SHA-256 of the canonical raw record |
| `idempotency_key` | SHA-256 of source identity, position, and raw hash |
| `git_head` / `git_branch` | Revision context when observed |
| `payload` | Understood normalized data |
| `raw` | Preserved source JSON |

## Initial event types

- session start, resume, compaction, and end;
- user prompts and agent responses;
- tool requests, completions, and failures;
- file reads, creates, modifications, and deletions;
- command starts/completions and test completions;
- Git commit/branch and deployment observations;
- task claims/releases/completions and authored checkpoints;
- unknown schemas and explicit capture gaps.

## Ledger rules

- One `EventLedger` instance is opened for exactly one project UUID.
- A batch containing another project UUID fails before any write.
- `idempotency_key` is unique. Replaying a committed batch inserts zero duplicates.
- Event insertion and source-cursor advancement occur in one SQLite `IMMEDIATE` transaction.
- A failed event insert rolls the batch back and leaves the previous cursor intact.
- Raw evidence is never updated through this API.

## SQLite configuration

The ledger enables foreign keys, a 1,000 ms busy timeout, `synchronous=NORMAL`, and WAL mode for file-backed databases. Schema migrations are recorded explicitly.

Indexes cover:

- `(project_id, occurred_at_ns DESC)` for bounded project timelines;
- `(native_session_id, source_offset)` for source order;
- `(event_type, occurred_at_ns DESC)` for typed evidence.

The query-plan test requires project-time lookups to use the scoped composite index.

Malformed native records go to `quarantine`. Discontinuities go to `capture_gaps`. Neither condition is allowed to masquerade as a successfully advanced source cursor.
