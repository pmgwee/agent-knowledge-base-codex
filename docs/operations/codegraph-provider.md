# CodeGraph provider

CodeGraph is an optional code-truth accelerator, not the memory store. It is
disabled until an explicitly configured executable exposes stable structured
`capabilities`, `index`, `status`, and `search` JSON behavior. Missing machine
output or index identity is a safe unsupported state.

Every task worktree receives its own index. Before each retrieval, the adapter
requires the exact canonical worktree path, current Git HEAD, provider version,
and index metadata to match. Hits outside the worktree are discarded. Accepted
citations include file, line range, optional symbol/relationship, observed
index time, and Git HEAD.

Activation is per project and provider version. A stored A/B report must show
at least 20% median targeted-read token reduction, no accuracy regression, all
citations current, and cold-index cost amortized within 20 sessions. Any
provider version or repository-size-class change requires revalidation. Index
refresh runs outside hooks; stale or unavailable indexes simply contribute no
optional context.
