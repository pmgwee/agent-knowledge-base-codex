# Cross-Harness Token Savings Benchmark Design

**Status:** Approved design direction; written-spec review pending  
**Date:** 2026-08-11  
**Owner:** Agent Brain  
**Repositories:** `agent-knowledge-base-codex`, `agent-brain-dashboard`

## Purpose

Replace the dashboard's current delivery-volume panel with an evidence-backed answer to the
project's headline question:

> How many tokens does the current Second Brain save in Claude Code and Codex while preserving
> answer quality?

The answer must be causal, reproducible, auditable, and automatically worded from measured data.
The dashboard may display an official savings percentage only when every validity and quality gate
in this specification passes.

## Current state and the gap

The existing dashboard panel called **Token baseline** counts only context sent by the brain. For
`agent-knowledge-base-codex`, the live 30-day snapshot on 2026-08-11 contained 159 deliveries,
161,336 delivered tokens, a 1,014.7-token mean, and a 1,453-token maximum. Those figures answer
"how much context did the brain inject?" They do not measure total model usage and have no
without-brain counterfactual, so they cannot support a savings percentage.

`scripts/token-ab.ps1` is a useful pilot but not the final instrument. It runs only Claude Code,
omits Claude cache-creation tokens, uses five historical question-answer tasks, writes the live
Claude settings file between conditions, and does not publish a durable benchmark result to the
dashboard. It must become a compatibility wrapper or be retired after the new orchestrator ships.

The cumulative totals shown by the Claude and Codex profile screens are secondary audit signals.
They are rounded, mix unrelated tasks and configurations, and cannot establish a counterfactual.
They must never be used as the numerator or denominator of the official savings percentage.

## Decisions already approved

1. Measure both Claude Code and Codex.
2. Use native provider-reported token counters, never the brain's heuristic text counter, for the
   benchmark result.
3. Compare the same task, repository revision, harness configuration, model, effort, tools, and
   limits with the Second Brain off and on.
4. Keep CodeGraph and every non-brain capability identical between the two official conditions.
5. Randomize and pair conditions, repeat tasks, and grade outputs without revealing condition.
6. Permit a strong savings statement only when there are zero critical wrong answers and the
   one-sided 95% lower confidence bound for the brain-minus-control success-rate difference is at
   least -0.02.
7. Preserve per-harness results. A combined result is additional and must not hide either harness.
8. Show passive production usage separately and label it observational, not causal.

## Canonical terminology

- **Control / brain off:** the current harness and all non-brain capabilities, with automatic brain
  delivery and on-demand brain access unavailable.
- **Treatment / brain on:** the same configuration with the current Second Brain integration fully
  available.
- **Native tokens:** counters reported by the harness/provider, not estimated from characters or
  words.
- **Qualified completion:** a run whose output passes its preregistered rubric or automated test.
- **Critical regression:** a brain-on answer that introduces a materially wrong, unsafe, stale, or
  destructive claim that the matched control answer did not introduce.
- **Official savings:** a causal percentage from a valid controlled benchmark whose quality gate
  passed.
- **Production trend:** actual historical usage under the configuration that happened to be active;
  useful operational telemetry, but not proof of savings.

## Measurement contract

### Claude Code

Run Claude non-interactively with a pinned executable version, model, effort, maximum turns,
permissions, repository revision, and JSON output. For one completed session:

```text
claude_total_tokens =
    input_tokens
  + cache_creation_input_tokens
  + cache_read_input_tokens
  + output_tokens
```

Store all four components. Anthropic documents the total input as the sum of normal input,
cache-creation input, and cache-read input; output is then added for total processed tokens. If any
required counter is absent, negative, malformed, or inconsistent with the raw result, mark the
sample `invalid_usage`. Do not fall back to the brain's token estimator.

### Codex

Run `codex exec --json` with a pinned executable version, model, reasoning effort, sandbox,
permissions, repository revision, maximum work budget, and effective configuration. Use the final
`event_msg/token_count/info/total_token_usage` object for the session:

```text
codex_total_tokens = total_token_usage.total_tokens
```

Store `input_tokens`, `cached_input_tokens`, `cache_write_input_tokens`, `output_tokens`,
`reasoning_output_tokens`, and `total_tokens`. Cached input and reasoning output are breakdowns of
native totals in the current Codex schema and must not be added a second time. Multiple
`token_count` events are cumulative snapshots; use the final monotonic value rather than summing
them. A missing, decreasing, or malformed cumulative record makes the sample `invalid_usage`.

### Savings calculations

The matched unit is `(task_id, harness, repeat)` with one control and one treatment sample.

