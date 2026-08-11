# Cross-Harness Token Savings Benchmark Implementation Plan

> **Execution rule:** Follow this plan test-first. Do not launch Claude Code or Codex benchmark
> sessions while implementing it. A real smoke or claimable run always requires a separate,
> explicit `brain benchmark run --execute` operator action.

**Goal:** Ship an auditable Claude Code + Codex benchmark that can make a causal token-savings
claim only when exact native usage, configuration isolation, statistical confidence, and blind
quality gates all pass.

**Architecture:** Keep the benchmark domain in `brain-cli`, preserve the existing scale benchmark
as `brain benchmark scale`, store append-only run artifacts under the project-scoped brain runtime,
and add only compact summaries to dashboard snapshot schema v2. Use file artifacts rather than a
ledger migration for experimental results. Add one narrow store query for observational native
usage records. The dashboard reads summaries only; raw answers never leave the local artifact
directory.

**Technology:** Rust 1.88, clap, serde/serde_json, SHA-256, atomicwrites, time, SQLite/rusqlite,
Next.js 16, React 19, TypeScript, Tailwind, Vitest, Testing Library.

---

## Non-negotiable measurement rules

- Claude total = input + cache-creation input + cache-read input + output.
- Codex total = the last monotonic native `total_token_usage.total_tokens`; never sum cumulative
  `token_count` events and never add cached/reasoning breakdowns twice.
- The matched unit is `(task_id, harness, repeat)` and the estimate is the ratio of matched sums.
- Token pairs stay in the calculation even when their answers fail grading.
- Bootstrap whole task clusters with a recorded seed and 10,000 resamples.
- A savings statement is possible only when every validity gate, every per-harness interval, and
  the overall/per-harness quality non-inferiority gates pass.
- The implementation never rewrites live Claude or Codex settings.
- The old dashboard metric remains available, renamed **Context delivery**.

## Task 1: Introduce benchmark domain types without changing behavior

**Files:**

- Create: `crates/brain-cli/src/token_benchmark/mod.rs`
- Create: `crates/brain-cli/src/token_benchmark/model.rs`
- Modify: `crates/brain-cli/src/lib.rs`
- Test: `crates/brain-cli/tests/token_benchmark_model.rs`

1. Write serialization round-trip tests for harness, condition, task stratum, suite manifest,
   native usage components, planned sample, normalized sample, grade, validity check, estimates,
   report status, and dashboard summary.
2. Run `cargo test -p brain-cli --test token_benchmark_model`; confirm it fails because the module
   does not exist.
3. Implement the minimum strongly typed model. Make ids and schema versions explicit. Reject zero
   control totals, duplicate task ids, absent strata, repeats below one, and non-finite estimates.
4. Re-run the focused test and commit: `feat: add token benchmark domain model`.

## Task 2: Parse exact native Claude and Codex token records

**Files:**

- Create: `crates/brain-cli/src/token_benchmark/usage.rs`
- Create: `crates/brain-cli/tests/fixtures/token-benchmark/claude-result.json`
- Create: `crates/brain-cli/tests/fixtures/token-benchmark/claude-missing-cache.json`
- Create: `crates/brain-cli/tests/fixtures/token-benchmark/codex-events.jsonl`
- Create: `crates/brain-cli/tests/fixtures/token-benchmark/codex-decreasing.jsonl`
- Test: `crates/brain-cli/tests/token_benchmark_usage.rs`

1. Write fixture tests that assert the exact Claude formula, require all four fields, reject
   malformed/negative counters, and retain raw components.
2. Write Codex tests proving cumulative snapshots are not summed, the last monotonic record wins,
   decreasing/malformed/missing records fail, and breakdown fields are retained without being
   added to `total_tokens`.
3. Run the focused test and observe the expected compile failure.
4. Implement parsers that accept readers/strings and return a typed `NativeUsage` or a structured
   invalid-usage reason. Preserve schema evidence needed by the audit report.
5. Re-run tests and commit: `feat: normalize native harness token usage`.

## Task 3: Build the deterministic paired execution matrix

