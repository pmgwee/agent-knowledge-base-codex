# Secondary brain — status

Where the system actually stands. Every figure below was read from the running system on
**9 August 2026** — the ledgers, the installed binaries, the vault on disk — not recalled from the
plan that proposed it.

The forward-looking half of this document lives in [roadmap.md](roadmap.md): the ranked plan, the
research it rests on, and the comparisons that shaped it. This file answers one question only:
**what is running right now.**

---

## Summary

**All 500 LongMemEval-S instances are measured: 96.0% R@5, 98.2% R@10, 0.922 MRR.** For the first
time this is a pooled number over the whole dataset rather than one category, so it can be set
beside a published one — 95.2% / 98.6% / 88.2% — without the caveat that killed every earlier
comparison. We are ahead on R@5 and MRR and behind on R@10. Caveats below, and they matter.

Waves 2, 4 and **5** are complete. Wave 0 is complete bar a quota-bound backlog. Wave 3 is complete
except for one generation step held back on purpose. Wave 1 is half done — retrieval quality is now
fully measured, the token saving never has been, and the harness for measuring it exists.

**The vault has revised a claim in place for the first time.** `brain reconcile --apply` folded four
contradictions into four claims, superseding seven memories without deleting anything. Before it ran
there were 2,097 memories, 2,097 distinct ids, and **zero supersession edges** — the whole lifecycle
was schema and reader filters production had never once exercised. `brain lint` now reports no
contradictions on this project.

**Shipped since the August competitor review:** the mid-session push, a continuous decay curve with
access-strengthening, `brain reconcile` and the fold, `brain explain`, `brain replay`,
retrieval-shaped notes, query expansion, a scheduled health digest, and the 500-instance benchmark.
**Every item that review raised.**

Two things remain that nothing external is holding up: the benchmark (running) and the token-saving
A/B (never started). Two more wait on provider quota by choice — draining the consolidation backlog,
and 3.2b's generation call.

| | |
|---|---|
| Events captured | **142,830** across 3 projects |
| Current memories | **13,246**, every one citing `event:<uuid>` |
| Vector index | **complete** — 13,246 memories and 142,248 events; 3 remaining |
| Vault | **15,808** notes · 450 subject pages · 20 flagged stale — 4 subject pages now carry a revision trail |
| Consolidation | **1,933 pending** · 3 dead-lettered |
| Orientation | mean **1,115** tokens over 35 receipts, max 1,490, against a 3,000 hard cap |
| Gates | `fmt` clean · clippy 0 errors · **120 test binaries green** — plus the LongMemEval harness, which cannot relink while the 500-instance run holds its executable open |

---

## What is left, in order

