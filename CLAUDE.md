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
| `brain-hook.exe` | Lifecycle hook for **both** harnesses — `SessionStart`, `SessionEnd`, `UserPromptSubmit`. Pushes over a named pipe. |
| `brain-mcp.exe` | MCP stdio server for Codex. **Depth on demand, not delivery** — `brain_search`, `brain_timeline` and four others answer questions a push cannot anticipate. |

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

- **No model installed means no vector channel, and no vector channel means no reordering.**
  Fusing a single channel is the identity, since RRF's score falls strictly with rank. There is
  no keyword-only branch to keep in step, and there must not be one. Note the graph channel is
  *not* gated on the model — it needs only `memory_evidence` — so a brain without a checkpoint
  still gets evidence expansion.
- **An id restriction replaces the keyword selector; it never narrows it.** The vector and graph
  channels choose documents, then a keyword statement re-fetches their bodies. If that statement
  keeps its `MATCH`, the two intersect and a document found *because* it shares no vocabulary
  with the question is dropped on the way back. Terms are OR-joined and include words like "I",
  so the intersection is nearly always non-empty and the bug reads as working code.
- **Both layers are embedded** — memories *and* raw events. Memories drain first (thousands, ~10
  minutes); events follow (~135k, a few hours). Embedding only memories was the original design
  and it was wrong: the category this exists to fix is answered by a raw user turn, and the
  benchmark's ledgers hold no memories at all.
- **`rank_score`, not `bm25_score`.** A hit found only by meaning has a BM25 score of zero. Any
  re-ranking caller that reads `bm25_score` scores it as worthless and silently undoes the
  fusion — see the comment in `crates/brain-context/src/retrieval.rs`.
- **Events and memories are separate keyword channels.** They live in different FTS tables with
  different field weights and corpus statistics, so their BM25 scores are on incompatible scales.
  Merging them into one list and sorting by raw score let the larger corpus win everything —
  measured, events scored to 16.7 and memories to 13.2, and a mixed search over 25,174 events and
  2,097 memories returned zero memories while 44 matched. Fuse by rank; never merge by score.
- **The vector channel re-applies every caller filter.** Time range, worktree, task, session,
  and — for memories — as-of and supersession. A channel that selected by id without those would
  resurrect retracted memories as current, on machines with a model and not on machines without.
- **The orientation's *events* are recency-shaped; its *memories* go through `search()`.** This
  entry used to say the session-start hook does not touch `search()` at all, and that half of it
  was wrong in a way that cost real time. `ContextCompiler::from_ledger` reads `recent_events()`
  chronologically — that part holds, and no retrieval work changes which turns appear — but it then
  calls `rank_memories_against_recent_work`, which builds a query from the last few turns and runs
  a **`memories_only` search** to order the memory section. So a change to `search()` does reach the
  orientation, and a slow search is a slow session start: that path was 12.6 s of a 14.9 s compile
  before the supersession index landed.
- **Time the stages before theorising about them.** Three `tracing::info!` lines carry this now, and
  they exist because three rounds of confident diagnosis picked the wrong suspect: `orientation
  compiled` (open / live_state / load / compile) in `hook_handler.rs`, `orientation material loaded`
  (events / ranking / stale / memories) in `compiler.rs`, and `search channels` (events / memories /
  vector / expansion / graph) in `search.rs`. They narrow a slow session start to one stage in a
  single hook invocation — read them before forming a hypothesis, not after.
- **`memory_supersession` must keep its index on `superseded_version_id`.** The primary key covers
  the *superseding* version and every read asks the opposite question. Without the second index
  SQLite answers "has this been superseded?" by sweeping `memory_versions` once per candidate row:
  measured 42,001 ms against 34.1 ms on a 5,669-version ledger, which was the entire 3 s hook budget
  on two of three projects. Pinned by plan, not by timing, in
  `crates/brain-store/tests/search_query_plan.rs`.
- **`all-MiniLM-L6-v2` is a bi-encoder.** It scores whether two texts are *alike*, not whether
  one *answers* the other. Measured: against a haystack sharing no vocabulary it lifts the
  answering turn from rank 3 to rank 1; against a merely on-topic haystack a turn about
  deployment *speed* scores 0.478 where the turn that actually answers scores 0.353. Reranking
  with a cross-encoder was the obvious next step — and measuring it produced the entry below.