**Files:**

- Create: `crates/brain-cli/src/token_benchmark/matrix.rs`
- Test: `crates/brain-cli/tests/token_benchmark_matrix.rs`

1. Test stable output for a fixed seed, a different order for a different seed, adjacent control
   and treatment samples, balanced condition-first order, counterbalanced harness order, unique
   sample ids, and identical non-condition metadata within a pair.
2. Implement a small documented deterministic PRNG locally so results do not depend on a new RNG
   dependency or version-specific shuffle behavior.
3. Store the algorithm name/version and seed in the manifest.
4. Commit: `feat: plan deterministic paired benchmark matrix`.

## Task 4: Implement matched-sum statistics and quality gates

**Files:**

- Create: `crates/brain-cli/src/token_benchmark/statistics.rs`
- Create: `crates/brain-cli/src/token_benchmark/report.rs`
- Test: `crates/brain-cli/tests/token_benchmark_statistics.rs`
- Test: `crates/brain-cli/tests/token_benchmark_report.rs`

1. Test absolute saved tokens and ratio-of-sums against a hand-calculated fixture where averaging
   pair percentages gives the wrong answer.
2. Test deterministic 10,000-resample task-clustered bootstrap output and prove repeats of one task
   are resampled together.
3. Test paired task-clustered success-difference intervals, including a lower bound exactly -0.02.
4. Test statement selection at all boundaries: proven, interval touches zero, missing pair,
   treatment-only critical regression, per-harness quality failure, invalid configuration, and
   incomplete run. Assert that quality failures remain in the token denominator.
5. Implement statistics and an exhaustive report-state evaluator. Generate prose only from the
   evaluator; do not accept free-form claim text in persisted reports.
6. Commit: `feat: gate token savings claims with paired evidence`.

## Task 5: Add append-only, resumable run artifacts

**Files:**

- Create: `crates/brain-cli/src/token_benchmark/artifacts.rs`
- Test: `crates/brain-cli/tests/token_benchmark_artifacts.rs`

1. Test project/run path isolation, create-new semantics, normalized JSONL append, content-addressed
   raw output, SHA-256 audit entries, interrupted-run resume, duplicate sample id idempotency,
   conflicting duplicate rejection, and non-destructive retirement.
2. Implement paths beneath
   `BRAIN_HOME/runtime/token-benchmarks/<project_id>/<run_id>/` only. Resolve and verify targets
   remain under that root before every write.
3. Write manifests/reports atomically. Treat `samples.jsonl` and `grades.jsonl` as append-only.
   Write `retired.json` rather than deleting anything.
4. Commit: `feat: persist resumable benchmark artifacts`.

## Task 6: Implement frozen-snapshot and condition-isolation preflight

**Files:**

- Create: `crates/brain-cli/src/token_benchmark/preflight.rs`
- Create: `crates/brain-cli/tests/fixtures/token-benchmark/condition-control.json`
- Create: `crates/brain-cli/tests/fixtures/token-benchmark/condition-treatment.json`
- Test: `crates/brain-cli/tests/token_benchmark_preflight.rs`

1. Test recursive file hashing, executable hash/version capture, repository/brain/suite commit
   capture, immutable snapshot hashing, and before/after hashes for the three production settings
   files.
2. Test normalized condition diff: only the explicit Agent Brain hook/MCP/endpoint allowlist may
   differ; a changed CodeGraph tool, model, effort, permission, skill, or non-brain hook aborts.
3. Test that all generated configs live under the benchmark run directory and that no API accepts a
   production settings path as a write target.
4. Implement a materialized, inspectable command/config plan for both harnesses. Claude uses an
   explicit generated settings source; Codex uses ignored user config plus explicit benchmark
   overrides. Brain-on points only to the frozen benchmark endpoint. Brain-off exposes no brain
   hook, MCP, or CLI access.
5. Commit: `feat: preflight isolated benchmark conditions`.

## Task 7: Add the safe dry-run/default runner

**Files:**

- Create: `crates/brain-cli/src/token_benchmark/runner.rs`
- Test: `crates/brain-cli/tests/token_benchmark_runner.rs`