```text
absolute_saved = control_total_tokens - treatment_total_tokens
pair_savings_fraction = absolute_saved / control_total_tokens
```

For a harness and for the combined headline, use the ratio of matched sums, not the arithmetic mean
of percentages:

```text
savings_fraction = 1 - sum(treatment_total_tokens) / sum(control_total_tokens)
```

Include every matched pair with valid native usage in the token calculation, including pairs where
one answer fails its quality rubric. Correctness is a co-primary gate, not a filter: removing wrong
answers after grading would selectively improve the measured result.

The combined figure is allowed only when both harnesses completed the same task strata and the
same number of valid pairs. Claude and Codex percentages remain visible beside it.

## Experimental design

### Task suite

The claimable suite contains 30-50 preregistered tasks divided evenly across four strata:

1. Historical/project-memory recall.
2. Codebase navigation and explanation.
3. Diagnosis and implementation planning.
4. Bounded code changes with automated verification.

Every task manifest fixes before execution:

- task id, stratum, prompt, fixture commit, and allowed files;
- time/turn/tool limits;
- reference facts and a human-readable blind rubric;
- automated tests or deterministic checks when applicable;
- critical-regression criteria;
- whether the task is eligible for the combined headline.

The five existing `token-ab.ps1` tasks may be included as historical-recall items but cannot make
up the whole suite. Benchmark tasks, rubrics, and fixture commits are committed before the first
claimable run and are immutable for that benchmark version.

### Repeats and power

Run a non-claimable pilot with at least 10 tasks and two repeats to validate configuration and
estimate paired variance. The claimable run uses a preregistered power calculation targeting 90%
power at two-sided alpha 0.05 for a minimum meaningful 10% token reduction, with these hard bounds:

- at least 30 tasks;
- at least five repeats per task, condition, and harness;
- no more than 50 tasks and five repeats without a new operator decision.

If the maximum planned sample cannot produce a conclusive interval, report `inconclusive`; do not
relax the confidence level or quality margin after seeing results.

### Pairing, order, and execution

- Build matched pairs first, then shuffle pair order with a recorded seed.
- Keep the two conditions of a pair adjacent to reduce time drift; randomly choose which condition
  runs first.
- Counterbalance harness order across tasks.
- Run one model session at a time so CPU, disk, provider throttling, and cache warming do not compete.
- Pin and record the harness executable hash/version, model id, effort, repository commit,
  Second Brain commit, memory-snapshot hash, task-suite commit, CodeGraph index revision, and every
  effective command-line/configuration value.
- A changed model, executable, fixture, tool set, or memory snapshot during a claimable run invalidates
  the affected run rather than silently creating a mixed population.

### Frozen brain and contamination control

A claimable run uses an immutable, project-scoped benchmark snapshot of the current ledger,
memories, provider configuration, retrieval configuration, and live-state fixture. The snapshot hash
is part of the run manifest.

The orchestrator creates a temporary checkout outside every registered production project root and
launches a dedicated benchmark brain endpoint against the frozen snapshot. Benchmark child
processes inherit an explicit endpoint/configuration selector; installed production hooks and MCP
servers remain untouched. The live service therefore cannot claim the benchmark checkout, and the
benchmark sessions cannot become production evidence, consolidation jobs, or future memories.

No implementation may rewrite `~/.claude/settings.json`, `~/.codex/hooks.json`, or
`~/.codex/config.toml`. Before and after execution, record SHA-256 hashes of those files and fail the
run if any changed. A hard kill must leave production configuration unchanged because the
orchestrator never edits it.

### Condition isolation

The orchestrator materializes two complete, inspectable effective configurations per harness:

- `brain_off`: excludes only Agent Brain hook delivery and Agent Brain MCP/CLI access;
- `brain_on`: enables the production-equivalent Agent Brain paths against the frozen benchmark
  endpoint.

Every non-brain hook, MCP server, skill, rule, repository instruction, permission, and model setting
must have the same normalized configuration hash in both conditions. A preflight diff allows only a
declared Agent Brain allowlist. Any other difference makes the run invalid before the first model
session is launched.

## Quality evaluation

### Blind grading

The orchestrator writes an opaque grading bundle containing sample id, prompt, rubric, answer,
automated-test result, and no harness/condition label. A separate key maps opaque ids back to run
metadata after grading.

Deterministic tests decide bounded implementation tasks. Human grading decides claims that cannot be
reduced to string matching. A model grader may provide an advisory score but cannot replace the
human decision for the official first benchmark.

Each sample receives:

- `pass`, `partial`, or `fail`;
- `critical_regression: true|false`;
- a concise grader reason;
- grader identity/version and grading timestamp.

