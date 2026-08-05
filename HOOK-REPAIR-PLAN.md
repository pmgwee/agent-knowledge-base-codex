# Hook delivery repair, then Codex hook validation

**Handoff plan — self-contained. Assume no prior conversation context.**

---

## 0. System you are working in

A cross-agent **secondary brain**: a local memory layer shared by Claude Code and Codex. It
captures session transcripts into per-project SQLite ledgers and serves a bounded, evidence-cited
orientation (~1,000–1,500 tokens) at session start.

Rust 1.88.0 (MSVC, edition 2024), 10-crate Cargo workspace at
`C:\Users\quekm\Desktop\projects\agent-knowledge-base-codex`, producing four binaries:

| Binary | Role |
|---|---|
| `brain.exe` | CLI — register, status, query, dashboard, service install |
| `brain-service.exe` | Background capture, consolidation, rediscovery; runs as a Task Scheduler task |
| `brain-hook.exe` | Session-start hook. Talks to the service over a Windows named pipe |
| `brain-mcp.exe` | MCP stdio server for Codex |

### CRITICAL — read before changing any Rust code

**The binaries that run are copies in `~/AgentBrain/bin/`, not `target/release/`.**
Building is not shipping.

**Committing is deploying.** A commit touching `crates/` or `Cargo.*` fires
`.githooks/post-commit`, which runs `scripts/deploy.ps1` detached: build → install to
`~/AgentBrain/bin/` → restart the service → write `~/AgentBrain/runtime/deploy.json`.

It is fail-safe: nothing is replaced unless `cargo build --release` succeeds. A commit that does
not compile leaves the previous deployment live and records the compiler errors.

To install the working tree without committing:

```
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/deploy.ps1
```

Check what is actually installed:

```
~/AgentBrain/bin/brain.exe --brain-home ~/AgentBrain dashboard
```

Read `CLAUDE.md` in the repo root for the full contract and project invariants.

---

## 1. The problem

**The brain has been silently failing to deliver orientations to both agents.**

`brain-hook.exe` gives up after `HOOK_HARD_TIMEOUT = 250 ms`
(`crates/brain-hook/src/lib.rs:13`). That budget must cover pipe connect + the service compiling
the orientation + the reply. It does not fit.

Every invocation returns `{}` with:

```
brain-hook service unavailable: hook request exceeded hard timeout: deadline has elapsed
```

Service log (`~/AgentBrain/runtime/logs/brain-service.<date>.jsonl`) shows the other half:

```json
{"level":"WARN","fields":{"message":"hook pipe request failed","error":"write hook reply"}}
```

The service finishes the orientation, records it, then finds the client already gone. Those
timestamps line up one-for-one with the most recent recorded deliveries. 33 abandoned requests sit
in `~/AgentBrain/runtime/spool/`.

### Measurements taken (2026-08-05, reproduce before trusting)

| Measurement | Result |
|---|---|
| Direct compile via `brain checkpoint` CLI, 5 runs, both projects | **120–144 ms** |
| One hook alone through the pipe | **fails at 250 ms**, ~1 s wall time |
| Two hooks concurrently | both fail |

**The gap is the point.** Direct compile is ~130 ms, but the same work through the pipe costs
roughly a second. The timeout is not merely a little too small — something in the pipe path is
adding ~800 ms. Do not stop at raising the number without understanding this.

---

## 2. Root causes — three, compounding

### 2.1 Every session start fires two hooks

`contains_owned_hook` (`crates/brain-cli/src/install_hooks.rs:25`) and
`contains_owned_codex_hook` (line 123) test idempotency against the **exact executable path**. When
the hook was re-pointed from `target/release/brain-hook.exe` to `~/AgentBrain/bin/brain-hook.exe`,
the path no longer matched, so install **appended a second registration** instead of replacing the
first.

Both `~/.claude/settings.json` and `~/.codex/hooks.json` now contain two brain entries, and the old
`target/release/brain-hook.exe` still exists — so two processes compile two full orientations on
every session start, against a pipe that serves one client at a time.

### 2.2 The pipe server is serial, and the handler blocks the runtime

`crates/brain-service/src/pipe.rs:59` — a loop that accepts one connection, handles it to
completion, then accepts the next.

`ProjectHookHandler::handle` (`crates/brain-service/src/hook_handler.rs:53`) is a **synchronous**
function called inside an async block from `crates/brain-service/src/main.rs`. It performs blocking
SQLite work on a tokio worker thread, alongside consolidation, projections, and rediscovery running
in the same `try_join!`. This is the most likely source of the ~800 ms gap: the accept loop is
starved while blocking work occupies the runtime.

### 2.3 The metric measures the wrong side of the pipe

`record_context_delivery` is called at `crates/brain-service/src/hook_handler.rs:136` — **before**
the reply is written. So `context_deliveries` counts orientations *compiled*, not *received*.

This is why an earlier verification reported hooks "delivering 976–1034 token orientations" when
essentially none were reaching the model. **Any fix that leaves this in place cannot prove itself.**

