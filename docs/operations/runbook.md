# AgentBrain operations runbook

## Daily checks

Run `brain status --json` for each project and `brain service status`. Healthy
operation means the service task exists, capture backlog is falling or zero,
there are no unresolved gaps/schema drifts, and disk pressure is below the
critical boundary. Run `brain diagnose --project <id>` before changing state;
the redacted bundle is the handoff artifact for investigation.

Check `BRAIN_HOME\runtime\logs` for the current structured JSON log and confirm
an hourly restore point exists outside the brain. Optional-provider failure is
degraded—not a canonical-memory outage.

## Incident responses

| Signal | Safe response |
|---|---|
| Capture backlog grows | Confirm source files are reachable, inspect last cursor/error, and restart the service task once. Never advance a cursor manually. |
| Schema drift | Leave that source paused, preserve its raw input, update and fixture-test only its adapter, then explicitly resolve the drift. Other projects/sources continue. |
| Provider timeout/outage | Disable the optional provider and continue on canonical SQLite FTS. Re-enable only after `brain providers status` is usable. |
| CodeGraph stale index | Run `providers index-codegraph` for the exact task worktree. If HEAD still differs, keep it disabled. |
| Disk warning/critical | Move verified backups or remove reproducible caches first. Capture blocks before cursor advancement only at emergency pressure. Never delete canonical segments/blobs. |
| Corrupt segment/blob | Stop compaction, preserve the corrupt artifact, verify the latest backup, and restore in isolation. Hot catalog records remain the diagnostic index. |
| Backup failure | Keep the last verified point, correct destination space/permissions, rerun `backup maintain`, then `backup verify`. Do not prune while no new verified point exists. |
| Restore drill failure | Read the JSON report under the drill root, verify the chosen backup, and run a new drill. Do not cut over to that backup. |
| Lease/path conflict | Run `task leases` and `task claims`; hand off or release the correct generation. Create a separate worktree instead of editing the same path. |
| Merge preflight conflict | Resolve in the task worktree after refreshing the target; do not bypass the preflight evidence. |
| Upgrade failure | Stop the service, reinstall prior exact binary paths, and point only to the untouched prior brain or verified isolated restore. |

## Recovery sequence

1. Stop the current-user service task.
2. Run `backup verify` on the selected point.
3. Run `backup drill` into an empty external drill root.
4. Run `restore` to a new destination; never target the active directory.
5. Run `upgrade check`, project queries, temporal recall, and hook smoke tests
   against the isolated copy.
6. Change `BRAIN_HOME`/service install paths only after review. Preserve the
   former active directory as the rollback target.

The release objectives are at most one hour RPO and two hours RTO on the
primary benchmark corpus. Record real drill duration rather than assuming the
targets from a small development fixture.
