# LLM Wiki provider

LLM Wiki remains a separate document-knowledge sidecar and may also be opened
as its own Obsidian-compatible vault. It never owns the canonical AgentBrain
directory and must not ingest the raw Claude/Codex/Hermes session firehose.

This integration enables only a reviewed Markdown-vault capability unless an
installed LLM Wiki release exposes a separately reviewed stable structured API.
The reader ignores `.obsidian`, session/transcript directories, symlinks, files
over 1 MiB, and vaults over 10,000 Markdown files. Results carry project and
worktree scope, relative source path, source modification date, retrieval time,
external-document trust, and relevance.

SessionStart never performs a live scan. It may use at most two unexpired cached
items. The first actual prompt may perform one durable, idempotent live search,
bounded to 300 ms, three results, and 600 tokens. Weak, stale, duplicate,
uncited, cross-scoped, or oversized results inject nothing. Cache loss or safe
provider removal changes no canonical event, memory, correction, task, lease,
or Obsidian projection.

```powershell
brain providers configure-llm-wiki --project <id-or-path> --vault D:\LLMWikiVault
brain providers status --project <id-or-path>
brain providers disable --project <id-or-path> llm-wiki
brain providers remove --project <id-or-path> llm-wiki
```

The configured vault must exist outside `BRAIN_HOME`. Disable preserves the
configuration for later use. Remove resets the AgentBrain configuration and
deletes only its cached result rows; it never deletes or edits the LLM Wiki
vault.
