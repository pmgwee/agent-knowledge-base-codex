# Non-mutating merge preflight

Run preflight before merging one task branch into another branch or deployment
target. It resolves both commits and their merge base, compares changed paths,
runs whitespace checks, and asks `git merge-tree` to predict conflicts.

~~~powershell
brain preflight --project <uuid-or-path> --source HEAD --target main
~~~

Preflight never calls `git merge`, `checkout`, `reset`, `stash`, or `add`, and it
does not modify the working tree, branch, or index. A dirty worktree is reported
as a blocker and left untouched. The result includes the Git version and
merge-tree exit code used for the analysis.

`conflicts` are Git-proven merge conflicts. `warnings` are explicitly labelled
heuristics for same-path changes, lockfiles, migration ordering, delete/modify
pairs, or claim/symbol overlaps. Heuristics are reasons to review, not claims
that Git will conflict.

`ready: true` means this read-only analysis found no dirty-tree blocker,
whitespace blocker, missing merge base, unavailable merge analysis, or predicted
Git conflict. It is not permission to merge without reviewing tests and current
deployment state.
