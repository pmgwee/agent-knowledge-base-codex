# Completion plan

What is left in the secondary brain, what is deliberately not being built, how the value gets
proved, and where the dashboard goes.

Written 2026-08-08, against commit `b4ffb34`. Every status below was checked against the code or
the live system on that date, not recalled.

---

## How to read this

Three verdicts are used throughout, and the second one carries most of the argument:

| Verdict | Meaning |
|---|---|
| **Shipped** | Built, deployed, and verified running |
| **Obsolete** | A real feature elsewhere that this architecture makes unnecessary — not skipped, *dissolved* |
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
| Vault projection | **Shipped** | Content-addressed generations, checksum-verified, staging now collected |
| LongMemEval-S harness | **Shipped** | Configurable, reports its own configuration |
| **CodeGraph** | **Gap — deferred** | 393 lines shipped, `enabled=false / usable=false` on all three projects, no binary installed |
| **LLM Wiki** | **Gap — deferred** | 305 lines, same state |
| **Decay / tiers / forgetting** | **Gap** | No code at all. Supersession is the only lifecycle mechanism |
| **Cross-encoder rerank** | **Gap** | No code. The measured next step for retrieval quality |

**On CodeGraph and LLM Wiki specifically — decided in Part 5.** Both are written, both are switched
off, and the reasoning for each turns out to be different. Short version: **ship CodeGraph, on the
pull path. Do not ship the LLM Wiki provider — adopt the methodology instead.**

### 0.2 The five-item clone list

| # | Item | Verdict |
|---|---|---|
| 1 | Auto-maintained `[[wikilinks]]` | **Shipped**, and stronger — links derive from evidence, not from a model's suggestion, and two tests pin that a link to a superseded memory is dropped rather than left dangling |
| 2 | PostToolUse vault validation | **Obsolete** — see below |
| 3 | Injection-size meter | **Gap, and it is a bug** — see 0.4 |
| 4 | Entity pages | **Gap — the highest-value item remaining** |
| 5 | Cross-linker for orphans / broken links | **Obsolete** |

**Why #2 is obsolete.** That validator exists because their agent hand-writes Markdown into the
vault. Ours never does. Every note is generated: frontmatter through escaped `format!`, wikilinks
from `LinkIndex`, paths through `ensure_safe_relative` (absolute paths and traversal rejected), and
then every file SHA-256-verified against the manifest before the generation is published.
Malformed frontmatter and misplaced files are not caught at write time — they are unconstructable.
The one uncovered case is **oversized notes**, which are unbounded; low risk, since memories are
LLM-written summaries. Folded into Wave 5 as a one-line guard.

**Why #5 is obsolete.** Broken backlinks are impossible by construction and test-pinned. Orphans do
occur, but here an orphan is *honest* — a memory that genuinely shares no evidence with any other.
"A note without links is a bug" holds when a human forgot to link; it is false when the linker is
derived. This tool would find almost nothing.

### 0.3 Where agentmemory is genuinely ahead

