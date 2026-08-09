# Registering a project into the secondary brain

> **This file is authoritative.** `CLAUDE.md` and `AGENTS.md` each carry a short *Registering a
> project* summary so both agents know the procedure without opening anything. Those two are
> mirrors — when the procedure changes here, update both. If they ever disagree with this file,
> this file wins.

## The prompt template

Copy this, fill in the path, send it. Nothing else is needed.

```
Register a new project into the secondary brain:

- Path: C:\Users\quekm\Desktop\projects\<PROJECT-FOLDER>
- Agents I've used on it: <Claude Code | Codex | both | neither yet>
```

That is the whole input. The second line is optional — it only sets expectations about how
much history should appear, since registration ingests whatever transcripts already exist.

## What is *not* needed, and why

| You might expect to give | Needed? | Why not |
|---|---|---|
| GitHub repo URL | **No** | The brain never contacts a remote. It reads local transcripts and the local git directory. A repo that was never pushed works identically. |
| Project name | **No** | Derived from the directory. The stable identity is a generated `project_id` (UUID), not a name — so renaming the folder later does not create a second project. |
| Which transcripts to capture | **No** | Discovered from `~/.claude/projects` and `~/.codex/sessions`. Ownership is decided by each transcript's own recorded `cwd`, never by folder proximity. |
| Language, framework, tech stack | **No** | Nothing about capture is language-aware. |

**Sessions opened in subfolders are claimed correctly.** Registering `Ai-community-channel`
picked up 371 Claude sources, most of them recorded under
`~/.claude/projects/c--…-Ai-community-channel-discord-mcp/` because the work happened in a
subdirectory. That is right, not a mis-claim: ownership tests whether the transcript's recorded
`cwd` is *contained by* the project root, not whether it equals it. Source paths that look
unfamiliar are expected — there is nothing to fix.

The only thing that matters is the **path**, and that it is the same path you actually open
sessions in. A transcript is claimed by a project when its recorded `cwd` sits inside the
project root — so if you sometimes work from a subfolder, that still resolves correctly, but a
different clone of the same repo elsewhere on disk is a *different* project by design.

## What happens when you send it

1. **Register.**
   ```
   brain.exe --brain-home ~/AgentBrain register <path>
   ```
   Creates the project's ledger and discovers every existing Claude Code and Codex transcript
   whose `cwd` falls inside the project root. This is a point-in-time snapshot.

2. **Restart the service.** Capture bindings are built once, at service startup
   (`build_capture_bindings` in `crates/brain-service/src/main.rs`). Until it restarts, the new
   project is in the config but nothing is capturing it.
   ```
   schtasks /Run /TN "AgentBrain.Service"
   ```

3. **Nothing.** Both harnesses are now wired globally, and neither needs a per-project step.

   This used to be the required, easily-forgotten step: append a brain section to the new
   project's `AGENTS.md` telling Codex to call `brain_checkpoint`. **It is obsolete**, and the
   section can be deleted from projects that carry it.

   Codex hooks were verified dispatching on 9 August 2026 (Desktop build `26.803.41515`, CLI
   `0.147.0`). Codex is now pushed to exactly as Claude Code is — `SessionStart`, `SessionEnd`
   and `UserPromptSubmit`, all harness-invoked, all registered globally in
   `~/.codex/hooks.json`.

   **Why removing it is an improvement rather than a simplification.** The `AGENTS.md`
   instruction was never wiring; it was a *request* that the model call a tool. That has three
   failure modes a hook does not have: the model may not read the file, may read it and skip
   the call, or may call it late — after it has already started reading the codebase, which is
   the cost the orientation exists to avoid. None of those are visible from the outside. A hook
   cannot be skipped by the model at all, because the model never sees the decision.

4. **Verify** — event count is non-zero, and a query returns cited results:
   ```
   brain.exe --brain-home ~/AgentBrain status --project <id>
   ```

Nothing needs doing per-harness. Both hooks are registered globally and resolve the project from
the session's `cwd`.

## After registration

New sessions are picked up automatically. The service rescans the transcript roots at startup
and every 120 s (`REDISCOVERY_INTERVAL` in `crates/brain-service/src/rediscover.rs`), so a
session opened after registration is found without any action.

