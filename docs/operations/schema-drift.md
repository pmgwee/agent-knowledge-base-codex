# Schema drift containment

The capture service treats an unreviewed provider schema as a source-local
quarantine, not as a service-wide failure.

When an adapter reports drift, the project ledger stores the expected and
observed fingerprints, the last committed cursor, a sample hash, a stable
diagnostic ID, the reason, and timestamps. The cursor is not advanced. Later
sources in the same reconciliation pass continue normally.

On later reconciliations, the quarantined source is fingerprinted but not read.
If its fingerprint returns to the reviewed value, the quarantine resolves and
capture resumes from the preserved cursor. Otherwise it remains paused. The
state survives service restarts.

Use the following command to produce a support-safe JSON report:

```powershell
brain diagnose --project <PROJECT_UUID>
```

The report replaces source paths and file identities with SHA-256 references,
omits native cursor values, and includes only native cursor field names. It
never includes transcript contents, raw evidence, session IDs, or local paths.

`brain status` is unhealthy while any schema drift or capture gap remains
active. A drift must not be manually cleared merely to restart capture; update
and re-review the adapter profile first so automatic recovery is evidence based.
