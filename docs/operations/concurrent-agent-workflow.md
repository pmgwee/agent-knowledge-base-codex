# Concurrent agent workflow

Treat a task worktree as the unit of safe parallel work. Never launch two
independent coding tasks in the same checkout merely because they belong to the
same project.

1. Create each task with `brain task create --project <project> --title <title>`.
   The command returns a validated path and an `agent/...` branch under the
   configured worktree parent.
2. Launch Claude Code, Codex, or Hermes from that exact task worktree with
   `BRAIN_TASK_ID` set to the returned task ID. SessionStart acquires the writer
   lease; later activity renews it; SessionEnd releases it.
3. Claim intended files, directories, globs, or symbols before editing with
   `brain claim add`. Definite and probable overlaps identify tasks that should
   coordinate before their edits diverge.
4. Inspect `brain task list`, `brain leases`, and `brain claims` when changing
   scope. A second live writer for a worktree is refused and the current owner
   and expiry are reported.
5. To move work between agents, use the generation-checked handoff command. It
   records a canonical checkpoint and transfers the lease without an unowned
   interval. A crashed owner can be replaced only after lease expiry; takeover
   is recorded in the audit ledger.
6. Before integration, run
   `brain preflight --project <project> --task <task-id> --target main`. Resolve
   Git-proven conflicts and review heuristic same-path warnings, then run the
   project tests. Preflight itself never changes a branch, index, or worktree.
7. Merge through the project’s normal reviewed workflow. Close the task with
   `brain task close`; the report preserves the worktree, reports dirty/ahead
   state, and prints an optional cleanup command. Execute cleanup only after
   confirming the branch is integrated and no uncommitted work remains.

If an agent starts without a task ID, it may read shared project memory but it
does not gain a task writer lease. The warning is intentional: canonical memory
sharing and safe filesystem concurrency are separate controls.
