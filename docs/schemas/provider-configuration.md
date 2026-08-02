# Optional provider configuration

Provider configuration is project-scoped and both providers are disabled by
default. CodeGraph requires a configured executable, per-worktree indexes, a
structured capability contract, current worktree path/HEAD provenance, and a
stored passing activation report. LLM Wiki requires either a reviewed
machine-readable endpoint or a separately owned Markdown vault.

LLM Wiki's vault may not be equal to, inside, or contain `BRAIN_HOME`. Hook
retrieval is capped at 300 ms, three results, and 600 tokens. Provider cache
records are derived, expire, and are keyed by project/task/query/config/source
version. Neither provider can write canonical events, memories, tasks, leases,
or corrections.

Three consecutive failures open a five-minute circuit. Cached results may be
served only within their TTL and exact project scope. Timeouts, malformed data,
cross-project/worktree results, stale CodeGraph revisions, missing citations,
and unsupported capabilities produce a visible provider status and no injected
result. Canonical retrieval remains available.
