# Source adapter conformance contract

Every coding-agent provider is an evidence adapter. It may understand different native records, but it must not weaken the canonical storage and isolation rules.

## Shared invariants

- Discovery is limited to explicitly configured roots. A directory basename never establishes project identity.
- `fingerprint` is stable for an unchanged source shape and changes when a reviewed native schema changes.
- A committed cursor is monotonic, carries a stable source identity, and never passes the last proven record boundary.
- Re-reading from the start produces identical semantic evidence and identical idempotency keys. Volatile ingestion fields such as `event_id` and `observed_at` are excluded from semantic comparison.
- Re-reading from a committed cursor produces no duplicate native records.
- Every normalized event retains the native `raw_hash`, source locator, source offset, immutable raw value, and explicit source schema.
- The adapter copies `project_id` and `worktree_id` only from the registration-provided `NormalizeContext`. Native directory names and untrusted payload fields cannot replace that scope.
- Unknown complete native shapes become `schema.unknown` evidence. They are retained, not guessed.
- Encrypted or opaque fields remain opaque. An adapter must not fabricate decrypted or summarized content.

The reusable `assert_adapter_conformance` gate checks stable fingerprints, monotonic cursor boundaries, deterministic semantic normalization, raw-hash retention, replay idempotency, and exact project/worktree scoping.

## Format-specific mandatory gates

The common gate cannot mutate every native storage format identically, so each adapter also owns tests for its format:

- Append-only files: incomplete final records do not advance the cursor; rotation reports a capture gap and restarts at byte zero.
- SQLite sources: read-only snapshots include reviewed WAL-visible rows; composite cursors advance only over fully read rows; no migration, checkpoint, vacuum, or write is permitted.
- All formats: malformed data is quarantined with its proven source boundary, and schema drift returns `ReadOutcome::SchemaDrift` without cursor advancement.

## Activation rule

Passing the shared gate is necessary but not sufficient for production activation. A provider activates only after its installed schema fingerprint matches a reviewed fixture/profile and its format-specific mutation, drift, replay, and isolation tests pass.
