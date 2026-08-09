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
mirrored sections, and this project's own `AGENTS.md`. Delete it from any project still carrying it.

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

### ⛔ Blocking: hook delivery is failing for **both** harnesses

Found while verifying the above, and it is the top priority — everything else in this document is
downstream of the pipe working.

All six Codex hook invocations **spooled instead of delivering**. The service log carries six
matching `hook pipe request failed / error="write hook reply"` — the service compiled each
orientation and found the client gone before it could reply.

What has been ruled out:

| | |
|---|---|
| Codex-specific | **No.** A `--harness claude-code` probe fails identically |
| Service down | No. Running, and restarted cleanly mid-diagnosis |
| Stale deployment | No. All four binaries present and matching the manifest |
| Pipe-name mismatch | No. `service.json` and `DEFAULT_PIPE_NAME` agree |
| A timeout | No. The hook returns `{}` in ~140 ms against a 3 s budget |
| Recent change to the pipe | No. `pipe.rs`, `hook_handler.rs` and `brain-hook/` are untouched since it last worked |

Reproducible on demand: a probe at 11:08:03 UTC produced a fresh failure. The next step is
instrumenting the hook client's error path — it fails open to `{}` and discards the reason, which is
correct for a session start and useless for debugging. **That discarded error is the whole
investigation**, and a `BRAIN_HOOK_DEBUG` escape hatch that prints it would have saved this session.

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
