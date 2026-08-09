# Secondary brain — roadmap and research

What is left to build, what is deliberately not being built, and the research each decision rests
on. Current state lives in [status.md](status.md); this file is the forward half and the reasoning
behind it.

Reconciled against the running system on **9 August 2026**. Where a conclusion here was overtaken by
what shipped, the original reasoning is kept and the outcome recorded beneath it — a plan that
silently rewrites its own predictions cannot be checked against reality later.

---

## How to read this

Four verdicts are used throughout, and the middle two carry most of the argument.

| Verdict | Meaning |
|---|---|
| **Shipped** | Built, deployed, and verified running |
| **Obsolete** | A real feature elsewhere that this architecture makes unnecessary — not skipped, *dissolved* |
| **Dissolved** | Planned here, then solved a different way that made this version pointless |
| **Gap** | Genuinely missing, ranked below |

The obsolete column matters because roughly a third of the imported wish-list falls in it. This
brain generates its Markdown from an append-only ledger and verifies every file by checksum before
publishing; a system whose agent hand-writes notes into a vault needs a whole class of guardrails
that cannot apply here. Copying those would be building a lint for a language nobody writes in.

---

## Correction of record — "superlinear in memory count" was never true

Two of three projects delivered no orientation for days, and the working hypothesis through three
rounds of measurement was that the compile scales badly with the number of memories. It kept nearly
fitting, and it was falsified twice before it was dropped:

| Round | Reading | What it meant |
|---|---|---|
| First | `Ai-community` 5,505 memories → 38.5 s; `subscription` 5,644 → 41.7 s | Consistent with memory count |
| Second | `subscription` 5,644 → **1.0 s**; `Ai-community` 5,505 → 39 s | **Falsified** — near-equal corpora, 39× apart |
| Third | `subscription` 5,749 → **0.7 s**; `Ai-community` 5,445 → 39 s | Falsified again |

The discriminator was never the corpus. `rank_memories_against_recent_work` returns early when the
recent turns yield no text, and `subscription-agent`'s fast runs were **all** empty-query runs — the
project looked healthy because it was skipping the work, not doing it quickly. Underneath sat a
single absent index on `memory_supersession(superseded_version_id)`: **42,001 ms → 34.1 ms**, built
in 3 ms.

Two rewrites were spent on the wrong suspect before that. Both were genuine defects and both were
kept — a per-memory loader doing ~23,000 round trips, and an access counter committing once per id —
and **neither moved the number**. What ended it was three `tracing::info!` lines, added after the
third failed guess, which located the cost in one pass. The general form is worth carrying:
`HOOK_HARD_TIMEOUT` was a reasonable number that went silently wrong, and **a stage nobody times is a
stage nobody can be right about.** Full account in
[enhancement-review-plan.md](enhancement-review-plan.md).

---

## Correction of record — the Codex asymmetry never existed

Several conclusions below were written on the premise that Codex could not be *pushed* to and had to
be *asked*. **That premise was false, and the cause was ours.** `install_codex_hooks` wrote
`commandWindows` with the executable quoted; Codex does not strip those quotes, so the hook exited 1
before reaching our binary. Verified dispatching on 9 August 2026 after the fix — Desktop build
`26.803.41515`, CLI `0.147.0`.

The failure is worth carrying because of how it hid. A hook that cannot *launch* delivers nothing
**and spools nothing** — identical to never being invoked. On that evidence we concluded first that
Codex Desktop did not implement hooks, then that an upstream regression was responsible, recording a
matching build number. Neither was true. Both were reached by reasoning that was sound from evidence
that could not distinguish the cases.

Where this document argues from the asymmetry, the argument is superseded. It is kept rather than
rewritten, because a plan that silently repairs its own predictions cannot be checked later.

## Part 0 — The full ledger of what is left

### 0.1 From the original roadmap

| Item | Status | Reality |
|---|---|---|
| Hybrid retrieval (BM25 + vector + graph, RRF) | **Shipped** | k=60, weights 0.4 / 0.6 / 0.2, session-diversified. 63.3% → 90.0% R@5 on the target category |
| Wikilinks + index pages | **Shipped** | Derived from shared evidence and supersession, never LLM-proposed |
| Vault projection | **Shipped** | Content-addressed generations, checksum-verified, staging collected |
| LongMemEval-S harness | **Shipped** | Configurable, reports its own configuration, and the configuration is printed with every number |
| CodeGraph | **Dissolved** | Planned as a 7th brain MCP tool. It ships its *own* MCP server against both harnesses instead, so the proxy was never needed — see 5.3 |
| LLM Wiki | **Obsolete as code, adopted as method** | The provider is a keyword searcher over Markdown and would duplicate our own FTS5. The *methodology* became 3.2, 3.6–3.8 — see Part 5 |
| Decay / tiers / forgetting | **Shipped** | Access counting (4.1), derived staleness (4.2), gated eviction (4.3). Was "no code at all" |
| Cross-encoder rerank | **Shipped, and switched off** | Built, measured, and the measurement said leave it off — see 1.3 |
| **Ingest — integrate rather than append** | **Shipped** | The last Karpathy operation. Measured before it ran: 2,097 memories, 2,097 distinct ids, **zero supersession edges**. `brain reconcile --apply` folded 4 contradictions into 4 claims, retiring 7 without deleting anything — `fbc6fe5` |
| Query expansion | **Shipped** | The redirection 1.3 pointed at. Pseudo-relevance feedback, fused as a channel so it can only add — `6b431c0` |
| Scheduled reflection | **Shipped** | `brain digest` daily. Derived arithmetic, no provider call — the schedule without the generation, deliberately — `d4629d6` |

### 0.2 The five-item clone list — all closed

| # | Item | Verdict |
|---|---|---|
| 1 | Auto-maintained `[[wikilinks]]` | **Shipped**, and stronger — links derive from evidence, not from a model's suggestion, and two tests pin that a link to a superseded memory is dropped rather than left dangling |
| 2 | PostToolUse vault validation | **Obsolete** — see below |
| 3 | Injection-size meter | **Shipped** — the feature existed; the bug was that it measured the wrong side of the pipe (`407d345`) |
| 4 | Entity pages | **Shipped** as subject pages (`56cfb8a`) — 450 live, derived from embedding tightness rather than proposed. Synthesis prose (3.2b) is the remaining half |
| 5 | Cross-linker for orphans / broken links | **Obsolete** |

**Why #2 is obsolete.** That validator exists because their agent hand-writes Markdown into the
vault. Ours never does. Every note is generated: frontmatter through escaped `format!`, wikilinks
from `LinkIndex`, paths through `ensure_safe_relative` (absolute paths and traversal rejected), and
every file SHA-256-verified against the manifest before the generation is published. Malformed
frontmatter and misplaced files are not caught at write time — they are unconstructable. The one
uncovered case is **oversized notes**, which remain unbounded; low risk, since memories are
LLM-written summaries.

**Why #5 is obsolete.** Broken backlinks are impossible by construction and test-pinned. Orphans do
occur — `brain lint` counts 660 — but here an orphan is *honest*: a memory that genuinely shares no
evidence with any other. "A note without links is a bug" holds when a human forgot to link; it is
false when the linker is derived. A cross-linker would find almost nothing, and what it did find it
would have to invent.

