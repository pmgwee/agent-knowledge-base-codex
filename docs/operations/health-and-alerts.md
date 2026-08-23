# Health, alerts, and pressure behavior

## Session lifecycle truth

Dashboard schema v3 and `brain sessions status` fold append-only lifecycle facts into active,
stale-open, closed, and historical-uninstrumented sessions. Four channels remain independent:
SessionStart orientation, UserPromptSubmit push, on-demand Brain MCP pull, and SessionEnd/capture.
`not requested` and `healthy silence` are neutral states. They are never inferred as failure or zero
delivery.

Use `hook_received` to prove invocation; `context_deliveries` proves only that text was returned.
`reply_flushed` proves the client received the bytes. MCP request/success/failure and capture-caught-up
are separate receipts. A missing exact native session correlation is stored as `unattributed`, never
assigned by timestamp proximity.

Red alerts cover stopped/missing/drifted service binaries, unreachable pipe, reply-flush failure,
cross-project leakage, and boundary persistence failure. Amber covers SessionStart not observed
within 10 seconds, capture lag, stale-open sessions, truncation, and prompt-push errors. For Codex
activity without a hook receipt, inspect `[hooks.state]` trust first. The UI polls toward a 10-second
near-real-time target, but remote snapshot freshness is producer-limited; the generated timestamp is
authoritative and the system does not claim a continuous heartbeat.

Service health reports per-source cursor, backlog, quarantine, gaps, schema
drift, last success/error, project event totals, disk bytes/percent, active
degradation state, backup/drill timestamps, projection/consolidation lag,
provider failures, and hook latency percentile fields.

Default free-space thresholds are 5 GiB/10% warning, 2 GiB/5% critical, and
256 MiB emergency. Warning pauses provider refresh and Basic Memory work.
Critical additionally pauses Markdown projection, consolidation, and cold-cache
growth. Capture remains protected until the emergency threshold, where it is
visibly blocked before reading a source, so its committed cursor cannot advance.

Recovery requires at least 6 GiB and 12% free space for two consecutive checks.
This hysteresis prevents optional subsystems from repeatedly stopping and
starting. Pressure handling never deletes raw evidence, retained backups,
unrecognized files, or an external provider vault. Operators free space by
removing reproducible caches first, then reviewed expired backups, or by moving
the complete brain through verified backup/restore.
