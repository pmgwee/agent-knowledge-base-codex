# Enhancement review — the plan, and what it is answering

Written 9 August 2026, after reading Karpathy's *LLM Wiki* against what this system actually does.
Updated the same evening, after the first half of it shipped and two of its assumptions turned out
to be wrong.

The other two documents answer *what is running* ([status.md](status.md)) and *what is left and why*
([roadmap.md](roadmap.md)). This one is narrower and has a shelf life: it is the working plan for one
round of enhancements, with the reasoning that produced it, so that a week from now it is possible to
check whether the reasoning held.

---

## Status board — every item, current as of 10 August 2026

**✅ shipped · ⚠️ open · ~~struck through~~ = the item is resolved and no longer applies.**

**Everything still open is gated on one thing:** this project's consolidation queue reaching zero.
Nothing that can be built without spending quota is waiting on it — see
[What is left, in the order it has to happen](#what-is-left-in-the-order-it-has-to-happen).

### The original seven, plus what the round added

| # | Item | Status | Where it stands |
|---|---|---|---|
| **A1** | `push-snapshot.mjs` 10 s → 45 s | ✅ shipped | `6e006c0`. `brain dashboard` takes 18 s, so every push timed out and Redis served a pre-Config snapshot. Config panel renders |
| **A2** | Architecture diagram — split recency-orientation from fused-push | ✅ shipped | `4c213f3`. It taught that RRF feeds session-start. It does not |
| **A3** | Harness named on delivery metrics | ✅ shipped | `33684d4`. One green badge covered two agents |
| **A3b** | Harness split in the dashboard JSON | ✅ shipped | `ee357eb`. Six harness/hook channels including the zeros — and it found four channels that recorded nothing |
| **A4** | Capture-staleness detection | ✅ shipped | `4c213f3`. A 100-minute stall with every panel green |
| **A5** | The file-back loop | ✅ shipped | `4c213f3`. `--evidence` optional, citations derived. **7+ human claims filed since**, against 0 in the two months before |
| **A6** | Search on the Retrieval panel | ✅ shipped **and rendered** | Verified in a real browser 10 Aug: live query, three memories above every event, per-result fusion explanations |
| **A7** | Session replay UI + inline citations | ✅ shipped | `fbb76d0`. Could not be built as designed: a session is 100 MB whole, so it pages |
| **A8a** | Cross-claim revision — detection | ✅ shipped | `5bd4c49`. Grouping by shared evidence after two groupings failed |
| **A8b** | Cross-claim revision — the rewrite | ✅ shipped, generation run | Both validator rules observed refusing real provider output. **⚠️ blanket `--apply` still not run** — and no longer the only option, since A9 makes it per-pair |
| **A9** | Human checkpoint on consolidation | ✅ **shipped, both halves** | `2750c29` gates the rewrite path per-pair; `8ddb828` gates ingest via `MemoryStatus::Proposed` + `brain review`. Off by default — gating a kind is a commitment to draining the queue |
| **A10** | Does Codex fire hooks? | ✅ settled | **Yes — all three.** The fork in the road, resolved |
| **A11** | Orientation compile too slow for the hook budget | ✅ closed | `35ef05f`. One absent index: 42,001 ms → 34.1 ms |
| **A12** | One slow hook starved every hook behind it | ✅ shipped | `fc4e897`. The accept loop awaited the handler, so a 6,571 ms cold-cache compile cost three sessions their orientation, not one |
| **A13** | Consolidation ran one provider call at a time | ✅ shipped | `3b33bcd`. Three ledgers now drain concurrently, one call each. Measured **~60 → ~215 jobs/hour** idle; this project's own queue goes ~30 h → ~7 h |
| **A14** | The A/B's second and third conditions measured the first | ✅ shipped | `e9b7bb7`. `Set-Condition` read the file it had just stripped. Plus shuffled order, session-id manifest, blind grading sheet |

### The Codex work, which was the fork in the road

| Item | Status | Where it stands |
|---|---|---|
| ~~Codex never fires hooks; MCP is its ceiling~~ | ~~resolved~~ | **Falsified.** Our `commandWindows` quoted the executable and Codex does not strip quotes, so the hook exited 1 before reaching our binary |
| ~~Un-register the Codex hooks so the dashboard stops claiming a feature that does not run~~ | ~~resolved~~ | Not done, and correctly not done — the feature runs |
| ~~Codex mid-session parity is structural~~ | ~~resolved~~ | **Falsified.** `UserPromptSubmit` is registered and observed firing four times in one Codex session |
| `SessionStart` on both harnesses | ✅ shipped | Harness-invoked, before the model reads anything |
| `SessionEnd` on both harnesses | ✅ shipped **and observed** | Real `session.ended` events from Codex. Codex clamps the timeout to 3 s and warns; the installer now declares 3 |
| `UserPromptSubmit` on both harnesses | ✅ shipped **and observed** | `eb67502`. Four Codex turns carry the pushed text in their own transcript |
| MCP demoted from delivery to depth | ✅ shipped | `[mcp_servers.brain]` **kept** — `brain_search`, `brain_timeline`, `brain_evidence`, `brain_claims`, `brain_leases` answer questions a push cannot anticipate |
| `AGENTS.md` brain block removed from every registered project | ✅ shipped | All three rewritten. The `brain_checkpoint` instruction is gone; **no per-project step remains** |
| `docs/registering-a-project.md` step 3 | ✅ shipped | Now reads *"Nothing."* |

**Two entries in the day's recap are struck through above because they were overtaken by evidence
the same day.** They are kept rather than deleted: "Codex hook test ✅ settled — never fires on
Desktop" was the confident wrong conclusion, and the reasoning that produced it — *zero deliveries
and zero spool entries must mean it was never invoked* — is sound and does not apply, because a hook
that cannot **launch** neither delivers nor spools.

### Each hook is proven by a different instrument — and looking at the wrong one cost a wrong answer twice

`context_deliveries` is the right evidence for `SessionStart` and nothing else. Reading the
delivery table and finding zeros for the other two produced "registered but not yet observed",
which was false for both. The same shape as the original mistake: **absence in the wrong table is
not evidence of absence.**

| Hook | Right instrument | Why | Codex evidence |
|---|---|---|---|
| `SessionStart` | a `context_deliveries` row | it pushes an orientation, so a receipt is the point | ✅ **4 real deliveries**, v7 session ids `019fe5f9…`, `019fe5ff…`, `019fe5b6…` ×2, 1,030–1,041 tokens |
| `UserPromptSubmit` | the pushed text **inside the harness's own transcript** | it pushes only when it has something to say, so silence is normal and a zero proves nothing | ✅ **4 Codex turns** carrying `Related memory for this turn:` — the push landed in Codex's own conversation |
| `SessionEnd` | a `session.ended` **event** | it usually pushes nothing at all; a delivery row would be the wrong receipt | ✅ **real `session.ended` events**, v7 ids, most recent `019fe790…` |

**The discriminator throughout is `native_session_id`.** A diagnostic run carries `diag-…` or
`probe-…`; a genuine session carries the harness's own id. Every row above is a genuine one — and a
first pass at this used `MAX(native_session_id)`, which sorts `diag-codex-1` above every UUID and
made the real deliveries invisible.

#### One instrument replaces all three — `bda5a2f`, 10 August

The table above needs a different receipt per hook, and getting the pairing wrong is what produced
every wrong conclusion in this document. That is a design flaw in the *measurement*, not in the
hooks. `crates/brain-service/src/pipe.rs` now logs one `info` line for **every** hook arriving at
the pipe — harness, event, session — before any decision about what to reply:

```bash
grep 'hook received' ~/AgentBrain/runtime/logs/brain-service.$(date +%F).jsonl
```

**All three Codex hooks, one live session, first use of the instrument** —
`019fe7c5-698c-7c63-b4bd-57c66056f628`:

| Time (UTC) | Event | Delivery row |
|---|---|---|
| 18:25:02 | `SessionEnd` | — (prior session `019fe7c5-4aee` closing) |
| 18:25:05 | `SessionStart` | ✅ 1,045 tokens, 46 coordination |
| 18:25:06 | `UserPromptSubmit` | **none** |
| 18:29:37 | `UserPromptSubmit` | ✅ 358 tokens, all coordination |

**Two invocations, one delivery row** — the ambiguity, caught on the instrument's first outing. The
first `UserPromptSubmit` fired 0.9 s after `SessionStart`, when the orientation had just gone out
and there was nothing to add, and correctly returned nothing. Counting deliveries reports "1 of 2
prompts"; in a session where both are quiet it reports zero, which is indistinguishable from a dead
hook. Three states that used to look identical:

| `hook received` | Delivery row | Meaning |
|---|---|---|
| absent | absent | The harness never invoked it — check `[hooks.state]` trust first |
| present | absent | Fired and had nothing to say. **Healthy.** |
| present | present | Fired and pushed |

**A fourth wrong conclusion, on 9 August, which this closes.** A test read
`UserPromptSubmit` as "did not fire" from a delivery count taken **36 seconds before** the row
appeared — in a window a hand-fired diagnostic was itself writing into. Polling a table you are also
writing to cannot establish who wrote the row, and no amount of care in reading it fixes that.

Trust survives a deploy: `~/.codex/hooks.json` is untouched by `scripts/deploy.ps1`, which replaces
binaries only, so the three `[hooks.state]` hashes stay valid. Editing the hook file is what revokes
trust.

### The quota-blocked four, on the day quota returned

| Item | Status | Where it stands |
|---|---|---|
| ~~A8b's generation call~~ | ✅ **run 10 August** | Both rules caught refusing real output: `LostNewEvidence` and `ForeignCitation` |
| ~~3 dead-lettered jobs~~ | ✅ **requeued** | Through `brain jobs --retry-dead`, which did not exist — the digest reported the count and nothing could act on it |
| ~~3.2b subject-page prose~~ | ✅ **shipped** | `brain synthesize`. 63 pages carry cited prose, ⚠️ 93 held back until the drain finishes — their prose is keyed to an exact memory set that consolidation is still changing |
| Consolidation backlog | ⚠️ **draining, ~3x faster since A13** | 2,191 → 1,834. `Ai-community-channel` is done; this project is 48% consolidated with ~7 h of uptime left, independent of `subscription-agent`'s 1,199 |
| Token-saving A/B | ⚠️ **not run, by decision** | Never blocked on a second quota — `claude -p` reports `glm-5.2`. Blocked on its own precondition: a half-consolidated brain understates the warm condition |

### Against Karpathy's pattern — the two skips that were wrong

| His element | Then | Now |
|---|---|---|
| **Ingest = integrate into existing pages** | ❌ we appended, never integrated | ✅ **`brain revise` merges; `brain synthesize` writes the page's prose.** The machine now integrates |
| **Query = file answers back** | ❌ 0 of 13,493 | ✅ **`brain remember` with derived citations.** 7+ filed, including today's three retractions |
| Human in the loop on ingest | ⚠️ debatable | ⚠️ still open as **A9**, and A8b's false merges are the argument for it |

The other six skips stand as correct and unchanged.

### Found while doing the above — none of it was on the plan

| Item | Status | What it was |
|---|---|---|
| Retired claims still served | ✅ fixed `7a75da6` | Two ways to retire a claim, one honoured. 30 retracted claims were still reaching the orientation, the vault and `brain export` |
| 26 claims asserting a disproved belief | ✅ retracted | The vault told any agent, 26 ways, that Codex does not fire hooks |
| `UserPromptSubmit` / `SessionEnd` recorded nothing | ✅ fixed `ee357eb` | They pushed context and staged no delivery, so four channels read zero however well they worked |
| One timeout discarded seven finished merges | ✅ fixed `650b978` | `?` propagated a transient provider failure through the whole run |
| No CLI could retry a dead letter | ✅ fixed `50547dd` | `brain jobs` |
| Nothing timed the orientation compile | ✅ fixed | Three `tracing::info!` lines; they ended a diagnosis that three rounds of reasoning got wrong |


---

## The finding that shaped everything below

Measured against the pattern, this system skipped **eight** of its elements. Six of those skips were
correct. Two were the same mistake seen from opposite sides.

| Karpathy's element | Us | Verdict |
|---|---|---|
| Obsidian Web Clipper | ❌ | ✅ Right — our sources are transcripts, not web articles |
| Local image download | ❌ | ✅ Right — there are no images in a transcript |
| Marp slide decks | ❌ | ✅ Right — our output is agent context, not a presentation |
| Dataview | ❌ | ✅ Right — the frontmatter is there if it is ever wanted |
| Vault as a git repo | ❌ | ✅ Right — the ledger is the source of truth |
| `index.md` as primary navigation | ⚠️ built, unused by retrieval | ✅ Right — he says it *"avoids embedding-based RAG"* at ~100 sources; we are at 144k events |
| **Ingest = integrate into existing pages** | ❌ | ❌ **Wrong. This is his central claim** |
| **Query = file answers back** | ❌ 0 of 13,493 | ❌ **Wrong. Same claim, other side** |
| Human in the loop on ingest | ❌ | ⚠️ Debatable — see A9 |

**The machine never integrated, and the human never filed.** That is why the vault accumulated
instead of compounding, and it is the single sentence this plan exists to make false.

### Why the second one had never happened

`brain remember` shipped in June. Across 13,493 memories, **not one carried `human_correction`
authority.** Every claim was machine-derived from a transcript.

That was not apathy. The command demanded `--evidence event:019fe4d2-c168-7882-…`, and nobody can
know a UUID by hand. **The interface made the operation impossible, so the operation never
happened** — and everything worked out in conversation evaporated when the session closed.

---

## Shipped in this round

| # | Item | Commit | What it changed |
|---|---|---|---|
| **A5** | **The file-back loop** | `4c213f3` | `--evidence` is now optional; citations are derived by running the claim's own text through the fused retrieval measured at 96.0% R@5, and printed for checking. Refuses rather than invents when retrieval finds nothing. **Six human claims filed on the day it shipped** |
| **A8a** | **Cross-claim revision — detection** | `5bd4c49` | `brain revise`. Finds older claims a later one appears to complete. Two groupings failed first; see below |
| **A2** | Architecture diagram corrected | `4c213f3` | It drew RRF fusion feeding the session-start orientation. The code does not: `ContextCompiler::from_ledger` reads `recent_events()` chronologically, and fusion feeds the **mid-session push** (`hook_handler.rs:421`). Split into two boxes |
| **A4** | Capture-staleness detection | `4c213f3` | Capture stalled 100 minutes with the service running and every panel green, because nothing compared the newest event to the wall clock. The digest now leads with it |
| **A1** | `push-snapshot.mjs` 10 s → 45 s | `6e006c0` | ✅ **Verified rendering.** `brain dashboard` takes 18 s, so every push timed out and Redis served a pre-Config snapshot. Pushed a fresh 13,941-byte payload; the Config panel now renders on the deployed dashboard |
| **A3** | Harness named on hook metrics | `33684d4` | One "SHIPPED · 75 delivered" badge covered two harnesses, one of them at zero for all three hooks |
| **—** | **Codex hook test** | `33684d4` | Settled. See below |

---

### A3b · The harness split — shipped `ee357eb`, and it found something on its first run

`context_delivery_summary` counts every row for a project, so one live harness covers for a dead one
and three hooks average into a number belonging to none of them. The snapshot now carries
`delivery_channels`: all six harness/hook pairs over 7 days, **including the pairs with no rows**.
The expected list is hardcoded rather than derived from what the ledger holds — a harness that
stopped has no rows and would otherwise vanish from the panel instead of reading zero.

It immediately found a gap, and the gap was ours rather than Codex's:

| Channel | Deliveries, 7 d |
|---|---|
| `claude-code` / `SessionStart` | 21 + 77 + 2 across three projects |
| `codex` / `SessionStart` | 6 |
| **`*` / `UserPromptSubmit`** | **0** |
| **`*` / `SessionEnd`** | **0** |

Both branches returned `HookOutcome::bare` — they push lease warnings and mid-session context and
recorded **none** of it. Every project would have read zero on those four channels however well they
worked. Both now stage a delivery when they actually return text, and only then: a push that returns
nothing is not a delivery.

**Verified by firing both hooks by hand against a registered project.** `UserPromptSubmit` pushed
1,030 characters in 0.13 s and its delivery row went 0 → 1. `SessionEnd` pushed nothing — no lease
was outstanding — and recorded nothing, which is the designed behaviour rather than a failure. So the
four zeros are a **measurement gap now closed, not four dead hooks**: all three hooks are registered
on both harnesses, and all three Codex hooks are trusted in `[hooks.state]`, both re-checked.

### A7 · Session replay and clickable citations — shipped `fbb76d0`

The route could not be written as planned. `brain replay --session --json` returned `StoredEvent`,
which carries the harness's original record beside the normalised one — correct for a ledger,
unusable on a wire. **The largest captured session here is 16,148 events and serialises to 100 MB.**

So two shapes rather than one. The list is a projection — a preview per turn, `raw` dropped — which
takes 100 turns to 32 KB, with `--limit`/`--offset` paging it. Opening a turn fetches that one event
whole, and `--event <uuid>` answers that directly. Which is exactly what a citation needs, so the
same control serves both: `CitedEvent` is a disclosure, not a link, and it appears on every event hit
in search results as well as on every turn in replay. A citation into another project's ledger
answers `null` → 404 → "not held here", never a cross-project read.

The part that had to be measured rather than assumed was extracting the turn text. The first version
read `payload["content"].as_str()` and rendered blank rows for the 818 most recent `agent.responded`
events, because claude-code stores those under `message.content` as a **block array**. Six shapes are
present across the two harnesses; they are tabled in the doc comment with which events use each.

---

## The Codex question — settled, and the cause was ours

**Both harnesses are pushed to.** Codex Desktop dispatches `SessionStart`, `SessionEnd` and
`UserPromptSubmit`, harness-invoked, before the model reads anything — the same guarantee Claude
Code has always had. Verified 9 August 2026 on Desktop `26.803.41515` / CLI `0.147.0`.

### The bug was one pair of quotes, in our installer

```jsonc
"commandWindows": "\"C:\…\brain-hook.exe\" --harness codex"   // hook exited with code 1
"commandWindows": "C:\…\brain-hook.exe --harness codex"        // runs
```

Codex does not strip quotes from `commandWindows`. `install_codex_hooks` wrote the quoted string to
**both** `command` and `commandWindows`, so every Codex install this tool ever produced carried a
hook that could not launch. Fixed; `command` keeps its quotes for POSIX, `commandWindows` drops
them, and the installer now refuses an executable path containing a space because the unquoted form
cannot survive one. Pinned by `the_windows_command_is_unquoted_and_the_posix_one_is_not`.

The second gate, once the command runs: Codex records a SHA-256 per hook in `~/.codex/config.toml`
under `[hooks.state]` and will not invoke an untrusted one. **Editing a hook revokes its trust**, so
re-approve after any reinstall.

### Why it hid for five days, and what to check next time

A hook that cannot launch delivers nothing **and spools nothing** — indistinguishable from never
being invoked. We reasoned that a hook which fired and failed to deliver would still spool, which is
true and did not apply, and concluded first that Codex Desktop does not implement hooks, then that
`openai/codex#21639` was responsible, recording a matching build number. **A guess with a citation
looks like a diagnosis.**

Order of checks, now written into both schema files: **the command line first**, then `[hooks.state]`,
then the spool, then the deliveries table, and only then an issue tracker.

### MCP stops being a delivery path

`brain_checkpoint` via `AGENTS.md` was never wiring — it was a *request* that the model call a tool,
with three failure modes a hook does not have: it may not read the file, may read it and skip the
call, or may call it after it has already started reading the codebase, which is the cost the
orientation exists to avoid. None are visible from outside.

**Removed:** the per-project `AGENTS.md` brain section, from `docs/registering-a-project.md`, both
mirrored sections, and **all three registered projects** — `agent-knowledge-base-codex`,
`Ai-community-channel` and `subscription-agent`.

That last part was claimed here before it was true. On 9 August only this project's block was
rewritten; the other two still carried the `brain_checkpoint` instruction verbatim until 10 August,
which means two of three registered projects spent a day being told to call a tool for context the
harness had already pushed. **No per-project step remains** — `brain register <path>` and a service
restart is the whole procedure.

**Kept:** `[mcp_servers.brain]`. The hook *pushes* an orientation; the tools *answer questions* —
`brain_search`, `brain_timeline`, `brain_evidence`, `brain_claims`, `brain_leases` have no hook
equivalent and never will, because nothing can push an answer to a question not yet asked. MCP went
back to being depth on demand, which is what it was always good at.

### A10 — answered. Codex fires all three hooks

Tested 19:00 with a fresh Codex Desktop session in `subscription-agent`, two prompts:

| Hook | Invocations |
|---|---|
| `SessionStart` | 2 |
| `UserPromptSubmit` | **4** |

**Codex fires `UserPromptSubmit`.** The parity gap this document called structural throughout was
one untested registration, and it is now registered and observed. Both harnesses receive all three
hooks, harness-invoked, before the model reads anything.

Two operational details, both from the operator and both now in the installer:

- **`SessionEnd` must declare a 3-second timeout.** Codex clamps anything larger and warns
  (`⚠ clamping SessionEnd hook timeout to 3s`). The behaviour is right — a session that is *ending*
  cannot be kept waiting — but emitting a number the harness overrules lets the config drift from
  what actually runs. `CODEX_SESSION_END_TIMEOUT_SECONDS = 3`.
- **Editing a hook revokes its trust.** Codex keys `[hooks.state]` on a SHA-256, so any reinstall
  requires re-approval at the CLI TUI. Claude Code has no equivalent gate — its hooks run from
  `~/.claude/settings.json` with no review step, which is why that side has never needed one.

### ✅ A11 · Closed — one absent index cost 42 s, and the "superlinear in memory count" framing was wrong

**Symptom:** two of three registered projects delivered no orientation. The hook gave up at its 3 s
ceiling, the service kept compiling for another 36 s behind it, and the log recorded
`hook pipe request failed / write hook reply` — which reads as a broken pipe and was a client that
had already left.

**Cause:** `memory_supersession`'s primary key indexes it by the *superseding* version, and every
read asks the opposite question — "has this version been superseded?" With no index on
`superseded_version_id` that is unreachable by lookup, so SQLite drove the subquery from
`memory_versions` instead — `SEARCH newer USING INDEX idx_memory_versions_validity
(valid_from_ns<?)`, once per candidate row. Cost scales with **versions × matched rows**, which is
why it was invisible on small projects and took the whole budget on the largest.

Measured on the live 5,669-version ledger, same query, identical results. The index builds in 3 ms.

| | before | after |
|---|---|---|
| memories keyword query | 42,001 ms | **34.1 ms** |
| session-start hook, `Ai-community-channel` | timeout, 0 delivered | **0.74–0.82 s** |
| session-start hook, `subscription-agent` | timeout, 0 delivered | **0.69–0.79 s** |
| session-start hook, `agent-knowledge-base-codex` | 1.5 s | **0.31–0.89 s** |
| `brain explain`, `Ai-community-channel` | 82,809 ms | **1,755 ms** |
| `brain explain`, `subscription-agent` | 91,883 ms | **3,089 ms** |

All three projects now deliver on every round, three rounds running. `ee357eb`, `35ef05f`.

#### The framing was wrong, and it cost two rewrites

"Superlinear in memory count" survived three rounds of measurement because it kept nearly fitting.
It was falsified twice and re-asserted anyway:

| Round | Reading | What it should have said |
|---|---|---|
| First | `Ai-community` 5,505 → 38.5 s; `subscription` 5,644 → 41.7 s | Consistent with memory count |
| Second | `subscription` 5,644 → **1.0 s**; `Ai-community` 5,505 → 39 s | **Falsified.** Nearly equal corpora, 39× apart |
| Third | `subscription` 5,749 → **0.7 s**; `Ai-community` 5,445 → 39 s | Falsified again |

The discriminator was never the corpus. It was whether the query text was non-empty:
`rank_memories_against_recent_work` returns early when the recent turns yield no text, and
`subscription-agent`'s fast runs were **all** empty-query runs. A project looked healthy because it
was skipping the work, not because it was doing it quickly.

**Two of the three fixes attempted were wrong about this, and both were real defects worth keeping:**

- `current_project_memories` ran ~4 queries **per memory version** — about 23,000 round trips at
  5,505 memories — and is now four queries flat. It was 14.3 s of the load on one project. Pinned by
  `crates/brain-store/tests/memory_bulk_load.rs`, which asserts it returns exactly what the
  per-memory path returned: this list is what `resolve_candidates` closes over for supersession and
  contradiction, so a dropped edge would surface as a confident wrong orientation, never an error.
- `record_memory_access` committed **once per id** — up to 64 write-lock acquisitions per search
  against a database the service writes to concurrently, each willing to wait out the 1 s
  `busy_timeout`. Now one transaction.

Neither moved the number.

#### What actually ended it was instrumentation, not reasoning

Nothing timed the compile, so three rounds of diagnosis were guesses dressed as deductions —
including one that drove the named pipe by hand from three languages because no CLI compiles an
orientation. Three `info` lines settled it:

| Line | Fields | Answered |
|---|---|---|
| `orientation compiled` | `open` / `live_state` / `load` / `compile` | The compile is 118 ms; the *load* is 14 s |
| `orientation material loaded` | `events` / `ranking` / `stale` / `memories` | The load is one search, not the memory fetch |
| `search channels` | `events` / `memories` / `vector` / `expansion` / `graph` | The search is one keyword channel, 12.6 s |

From there, `EXPLAIN QUERY PLAN` named the clause in one call. **The 42 s was one log line away the
whole time**, and the lesson generalises past this bug: `HOOK_HARD_TIMEOUT` was a reasonable number
that went silently wrong, and a stage nobody times is a stage nobody can be right about.

The regression is pinned by **plan, not duration** — `crates/brain-store/tests/search_query_plan.rs`
asserts the filter reaches `idx_memory_supersession_superseded` and never sweeps `memory_versions`.
A timing assertion would be flaky on a fixture and silent on the only shape that matters.

### ✅ A12 · Closed — one slow hook was costing every hook behind it

A11 made the compile fast. It did not make a *slow* compile harmless, and on 10 August the service
restarted into an empty OS file cache and produced one:

```
00:50:35 orientation compiled  open_ms=2172  live_state_ms=840  load_ms=3471  total_ms=6571
00:50:35 WARN hook pipe request failed  error="write hook reply"   ×3
```

**Three failures, one slow request.** `pipe.rs` created the next pipe instance before serving the
current one — so a second client could *connect* — and then `await`ed the handler in the accept
loop, so nobody read that client's request until the first finished. Connected and served are
different things, and from the far side of the pipe they are indistinguishable right up until
`HOOK_HARD_TIMEOUT` expires. The queued hooks timed out having done nothing wrong; their replies
were then written to pipes whose clients had gone, which is what `write hook reply` means.

Two changes, `fc4e897`:

- **Each request runs on its own task**, and the accept loop returns immediately to waiting.
  `max_instances` goes 2 → 16, since a queue of one only ever sufficed while every request was fast.
- **`ProjectHookHandler::warm`** makes the same two reads `from_ledger` makes — `recent_events` at
  `ORIENTATION_EVENT_LIMIT`, then `current_project_memories` — once at startup on the blocking pool,
  so the cold-cache cost lands on nobody's session. It deliberately does not block the pipe coming
  up: a hook that finds no pipe gets nothing *immediately*, which is worse than one that finds a
  slow pipe.

**The test is the reason to believe the first half.** `a_slow_request_does_not_starve_the_one_behind_it`
holds a 1,500 ms request open and gives a second one a 700 ms budget. Against the old loop it fails
with the production error verbatim — `hook request exceeded hard timeout`. A concurrency test that
passes both before and after pins nothing.

**The second half is reasoned, not yet measured.** Post-deploy the warm-up ran in 255 / 538 / 615 ms
and a probe compiled in 333 ms (`open_ms` 2, `load_ms` 212, against 2,172 and 3,471 cold) — but the
OS cache was already warm from the previous process, so that probe does not isolate the warm-up's
contribution. Only a genuine cold boot can, and the honest claim until then is that the warm-up runs
and touches exactly the pages the hook path needs.

---

## How this round is verified — and the one thing that cannot be

**Falsified 10 August — and the cause was never what the elimination concluded.** The claim below
was that rendered UI cannot be checked from this session; the Retrieval panel has now been verified
in a real browser via the Playwright MCP, with a live query returning three on-topic memories above
every event and a fusion explanation on each result.

What is true is narrower: **the Browser *pane* cannot composite**, so `computer{action:"screenshot"}`
against it times out. Everything downstream of that was misattributed. In the pane React never
hydrates — no fiber keys, 59 server-rendered skeletons — so no client component ever fetches, and
`Reveal`'s `whileInView` never fires, leaving every panel at `opacity: 0`. Three separate symptoms,
one cause, and none of them means the app does not render.

The lesson is the same one the hook investigation produced: *"cannot be verified"* is a claim about
the instrument, and reaching for a second instrument is cheaper than three rounds of elimination.

The original reasoning, kept:

Established by elimination, three times over: **rendered UI cannot be checked from this session.**

The Browser pane is never displayed, so `document.hidden` is `true`, nothing paints, and the
`Reveal` wrapper's `whileInView` — an IntersectionObserver — never fires. React hydrates; the panels
never populate. Confirmed on the local dev server, on **unmodified `HEAD`** with every change
stashed, and on the deployed Vercel site. It is the environment, not the code.

So the split for anything with a user interface:

| Verified here | Verified by you |
|---|---|
| The API route returns correct values — checked with `fetch` against the live deployment | That it renders |
| The CLI's JSON shape backing it | Spacing, dark mode, layout |
| TypeScript, lint, and the SSR DOM structure | "this reads wrong" |

**One screenshot per feature, after it is built.** Before is no signal — the panel does not have the
feature yet. This is the shape every successful verification in this round already took.

---

## Open — detail behind the board above

### ✅ A8b · Cross-claim revision — the rewrite

**Detection shipped (`5bd4c49`). The rewrite is designed and buildable today except the generation
call.**

#### What detection found, and the two groupings that failed first

| Grouping | Result | Why it failed |
|---|---|---|
| By title — the contradiction grouping | **0 candidates** across 2,095 subjects | Needs an identical title; after the fold almost every subject is a singleton |
| By derived subject page | **2,477 — noise** | Those subjects are single shared *terms*. `codex` alone produced 163 pairs; `detection` grouped "duplicate source_id detection" with "staging directory age detection" |
| **By shared evidence** | **454, real** | The criterion wikilinks already use. Two claims citing the same event are demonstrably about the same episode — not similarly worded, *the same thing happened* |

Of the 454, **441 were written the same day** — consolidation windows overlapping, which is the
fold's business. Same-day is hidden by default; `--all` shows it. The remaining **13** are where
something was learnt later.

And the top of that list is A5 and A8 demonstrating each other: the widest gaps are older claims —
*"Claude Code SessionStart hooks confirmed working in production"*, *"MCP is sole delivery channel
for Codex"* — that the claim filed an hour earlier now completes.

#### The rewrite, designed

The detection output currently ends with *"read the unseen evidence, then file with
`brain remember --supersedes <older-id>`"*. That is **the same interface failure that made
`brain remember` unused for two months** — technically possible, practically never. The design starts
there.

```
brain revise --project X                                      # detect only
brain revise --project X --propose                            # draft, change nothing
brain revise --project X --propose --review-sheet merges.json # draft for human approval  ← prefer this
brain revise --project X --apply-reviewed merges.json         # write only what was approved
brain revise --project X --propose --apply                    # write all of them, unreviewed
```

The last line is the original design and it approves a whole *run*. A9 added the two above it after
this command's own output proved why — see **A9** below.

**What the model is allowed to do.** One narrow job: given two claims and the union of their
evidence, write the one claim that is true now. Not summarise, not expand — merge. The prompt
receives nothing else, so it cannot reach for context it was not given.

**What derivation refuses, with no model involved.** This is where the design earns its keep:

| Rule | Why |
|---|---|
| Every citation ∈ `union(older.evidence, newer.evidence)` | A merge cannot invent provenance. Reuses 3.2b's validator |
| Output shorter than the two inputs combined | A merge that grows is a concatenation wearing a merge's name |
| Must retain the newer claim's unseen evidence | Otherwise it is the older claim restated and nothing was learnt |
| Non-empty title and content | The floor `remember` already enforces |

**Fails validation → nothing is written, and the failure is reported with its reason.** Precisely
what the three dead-lettered jobs should have done instead of storing 400 characters of plausible
JSON and no reason.

**What it writes.** A new claim superseding **both** sides, carrying the union of their evidence and
the higher of the two authorities. Append-only like the fold — both originals stay reachable through
`replay` and `verify memory`.

**Shipping posture, deliberately the same as 3.2b:** build the validator and the apply path now, hold
the live generation until quota. Watching the citation check refuse a *real* bad citation from a
*real* provider is the point; shipping it against a stub and calling it verified would invert the
entire argument for having a validator.

### ✅ A6 · Search on the Retrieval panel

The panel reads configuration and runs zero searches, which is why it feels inert. `brain query` and
`brain explain` already compute everything a results view needs — per-channel rank beside the fused
position. Only the renderer is missing.

**Shape:** a `/api/search` route shelling to `brain explain --json`, and a results view under the
existing channel cards. Verified here: the route's values. Verified by you: that it renders.

### ✅ A13 · Consolidation made one provider call at a time — measured 60 → ~215 jobs/hour

`for project in &config.projects` awaited each project, and the inner `for _ in 0..8` awaited each
job, so three projects and eight slots produced exactly one call in flight. At ~40 s per call that
caps the service near 90 jobs/hour — the ceiling every measurement had been landing under, with
nothing in the code saying so.

One task per project now, capped at three in flight by a semaphore, all awaited before the tick
ends. **Awaiting is what keeps the one-call-per-ledger promise**: without it a slow project would
still be draining when the next tick spawned a second task against the same SQLite file.

| | Before | After |
|---|---|---|
| Drain rate | ~60 jobs/hour | **~215 jobs/hour** |

Two measurements, 10 August, and the difference between them is the point. Over a 15-minute window
that included two `cargo build --release` runs, the test suite and a service restart, the rate was
**171/hour**. Over a 10-minute window with the machine otherwise idle — 1,893 → 1,857 — it was
**215/hour**. Quote the second as the rate and the first as what to expect while also building.

**The second goal mattered more than the first.** Per project, ten minutes after the change:

| Project | Pending | jobs/hour | Uptime hours left |
|---|---|---|---|
| `subscription-agent` | 1,210 | 78 | 15.5 |
| **`agent-knowledge-base-codex`** | **632** | **90** | **7.0** |
| `Ai-community-channel` | 11 | 72 | 0.2 |

The three now drain *independently*, so this project no longer queues behind `subscription-agent`'s
1,210. Serially at ~60/hour the combined 1,853 was ~30 hours before this project could reach zero;
it is now ~7 hours of uptime, and `subscription-agent`'s much larger queue no longer figures in that
number at all. Zero dead letters throughout.

**Uptime, not wall-clock.** The service is a logon-triggered task and does not run while the machine
sleeps — on 9 August that cost six hours. The drain is today if the machine stays on and tomorrow if
it does not.

**And `pending` is not the remaining work.** Jobs are chunked from uncovered evidence one per tick
per project (`enqueue_event_threshold_job(1)`), so the queue is topped up as it drains. Measured at
03:04 UTC, this project had **633 pending *plus* 17,447 uncovered events** — about 88 jobs not yet
created — so ~721 in truth, or **~10 hours** rather than the ~7 the pending count alone suggests.
`subscription-agent` has 1,181 pending and nothing uncovered; `Ai-community-channel` is at zero on
both and is the first project fully consolidated.

The corollary is worth planning around: **working in a project extends its own backlog.** A long
session here adds events, which become jobs, which the drain then has to consume. The queue empties
fastest when the machine is on and nobody is using it.

Note that when a project reaches zero its permit frees but nothing speeds up, because the binding
constraint is the per-project sequential drain rather than the cap of three. That is the design, not
a shortfall: concurrency inside one ledger would put two writers on one SQLite file.

Three is the project count, and it is a *cap* rather than "one per project" so a fourth project
widens the backlog instead of the request rate — a quota shared with `claude -p` is not one to find
the edge of by accident. Backoff on `ProviderUnavailable` stays per-project; a shared one would make
the cap behave like the serial loop again the moment any single project got throttled.

The test asserts both directions because they pull against each other: some pair of calls must
overlap *across* projects, and no pair may overlap *within* one. It fails against the serial
arrangement on the first.

### ✅ A14 · The A/B harness was measuring one condition three times

Found while implementing the review's item 2, and it would have wasted the entire run.

`Set-Condition` read `$settings` — the file it had itself just stripped. With the old block order
that is silently fatal: `bare` removes both hooks and writes the result, `code` then filters *that*
and keeps nothing, `warm` keeps nothing again. All forty-five sessions would have run with no hooks,
the report would have shown three near-identical columns, and the honest reading of that is *"the
brain saves nothing"* — a conclusion about the harness wearing the costume of a conclusion about the
system.

Every condition is now derived from the pristine backup, asserted off disk before each condition's
first run, and `-SelfTest` applies the conditions in several orders against a throwaway fixture.
Against the old version that self-test reports **seven failures** naming the exact hooks that went
missing. It also checks that an unrelated third-party hook survives, since stripping someone else's
hook would be a worse bug than the one it was written for.

Three further changes from the review:

- **Order is shuffled** as one flat list with a recorded seed, so condition and position are
  independent. `warm` no longer always runs last, which was the direction that flattered the result.
- **Every run gets an explicit `--session-id`**, all forty-five written to `sessions.json`. The
  reason is *not* backlog growth: measured against this project's ledger a session's median is ~15
  events, so the whole matrix is about three consolidation jobs, and the earlier note claiming
  otherwise was wrong by two orders of magnitude. It is that a re-run would otherwise score against
  a brain that had consolidated this benchmark's own answers.
- **Blind grading** — `grading-sheet.csv` and `grading-key.csv` are written separately, answers
  reshuffled, condition stripped. Nobody grades a column labelled `warm` the way they grade one
  labelled `bare`, and the failure this exists to catch — a warm session answering confidently from
  a stale memory and stopping early — *wins* on tokens.

### ✅ A9 · A human checkpoint — shipped for merges, still open at ingest

`--apply` approved a whole run: reading thirteen proposals and agreeing with eleven meant writing
all thirteen or none. `merge_candidates` has named that gap in its own doc comment since it was
written.

`brain revise --review-sheet <path>` writes the proposals as JSON, one item each, with a `decision`
field. `brain revise --apply-reviewed <path>` writes only what a human marked `approve`. Three
properties, and it is worthless without any one:

| Property | Why |
|---|---|
| **No provider call at apply time** | `apply_reviewed` is not passed one, so there is no path by which a second draft reaches the ledger. What was reviewed is what is written |
| **An edit is re-checked, not trusted** | A reviewer may rewrite the claim — that is the point — but the text goes back through `validate_merge` against the live pair. A human may fix a sentence; a human may not cite evidence the pair does not carry |
| **A stale sheet is refused per item** | Review is asynchronous *by design*, so consolidation may supersede one side while the sheet sits unread. Applying then would revive a retired claim as half of a current one |

**The third did not work when first written**, and the reason is worth keeping: `current_memory`
returns the memory's latest *version* and nothing more — a `Superseded` version just as readily as a
current one. That is exactly the half-predicate `CURRENT_CLAIM` warns about, sitting behind a
function called `current_memory`. The status check is the other half, and the test fails without it.

**Still open: decision-grade memories at ingest.** Consolidation still writes them unattended. This
half gates the *rewrite* path, which is where A8b's false merges came from; it does not gate the
200-event batches that produce the claims in the first place.

### ✅ A9's ingest half — `8ddb828`

The rewrite path was the smaller half. Consolidation batches 200 events to a provider and appends
whatever survives validation, unattended — and validation is about *form*. The 10 August merges
passed every mechanical rule and asserted a falsehood.

**The gate is a status, not a queue table.** `MemoryStatus::Proposed` was in the model from the
beginning and nothing had ever written it. A memory in that state fails `CURRENT_CLAIM`, so
retrieval skips it for free.

```bash
brain review --project .                 # what is waiting
brain review --project . --approve <id>  # it becomes current
brain review --project . --reject  <id>  # marked invalid, and kept
```

Both rulings *append*. A rejected claim is not deleted — the ledger records that a person looked and
said no, which is what the next proposal of the same claim needs to know.

**Off by default**, via `review.gated_kinds` in `service.json`, and that default is a position
rather than caution: gating a kind means the brain stops telling agents about it until someone gets
to it, and a review queue nobody drains is a brain that forgets on purpose.

#### The part that was not the plan — three read paths would have leaked it

Writing the test as *"the memory is invisible"* rather than *"the memory is marked proposed"* caught
that it was neither. Three read paths carried the same **denylist** — not `invalid` and not
`superseded`:

| Path | Feeds |
|---|---|
| `current_project_memories` | the session-start orientation, the Markdown projection, `brain export` |
| `current_preferences` | global preferences |
| `resolve_memory_set` | the active set after supersession |

That is correct only while there are exactly three statuses, and it **fails open** the moment a
fourth appears. All three promptly served proposed memories. The gate would have been a gate in name
only, and nothing about it would have looked wrong.

Replaced with `MemoryStatus::is_readable` — an **allowlist**, defined once. `Conflict` stays
readable on purpose: it marks a claim that disagrees with another, not one that is wrong, and
retrieval already weights it down rather than hiding it.

**This is the `CURRENT_CLAIM` lesson for the third time in two days** — after
`current_project_memories` and the two retirement mechanisms, and after `current_memory` returning
superseded versions in the reviewed-merge path. A predicate with a missing half reads as working
code.

### ⚠️ A9's original framing — *the argument, kept*

Karpathy stays involved on every ingest. We batch 200 events to a provider unattended, and the three
dead-lettered jobs are that gap showing. Reviewing all of it is not realistic; reviewing **decisions**
(22% of memories, and what the orientation leans on hardest) might be.

Not scheduled. Recorded so the choice is deliberate rather than forgotten.

**10 August gave it evidence.** A8b's generation produced merges that passed every mechanical
check — provenance, brevity, evidence retention — and asserted a falsehood, because the claims they
merged asserted it. No validator catches that, and no amount of validator design will: the rules are
about *form*, and this is about *truth*. That is the case for a human checkpoint stated better than
the original argument stated it.

---

## Order of work

1. **A8b's validator and apply path** — ✅ built and verified. The generation call ran on 10 August.
2. **A6** — ✅ shipped, rendering confirmed.
3. **A7** — ✅ shipped. Route verified against the live ledger; the rendered panel is yours to confirm.
4. **A3b** — ✅ shipped, and it found four dead channels on its first run.

### What is left, in the order it has to happen

Everything below the line is gated on the drain, and the gate is *this project's* queue, not the
global one.

| | Step | Gate |
|---|---|---|
| 1 | ✅ **A13** — make consolidation concurrent | done first, because it moved every estimate after it |
| 2 | ✅ **A14** — fix the A/B harness · ✅ **A9** — the merge checkpoint | done *while* the queue drains; neither spends quota |
| 2b | ✅ **A6** verified in rendered pixels · ✅ **A9** ingest half | done; neither spends quota |
| 3 | ⚠️ Generate the remaining 93 subject pages | this project reaches **0 pending** — their prose is keyed to an exact memory set, and consolidation is still changing it |
| 4 | ⚠️ Confirm 0 dead letters, embeddings caught up | after 3 |
| 5 | ⚠️ Run the interleaved 45-session A/B | after 4 |
| 6 | ⚠️ Grade blind against the rubrics, *then* quote a percentage | after 5, and it is not optional — the token column cannot see a confidently wrong answer |

Steps 3 and 5 are the only ones that spend quota, which is why they are last and why nothing above
them had to wait for the queue.

---

## The quota returned — what that unblocked, and the one thing it did not

GLM answered again on 10 August. Four items had been waiting on it, and the same key turned out to
unlock a fifth: `claude -p` authenticates and reports `glm-5.2` in its usage, so the "Anthropic
weekly limit" and the "GLM quota" were never two blockers. The roadmap predicted exactly that and
was right.

### A8b's generation ran, and both validator rules refused real output

This was held back on purpose — *"watching the citation check refuse a real bad citation from a real
provider is the point"* — and both substantive rules were caught doing it:

| Rule | What it refused |
|---|---|
| `LostNewEvidence` | *"dropped 1 event(s) that only the newer claim saw — those are the reason this pair was a candidate at all"* — a merge that was really the older claim restated |
| `ForeignCitation` | *"cites event:019fd14e…, which neither claim rests on — a merge cannot invent provenance"* |

**And the merges were confidently false anyway**, which is the finding that matters. The provider
produced well-formed, well-cited, correctly-shortened prose asserting *"Codex Desktop does not fire
SessionStart hooks"* — because the claims it was merging said so. **Every guarantee A8b makes is
mechanical, and none of them is truth.** `--apply` was not run.

**This is the evidence A9 was built from, and A9 now gives it somewhere to go.** A blanket `--apply`
was the only way to write these, so the choice was thirteen or none. `--review-sheet` makes it a
per-pair decision by a person, which is the only check that operates on truth rather than form.

### What that exposed: 26 current claims asserting a known falsehood

The vault told any agent, twenty-six different ways, that Codex does not fire hooks — a belief this
project disproved on 9 August. Retracted with three accurate claims via `remember --supersedes`,
append-only, keeping the six that were true or explicitly historical (`Phase B test was confounded
by an untrusted hook` was right all along and was outvoted by louder wrong ones).

**Retracting them exposed a worse bug.** A claim can be retired two ways — `reconcile --apply`
appends a `superseded` *version*, `remember --supersedes` writes a supersession *edge* — and
`current_project_memories` honoured only the first. So after retracting 26 claims it still returned
**30 retired ones**, and that query feeds the session-start orientation, the Markdown projection and
`brain export`. `search_memories` had excluded them all along, so the two read paths disagreed about
what "current" means. Third variant of the `CURRENT_CLAIM` bug. Fixed `7a75da6`; live count 2,315 →
2,285.

### 3.2b's generation shipped

`brain synthesize` is the call that never existed — the validator, the store and the projector's
read path had all been in place for months. 63 subject pages now carry prose; 93 remain. One
refusal in 45, a provider timeout, correctly scoped to that subject rather than the run.

### One resilience fix each, both from the same lesson

`brain revise --limit 8` lost all seven completed merges to a single transient timeout, because `?`
propagated it. A provider failure is now that candidate's failure. `brain synthesize` was written
with the same rule from the start.

### The A/B is unblocked and deliberately not run

Its precondition is not met: this project is **48% consolidated, 632 jobs still queued**, and a
half-consolidated brain understates the warm condition, so the figure would have to be re-run.
Operator's decision, taken: wait for the drain, then run 5 × 3 × 3.

At the post-A13 rate that is **~7 hours of uptime** for this project — not for the whole backlog,
which is a distinction worth keeping when the number is finally quoted. `subscription-agent` will
still hold ~1,000 jobs at that point. The benchmark asks five questions about *this* repository, so
that is fine; it does mean "the backlog has drained" will be true of the thing being measured and
false globally.

The waiting time was also spent, not merely passed: **A14** found that the harness's second and
third conditions were both measuring the first, which would have made the run worthless.

---

## Blocked, and by what

| Item | Blocker |
|---|---|
| ~~A8b's generation call~~ | ✅ **Run 10 August.** Both validator rules observed refusing real provider output |
| Token-saving A/B | **Its own precondition, not quota.** `claude -p` authenticates and reports `glm-5.2`. This project is 48% consolidated with 632 jobs queued, and a half-consolidated brain understates the warm condition. ~7 h of uptime left — see below |
| ~~3 dead-lettered jobs~~ | ✅ **Requeued 10 August** via `brain jobs --retry-dead`, which did not exist — the digest reported the count and nothing could act on it |
| ~~3.2b synthesis prose~~ | ✅ **Shipped 10 August.** `brain synthesize`; 63 subject pages carry prose. The other 93 are **held back on purpose**, not outstanding work: their prose is keyed to an exact memory set, and generating now would be regenerating after the drain |
| ~~Codex mid-session parity~~ | ✅ **Resolved.** It was never structural: `UserPromptSubmit` is registered and observed firing four times across two Codex prompts. Nothing is invoked voluntarily any more |
| Rendered-UI verification | The Browser pane never paints. Split above |

### The backlog ETA was a rate quoted as a duration

"~62 jobs/hour, so ~34 hours, unattended" was checked seven hours later and found 95 jobs drained.
The rate was right; the sentence was wrong. **62/hour is the rate while the provider answers and the
machine is awake**, and neither holds most of the time.

| Day (UTC) | Consolidated |
|---|---|
| 6 Aug | 789 |
| 7 Aug | 2,020 |
| 8 Aug | 75 |
| 9 Aug | 295 |

9 August logged **4,554 HTTP 429s**, 300–390 per hour from 00:00 to 14:00, and the drain did almost
nothing until quota returned; it then ran at ~60/hour for five hours until the machine slept at 18:51
and did not resume until 00:49. So 34 h of *productive uptime* was somewhere between one day and a
week of calendar, depending entirely on a quota nobody here can predict.

Two things came out of that, one still true and one since fixed:

- **The service only drains while the machine is on.** Still true, and unfixable from here: it is a
  logon-triggered task, so an overnight is simply not counted, and an ETA in wall-clock hours
  silently assumes 24-hour uptime. Every figure below is *uptime*.
- **The whole system made one provider call at a time.** ✅ **Fixed — A13.** The
  `for project in &config.projects` loop awaited each project and the inner `for _ in 0..8` awaited
  each job, so three projects and eight slots added up to no concurrency at all, capping the service
  near 90 jobs/hour. That is why every measurement kept landing under it. Idle rate is now
  **~215 jobs/hour**, and — the part that mattered more — the three ledgers drain independently, so
  this project no longer queues behind `subscription-agent`.

**And the backlog is not old backfill.** The pending jobs for `agent-knowledge-base-codex` cover
7 August through 10 August — the project is 48% consolidated, the lowest of the three, and the
unconsolidated part is the recent work an A/B question here would actually be about. That is the
whole reason the A/B waits, and it is a measurement-validity reason rather than a cost one.

**On cost, for the record.** With GLM quota no longer scarce, the ranking of reasons to wait is:
the number would not be quotable (validity); the run competes with the drain for the same quota
(throughput); the money (smallest, and recorded per run as `total_cost_usd`). An earlier version of
this section led with cost and also claimed the run would meaningfully grow the backlog — measured,
a session's median is ~15 events, so 45 of them is roughly **three** consolidation jobs. That claim
was wrong by two orders of magnitude. The real reason to keep those sessions identifiable is
self-contamination on a re-run, which is why they now carry explicit `--session-id`s.

---

## What would make this round a failure

Worth writing down before the work, not after. Two of these have already been tested.

- **A5 ships and nothing gets filed.** ✅ *Survived* — six claims filed the same day, and one of them
  immediately produced the highest-value revision candidates.
- **A8's candidates are noise.** ⚠️ *Nearly happened.* The first two groupings produced 0 and 2,477.
  The rule held: stricter detection, not a louder feature. Shared evidence gave 13 actionable pairs.
- **A8b proposes rewrites nobody accepts.** Untested — the honest measure is how many proposals are
  applied versus skipped, and that number should be recorded rather than assumed.
- **The A/B, when it finally runs, shows no saving.** The outcome this project should most want to
  know and least wants to be true. It is why the counter-metric is reported with equal prominence.
