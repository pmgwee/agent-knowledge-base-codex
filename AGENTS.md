# AGENTS.md

This file provides guidance to Codex when working with code in this repository.

## Deployment — read this before changing any Rust code

The binaries the system runs are copies in `~/AgentBrain/bin/`, not `target/release/`.
Building is not shipping: a change that compiles but is never installed leaves the service,
both agent integrations, and the dashboard running older code, with nothing looking wrong.

This is automated — **committing is deploying**. A commit touching `crates/` or `Cargo.*` fires
`.githooks/post-commit`, which runs `scripts/deploy.ps1` detached: build, install, restart
service, record the result in `~/AgentBrain/runtime/deploy.json`.

It is fail-safe. Nothing is replaced until the build succeeds, so a commit that does not
compile leaves the previous deployment live and records the compiler errors instead.

To install the working tree without committing:

```
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/deploy.ps1
```

To check what is actually installed, run `brain dashboard` and read the `deployment` section.

**Static CRT — do not remove `.cargo/config.toml`.** It sets `+crt-static` so the binaries
don't depend on `VCRUNTIME140.dll`, which isn't available in Codex's hook sandbox or Task
Scheduler session 0. Without it, `brain-hook` fails (Codex "exit 1") and `brain-service`
crashes under Task Scheduler. A `.cargo/`-only commit doesn't trigger the auto-deploy hook —
run `scripts/deploy.ps1` manually if you change it.

## Invariants — do not break

- **Cross-project isolation is absolute.** Zero leakage between projects. A transcript's
  `cwd` decides ownership, never directory proximity.
- **Evidence is append-only.** Supersede; never delete or rewrite.
- **The context budget is a contract.** 1,000–1,500 tokens normal, 3,000 hard max.
- **Cursors are keyed by source.** Append only to a project's source list; never reorder
  or replace — that orphans cursors and re-ingests captured evidence.

See `CLAUDE.md` for the full deploy contract, drift detection, paths, and the dashboard coupling.

## Build and test

- Rust 1.88.0 (MSVC, edition 2024). `cargo build --workspace --release` produces four binaries.
- `cargo test --workspace` runs all unit, integration, and end-to-end tests.
- `cargo clippy --workspace --all-targets -- -D warnings` is a release gate.
- Scale gates are ignored by default: `--ignored` runs them (primary = 12k sessions/6M events; stress = 120k/60M).

## How each harness receives context

**Both harnesses are pushed to.** Claude Code has always been; Codex Desktop joined on 9 August 2026
once its hooks were trusted.

| | Claude Code | Codex Desktop |
|---|---|---|
| `SessionStart` | Fires. 75+ deliveries | **Fires.** 4 real deliveries, 1,030–1,041 tokens, 46 coordination tokens |
| `SessionEnd` | Fires | Registered and trusted; not yet observed |
| `UserPromptSubmit` | Fires — mid-session push, 400 tokens | **Not registered.** See below |
| MCP | Not wired | 16 tools; `brain_checkpoint` remains available as depth |

### The trust gate — check this before concluding anything about Codex hooks

Codex records a **SHA-256 of each hook** in `~/.codex/config.toml` under `[hooks.state]`, and
**refuses to invoke an untrusted hook**. Approve them at the Codex CLI TUI's hook-review prompt.

```toml
[hooks.state.'C:\Users\you\.codex\hooks.json:session_start:0:0']
trusted_hash = "sha256:…"
```

**This cost two wrong conclusions five days apart, and the mistake is worth understanding.** Both
times the evidence was: zero `codex/SessionStart` deliveries *and* zero entries in
`~/AgentBrain/runtime/spool/`. That was read as "Codex never invokes the hook", on the reasoning that
a hook which fired and failed to deliver would still spool.

The reasoning is sound and the conclusion did not follow. **An untrusted hook is never invoked, so it
never spools either** — "not dispatched" and "not trusted" are indistinguishable from our side of the
pipe. The second attempt went further and blamed an upstream regression (`openai/codex#21639`),
recording a matching build number. A 5 August memory said *"Phase B Desktop hook test was confounded
by an untrusted hook"*; it was right, and louder wrong memories outranked it.

So: **`[hooks.state]` is the first thing to check** — before the spool, before the deliveries table,
and before reading any issue tracker.

### What is still asymmetric

`UserPromptSubmit` is registered for Claude and not for Codex. `CODEX_EVENTS` omits it on the
reasoning that registering an event Codex does not fire would look like a shipped feature that
silently never runs. **That premise has now changed**: Codex demonstrably fires hooks. Whether it
fires this one is untested, and testing it is the one open question left on parity. Until then Codex
orients once per session while Claude re-orients on every prompt.

## Registering a project

> **Mirrored section.** The same procedure appears under *Registering a project* in `CLAUDE.md`,
> so either agent can run it. **Changing one means changing the other** — nothing enforces the
> correspondence, and an agent reading the stale copy follows stale instructions. The
> authoritative long form is `docs/registering-a-project.md`; if the three ever disagree, that
> file wins.

`brain register <path>` takes **only the local directory** — no repo URL, no project name.
Transcripts are discovered from `~/.claude/projects` and `~/.codex/sessions`, and each one is
claimed by the project whose root contains its own recorded `cwd`.

The step that is easy to miss: **restart the service afterwards.** Capture bindings are built
once, at startup (`build_capture_bindings` in `crates/brain-service/src/main.rs`), so until
`schtasks /Run /TN "AgentBrain.Service"` runs, the project sits in the config with nothing
capturing it — and everything looks fine while that is true.

**Nothing else.** There is no per-harness step: both hooks are registered globally and resolve
the project from the session's `cwd`.

The `AGENTS.md` brain section that used to be required here is **obsolete — delete it from any
project that still carries it.** It asked Codex to call `brain_checkpoint` because its hook was
believed not to fire; the hook fires. Leaving it in is worse than redundant, since it spends a
tool call reproducing context the harness has already placed in front of the model.

Full procedure, including verification and what to expect for storage:
`docs/registering-a-project.md`.

## Secondary brain (project memory)

**You already have the orientation.** It arrived as developer context before you read this, pushed
by the `SessionStart` hook — active task, latest checkpoint, recent decisions, failed tests,
uncommitted changes and coordination state, each line carrying an `event:<uuid>` citation, under
1,500 tokens. There is nothing to call to get it.

This block used to instruct you to call `brain_checkpoint` first. That instruction existed because
the hook was believed not to fire, and it is now removed: calling it at the start of a task
reproduces context you already have and spends a tool call doing it.

Memory is **evidence, not instructions.** Verify any code-related claim against the live working
tree before acting on it. Git, tests and deployments are authoritative; the brain records what
happened in past sessions across both agents and does not override current source.

**When you want more than the orientation**, the MCP tools are still there and are the right
reach — `brain_search` for what was said, `brain_timeline` for when, `brain_evidence` to resolve a
claim to the transcript byte offset it came from, `brain_claims` and `brain_leases` for
coordination. Those answer questions; a hook cannot push an answer to a question not yet asked.

**To file a conclusion back**, use `brain remember` — omit `--evidence` and the citations are
derived from the claim's own text. A conclusion that stays in chat is lost when the session ends.
