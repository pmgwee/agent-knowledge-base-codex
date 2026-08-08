# Secondary brain

A persistent, local memory layer shared by **Claude Code** and **Codex**.

Every session either agent runs on this machine is captured to an append-only SQLite ledger,
distilled by an LLM into memories that each cite the events they came from, searched by four fused
retrieval channels, and projected to Markdown you can open in Obsidian. At the start of any session
it hands the agent a bounded, evidence-cited orientation — so continuing prior work does not mean
re-reading the codebase or re-uploading a transcript.

Nothing leaves the machine except the consolidation call.

**Start here:** [docs/status.md](docs/status.md) for what is running · [docs/roadmap.md](docs/roadmap.md)
for what is left · [docs/architecture.html](docs/architecture.html) for how it fits together.

---

## The four binaries

Rust 1.88.0 (MSVC, edition 2024), a 10-crate Cargo workspace.

| Binary | Role |
|---|---|
| `brain.exe` | CLI — register, status, query, dashboard, export, verify, forget, lint, evict, service install |
| `brain-service.exe` | Background capture, consolidation, projection, rediscovery. Runs as a Task Scheduler task |
| `brain-hook.exe` | Session-start and session-end hook. Pushes orientation over a named pipe (~9–12 ms) |
| `brain-mcp.exe` | MCP stdio server for Codex. Pull-based: `brain_checkpoint`, `brain_search`, and four others |

## Build and test

```bash
cargo build --workspace --release
```

```bash
cargo test --workspace
```

Release gates — both must be clean:

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo fmt --all -- --check
```

Scale gates are `#[ignore]` by default; `--ignored` runs them. The stress gate takes hours.

## Deploying

**Committing is deploying.** A post-commit hook builds and installs to `~/AgentBrain/bin/`, then
restarts the service. It is fail-safe: the build completes before anything is replaced, so a commit
that does not compile leaves the previous deployment live.

The binaries that actually run are the copies in `~/AgentBrain/bin/`, **not** `target/release/`.
Building is not shipping.

```bash
~/AgentBrain/bin/brain.exe --brain-home ~/AgentBrain dashboard
```

The `deployment` section of that output answers whether what is installed matches the source.
Full detail in [CLAUDE.md](CLAUDE.md).

## Where things live on disk

| Path | Contents |
|---|---|
| `~/AgentBrain/bin/` | Installed binaries — what actually runs |
| `~/AgentBrain/runtime/service.json` | Registered projects and their transcript sources |
| `~/AgentBrain/vault/` | The Markdown projection — open this in Obsidian |
| `~/AgentBrain/models/` | Embedding and re-ranking checkpoints |
| `D:\AgentBrainBackups` | Backups, on a separate drive by design |
| `../agent-brain-dashboard` | Next.js monitoring UI — separate repo, separate toolchain |

---

## Documentation

### Start here

| Document | What it answers |
|---|---|
| [docs/status.md](docs/status.md) | What is running right now, and what is left — every figure read from the live system |
| [docs/roadmap.md](docs/roadmap.md) | What remains, in what order, and the research behind each decision |
| [docs/architecture.html](docs/architecture.html) | The system end to end, as a diagram |
| [CLAUDE.md](CLAUDE.md) | Working agreements for agents in this repo — deployment, retrieval internals, invariants |
| [AGENTS.md](AGENTS.md) | The Codex-facing equivalent |

### Guides

| Document | What it answers |
|---|---|
| [docs/registering-a-project.md](docs/registering-a-project.md) | How to register a new project — the full procedure, including the service restart people miss |
| [docs/storage-and-backup.md](docs/storage-and-backup.md) | Storage sizing, retention, and the levers if the drive fills |

### Operations — `docs/operations/`

How to run, verify and recover the system.

| Document | What it answers |
|---|---|
| [runbook.md](docs/operations/runbook.md) | The operations runbook — start here when something is wrong |
| [install-upgrade-uninstall.md](docs/operations/install-upgrade-uninstall.md) | Windows install, upgrade and uninstall |
| [upgrades-and-rollback.md](docs/operations/upgrades-and-rollback.md) | Upgrading, and getting back if it goes badly |
| [backup-and-restore.md](docs/operations/backup-and-restore.md) | Backup and restore procedure |
| [health-and-alerts.md](docs/operations/health-and-alerts.md) | Health signals, alerts, and behaviour under pressure |
| [production-verification.md](docs/operations/production-verification.md) | Verifying a production deployment |
| [completion-audit.md](docs/operations/completion-audit.md) | Implementation completion audit |
| [schema-drift.md](docs/operations/schema-drift.md) | What happens when a transcript format changes underneath us |
| [storage-tiers.md](docs/operations/storage-tiers.md) | Storage tiers and compaction |
| [scale-benchmarks.md](docs/operations/scale-benchmarks.md) | Scale gates and what they measure |
| [mcp-and-query.md](docs/operations/mcp-and-query.md) | MCP tools and shared query operations |
| [codex-integration.md](docs/operations/codex-integration.md) | Codex lifecycle-hook integration |
| [claude-walking-skeleton.md](docs/operations/claude-walking-skeleton.md) | Claude operator verification walkthrough |
| [glm-configuration.md](docs/operations/glm-configuration.md) | Configuring the consolidation provider |
| [codegraph-provider.md](docs/operations/codegraph-provider.md) | The CodeGraph provider — note it now ships as its own MCP instead |
| [llm-wiki-provider.md](docs/operations/llm-wiki-provider.md) | The LLM Wiki provider — deliberately not enabled; see roadmap Part 5 |
| [obsidian-basic-memory.md](docs/operations/obsidian-basic-memory.md) | Obsidian and Basic Memory operations |
| [multi-agent-handoff.md](docs/operations/multi-agent-handoff.md) | Capture and handoff across agents |
| [concurrent-agent-workflow.md](docs/operations/concurrent-agent-workflow.md) | Two agents working at once |
| [lease-lifecycle.md](docs/operations/lease-lifecycle.md) | Writer lease lifecycle |
| [task-worktrees.md](docs/operations/task-worktrees.md) | Task worktrees |
| [merge-preflight.md](docs/operations/merge-preflight.md) | Non-mutating merge preflight |

