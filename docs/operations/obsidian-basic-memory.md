# Obsidian and Basic Memory operations

SQLite evidence and versioned memory remain canonical. Obsidian is a viewer and
controlled human-edit surface. Basic Memory v0.22.1 is an optional, disposable
search/MCP index over a generated Markdown view.

## Vault ownership

Open `<BRAIN_HOME>/vault` as the Obsidian vault.

| Path | Owner | Editing rule |
|---|---|---|
| `projects/<project-id>/generated/generations/` | brain service | Never edit |
| `projects/<project-id>/generated/current.json` | brain service | Never edit |
| `projects/<project-id>/notes/` | user | Controlled project corrections |
| `projects/<project-id>/basic-memory/` | brain service | Disposable; never edit |
| `global-preferences/notes/` | user | Explicit global preferences only |

Generated memory uses Basic Memory-compatible front matter, observations, and
evidence links. A content-addressed generation is written and verified before
`current.json` is atomically replaced. User notes are outside every generated
directory and are never removed by projection rebuilds.

## Project correction note

```markdown
---
title: "Storage correction"
project_id: "<project-uuid>"
kind: decision
memory_id: "<existing-memory-uuid>" # omit for a new logical memory
evidence_ids: ["<event-uuid>"]      # may be empty; the audited note event is cited
valid_from: "2026-08-02T08:00:00Z"  # optional; defaults to import time
---

SQLite is the canonical store.
```

Allowed project kinds are `checkpoint`, `decision`, `fact`, `investigation`,
`procedure`, `deployment`, `timeline`, and `task`. A project note cannot create
a preference or promote globally. Invalid scope, evidence, kind, or front
matter enters the durable review queue without creating memory.

## Explicit global preference

Place this only under `vault/global-preferences/notes/`:

```markdown
---
title: "Response style"
kind: preference
promote_global: true
preference_id: "<existing-preference-uuid>" # optional
---

Prefer concise implementation updates.
```

`promote_global: true` is mandatory. A project ID and project evidence are
forbidden. Each accepted promotion creates a durable audit record.

## Rebuild and verify

```powershell
brain rebuild markdown --project <project-uuid>
brain verify projections --project <project-uuid>
brain rebuild basic-memory --project <project-uuid>
```

The service also detects new memory versions and refreshes Markdown in the
background. Capture, SQLite FTS, and startup context continue if projection or
Basic Memory fails.

## Optional Basic Memory v0.22.1

The integration uses only documented public CLI behavior: `project add` and
`reindex`. It never opens, copies, deletes, or migrates Basic Memory's internal
database.

```powershell
uv tool install "basic-memory==0.22.1"
basic-memory --version
```

Disable Basic Memory automatic updates in its public configuration to preserve
the tested pin:

```json
{ "auto_update": false }
```

Each project is registered as `agent-brain-<project-uuid-without-dashes>` and
points to the stable brain-owned `basic-memory/` view. That view is rebuilt
from the active verified generation before `reindex`. It is safe to delete and
rebuild; deleting it never deletes SQLite evidence, memory versions, generated
Markdown, or user notes.

Official v0.22.1 reference:
<https://github.com/basicmachines-co/basic-memory/blob/v0.22.1/README.md>
