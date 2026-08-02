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

```powershell
brain providers configure-codegraph --project <id-or-path> --executable C:\path\to\codegraph.exe --activation-report C:\path\to\report.json
brain providers index-codegraph --project <id-or-path> --task <task-id>
brain providers status --project <id-or-path>
brain providers disable --project <id-or-path> codegraph
brain providers remove --project <id-or-path> codegraph
```

`configure-codegraph` refuses a report that misses any locked activation gate
or names a different provider version. `remove` clears only AgentBrain's
configuration and cache. An external index remains provider-owned and is never
deleted implicitly.
