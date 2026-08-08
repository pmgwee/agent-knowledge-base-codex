1. Cross-session blindness within a project. Your 2–4 Claude Code mega-sessions on the same project have no memory of each other's work. Two sessions can edit the same file — even the same line — in conflicting ways, or one edits a file another deleted, because neither knows the other moved.

2. Continuing a task in a new session burns your token quota.

2.1 When a mega-session gets "stupid/confusing/conflicting" and you spin up a fresh 5th session to continue that task, you must export the old session and have the new one read it all — consuming a big chunk of your subscription just to re-load context.
2.2 To get the new session current on the latest code/deployed state, you make it read the whole codebase (or an outdated CLAUDE.md/README) — more token burn, and those docs are often wrong/stale anyway.
3. Codex pays the same onboarding tax. A new Codex session on the same project needs the same 2.1 + 2.2 dance — export+read prior session, read latest code. Same waste, different platform.

4. No shared memory between Claude Code and Codex. The two agents store sessions in their own separate database folders with zero sync. Claude Code's agent/memory isn't reachable by Codex. Work done on one platform is invisible to the other.

5. "What did I do last week?" is unreliable. Ask either agent about past work and you get only partial fragments — and you can't tell if the memory is correct or stale.

The goal these point at (the project's success criterion): a pluggable secondary brain shared by both Codex and Claude Code — new sessions on either platform pull precise, up-to-date, accumulated memory (yesterday / last week / last month, since the brain was built) instead of re-reading exports or codebases, so agents work at top quality with complete cross-platform memory.

Worth noting in the context of tonight's work: pain point #5 (unreliable past-memory) is exactly what the dashboard you just fixed serves — it's the visibility surface for the brain that solves #1–#5. And the brain-service exit we diagnosed is a direct hit on the brain's ability to deliver on these (when it's down, new sessions go blind = pain point #2/#4 regressing live). That's why that exit matters more than a normal service hiccup.