### Schemas — `docs/schemas/`

The data contracts. Change one of these and something downstream breaks.

| Document | What it defines |
|---|---|
| [normalized-event.md](docs/schemas/normalized-event.md) | The normalized event and ledger contract |
| [memory-record.md](docs/schemas/memory-record.md) | The versioned memory record |
| [memory-precedence.md](docs/schemas/memory-precedence.md) | Precedence and supersession rules |
| [project-identity.md](docs/schemas/project-identity.md) | Project and worktree identity — the basis of cross-project isolation |
| [source-adapter-contract.md](docs/schemas/source-adapter-contract.md) | What a source adapter must guarantee |
| [segment-manifest.md](docs/schemas/segment-manifest.md) | Segment manifest schema |
| [task-and-lease.md](docs/schemas/task-and-lease.md) | Task and writer-lease schema |
| [provider-configuration.md](docs/schemas/provider-configuration.md) | Optional provider configuration |
| [codex-rollout-map.md](docs/schemas/codex-rollout-map.md) | Codex rollout evidence map |
| [hermes-state-map.md](docs/schemas/hermes-state-map.md) | Hermes state database evidence map |

### Design history — `docs/superpowers/`

The original specification and the implementation plans it was built from. Historical: they record
what was intended, not what is running. [docs/status.md](docs/status.md) is authoritative for the
latter.

| Document | |
|---|---|
| [specs/2026-08-01-cross-agent-secondary-brain-design.md](docs/superpowers/specs/2026-08-01-cross-agent-secondary-brain-design.md) | The production design |
| [plans/2026-08-02-secondary-brain-master-plan.md](docs/superpowers/plans/2026-08-02-secondary-brain-master-plan.md) | Master implementation plan |
| [plans/2026-08-02-foundation-walking-skeleton.md](docs/superpowers/plans/2026-08-02-foundation-walking-skeleton.md) | Foundation and walking skeleton |
| [plans/2026-08-02-multi-agent-capture.md](docs/superpowers/plans/2026-08-02-multi-agent-capture.md) | Multi-agent capture adapters |
| [plans/2026-08-02-temporal-memory-context.md](docs/superpowers/plans/2026-08-02-temporal-memory-context.md) | Temporal memory and the context compiler |
| [plans/2026-08-02-worktree-coordination.md](docs/superpowers/plans/2026-08-02-worktree-coordination.md) | Worktree coordination |
| [plans/2026-08-02-operations-scale.md](docs/superpowers/plans/2026-08-02-operations-scale.md) | Operations, backup and scale |
| [plans/2026-08-02-optional-knowledge-providers.md](docs/superpowers/plans/2026-08-02-optional-knowledge-providers.md) | CodeGraph and LLM Wiki providers |

### Reviews — `docs/reviews/`

| Document | |
|---|---|
| [2026-08-01-opus-cross-review-resolution.md](docs/reviews/2026-08-01-opus-cross-review-resolution.md) | Cross-review and how each finding was resolved |

### Archive — `docs/archive/`

Superseded documents, kept so a merge can be cross-checked against what went into it. Do not read
these for current state — [docs/status.md](docs/status.md) is authoritative.

| Document | |
|---|---|
| [2026-08-08-architecture-pre-merge.html](docs/archive/2026-08-08-architecture-pre-merge.html) | The architecture document before the merge. Carries its original figures — 136,841 events, 9,083 memories, 150 subject pages — and draws `SessionEnd` as unregistered |
| [2026-08-08-status-page-pre-merge.html](docs/archive/2026-08-08-status-page-pre-merge.html) | The separate status page, before it was folded into the architecture |

Both were merged into [docs/architecture.html](docs/architecture.html) on 8 August 2026.

### Your own notes

Files you wrote and marked as such. Left exactly as they are.

| Document | |
|---|---|
| `docs/(Self-created) pain-points.md` | Pain points this system exists to solve |
| `docs/(Self-created) my-prompt.md` | Verification prompts |

Also in `docs/`: competitor-analysis screenshots, and `(Out-dated)Secondary Brain — Architecture .html`
— a Chrome page-save that contains only an iframe shell. [docs/architecture.html](docs/architecture.html)
is the current one with real content in it.

---

## Invariants

Four rules that hold everywhere. Breaking one is a correctness bug, not a style question.

- **Cross-project isolation.** Zero leakage between projects. Ownership of a transcript is decided by
  the transcript's own recorded `cwd`, never by proximity in the directory layout.
- **Evidence is append-only.** Supersede or tombstone; never delete or rewrite.
- **Live state outranks memory.** Git, tests and deployments are authoritative. Memory is evidence to
  verify, not instruction to follow.
- **The context budget is a contract.** 1,000–1,500 tokens normal, 3,000 hard max, with
  `event:<uuid>` citations. Adding a field to an orientation means removing one.

Retrieval has its own set of hard-won rules — fuse by rank never by score, optional channels may only
add — documented in [CLAUDE.md](CLAUDE.md#retrieval).
