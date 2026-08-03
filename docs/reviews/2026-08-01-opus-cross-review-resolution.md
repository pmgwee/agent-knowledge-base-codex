# Opus 5 Cross-Review Resolution

**Date:** 2026-08-01  
**Scope:** Architecture and planning only; no system implementation  
**Reviewed artifact:** C:\Users\quekm\Desktop\projects\Agent-Knowledge-Base\CROSS-REVIEW-FOR-CODEX.md

## Outcome

The cross-review is useful and changes the final design in four important ways:

1. CodeGraph is no longer eligible as a default accelerator across worktrees.
2. The hook hot path is explicitly prohibited from launching Python, uv, Git Bash, an LLM, or the memory engine.
3. Agent-authored checkpoints and mechanically observed transcript evidence now have a formal conflict-resolution rule.
4. “Basic Memory-compatible layer” is replaced by an exact boundary: Basic Memory is a pinned, replaceable curated-memory projection and search service, never the canonical evidence store.

The earlier PLAN.md is superseded. Its claims that worktrees eliminate conflicts, Basic Memory capture is configuration-only, claude-mem has no Codex support, and successful orientation means zero file reads remain rejected.

## Evidence discipline

- ✅ Directly verified from current source, repository metadata, local files, or command output.
- 📄 Present in an upstream issue or vendor/user report but not independently reproduced here.
- ❓ Unresolved.

## Resolution of the eight requested items

### 1. CodeGraph and worktrees

**Verdict: challenge accepted.**