### 0.3 Where agentmemory was genuinely ahead

| Item | Then | Now |
|---|---|---|
| Governance: delete + export + audit trail | Gap | **Closed** — `brain export` (`5c6eebc`), `brain forget` (`b05e67f`). Withdrawal is a tombstone every read path honours, so the ledger stayed append-only and the withdrawal is itself on the record |
| `memory_verify` — provenance as a tool | Gap | **Closed** — `brain verify memory` (`fa610b0`) resolves a claim to the transcript file and byte offset its evidence came from |
| Auto-forgetting (TTL, importance eviction) | Gap | **Closed, gated** — 4.1–4.3. Eviction *refuses* to run until thirty days of access data exist |
| Knowledge graph: entity extraction + BFS | Partly closed | **Closed enough** — 450 subject pages. BFS is still not built and still not obviously wanted; one hop over evidence already backs the graph channel |
| **Session replay** | Gap | **Closed** — `brain replay` (`3a2c5b1`) lists captured sessions and walks one in order. Verified on a three-day-old session: 1,369 events |
| Provider fallback chain | Gap — low | **Still a gap — low.** One provider; an outage defers rather than fails, which covers most of the value |
| Claude bridge (MEMORY.md sync) | Partial | **Still partial** — global preferences store exists; no bidirectional file sync |
| Privacy filter | Shipped — and ahead | Unchanged. `redact_string` strips credential assignments and secret tokens before egress *and* writes a per-job redaction manifest. Theirs strips silently; ours leaves an audit trail |
| Self-healing | Shipped | Unchanged. Six supervised loops with capped backoff, provider-unavailable deferral, dashboard health |
| Team memory (namespaced shared/private) | Not building | Unchanged. Single-operator system; cost is real, value is zero here |
| Git snapshots of memory state | Not building | Unchanged. Append-only ledger plus GFS backups already give version, rollback and diff |

### 0.4 Cross-check against their pipeline — the three gaps it surfaced

Reading their `PostToolUse → … → SessionStart` pipeline against ours turned up three items nobody
had written down. All three have since resolved, and two resolved differently than predicted.

1. **The orientation does not use retrieval.** `ContextCompiler::from_ledger` reads
   `recent_events()` chronologically, so all the hybrid retrieval work reaches `brain query` and the
   MCP tools and **none of it reaches the automatic session-start orientation**.
   → **Partly fixed, and the diagnosis was partly wrong.** 3.1 shipped relevance ranking over the
   orientation's memories, which was the real defect — they had been selected *alphabetically*. The
   orientation still does not go through `search()`, and that remains true by design: it is
   recency-shaped, and the ranking operates within that.
2. **No episodic layer.** Their `Stop / SessionEnd → summarize session` produces a per-session
   summary; our `MemoryKind` covered Semantic and Procedural but not Episodic.
   → **Fixed, and it was worse than described.** `session.ended` had been emitted *zero times* in the
   system's entire history, so the consolidation reason keyed to it was unreachable code. 3.3.
3. **No `PreCompact` re-injection.**
   → **Cut as redundant.** `SESSION_MATCHER` is `startup|resume|clear|compact|fork`, so `SessionStart`
   already re-fires *after* a compaction — the orientation is already re-delivered at exactly the
   moment context was discarded. A `PreCompact` hook injects *before*, which helps the compactor
   summarise rather than helping the session. A different and much smaller benefit than the item was
   written for.

---

## Part 1 — Ranked plan

Ordered by what unblocks what, not by size. Every item carries a **done-when** that can be checked by
running something, because "implemented" is not a state anyone can verify.

Sizes: **S** ≈ a sitting, **M** ≈ a day, **L** ≈ several days.

### 1.1 What remains — the full ranked list

**Nine items, nine shipped or built.** Four carried over, five added by the August competitor
review. What remains is two measurements and two provider-quota items — no unwritten feature except
3.2b's generation call. Ranked by value per effort, not by size.

| # | Work | Why here | Blocked on | Size |
|---|---|---|---|---|
| ~~**1**~~ | ~~**Run all 500 LongMemEval instances**~~ — **done** 9 Aug. **96.0% R@5 · 98.2% R@10 · 0.922 MRR** over 23,867 sessions and 246,750 turns, 3 h 54 m | Settled the competitive question with a number instead of an argument. `single-session-preference` reproduced at exactly 90.0% against a corpus 17× larger, which is why the earlier figure was worth trusting | — | — |
| ~~**2**~~ | ~~**Mid-session push via `UserPromptSubmit`**~~ — **shipped** `af81e4d` `4aafe32` | Was the largest architectural gap: the brain pushed **once**, at session start, so a session that pivoted was never re-oriented. Now re-queries retrieval on every prompt, 400 tokens, four memories, twenty per session, metered | — | — |
| ~~**3**~~ | ~~**Query expansion**~~ — **shipped** `6b431c0`. Pseudo-relevance feedback: top-five hits, terms by document frequency ≥ 2, best six appended, retrieve again. Fused as an extra channel, so it **cannot lose** a result the plain query found. Live: 0 lost, 1 newly reached | The measured answer to the vocabulary gap the cross-encoder failed to fix (1.4). The link between `ship to production` and `deploy` is in the corpus, not in any model | — | — |
| ~~**4**~~ | ~~**AI-first note format**~~ — **shipped** `6a1e34f`. Every note opens with a derived "For future agents" block: the claim, its weight, how to check it, and what would make it wrong | Notes are written for humans and retrieved by a model. A stable preamble and compiler-read frontmatter is cheap, and plausibly worth more than reranking was | Nothing | S |
| ~~**5**~~ | ~~**Contradiction resolution**~~ — **shipped** `eec8925` as `brain reconcile`. Authority, then recency, then evidence weight; no proposal when those are level | `brain lint` finds contradictions and stops. A cited proposal for one-click human approval closes the gap without a model silently deciding what is true | Nothing | M |
| ~~**6**~~ | ~~**Scheduled reflection**~~ — **shipped** `d4629d6` as `brain digest` + `AgentBrain.Digest`, daily. Never-retrieved share, retention distribution, contradictions, queue depth, appended to each project's vault `log.md`. **No provider call**, so it reports during exactly the outage that makes it most useful | The "maintains itself" claim. Consolidation already ran continuously; what was missing was anything that stepped back and asked whether the result was still coherent | — | — |
| **7** | **3.2b** — the synthesis *generation* call | The last piece of the Karpathy pattern. Validator, store and rendering ship | Watching the citation check refuse a real bad citation from a live provider | M |
| ~~**8**~~ | ~~**5.6** — the config panel~~ — **shipped** `10b72f8` (Rust) and `c75507c` (dashboard). Wave 5 complete | Every delivery defect here looked identical from a settings page, so no setting renders without the fact that decides whether it does anything. Caught two things on its first run, one of them a bug in itself | — | — |
| **9** | **0.2** — drain the consolidation backlog | 1,973 pending across three projects | **Provider quota.** 290 of 294 deferrals were plain HTTP 429. *Nothing to build* | — |
| ~~**10**~~ | ~~**Resolve the 4 contradictions**~~ — **done** `fbc6fe5`. Folded rather than decided: all four were re-derivations of one claim from overlapping event windows, which is arithmetic, not a judgement about truth | 7 memories superseded, 0 deleted. The first supersession this system has ever performed | — | — |
| ~~**11**~~ | ~~**Fold the other projects**~~ — **done**. 97 contradictions folded across three projects, 142 memories superseded, nothing deleted | 10 remain that derivation cannot separate: level on authority, date and evidence, where telling a re-wording from a disagreement is a reading of the text | — | — |
| **12** | **Decide the last 10 contradictions** | Pairs filed twice the same day from the same evidence, differing only in wording. Whether two bodies say the same thing is not derivable | **You** | S |
| **—** | **1 (token-saving A/B)** — harness **built** `6b07b18`, design in Part 2 | The headline this project is asked about, and the numerator is all that has ever been counted | An authenticated terminal (`claude -p` returns 401 here), then 0.2 | L |

