# Secondary brain — status

Where the system actually stands. Every figure below was read from the running system on
**8 August 2026** — the ledgers, the installed binaries, the vault on disk — not recalled from the
plan that proposed it.

The forward-looking half of this document lives in [roadmap.md](roadmap.md): the ranked plan, the
research it rests on, and the comparisons that shaped it. This file answers one question only:
**what is running right now.**

---

## Summary

Waves 2 and 4 are complete. Wave 0 is complete bar a quota-bound backlog. Wave 3 is complete except
for one generation step held back on purpose. Wave 5 has three of six panels, plus one unplanned. Wave 1 is half done —
retrieval quality is measured, the token saving never has been.

| | |
|---|---|
| Events captured | **140,134** across 3 projects |
| Current memories | **13,246**, every one citing `event:<uuid>` |
| Vector index | **complete** — 13,246 memories and 136,057 events, nothing remaining |
| Vault | **13,702** notes · 9,930 wikilinked · 450 subject pages · 23 flagged stale |
| Consolidation | 2,884 completed · **1,693 pending** · 3 dead-lettered |
| Orientation | mean **1,115** tokens over 35 receipts, max 1,490, against a 3,000 hard cap |
| Gates | `fmt` clean · clippy 0 errors · **124 test binaries green** |

---

## What is left, in order

| Wave | Work | Where it stands |
|---|---|---|
| **0 · Instruments** — done bar one | ~~Record deliveries after the flush~~ · ~~surface jobs and dead letters~~ · drain the backlog | Deliveries count receipts. The queue is visible. The backlog is quota-bound — 290 of 294 provider deferrals were plain HTTP 429 — so it finishes when quota allows, not when code changes |
| **1 · Proof** — half | ~~Retrieval quality on LongMemEval~~ · matched-pair A/B, 5 tasks × 2 conditions × 3 repeats | Retrieval measured and reproduced at 90.0% R@5. The token-saving percentage still does not exist; only the numerator has ever been counted |
| **2 · Trust** — done | ~~`brain export`~~ · ~~`brain verify memory`~~ · ~~`brain forget` as a tombstone~~ | Export runs clean. Verify resolves a claim to the transcript byte offset it came from. Withdrawal is a tombstone every read path honours, so the ledger stayed append-only |
| **3 · Quality** — 7 of 8 | ~~Orientation ranked by relevance~~ · ~~subject pages~~ · ~~`brain remember`~~ · ~~`brain lint`~~ · ~~vault `log.md`~~ · ~~session summaries~~ · ~~cross-encoder rerank~~ · synthesis prose | The orientation's memories had been ordered *alphabetically*. Rerank shipped and was measured *worse*, so it ships off. Synthesis prose is the last piece, held back on purpose |
| **4 · Lifecycle** — done | ~~Access counts~~ · ~~staleness marking~~ · ~~gated eviction~~ | Decay stays mechanical and logged — the model proposes, derivation disposes. Eviction refuses to run until thirty days of access data exist, because on day one every memory is never-retrieved |
| **5 · Console** — 3 of 6, plus one | ~~Vector coverage~~ · ~~jobs and dead letters~~ · ~~memory lifecycle~~ · session replay · retrieval explain · config · **+ retrieval configuration** *(unplanned)* | Every shipped figure already existed inside a command; the work was putting it where someone looks. The engine-console pages stay excluded — there is no such runtime here |

**One correction to an earlier count.** I previously recorded Wave 5 as "4 of 6" on the strength of the
retrieval panel. That panel shows retrieval's *configuration* — which channels exist, their weights,
whether each can currently fire. **5.5 asked for something different**: a per-query explain showing
why *that* result came back. `explain_text_search` still exists in the store and is still called from
nothing but a test. The panel was worth building and is not the item it was counted as.

---

## Every item, and where it stands

