# Multi-agent capture and handoff

The service configuration is a versioned central registry. Each registered
project has its own immutable project ID, worktree ID, evidence ledger, Claude
transcripts, Codex rollouts, and optional Hermes binding. Registering another
project adds or updates that namespace; it does not replace existing projects.

```powershell
brain register C:\path\to\project
brain status --project <PROJECT_UUID>
```

Registration discovers Claude transcripts and Codex rollouts only when a
recorded `cwd` resolves inside the registered project. It never infers a project
from a provider folder name or repository basename. If the installed Hermes
database exists and matches the reviewed v22 fingerprint, the service reads it
in WAL-safe read-only mode and filters sessions by canonical project path.

The running service creates one normalized capture binding for every configured
source. The same global Hermes database can be bound to several project ledgers;
its source cursor, health, and quarantine are keyed by `(project_id, source_id)`.
Native evidence is never copied between project ledgers.

Claude and Codex SessionStart hooks enter the same project router. The router
canonicalizes `cwd`, chooses the most-specific registered root, and compiles at
most 1,500 tokens from that project's ledger. The orientation includes the
latest task, agent outcome, compaction/checkpoint, recent tool or test state,
file activity, and observed revision when those facts exist.

The release gates prove:

- Claude, Codex, and Hermes evidence is present in one project ledger.
- Codex can resume work last updated in Hermes without transcript export.
- a Codex compaction checkpoint survives a fresh session.
- identical adversarial sentinel text from Project B never enters Project A's
  ledger or startup context.
- one physical Hermes database remains isolated across two project namespaces.

If multiple projects are configured, operator commands require
`--project <PROJECT_UUID>` to prevent accidental ambiguity.