**Done when**, for the ones where it is not obvious:

- **1 (500-run)** — a report stating R@5, R@10 and MRR *per category and pooled*, with the
  configuration printed beside every number.
- **2** — a session that changes subject receives a re-orientation, and the injection meter shows
  what it cost. See the caution in 1.2.
- **4** — the compiler reads a note's frontmatter rather than re-parsing prose, and orientation
  quality is re-measured after the change rather than assumed.
- **3** — an expanded query returns a superset of the plain one, always. That is the invariant, and
  it is pinned by a test rather than observed once.
- **5** — a proposed resolution cites both sides and applies only on explicit approval; refusing it
  leaves both memories current.
- **6** — the scheduled task runs unattended and the digest is on disk in every project's vault,
  with an unremarkable brain producing "Nothing needs a decision." rather than manufactured concern.
- **7** — a paragraph cites only memory ids present on its own page, and a bad citation leaves the
  links-only page intact.
- **9** — `consolidation_jobs` holds zero `pending` for an hour with the service running.

### 1.2 Item 2, as shipped — the caution and what it cost

The caution was that a push on every message spends tokens on every message, and the discipline that
kept the orientation at 1,115 tokens is that adding a field means removing one. That held: the push
gets **400 tokens and four memories**, against 1,000–1,500 for a session start that fires once.

The **injection-size meter** was copied outright from the `obsidian-mind` pattern and it is the last
line of every push, naming anything dropped: `[brain · 4 of 6 memories · 324 tokens · 2 dropped over
budget]`. A silent loss is worse than the bloat it avoids.

Three guards were not in the plan and came out of building it:

1. **A relevance floor stricter than search uses.** FTS terms are OR-joined, so a prompt shares
   "is"/"the"/"of" with almost everything and the ranking returns *something* for any input.
   Measured: a question about unladen swallows retrieved a database-migration memory and would have
   injected it. Two shared content terms minimum.
2. **A per-session cap**, found by live verification rather than by test. The no-repeat rule is per
   *memory*, so asking the same question twice correctly surfaces the *next* matches — two identical
   prompts pushed eight distinct memories. Right behaviour, unbounded; twenty per session bounds it.
3. **Silence as the default.** Short prompt, no session id, nothing new, nothing above the floor —
   all return nothing. Six of the nine tests are about when it stays quiet.

**The floor is a keyword floor, and that is a known handicap.** It will miss a memory that is
genuinely relevant and shares no vocabulary — the same gap the cross-encoder failed to close.
Cosine is no better as a gate: `all-MiniLM-L6-v2` scored a deployment-*speed* turn at 0.478 and the
turn that actually answered at 0.353, so no fixed cutoff separates them. Query expansion removes the
handicap, and this was the second independent argument for it. **Shipped** `6b431c0`; the floor now
sees the corpus's own vocabulary as well as the prompt's.

**One finding worth carrying.** The first live push surfaced two memories that are now false —
`decay/tiers has no code` and `this project uses embeddinggemma`. Both were true when written. That
is the counter-metric from Part 2 arriving early: a memory system can mislead with stale context, and
an unsolicited injection of a stale claim costs more attention than a stale note nobody opened.

### 1.3 The waves, as they finished

| Wave | Outcome |
|---|---|
| **0 — Repair the instruments** | 0.1 and 0.3 shipped. 0.2 is quota-bound. Nothing downstream was trustworthy without this, and 0.1 in particular blocked Part 2 entirely: an instrument that overstates cannot prove a saving |
| **1 — Prove the value** | Half. Retrieval quality measured and reproduced; the token saving never measured |
| **2 — Close the trust gap** | Complete. Export, tombstoned withdrawal, provenance walk. A hard delete would have broken the append-only invariant; a tombstone every read path honours gives the same user-visible result without breaking it |
| **3 — Quality** | 7 of 8 shipped, 3.5 cut, 3.2b half. The one item that could not be estimated was 3.2 — designed in Part 7 |
| **4 — Lifecycle** | Complete. Access counting, derived staleness, gated eviction |
| **5 — Console** | 3 of 6 panels, plus one unplanned. Session replay, retrieval explain and config remain — the retrieval panel that shipped shows *configuration*, not per-query explain |

### 1.4 The rerank result, and why it changed the plan

3.4 was written as *"LongMemEval `single-session-preference` R@5 improves on 90.0%, and no category
regresses"*. It shipped, it was measured, and it **failed its own done-when**: 90.0% → 86.7%, MRR
0.779 → 0.715, for 25% more wall-clock.

R@10 was unchanged at 96.7%, which locates the damage exactly — the answering session is still
retrieved, and the cross-encoder moves it *down*. Isolated on a fixture: changing one word of the
answer, `ship` → `deploy`, moved its score 10.2 points, the model's whole range, for a sentence
meaning the same thing. The bi-encoder fails the same cases in the same direction.

So it ships **off**, and the conclusion is a redirection rather than a retreat: **the fix for a
vocabulary gap is query expansion, not a second scoring model.** That became the next retrieval item —
new, on no earlier list — and it shipped as `6b431c0`. The corpus contains both words; asking twice,
the second time in the corpus's own vocabulary, reaches what one pass could not, and it costs a
retrieval rather than a model.

`Qwen3-Reranker-0.6B` was evaluated and rejected on cost rather than quality. It is the better model,
and instruction-following is the property that would actually address the gap — but at 1.1 GB on
disk, ~2.3 GB resident in a permanently-running service, and 26× the parameters of a stage already
costing 81–98 ms per candidate, no CPU-only configuration fits an interactive query. Revisit on a
GPU; the next rung short of that is `bge-reranker-base`.

### 1.5 Out of scope, and worth naming

**Hermes as a third harness.** `Harness::Hermes` exists in the domain model and the dashboard already
counts its events, so the brain would capture it the moment a transcript root existed. Out of scope
only because nothing writes one on this machine yet. When it arrives it is a transcript root and an
`AGENTS.md` section, not a wave.

### 1.6 Deliberately not building

Team memory · git snapshots of memory state · PostToolUse vault validation · orphan cross-linker ·
iii-style Workers/Functions/Triggers/States pages. The last deserves a sentence: those pages expose a
generic function runtime, and we do not have one. Our equivalent value is jobs, retrieval traces and
session replay — which are Wave 5 — not a KV browser over an engine we never ran.

