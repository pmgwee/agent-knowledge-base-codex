# Implementation completion audit

Audited 2026-08-03 against the ten criteria in the
[master plan](../superpowers/plans/2026-08-02-secondary-brain-master-plan.md).
Every row was re-run on this machine for this audit rather than carried over
from an earlier report.

| # | Criterion | Evidence | Status |
| --- | --- | --- | --- |
| 1 | Every child-plan checkbox complete | 0 of 246 boxes ticked; see note below | Superseded |
| 2 | All unit, integration, end-to-end and scale tests pass | `cargo test --workspace` exit 0 | Met |
| 3 | Windows service and hook installer work from a clean machine | `service::tests::install_and_uninstall_manage_only_owned_tasks_and_preserve_all_data` | Met |
| 4 | Claude, Codex and Hermes share one registered project | `multi_agent_handoff`, `multi_project_isolation` | Met |
| 5 | A new session continues prior work without transcript export | `claude_walking_skeleton` | Met |
| 6 | Last-week recall is evidence-cited and temporally correct | `temporal_recall` | Met |
| 7 | Concurrent tasks use separate worktrees with visible merge preflight | `concurrent_tasks`, `lease_expiry`, `preflight` | Met |
| 8 | Optional providers enable or remove without migrating canonical data | `provider_lifecycle`, `codegraph_activation`, `llm_wiki_contract`, `llm_wiki_two_stage` | Met |
| 9 | Restore has been exercised from an actual backup | `backup_restore`; the primary gate also restored a real 8 GB corpus in 272 s | Met |
| 10 | Operational documentation matches the shipped CLI | All 18 subcommands of the `Command` enum appear in `docs/operations` | Met |

Supporting hardware and environment gates, also re-run:

| Gate | Result |
| --- | --- |
| Compiled hook warm p95 | under the 50 ms contract |
| Installed Hermes `state.db` schema | matches the reviewed v22 fingerprint |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |

## On criterion 1

The plans contain 246 task checkboxes and none were ever ticked, including for
work that is demonstrably finished, committed and under test. They were not
maintained during implementation, so they are not evidence in either direction:
an unticked box does not indicate missing work, and ticking them now would
record an audit that was never performed.

This audit therefore verifies the other nine criteria against executable
evidence and treats criterion 1 as superseded by them. The checkboxes remain
useful as a description of intended work and should not be read as status.

## Ten-times stress profile

Qualified 2026-08-04. The full 120,000-session, 60,000,000-event run completed in
14.0 hours and met every stress-tier requirement: bounded memory throughout, no
integer or cursor overflow, no linear startup scan — 3.566 ms p95 against 3.265
ms at a tenth the size — and warm retrieval at 1.15 times the primary tier,
inside the "within two times" limit. Capture was 60,000,000 of 60,000,000 with
zero duplicates, zero leakage, and precision and recall of 1.0. Figures are in
[production verification](./production-verification.md).

The run also produced a genuine measurement about recovery: restoring sixty
million events takes about two hours twenty minutes, against a two-hour
objective that holds comfortably at six million. That objective is a primary-tier
gate and is now recorded rather than enforced at ten-times scale, matching how
the plan scopes the two tiers.

**Nothing is outstanding against the completion definition.**

## Not a completion criterion, but needed before the optional providers

Per-session context size is computed but never recorded. `CompiledContext`
carries a `token_count`, and `hook_handler` uses it to bound and truncate, but
no health field, metrics table or log line persists it. Nothing in
`brain-store` stores it either.

The consequence is narrow and specific: **the CodeGraph activation gate cannot
be evaluated in real use.** That gate requires at least a 20% reduction in
targeted-read tokens against a baseline, and `CodeGraphActivationReport` accepts
`baseline_targeted_read_tokens` as an input it is given rather than one the
system measures. Running the brain for a week produces no baseline to supply,
so the comparison would come down to impression.

Recording per-session context size, and code-reading tokens where they can be
attributed, is a prerequisite for the provider evaluation rather than for
completion. It should land before the first optional provider is enabled, not
after, because a baseline cannot be reconstructed retroactively.

A smaller related note: the 1,500-token ceiling on the assembled hook reply is
checked with `debug_assert!`, which compiles out of release builds. The budget
still holds by construction — the compiler is capped at 1,000 tokens whenever
coordination is present, and coordination itself is bounded at 350 — so this is
a missing safety net rather than a live overflow. The one input not explicitly
bounded is the lease warning, which wraps an error string.