| Wave | Item | State | Evidence, or why not |
|---|---|---|---|
| 0.1 | Deliveries counted at receipt | **Shipped** `407d345` | An outcome dropped without recording leaves zero rows |
| **0.2** | **Consolidation drained** | **Quota-bound** | 1,693 pending; 290 of 294 deferrals were plain HTTP 429 |
| 0.3 | Queue and dead letters visible | **Shipped** `6d8c6cd` | Pending / leased / completed / dead per project, with the reason a job died |
| **1** | **Value proved** | **Half** | Retrieval measured; the token-saving A/B has never been run |
| 2.1 | `brain export` | **Shipped** `5c6eebc` | 41 MB, zero unresolved citations |
| 2.2 | `brain forget` | **Shipped** `b05e67f` | One test walks all six read paths |
| 2.3 | `brain verify memory` | **Shipped** `fa610b0` | Resolves a claim to `…jsonl:1051092` |
| 3.1 | Orientation ranked by relevance | **Shipped** `bc75702` | Memories had been ordered *alphabetically by subject* |
| 3.2a | Subject pages, derived | **Shipped** `56cfb8a` | 450 pages live |
| **3.2b** | **Synthesis prose** | **Partial** `2e02dac` | Validator, store, rendering shipped with 16 tests; the generation call is withheld |
| 3.3 | Session boundary + episodic consolidation | **Shipped** `53cc870` `d603260` | 3 `session.ended` events, 3 `session_stopped` jobs — from zero ever |
| 3.4 | Cross-encoder rerank | **Shipped, off** `4658271` | Measured *harmful* on the target category |
| 3.5 | `PreCompact` re-injection | **Cut** | Redundant — `SessionStart` already re-fires after a compaction |
| 3.6 | `brain remember` | **Shipped** `22f9e97` | Uncited claims refused at the argument level |
| 3.7 | `brain lint` | **Shipped** `7816bc3` | Six derived rules; defects exit non-zero, observations do not |
| 3.8 | Greppable vault `log.md` | **Shipped** `e78762a` | `grep "^## \[" log.md \| tail -5` returns the last projections |
| **3.9** | **CodeGraph on the pull path** | **Dissolved** | Runs its own MCP instead — see below |
| 4.1 | Access counting | **Shipped** `11f91dc` | Counted per result, not per search |
| 4.2 | Staleness surfaced | **Shipped** `6707add` `2ba9729` | 23 notes carry `stale: true`, reversed by retrieval |
| 4.3 | Gated eviction | **Shipped, gated** `73655b2` | Refuses for ~29 more days by design |
| 5.1–5.3 | Vector coverage · jobs · lifecycle panels | **Shipped** `89bd75a` `6d8c6cd` `7d12c15` | Per-project, live |
| — | Retrieval configuration panel *(unplanned)* | **Shipped** `a4ce736` | Each channel's weight beside whether it can fire. In-browser render unverified — see *Known gaps* |
| **5.4 / 5.5 / 5.6** | **Session replay · retrieval explain · config** | **Gap** | No dependency on anything. 5.5 is a renderer over `explain_text_search`, which exists in the store and is called from nothing but a test |

---

## Memory tiers

All four tiers exist. The lifecycle over them is where the remaining work is.

| Tier | Ours | State |
|---|---|---|
| **Working** — raw observations | `events`, 140,134 captured | ✅ |
| **Episodic** — session summaries | Boundary hook + `session_stopped` consolidation | ✅ shipped 8 Aug |
| **Semantic** — facts and patterns | `Fact`, `Decision`, `Investigation`, `Preference` | ✅ |
| **Procedural** — workflows | `Procedure`, `Task`, `Deployment`, `Checkpoint`, `Timeline` | ✅ |

Every memory additionally cites the events it came from, which no tier label supplies: a tier says
what kind of thing a memory is; a citation says whether it is true.

**What the lifecycle does and does not do:**