---

## Part 2 — Proving the saving

**Still unmeasured, and still the largest claim this project cannot back.** What exists is one half
of the fraction — **1,115** mean tokens delivered per orientation over 35 receipts. The other half,
what a session *would* have spent without it, has never been captured. agentmemory's own
"19.5M → 170K" table is a modelled projection, not a measurement, so it is not a number to match.

### 2.1 Design: matched-pair A/B on real tasks

**Conditions.** Same task, same model, fresh session, one variable.

- **Cold** — brain hook uninstalled, MCP disabled. The agent starts from the repo alone.
- **Warm** — brain fully enabled.

**Task set.** Five tasks from this repository's real history, each with a known-correct answer that
lives in the brain and is expensive to re-derive from code:

| # | Task | The context that decides it |
|---|---|---|
| 1 | "Change the build config safely" | `.cargo/config.toml` `+crt-static` is load-bearing; removing it fails only in Codex's sandbox and Task Scheduler |
| 2 | "Add a field to the dashboard snapshot" | The TypeScript mirror in `lib/snapshot-types.ts` must change too; nothing enforces it |
| 3 | "Register a new project" | The service must be restarted or capture bindings never bind |
| 4 | "Retrieval returns nothing — why?" | FTS terms were `AND`-joined; nothing reached the ranking |
| 5 | "A consolidation job is enormous" | Dual bounds: ≤200 events *and* ≤400 KB, counting payload **and** raw |

**Repeats.** 3 per condition per task = **30 sessions**.

**Metrics per run.**

| Metric | How | What it proves |
|---|---|---|
| Input tokens to first correct action | Session usage up to the first action a rubric marks correct | The orientation's actual job: not re-deriving what is already known |
| Total input tokens for the task | Session usage at completion | The headline saving |
| Wall-clock to completion | Timestamps | Whether saved tokens translate to saved time |
| Quality (0–3) | Blind rubric, grader sees output only | Guards against "cheaper and worse" |
| Repeated-mistake count | Did it re-make a mistake the brain records? | The clearest single demonstration |

**Headline figure.** `(tokens_cold − tokens_warm) / tokens_cold`, per task *and* pooled, with the
spread — never a bare mean over five tasks.

**The counter-metric, reported with equal prominence.** Count of tasks where warm scored *worse* than
cold. A memory system that misleads with stale context is a real failure mode, and a report that
cannot show it is a marketing document.

### 2.2 Preconditions

1. ~~Wave 0.1 must land first~~ — **landed** (`407d345`). Measuring saved tokens with an instrument
   that counted compiled orientations rather than received ones would have overstated the numerator.
2. **Wave 0.2 must land first.** A half-consolidated brain understates the warm condition, and 1,973
   jobs are still queued.
3. Stop `AgentBrain.Service` during runs. Measured: with the backfill draining, a hybrid benchmark
   took over three hours for work that takes four minutes with the machine to itself. **And keep it
   stopped** — committing deploys, and deploying restarts the service, so a long run interrupted by
   any commit is back to contending with the backfill. The first 500-instance attempt died at 2h07m
   for a related reason: `cargo` relinking the harness out from under a run started through
   `cargo test`. Run a copy pinned outside `target/`.
4. **Provider quota.** `claude -p` first appears to fail with `401 Invalid bearer token` when
   spawned from an agent session — that is the child inheriting `ANTHROPIC_BASE_URL` without the
   auth that endpoint needs. Clear the inherited proxy variables and it authenticates, then
   returns `429 · Weekly/Monthly Limit Exhausted`. So this is the same blocker as 0.2, not a
   separate one, and it belongs beside it rather than in the unblocked column.

### 2.4 The harness, and what is already verified

`scripts/token-ab.ps1` (`6b07b18`). Dry run by default — it spends real money on 45 sessions, and
that is the operator's call, so nothing about it runs on a schedule.

Conditions are applied by rewriting the hook file and restoring it in a `finally`, including on
Ctrl-C: a benchmark that leaves the machine's hooks disabled is worse than one that never ran.
Verified on a 15-session execution — all three conditions switched, every session spawned, every
result parsed, and `~/.claude/settings.json` hashed identical before and after. The sessions
themselves returned 401, which is the blocker above and not a harness fault.

The five tasks and their rubrics are fixed in the file, in git, before any run — so nobody can say
the set was chosen after seeing the numbers.

### 2.3 Honest bounds

n=30, one machine, one operator, tasks chosen by the person who built the system. That is **evidence,
not a study**, and the write-up should say so in its first paragraph. What makes it credible is that
the task set is public, the rubric is fixed before running, and the counter-metric is reported.
Anyone can disagree with the tasks; nobody can say the result was selected after the fact.

---

## Part 3 — Should the LLM maintain it automatically?

**Yes — with exactly one exception. And the exception is the whole design.**

Karpathy settles the general question and it is not close: *"Humans abandon wikis because the
maintenance burden grows faster than the value… The wiki stays maintained because the cost of
maintenance is near zero."* A production second brain that needs a human to do bookkeeping is a
second brain that will be abandoned. Writing pages, revising entity pages, updating cross-references,
flagging contradictions, linting — automate all of it, fully, with no human in the loop.

We are in fact *more* automated than his pattern at the input end. His ingest is manual: you drop a
source in and tell the LLM to process it. Ours captures every session automatically with no action at
all. His human job — "curate sources, direct the analysis, ask good questions" — is, for us, already
just doing the work.

**The exception: deletion and eviction stay human-triggered.**

The value of this brain is not that it remembers — a vector store remembers. It is that every claim
cites `event:<uuid>` and you can walk it back to a transcript. Full LLM self-maintenance means a
model deciding what to forget, what to merge and what matters, and those decisions leave no evidence
trail. Automating them converts a **verifiable** system into a **plausible** one, which is the exact
failure this project was built to avoid.

The workable split:

| The LLM proposes | Derivation disposes |
|---|---|
| Extracting memories from evidence | Which memories link — shared evidence and supersession |
| Summarizing a session | Which claim supersedes which — explicit contradiction |
| Naming an entity | What decays — access counts, age, whether cited events are stale |
| Suggesting a merge | What is evicted — a mechanical policy, logged and reversible |

Everything in the right column is computed and auditable; everything in the left is a proposal that
lands as evidence-cited data.

**Wave 4 shipped exactly this, and the gate is the proof.** `brain evict` refuses to run until access
counting has run for thirty days, because on the day counting ships *every* memory is
never-retrieved — a policy reading that number would retire all 13,246 of them and write a defensible
reason for each. The refusal is arithmetic, not caution.

**One correction to the original framing, which still holds.** The brain is not *insufficiently
intelligent*; it is *incompletely running*. 1,973 pending jobs across three projects is a throughput
problem. Adding autonomy on top of a queue that is not draining would make an unreliable system
harder to diagnose.

---

## Part 4 — The dashboard as a viewer console

