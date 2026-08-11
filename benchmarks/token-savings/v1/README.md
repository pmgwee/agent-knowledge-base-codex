# Token savings benchmark v1

This is the preregistered, balanced task suite for the causal Claude Code + Codex comparison.
It contains 32 tasks: eight in each approved stratum. All tasks use repository fixture commit
`737b362aba76f3dfd557e5e38c804f60d28d3be5`.

The twelve ids in `pilot_task_ids` are for a non-claimable two-repeat schema/isolation smoke. A
claimable run uses all 32 tasks and at least five repeats. Neither run starts without an explicit
`brain benchmark run --execute` command.

The four `configs/*.json` files are normalized configuration declarations used by preflight. They
are not production settings and must never be copied over a live harness configuration. The only
allowed control/treatment differences are the `agent_brain` hook/MCP blocks and benchmark-scoped
brain endpoint environment values.

## Native launch profiles

`execution-templates.example.json` documents the audited launcher contract. Copy it outside the
committed suite, replace all three program paths with absolute, reviewable launchers, and pass it to
`brain benchmark preflight --execution-templates <file>`. A launcher must emit Claude's final JSON
or Codex's JSONL unchanged on stdout and implement `--version`; preflight pins both its version and
SHA-256. It must interpret the immutable condition declaration without reading or writing live
harness settings.

Control and treatment use the same launcher, arguments, timeout and non-brain environment. Only the
condition-config contents and treatment's benchmark-scoped `BRAIN_HOME` / `BRAIN_PIPE_NAME` differ.
The runner refuses a production endpoint, fresh-clones every attempt, starts a writable copy of the
frozen brain only for treatment, checks for a real `SessionStart` delivery in that isolated ledger,
and rechecks the three live settings hashes after every sample.

The example intentionally contains non-runnable placeholder paths. This prevents an operator from
turning a generic template into paid sessions before the harness-specific launchers have been
reviewed and pinned; `--execute` never guesses how a local installation is wired.
