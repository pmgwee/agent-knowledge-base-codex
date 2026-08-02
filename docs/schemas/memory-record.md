# Versioned memory record

Raw events remain canonical. Curated memory is an append-only interpretation of
one or more raw event IDs; it never replaces or edits the evidence ledger.

Each logical memory has a UUIDv7 `id` and one or more immutable `version_id`
rows. A version records scope, kind, title, content, validity interval,
recording time, confidence, authority, status, evidence links, and explicit
supersession links. The current version is the highest committed version number;
older versions remain queryable.

Project memory is accepted only by the matching project ledger and must cite at
least one event already stored in that ledger. Foreign or missing evidence is
rejected inside the same SQLite transaction. Logical IDs cannot change project
or kind between versions.

Global preferences use a separate SQLite store. It accepts only
`global_preferences` scope with kind `preference`; project facts, project
evidence IDs, and project supersession links are rejected. No project memory is
automatically promoted.

Generated Markdown paths have this shape:

```text
projects/<project-uuid>/generated/<kind>/<yyyy>/<mm>/<memory-uuid>.md
global-preferences/generated/preference/<yyyy>/<mm>/<memory-uuid>.md
```

The year and month come from the stable UUIDv7 logical memory ID, so later
versions never move the projection even if their validity interval changes.
