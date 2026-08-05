# Handoff

## State

**Dashboard** is live and working: https://agent-brain-dashboard-ten.vercel.app/
Remote snapshot mode via Upstash Redis — `scripts/push-snapshot.mjs` → key
`brain:snapshot` → `app/api/snapshot/route.ts`. Scheduled task `AgentBrain.PushSnapshot`
fires every 60 s hidden via `scripts/push-snapshot.vbs` (which correctly propagates node's
exit code via `WScript.Quit sh.Run(...)`). The `.vbs` runs `node scripts/push-snapshot.mjs`
with cwd set to the repo root so `.env.local` is found.

**brain-service exit — RESOLVED.** The recurring service crash (and the Codex hook
"exit 1") were root-caused and fixed in commit `f63640b`. The cause was **not** a
`main()` `anyhow::Err` (the earlier diagnosis below was wrong) — it was a **DLL load
failure**: the binaries were dynamically linked to `VCRUNTIME140.dll`, which is not on
the DLL search path in restricted launch contexts (Codex's hook sandbox, Task Scheduler
session 0). The fix: `.cargo/config.toml` sets `target-feature = +crt-static` for the
MSVC target, baking the CRT into each binary so they are self-contained. Service has been
stable since.

## What's left (optional hardening — Phase C)

1. **Concurrent pipe accept** (`crates/brain-service/src/pipe.rs`): the accept loop handles
   one connection at a time. At ~140 ms per hook this is fine, but two truly-simultaneous
   session starts would serialize.
2. **Record deliveries only after flush** (`hook_handler.rs` / `pipe.rs`): the metric
   counts orientations *compiled*, not *received by the client*. For Codex (MCP pull ==
   receipt) this is moot; for Claude Code's hook it could theoretically overcount.
3. **Dashboard: surface hook health** (spool depth, delivery-failure rate). The 4-hour
   stale-dashboard incident was invisible because nothing displayed capture/push health.
4. **`#33229` phantom-task guard**: if you switch to the Codex CLI (where hooks fire),
   ChatGPT Desktop's internal suggestion-tasks will also trigger the brain hook with
   non-resumable session ids. Add a resumability guard then. Moot for Codex Desktop.

## Context

- **Dashboard data flow:** `brain-service` captures → `brain.exe dashboard` reads the
  ledger → `push-snapshot.mjs` ships the JSON to Upstash → Vercel `/api/snapshot` reads
  it. If the service stops capturing, the dashboard goes stale even though the pusher
  keeps running (the pusher pushes whatever `brain.exe dashboard` returns, which is stale
  if no new captures landed). This is what caused the 4-hour stale dashboard on Aug 5.
- **Vercel provisions** `KV_REST_API_URL` / `KV_REST_API_TOKEN`, NOT
  `UPSTASH_REDIS_REST_*` — the route/pusher accept either naming. `@upstash/redis` auto-
  deserializes on `get`.
- `LastTaskResult 267009` = `0x41301` = "task currently running" — not an exit code.
- **HOOK-REPAIR-PLAN.md** §1 "~800 ms gap" was a **measurement artifact** (PowerShell
  `Start-Process -Wait` stream-drain overhead), not real latency. True hook latency is
  ~140 ms (compile-bound), confirmed via `ExitTime - StartTime` process-lifetime timing.
- **Git Bash hand-fire warning:** `printf '%s' "$payload" | brain-hook` can mangle the
  JSON payload in a way that makes brain-hook's `serde_json::from_slice` fail silently,
  returning `{}`. Real Claude Code / Codex payloads never hit this. Use a UTF-8 file
  redirect (`[IO.File]::WriteAllText` + `-RedirectStandardInput`) for manual testing.
