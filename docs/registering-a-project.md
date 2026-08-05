# Registering a project into the secondary brain

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

3. **Add `AGENTS.md`** to the project root, so Codex knows the brain exists and calls
   `brain_checkpoint`. Codex reads this file automatically at session start. Skip only if you
   will never use Codex on the project.

4. **Verify** — event count is non-zero, and a query returns cited results:
   ```
   brain.exe --brain-home ~/AgentBrain status --project <id>
   ```

Nothing needs doing for Claude Code. Its hook is registered globally in
`~/.claude/settings.json` and resolves the project from the session's `cwd`.

## After registration

New sessions are picked up automatically. The service rescans the transcript roots at startup
and every 120 s (`REDISCOVERY_INTERVAL` in `crates/brain-service/src/rediscover.rs`), so a
session opened after registration is found without any action.

**One caveat worth knowing:** rediscovery *records* new sources into the config but does not
rebuild capture bindings live — they are picked up on the next service start. In practice the
service restarts often enough that this is invisible, but if a project seems to stop
accumulating events, a restart is the first thing to try.

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