### Claim gate

The official statement is enabled only when all of the following are true:

1. Every planned pair is present. Infrastructure-invalid samples are rerun before unblinding;
   quality failures remain in the result and are never excluded.
2. Native usage validation passes for every included sample.
3. Both harnesses meet their preregistered minimum sample size.
4. There are zero treatment-only critical regressions.
5. The one-sided 95% lower confidence bound for
   `treatment_success_rate - control_success_rate` is at least -0.02, overall and for each harness.
6. The 95% confidence interval for token savings is entirely above zero, overall and for each
   harness quoted as saving tokens.
7. No configuration, model, fixture, ordering, grading, or contamination validity check failed.

Use a task-clustered bootstrap with 10,000 recorded-seed resamples for token-savings intervals so
repeats of one task are not treated as independent tasks. Use a paired binary-confidence method for
the success-rate difference. Store the full method and seed in the report.

## Automatically generated statements

Dashboard prose is generated from report state and cannot be edited.

### Proven

> Using exact provider-reported token counts across {pairs} matched Claude Code and Codex pairs,
> Second Brain {brain_version} reduced total tokens per matched task by {combined}% overall (95%
> CI {low}-{high}). Claude Code saved {claude}% and Codex saved {codex}%. The preregistered quality
> gate passed with zero critical regressions.

### Inconclusive

> This benchmark did not demonstrate a statistically reliable token saving. The measured interval
> includes zero; no savings percentage is claimed.

### Quality blocked

> Lower token usage was observed, but the quality gate failed. No token-savings claim is valid for
> this run.

### Invalid

> This run cannot support a token-savings claim because {validity_reason}.

No dashboard state may substitute softer marketing wording for these outcomes.

## Persistence and audit model

Benchmark data are project-scoped and append-only. Store a compact index/report in the project
ledger or a dedicated project-scoped benchmark database, and content-addressed raw artifacts under:

```text
BRAIN_HOME/runtime/token-benchmarks/<project_id>/<run_id>/
```

Required artifacts:

- `manifest.json`: preregistered suite, versions, hashes, seed, power target, and planned matrix;
- `preflight.json`: configuration diff and production-config hashes;
- `samples.jsonl`: one immutable normalized sample per attempted session;
- `raw/<sample_id>.*`: native stdout/stderr and result artifacts with SHA-256 entries;
- `grading-sheet.csv` and separately protected `grading-key.csv`;
- `grades.jsonl`: append-only grading decisions;
- `report.json`: calculated statistics, gates, and generated statement;
- `audit.json`: completeness, monotonic-token, contamination, and hash checks.

Answers may contain repository information, so the dashboard receives summarized data and explicit
drill-down endpoints; raw artifacts are local-only and never enter the Redis snapshot.

## Production usage telemetry

Normalize actual captured session usage separately:

- Claude: group records by native `message.id`, keep the final monotonic usage object for each
  message because one native message may be written more than once, then sum unique messages by
  native session id using the Claude formula.
- Codex: take the final monotonic cumulative `token_count` object by native session id.
- Deduplicate by project, harness, native session id, and native usage-record identity.
- Preserve native components and provenance to the raw event offset.

The production view shows total tokens, sessions, median tokens/session, cached share, output share,
and brain context delivered for Claude and Codex over 1/7/30-day windows. It may compare periods,
but must say **observational trend** and must not generate a causal savings percentage.

## CLI and dashboard contract

### Brain CLI

Add a `brain benchmark` command family with these responsibilities:

- `preflight`: validate suite, tools, native usage schemas, frozen snapshot, and condition diff;
- `run`: dry-run by default; `--execute` launches the recorded matrix;
- `grade`: export/import the blind grading bundle without exposing the key;
- `report`: calculate intervals, gates, and statement deterministically;
- `show`: emit a project-scoped JSON summary for the dashboard;
- `retire`: mark a run non-current without deleting any artifact.

Every mutating command is idempotent by run/sample id. Interrupted runs resume missing samples and
never repeat completed samples unless the operator creates a new run id.

### Dashboard snapshot

Bump the Rust snapshot schema and mirror it in `agent-brain-dashboard/lib/snapshot-types.ts`. Add an
optional project-scoped token-savings summary so an older Redis snapshot renders `No benchmark`
instead of plausible zeroes. The snapshot carries only summaries and run ids, not answers or raw
session text.

### Dashboard UI

