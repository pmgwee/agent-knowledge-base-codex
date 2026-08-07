# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

A cross-agent **secondary brain**: a persistent, local memory layer shared by Claude Code and
Codex. It captures every session transcript into per-project SQLite ledgers and serves a
bounded, evidence-cited orientation (~1,000–1,500 tokens, 3,000 hard max) at the start of any
session — so continuing prior work does not require re-reading the codebase or re-uploading a
transcript.

Rust 1.88.0 (MSVC, edition 2024), a 10-crate Cargo workspace producing four binaries.

| Binary | Role |
|---|---|
| `brain.exe` | CLI — register, status, query, dashboard, service install |
| `brain-service.exe` | Background capture, consolidation, rediscovery. Runs as a Task Scheduler task. |
| `brain-hook.exe` | Claude Code session-start hook. Pushes orientation over a named pipe (~9–12 ms). |
| `brain-mcp.exe` | MCP stdio server for Codex. Pull-based: `brain_checkpoint`, `brain_search`, and four others. |

## Deployment — read this before changing any Rust code

**The binaries the system runs are copies in `~/AgentBrain/bin/`, not `target/release/`.**
Building is not shipping. A source change that compiles but is never installed leaves the
service, the Claude hook, the Codex MCP server, and the dashboard all running older code —
and nothing about the running system looks wrong while that is true.

This is automated. **Committing is deploying.**

```
commit touching crates/ or Cargo.*  →  .githooks/post-commit  →  scripts/deploy.ps1 (detached)
                                                                    ↓
                              cargo build --release  →  install to ~/AgentBrain/bin/  →  restart service
                                                                    ↓
                                              ~/AgentBrain/runtime/deploy.json  →  dashboard
```

Enabled by `git config core.hooksPath .githooks`, which is set in this clone. The hook is
tracked in the repo, so it survives a fresh clone once that one config line is set.

### What you can rely on

- **Fail-safe.** The build completes before anything is replaced. A commit that does not
  compile leaves the previous deployment live and records `status: "failed"` with the compiler
  errors. It never installs a broken binary.
- **Non-blocking.** Deploy runs detached; the commit returns immediately. Progress lands in
  `~/AgentBrain/runtime/logs/deploy-*.log`.
- **Locked binaries are still replaced.** Windows refuses to overwrite a running image but
  permits renaming one, so the deploy retires the old file and writes the new one. `brain-mcp.exe`
  gets updated even while Codex holds it open — Codex picks it up on its next restart.
- **Verified, not assumed.** The deploy records a SHA-256 per binary; `brain dashboard` re-hashes
  what is on disk and compares. A half-applied deploy shows up as drift, not as success.
- **Docs-only and dashboard-only commits skip the build**, since they change no binary.

### Deploying manually

Needed when you want the working tree installed without committing:

```bash
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/deploy.ps1
```

Exit codes: `0` clean, `1` build failed (nothing replaced), `2` installed but one or more
binaries could not be replaced.

### Checking what is actually installed

```bash
~/AgentBrain/bin/brain.exe --brain-home ~/AgentBrain dashboard
```

The `deployment` section answers it: `up_to_date`, `source_changed`, `drifted_binaries`,
`unreplaced_binaries`, and the last build error. The dashboard UI renders the same thing under
**Deployment**.

### How drift is detected

By **hashing the build inputs** (`crates/`, `Cargo.toml`, `Cargo.lock`), not by comparing
commits. Comparing commits is wrong in both directions and often enough to matter: a docs
commit moves `HEAD` without changing any binary, and an uncommitted edit changes what a rebuild
would produce while `HEAD` sits still. A signal that cries wolf on every README commit is one
nobody reads.

So `source_changed` means exactly *"rebuilding now would produce different binaries"* — which
also means it catches uncommitted work in progress. That is intentional; it is the honest
answer. The commit is still shown, for orientation.

The deploy script records the fingerprint by invoking the binary it just built
(`brain source-fingerprint`), so the two sides cannot disagree about what "unchanged" means.

### Static CRT — do not remove `.cargo/config.toml`

The binaries are invoked from restricted launch contexts — Codex's hook sandbox and
Task Scheduler session 0 — where the dynamic VC++ runtime (`VCRUNTIME140.dll`) is not
on the DLL search path. Without static linking, `brain-hook.exe` fails to load in
Codex (reported as "hook exited with code 1") and `brain-service.exe` crashes on
startup under Task Scheduler (`NTSTATUS 0xC000013A`).