| Item | Status | Note |
|---|---|---|
| **Governance: delete + export + audit trail** | **Gap — highest risk** | No delete, no export, no redaction command. On an append-only ledger holding every keystroke, this is the most serious gap on the list |
| **`memory_verify` — provenance as a tool** | **Gap** | Citations exist in data; no command walks one back to its source events |
| **Session replay** | **Gap** | Dashboard shows aggregates and cannot show you a single session |
| Auto-forgetting (TTL, importance eviction) | **Gap** | Same item as decay/tiers |
| Knowledge graph: entity extraction + BFS | **Gap** | Subsumed by entity pages (0.2 #4) |
| Team memory (namespaced shared/private) | **Not building** | Single-operator system. Cost is real, value is zero here |
| Git snapshots of memory state | **Not building** | Append-only ledger plus GFS backups already give version, rollback and diff |
| Privacy filter | **Shipped — and ahead** | `redact_string` strips credential assignments and secret tokens before egress, and writes a **per-job redaction manifest**. Theirs strips silently; ours leaves an audit trail |
| Self-healing (circuit breaker, health) | **Shipped** | Six supervised loops with capped backoff, provider-unavailable deferral, dashboard health |
| Provider fallback chain | **Gap — low** | One provider today; an outage defers rather than fails, which covers most of the value |
| Claude bridge (MEMORY.md sync) | **Partial** | Global preferences store exists; no bidirectional file sync |

### 0.4 Cross-check against their pipeline — three gaps this surfaced that were not on any list

Reading their `PostToolUse → … → SessionStart` pipeline against ours turned up three items nobody
had written down:

1. **The orientation does not use retrieval.** `ContextCompiler::from_ledger` reads
   `recent_events()` chronologically. All the hybrid retrieval work reaches `brain query` and the
   MCP tools and **none of it reaches the automatic session-start orientation**. Theirs runs hybrid
   search at SessionStart. This is the single largest quality gap in the system and it was invisible
   because both halves work.
2. **No episodic layer.** Their `Stop / SessionEnd → summarize session` produces a per-session
   summary. Our `MemoryKind` already covers their Semantic (`Fact`) and Procedural (`Procedure`)
   tiers — what is missing is Episodic. A session summary is also the natural input to entity pages.
3. **No `PreCompact` re-injection.** When a session compacts, memory could be re-injected at exactly
   the moment context was just discarded. Cheap hook, high value, and not previously considered.

**The delivery metric is measured on the wrong side of the pipe.** `record_context_delivery` fires
at `hook_handler.rs:136`, one step *before* the reply is written and flushed at `pipe.rs:138`, so it
counts orientations **compiled**, not **received**. Today's log holds ten `write hook reply` failures
with nine requests still spooled — every one recorded as a delivery. This is clone-list item #3, and
it blocks Part 2 entirely: an instrument that overstates cannot prove a saving.

---

## Part 1 — Ranked plan

Ordered by what unblocks what, not by size.

### Wave 0 — Repair the instruments *(blocks everything measurable)*

| | Work | Why first |
|---|---|---|
| 0.1 | Record the delivery **after** the flush succeeds. Handler returns the reply plus a pending `ContextDelivery`; the pipe records it once the client has it | Every number in Part 2 comes from this table |
| 0.2 | Drain the consolidation backlog — 2,328 pending against 2,026 completed | Memories are incomplete until it drains; a benchmark over a half-built brain measures the backlog |
| 0.3 | Surface job queue depth and dead letters on the dashboard | 2,328 pending and 1 dead-lettered job are currently invisible |

Wave 0 is days, not weeks, and nothing downstream is trustworthy without it.

### Wave 1 — Prove the value *(Part 2 below is the full design)*

### Wave 2 — Close the trust gap

| | Work | Note |
|---|---|---|
| 2.1 | `brain export --project <p> [--since]` → JSON + Markdown bundle | Your data, retrievable without SQLite |
| 2.2 | `brain forget <selector>` — **tombstone, not deletion** | Append-only survives: write a redaction record that suppresses the target from retrieval and projection, keeps the evidence chain intact, and logs who/when/why. This is the append-only-compatible form of `memory_governance_delete` |
| 2.3 | `brain verify <memory-id>` — walk a claim back to its source events | Their `memory_verify`. Small, and it is the single best demonstration of what this brain has that a vector store does not |

2.2 needs care: a hard delete would break the "evidence is append-only" invariant, and a tombstone
that retrieval ignores gives the same user-visible result without breaking it.

### Wave 3 — Quality: make the orientation as good as the search

| | Work | Expected effect |
|---|---|---|
| 3.1 | **Orientation uses hybrid retrieval**, not just recency | The largest single quality gain available. Retrieval already works; it simply is not wired to the push path |
| 3.2 | **Entity pages**, derived — one page per recurring subject, compounding across sessions | Turns episodic notes into a wiki. Must be *derived* (co-citation, shared evidence, title n-grams), never LLM-asserted, or the provenance property is lost |
| 3.3 | **Episodic session summaries** at `SessionEnd` | Completes the tier model and feeds 3.2 |
| 3.4 | **Cross-encoder rerank** over the fused top-k | Measured need: `all-MiniLM-L6-v2` is a bi-encoder and scores likeness, not responsiveness — a turn about deployment *speed* scored 0.478 where the answering turn scored 0.353 |
| 3.5 | `PreCompact` re-injection | Cheap; restores context exactly when it was discarded |
| 3.6 | File kept query answers back as cited pages | Karpathy's "explorations compound"; see Part 5.4 |
| 3.7 | `brain lint` — contradictions, stale claims, concepts without a page | His third operation, which we have none of |
| 3.8 | `log.md` in the vault, append-only and greppable | His logging convention, nearly free |
| 3.9 | **Ship CodeGraph on the pull path** — a seventh MCP tool, not an orientation contributor | Zero orientation tokens; see Part 5.3 |

### Wave 4 — Lifecycle: decay without handing over judgement

| | Work | Design constraint |
|---|---|---|
| 4.1 | Access counting and last-used timestamps on memories | Mechanical input, no model involved |
| 4.2 | Staleness surfacing — mark, do not delete | A memory whose cited events are all old and never retrieved gets flagged in the projection and demoted in ranking |
| 4.3 | Eviction policy, opt-in and reversible | Only after 4.1/4.2 have run long enough to show the policy would have been right |

See Part 3 for why decay is deliberately mechanical here.

### Wave 5 — The dashboard becomes a console

Detailed in Part 4.

### Deliberately not building

Team memory · git snapshots of memory state · PostToolUse vault validation · orphan cross-linker ·
iii-style Workers/Functions/Triggers/States pages. The last one deserves a sentence: those pages
expose a generic function runtime, and we do not have one. Our equivalent value is jobs, retrieval
traces, and session replay — which are in Wave 5 — not a KV browser over an engine we never ran.

---

## Part 2 — Proving the saving

**The honest starting position: no percentage has been measured, and none can be quoted yet.**
What exists is one half of the fraction — 1,078 mean tokens delivered per orientation. The other
half, what a session *would* have spent without it, has never been captured. agentmemory's own
"19.5M → 170K" table is a modelled projection, not a measurement, so it is not a number to match.

### Design: matched-pair A/B on real tasks

**Conditions.** Same task, same model, fresh session, one variable.

- **Cold** — brain hook uninstalled, MCP disabled. The agent starts from the repo alone.
- **Warm** — brain fully enabled.

**Task set.** Five tasks, each drawn from this repository's real history, each with a known-correct
answer that lives in the brain and is expensive to re-derive from code:

| # | Task | The context that decides it |
|---|---|---|
| 1 | "Change the build config safely" | `.cargo/config.toml` `+crt-static` is load-bearing; removing it fails only in Codex's sandbox and Task Scheduler |
| 2 | "Add a field to the dashboard snapshot" | The TypeScript mirror in `lib/snapshot-types.ts` must change too; nothing enforces it |
| 3 | "Register a new project" | The service must be restarted or capture bindings never bind |
| 4 | "Retrieval returns nothing — why?" | FTS terms were `AND`-joined; nothing reached the ranking |
| 5 | "A consolidation job is enormous" | Dual bounds: ≤200 events *and* ≤150 KB, counting payload **and** raw |

**Repeats.** 3 per condition per task = **30 sessions**.

**Metrics per run.**

| Metric | How | What it proves |
|---|---|---|
| Input tokens to first correct action | Session usage up to the first action a rubric marks correct | The orientation's actual job: not re-deriving what is already known |
| Total input tokens for the task | Session usage at completion | The headline saving |
| Wall-clock to completion | Timestamps | Whether saved tokens translate to saved time |
| Quality (0–3) | Blind rubric, grader sees output only | Guards against "cheaper and worse" |
| Repeated-mistake count | Did it re-make a mistake the brain records? | The clearest single demonstration |

**Headline figure.** `(tokens_cold − tokens_warm) / tokens_cold`, reported per task and pooled, with
the spread — never a bare mean over 5 tasks.

**The counter-metric, reported with equal prominence.** Count of tasks where warm scored *worse*
than cold. A memory system that misleads with stale context is a real failure mode, and a report
that cannot show it is a marketing document.

### Preconditions

1. **Wave 0.1 must land first.** Measuring saved tokens with an instrument that counts compiled
   orientations rather than received ones would overstate the numerator.
2. **Wave 0.2 must land first.** A half-consolidated brain understates the warm condition.
3. Stop `AgentBrain.Service` during runs — with the backfill draining, a hybrid benchmark measured
   over three hours for work that takes four minutes idle.

### Honest bounds

n=30, one machine, one operator, tasks chosen by the person who built the system. That is
**evidence, not a study**, and the write-up should say so in its first paragraph. What makes it
credible is that the task set is public, the rubric is fixed before running, and the counter-metric
is reported. Anyone can disagree with the tasks; nobody can say the result was selected after the
fact.

---

## Part 3 — Should the LLM maintain it automatically?

**Yes — with exactly one exception. And the exception is the whole design.**

Karpathy settles the general question and it is not close: *"Humans abandon wikis because the
maintenance burden grows faster than the value… The wiki stays maintained because the cost of
maintenance is near zero."* A production second brain that needs a human to do bookkeeping is a
second brain that will be abandoned. Writing pages, revising entity pages, updating cross-
references, flagging contradictions, linting — automate all of it, fully, with no human in the loop.

We are in fact *more* automated than his pattern at the input end. His ingest is manual: you drop a
source in and tell the LLM to process it. Ours captures every session automatically with no action
at all. His human job — "curate sources, direct the analysis, ask good questions" — is, for us,
already just doing the work.

**The exception: deletion and eviction stay human-triggered.**

The value of this brain is not that it remembers — a vector store remembers. It is that every claim
cites `event:<uuid>` and you can walk it back to a transcript. Full LLM self-maintenance means a
model deciding what to forget, what to merge, and what matters, and those decisions leave no
evidence trail. Automating them converts a **verifiable** system into a **plausible** one, which is
the exact failure this project was built to avoid.

The workable split:

| The LLM proposes | Derivation disposes |
|---|---|
| Extracting memories from evidence | Which memories link — shared evidence and supersession |
| Summarizing a session | Which claim supersedes which — explicit contradiction |
| Naming an entity | What decays — access counts, age, whether cited events are stale |
| Suggesting a merge | What is evicted — a mechanical policy, logged and reversible |

Everything in the right column is computed and auditable. Everything in the left is a proposal that
lands as evidence-cited data. That is already the pattern used for wikilinks — derived, never
LLM-proposed — and Wave 4 applies it to decay.

**One correction to the framing.** The brain today is not *insufficiently intelligent*; it is
*incompletely running*. 2,328 pending jobs against 2,026 completed is a throughput problem. Adding
autonomy on top of a queue that is not draining would make an unreliable system harder to diagnose.
Fix throughput (Wave 0.2), then add judgement — and add it on the derived side.

---

## Part 4 — The dashboard as a viewer console

Lives at `C:\Users\quekm\Desktop\projects\agent-brain-dashboard` — separate repo, separate toolchain
(pnpm/Next.js 16), outside the Cargo workspace. It shells out to `brain dashboard --json` and caches
30 s, so it always reflects live data and never rebuilds the brain. Its TypeScript types in
`lib/snapshot-types.ts` mirror the Rust structs **by hand**: changing a field in
`crates/brain-cli/src/dashboard.rs` means changing that file too, and nothing enforces the
correspondence — a mismatch renders as plausible-looking wrong data.

Today: seven panels — overview, service health, deployment, storage, projects, token baseline,
coordination — plus vector coverage, added this session. Strong on *is it healthy* and *is what is
running what was built*. Blind on *what did it just do*.

The right target is **agentmemory's viewer, not iii's console**. Their console is a window on a
generic function runtime we do not have; the pages worth taking are the ones that answer questions
about memory.

| Panel | Answers | Why now |
|---|---|---|
| **Session replay** | "What happened in that session?" | Scrub a captured session as discrete events. All the data exists; nothing renders it |
| **Jobs & dead letters** | "Is consolidation keeping up?" | 2,328 pending and 1 dead-lettered are invisible today. Replay a dead letter from the UI |
| **Retrieval explain** | "Why did *that* come back?" | Per-channel contribution and fused rank for one query. `explain_text_search` already exists in the store — this is a renderer over it |
| **Vector coverage** | "Is the index built?" | **Shipped this session** — per-project progress with the no-model case called out |
| **Provenance walk** | "What is this claim based on?" | The UI half of Wave 2.3 |
| **Config** | "What is actually configured?" | Providers, ports, paths, model, brain home |
| **Live stream** | "Is it capturing right now?" | Lowest value of the set; the 30 s poll nearly covers it |

Deliberately excluded: Workers, Functions, Triggers, States, Flow. Those are an engine console for a
runtime this system does not have, and reproducing them would mean inventing the runtime first.

---

## Part 5 — CodeGraph, LLM Wiki, and the Karpathy pattern

### 5.1 The distinction that decides both

`crates/brain-context/src/llm_wiki.rs` is a **keyword searcher over a directory of Markdown files**
(`LlmWikiSourceStatus::ReviewedMarkdownVault`, term-matching, byte- and file-count capped). Pointed
at our vault it would search *our own generated notes* with a weaker index than the FTS5 we already
run over the same memories in the ledger. Enabling it does not add a capability; it adds a worse
duplicate of one.

Karpathy's LLM Wiki is **not a retrieval source at all.** It is a maintenance discipline. The claim
in the document is explicit: RAG makes the model "rediscover knowledge from scratch on every
question… Nothing is built up," whereas a wiki is "compiled once and then *kept current*." The value
is in pages that **compound and get revised**, not in another way to search.

So the two names collide and mean different things. Shipping the provider is not adopting the
pattern, and adopting the pattern does not require the provider.

### 5.2 We already have his architecture. We are missing his operations.

| Karpathy's layer | Ours | Status |
|---|---|---|
| **Raw sources** — immutable, LLM reads but never modifies | Captured events, append-only by invariant | **Have — and stricter.** His immutability is a convention; ours is enforced |
| **The wiki** — LLM-generated Markdown, LLM owns it entirely | `vault/generated/`, projector owns it; `notes/` stays yours | **Have** |
| **The schema** — `CLAUDE.md` / `AGENTS.md` telling the LLM how to maintain it | Both files exist and are mirrored | **Have** |
| `index.md` — content catalog, updated every ingest | `render_index`, regenerated each projection | **Have** |
| `log.md` — chronological, append-only, greppable | `brain timeline` as a command; no file | **Gap — small** |

| Karpathy's operation | Ours | Status |
|---|---|---|
| **Ingest** — read source, then *update entity and concept pages across the wiki*; "a single source might touch 10–15 wiki pages" | Consolidation writes one new memory per episode and revises nothing | **Gap — this is the whole pattern** |
| **Query** — and "good answers can be filed back into the wiki as new pages… your explorations compound" | `brain query` returns hits; answers vanish into chat | **Gap** |
| **Lint** — contradictions, stale claims, orphans, concepts lacking a page, missing cross-references | None | **Gap** |

The single sentence that indicts our current design: *"the LLM doesn't just index it for later
retrieval. It reads it, extracts the key information, and integrates it into the existing wiki."*
Ours indexes and appends. It never integrates. **Nine thousand memories and not one of them has ever
been revised.**

That is the same finding as clone-list item #4 (entity pages) arriving from a different direction,
which is a good sign it is the real gap.

### 5.3 Verdicts

**LLM Wiki provider — do not enable.** It is a redundant retrieval path, and enabling it would spend
orientation tokens to search a copy of what the ledger already indexes better. The methodology it is
named after is adopted through Wave 3.2, Wave 3.3, and two additions below — none of which need that
code.

**CodeGraph — ship it, on the pull path.** The blocker was never value; it was the 400 tokens of
headroom in a 1,500-token orientation. That constraint only exists if CodeGraph is an *orientation
contributor*. As a seventh MCP tool and a `brain query` source it costs **zero** orientation tokens
and is available exactly when an agent asks "where is X" — which is when code structure is worth
having and not before. The real work is installing the external binary, not the integration.

This also fixes a category error in the original design: code structure answers *"where is it"*,
which an agent can already re-derive by reading files. The brain's irreplaceable answer is *"why is
it like that"*. Push the irreplaceable thing; make the re-derivable thing available on demand.

### 5.4 Additions to Wave 3, from the methodology

| | Work | From |
|---|---|---|
| 3.6 | **File query answers back as pages.** A `brain query` result the user keeps becomes a memory citing the events it drew on | "Good answers can be filed back into the wiki as new pages" |
| 3.7 | **`brain lint`** — contradictions between memories, claims whose cited events are all stale, concepts mentioned across many memories with no page of their own, orphans | Karpathy's third operation, which we have none of |
| 3.8 | **`log.md` in the vault** — append-only, one line per consolidation and projection, `## [date] kind \| title` so `grep "^## \[" log.md \| tail -5` works | His logging convention, and it is nearly free |

---

## Part 6 — Do we match agentmemory, and where do we beat it?

### 6.1 The Session 1 → Session 2 scenario

**Today: partially. After Wave 3.1: yes.**

Capture is complete — everything in their Session 1 is already recorded, and more reliably. The
weakness is Session 2: their SessionStart runs a hybrid search; ours injects the *most recent* 500
events. When the last thing you did is the relevant thing, recency and retrieval agree and it looks
identical. When you come back to a project after a week on something else, recency hands you the
wrong week. Wave 3.1 is precisely this fix, and it is why it sits at the top of Wave 3.

### 6.2 Their nine hooks — we need three

The decisive architectural difference: **their capture is hook-driven, ours is transcript-driven.**
We tail the JSONL each agent already writes. A hook that fails to fire loses an observation
permanently; a transcript file does not, and our watcher rescans every 120 s and resumes from a
byte-offset cursor.

| Hook | Their use | Us |
|---|---|---|
| `SessionStart` | project profile + inject | **Needed — have it** |
| `SessionEnd` / `Stop` | summarize the session | **Needed — Wave 3.3.** Not for capture; for the episodic summary |
| `PreCompact` | re-inject before compaction | **Needed — Wave 3.5.** Cannot be replaced by transcripts: it is a re-injection point, not a capture point |
| `UserPromptSubmit` | capture prompts | Not needed — in the transcript |
| `PreToolUse` | capture file access | Not needed — in the transcript |
| `PostToolUse` | capture tool + output | Not needed — in the transcript |
| `PostToolUseFailure` | capture errors | Not needed — in the transcript |
| `SubagentStart/Stop` | subagent lifecycle | Not needed — in the transcript |

Six of nine are redundant here, and that is a strength rather than a shortfall: fewer moving parts
in the path that loses data when it breaks.

### 6.3 Their four tiers against our nine kinds

| Their tier | Ours |
|---|---|
| Working — raw observations | `events`, append-only, 136,841 captured |
| Episodic — session summaries | **Missing.** Wave 3.3 |
| Semantic — facts and patterns | `Fact`, `Decision`, `Investigation`, `Preference` |
| Procedural — workflows | `Procedure`, `Task`, `Deployment`, `Checkpoint`, `Timeline` |

Ours is finer-grained where it matters and carries a property theirs does not require at all:
**every memory cites the events it came from.** A tier label says what kind of thing a memory is; a
citation says whether it is true.

What they have and we do not is the *lifecycle*: Ebbinghaus decay, access-strengthening, auto-evict,
automatic contradiction resolution. We detect conflicts and surface them; we never resolve or
retire anything on our own. Wave 4.

**Verdict: better on structure and on trust, worse on freshness. After Waves 3.3 and 4, better on
all three.**

### 6.4 Capability by capability

| Capability | Us | Reading |
|---|---|---|
| Automatic capture | **Better** | Transcript-based: survives a hook that never fires |
| Semantic search (BM25 + vector + graph, RRF) | **Equal** | Same design, same k=60, same max-3-per-session diversification |
| Memory evolution (versioning, supersession, graphs) | **Better** | Our relationship graph derives from shared evidence; theirs is LLM-proposed, so it can assert a link nothing supports |
| Auto-forgetting (TTL, contradiction, eviction) | **Worse** | We have none. Wave 4 |
| Privacy first | **Better** | We strip credential assignments and secret tokens *and* write a per-job redaction manifest. Theirs strips silently — ours can be audited |
| Self-healing | **Equal** | Six supervised loops, capped backoff, provider-outage deferral, health panel. Missing only a multi-provider fallback chain |
| Claude bridge (MEMORY.md sync) | **Better by omission** | They sync memory into a file the model reads. We inject at session start, so there is no file to go stale or be edited into disagreement with the ledger |
| Knowledge graph (entities + BFS) | **Worse** | One hop over evidence, no entity extraction. Wave 3.2 |
| Team memory | **Not building** | Single-operator system |
| Citation provenance | **Better in data, worse in tooling** | Every claim cites; no command walks one back. Wave 2.3 |
| Git snapshots of memory | **Equal by other means** | Append-only ledger plus GFS backups with restore drills give version, rollback and diff |

Two things they have no equivalent of: `+crt-static` single-binary deployment with hash-verified
drift detection, and cross-project isolation enforced by each transcript's own recorded `cwd`.

### 6.5 So: better than agentmemory?

**On the thing that matters most here, yes — and for a reason that is structural rather than a
feature count.** Their memories are compressed observations. Ours are claims with citations, and the
projection is generated from the ledger and checksum-verified, so nothing in the vault can drift
from what the evidence supports. That is why our wikilinks can be derived and theirs must be
proposed.

**Today they are ahead on three things**, all on the plan: forgetting (Wave 4), entity/knowledge
graph (Wave 3.2), and session replay in the viewer (Wave 5). None is architectural; each is work.

**One place we should not follow them.** Their SessionStart budget is 2,000 tokens against our 1,500.
Raising ours to match would be the easiest possible "improvement" and the wrong one — the budget is
a contract, and the discipline that adding a field means removing one is what has kept the
orientation at 1,078 tokens with 5.1 citations instead of drifting into a wall of text.

---

## Sequencing

```
Wave 0  instruments        ──►  Wave 1  proof
   │                                │
   └──►  Wave 2  governance         └──►  Wave 3  quality  ──►  Wave 4  lifecycle
                    │                            │
                    └────────────────────────────┴──►  Wave 5  console
```

Wave 0 gates Wave 1 because the measurement depends on it. Waves 2 and 3 are independent of each
other and can run in either order; 2 is higher risk-reduction, 3 is higher visible quality. Wave 4
depends on 3.3 (episodic summaries) for anything to decay meaningfully. Wave 5 trails everything,
because a console is most useful once there is more to show.

If only one wave gets built: **Wave 0 then Wave 2.** Instruments that lie and a brain you cannot
export from are the two things that would make everything above it untrustworthy.