| Wave | Work | Where it stands |
|---|---|---|
| **0 · Instruments** — done bar one | ~~Record deliveries after the flush~~ · ~~surface jobs and dead letters~~ · drain the backlog | Deliveries count receipts. The queue is visible. The backlog is quota-bound — 290 of 294 provider deferrals were plain HTTP 429 — so it finishes when quota allows, not when code changes |
| **1 · Proof** — half | ~~Retrieval quality on LongMemEval~~ · matched-pair A/B, 5 tasks × 2 conditions × 3 repeats | **All 500 instances measured: 96.0% R@5, 98.2% R@10, 0.922 MRR.** The token-saving percentage still does not exist; only the numerator has ever been counted |
| **2 · Trust** — done | ~~`brain export`~~ · ~~`brain verify memory`~~ · ~~`brain forget` as a tombstone~~ | Export runs clean. Verify resolves a claim to the transcript byte offset it came from. Withdrawal is a tombstone every read path honours, so the ledger stayed append-only |
| **3 · Quality** — 7 of 8 | ~~Orientation ranked by relevance~~ · ~~subject pages~~ · ~~`brain remember`~~ · ~~`brain lint`~~ · ~~vault `log.md`~~ · ~~session summaries~~ · ~~cross-encoder rerank~~ · synthesis prose | The orientation's memories had been ordered *alphabetically*. Rerank shipped and was measured *worse*, so it ships off. Synthesis prose is the last piece, held back on purpose |
| **4 · Lifecycle** — done | ~~Access counts~~ · ~~staleness marking~~ · ~~gated eviction~~ | Decay stays mechanical and logged — the model proposes, derivation disposes. Eviction refuses to run until thirty days of access data exist, because on day one every memory is never-retrieved |
| **6 · Depth** — **done**, 7 of 7 | ~~Mid-session push~~ · ~~AI-first notes~~ · ~~contradiction proposals~~ · ~~decay + strengthening~~ · ~~query expansion~~ · ~~scheduled reflection~~ · ~~500-instance benchmark~~ | Added by the August competitor review. The push closed the largest architectural gap: the brain used to orient once and then stop helping. Expansion closed the second — the corpus's own words bridge a vocabulary gap no scoring model could invent |
| **7 · Integration** — done | ~~Fold a contradiction by supersession~~ · ~~revision trail on subject pages~~ · ~~one definition of "current"~~ | The Karpathy operation nothing here had ever performed. Closing it immediately exposed a defect eleven queries deep: `status = 'current'` had been indistinguishable from "the memory's latest version" for as long as no memory had two |
| **5 · Console** — **done**, 6 of 6 plus one | ~~Vector coverage~~ · ~~jobs and dead letters~~ · ~~memory lifecycle~~ · ~~session replay~~ · ~~retrieval explain~~ · ~~config~~ · **+ retrieval configuration** *(unplanned)* | Every shipped figure already existed inside a command; the work was putting it where someone looks. The engine-console pages stay excluded — there is no such runtime here |

**One correction to an earlier count, now closed.** Wave 5 was once recorded as "4 of 6" on the
strength of the retrieval panel. That panel shows retrieval's *configuration*; **5.5 asked for
something different** — a per-query explain showing why *that* result came back. Both exist now, as
separate things (`ec30787` and `a4ce736`), so the wave is complete rather than complete by miscount.

---

## Every item, and where it stands

| Wave | Item | State | Evidence, or why not |
|---|---|---|---|
| 0.1 | Deliveries counted at receipt | **Shipped** `407d345` | An outcome dropped without recording leaves zero rows |
| **0.2** | **Consolidation drained** | **Quota-bound** | 1,933 pending across 3 projects; 290 of 294 deferrals were plain HTTP 429 |
| 0.3 | Queue and dead letters visible | **Shipped** `6d8c6cd` | Pending / leased / completed / dead per project, with the reason a job died |
| **1** | **Value proved** | **Half** | Retrieval measured on all 500 — 96.0% R@5. The token-saving A/B has never been run |
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
| **6.1** | **Mid-session push** (`UserPromptSubmit`) | **Shipped** `af81e4d` `4aafe32` | 400 tokens, four memories, twenty per session, metered. Claude only — Codex documents no such hook |
| 5.1–5.3 | Vector coverage · jobs · lifecycle panels | **Shipped** `89bd75a` `6d8c6cd` `7d12c15` | Per-project, live |
| — | Retrieval configuration panel *(unplanned)* | **Shipped** `a4ce736` | Each channel's weight beside whether it can fire. In-browser render unverified — see *Known gaps* |
| 5.4 | Session replay | **Shipped** `3a2c5b1` | `brain replay` lists sessions and walks one in order |
| 5.5 | Retrieval explain | **Shipped** `ec30787` | `brain explain` — per-channel rank beside the fused position |
| **5.6** | **Config panel** | **Shipped** `10b72f8` | Caught two things on its first live run — one of them a bug in itself |
| **6.2** | **Decay curve + access-strengthening** | **Shipped** `de9098a` | Ebbinghaus, with use buying survival |
| **6.3** | **`brain reconcile`** | **Shipped** `eec8925` | Proposes from authority → recency → evidence; refuses when level |
| **6.4** | **AI-first note format** | **Shipped** `6a1e34f` | A derived "For future agents" preamble, and `retention` in frontmatter |
| **6.5** | **Query expansion** | **Shipped** `6b431c0` | Pseudo-relevance feedback, fused as a channel so it cannot lose a result. Live: 0 lost, 1 newly reached |
| **6.6** | **Scheduled reflection** | **Shipped** `d4629d6` | `brain digest` daily via `AgentBrain.Digest`. Derived only, so a rate-limited provider cannot silence it |
| **7.1** | **Fold a contradiction** | **Shipped** `fbc6fe5` | `brain reconcile --apply`. Live: 4 folded, 7 superseded, 0 deleted. The first supersession this system has ever performed |
| **7.2** | **Revision trail on subject pages** | **Shipped** `fbc6fe5` | `subjects/backup.md` now opens with *"1 of these 35 claims has been revised in place, absorbing 4 earlier claims"* |
| **7.3** | **One definition of "current"** | **Shipped** `74423db` | Eleven queries disagreed with one. `digest` said 2,101 memories, `lint` said 2,090; both now say 2,090 |
| **1.2** | **Token-saving A/B harness** | **Built, not run** `6b07b18` | `scripts/token-ab.ps1`. Three conditions, not two — see *Known gaps* |

