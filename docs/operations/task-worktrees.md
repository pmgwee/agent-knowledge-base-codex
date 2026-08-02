# Task worktrees

Create one Git worktree per concurrent task. The brain chooses a collision-free
`agent/<task>-<id>` branch and a path below the explicitly approved worktree
parent, verifies the base commit, asks Git to create the worktree, then registers
its project/worktree identity before creating the coordination task.

~~~powershell
brain task create --project <uuid-or-project-path> --title "OAuth callback" `
  --worktree-parent "C:\agent-worktrees\my-project"
~~~

Launch exactly one active writer in the returned directory. Other agents may
read it, but should use a different task worktree for independent changes.

~~~powershell
brain task list --project <uuid-or-project-path>
brain task close --project <uuid-or-project-path> --task <task-uuid>
~~~

Closing releases the known lease, marks the task completed, and reports dirty
and upstream-ahead state. It never deletes a worktree or branch. The response
contains an exact optional `git worktree remove` command for a user-reviewed
cleanup after changes are merged and the directory is clean.
