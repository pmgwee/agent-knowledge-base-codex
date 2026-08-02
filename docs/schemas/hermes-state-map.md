# Hermes state database evidence map

The reviewed production profile targets the locally installed Hermes `state.db` schema version 22. The activation fingerprint covers every `PRAGMA table_info` field for the canonical `sessions` and `messages` tables plus `schema_version`. A mismatch returns `SchemaDrift`, advances no cursor, and reports `fixture_only` until reviewed.

## Read-safety contract

- Open the native database with SQLite read-only flags and `query_only=ON`.
- Do not use `immutable=1`, because that can ignore committed WAL content.
- Never migrate, checkpoint, vacuum, repair, or write Hermes data.
- Read at most 1,000 messages per batch in a transaction-consistent SQLite snapshot.
- Store both the monotonic message ID and `{session_id, message_id}` native cursor position.
- Treat a changed database file identity as rotation and surface a capture gap.

The WAL test keeps a writer open with auto-checkpoint disabled and proves the read-only adapter can see committed WAL rows without changing WAL size.

## Project boundary

Production activation requires an explicit canonical project root. Rows are selected only when the session `cwd` equals or is below that root. A cwd-less child/subagent inherits only its explicitly linked parent session's `cwd`. Unbound sessions are skipped rather than guessed. Same-named or foreign project paths cannot enter the project ledger.

## Evidence mapping

| Native row | Canonical evidence |
|---|---|
| first message in a session | `session.started` plus the message event |
| `role=user` | `user.prompted` |
| `role=assistant` | `agent.responded` |
| `role=tool` with explicit failed/error/denied disposition | `tool.failed` |
| other `role=tool` | `tool.completed` |
| `role=system` | `system.observed` |
| unknown role | `schema.unknown` |
| any reviewed reasoning field | additional `evidence.opaque` record |

Reasoning fields remain only in immutable raw evidence. Normalized opaque payloads contain message identity and `retention: raw-only`, never reasoning text.

## Activation check

```powershell
& .\target\release\brain.exe status --harness hermes --hermes-db "$env:LOCALAPPDATA\hermes\state.db" --json
```

`activation` is `active` only when the database matches the reviewed v22 fingerprint and the currently registered project root is bound. Otherwise it is `fixture_only` with expected/observed fingerprints and a remediation reason.