`.cargo/config.toml` sets `target-feature = +crt-static` for the MSVC target, baking the
CRT into each binary so they are self-contained and loadable anywhere. **Do not remove or
override this file.** A clean `cargo build` in an interactive shell will appear to work
even without static linking (VCRUNTIME140.dll is present in dev sessions) — the failure
is silent and only surfaces in production contexts.

Note: the post-commit deploy hook triggers on `crates/` or `Cargo.*`, **not** on
`.cargo/`-only commits. If you change `.cargo/config.toml`, run
`scripts/deploy.ps1` manually — the hook won't fire on its own.

## Build and test

```bash
cargo build --workspace --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings   # release gate
cargo fmt --all -- --check                              # release gate
```

Scale gates are `#[ignore]` by default; `--ignored` runs them (primary = 12k sessions / 6M
events, stress = 120k / 60M). The stress gate takes hours — do not start one casually.

## Retrieval

Search is three channels fused by **Reciprocal Rank Fusion** (`crates/brain-store/src/search.rs`).

| Channel | What it is | Weight |
|---|---|---|
| BM25 | FTS5 over events and memories | 0.4 |
| Vector | cosine over `all-MiniLM-L6-v2` embeddings, 384-dim | 0.6 |
| Graph | one hop across `memory_evidence`, both directions | 0.2 |

BM25 and vector are *retrieval* — each answers the query independently. The graph channel is
*association*, defined over what those two found, so fusion runs twice rather than once over
three lists.

RRF fuses by **rank, never by score**. A BM25 score and a cosine similarity are numbers on
incompatible scales; combining them directly needs a normalisation that is itself a guess, and
one that shifts as the corpus grows. `k = 60` is the constant from the original paper, kept so
our fusion is the one everyone else measured.

### Things worth knowing before changing this

- **No model installed means nothing changes.** Fusing a single channel is the identity, since
  RRF's score falls strictly with rank. There is no keyword-only branch to keep in step, and
  there must not be one.
- **Both layers are embedded** — memories *and* raw events. Memories drain first (thousands, ~10
  minutes); events follow (~135k, a few hours). Embedding only memories was the original design
  and it was wrong: the category this exists to fix is answered by a raw user turn, and the
  benchmark's ledgers hold no memories at all.
- **`rank_score`, not `bm25_score`.** A hit found only by meaning has a BM25 score of zero. Any
  re-ranking caller that reads `bm25_score` scores it as worthless and silently undoes the
  fusion — see the comment in `crates/brain-context/src/retrieval.rs`.
- **The vector channel re-applies every caller filter.** Time range, worktree, task, session,
  and — for memories — as-of and supersession. A channel that selected by id without those would
  resurrect retracted memories as current, on machines with a model and not on machines without.
- **The pull path has vectors; the push path does not.** `brain query` and the Codex MCP tools
  encode the question (~77 ms). The session-start hook does not: it runs against a hard timeout
  that has already failed silently once. Measure before changing that.
- **`all-MiniLM-L6-v2` is a bi-encoder.** It scores whether two texts are *alike*, not whether
  one *answers* the other. Measured: against a haystack sharing no vocabulary it lifts the
  answering turn from rank 3 to rank 1; against a merely on-topic haystack a turn about
  deployment *speed* scores 0.478 where the turn that actually answers scores 0.353. Reranking
  with a cross-encoder is the honest next step, not a weight to be tuned.

### Measuring a change

```bash
cargo test -p brain-cli --test longmemeval --release -- --ignored --nocapture
```

`LONGMEMEVAL_TYPES` scopes to a question category, `LONGMEMEVAL_BRAIN_HOME` turns on embedding
and fusion, `LONGMEMEVAL_DIVERSIFY=1` turns on the per-session cap. The report states its own
configuration; every switch changes the number, so never quote one without it.

## Invariants — do not break these

- **Cross-project isolation is a locked release criterion.** Zero leakage between projects.
  Ownership of a transcript is decided by the transcript's own recorded `cwd`, never by
  proximity in the directory layout. See `crates/brain-service/src/rediscover.rs` and the
  `discovery_never_crosses_a_project_boundary` test.
