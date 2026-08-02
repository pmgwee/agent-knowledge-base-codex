# Storage tiers and compaction

Canonical capture first lands in the per-project SQLite WAL ledger. Searchable
identity, timestamps, FTS terms, evidence links, and segment locators remain in
SQLite for bounded startup and historical queries.

At 256 MiB or a month boundary, an operator or maintenance cycle may seal a
completed event range. Sealing writes deterministic JSONL, compresses it with
the pinned Zstd level, verifies compressed and uncompressed SHA-256 hashes and
event count, publishes the manifest last, and then commits catalog locators.
An interruption before manifest/catalog publication leaves every hot row
queryable; replaying the same range is idempotent.

Compaction is a separate step. It refuses a segment referenced by unfinished
consolidation work, re-verifies the immutable segment, retains FTS/search text
and locators, and only then replaces duplicate hot payload bodies. Normal
startup and scoped search do not decompress cold segments. Explicit evidence
expansion verifies and reads the segment; checksum errors are returned as
evidence-unavailable errors.

Large identical payloads use a brain-wide SHA-256 blob object while each
project ledger keeps its own reference. Physical deduplication never permits a
query to cross a project boundary.
