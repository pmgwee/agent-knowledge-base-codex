# Enhancement review — the plan, and what it is answering

Written 9 August 2026, after reading Karpathy's *LLM Wiki* against what this system actually does.

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
| `index.md` as primary navigation | ⚠️ built, unused by retrieval | ✅ Right — he says it *"avoids embedding-based RAG"* at ~100 sources; we are at 143k events |
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
| **A5** | **The file-back loop** | `4c213f3` | `--evidence` is now optional; citations are derived by running the claim's own text through the fused retrieval measured at 96.0% R@5, and printed for checking. Refuses rather than invents when retrieval finds nothing. **Five human claims filed on the day it shipped** |
| **A2** | Architecture diagram corrected | `4c213f3` | It drew RRF fusion feeding the session-start orientation. The code does not do that: `ContextCompiler::from_ledger` reads `recent_events()` chronologically, and fusion feeds the **mid-session push** (`hook_handler.rs:421`). Split into two boxes |
| **A4** | Capture-staleness detection | `4c213f3` | Capture stalled 100 minutes with the service running and every panel green, because nothing compared the newest event to the wall clock. The digest now leads with it |
| **A1** | `push-snapshot.mjs` 10 s → 45 s | `6e006c0` | Same defect as the API route, second location. `brain dashboard` takes 18 s, so every push timed out and Redis served a pre-Config snapshot — which is why the Config panel loaded forever |
| **A3** | Harness named on hook metrics | `33684d4` | One "SHIPPED · 75 delivered" badge covered two harnesses, one of them at zero for all three hooks |
| **—** | **Codex hook test** | `33684d4` | Settled. See below |

---

## The Codex question, settled

Tested 9 August against a real Codex Desktop session.

- **Zero `codex/SessionStart` deliveries, ever.** The only row is `diag-codex-1`, a diagnostic.
- **Zero of 144 spooled hook requests come from Codex** — the discriminator, since a hook that fired
  and failed to deliver would still spool. It is never invoked.
- The session obtained context by calling `brain_checkpoint` over MCP, exactly as `AGENTS.md`
  instructs.

**Cause:** there is no Codex CLI on this machine. The official hook documentation describes the
**CLI's** surface; **Codex Desktop does not implement hooks.**

**Decision: keep `~/.codex/hooks.json` registered and wait for Desktop support.** Removing it would
break the CLI path and would have to be redone. What changes is the *reporting*. Recorded in both
`CLAUDE.md` and `AGENTS.md` so neither agent re-derives it.

**The residual gap is continuous re-orientation, not the handover.** Claude re-orients on every
prompt whether it wants to or not. On Desktop that cannot be closed with hooks — only with an
instruction Codex may ignore.

---

## Open

### A8 · Cross-claim revision — the missing half

**Status: designed, not built. Nothing blocks it.**

This is the half of the two-sided mistake that A5 did not close, and it was nearly missed: it had
been folded into *"3.2b at subject-page level and no further,"* which does not do it.

Three things are easy to confuse:

| | What it does | Have it? |
|---|---|---|
| The fold (`fbc6fe5`) | Two claims that are the *same claim* merge | ✅ 97 folded |
| 3.2b synthesis prose | A paragraph on top of a subject page's list | ⏳ quota |
| **Cross-claim revision** | A new claim causes an **existing, different** claim to be **rewritten** | ❌ **nothing** |

Karpathy: *"a single source might touch 10–15 wiki pages."* We touch zero.

**Detection needs no provider.** A new claim that shares a subject with an existing one, does not
contradict it, and adds a fact the older one lacks is a *revision candidate* — derivable from the
subject key, the evidence overlap, and the supersession graph we already maintain. Only the rewrite
needs a model, and it can propose rather than apply, like `brain reconcile` does.

**Done when:** a claim filed today causes an older claim on the same subject to be reissued with the
new information, cited to both, with the original superseded and reachable.

### A3b · Harness split in the dashboard JSON

`DeliverySummary` is per project, not per harness. The architecture band is corrected; the dashboard
still averages two agents into one number.

### A6 · Search on the Retrieval panel

The panel reads configuration and runs zero searches, which is why it feels inert. `brain query` and
`brain explain` already compute everything a results view needs — per-channel rank beside the fused
position. Only the renderer is missing.

### A7 · Session replay UI, and citations clickable inline

Replay belongs in a UI: 1,369 events is unreadable in a terminal. Provenance does **not** belong in a
panel of its own — it belongs on the citation, wherever a citation appears.

### A9 · A human checkpoint on consolidation — *debatable, listed honestly*

Karpathy stays involved on every ingest. We batch 200 events to a provider unattended, and the three
dead-lettered jobs are that gap showing. Reviewing all of it is not realistic; reviewing **decisions**
(22% of memories, and what the orientation leans on hardest) might be.

Not scheduled. Recorded so the choice is deliberate rather than forgotten.

---

## Blocked, and by what

| Item | Blocker |
|---|---|
| Token-saving A/B | **Provider quota.** Headless sessions return 429 until the weekly limit resets. Harness built and verified; run it with the inherited proxy variables cleared |
| 3 dead-lettered jobs | Provider quota. They never retry on their own |
| 3.2b synthesis prose | Provider quota, by choice — watching the citation check refuse a *real* bad citation is the point |
| Codex mid-session parity | **Structural.** No hooks on Desktop. The ceiling is `brain_context_for_prompt` invoked voluntarily |
| Dashboard visual verification | The preview pane never composites, so a rendered UI cannot be checked here. Confirmed pre-existing by stashing every change and reloading |

---

## What would make this round a failure

Worth writing down before the work, not after:

- **A5 ships and nothing gets filed.** The interface was the blocker; if filing still does not happen,
  the diagnosis was wrong and the fix was cosmetic.
- **A8 proposes revisions nobody accepts.** That would mean the candidates are noise, and detection
  needs to be stricter rather than the feature louder.
- **The A/B, when it finally runs, shows no saving.** That is the outcome this project should most
  want to know and least wants to be true. It is the reason the counter-metric is reported with equal
  prominence.