1. Build a fake process runner and test that default execution only prints/materializes commands,
   while `execute=true` is the only path that spawns a child.
2. Test sequential execution, adjacent pair order, retry accounting, timeout/crash artifacts,
   completed-sample resume, native parser integration, live-config hash recheck after every sample,
   and an injected hard-kill path that leaves production files byte-identical.
3. Test an endpoint guard that refuses a production pipe/home and a checkout guard that refuses any
   registered project root.
4. Implement the runner behind a process-runner trait. Record stdout/stderr before normalization and
   never silently retry a completed valid sample.
5. Commit: `feat: run token benchmark without live config mutation`.

## Task 8: Add blind grading export/import

**Files:**

- Create: `crates/brain-cli/src/token_benchmark/grading.rs`
- Test: `crates/brain-cli/tests/token_benchmark_grading.rs`

1. Test opaque randomized ids, a public sheet without harness/condition/run metadata, a separate
   key, deterministic-test results in the sheet, append-only grade import, grader identity/version,
   duplicate id idempotency, and conflicting-decision rejection.
2. Implement CSV escaping without exposing the key in dashboard artifacts.
3. Commit: `feat: add blind benchmark grading workflow`.

## Task 9: Turn `brain benchmark` into a backward-compatible command family

**Files:**

- Modify: `crates/brain-cli/src/main.rs`
- Modify: `crates/brain-cli/src/lib.rs`
- Modify: `crates/brain-cli/src/benchmark.rs`
- Test: `crates/brain-cli/tests/benchmark_cli.rs`

1. Add CLI parse/behavior tests for `scale`, `preflight`, `run`, `grade export`, `grade import`,
   `report`, `show`, `retire`, and `production`.
2. Preserve the existing corpus benchmark exactly under `brain benchmark scale`. Provide a clear
   compatibility error for legacy top-level scale flags instead of reinterpreting them.
3. Make `run` dry by default and require the literal `--execute` flag. Print the run id, artifact
   directory, sample count, and whether any model process was launched.
4. Ensure JSON modes contain stable schema versions and no raw answer content in `show`.
5. Commit: `feat: expose token benchmark command family`.

## Task 10: Preregister a balanced benchmark suite

**Files:**

- Create: `benchmarks/token-savings/v1/suite.json`
- Create: `benchmarks/token-savings/v1/README.md`
- Create: `benchmarks/token-savings/v1/rubrics/*.md`
- Test: `crates/brain-cli/tests/token_benchmark_suite.rs`

1. Add 32 immutable tasks, eight in each approved stratum. Fix prompt, fixture commit, allowed
   files, turn/tool/time limits, reference facts, rubric path, automated checks, critical-regression
   rule, and combined-headline eligibility.
2. Test exact balance, unique ids, committed fixture references, valid paths, bounded-change checks,
   and no answer/condition labels in prompts.
3. Add pilot selection metadata for at least ten tasks/two repeats. Main defaults to five repeats.
4. Commit before any live benchmark: `test: preregister token savings benchmark suite`.

## Task 11: Normalize observational production usage efficiently

**Files:**

- Modify: `crates/brain-store/src/lib.rs`
- Create: `crates/brain-cli/src/token_benchmark/production.rs`
- Test: `crates/brain-store/tests/native_usage.rs`
- Test: `crates/brain-cli/tests/production_token_usage.rs`

1. Write store tests for a project-scoped, time-bounded query that returns raw native usage events
   with session id, harness, timestamp, source id, source offset, and raw JSON. Prove another
   project's events cannot leak.
2. Test Claude message-id deduplication/final monotonic usage and Codex final cumulative usage per
   session. Test 1/7/30-day totals, sessions, median, cache/output share, and provenance.
3. Implement the narrow query and normalizer. Label the resulting type `observational` in its schema.
4. Commit: `feat: summarize observational native token usage`.

## Task 12: Extend dashboard snapshot schema compatibly

**Files:**

