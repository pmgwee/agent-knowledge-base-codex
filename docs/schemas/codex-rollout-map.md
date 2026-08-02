# Codex rollout evidence map

The production source is local Codex rollout JSONL below explicitly configured roots (normally `%USERPROFILE%\.codex\sessions`). Discovery accepts only `rollout*.jsonl`; a filename or directory name never assigns the project. Registration validates record-level `cwd` against the canonical project root before binding a source.

This map was reviewed against locally installed Codex records using only envelope field names and types. The committed fixtures are synthetic and contain no copied user prompts, reasoning, credentials, or tool output.

| Native envelope | Native payload | Canonical evidence |
|---|---|---|
| `session_meta` | metadata | `session.started` |
| `turn_context` | turn/cwd/model/summary | `system.observed` |
| `event_msg` | `user_message` | `user.prompted` |
| `event_msg` | `agent_message` | `agent.responded` |
| `event_msg` | `context_compacted` | `session.compacted` |
| `event_msg` | `task_complete` | `task.completed` |
| `event_msg` | `turn_aborted` | `tool.failed` |
| `event_msg` | reasoning | `evidence.opaque` |
| `response_item` | user/assistant `message` | `user.prompted` / `agent.responded` |
| `response_item` | function/custom tool call | `tool.requested` |
| `response_item` | function/custom output | `tool.completed` or `tool.failed` when an explicit failure flag exists |
| `response_item` | encrypted/plain reasoning | `evidence.opaque` |
| `world_state` | state snapshot | `checkpoint.authored` |
| `compacted` | compaction metadata | `session.compacted` |
| anything else | unknown | `schema.unknown` |

## Opaque reasoning rule

Reasoning records retain the immutable raw JSON and SHA-256 hash in the evidence ledger. Their normalized payload contains only type/ID presence metadata plus `retention: raw-only`. It never copies `encrypted_content`, plain reasoning text, or reasoning summaries, and bounded context does not select `evidence.opaque`.

## Cursor and identity

The adapter uses byte cursors plus Windows file identity. It commits only newline-complete records, detects replacement/truncation as rotation, and derives the native session key from the stable rollout source filename because subsequent native records do not repeat the `session_meta` ID. All normalized records receive project/worktree identity only from explicit registration.
