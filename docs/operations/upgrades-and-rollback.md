# Upgrades and rollback

Every persisted boundary has an explicit supported version: hook protocol,
project registry, ledger, normalized event, memory, Markdown projection,
segment manifest, service configuration/API, and backup inventory.

Run `brain upgrade check` while the current system is still active. It is
read-only and refuses newer unknown versions, corrupt SQLite, or invalid segment
manifests. Run `brain upgrade stage --destination <new-directory>` to create a
verified backup, restore it outside the active brain, migrate only that copy,
run integrity and compatibility checks, and prove that the aggregate canonical
raw-event hash is unchanged.

Cutover is deliberately not automatic. Stop the service, keep the prior
`BRAIN_HOME` unchanged, point the service configuration/environment at the
staged directory, and run startup/status/retrieval checks. If startup fails,
stop the new process and point back to the untouched prior directory. Never run
an in-place downgrade or delete the rollback brain until its successor has
passed a verified backup and restore drill.
