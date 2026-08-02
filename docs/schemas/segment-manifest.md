# Segment manifest schema

Segment manifest format version `1` describes one immutable, project-scoped
`.jsonl.zst` evidence segment. The manifest is published only after the data
file has been flushed, decompressed, checksummed, counted, and parsed.

Required identity fields are `project_id`, `segment_id`, `first_event_id`, and
`last_event_id`. Integrity fields are SHA-256 hashes and raw/compressed byte
counts. Temporal fields record the minimum and maximum event occurrence time
and publication time. `data_file` is a filename relative to the manifest; paths
outside the segment directory are not generated.

The hot SQLite catalog keeps event identity, ordering, project scope, raw hash,
and the segment locator. A manifest or data checksum failure makes cold evidence
explicitly unavailable; it never causes the system to invent or silently omit a
canonical fact. Segment files are append-only backup inputs and are never
rewritten in place.
