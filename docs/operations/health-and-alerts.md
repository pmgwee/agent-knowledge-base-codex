# Health, alerts, and pressure behavior

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
