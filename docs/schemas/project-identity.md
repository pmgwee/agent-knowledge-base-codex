# Project and Worktree Identity

The brain never scopes memory by a repository basename. Every registered project receives a persisted UUID, and every physical checkout receives its own worktree UUID.

## Brain home

`BRAIN_HOME` selects the central brain directory. When it is unset, the default is `%USERPROFILE%\AgentBrain`.

The project registry is `BRAIN_HOME\projects.json`. Updates use a same-directory atomic replacement and flush the completed file before publication.

## Inspection keys

Inspection keys locate an existing registration; they are not the canonical IDs exposed to events or queries.

- Git projects use the canonical result of `git rev-parse --git-common-dir` for the project key.
- Each Git worktree uses its canonical checkout root for the worktree key.
- Non-Git projects use their canonical root for both initial keys.
- Paths are separator-normalized and case-folded before SHA-256 hashing on Windows.
- Repository basenames are display data only.

Two linked Git worktrees therefore share one project UUID while retaining different worktree UUIDs. Two unrelated directories named `api` never collide.

## Registry records

Each project record stores:

- schema version;
- project UUID and inspection key;
- explicit canonical-path aliases;
- Git common directory and origin remote when available;
- registered worktrees, their UUIDs, paths, branches, and observed HEADs.

Unsupported registry schema versions are rejected instead of guessed.

## Moved projects

A moved checkout retains its project UUID only through an explicit alias operation. Automatic basename or remote-only matching is forbidden because separate clones of one remote may represent intentionally separate projects.

An alias that already belongs to another project is rejected as ambiguous. The registry never silently selects one project from conflicting matches.