**One caveat worth knowing:** rediscovery *records* new sources into the config but does not
rebuild capture bindings live — they are picked up on the next service start. In practice the
service restarts often enough that this is invisible, but if a project seems to stop
accumulating events, a restart is the first thing to try.

## The Codex hook trust gate, and the Windows quoting rule

Two things must both hold before a Codex hook runs. Both were learned the hard way, and each
looked like "Codex does not support hooks" from the outside.

**1. The command must not quote the executable on Windows.** Codex does not strip quotes from
`commandWindows`, so the quoted form fails with `hook exited with code 1`:

```jsonc
"command":        "\"C:\\Users\\you\\AgentBrain\\bin\\brain-hook.exe\" --harness codex",  // POSIX: quoted
"commandWindows": "C:\\Users\\you\\AgentBrain\\bin\\brain-hook.exe --harness codex"       // Windows: NOT quoted
```

`brain install-hooks codex` writes both correctly and **refuses an executable path containing a
space**, since the unquoted form cannot survive one. Keep the binary at `~/AgentBrain/bin`.

**2. The hook must be trusted.** Codex records a SHA-256 per hook in `~/.codex/config.toml` under
`[hooks.state]` and will not invoke an untrusted one. Approve it at the Codex CLI TUI's hook
review prompt. **Editing the hook changes its hash and revokes trust**, so re-approve after any
reinstall.

```toml
[hooks.state.'C:\Users\you\.codex\hooks.json:session_start:0:0']
trusted_hash = "sha256:…"
```

**Check `[hooks.state]` first when a Codex hook seems dead.** A hook that cannot launch, and one
that is untrusted, both produce zero deliveries *and* zero spool entries — identical to never
being invoked. That ambiguity produced two confident wrong conclusions here, five days apart,
the second of which blamed an upstream issue and recorded a matching build number.


## Why the per-project `AGENTS.md` step is gone — kept, because it was argued at length

This section used to defend requiring a brain block in each project's `AGENTS.md`, and to explain
why a single global `~/.codex/AGENTS.md` was rejected. Both questions are now moot: **neither is
needed, because Codex is pushed to.**

The original reasoning, preserved because the prediction it made came true:

> `AGENTS.md` is not wiring. It is instruction. The asymmetry is about **who pulls the trigger** —
> Claude Code is invoked by the harness before the model reads anything and cannot skip it; Codex
> was invoked by the model, from a tool it could already see, and therefore had to be told.
>
> *"That gap exists only because Codex hooks do not fire on this build. If Codex ships working
> hooks, `AGENTS.md` drops from required to optional and this section becomes history."*

That is what happened, though not for the reason expected. Codex hooks were dispatching all along;
what did not work was **our command line** — `commandWindows` quoted the executable, which Codex
does not strip, so the hook exited 1 before reaching our binary. Fixed, and pinned by
`the_windows_command_is_unquoted_and_the_posix_one_is_not`.

**Delete the brain section from any project that still carries it.** It is not merely redundant now
— it is worse than nothing, because it asks the model to spend a tool call reproducing context the
harness already placed in front of it.

**What MCP is still for.** `~/.codex/config.toml` keeps `[mcp_servers.brain]`, and it should. The
hook *pushes* an orientation; the tools *answer questions* — `brain_search`, `brain_timeline`,
`brain_evidence`, `brain_claims`, `brain_leases` have no hook equivalent and never will, because
nothing can push an answer to a question not yet asked. What changed is that MCP stopped being the
delivery path and went back to being depth on demand.

## Cross-project isolation

Registering more projects never causes leakage. Ownership is decided per-transcript from its own
`cwd`, and the guarantee is pinned by `discovery_never_crosses_a_project_boundary` in
`crates/brain-service/tests/source_rediscovery.rs`. Two projects can share a transcript
directory and still claim only their own sessions.

## Storage expectation

Budget roughly **100–600 MB of ledger per project**, depending on how much history exists at
registration. Most of the initial size is backlog: registration ingests *all* pre-existing
transcripts, not just new ones. Ongoing growth is much slower.

Each project also multiplies backup storage by the retention count — see
[storage-and-backup.md](storage-and-backup.md) before adding several at once.