---

## Memory tiers

All four tiers exist. The lifecycle over them is where the remaining work is.

| Tier | Ours | State |
|---|---|---|
| **Working** — raw observations | `events`, 142,830 captured | ✅ |
| **Episodic** — session summaries | Boundary hook + `session_stopped` consolidation | ✅ shipped 8 Aug |
| **Semantic** — facts and patterns | `Fact`, `Decision`, `Investigation`, `Preference` | ✅ |
| **Procedural** — workflows | `Procedure`, `Task`, `Deployment`, `Checkpoint`, `Timeline` | ✅ |

Every memory additionally cites the events it came from, which no tier label supplies: a tier says
what kind of thing a memory is; a citation says whether it is true.

**What the lifecycle does and does not do:**

| | State |
|---|---|
| Access counting | ✅ per result, not per search |
| Derived staleness | ✅ 20 notes flagged, reversed the moment retrieval returns one |
| Gated eviction | ✅ refuses until thirty days of access data exist |
| Ebbinghaus decay curve | ✅ `retention_score` — continuous, and on every note's frontmatter |
| Access-strengthening | ✅ ten retrievals decay ~3.4× slower; logarithmic, so the tenth matters less than the first |
| Contradiction resolution | ✅ **proposed, then applied on request** — `brain reconcile` derives from authority, then recency, then evidence, refuses when level, and `--apply` folds what it decided |
| Revision in place | ✅ supersession, live for the first time — 7 memories retired into 4 claims, nothing deleted |
| Scheduled reflection | ✅ `brain digest` daily — never-retrieved share, retention distribution, contradiction and queue counts, appended to each vault's `log.md` |

The contradiction row is still a position, not a licence: *resolving* means deciding which claim is
true, and a model doing that leaves no evidence trail. What derivation does is narrower and
checkable — authority, then recency, then evidence weight, with the rule printed beside the
proposal — and `--apply` is the approval, not a schedule. Anything the rules cannot separate is
left alone.

What the four live contradictions turned out to be is worth recording: **not disagreements**.
Consolidation had re-derived the same conclusion from overlapping event windows and filed each one
as a new memory, so eleven memories carried four claims between them, phrased slightly differently.
That is a duplicate, and folding it is arithmetic.

The reflection row is deliberately arithmetic. A written weekly review needs a provider, and the
provider is the thing most likely to be unavailable — so the digest reports only what can be derived,
and therefore still reports during exactly the outage that makes it most useful.

---

## The four that are not simply "done"

**0.2 is quota-bound, not code-bound.** 1,933 jobs pending across three projects. The deferral path
works exactly as designed — no attempt consumed, nothing lost — and it finishes when quota allows.
There is no work here to do.