1. Rename the existing panel to **Context delivery** and retain its delivery-budget information.
2. Add **Measured token savings** with:
   - `Proven`, `Inconclusive`, `Quality blocked`, `Invalid`, or `No benchmark` status;
   - the generated evidence statement;
   - combined and side-by-side Claude/Codex percentages with 95% intervals;
   - absolute native tokens with brain off/on/saved;
   - task, pair, repeat, model, version, date, and frozen-snapshot metadata;
   - quality pass rates and critical-regression count;
   - an audit drawer listing every validity gate and per-task pair.
3. Add **Production token trend** as a clearly labelled observational section.

The UI must never color a negative or inconclusive result as success, and one healthy harness must
not hide a failed or absent harness.

## Error handling and validity states

- Missing native counters: invalidate the sample; never estimate.
- Harness crash or timeout: record the attempt and retry according to the preregistered retry rule;
  exhausted retries remain visible.
- Hook/MCP condition mismatch: abort before launching a paid session.
- Production config hash changed: abort and mark the run invalid.
- Frozen snapshot changed: abort and mark the run invalid.
- Provider/model drift: split into a new run id; never pool.
- Failed task: retain its token usage and quality result; do not silently exclude it.
- Partial/interrupted matrix: resumable, never claimable until complete.
- Dashboard snapshot too old: render `No benchmark data in this snapshot`, not zero savings.

## Verification strategy

### Rust and orchestration

- Fixture tests for Claude and Codex native-usage parsers, including missing, malformed, duplicate,
  and decreasing counters.
- Property tests that cumulative Codex snapshots are never summed.
- Tests for matched-sum savings and task-clustered bootstrap determinism.
- Quality-gate boundary tests at -2 percentage points, zero critical regressions, and intervals that
  touch zero.
- Condition-diff tests proving only Agent Brain configuration changes.
- Hard-kill tests proving live settings hashes remain identical.
- Resume/idempotency tests for interrupted matrices.
- Cross-project tests proving benchmark summaries and artifacts never leak.
- Contamination tests proving benchmark sessions create no production events or jobs.

### Dashboard

- Type fixtures for every benchmark status and old snapshot schema.
- Component tests for strong, inconclusive, quality-blocked, invalid, and no-data statements.
- Tests that per-harness failures remain visible beside a combined result.
- Tests that production trends never use causal-savings wording.
- Production build, lint, accessibility, responsive-layout, and rendered visual verification.

### End to end

- A zero-cost synthetic run with known token records and known grades must reproduce an exact report.
- A two-task live smoke run across both harnesses validates native schemas and condition isolation but
  is labelled non-claimable.
- Only after both pass may the preregistered main run execute.

## Delivery sequence

1. Native usage normalization, benchmark data model, statistics, and claim gates in the brain.
2. Safe frozen-snapshot orchestrator and condition-isolation preflight for Claude and Codex.
3. Blind grading and deterministic report generation.
4. Dashboard snapshot contract and backward-compatible remote handling.
5. Dashboard context-delivery rename, measured-savings panel, audit drawer, and production trend.
6. Synthetic end-to-end verification and two-task live smoke run.
7. Preregister the full task suite and power calculation.
8. Execute, grade, and publish the first claimable benchmark only after an explicit operator action.

Implementation does not automatically spend model usage. Dry-run, synthetic verification, and the
live smoke run are separate gates; the full 30-50-task run requires an explicit `--execute` decision.

## Success criteria

The feature is complete when:

- both harnesses produce exact, auditable native token totals for matched brain-off/on runs;
- no live agent configuration is rewritten;
- benchmark sessions cannot contaminate the production brain;
- statistics and quality gates deterministically select one allowed statement state;
- the dashboard shows combined and per-harness results, raw totals, intervals, quality, and audit
  provenance;
- the existing context-delivery metric is retained under an accurate name;
- passive production usage is visible but never presented as causal savings;
- synthetic and smoke verification pass; and
- the first full benchmark remains an explicit operator-run measurement rather than an automatic
  side effect of deployment.

## References

- Anthropic Claude Code CLI JSON output:
  <https://docs.anthropic.com/en/docs/claude-code/cli-usage>
- Anthropic usage and cache token accounting:
  <https://docs.anthropic.com/en/docs/about-claude/pricing>
- NIST paired observations:
  <https://www.itl.nist.gov/div898/handbook/prc/section3/prc311.htm>
- NIST confidence intervals for paired differences:
  <https://www.itl.nist.gov/div898/handbook/prc/section3/prc312.htm>
- NIST bootstrap uncertainty:
  <https://www.itl.nist.gov/div898/handbook/eda/section3/bootplot.htm>
- OpenAI blind expert grading and predefined rubrics:
  <https://openai.com/index/gdpval/>
