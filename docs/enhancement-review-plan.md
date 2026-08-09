# Enhancement review — the plan, and what it is answering

Written 9 August 2026, after reading Karpathy's *LLM Wiki* against what this system actually does.
Updated the same evening, after the first half of it shipped and two of its assumptions turned out
to be wrong.

The other two documents answer *what is running* ([status.md](status.md)) and *what is left and why*
([roadmap.md](roadmap.md)). This one is narrower and has a shelf life: it is the working plan for one
round of enhancements, with the reasoning that produced it, so that a week from now it is possible to
check whether the reasoning held.

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

## The Codex question — reversed, twice, and now settled by finding the actual gate

**Codex Desktop fires `SessionStart`.** Four real deliveries on 9 August at 10:02–10:11 UTC, genuine
v7 session ids, 1,030–1,041 tokens each with 46 coordination tokens and 4–7 citations, and zero spool
entries.

**What changed was trust, not dispatch.** `~/.codex/config.toml` gained a `[hooks.state]` section
carrying a `trusted_hash` per hook. Codex records a SHA-256 of each hook and refuses to invoke an
untrusted one.

**Two wrong conclusions, five days apart, from the same unsound discriminator.** Both times the
evidence was zero deliveries *and* zero spool entries, read as "never invoked" on the reasoning that
a hook which fired and failed would still spool. The reasoning is sound; the conclusion does not
follow. **An untrusted hook is never invoked, so it never spools either** — from our side of the pipe
"not dispatched" and "not trusted" are the same observation. The second attempt compounded it by
attributing the behaviour to `openai/codex#21639` and recording a matching build number, which made
a guess look like a diagnosis.

A 5 August memory said *"Phase B Desktop hook test was confounded by an untrusted hook."* It was
right. Louder, more confident, wrong memories outranked it — which is an argument for `brain lint`
and `reconcile` doing their jobs, and for weighting a claim's *specificity* rather than its volume.

**So `[hooks.state]` is the first thing to check** — before the spool, before the deliveries table,
before any issue tracker. Recorded in `CLAUDE.md` and `AGENTS.md`.

### What this opens

`UserPromptSubmit` is still not registered for Codex. `CODEX_EVENTS` omits it because registering an
event Codex does not fire would look like a shipped feature that never runs — sound reasoning whose
**premise has now changed**. Codex demonstrably fires hooks.

**A10 · Test `UserPromptSubmit` on Codex, and register it if it fires.** If it does, Codex gains
mid-session re-orientation and the harnesses reach real parity — the gap that has been described as
structural throughout this document turns out to be one untested registration.

## How this round is verified — and the one thing that cannot be

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

## Open

### A8b · Cross-claim revision — the rewrite

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
brain revise --project X                     # detect only            ← shipped
brain revise --project X --propose           # write the merged claim, change nothing
brain revise --project X --propose --apply   # write it, supersede both sides
```

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

### A6 · Search on the Retrieval panel

The panel reads configuration and runs zero searches, which is why it feels inert. `brain query` and
`brain explain` already compute everything a results view needs — per-channel rank beside the fused
position. Only the renderer is missing.

**Shape:** a `/api/search` route shelling to `brain explain --json`, and a results view under the
existing channel cards. Verified here: the route's values. Verified by you: that it renders.

### A7 · Session replay UI, and citations clickable inline

Replay belongs in a UI: 1,369 events is unreadable in a terminal. Provenance does **not** belong in a
panel of its own — it belongs on the citation, wherever a citation appears.

**Shape:** a `/api/replay` route over `brain replay --json`; citations become controls that expand
the cited turn inline. Same verification split.

### A3b · Harness split in the dashboard JSON

`DeliverySummary` is per project, not per harness. The architecture band is corrected; the dashboard
still averages two agents into one number, one of which is at zero.

### A9 · A human checkpoint on consolidation — *debatable, listed honestly*

Karpathy stays involved on every ingest. We batch 200 events to a provider unattended, and the three
dead-lettered jobs are that gap showing. Reviewing all of it is not realistic; reviewing **decisions**
(22% of memories, and what the orientation leans on hardest) might be.

Not scheduled. Recorded so the choice is deliberate rather than forgotten.

---

## Order of work

1. **A8b's validator and apply path** — fully verifiable here, and it is the operation that makes the
   vault compound. Ready to run the moment quota returns.
2. **A6**, then a screenshot.
3. **A7**, then a screenshot.
4. **A3b** alongside whichever of those touches the snapshot shape.

---

## Blocked, and by what

| Item | Blocker |
|---|---|
| A8b's generation call | **Provider quota**, by choice — see the shipping posture above |
| Token-saving A/B | **Provider quota.** Headless sessions return 429 until the weekly limit resets. Harness built and verified; run it with the inherited proxy variables cleared |
| 3 dead-lettered jobs | Provider quota. They never retry on their own |
| 3.2b synthesis prose | Provider quota, by choice |
| Codex mid-session parity | **Structural.** No hooks on Desktop. The ceiling is `brain_context_for_prompt` invoked voluntarily |
| Rendered-UI verification | The Browser pane never paints. Split above |

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
