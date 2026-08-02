# Task and writer-lease schema

Each coordination task is permanently scoped to one registered project and one
worktree. Active tasks have a title, optional validated worktree path/branch, and
an append-only creation/closure history. Task IDs and worktree IDs are UUIDs.

A task can have at most one writer lease. A lease records the harness and native
session, acquisition/renewal/expiry timestamps, and a monotonically increasing
generation. The default duration is 30 minutes and active sessions should renew
at five-minute intervals.

Acquire, renew, release, takeover, and handoff use immediate SQLite transactions.
A non-expired lease rejects another writer. Expiry permits takeover, increments
the generation, and records both owners in `coordination_events`. Release and
handoff require the exact owner and generation so a stale session cannot release
a newer owner's lease.

The coordination tables live in the same project-scoped SQLite file as canonical
evidence but do not alter or replace evidence/memory rows. Cross-project task and
lease operations are rejected before mutation.
