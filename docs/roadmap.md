# Secondary brain — roadmap and research

What is left to build, what is deliberately not being built, and the research each decision rests
on. Current state lives in [status.md](status.md); this file is the forward half and the reasoning
behind it.

Reconciled against the running system on **8 August 2026**. Where a conclusion here was overtaken by
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
| **Session replay** | Gap | **Still a gap** — the dashboard shows aggregates and cannot show you a single session |
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

### 1.1 What remains

Four items, and only one of them is code.

| | Work | Blocked on | Size |
|---|---|---|---|
| **0.2** | Drain the consolidation backlog — 1,693 pending against 2,884 completed | **Provider quota.** 290 of 294 deferrals were plain HTTP 429. Nothing to build | — |
| **1** | The token-saving A/B — design in Part 2 | 0.2, and a day of runs | L |
| **3.2b** | The synthesis *generation* call. Validator, store and rendering ship | Watching the citation check refuse a real bad citation from a live provider | M |
| **5.4/5.5/5.6** | Session replay · retrieval explain · config | Nothing | M each |

**Done when**, for each: 0.2 — `consolidation_jobs` holds zero `pending` for an hour with the service
running. 1 — a written report carrying per-task and pooled token deltas *with spread*, the rubric
scores, and the counter-metric, whose first paragraph states the sample size and who chose the tasks.
3.2b — a paragraph cites only memory ids present on its own page, and a bad citation leaves the
links-only page intact. 5.4 — a captured session can be scrubbed as discrete events.

### 1.2 The waves, as they finished

| Wave | Outcome |
|---|---|
| **0 — Repair the instruments** | 0.1 and 0.3 shipped. 0.2 is quota-bound. Nothing downstream was trustworthy without this, and 0.1 in particular blocked Part 2 entirely: an instrument that overstates cannot prove a saving |
| **1 — Prove the value** | Half. Retrieval quality measured and reproduced; the token saving never measured |
| **2 — Close the trust gap** | Complete. Export, tombstoned withdrawal, provenance walk. A hard delete would have broken the append-only invariant; a tombstone every read path honours gives the same user-visible result without breaking it |
| **3 — Quality** | 7 of 8 shipped, 3.5 cut, 3.2b half. The one item that could not be estimated was 3.2 — designed in Part 7 |
| **4 — Lifecycle** | Complete. Access counting, derived staleness, gated eviction |
| **5 — Console** | 3 of 6 panels, plus one unplanned. Session replay, retrieval explain and config remain — the retrieval panel that shipped shows *configuration*, not per-query explain |

### 1.3 The rerank result, and why it changed the plan

3.4 was written as *"LongMemEval `single-session-preference` R@5 improves on 90.0%, and no category
regresses"*. It shipped, it was measured, and it **failed its own done-when**: 90.0% → 86.7%, MRR
0.779 → 0.715, for 25% more wall-clock.

R@10 was unchanged at 96.7%, which locates the damage exactly — the answering session is still
retrieved, and the cross-encoder moves it *down*. Isolated on a fixture: changing one word of the
answer, `ship` → `deploy`, moved its score 10.2 points, the model's whole range, for a sentence
meaning the same thing. The bi-encoder fails the same cases in the same direction.

So it ships **off**, and the conclusion is a redirection rather than a retreat: **the fix for a
vocabulary gap is query expansion, not a second scoring model.** That is the next retrieval item, and
it is new — it was on no earlier list.

`Qwen3-Reranker-0.6B` was evaluated and rejected on cost rather than quality. It is the better model,
and instruction-following is the property that would actually address the gap — but at 1.1 GB on
disk, ~2.3 GB resident in a permanently-running service, and 26× the parameters of a stage already
costing 81–98 ms per candidate, no CPU-only configuration fits an interactive query. Revisit on a
GPU; the next rung short of that is `bge-reranker-base`.

### 1.4 Out of scope, and worth naming

