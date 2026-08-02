# Writer lease lifecycle

Set `BRAIN_TASK_ID` when launching an agent in a task worktree. If a harness does
not supply a stable native session ID in hook payloads, also set
`BRAIN_NATIVE_SESSION_ID`. The compiled hook shim adds these values to its local
request; it does not write coordination state itself.

At `SessionStart`, the persistent service acquires the task's 30-minute writer
lease. Activity hooks renew only after the five-minute renewal interval. A
graceful `SessionEnd` releases the exact owner/generation; an unexpected process
death leaves the lease to expire naturally.

If another session holds the lease, context names the owner and expiry and tells
the new agent not to edit that worktree. The safe choices are a separate task
worktree or an explicit handoff. Expired takeover and handoff increment the
generation and remain in the append-only coordination audit.

~~~powershell
$env:BRAIN_TASK_ID = "<task-uuid>"
$env:BRAIN_NATIVE_SESSION_ID = "<stable-session-id>" # only when required
claude # or codex / hermes
~~~

Do not reuse one `BRAIN_TASK_ID` in multiple simultaneously writing sessions.