✅ CodeGraph issue [#1236](https://github.com/colbymchenry/codegraph/issues/1236) remains open. Current source includes worktree mismatch detection and warnings, but does not provide a correct shared index for several independently modified worktrees. Borrowing a parent checkout’s index can describe the wrong branch.

Locked policy:

- Git, targeted ripgrep, language tooling, tests, and direct file reads are the initial code-truth path.
- CodeGraph is disabled by default in feature worktrees.
- A primary checkout may benchmark its own CodeGraph index, but no feature worktree may query that index unless the requesting worktree state is proven identical. Current CodeGraph does not provide that proof, so cross-worktree reuse is disabled.
- Fresh worktree creation never waits for CodeGraph indexing.
- Per-worktree indexes are permitted only after an A/B test shows no accuracy regression and the indexing cost amortizes within 20 sessions.
- Shared-index support may be reconsidered when upstream provides commit/worktree-aware overlays or equivalent isolation and passes the same conformance suite.

This moves CodeGraph to an optional post-core experiment pending both retrieval value and worktree compatibility.

### 2. Windows hook latency

**Verdict: challenge accepted; mechanism changed.**

✅ Basic Memory’s current Claude and Codex SessionStart launchers use uv-run Python scripts requiring Python 3.12 or newer. This execution environment currently exposes neither uv nor bm, so reproducing their cold-start p95 would require installing dependencies. That would violate the user’s instruction to remain in architecture planning.

📄 claude-mem issues [#3449](https://github.com/thedotmack/claude-mem/issues/3449) and [#3451](https://github.com/thedotmack/claude-mem/issues/3451) report approximately 1.8 seconds per wrapper invocation and about 420 ms for a Windows login-shell PATH probe. The issue existence and titles are verified; the timing measurements were not independently reproduced.

Locked policy:

- No interpreter, package resolver, shell login, network call, LLM call, or memory-engine startup is allowed in the hook hot path.
- Hooks call a small compiled Windows shim that writes to a persistent local service over a named pipe.
- If the service is unavailable, the shim appends a minimal envelope to a local spool and exits successfully.
- Hook p95 target is at most 100 ms cold and 50 ms warm, with a 250 ms hard timeout and fail-open behavior.
- These targets are a Phase 0 measurement gate. Failure changes the mechanism before feature work continues.

### 3. Scale target

**Verdict: challenge partially accepted.**

The earlier 100,000-session / 50-million-event target was too large as the primary release gate, but not unreasonable as a stress-capacity tier.

Usage model:

- Planning rate: 10 sessions per day across projects and agents.
- Three-year primary corpus: approximately 10,950 sessions.
- Rounded primary qualification target: 12,000 sessions and 6 million events.
- Ten-times stress target: 120,000 sessions and 60 million events.

The primary target gates production readiness. The stress target verifies that partitioning, cursor-based ingestion, bounded context, and rebuildable indexes do not introduce a fixed architectural ceiling. No launch is blocked merely because the 10× test needs extended hardware time.

### 4. Build, contribute, or lift

**Verdict: hybrid decision.**

✅ Basic Memory issue [#669](https://github.com/basicmachines-co/basic-memory/issues/669) remains open and describes a transcript-watching sidecar close to this project’s capture requirement.

✅ claude-mem contains a generic file tailer and schema-driven transcript watcher. However, its event processor imports claude-mem’s worker lifecycle, session handlers, project naming, AGENTS.md writer, observation ingestion, and runtime services. Its current Codex hook adapter is small and not a complete transcript parser. claude-mem also disabled default Codex transcript watching after moving to native hooks because replay could duplicate history.

Locked decision:

- Build the small canonical adapter contract and normalization ledger locally.
- Use official agent formats first and preserve unknown raw events.
- Use claude-mem’s Apache-2.0 schemas, edge cases, and fixtures as reference material with provenance; do not import its worker/runtime.
- Reuse isolated parser code only when coupling analysis and license headers show it is cheaper than implementing the equivalent focused adapter.
- Track or contribute to #669, but do not make delivery depend on an upstream issue inactive since March 2026.
- If #669 later implements the required adapter contract and passes conformance tests, it can replace parts of capture without migrating canonical history.

### 5. Checkpoint precedence

**Verdict: challenge accepted.**

For mechanically verifiable claims, later raw evidence wins:

1. Current Git, working tree, tests, and deployment state for what is true now.
2. Timestamped transcript/tool evidence for what happened during a session.
3. Explicit human statements and corrections for intent, preference, and approval.
4. Agent-authored checkpoints for rationale, intent, and next action.
5. LLM-derived consolidation as a searchable hypothesis backed by citations.

Examples:

- A checkpoint says tests passed but the last captured test exited non-zero: the test status is failed and the checkpoint claim is marked contested.
- A checkpoint explains why OAuth was chosen: its rationale remains authoritative unless the user corrects it.
- A later successful test supersedes an earlier failure for current status, while both remain in history.

Contradictions are never silently overwritten. They create a conflict record with provenance and temporal validity.

### 6. Week-one walking skeleton

**Verdict: challenge accepted.**

The first vertical slice is one Claude Code project:

native JSONL tail → normalized append-only SQLite ledger → deterministic recent-task query → SessionStart context injection.

It contains no LLM extraction, embeddings, Basic Memory, CodeGraph, Obsidian automation, or Hermes adapter. It must prove:

- restart-safe cursor capture;
- zero duplicate canonical events after replay;
- no project leakage;
- hook p95 within the locked budget;
- a 1,500-token-or-less orientation;
- successful continuation without transcript export.

Only after this slice passes are Codex, Hermes, consolidation, and coordination added.

### 7. Exact Basic Memory boundary

**Verdict: resolved as upstream projection, not reimplementation.**

The system does not reimplement Basic Memory’s schema and does not use its internal SQLite database as canonical storage.

- Canonical raw evidence is owned by this project.
- Curated Markdown is owned by this project’s typed memory schema.
- Basic Memory v0.22.1 is pinned as an external indexing/search projection through public CLI/MCP behavior.
- Basic Memory indexes are disposable and rebuildable from curated Markdown.
- main-branch v0.23 work is not consumed until released and qualified.
- Upgrades run against a staging copy, rebuild the index, execute retrieval and cross-agent conformance tests, and then switch. Rollback restores the previous pin and rebuilds from the same Markdown.
- If Basic Memory is unavailable, SQLite FTS over curated notes provides degraded retrieval.

For this local single-user system, AGPL does not block use. Distribution or hosted-product plans require a separate license review; this document is not legal advice.

### 8. Where the cross-review over-corrected

Three qualifications are necessary:

1. A 100k-scale stress test is not inherently overengineering. It was incorrectly positioned as the primary gate, so it has been moved to the 10× tier rather than deleted.
2. claude-mem’s adapter layer is not a drop-in replacement. The reusable generic tailer is small; the semantic processor is coupled to the runtime whose failure modes are being avoided.
3. Git, test, and deployment data have two roles. Live snapshots feed the context compiler as current authority, while observed commits, test runs, and deployments also enter the immutable evidence ledger as history. The Appendix A diagram’s compiler-only edge would otherwise lose the ability to answer “what happened last week?” with evidence.

## Repository health snapshot

✅ Verified 2026-08-01 using pushedAt rather than search-result updatedAt.

| Repository | Stars | Created | pushedAt | State |
|---|---:|---|---|---|
| [basicmachines-co/basic-memory](https://github.com/basicmachines-co/basic-memory) | 3,542 | 2024-12-02 | 2026-07-30 | Active, original, AGPL-3.0 |
| [colbymchenry/codegraph](https://github.com/colbymchenry/codegraph) | 63,877 | 2026-01-18 | 2026-08-01 | Active, original, MIT |
| [thedotmack/claude-mem](https://github.com/thedotmack/claude-mem) | 89,189 | 2025-08-31 | 2026-07-31 | Active, original, Apache-2.0 |
| [NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent) | 223,448 | 2025-07-22 | 2026-08-01 | Active, original, MIT |
| [openai/codex](https://github.com/openai/codex) | 102,944 | 2025-04-13 | 2026-08-01 | Active, original, Apache-2.0 |

## Final assessment

Opus’s review improves the design and is adopted where evidence supports it. The final system keeps the original correctness model but stages it around an early vertical slice, uses realistic primary capacity targets, and treats all optional engines as replaceable projections.