- Modify: `crates/brain-cli/src/dashboard.rs`
- Modify: `crates/brain-cli/src/lib.rs`
- Test: `crates/brain-cli/tests/dashboard.rs`

1. Add failing snapshot tests for schema v2, optional latest non-retired benchmark summary,
   observational production windows, no raw answer fields, and cross-project isolation.
2. Add a backward-safe optional benchmark field and production trend. Missing artifacts produce
   `None`, never zero savings. A retired or invalid newer run must not resurrect stale marketing
   prose.
3. Commit: `feat: publish benchmark summaries in dashboard snapshot`.

## Task 13: Build the dashboard panels in an isolated dashboard worktree

**Repository:** `C:/Users/quekm/Desktop/projects/agent-brain-dashboard`

**Files:**

- Move: `features/token-baseline/token-baseline-panel.tsx` to
  `features/context-delivery/context-delivery-panel.tsx`
- Create: `features/token-savings/token-savings-panel.tsx`
- Create: `features/token-savings/token-savings-panel.test.tsx`
- Create: `features/token-savings/production-token-trend.tsx`
- Create: `features/token-savings/production-token-trend.test.tsx`
- Modify: `lib/snapshot-types.ts`
- Modify: `app/page.tsx`
- Modify: `package.json`
- Modify: lockfile and test configuration

1. Create a `codex/token-savings-benchmark` dashboard worktree after verifying the source worktree is
   clean. Never edit the user's current dashboard worktree in place.
2. Add type fixtures and component tests for Proven, Inconclusive, Quality blocked, Invalid, No
   benchmark, an old schema-v1 snapshot, and a per-harness failure next to a combined estimate.
3. Test that production trend always contains `Observational` and never contains causal savings
   wording.
4. Rename the current panel heading to **Context delivery** without deleting any delivery data.
5. Implement the measured panel, native off/on/saved totals, intervals, metadata, quality metrics,
   and accessible audit disclosure. Use neutral styling for invalid/inconclusive/negative results.
6. Implement the 1/7/30-day observational production panel.
7. Run component tests, `npm run lint`, and `npm run build`. Commit in the dashboard worktree:
   `feat: show audited token savings benchmark`.

## Task 14: Synthetic end-to-end and release gates

**Files:**

- Create: `crates/brain-cli/tests/fixtures/token-benchmark/synthetic-run/**`
- Create: `crates/brain-cli/tests/token_benchmark_e2e.rs`
- Modify: `scripts/token-ab.ps1`
- Modify: `docs/status.md`
- Modify: `docs/roadmap.md`
- Modify: `docs/enhancement-review-plan.md`

1. Create a zero-cost run with known Claude/Codex raw records, grades, one failed answer that remains
   in token totals, and a hand-calculated expected report. Assert byte-stable deterministic output.
2. Test contamination by comparing event/job counts before/after the synthetic run and by refusing a
   registered checkout/production endpoint.
3. Replace `scripts/token-ab.ps1` with a compatibility message or wrapper that points to the new
   CLI; it must no longer rewrite live settings.
4. Update status/roadmap/review-plan wording: implementation shipped, synthetic verification
   passed, live smoke and claimable run remain explicit operator actions, and no percentage exists
   until the quality-gated report is Proven.
5. Run focused tests, then `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
   -D warnings`, and `cargo test --workspace`.
6. Commit Rust changes and wait for the automatic deployment. Verify `brain dashboard` deployment
   reports the new source commit and success.
7. Do **not** run the two-task paid smoke or main benchmark. Hand the exact commands and expected
   artifact locations to the operator.

## Completion evidence

Before claiming completion, record:

- focused parser, matrix, statistics, report, artifact, preflight, runner, grading, CLI, production,
  dashboard, and end-to-end test results;
- full Rust fmt/clippy/test results;
- dashboard component/lint/build results;
- clean status and commit ids for both worktrees;
- deployed binary source commit/status;
- proof that the three live settings hashes are unchanged;
- explicit statement that zero Claude/Codex benchmark sessions were launched during implementation.

The first live smoke and the first claimable benchmark are follow-on operator actions, not hidden
completion steps.
