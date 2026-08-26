# MCP and shared query operations

`brain-mcp` is the agent-neutral, newline-delimited stdio MCP server for the
secondary brain. It implements MCP protocol revision `2025-06-18`. Every tool
request requires a registered project UUID or an exact registered project path
alias; there is no implicit "current" project when an agent calls MCP.

## Start the server

~~~powershell
$env:BRAIN_HOME = "$env:USERPROFILE\AgentBrain"
brain-mcp
~~~

An MCP client should launch the binary directly and exchange one JSON-RPC message per line over
stdin/stdout. Logs, if added, must go to stderr.

For Claude Code, use its supported user-scope interface rather than hand-editing configuration:

~~~powershell
brain --brain-home "$env:USERPROFILE\AgentBrain" install-mcp --harness claude
claude mcp get brain
~~~

The installer is an explicit operator action. It pins `AgentBrain\bin\brain-mcp.exe`, refuses
`target\release` and drifted deployments, and is idempotent. `uninstall-mcp --harness claude` removes only a
server named `brain` whose command still matches Agent Brain. Codex uses its user-level
`[mcp_servers.brain]` setting. Both harnesses receive bounded hook pushes; MCP is historical depth
on demand.

## Tools

- `brain_search`: ranked SQLite FTS search over canonical events and temporal
  memories.
- `brain_timeline`: last day, week, month, or an explicit RFC3339 interval.
- `brain_checkpoint`: the bounded, evidence-cited orientation used at session
  start.
- `brain_evidence`: bounded expansion of one cited event or memory version.
- `brain_correct`: append-only, human-authority correction. Callers must provide
  a stable `correction_id` UUID so retries are idempotent.
- `brain_status`: canonical store health and optional-provider independence.
- `brain_claim`, `brain_claims`, `brain_release_claim`: project-scoped coordination claims.
- `brain_lease_acquire`, `brain_lease_renew`, `brain_lease_release`, `brain_lease_handoff`, and
  `brain_leases`: one-writer task coordination.
- `brain_merge_preflight`: verify integration state before merging task-owned work.
- `brain_context_for_prompt`: bounded prompt context for explicit clients.

Read tools return `structuredContent` plus the same JSON serialized in a text
content block for older clients. Results state whether an item is current
memory, as-of memory, or historical evidence. Each item includes at least one
citation, occurrence and observation times, late-observation state, and visible
ranking reasons.

`brain_correct` never edits or deletes an old memory. It appends a correction
audit event and a new memory version that supersedes the prior version. A
correction must always be reviewed by the user before an agent invokes it.

## CLI parity

The CLI and MCP server call the same `BrainQueryService` boundary:

~~~powershell
brain query --project <uuid-or-path> "OAuth callback" --limit 20
brain timeline --project <uuid-or-path> --window week
brain checkpoint --project <uuid-or-path> --prompt "continue OAuth work"
~~~

All three commands emit structured JSON. Timestamps accepted by query commands
are RFC3339 values such as `2026-08-02T12:00:00Z`.

## Failure behavior

Canonical retrieval is SQLite FTS5 and does not depend on the consolidation
provider, Basic Memory, Obsidian, CodeGraph, or LLM Wiki. If any optional process is unavailable, MCP
search, timeline, checkpoint, evidence, correction, and status continue from the
per-project ledger. `optional_provider_state: canonical_fts_only` is explicit in
retrieval results.

Every call records request plus success/failure telemetry. Claude supplies exact session
correlation when its client exposes it; otherwise the receipt is explicitly `unattributed` rather
than guessed.

An unknown project, malformed timestamp, cross-project evidence reference, or
invalid correction is returned as an MCP tool execution error (`isError: true`)
without widening project scope. Unknown tool names remain JSON-RPC invalid-params
errors.