Lives at `C:\Users\quekm\Desktop\projects\agent-brain-dashboard` — separate repo, separate toolchain
(pnpm/Next.js 16), outside the Cargo workspace. It shells out to `brain dashboard --json` and caches
30 s, so it always reflects live data and never rebuilds the brain. Its TypeScript types in
`lib/snapshot-types.ts` mirror the Rust structs **by hand**: changing a field in
`crates/brain-cli/src/dashboard.rs` means changing that file too, and nothing enforces the
correspondence — a mismatch renders as plausible-looking wrong data.

The right target was **agentmemory's viewer, not iii's console**. Their console is a window on a
generic function runtime we do not have; the pages worth taking are the ones that answer questions
about memory.

| Panel | Answers | Status |
|---|---|---|
| Vector coverage | "Is the index built?" | **Shipped** `89bd75a` — per-project progress with the no-model case called out |
| Jobs & dead letters | "Is consolidation keeping up?" | **Shipped** `6d8c6cd` — with the *reason* a job died, not only that it did |
| Memory lifecycle | "Is anything being used?" | **Shipped** `7d12c15` — retrieved / stale / unlinked / withdrawn |
| **Retrieval configuration** *(unplanned)* | "What would a query do — and which stages can fire?" | **Shipped** `a4ce736` — each channel's weight beside whether it is available |
| Retrieval explain | "Why did *that* come back?" | **Gap** — and not what the panel above does. Per-channel contribution and fused rank for *one query*. `explain_text_search` exists in the store and is called from nothing but a test |
| Session replay | "What happened in that session?" | **Gap** — all the data exists; nothing renders it |
| Config | "What is actually configured?" | **Gap** — providers, ports, paths, model, brain home |
| Live stream | "Is it capturing right now?" | **Dropped** — lowest value of the set; the 30 s poll covers it |

**Why the retrieval panel earned its place.** Every retrieval defect in this project hid the same
way: the *configuration* looked right and the *capability* was absent. The vector channel silently
off with no model installed; the reranker installed but never enabled; events and memories merged
into one channel returning zero memories for weeks. A memory count cannot distinguish any of those
from a brain with less to say.

Deliberately excluded: Workers, Functions, Triggers, States, Flow. Those are an engine console for a
runtime this system does not have, and reproducing them would mean inventing the runtime first.

---

## Part 5 — CodeGraph, LLM Wiki, and the Karpathy pattern

### 5.1 The distinction that decides both

`crates/brain-context/src/llm_wiki.rs` is a **keyword searcher over a directory of Markdown files**
(`LlmWikiSourceStatus::ReviewedMarkdownVault`, term-matching, byte- and file-count capped). Pointed at
our vault it would search *our own generated notes* with a weaker index than the FTS5 we already run
over the same memories in the ledger. Enabling it does not add a capability; it adds a worse
duplicate of one.

Karpathy's LLM Wiki is **not a retrieval source at all.** It is a maintenance discipline. The claim is
explicit: RAG makes the model "rediscover knowledge from scratch on every question… Nothing is built
up," whereas a wiki is "compiled once and then *kept current*." The value is in pages that **compound
and get revised**, not in another way to search.

The two names collide and mean different things. Shipping the provider is not adopting the pattern,
and adopting the pattern does not require the provider.

### 5.2 We had his architecture. We were missing his operations.

| Karpathy's layer | Ours | Status |
|---|---|---|
| **Raw sources** — immutable, LLM reads but never modifies | Captured events, append-only by invariant | **Have — and stricter.** His immutability is a convention; ours is enforced |
| **The wiki** — LLM-generated Markdown, LLM owns it entirely | `vault/generated/`, projector owns it; `notes/` stays yours | **Have** |
| **The schema** — `CLAUDE.md` / `AGENTS.md` telling the LLM how to maintain it | Both exist and are mirrored | **Have** |
| `index.md` — content catalog, updated every ingest | `render_index`, regenerated each projection | **Have** |
| `log.md` — chronological, append-only, greppable | **Shipped** `e78762a` | **Closed** |

| Karpathy's operation | Ours | Status |
|---|---|---|
| **Ingest** — read source, then *update entity and concept pages across the wiki*; "a single source might touch 10–15 wiki pages" | Consolidation still writes one memory per episode, but a re-derived claim now **folds into the one it repeats** by supersession, and the subject page says so | **Closed structurally** `fbc6fe5`. The diagnosis in this row was subtly wrong: the problem was never that *pages* were unrevised, it was that *claims* were. 2,097 memories carried 2,097 distinct ids and zero supersession edges, so duplicates accumulated and `lint` called them contradictions. What is still open is *prose* — 3.2b, deliberately unfinished |
| **Query** — "good answers can be filed back into the wiki as new pages… your explorations compound" | **Shipped** `22f9e97` as `brain remember` | **Closed** |
| **Lint** — contradictions, stale claims, orphans, concepts lacking a page, missing cross-references | **Shipped** `7816bc3`, six derived rules | **Closed** |

The sentence that indicted the original design: *"the LLM doesn't just index it for later retrieval.
It reads it, extracts the key information, and integrates it into the existing wiki."*

**That indictment is now half answered.** Ours indexes, appends, links, supersedes, tombstones and
retires. It still does not *integrate* — **13,246 memories and not one has been revised in place.**
Supersession replaces a claim; it does not fold a new observation into an existing page. That is what
3.2b is for, and it is the last piece of the pattern.

### 5.3 Verdicts, and what actually happened

**LLM Wiki provider — did not enable, and should not.** A redundant retrieval path that would spend
orientation tokens searching a copy of what the ledger already indexes better. The methodology it is
named after was adopted through 3.2, 3.6, 3.7 and 3.8 — none of which needed that code.

**CodeGraph — the verdict was right and the plan was wrong.** The reasoning holds exactly: the
blocker was never value, it was the 400 tokens of headroom in a 1,500-token orientation, and that
constraint only exists if CodeGraph is an *orientation contributor*. On the pull path it costs zero
orientation tokens and is available exactly when an agent asks "where is X".

What the plan got wrong was the mechanism. CodeGraph ships its **own** MCP server and registers
itself with both harnesses directly, so the brain never needed a seventh tool. Installed v1.5.0 via
npm with provenance attestations verified, telemetry off, all three projects indexed. The brain's
internal provider stays `enabled: false` on purpose — proxying it would build a second path to data
both agents already reach.

The category error the original design made is worth keeping: code structure answers *"where is it"*,
which an agent can re-derive by reading files. The brain's irreplaceable answer is *"why is it like
that"*. Push the irreplaceable thing; make the re-derivable thing available on demand.

### 5.4 What the methodology added to Wave 3

| | Work | From | Status |
|---|---|---|---|
| 3.6 | File query answers back as pages | "Good answers can be filed back into the wiki as new pages" | **Shipped** — and it refuses an uncited claim at the argument level |
| 3.7 | `brain lint` | Karpathy's third operation, which we had none of | **Shipped** — found 4 contradictions, 660 islands, and 4 undated memories nobody was looking for |
| 3.8 | `log.md` in the vault | His logging convention, and nearly free | **Shipped** |

---

### 5.5 Taken further — `obsidian-second-brain`

A third system takes Karpathy's pattern past where we stopped, and its first row is the one that
hurts. Reviewed August 2026.

