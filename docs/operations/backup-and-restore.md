# Backup and restore

Use an hourly backup destination outside `BRAIN_HOME`. Each backup is staged,
copies live SQLite databases through SQLite's online backup API, copies
immutable segments/blobs and configuration/projection files, hashes every
item, verifies SQLite integrity and supported schema versions, and publishes
`backup.json` last. A staging failure is never a restore point.

Target policy is at most one hour RPO: keep 24 hourly points, 30 daily points,
and 12 monthly points. A daily job must verify its new backup. A monthly job
must restore one backup into a newly created isolated directory, run the
verification suite, record duration and inventory hash, and remove that drill
copy only after success.

Restore never writes into an existing brain directory. It first verifies the
source backup, copies to a sibling staging directory, validates every file and
database again, then atomically publishes the new isolated directory. Point
`BRAIN_HOME` at that directory only after manual review and keep the prior
directory as the rollback target. A checksum, missing file, newer schema, or
SQLite integrity failure leaves the active brain untouched.

Never copy `*-wal` or `*-shm` files manually. They are intentionally excluded
because the online SQLite backup already captures a consistent committed
boundary.

Operator commands:

```powershell
brain backup maintain --root D:\AgentBrainBackups
brain backup prune --root D:\AgentBrainBackups
brain backup prune --root D:\AgentBrainBackups --apply
brain backup drill --backup D:\AgentBrainBackups\<restore-point> --work-root D:\AgentBrainDrills
```

`maintain` creates and verifies a restore point, then applies the locked
24-hour/30-day/12-month policy. `prune` is a dry run unless `--apply` is
present. Retention ignores unknown files, symlinks, staging directories, and
directories without a valid inventory. It removes only direct children of the
resolved backup root.

The drill command always writes a JSON success/failure report under
`<work-root>\reports`. A successful isolated restore is removed only after its
hashes and SQLite databases pass verification. The backup itself and active
`BRAIN_HOME` are never modified.