**Hermes as a third harness.** `Harness::Hermes` exists in the domain model and the dashboard already
counts its events, so the brain would capture it the moment a transcript root existed. Out of scope
only because nothing writes one on this machine yet. When it arrives it is a transcript root and an
`AGENTS.md` section, not a wave.

### 1.5 Deliberately not building

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
2. **Wave 0.2 must land first.** A half-consolidated brain understates the warm condition, and 1,693
   jobs are still queued.
3. Stop `AgentBrain.Service` during runs. Measured: with the backfill draining, a hybrid benchmark
   took over three hours for work that takes four minutes with the machine to itself.

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
intelligent*; it is *incompletely running*. 1,693 pending jobs against 2,884 completed is a throughput
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
| **Ingest** — read source, then *update entity and concept pages across the wiki*; "a single source might touch 10–15 wiki pages" | Consolidation writes one new memory per episode and revises nothing | **Still the gap.** 450 subject pages compound in *coverage* — a new memory appears at the next projection — but no page is ever *revised*. 3.2b is the revision half, and it is deliberately unfinished |
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

### 6.2 Their nine hooks — we needed two

The decisive architectural difference: **their capture is hook-driven, ours is transcript-driven.** We
tail the JSONL each agent already writes. A hook that fails to fire loses an observation permanently;
a transcript file does not, and our watcher rescans every 120 s and resumes from a byte-offset cursor.

| Hook | Their use | Us |
|---|---|---|
| `SessionStart` | project profile + inject | **Needed — have it** |
| `SessionEnd` / `Stop` | summarize the session | **Needed — have it.** Not for capture: for the boundary, which transcripts cannot express |
| `PreCompact` | re-inject before compaction | **Not needed — cut.** `SessionStart` already matches `compact` |
| `UserPromptSubmit` | capture prompts | Not needed — in the transcript |
| `PreToolUse` | capture file access | Not needed — in the transcript |
| `PostToolUse` | capture tool + output | Not needed — in the transcript |
| `PostToolUseFailure` | capture errors | Not needed — in the transcript |
| `SubagentStart/Stop` | subagent lifecycle | Not needed — in the transcript |

Seven of nine are redundant here, and that is a strength rather than a shortfall: fewer moving parts
in the path that loses data when it breaks.

**The one thing transcripts genuinely cannot supply is the boundary.** A session ending writes no
line — the file simply stops growing — which is why `session.ended` had been emitted zero times
across 139,192 events while looking fully wired.

### 6.3 Their four tiers against our nine kinds

| Their tier | Ours |
|---|---|
| Working — raw observations | `events`, append-only, **140,134** captured |
| Episodic — session summaries | **Closed** — 3.3, triggered by the boundary hook and told in the prompt that it is looking at a finished episode |
| Semantic — facts and patterns | `Fact`, `Decision`, `Investigation`, `Preference` |
| Procedural — workflows | `Procedure`, `Task`, `Deployment`, `Checkpoint`, `Timeline` |

Ours is finer-grained where it matters and carries a property theirs does not require at all: **every
memory cites the events it came from.** A tier label says what kind of thing a memory is; a citation
says whether it is true.

The *lifecycle* gap — Ebbinghaus decay, access-strengthening, auto-evict — has closed, though
deliberately more conservatively than theirs: we surface conflicts rather than resolving them, and
eviction is gated on thirty days of evidence rather than a decay curve.

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
   verify rather than a stub.
3. **Wave 1**, which needs 0.2 first.
4. **Query expansion**, the new item — the measured answer to the vocabulary gap that the
   cross-encoder did not fix.
5. **Session replay, retrieval explain and config**, which have no dependencies and are the obvious
   things to pick up while waiting on quota.

The decision is whether query expansion outranks Wave 1. It probably does: Wave 1 measures the value
of a system, and query expansion is the last known defect in the part of that system the measurement
would be measuring.