| | Them | Us |
|---|---|---|
| **New sources** | **Rewrite existing pages.** People get updated, claims revised, stale facts replaced | **Append.** 13,246 memories and not one revised in place |
| Contradictions | Resolved automatically | Detected and surfaced; never resolved |
| Patterns | Synthesised on their own into new pages | Not at all |
| When it runs | Four scheduled agents — morning brief, nightly consolidation, weekly review, health check | On capture only |
| Note format | **AI-first** — a `## For future Claude` preamble plus frontmatter written for retrieval | Human-readable Markdown |

Three of those five were ranked in Part 1: revision is item 7 and still open; contradiction
resolution shipped as `brain reconcile` (`eec8925`); scheduled reflection shipped as `brain digest`
and `AgentBrain.Digest` (`d4629d6`).

**The scheduling row deserves a note, because we did not copy it.** Four scheduled agents is a
schedule *and* a set of generation calls. We took the schedule and left the generation out: our
digest is derived arithmetic, so it runs whether or not a provider answers. That is a smaller feature
and a more reliable one — the review a maintainer acts on is "how much of this is unread, what
contradicts what, how far behind is the queue", and none of that needs a model.

**The note-format row is new, and it is the cheapest idea on this page.** Our notes are written for a
human to read and then retrieved by a model. Nothing about that ordering was decided; it is inherited
from the vault being an Obsidian vault. Shaping notes *for retrieval* — a stable preamble the
compiler can anchor on, frontmatter it reads rather than re-derives from prose — costs a projection
change and no new machinery, and it plausibly does more for orientation quality than re-ranking did.
It is item 4, and unlike the reranker it should be **measured after shipping rather than argued
before**.

**On auto-resolving contradictions, we should not follow them.** Resolving means a model deciding
which of two claims is true, and that decision leaves no evidence trail — the exact conversion of a
verifiable system into a plausible one this project exists to avoid. But "detect and stop" is not the
only alternative. The middle we have not built is a **proposal**: cite both sides, state which
supersedes which and why, and apply only on explicit approval. That keeps derivation in charge of the
disposition while letting the model do what it is good at. Item 5.

---

## Part 6 — Do we match agentmemory, and where do we beat it?

### 6.1 The Session 1 → Session 2 scenario

**Then: partially. Now: yes, by a different route than predicted.**

Capture was always complete — everything in their Session 1 is recorded, and more reliably. The
weakness was Session 2: their SessionStart runs a hybrid search; ours injected the *most recent* 500
events. When the last thing you did is the relevant thing, recency and retrieval agree and it looks
identical. Come back after a week on something else and recency hands you the wrong week.

3.1 fixed it — but the actual defect was worse and different. Events *were* correctly
recency-ordered; **memories were ordered alphabetically by subject**, so with 2,097 of them and room
for three, the alphabet was the selection. The orientation is still recency-shaped by design; the
ranking now operates within that.

### 6.2 Their nine hooks — and the half of the question we got wrong

**For capture, seven of nine are genuinely redundant, and that conclusion holds.** Their capture is
hook-driven; ours is transcript-driven. We tail the JSONL each agent already writes. A hook that
fails to fire loses an observation permanently; a transcript does not, and our watcher rescans every
120 s and resumes from a byte-offset cursor.

Their entire `PostToolUse` pipeline we already run, triggered by a file watcher rather than a hook:

| Their stage | Ours | Standing |
|---|---|---|
| SHA-256 dedup, 5-minute window | `UNIQUE(idempotency_key)`, 32 bytes | **Ahead** — schema-enforced and permanent, not a time window |
| Privacy filter — strip secrets | `redact_string` **plus a per-job redaction manifest** | **Ahead** — theirs strips silently; ours leaves an audit trail |
| Store raw observation | Append-only events | **Ahead** — immutability is enforced, not conventional |
| LLM compress → structured facts | `ConsolidationLlm::propose`, evidence-cited | **Equal**, and ours carries citations |
| Vector embedding | In-process MiniLM | **Behind on count** — one provider against six. Deliberate: no sidecar, no network, no install step |
| Index in BM25 + vector | FTS5 + vectors, fused by RRF | **Equal** |

**But capture is not the only thing those hooks do, and that is what the earlier analysis missed.**
Look at what the `obsidian-mind` pattern actually uses them for:

- `UserPromptSubmit` → injects **routing hints** back into the conversation
- `PreToolUse` → injects **enriched context** before a tool runs

Those are *push*, not capture. Which surfaces the finding:

> **The brain pushed exactly once, at session start.** A session that ran for hours and pivoted to a
> different subject was never re-oriented. **Closed** — `af81e4d`: the brain now sits on
> `UserPromptSubmit` alongside CodeGraph and re-queries retrieval on each prompt, under its own
> 400-token budget with a meter.

The proof is in this machine's own configuration: `UserPromptSubmit` **is** registered — by
**CodeGraph**, not by the brain. CodeGraph re-orients on every prompt; the brain does not. Two
systems on the same hook, one using it and one not.

It stayed invisible because both halves work: capture is complete, and the session-start orientation
is good. Nothing fails. The system simply stops helping after the first message.

That is item 2 in Part 1, and the caution in 1.2 applies — a push on every message needs its own
budget and a meter, or the contract that has held the orientation to 1,115 tokens quietly stops
holding.

| Hook | Their use | Us |
|---|---|---|
| `SessionStart` | project profile + inject | **Have** |
| `SessionEnd` / `Stop` | summarize the session | **Have** — for the boundary, which transcripts cannot express |
| **`UserPromptSubmit`** | capture prompts *and inject routing hints* | **Both covered, both harnesses.** Capture from the transcript; push shipped `af81e4d`, registered for Codex in `eb67502` and observed firing |
| `PreToolUse` | capture file access *and inject context* | Capture covered; the push is a lesser version of item 2 |
| `PreCompact` | re-inject before compaction | **Not needed** — `SessionStart` already matches `compact` |
| `PostToolUse` | capture tool + output | Not needed — in the transcript |
| `PostToolUseFailure` | capture errors | Not needed — in the transcript |
| `SubagentStart/Stop` | subagent lifecycle | Not needed — in the transcript |

### 6.3 Their four tiers against our nine kinds

| Their tier | Ours |
|---|---|
| Working — raw observations | `events`, append-only, **143,479** captured, none undated |
| Episodic — session summaries | **Closed** — 3.3, triggered by the boundary hook and told in the prompt that it is looking at a finished episode |
| Semantic — facts and patterns | `Fact`, `Decision`, `Investigation`, `Preference` |
| Procedural — workflows | `Procedure`, `Task`, `Deployment`, `Checkpoint`, `Timeline` |

Ours is finer-grained where it matters and carries a property theirs does not require at all: **every
memory cites the events it came from.** A tier label says what kind of thing a memory is; a citation
says whether it is true.

The *lifecycle* comparison needs splitting rather than a single verdict:

| Theirs | Ours |
|---|---|
| Ebbinghaus decay curve | **Not built.** Staleness is a boolean derived from age *and* disuse, not a continuous score |
| Access-strengthening | **Half.** We count retrievals; nothing is ranked up for being used |
| Auto-evict | **Built, gated.** Refuses until thirty days of access data exist |
| Automatic contradiction resolution | **Detect only** — `brain lint` finds them and stops. Item 5 proposes rather than applies |