- **The cross-encoder is off by default, and it does not fix what it was adopted to fix.**
  `ms-marco-MiniLM-L6-v2` is installed and wired as an opt-in stage (`--rerank`, `rerank: true`,
  `LONGMEMEVAL_RERANK=1`). On a factual question it separates cleanly — answer 6.230, on-topic
  neighbour 2.365, unrelated −11.348. On a **vocabulary gap it fails**, and the isolation is
  unambiguous: holding the distractor constant and changing one word of the answer, `ship` →
  `deploy`, moved its score from −11.131 to −0.913. Ten points, the model's whole range, for a
  sentence that means the same thing. Ungapped, the answer scores below a sentence about dogs.

  **The bi-encoder fails the same four cases in the same direction** (0.353 / 0.498 / 0.493 /
  0.618), which is what makes this structural rather than a checkpoint being weak: re-ranking
  sharpens ordering among candidates that already share the question's words, and cannot invent a
  link between "ship to production" and "deploying". `single-session-preference` — the category
  hybrid retrieval exists for — is made of exactly that shape. **The fix for a vocabulary gap is
  query expansion, not a second scoring model.** Pinned in
  `crates/brain-store/tests/reranker_model.rs`, including the failing direction.
- **`Qwen3-Reranker-0.6B` was evaluated and rejected on cost, not on quality.** It is the better
  model — 28 layers of 1024 against 6 of 384, a 151k vocab against 30k, and instruction-following,
  which is the property that would actually address the gap above. It is also 1.1 GB on disk and
  ~2.3 GB resident at our `F32` loader inside a permanently-running service, and re-ranking already
  costs **81–98 ms per candidate** on this CPU (32 candidates = 2.61 s) at 26× fewer parameters.
  There is no CPU-only configuration where it fits an interactive query. Revisit if this ever moves
  to a GPU; the next rung short of that is `bge-reranker-base`.
- **A re-ranked list must never be sorted against an unranked one.** Only `RERANK_DEPTH` head
  entries get a score, `rerank_score` is a separate field from `rank_score`, and the window is
  re-sorted in place. An unbounded logit near −11 and a fused score near 0.016 are not on one
  scale — sorting them together is the same defect as the events-versus-memories merge above, and
  it would look exactly like working code.

### Measured on LongMemEval-S — all 500 instances

Run 9 August 2026: 23,867 sessions, 246,750 turns, 243,657 vectors, 3 h 54 m with the service
stopped. BM25 + vector RRF-fused; **no rerank and no query expansion** — the harness has no switch
for expansion, so `6b431c0` contributed nothing to these numbers and remains unmeasured at scale.

| | Pooled, all 500 |
|---|---|
| R@5 | **96.0%** |
| R@10 | 98.2% |
| MRR | **0.922** |

| Category | Instances | R@5 |
|---|---|---|
| `single-session-assistant` | 56 | **100.0%** |
| `knowledge-update` | 78 | 98.7% |
| `multi-session` | 133 | 97.0% |
| `single-session-user` | 70 | 94.3% |
| `temporal-reasoning` | 133 | 94.0% |
| `single-session-preference` | 30 | 90.0% |

**`single-session-preference` reproduced at exactly 90.0%** — identical to the 30-instance run below,
against a corpus seventeen times larger. Two independent measurements agreeing to the decimal is the
reason to trust either.

Against the published comparison — 95.2% R@5 / 98.6% R@10 / 88.2% MRR — we are ahead on R@5 and MRR
and **behind on R@10**. Say that second part when quoting the first two. And note it is the same
dataset, not a controlled head-to-head: two independent harnesses computing the same metric.

#### The earlier per-category runs, kept

| Category | Instances | BM25 R@5 | Hybrid R@5 | BM25 MRR | Hybrid MRR |
|---|---|---|---|---|---|
| `single-session-preference` | 30 (all) | 63.3% | **90.0%** | 0.509 | 0.779 |
| `knowledge-update` | 10 | 100.0% | 100.0% | 0.933 | **1.000** |

R@10 on the preference category goes 73.3% → 96.7%. The second row is the regression check, not
a win: it is the category BM25 already tops out on, and the point is that fusion costs nothing
there. Session diversification alone moves neither, measured separately.

**Re-ranking was measured on the same 30 instances and it is a regression, not a gain.** Same
corpus, same 14,551 vectors, one switch changed:

| Configuration | R@5 | R@10 | MRR | Elapsed |
|---|---|---|---|---|
| BM25 + vector, RRF-fused | **90.0%** | 96.7% | **0.779** | 870 s |
| …plus cross-encoder rerank | 86.7% | 96.7% | 0.715 | 1,084 s |