- **Evidence is append-only.** Supersede; never delete or rewrite.
- **Live state outranks memory.** Git, tests, and deployments are authoritative. Memory is
  evidence to verify, not instruction to follow.
- **The context budget is a contract.** 1,000–1,500 tokens normal, 3,000 hard max, with
  `event:<uuid>` citations. Adding a field to an orientation means removing one.
- **Cursors are keyed by source.** Never reorder or replace an entry in a project's source
  list — that orphans its cursor and re-ingests captured evidence. Append only.
- **Optional retrieval may only add.** The vector and graph channels are absent on a brain with
  no model, and a missing optional index must never break search that already works. Every entry
  point returns `None` or an empty channel rather than failing, and a query the model cannot
  encode falls back to keyword rather than erroring.

## Where things live

| Path | Contents |
|---|---|
| `~/AgentBrain/bin/` | Installed binaries — what actually runs |
| `~/AgentBrain/runtime/service.json` | Registered projects and their transcript sources |
| `~/AgentBrain/runtime/deploy.json` | Last deploy: commit, hashes, status |
| `~/AgentBrain/runtime/logs/` | Service and deploy logs |
| `D:\AgentBrainBackups` | Backups (separate drive, by design) |
| `../agent-brain-dashboard` | Next.js monitoring UI — separate project, separate toolchain |
| `docs/registering-a-project.md` | How to register a new project — the full procedure |
| `docs/storage-and-backup.md` | Storage sizing, retention, and the levers if the drive fills |

Three Task Scheduler tasks run as the current user: `AgentBrain.Service` (logon trigger,
restart 999× / 1 min), `AgentBrain.Backup`, `AgentBrain.RestoreDrill`.

## The dashboard

Lives at `../agent-brain-dashboard`, outside this workspace — different toolchain (pnpm/Node),
different lifecycle. It shells out to `brain.exe dashboard` and caches for 30 s, so it always
reflects live data. It does **not** rebuild the brain; that is what the deploy pipeline above
is for.

Its TypeScript types in `lib/snapshot-types.ts` mirror the Rust structs by hand. **Changing a
field in `crates/brain-cli/src/dashboard.rs` or `deployment.rs` means updating that file too** —
nothing enforces the correspondence, and a mismatch renders as plausible-looking wrong data.
Note that `time::OffsetDateTime` serializes as a 9-element array, except in the deployment
section, where timestamps originate as RFC 3339 strings in the deploy manifest.

## Registering a project

> **Mirrored section.** The same procedure appears under *Registering a project* in `AGENTS.md`,
> so either agent can run it. **Changing one means changing the other** — nothing enforces the
> correspondence, and an agent reading the stale copy follows stale instructions. The
> authoritative long form is `docs/registering-a-project.md`; if the three ever disagree, that
> file wins.

`brain register <path>` takes **only the local directory** — no repo URL, no project name.
Transcripts are discovered from `~/.claude/projects` and `~/.codex/sessions`, and each one is
claimed by the project whose root contains its own recorded `cwd`.

The step that is easy to miss: **restart the service afterwards.** Capture bindings are built
once, at startup (`build_capture_bindings` in `crates/brain-service/src/main.rs`), so until
`schtasks /Run /TN "AgentBrain.Service"` runs, the project sits in the config with nothing
capturing it — and everything looks fine while that is true.

Then **append the brain section to the new project's `AGENTS.md`** — append, never overwrite,
and substitute that project's own absolute path into the `brain_checkpoint(project: "...")`
call. Both agents are already wired globally, so this is not wiring; it is the instruction that
makes Codex *use* a tool it can already see, because its hook does not fire. Omitting it fails
silently — Codex simply works without prior context and nothing looks wrong. Claude Code needs
nothing: its hook is invoked by the harness and resolves the project from the session's `cwd`.

Full procedure, including verification and what to expect for storage:
`docs/registering-a-project.md`.

## Secondary brain (this project's own memory)

Claude Code sessions receive orientation automatically via the session-start hook — no action
needed, and nothing visible in the UI (it is injected as developer context in ~10 ms).

New sessions are discovered and captured within ~2 minutes; the service rescans the transcript
roots on startup and every 120 s. Registration alone is only a point-in-time snapshot, which is
why that loop exists.
