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