R@10 is unchanged, which locates the damage exactly: the answering session is still *retrieved*,
and re-ranking moves it **down** out of the top five. MRR falls 8.2%, a larger relative drop than
R@5, because the cost is spread across ordering rather than concentrated in one lost instance. It
also costs 214 s — 25% more — to do it.

This is the vocabulary gap from the entry above, now measured end to end rather than on a fixture:
the cross-encoder scores confidently and puts the wrong turn first. **Leave `--rerank` off for
preference-shaped questions.** It remains available because it separates cleanly on factual ones
(6.230 / 2.365 / −11.348), and that half has not been benchmarked yet — but nothing should turn it
on globally on the strength of that.

A full sweep is ~4 hours and contends directly with the service's own backfill. Two things killed
the first attempt at 2 h 07 m with no report, both worth knowing before starting another:

- **`cargo test` relinks the harness mid-run.** Any `cargo build` in another shell tries to replace
  the running executable. Build once with `--no-run`, copy the binary out of `target/`, and execute
  the copy.
- **Committing deploys, and deploying restarts the service.** A run you carefully gave the machine
  to is back to contending with the backfill the moment you commit anything touching `crates/`.

### Measuring a change

```bash
cargo test -p brain-cli --test longmemeval --release -- --ignored --nocapture
```

`LONGMEMEVAL_TYPES` scopes to a question category, `LONGMEMEVAL_LIMIT` caps instances,
`LONGMEMEVAL_BRAIN_HOME` turns on embedding and fusion, `LONGMEMEVAL_DIVERSIFY=1` turns on the
per-session cap. The report states its own configuration; every switch changes the number, so
never quote one without it.

Stop `AgentBrain.Service` first if the backfill is still draining. It is not politeness — with
the backfill running, a hybrid run was measured taking **over three hours** for work that takes
four minutes with the machine to itself, and capture resumes losslessly from its cursors.

## Invariants — do not break these

- **Cross-project isolation is a locked release criterion.** Zero leakage between projects.
  Ownership of a transcript is decided by the transcript's own recorded `cwd`, never by
  proximity in the directory layout. See `crates/brain-service/src/rediscover.rs` and the
  `discovery_never_crosses_a_project_boundary` test.
- **Evidence is append-only.** Supersede; never delete or rewrite. The one exception is
  `repair_epoch_event_dates`, which moves `occurred_at` from the epoch to `observed_at` in place —
  justified because `occurred_at = 0` is the *absence* of an observation and `observed_at` is a fact
  the same row already holds. Row counts cannot change and a dated event is never touched.
- **Normalisation is a pure function of the record.** `assert_adapter_contract` normalises twice and
  compares; anything time-dependent inside an adapter breaks it. An undated record is dated at the
  ledger boundary, not in the adapter — that is where `observed_at` already varies per ingest.
- **`current` means the memory's *latest* version, and that it is current.** Both halves. They were
  indistinguishable until the first supersession, and eleven queries had only the second — see
  `CURRENT_CLAIM` in `crates/brain-store/src/lib.rs`.
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
| `docs/status.md` | What is running right now — every figure read from the live system |
| `docs/roadmap.md` | What is left, why, and the research behind each decision — seven parts |
| `docs/architecture.html` | The system end to end, as a diagram |
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

## How each harness receives context

**Both harnesses are pushed to.** Claude Code has always been; Codex Desktop joined on 9 August 2026
once its hooks were trusted.

| | Claude Code | Codex Desktop |
|---|---|---|
| `SessionStart` | Fires. 75+ deliveries | **Fires.** 1,043–1,045 tokens, 46 coordination tokens |
| `SessionEnd` | Fires | **Fires.** Real `session.ended` events with v7 ids. Declares a 3 s timeout because Codex clamps anything larger |
| `UserPromptSubmit` | Fires — mid-session push, 400 tokens | **Fires.** Registered `eb67502` |
| MCP | Not wired | 16 tools — **depth on demand, never delivery.** `brain_checkpoint` is redundant with the hook and no longer requested anywhere |

**Full parity, and all of it observed in one instrumented Codex session** —
`019fe7c5-698c-7c63-b4bd-57c66056f628`, 10 August 2026:

| Time (UTC) | Event | Delivery |
|---|---|---|
| 18:25:02 | `SessionEnd` | — (prior session closing) |
| 18:25:05 | `SessionStart` | 1,045 tokens, 46 coordination |
| 18:25:06 | `UserPromptSubmit` | **none** |
| 18:29:37 | `UserPromptSubmit` | 358 tokens, all coordination |

### To ask whether a hook fired, count `hook received` — never `context_deliveries`