| | State |
|---|---|
| Access counting | ✅ per result, not per search |
| Derived staleness | ✅ 23 notes flagged, reversed the moment retrieval returns one |
| Gated eviction | ✅ refuses until thirty days of access data exist |
| Ebbinghaus decay curve | ❌ not built — staleness is a boolean, not a continuous score |
| Access-strengthening | ❌ half — retrievals are counted; nothing ranks up for being used |
| Contradiction resolution | ❌ detect only — `brain lint` finds them and stops |

The last one is a position, not a gap: resolving means a model deciding which claim is true, and that
leaves no evidence trail. The middle we have not built is *proposing* a resolution with citations for
approval — item 5.

---

## The four that are not simply "done"

**0.2 is quota-bound, not code-bound.** 1,693 jobs pending against 2,884 completed. The deferral
path works exactly as designed — no attempt consumed, nothing lost — and it finishes when quota
allows. There is no work here to do.

**1 is half measured.** Retrieval quality has a number and it reproduced exactly. The *token saving*
— the headline this project is usually asked about — has never been measured, and only the
numerator ever has. Design in [roadmap.md, Part 2](roadmap.md#part-2--proving-the-saving).

**3.2b is deliberately incomplete, and it is the last piece of the Karpathy pattern.** 450 subject
pages exist and every one of them only *lists* — **nothing in this vault has ever been revised in
place.** Supersession replaces a claim; it does not fold a new observation into an existing page.
The validator, store and rendering ship with 16 tests. The
generation call does not, and the reason is not effort: a subject page that only links *cannot*
contradict the ledger and a paragraph *can*, so the citation validation should be watched rejecting
a real bad citation from a real provider before prose reaches the vault. Building it against a stub
and shipping it unverified would invert the entire argument for having a validator.

**3.9 was dissolved, not skipped.** CodeGraph runs its **own** MCP server, registered in
`~/.claude.json` and `~/.codex/config.toml`, with all three projects indexed. The brain's internal
`codegraph` provider remains `enabled: false`, and that is the correct end state — proxying it would
build a second path to data both agents already reach directly.

---

## What the benchmark decided

Thirty `single-session-preference` instances, 1,427 sessions, 14,551 vectors, service stopped so the
run had the machine. One switch changed between the last two rows.

| Configuration | R@5 | R@10 | MRR | Elapsed |
|---|---|---|---|---|
| BM25 only *(prior baseline)* | 63.3% | 73.3% | 0.509 | — |
| BM25 + vector, RRF-fused | **90.0%** | 96.7% | **0.779** | 870 s |
| …plus cross-encoder rerank | 86.7% | 96.7% | 0.715 | 1,084 s |

**The baseline reproduced exactly**, which is the regression check that mattered: splitting events
and memories into separate keyword channels (`fafff21`) cost this category nothing.

**Re-ranking made it worse.** R@10 is identical, which locates the damage precisely — the answering
session is still *retrieved*, and the cross-encoder moves it *down* out of the top five. MRR falls
further than R@5 does, so the cost is spread through the ordering rather than concentrated in one
lost instance, and it spends 25% more wall-clock to do it.

Isolated beforehand on a fixture: holding the distractor constant and changing one word of the
answer, `ship` → `deploy`, moved its score **10.2 points** — the model's entire range — for a
sentence meaning the same thing. The bi-encoder fails the same cases in the same direction, which
makes this structural rather than a weak checkpoint. **The fix for a vocabulary gap is query
expansion, not a second scoring model.**

`--rerank` stays available because it separates cleanly on factual questions (6.230 / 2.365 /
−11.348). That half has not been benchmarked, and nothing should enable it globally on that basis.

### This number is not comparable to a published one

`single-session-preference` is **30 of 500 instances — 6.0% of the dataset**, and it is the hardest
slice. We have measured **40 of 500 — 8%** — and the figure we report is our worst category.
Competing systems publish a weighted mean across all six.

| Category | Instances | Share | Measured |
|---|---|---|---|
| multi-session | 133 | 26.6% | **never** |
| temporal-reasoning | 133 | 26.6% | **never** |
| knowledge-update | 78 | 15.6% | 10 → **100%** |
| single-session-user | 70 | 14.0% | **never** |
| single-session-assistant | 56 | 11.2% | **never** |
| single-session-preference | 30 | 6.0% | 30 → 90.0% |

**Do not quote 90.0% against anyone else's headline.** The honest statement is: *on the hardest
category, hybrid retrieval takes this system from 63.3% to 90.0%.* Running the other 92% is item 1 in
[what to pick up next](#what-to-pick-up-next), and it is first for exactly this reason.

---

## Findings worth carrying forward

Each of these was discovered while building something else, and each would have stayed invisible.

**The orientation was selected alphabetically.** Wave 3.1 was written as "the orientation is
recency-shaped and retrieval never reaches it". That was wrong in an interesting way: events *were*
recency-ordered, correctly, but memories were ordered **alphabetically by subject** —
`resolve_candidates` ended with `current.sort_by_key(subject_key)`. With 2,097 memories and room for
two or three, the alphabet was the selection. Every session opened with a note about a 150 ms status
message because it sorted first, and a memory titled *"Zero of 1,265 vault files contain wikilinks"*
could never appear at all.

**Mixed search returned no memories at all.** Events and memories were merged into one keyword list
and sorted by raw BM25. They live in different FTS tables with different corpus statistics, so their
scores are on incomparable scales — measured, events scored to 16.7 and memories to 13.2. With
25,174 events against 2,097 memories, events took every slot. Forty-four memories mentioned the query
term and not one was reachable. Fixed in `fafff21`.

**The session boundary had never once been observed.** Transcripts are append-only JSONL — a session
ending writes no line, the file simply stops growing — so `session.ended` was emitted **zero times
across 139,192 captured events**, and `ConsolidationReason::SessionStopped` was unreachable code that
read as fully wired. Sessions consolidated only on crossing 200 events, so a short session's work
waited for the *next* session to push it over.

**The delivery metric measured the wrong side of the pipe.** It recorded the moment an orientation
was *compiled*, one step before the reply was written, so a reply that never reached the agent
counted the same as one that did. A single day's log held ten `write hook reply` failures with nine
requests still spooled, every one counted. Rows written before `407d345` still overstate.

**`brain lint` found something nobody was looking for.** Four memories carry `valid_from` of
1970-01-01 — the Unix epoch, so they are not old but *undated*: the provider supplied none, and they
sort to the front of every chronological view, which is the opposite of what an unset field should
do. Split into its own rule because the fix is different from staleness.

**A dangling citation is unreachable through the supported path.** `append_memory` refuses a memory
citing an event the ledger does not hold, so the guarantee is enforced at write time rather than
checked at read time. `verify` keeps the branch as defence in depth, and a test now pins the refusal
that makes it dead code — a stronger property than the original report assumed.

**A narrow collision risk.** `projection_file_name` disambiguates with the first eight hex characters
of a memory id, and in a v7 UUID those are *timestamp bits* — so two memories with the same title
minted in the same millisecond collide on the `projection_path` unique index. It fails loudly rather
than overwriting, which is why it has never been seen.

**Three dead letters recorded 400 characters of plausible JSON and no reason.** `fail()` stored
`error.to_string()`, which takes anyhow's top-level message only, so serde's message naming the
offending field went on the floor. Same class of defect as the delivery metric: an instrument
reporting confidently and wrongly.

---

## Known gaps

**Open findings that need a human.** `brain lint` reports **4 contradictions**, **4 misdated**
memories, and **660 of 2,097 unlinked islands** (31.5%). Building the instrument was the
deliverable; deciding which side of a contradiction is true is not something derivation can do.

**The dashboard did not hydrate in the preview pane.** Fifty skeletons, unchanged across a 30-second
refetch interval, on *every* panel — not only the new one. The API returns correct data and the SSR
markup is right. This predates the retrieval panel and could not be distinguished from a limitation
of that browser pane; worth checking in a real browser.

**The brain pushes exactly once, at session start.** A session that runs for hours and pivots to a
different subject is never re-oriented. `UserPromptSubmit` is registered on this machine — by
**CodeGraph**, not by the brain — so the hook demonstrably works here and the brain simply does not
use it. Nothing fails; the system stops helping after the first message. Item 2 in
[what to pick up next](#what-to-pick-up-next).

**One number that could not be traced.** A 93.2% R@5 figure appears in an earlier roadmap and in no
document or commit. The recorded numbers are per-category: 63.3% → 90.0% on
`single-session-preference`. Those are not the same measurement and should not be quoted as one.

---

## What to pick up next

**Nine items.** Four carried over from the original waves, five added by the August competitor review.
The wave table above is the inventory of what was planned; this is the order to work in now. Full
reasoning and done-when criteria in [roadmap.md, Part 1](roadmap.md#part-1--ranked-plan).

| # | Work | Blocked on | Size |
|---|---|---|---|
| 1 | **Run all 500 LongMemEval instances** | Nothing — ~4 h unattended, service stopped | L |
| 2 | **Mid-session push via `UserPromptSubmit`** | Nothing | M |
| 3 | **Query expansion** | Nothing — the provider is already wired | M |
| 4 | **AI-first note format** | Nothing | S |
| 5 | **Contradiction resolution — proposed, never applied** | Nothing | M |
| 6 | **Scheduled reflection** — nightly, weekly | Nothing | M |
| 7 | **3.2b** — the synthesis generation call | A live provider, to watch the citation check refuse a real bad citation | M |
| 8 | **5.4 / 5.5 / 5.6** — session replay, retrieval explain, config | Nothing | M each |
| 9 | **0.2** — drain the consolidation backlog | **Quota.** 1,693 pending; nothing to build | — |
| — | **Token-saving A/B** | 0.2 first, then a day of runs | L |

### Why 1 is first

We have measured **40 of 500 LongMemEval instances — 8%** — and the one we report is the *hardest*
category. `single-session-preference` is 6.0% of the dataset; the other 92% has never been run. Every
competitive claim about this system is speculation until that number exists, in either direction.

### Why 2 is second — and it is a real architectural gap

**The brain pushes exactly once, at session start.** A session that runs for hours and pivots to a
different subject is never re-oriented; what it received at minute zero is all it ever gets.

The proof is in this machine's own configuration: `UserPromptSubmit` **is** registered — by
**CodeGraph**, not by the brain. CodeGraph re-orients on every prompt; the brain does not. It stayed
invisible because both halves work — capture is complete and the session-start orientation is good.
Nothing fails. The system simply stops helping after the first message.

**Caution.** A push on every message spends tokens on every message. The budget is a contract, and
the discipline that kept the orientation at 1,115 tokens with 5.1 citations is that adding a field
means removing one. This needs its own much smaller budget and an **injection-size meter** reporting
what each push cost — including naming anything it dropped, because a silent loss is worse than the
bloat.

### Why query expansion is third rather than later

It is the measured answer to the vocabulary gap the cross-encoder failed to fix — `ship` and `deploy`
are the same claim and score ten points apart, and a second scoring model cannot invent that link.
It is also the change most likely to lift the four categories item 1 will expose.

---

## Where to look next

| Question | File |
|---|---|
| Why each remaining item is ranked where it is | [roadmap.md](roadmap.md) |
| How the system fits together | [architecture.html](architecture.html) |
| How to register a project | [registering-a-project.md](registering-a-project.md) |
| Storage sizing and retention | [storage-and-backup.md](storage-and-backup.md) |
