# Handoff

## State
Dashboard Vercel deploy is live and fixed (https://agent-brain-dashboard-ten.vercel.app/): remote snapshot mode via Upstash Redis — `scripts/push-snapshot.mjs` → key `brain:snapshot` → `app/api/snapshot/route.ts`. Scheduled task `AgentBrain.PushSnapshot` fires every 60s hidden via `scripts/push-snapshot.vbs`. Commits on main: 32cb4ef, c1798e3, 806d8fe, cac1d23.
Separately I diagnosed a `brain-service.exe` exit (Rust repo `..\agent-knowledge-base-codex`) — mechanism root-caused, NOT yet fixed; waiting for your go-ahead since a commit there auto-deploys.

## Next
1. Rust repo: add brain-service fatal-error logging before `main` returns (`crates/brain-service/src/main.rs:102-126`) + capture stderr in the `AgentBrain.Service` task action. Safe, observability-only.
2. Rust repo: make the 5 `try_join!` loops resilient so one transient error stops killing the whole service.
3. Dashboard: done, no active work.

## Context
Vercel provisions `KV_REST_API_URL`/`KV_REST_API_TOKEN`, NOT `UPSTASH_REDIS_REST_*` — route/pusher accept either. `@upstash/redis` auto-deserializes on `get`. `LastTaskResult 267009` = `0x41301` = "task running", not an exit code; `brain-service` exit 1 = `main` returned `anyhow::Err` (error printed to un-captured stderr, never the JSON log, no WER). HOOK-REPAIR-PLAN.md §1 "~800ms gap" is a measurement artifact (PowerShell Start-Process overhead), not a real bug.