The existing fail-open intent is correct and must be preserved — a metric failure must never cost a
session its orientation. Only the ordering is wrong.

---

## 3. Phase A — Unblock delivery (do first)

### A1. Re-measure

Reproduce the table in §1 on the current machine state before choosing any number. 250 ms was
reasonable when written and became wrong silently; an arbitrary replacement would do the same.

```powershell
$B="$env:USERPROFILE\AgentBrain\bin\brain.exe"; $H="$env:USERPROFILE\AgentBrain"
$cfg = Get-Content "$H\runtime\service.json" -Raw | ConvertFrom-Json
foreach ($p in $cfg.projects) {
  1..5 | ForEach-Object {
    $sw=[System.Diagnostics.Stopwatch]::StartNew()
    $null = & $B --brain-home $H checkpoint --project $p.project_id 2>&1
    $sw.Stop(); "$($p.project_id) $($sw.ElapsedMilliseconds)ms"
  }
}
```

### A2. Fix installer idempotency, then re-run install

`crates/brain-cli/src/install_hooks.rs`

- `contains_owned_hook` (line 25) and `contains_owned_codex_hook` (line 123): identify brain-owned
  entries by **binary name** (`brain-hook`), not full path, so re-pointing **replaces** rather than
  appends.
- `is_owned_command` (used by `retain` at line 87) and `is_owned_codex_command` (line 188) need the
  same treatment, or old-path entries will never be stripped.
- Raise the hardcoded `"timeout": 1` at **line 52** (Claude) and **line 151** (Codex). This is the
  *outer* budget in seconds and must comfortably exceed the new internal timeout, or the harness
  kills the hook first. Codex's documented default is 600 s — there is no reason to be tight.

Then re-run install for both harnesses and verify each config holds exactly **one** brain entry
pointing at `~/AgentBrain/bin/brain-hook.exe`.

### A3. Raise the internal hard timeout

`crates/brain-hook/src/lib.rs:13` — set `HOOK_HARD_TIMEOUT` from the A1 measurement with headroom.

Keep the comment honest about the tradeoff: this hook **blocks session start**, so the ceiling is a
real cost paid by every session, not free safety margin.

> Note: because a single hook already fails, A2 alone is **not** sufficient. A3 is required.
> If A1 shows the pipe path still costing ~1 s against a ~130 ms compile, treat Phase C item 2 as
> urgent rather than deferred.

### A4. Clear the spool

`~/AgentBrain/runtime/spool/` holds 33 entries from failures already diagnosed. Confirm nothing
downstream replays them, then clear, so the directory becomes a live health signal instead of
accumulated history.

---

## 4. Phase B — Validate Codex hooks

Only meaningful after Phase A, because the timeout masks the answer.