The conservatism is deliberate in two of those four and simply unbuilt in the other two. Worth being
precise about which is which: a decay curve and access-strengthening are ordinary work nobody has
done, while refusing to auto-resolve is a position.

**Verdict: better on structure and trust, and no longer worse on freshness.**

### 6.4 Capability by capability

| Capability | Us | Reading |
|---|---|---|
| Automatic capture | **Better** | Transcript-based: survives a hook that never fires |
| Semantic search (BM25 + vector + graph, RRF) | **Better** | Same design and same k=60 — but our keyword channel is split in two, because events and memories have incomparable BM25 scales. Merging them returned zero memories for weeks |
| Memory evolution (versioning, supersession, graphs) | **Better** | Our relationship graph derives from shared evidence; theirs is LLM-proposed, so it can assert a link nothing supports |
| Auto-forgetting (TTL, contradiction, eviction) | **Equal, differently** | Shipped and gated. We refuse to evict on a young counter; they decay continuously |
| Privacy first | **Better** | We strip credentials *and* write a per-job redaction manifest. Theirs strips silently — ours can be audited |
| Self-healing | **Equal** | Six supervised loops, capped backoff, provider-outage deferral, health panel. Missing only a multi-provider fallback chain |
| Claude bridge (MEMORY.md sync) | **Better by omission** | They sync memory into a file the model reads. We inject at session start, so there is no file to go stale or be edited into disagreement with the ledger |
| Knowledge graph (entities + BFS) | **Equal** | 450 derived subject pages; no BFS, and no evident need for one |
| Citation provenance | **Better** | Every claim cites, and `brain verify memory` walks one back to a transcript byte offset |
| Session replay | **Worse** | Their viewer shows a session; ours shows aggregates |
| Team memory | **Not building** | Single-operator system |
| Git snapshots of memory | **Equal by other means** | Append-only ledger plus GFS backups with restore drills give version, rollback and diff |

Two things they have no equivalent of: `+crt-static` single-binary deployment with hash-verified
drift detection, and cross-project isolation enforced by each transcript's own recorded `cwd`.

### 6.5 So: better than agentmemory?

**On the thing that matters most here, yes — and for a structural reason rather than a feature
count.** Their memories are compressed observations. Ours are claims with citations, and the
projection is generated from the ledger and checksum-verified, so nothing in the vault can drift from
what the evidence supports. That is why our wikilinks can be *derived* and theirs must be *proposed*.

**One place we should not follow them.** Their SessionStart budget is 2,000 tokens against our 1,500.
Raising ours to match would be the easiest possible "improvement" and the wrong one — the budget is a
contract, and the discipline that adding a field means removing one is what has kept the orientation
at 1,115 tokens with 5.1 citations instead of drifting into a wall of text.

**What remains genuinely theirs:** session replay in the viewer, and a provider fallback chain. Both
are work, neither is architectural.

---

### 6.6 Their LongMemEval number, and ours — now comparable

**Resolved 9 August.** The run happened, and the answer to *"is their number bombast, or are we
behind?"* is **neither**: we are marginally ahead on two of the three figures and behind on the
third.

| | Ours, all 500 | Theirs, all 500 |
|---|---|---|
| R@5 | **96.0%** | 95.2% |
| R@10 | 98.2% | **98.6%** |
| MRR | **0.922** | 0.882 |

| Category | Instances | Share | Our R@5 |
|---|---|---|---|
| multi-session | 133 | 26.6% | 97.0% |
| temporal-reasoning | 133 | 26.6% | 94.0% |
| knowledge-update | 78 | 15.6% | 98.7% |
| single-session-user | 70 | 14.0% | 94.3% |
| single-session-assistant | 56 | 11.2% | **100.0%** |
| single-session-preference | 30 | 6.0% | 90.0% |

Four things to carry, and the last two are the ones that keep this honest:

1. **`single-session-preference` came back at exactly 90.0%**, reproducing the 30-instance figure
   against a corpus 17× larger — 23,867 sessions where the earlier run had 1,427. Two independent
   measurements agreeing to the decimal is the reason to trust either.
2. **The category-mix artefact is gone.** Their 86.2% BM25 baseline against our 63.3% was never
   evidence their keyword search was better; both were the same artefact seen from opposite ends.
3. **R@10 is behind.** 98.2% against 98.6%. This system ranks better inside the top five and
   retrieves marginally less inside ten, and that sentence belongs beside the other two figures
   every time they are quoted.
4. **Same dataset, not a controlled head-to-head.** Two independent harnesses computing the same
   metric over the same 500 questions is far closer than anything we could say before, and still not
   one harness running both systems.

*The original argument for why the comparison could not be made is kept below, because a plan that
silently rewrites its own predictions cannot be checked later.*

---

They publish **95.2% R@5 / 98.6% R@10 / 88.2% MRR** over all 500 questions, with an 86.2% BM25
fallback. We publish 90.0% R@5. Read side by side that looks like a five-point deficit. It is not a
comparison at all.

`single-session-preference` is **30 of 500 instances — 6.0% of the dataset**, and it is the hardest
slice: the category where BM25 alone scores 63.3%. Their figure is a weighted mean across all six
categories. Ours is our worst one.

| Category | Instances | Share | We have measured |
|---|---|---|---|
| multi-session | 133 | 26.6% | **never** |
| temporal-reasoning | 133 | 26.6% | **never** |
| knowledge-update | 78 | 15.6% | 10 → **100%** |
| single-session-user | 70 | 14.0% | **never** |
| single-session-assistant | 56 | 11.2% | **never** |
| single-session-preference | 30 | 6.0% | 30 → 90.0% |

**We have measured 40 of 500 instances — 8% — and reported the hardest 6%.** Their BM25 baseline tells
the same story from the other side: 86.2% against our 63.3% is not evidence their keyword search is
better, it is the same category-mix artefact. BM25 scores 100% on `knowledge-update` in our own
harness.

So the answer to "is their number bombast, or are we behind?" is **neither, and we cannot say yet**.
It is not a data problem and not a time problem in the sense of needing more history — the corpus is
the published dataset, identical for both. It is 3.5–4 hours of embedding we have not spent, on 92% of
the questions.

**Until that run exists, stop quoting 90.0% against 95.2%.** The honest statement is: *on the hardest
category, hybrid retrieval takes us from 63.3% to 90.0%.* That is a real result about a real
weakness. It is not a headline, and pooling it against someone else's headline is the kind of
comparison this project is supposed to be better than.

~~Item 1 in Part 1, and it is first for exactly this reason.~~ **Done** — see the table at the top of
this section. The prediction the paragraph above refused to make turned out to be right in substance:
the deficit was an artefact, not a gap.

---

## Part 7 — Subject pages: the design pass

The blocker was never the rendering. It was **how to decide what a subject is** without letting a
model assert one, because a subject an LLM invented is an unsourced claim sitting in a vault whose
entire property is that nothing in it is unsourced.

### 7.1 Two candidate discriminators, one of which failed

Candidate terms are easy: strip stopwords from memory titles and count. On the live corpus that gives
6,683 terms appearing in ≥3 memories — hopelessly many, and mixed. `discord`, `pipeline`,
`architecture` are real subjects; `fix`, `via`, `only`, `usage` are not. Frequency alone cannot tell
them apart.