This is the entry that would have prevented three wrong conclusions in five days, so read it before
running any hook diagnosis.

`context_deliveries` records that the brain **had something to say**, not that a hook ran. Only
`SessionStart` always does; `UserPromptSubmit` pushes when there is something new, and `SessionEnd`
usually pushes nothing at all. So a live, correct, fully-wired hook leaves **no trace** in that table
on most invocations — and a diagnosis that counts rows there reads healthy silence as a dead hook.

The session above shows it exactly: **two `UserPromptSubmit` invocations, one delivery row.** The
first fired 0.9 s after `SessionStart`, when the orientation had just gone out and there was nothing
to add, and correctly returned nothing. Counting deliveries reports "1 of 2 prompts"; in a session
where both are quiet it reports zero, which is indistinguishable from the hook never running.

`crates/brain-service/src/pipe.rs` therefore logs one `info` line — harness, event, session — for
**every** hook arriving at the pipe, before any decision about what to reply:

```bash
grep 'hook received' ~/AgentBrain/runtime/logs/brain-service.$(date +%F).jsonl
```

Three states, now distinguishable, that used to look identical:

| `hook received` | Delivery row | Meaning |
|---|---|---|
| absent | absent | The harness never invoked it — check `[hooks.state]` trust first |
| present | absent | Fired and had nothing to say. **Healthy.** |
| present | present | Fired and pushed |

**Two failure modes this section is built out of.** The table above contradicted itself for a day —
the `UserPromptSubmit` row said "not registered" while the paragraph below said the opposite, which
is what a hand-updated table beside hand-updated prose does. And on 9 August a test concluded
`UserPromptSubmit` "did not fire" from a `context_deliveries` count taken **36 seconds before** the
row appeared, in a window a hand-fired diagnostic was itself writing into. Polling a table you are
also writing to cannot answer who wrote the row.

### The trust gate — check this before concluding anything about Codex hooks

Codex records a **SHA-256 of each hook** in `~/.codex/config.toml` under `[hooks.state]`, and
**refuses to invoke an untrusted hook**. Approve them at the Codex CLI TUI's hook-review prompt.

```toml
[hooks.state.'C:\Users\you\.codex\hooks.json:session_start:0:0']
trusted_hash = "sha256:…"
```

**This cost two wrong conclusions five days apart, and the mistake is worth understanding.** Both
times the evidence was: zero `codex/SessionStart` deliveries *and* zero entries in
`~/AgentBrain/runtime/spool/`. That was read as "Codex never invokes the hook", on the reasoning that
a hook which fired and failed to deliver would still spool.

The reasoning is sound and the conclusion did not follow. **An untrusted hook is never invoked, so it
never spools either** — "not dispatched" and "not trusted" are indistinguishable from our side of the
pipe. The second attempt went further and blamed an upstream regression (`openai/codex#21639`),
recording a matching build number. A 5 August memory said *"Phase B Desktop hook test was confounded
by an untrusted hook"*; it was right, and louder wrong memories outranked it.

So: **`[hooks.state]` is the first thing to check** — before the spool, before the deliveries table,
and before reading any issue tracker.

### What is still asymmetric

**Nothing.** All three hooks apply to both harnesses, harness-invoked, before the model reads
anything — and as of 10 August that is no longer inferred from delivery rows but read directly off
`hook received`, which records the invocation whether or not anything was pushed.

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

**Nothing else.** There is no per-harness step: both hooks are registered globally and resolve
the project from the session's `cwd`.

The `AGENTS.md` brain section that used to be required here is **obsolete, and has been removed
from all three registered projects.** It asked Codex to call `brain_checkpoint` because its hook was
believed not to fire; the hook fires. Leaving it in is worse than redundant, since it spends a
tool call reproducing context the harness has already placed in front of the model — and it is a
*request*, not wiring, so the model may skip it, or make it after it has already started reading the
codebase, which is the cost the orientation exists to avoid.

Worth knowing if you carry this instruction to a new machine: this claim was written here on
9 August while only one of the three projects had actually been cleaned. The other two spent a day
telling Codex to fetch context the harness had already pushed. **Check the file, do not trust the
note.**

Full procedure, including verification and what to expect for storage:
`docs/registering-a-project.md`.

## Secondary brain (this project's own memory)

Claude Code sessions receive orientation automatically via the session-start hook — no action
needed, and nothing visible in the UI (it is injected as developer context in ~10 ms).

New sessions are discovered and captured within ~2 minutes; the service rescans the transcript
roots on startup and every 120 s. Registration alone is only a point-in-time snapshot, which is
why that loop exists.