**Background:** Codex **does** support hooks — `SessionStart`, `~/.codex/hooks.json`,
`hookSpecificOutput.additionalContext` (https://learn.chatgpt.com/docs/hooks). Our config and
binary are already correct for it: `render_reply` (`crates/brain-hook/src/lib.rs:96`) emits exactly
that shape for `Harness::Codex`, and a manual invocation proved the service compiles a full Codex
orientation with **46 coordination tokens** (lease and path-claim warnings included).

What is **unproven** is whether Codex actually *invokes* the hook on this build. The 250 ms timeout
hides it, because a Codex-fired hook fails identically to a hand-fired one.

**Procedure**

1. Restart Codex so it re-reads `~/.codex/hooks.json`.
2. Open a session in a registered project. Do nothing else.
3. Query the ledger:

```sql
SELECT harness, event_name, native_session_id, total_tokens, coordination_tokens
FROM context_deliveries
WHERE harness='codex' ORDER BY delivered_at_ns DESC LIMIT 5;
```

Ledger paths are listed in `~/AgentBrain/runtime/service.json` under each project's `ledger_path`.

**The discriminator is `native_session_id`.** Diagnostic runs used ids like `diag-0001` /
`probe-single`. A genuine Codex session carries a real session id.

| Outcome | Meaning | Action |
|---|---|---|
| Real-id `codex/SessionStart` row | Codex fires hooks | Codex gains deterministic push including coordination warnings; MCP returns to its intended role as optional depth |
| No row **and** no new codex spool entry | Codex never invoked it | MCP stays the sole delivery channel; prioritise adding coordination to `brain_checkpoint` |
| No row **but** a new codex spool entry | Codex fired it, delivery still failed | Phase A was insufficient — return to A1 |

---

## 5. Phase C — Structural (after B)

1. **Concurrent pipe.** `crates/brain-service/src/pipe.rs` — create the next `NamedPipeServer`
   instance *before* handling the connected one, and `tokio::spawn` the handler so parallel sessions
   stop queueing.
2. **Move compile off the reactor.** Wrap `ProjectHookHandler::handle` in `spawn_blocking` so
   blocking SQLite work cannot stall the accept loop. *Promote this if A1 confirms the ~800 ms gap.*
3. **Record deliveries only on success.** Have the handler return the reply **plus** a pending
   `ContextDelivery`, recorded by `handle_connected` (`crates/brain-service/src/pipe.rs:81`) after
   `flush()` succeeds. Preserve fail-open semantics; stop counting compilations as receipts.
4. **Surface hook health on the dashboard.** Spool depth and delivery-failure rate, beside the
   existing Deployment panel. This entire problem was invisible because nothing displayed it.
   Dashboard lives at `../agent-brain-dashboard` (separate repo, pnpm/Next.js). Its TypeScript
   types in `lib/snapshot-types.ts` mirror the Rust structs **by hand** — changing a field in
   `crates/brain-cli/src/dashboard.rs` means updating that file too.

---

## 6. Do NOT touch these

Deliberate decisions, already validated. Reverting any of them is a regression:

| Item | Why it stays |
|---|---|
| `[mcp_servers.brain]` in `~/.codex/config.toml` | Hooks and MCP coexist by design. Only `brain_checkpoint` overlaps with the hook; `brain_search`, `brain_timeline`, `brain_evidence`, `brain_claims`, `brain_leases` have no hook equivalent. Removing it leaves Codex unable to query the brain mid-session. |
| `AGENTS.md` in registered projects | Codex loads it deterministically at session start; also the fallback if a hook fails. |
| Binaries in `~/AgentBrain/bin/` | Survives `cargo clean`; the deploy pipeline's install target. |
| MCP delivery recording (commit `1fbfae0`) | **More** important if hooks work — with two Codex channels it is the only way to tell which delivered. Also the token baseline needed before enabling CodeGraph / LLM Wiki. |

---

## 7. Project invariants — do not break

- **Cross-project isolation is a locked release criterion.** Zero leakage. Transcript ownership is
  decided by the transcript's own recorded `cwd`, never by directory proximity. See
  `crates/brain-service/src/rediscover.rs` and the `discovery_never_crosses_a_project_boundary` test.
- **Evidence is append-only.** Supersede; never delete or rewrite.
- **Live state outranks memory.** Git, tests, and deployments are authoritative.
- **The context budget is a contract.** 1,000–1,500 tokens normal, 3,000 hard max, with
  `event:<uuid>` citations. Adding a field means removing one.
- **Cursors are keyed by source.** Never reorder or replace an entry in a project's source list —
  that orphans its cursor and re-ingests captured evidence. Append only.

---

## 8. Verification

### Phase A landed when all four hold

1. **Exactly one brain hook per config**, pointing at `~/AgentBrain/bin/brain-hook.exe`:

```powershell
python -c "import json,os;print(json.dumps(json.load(open(os.path.expanduser('~/.claude/settings.json')))['hooks']['SessionStart'],indent=2))"
python -c "import json,os;print(json.dumps(json.load(open(os.path.expanduser('~/.codex/hooks.json')))['hooks']['SessionStart'],indent=2))"
```

2. **A hand-invoked hook returns real context, not `{}`, with empty stderr:**

```powershell
$payload='{"session_id":"verify-1","cwd":"C:\\Users\\quekm\\Desktop\\projects\\agent-knowledge-base-codex","hook_event_name":"SessionStart","source":"startup"}'
$inF="$env:TEMP\v.json"; [System.IO.File]::WriteAllText($inF,$payload,(New-Object System.Text.UTF8Encoding $false))
Start-Process "$env:USERPROFILE\AgentBrain\bin\brain-hook.exe" -ArgumentList '--harness','claude-code' `
  -NoNewWindow -Wait -RedirectStandardInput $inF -RedirectStandardOutput "$env:TEMP\vo.txt" -RedirectStandardError "$env:TEMP\ve.txt"
Get-Content "$env:TEMP\vo.txt"   # expect hookSpecificOutput.additionalContext
Get-Content "$env:TEMP\ve.txt"   # expect empty
```

3. **No new `hook pipe request failed / write hook reply`** in
   `~/AgentBrain/runtime/logs/brain-service.<date>.jsonl`.

4. **Start a fresh Claude Code session:** a new `claude-code/SessionStart` row appears **and the
   spool directory does not grow.**

> Point 4 is the one that matters. A delivery row alone was never proof — that is the whole reason
> this plan exists.

### Phase B landed when

A `codex/SessionStart` row exists with a **real** Codex `native_session_id` and non-zero
`coordination_tokens`.

### Phase C landed when

Two Claude Code sessions started simultaneously both receive orientations, delivery count equals
hook invocations, and the spool stays empty.

### Gates (every phase)

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Scale gates are `#[ignore]` by default. Do not run the stress gate casually — it takes hours.

**Remember: committing deploys.** After committing, confirm with
`~/AgentBrain/bin/brain.exe --brain-home ~/AgentBrain dashboard` that the `deployment` section
reports `up_to_date: true` and no `drifted_binaries`.