**Tried first: evidence cohesion.** The intuition was that memories about one subject would share
evidence events, so a subject would show high pairwise co-citation and a filler word would not.
Measured on 2,030 memories, and **it does not work**:

| term | memories | evidence cohesion |
|---|---|---|
| `consolidation` | 66 | 0.004 |
| `retrieval` | 53 | 0.005 |
| `deploy` | 31 | 0.006 |
| `via` | 48 | 0.003 |
| `add` | 11 | **0.018** |

`add` scores higher than every real subject. The reason is worth recording because it is not obvious:
memories sharing evidence were written from the same **bounded consolidation job**, so co-citation
measures *temporal batching*, not subject affinity. Two memories about deployment written a month
apart cite no events in common at all. Co-citation is the right signal for wikilinks — which is what
it is already used for — and the wrong one for subjects.

**What works: embedding tightness.** Every memory has a vector. A real subject's memories sit
measurably closer together than two picked at random; a filler word's do not. Measured on the same
corpus, with a per-project random-pair baseline of **0.220**:

| term | memories | mean pairwise cosine | lift |
|---|---|---|---|
| `vault` | 22 | 0.481 | **+0.261** |
| `consolidation` | 66 | 0.435 | **+0.215** |
| `deploy` | 31 | 0.411 | **+0.191** |
| `backup` | 38 | 0.408 | **+0.188** |
| `retrieval` | 53 | 0.386 | **+0.166** |
| `fix` | 57 | 0.334 | +0.114 |
| `session` | 49 | 0.304 | +0.084 |
| `project` | 68 | 0.276 | +0.056 |
| `add` | 11 | 0.234 | +0.014 |
| `via` | 48 | 0.226 | +0.006 |
| `only` | 50 | 0.208 | **−0.012** |

A threshold at **+0.15 over the project's own random baseline** separates every real subject from
every filler word on this corpus. The baseline is computed per project rather than fixed, because a
narrow corpus is uniformly more similar than a broad one and a constant would mean something
different in each.

**Known miss:** `hook` scores +0.090 and is excluded, though it is arguably a real subject. It is
genuinely diffuse here — Claude hooks, Codex hooks, a research-guard hook and git hooks are four
different things sharing a word. Excluding it is the honest outcome, not a tuning failure, and it
argues for eventually keying subjects on something richer than a single token.

### 7.2 The definition

> A **subject** is a term appearing in at least five current memories whose vectors are at least 0.15
> more similar to each other, on average, than two memories drawn at random from that project.

Derived, deterministic given a ledger, no model asked to name anything, and it reuses the embedding
index built for retrieval rather than adding machinery.

### 7.3 What the page contains — and what it must not

**3.2a — the page asserts nothing. Shipped.** A subject-scoped index: the term, the memory count, and
every memory that mentions it as a wikilink, newest first. This is exactly the rule the project index
already follows — *"everything on it is a title and a link to a note that states its own evidence, so
the index cannot become wrong independently of the memories."* A page that only links cannot
contradict the ledger.

It compounds in **coverage**: a new memory mentioning the subject appears at the next projection, with
no revision step and nothing to go stale. 450 such pages are live.

**3.2b — the synthesis, which is where the risk is.** Coverage is not what Karpathy describes; he
wants pages that get *revised*. That needs prose, and prose is a new claim.

The validator shipped and is the reason the generation step is allowed to exist at all. Three rules,
and the third carries the argument:

1. Every sentence must cite at least one memory.
2. Every citation must name a memory **in this subject's own set** — not merely one that exists. A
   real memory from an unrelated subject is still a sentence this page cannot support, and accepting
   those lets a page drift into being about something else, one individually defensible sentence at a
   time.
3. **One bad sentence rejects the whole paragraph.** Dropping the offender would leave prose that
   reads as complete while having been quietly edited, and the reader cannot see the hole. Rejection
   falls back to links-only, which was already safe.

Sentences are length-capped for the same reason rather than for style: the unit of citation is the
sentence, so a "sentence" carrying four claims under one citation is three uncited claims wearing a
valid one's coat.

Storage is keyed by a **hash of the memory set**, not the subject's name. Prose describing five
memories is wrong once there are seven, and subjects gain memories continuously — that is what they
are for. A changed subject therefore has *no* paragraph rather than a stale one.

### 7.4 Implementation notes

- **Cost.** Mean-pairwise is O(n²); the shipped code uses the closed form
  `(‖Σv‖² − n) / (n(n−1))`, which is exact and O(n) for L2-normalised vectors. Candidates are cut to
  terms with ≥5 memories first.
- **Cap the vault.** Rank surviving subjects by lift × log(memory count) and keep the top ~150 per
  project, so the vault stays navigable and the generation hash stays stable.
- **Stability.** Subject sets change only when memories change, so the content-addressed generation
  machinery already handles republishing; no new invalidation logic.
- **Failure mode to watch.** Two spellings of one subject (`deploy` / `deployment`) produce two pages.
  Merging them by centroid distance is a refinement, not a blocker — two thin pages is a smaller
  problem than one wrong page.

---

## Sequencing

```
Wave 0  instruments  ──►  Wave 1  proof
   │                          │
   └─►  Wave 2  governance    └─►  Wave 3  quality  ──►  Wave 4  lifecycle
                 │                        │
                 └────────────────────────┴──►  Wave 5  console
```

Wave 0 gated Wave 1 because the measurement depends on it. Waves 2 and 3 were independent and could
have run in either order. Wave 4 depended on 3.3 for anything to decay meaningfully. Wave 5 trailed
everything, because a console is most useful once there is more to show.

**Waves 0, 2, 4 and 5 are done** bar the quota-bound backlog and two panels. **Wave 3** has 3.1,
3.2a, 3.3, 3.4, 3.6, 3.7 and 3.8 shipped, 3.5 cut, and 3.2b half. **Wave 1** is half.

The remaining order is short and has one real decision in it:

1. **0.2 drains itself** when quota returns. Nothing to do.
2. **3.2b's generation call** — the smallest remaining piece of code, and it wants a live provider to
   verify rather than a stub. It is now the *only* unwritten feature in any wave.
3. **Wave 1**, which needs 0.2 first — and an authenticated terminal, which was not on this list
   because nobody had tried to run the harness from inside an agent session before.
4. ~~**Query expansion**~~ — **shipped** `6b431c0`.
5. ~~**Session replay, retrieval explain and config**~~ — **all shipped** (`3a2c5b1`, `ec30787`,
   `10b72f8`). Wave 5 is complete.

**How that decision resolved.** The open question was whether query expansion outranked Wave 1. It
did, and for the stated reason: Wave 1 measures the value of a system, and expansion was the last
known defect in the part of that system the measurement would be measuring. It shipped first, so the
500-instance run measured the fixed system rather than one we already knew was handicapped.

**Except it did not, quite.** The harness has no switch for expansion, so the 96.0% R@5 it returned
is BM25 + vector only. Expansion shipped and is unmeasured at this scale — which is a smaller
embarrassment than it sounds, because the number stands without it, but it does mean the ordering
argument above was never actually tested.