**1 is half measured.** Retrieval quality has a number and it reproduced exactly. The *token saving*
— the headline this project is usually asked about — has never been measured, and only the
numerator ever has. Design in [roadmap.md, Part 2](roadmap.md#part-2--proving-the-saving).

**3.2b is deliberately incomplete, and it is now the last unshipped feature of any wave.** 450
subject pages exist and every one of them only *lists*. What changed today is that the listing is no
longer the whole story: four pages carry a revision trail, because seven claims were folded into
four. **The vault has revised in place.** What it still cannot do is write a *paragraph* that
integrates them — that is 3.2b, and it needs a live provider. Supersession replaces a claim; it does not fold a new observation into an existing page.
The validator, store and rendering ship with 16 tests. The
generation call does not, and the reason is not effort: a subject page that only links *cannot*
contradict the ledger and a paragraph *can*, so the citation validation should be watched rejecting
a real bad citation from a real provider before prose reaches the vault. Building it against a stub
and shipping it unverified would invert the entire argument for having a validator.

**3.9 was dissolved, not skipped — and here is exactly what runs.** Two things carry the CodeGraph
name in this system and only one of them is live.

| | State | What it is |
|---|---|---|
| **CodeGraph, the MCP server** | **Running**, v1.5.0 | Registered in `~/.claude.json` *and* `~/.codex/config.toml` as `codegraph serve --mcp`. This repo indexed: **219 files, 3,466 nodes, 10,921 edges, 15.24 MB** SQLite in WAL, file watcher live. It also sits on `UserPromptSubmit` as `codegraph prompt-hook` |
| The brain's internal `codegraph` provider | **Built, disabled, correctly so** | `enabled: false · usable: false` on all three projects. It was designed to proxy the above as a seventh brain MCP tool |

So yes: pre-indexed code knowledge graph, over MCP, answering structural questions without grepping.
The routing works exactly as the pattern describes — *"where is `auth_check` defined"* goes to
CodeGraph, *"what did we decide about retries"* goes to the brain — and **that separation is the
integration**. Neither queries the other. Code structure is a live derivative of the working tree
and belongs to a file watcher; a decision is an append-only claim about the past and belongs to a
ledger. A proxy would build a second path to data both agents already reach directly, and it would
have to invalidate on every keystroke.

**On the token claim: unmeasured here.** CodeGraph publishes ~35% less cost and ~70% fewer tool calls
across seven repositories. Nothing in this system has measured that on this machine, and the number
should not be repeated as though it had been. It is also the reason the brain's own A/B needed
redesigning — see *Known gaps*.

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

### All 500 instances — the number that was missing

Run 9 August, 3 h 54 m, service stopped, harness pinned outside `target/` so nothing could relink it
mid-run. **23,867 sessions, 246,750 turns, 243,657 vectors** — a corpus 17× the one the 30-instance
run used.

| | Ours, all 500 | Published comparison |
|---|---|---|
| **R@5** | **96.0%** | 95.2% |
| **R@10** | 98.2% | **98.6%** |
| **MRR** | **0.922** | 0.882 |

**R@5 by question type**

| Category | Instances | Share | R@5 |
|---|---|---|---|
| single-session-assistant | 56 | 11.2% | **100.0%** |
| knowledge-update | 78 | 15.6% | 98.7% |
| multi-session | 133 | 26.6% | 97.0% |
| single-session-user | 70 | 14.0% | 94.3% |
| temporal-reasoning | 133 | 26.6% | 94.0% |
| single-session-preference | 30 | 6.0% | 90.0% |

**Read the caveats before quoting any of this.**

- **`single-session-preference` reproduced at exactly 90.0%** — identical to the 30-instance run,
  against a corpus seventeen times larger. That is the strongest stability signal in this table, and
  it is why the earlier number was worth trusting.
- **R@10 is behind**, 98.2% against 98.6%, and that is stated here rather than buried under the two
  wins. We rank better within the top five; they retrieve marginally more within ten.
- **This is the same dataset, not a controlled head-to-head.** Two independent harnesses computing
  the same metric over the same 500 questions is much closer than what we could say yesterday, and
  still not the same thing as one harness running both systems.
- **Query expansion was off.** The harness has no switch for it, so `6b431c0` contributed nothing to
  these numbers. Whatever it is worth at scale is unmeasured.
- **The cross-encoder was off**, as it should be — measured harmful on the preference category.

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

**Open findings that need a human.** `brain lint` now reports **0 contradictions** on this project —
all four were folded — plus **4 misdated** memories and **661 of 2,090 unlinked islands** (31.6%).

The daily digest surfaces the same findings across every project, where the numbers are considerably
worse. `Ai-community-channel` carries **74 contradictions, 9 of which derivation cannot separate**,
and **all 5,505** of its memories have never been retrieved. `brain reconcile --apply` would fold the
65 decidable ones there too; it has not been run, because that vault is not this one and the fold is
the operator's call per project.

A count alone cannot distinguish a neglected corpus from a young access counter, which is why the
digest says so in the note rather than colouring it red.

**The A/B design had a confound, and it would have credited the brain with someone else's saving.**
Part 2 specified two conditions: cold and warm. But **CodeGraph also sits on `UserPromptSubmit`**,
in the same settings file, firing on every prompt beside the brain hook. A cold/warm split therefore
measures *brain + CodeGraph* against *CodeGraph* and attributes the difference entirely to the brain.
CodeGraph's own published claim is ~35% less cost — the same order as anything the brain could show —
so the two are not separable after the fact. `scripts/token-ab.ps1` now runs **three** conditions,
and the brain's contribution is `(code − warm)`, never `(bare − warm)`.

**The A/B has not been run.** Two blockers, both stated rather than worked around: headless
`claude -p` returns **401 Invalid bearer token** from this environment, so it needs an authenticated
terminal; and the design's own precondition says a half-consolidated brain understates the warm
condition, with 1,933 jobs still queued. The harness itself is verified — a 15-session dry
execution switched all three conditions, spawned every session, parsed every result, and restored
`~/.claude/settings.json` to a byte-identical hash.

**The dashboard did not hydrate in the preview pane — now with a cause and a partial fix.** Every
panel renders its skeleton and never its data. Two separate things were found underneath, and neither
is a panel defect:

- **Local mode was 502ing.** `brain dashboard` takes **18 s** against the current brain — it sizes
  `BRAIN_HOME` and the backup root by walking them, so it grew with the corpus — against a 10 s
  `SPAWN_TIMEOUT_MS`. Every local request failed as `brain-unreachable`, which reads as a broken
  binary and sends you to the wrong repo. Raised to 45 s (`c75507c`); the API now returns 200 with
  the full snapshot, verified by `curl`.
- **The pane itself never composites.** With the browser pane hidden, the tab does not paint and the
  client-side fetch never fires — `performance.getEntriesByType('resource')` shows no request at all.
  Confirmed *pre-existing* by stashing every change and reloading: unmodified `HEAD` renders exactly
  the same empty panels.

Also worth knowing before debugging this again: `pnpm dev` reads the **Redis** snapshot, not the local
binary, because `.env.local` carries `KV_REST_API_*` and `chooseMode()` prefers remote when they are
set. A field added to the Rust struct will not appear locally until `scripts/push-snapshot.mjs` runs.

What *is* verified for 5.6: the API serves `config` end to end with correct live values, `tsc` is
clean, and the panel mounts in the right section with its nav entry. What is not: the rendered
pixels.

**Two memories the mid-session push surfaced are false.** `decay/tiers has no code at all` and
`this project uses embeddinggemma-300M and Qwen3-Reranker-0.6B` were both true when written and are
both wrong now. Note these are *not* what the fold resolved — folding merges re-derivations of one
claim, and these are single claims overtaken by events. Correcting one means filing a replacement
with `brain remember --supersedes`, which is a judgement about what is true now. They are among the four contradictions `brain lint` reports, and the push makes them
*louder* — an unsolicited injection of a stale claim costs more attention than a stale note nobody
opened. Resolving them needs your judgement about which side is true — item 5 in
[what to pick up next](#what-to-pick-up-next), and the daily digest now raises it unprompted.

**One number that could not be traced.** A 93.2% R@5 figure appears in an earlier roadmap and in no
document or commit. The recorded numbers are per-category: 63.3% → 90.0% on
`single-session-preference`. Those are not the same measurement and should not be quoted as one.

---

## What to pick up next

**Two items nothing is holding up, and three that wait on something.** Everything else on the
original nine-item list has shipped. Full reasoning and done-when criteria in
[roadmap.md, Part 1](roadmap.md#part-1--ranked-plan).

| # | Work | Blocked on | Size |
|---|---|---|---|
| ~~1~~ | ~~**Run all 500 LongMemEval instances**~~ — **done** 9 Aug. 96.0% R@5 / 98.2% R@10 / 0.922 MRR over 23,867 sessions in 3 h 54 m | — | — |
| 2 | **Run the token-saving A/B** — 5 tasks × **3** conditions × 3 repeats | **An authenticated terminal** (`claude -p` returns 401 here), then 0.2. The harness is built and verified | L |
| 3 | **3.2b** — the synthesis generation call | **Quota**, by choice: watching the citation check refuse a *real* bad citation is the point | M |
| 4 | **0.2** — drain the consolidation backlog | **Quota.** 1,933 pending; nothing to build | — |
| ~~5~~ | ~~**Resolve the 4 contradictions**~~ — **done** `fbc6fe5`. Folded, not decided: all four were re-derivations of one claim, which is arithmetic | — | — |
| ~~—~~ | ~~Mid-session push~~ `af81e4d` · ~~AI-first notes~~ `6a1e34f` · ~~contradiction proposals~~ `eec8925` · ~~query expansion~~ `6b431c0` · ~~scheduled reflection~~ `d4629d6` · ~~5.6 config panel~~ `10b72f8` · ~~the fold~~ `fbc6fe5` | — | — |

### What the 500-instance run settled, and what it did not

**Settled.** Every competitive claim about retrieval was speculation while 92% of the dataset had
never been run. It has now: 96.0% R@5 pooled, and the category we had been reporting —
`single-session-preference` — came back at **exactly 90.0%**, reproducing the 30-instance figure
against a corpus seventeen times larger. Two independent measurements agreeing to the decimal is the
reason to believe either.

**Not settled.** The run predates query expansion having any effect — the harness has no switch for
it — so `6b431c0` is still unmeasured at scale. And R@10 came back *behind* the published
comparison, 98.2% against 98.6%, which is the honest counterweight to the two wins: this system
ranks better inside the top five and retrieves marginally less inside ten.

### Item 2 shipped — what it took, and what it revealed

The brain used to push exactly once, at session start. It now re-queries retrieval on every prompt
and re-injects when the subject moves. Live: a question about the rerank benchmark returned four
cited memories at 324 tokens, with the meter reporting two dropped over budget.

Most of the work was **restraint**, because a push on every message has the opposite failure mode
from a push on none:

| Guard | Why |
|---|---|
| Never repeat a memory; twenty per session | Re-injecting what the model already has spends the budget hardest exactly when the topic is *not* moving |
| Silence for short prompts, no session id, or no new match | "ok" and "continue" share no vocabulary with anything specific, so retrieval returns whatever is generally popular |
| **A stricter floor than search uses** | FTS terms are OR-joined — measured, a question about unladen swallows retrieved a database-migration memory and would have injected it |
| 400 tokens, four memories, and a meter naming what it dropped | Session start gets 1,000–1,500 and fires once; this fires every message |

**Two things live verification caught that the tests could not.** Repeating a prompt surfaced the
*next* four memories rather than the same four — correct behaviour, and unbounded, so a per-session
cap was added afterwards. And the push surfaced two memories that are now **false**, which is exactly
the counter-metric the benchmark design names: a memory system that misleads with stale context. See
*Known gaps*.

**The floor is a keyword floor**, deliberately conservative. It will miss a genuinely relevant memory
that shares no vocabulary — the same gap the cross-encoder failed to close. Query expansion is what
removes the handicap, which is why it is next.

### Query expansion shipped — what it does, and the property that matters

`6b431c0`. Pseudo-relevance feedback: take the top five hits, harvest terms appearing in at least two
of them, append the best six, retrieve again. Terms are selected by **document frequency**, not raw
count — a word repeated ten times in one document is about that document; a word in two is about the
subject.

The property worth more than the gain: expansion is **fused as an extra channel**, never a rewrite.
A rewritten query can drop the very document that seeded it. A fused one cannot, and a test pins it.
Live on this corpus: **0 results lost, 1 newly reached.**

It is the measured answer to the gap the cross-encoder failed to close — `ship to production` and
`deploy` are the same claim and score ten points apart, and no scoring model can invent that link
because the link is in the corpus, not in the model.

### The fold shipped — and what it broke on the way through

`fbc6fe5`. The Karpathy operation nothing here had ever performed: *"doesn't just index it for later
retrieval — it integrates it into the existing wiki."*

The gap was never really *"pages are not revised"*. It was that **claims** were not revised.
Consolidation re-derives the same conclusion from overlapping event windows and files each as a new
memory, so the vault accumulated duplicates and `brain lint` called them contradictions. Two appends
per fold, nothing deleted: a new version of the keeper carrying the supersession edges, and a
retirement version per loser. 2,097 records before and after; 2,097 versions became 2,108.

**Then it exposed a defect eleven queries deep.** `brain digest` reported 2,101 memories where
`brain lint` reported 2,090. Eleven queries selected versions by `status = 'current'`, which had been
indistinguishable from *"the memory's latest version"* for as long as no memory had two. A fold
breaks the equivalence in both directions at once — the keeper gains a second current version and is
counted twice, the loser keeps its original and never leaves. The vector backfill had been counting
retired claims as work to do, eviction had been scoring them as candidates, and the retention curve
had been averaging them into the health number the daily digest reports. Fixed in `74423db`; the
predicate now lives once, as `CURRENT_CLAIM`, and both commands say 2,090.

This is the third time in this project that a defect survived because production had never exercised
the other branch — after the half-life that was a time constant, and the session replay that reused
a project-wide query. It is worth expecting a fourth.

### Scheduled reflection shipped — and why it has no model in it

`d4629d6`. `AgentBrain.Digest` runs daily and appends a health reading to every registered project's
vault `log.md`: never-retrieved share, retention distribution, contradictions, queue depth. Verified
end to end — the task ran with result 0 and the entries are on disk.

The competitor pattern here is four scheduled agents and a claim that the vault maintains itself.
Ours consolidates continuously already, so the missing half was never the *schedule* — it was that
nothing ever stepped back and asked whether what had been built was still coherent. A written review
needs a provider, and the provider is the thing most likely to be down; the reading a maintainer
actually acts on is arithmetic. So it is arithmetic, and it still reports during an outage.

It reports and stops. An unremarkable brain gets `Nothing needs a decision.`, and a test pins that —
a health report that manufactures concern to justify itself is one nobody reads twice.

---

## Where to look next

| Question | File |
|---|---|
| Why each remaining item is ranked where it is | [roadmap.md](roadmap.md) |
| How the system fits together | [architecture.html](architecture.html) |
| How to register a project | [registering-a-project.md](registering-a-project.md) |
| Storage sizing and retention | [storage-and-backup.md](storage-and-backup.md) |
