# Second Brain Status and Five-Condition Benchmark Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Determine, with causal and reproducible percentage metrics, whether the Secondary Brain lowers net native-harness token usage, reduces end-to-end coding time, or improves retrieval and final-answer quality for Claude Code and Codex; then keep, tune, or disable each layer according to its measured incremental value.

**Architecture:** Extend the existing isolated cross-harness benchmark from two bundled conditions to five counterbalanced conditions. Keep exact provider token counters as the net-token authority, add process and tool-trace timing, grade answers against pinned code/tests and evidence-cited historical facts, and persist every hook/MCP lifecycle stage as append-only telemetry. Benchmark artifacts remain project-scoped and immutable. The dashboard receives bounded summaries and per-session lifecycle folds, never raw benchmark answers or unrestricted transcripts.

**Tech Stack:** Rust 1.88, SQLite/rusqlite, serde/serde_json, clap, SHA-256 content-addressed artifacts, task-clustered bootstrap statistics, Claude Code native JSON, Codex native JSONL, CodeGraph, Next.js 16, React 19, TypeScript, Vitest, and Testing Library.

## Global Constraints

- Do not quote a token-saving, speed, or answer-quality percentage until a completed run has passed the corresponding release gate. `Not measured` is a result state; it is not `0%`.
- Keep external competitor measurements and modeled estimates in a separate evidence class. They may provide context, but they can never populate the C0-C4 Goal 1, Goal 2, or Goal 3 cells.
- Report raw counts and times for auditability, but transform every comparison into a percentage or percentage-point difference with a confidence interval.
- Treat the three goals independently. A run may prove token savings while speed is inconclusive, or improve quality while burning more tokens. Never collapse them into one success badge.
- The primary causal unit is `(task_id, harness, repeat)`. The five conditions for that unit use the same task, repository fixture, native-history fixture, model, effort, permissions, native-memory policy, and limits.
- Use exact native totals as the net-token outcome. Brain-injected context is already part of native model input and must not be added again. Report injected Brain tokens only as an attributed subset.
- Measure elapsed wall time around the whole native process. Hook, Brain retrieval, CodeGraph, MCP, tool use, model latency, tests, and retries are therefore included once in the end-to-end result.
- Keep live Claude and Codex settings byte-identical. All condition files, harness homes, Brain homes, pipes, checkouts, CodeGraph indexes, and transcript fixtures are benchmark-scoped.
- Pin and hash the harness executable, launcher, model, effort, repository commit, Brain commit, suite, condition files, native-memory policy, CodeGraph version/index, embedding model, frozen Brain snapshot, and grader version.
- Use a neutral benchmark instruction overlay for every condition. It must replace project Brain-specific `AGENTS.md`, `CLAUDE.md`, MCP, and hook instructions identically in all five conditions. Otherwise the “native default” condition is not native-only.
- Do not describe the control as “what 99% of users use.” That population claim has no measurement. Name it `native_default` and record the exact effective defaults of the pinned harness versions.
- Preserve project isolation, append-only evidence, `MemoryStatus::is_readable`, the 1,000–1,500 normal orientation budget with a 3,000 hard maximum, and append-only source ordering.
- Historical memory is evidence, not current source authority. Code, Git, tests, and deployment state are authoritative for current-code claims.
- A hook invocation is not a delivery. `hook received`, retrieval decision, reply flush, and capture persistence are separate facts.
- `UserPromptSubmit` is an unsolicited current-session push. Brain MCP is an explicit on-demand historical pull. They must never share a status or be reported as the same mechanism.
- A real pilot or full native-harness run requires a separate explicit `brain benchmark run --execute` action. Dry-run remains the default.
- Do not commit intermediate Rust work casually: a commit touching `crates/` or `Cargo.*` deploys. Build and test in isolation, then obtain explicit deployment authority before the release commit.

---

## Implementation execution record (14 August 2026)

This checklist distinguishes implementation from experiments that deliberately require a later operator decision. An unchecked paid, live-install, trigger-only, or deployment item is a safety boundary, not silently unfinished engineering.

| Scope | Status | Evidence |
|---|---|---|
| Tasks 1–9 benchmark, telemetry, evaluator, five-condition launcher, and session-status implementation | **Implemented and locally verified** | Fresh 14 August release gate: `cargo fmt --all -- --check`, `cargo test --workspace --no-fail-fast`, and `cargo clippy --workspace --all-targets -- -D warnings` all exited 0 |
| Retrieval evaluator calibration fixture | **Evaluator calibrated; not a native-agent result** | 120/120 calibration records valid; precision/recall/faithfulness/healthy-silence metrics were 100.0%, harmful-push and leakage were 0.0%. The report declares `fixture_generated=true`, so these numbers prove evaluator/fixture consistency only |
| Native C0–C4 smoke preflight | **Valid preflight; provider execution blocked before a valid sample** | Run `01a00019-2e91-7d53-af0e-0507d7b753e4` passed isolation, profile, condition, and native-auth checks. Claude then returned `Credit balance is too low` before a turn or token counter existed. The controller was stopped and its exact run process tree removed; no zero-turn record is treated as a benchmark result. |
| Task 10 Claude Brain MCP | **Installed and verified in both native harnesses** | Fresh Claude session `0c11459c-4721-4adc-99c2-6808f0e9e955` and fresh Codex session `019fffe2-24f0-74e3-8997-a375c9dce2d9` each discovered 16 tools and completed read-only status/search/timeline/evidence calls. All 8 MCP requests succeeded; native stdio supplied no session id, so telemetry honestly records them as `unattributed`. |
| Task 11 dashboard | **Integrated, running, and live-data verified** | Dashboard repository commits `6df7e14`, `d470953`, `cacbbe5`, and `92311da`; 6 test files/27 tests pass, ESLint exits 0 (one unrelated warning), and the production build exits 0. `http://127.0.0.1:3000/api/snapshot` returned schema v3, all three projects, seven recent closed sessions, and the 8/8 MCP summary. A 68.017 s cold request now succeeds under the 90 s ceiling; after fixing the completion-time cache clock, the measured pair was 32.906 s generation followed by a 0.140 s cache hit. The UI labels polling separately from generation latency. |
| LongMemEval-S, 500 questions | **Matched hybrid gate passed** | BM25+vector RRF: R@5 96.0%, R@10 98.2%, MRR 92.2%, 243,657 vectors, 500/500 in 13,214.1 s. Change from frozen baseline: 0.0 percentage points / 0.0% relative. BM25-only diagnostic: R@5 93.2%, R@10 96.6%, MRR 89.2% in 151.8 s. Raw logs: `target/benchmark-evidence/longmemeval-current-hybrid-release/` and `longmemeval-current/` |
| AgentMemory reference and final comparison surface | **Pinned and implemented** | Six supplied Markdown references exactly match AgentMemory commit `2973e4ec4c40d323a08daa34220118010e73a2c3`; `suite.json` pins their claims, evidence classes, URLs, and SHA-256 values. `FINAL-OUTCOMES.md` presents retrieval, token-model, and competitor tables without converting external estimates into local results |
| 20-session smoke, 240-session pilot, triggered C2e/C3e experiments, and 1,600-session claimable matrix | **Blocked before the first valid paid sample** | Claude authentication succeeds in the isolated benchmark home, but the provider account reports insufficient credit. Therefore all three native goal percentages and all trigger decisions remain `Not measured`; the pilot, C2e/C3e, and claimable matrix must not run until a valid 20-session smoke passes. |
| Commit, install, service restart, and deployment | **Completed and live-verified** | Brain commit `cf60360108b200c53e7c24ddb1f731a80b98fd4a` is installed; deployment succeeded and restarted the service. Real Claude and Codex lifecycle receipts were captured after deployment. |

### Resume boundary for the quantitative native benchmark

The implementation is ready, but the C0–C4 experiment cannot produce real percentages until the
Claude account can execute native turns. After credit is restored, rebuild
`brain-benchmark-launcher`, generate fresh run templates, create a new smoke run, execute it, and
grade it. Do not reuse an aborted run id. Proceed to the 240-session pilot only if all 20 smoke
sessions have non-zero native usage, valid lifecycle evidence, and passing isolation hashes; proceed
to Priority 2/4 experiments only if their preregistered triggers fire. The 1,600-session matrix still
requires its separate explicit approval.

Evidence SHA-256 values: LongMemEval dataset `d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442`; matched hybrid stdout `ba79120da5f18586d28a607475a2703edf58f1ee08d9d77f2e78d76dab85cb34`; release benchmark binary `a9cfc6d8fe8da8da5ca5d4193789bb060a8be9ca7a0d98bc326f7a594dc68e70`; BM25-only stdout `76674e67db4c08cc42d4579bfb8bfa67a3c3a207ce91d4cf32b30f86a5540d8c`; calibration summary `56af4651513a21d3bcb25ef7db2f0b6ea562b310c02243026faa9721ee8629ab`; smoke preflight `73c4f4772271a1e7007a88ec664ca6b7a9c3b36a00e3e94ba161c20e4baa6b10`.

AgentMemory downloaded-file SHA-256 values: coding-agent-life-v1 `d3b3d1519775c794f91093338983055bd7ff87c7beb05646453c4303dea5ca1f`; LongMemEval `72a5f411a969691bb893d6cc3613ae4737fa27d1e9e0de64c5e209322244c267`; quality `dd73283a2f3059d836c455affbc0dad24fe6f6ce2c6297cd4b8651d65642587a`; scale `d410e6b14aafc71715d25ba92a1287b98917d9cc3ddbbc94689102d27fb2a192`; comparison `fa989e41dc0b9a581e365a7993cf9d666169e6bfc2690b754ba5f5d6eb5e10dd`; template `7aa9f0d585aa2eec31bdbe94ca84089a241a9c9780b1c6d3d835d5258a4614e5`.

---

## Honest starting conclusion

The Secondary Brain is plausibly valuable for cross-session and cross-harness continuity, but the current evidence does **not** prove any of the three new goals.

| Existing evidence | Measured result | What it proves | What it does not prove |
|---|---:|---|---|
| LongMemEval-S, all 500 questions | R@5 **96.0%**, R@10 **98.2%**, MRR **92.2%** | The historical retrieval engine can find relevant evidence on a recognized memory benchmark | Lower native tokens, faster coding, or better final coding answers |
| Hard `single-session-preference` slice | BM25 R@5 **63.3%** to hybrid R@5 **90.0%**, a **42.2% relative improvement** and **+26.7 percentage points** | Vector-plus-BM25 fusion materially helps the target semantic gap | SessionStart quality, because startup currently does not enable vector search |
| Missing SQLite index repair | **42,001 ms** to **34.1 ms**, a **99.92% reduction** | A specific orientation query bottleneck was fixed | End-to-end agent speed against native-only control |
| Existing cross-harness token benchmark | No paid pilot or claimable matrix completed | The instrument can parse exact native token totals and blind grades | Any savings percentage; the dashboard must continue to say `not measured` |

The existing two-condition suite is insufficient for this decision. Its control already enables CodeGraph, while its treatment bundles SessionStart, UserPromptSubmit, SessionEnd, and Brain MCP. It can compare a bundle with CodeGraph held constant, but it cannot attribute value to CodeGraph, startup orientation, prompt pushes, or explicit historical pull.

## Do the five pain points disappear today?

No. The architecture mitigates all five, but it does not kill all five.

| Pain point | Current verdict | Why |
|---|---|---|
| 1. Cross-session blindness and conflicting edits | **Partially mitigated** | Shared history and lease/path warnings can expose other work, but coordination is advisory and depends on a task identity. It does not prevent two uncoordinated sessions from editing the same line. |
| 2. New-session reload burns quota | **Plausibly mitigated; unproved** | SessionStart supplies a bounded orientation and historical pull can avoid transcript export, but no causal native-token comparison has run. Live code still must be inspected when memory may be stale. |
| 3. Codex pays the same onboarding tax | **Plausibly mitigated; unproved** | Codex now receives all three hooks, but the same token/time experiment is still missing. |
| 4. No Claude/Codex shared memory | **Mostly mitigated for pushed memory; incomplete for depth** | Both harnesses consume one project ledger through hooks. On-demand MCP depth must be verified and normalized for both harnesses rather than assumed from configuration. Their native proprietary memories remain separate. |
| 5. “What did I do last week?” is unreliable | **Materially mitigated, not eliminated** | Search, timeline, evidence offsets, replay, and the dashboard make answers inspectable. Stale or contradictory captured claims remain possible, so citation correctness, freshness, and abstention must be measured. |

The benchmark will translate these pain points into measurable outcomes:

- Pain points 2 and 3: new-session onboarding token reduction and time reduction, per harness.
- Pain point 4: cross-harness transfer success rate when history was produced by the other harness.
- Pain point 5: historical fact accuracy, evidence support, freshness accuracy, and abstention accuracy.
- Pain point 1: conflict-warning recall is measured, but prevention remains outside this retrieval benchmark because Priority 3 is held.

---

## Priority 0 and Priority 1: five-condition component ablation

### Conditions

| ID | Name | CodeGraph | SessionStart | SessionEnd | UserPromptSubmit push | Brain MCP pull |
|---|---|---:|---:|---:|---:|---:|
| C0 | `native_default` | Off | Off | Off | Off | Off |
| C1 | `codegraph_only` | On | Off | Off | Off | Off |
| C2 | `startup_lifecycle` | On | On | On | Off | Off |
| C3 | `prompt_push` | On | On | On | On | Off |
| C4 | `full_brain` | On | On | On | On | On |

Important interpretation:

- C2 measures the startup-orientation contribution in consumer sessions. SessionEnd is included because it is part of the lifecycle and future-session capture contract, but it cannot improve the answer in the session that is already ending.
- SessionEnd reliability is therefore also measured in a separate scripted lifecycle suite: hook received, boundary persisted, transcript cursor caught up, and a subsequent session can retrieve the newly stored fact.
- C4 makes Brain MCP available. The primary result is intention-to-treat: the condition counts even if the agent does not call it. A secondary adoption report shows request rate, success rate, and realized outcome among calls without replacing the causal result.
- The deployed architecture currently has hook parity, but the repository contract still describes Brain MCP depth as wired for Codex and not wired for Claude. Therefore Claude C3 is the closest deployed behavior and Claude C4 is a proposed parity condition until preflight proves otherwise. The report must label `as_deployed` and `normalized_experiment` separately.
- Native-memory behavior is identical across C0–C4. Preflight records the pinned harness’s actual effective default and fails if one condition changes it.

### Preregistered contrasts

| Contrast | Question |
|---|---|
| C1 − C0 | Does CodeGraph alone improve tokens, time, or quality over native default? |
| C2 − C1 | What is the incremental value of SessionStart orientation plus the SessionEnd lifecycle? |
| C3 − C2 | What is the incremental value or cost of unsolicited prompt-time pushes? |
| C4 − C3 | What is the incremental value of making on-demand historical pull available? |
| C4 − C0 | Does the complete current architecture beat native default? This is the headline contrast. |
| C4 − C1 | Does the Secondary Brain add value after CodeGraph’s current-code advantage is held constant? |

Report every contrast overall, per harness, and per task stratum. The pooled headline is invalid if one harness is missing or if the directions materially conflict; the per-harness results remain visible.

### Workload

Retain the 32-task balanced suite and add episode metadata rather than inventing a smaller favorable dataset:

- 8 historical-recall tasks, including yesterday/week/month and cross-harness history.
- 8 codebase-navigation tasks.
- 8 diagnosis/planning tasks.
- 8 bounded-change tasks with automated checks.
- At least 8 of the 32 are new-session continuation episodes backed by identical pinned native-history and Brain-history fixtures.
- At least 4 continuation episodes were produced by Claude and consumed by Codex, and at least 4 in the reverse direction.
- Every task pins reference facts, evidence UUIDs/source offsets where historical, allowed files, a fixture commit, automated checks, maximum turns, maximum tool calls, and timeout.

Use pre-recorded, immutable history fixtures so every condition receives the same past. Do not pay for a stochastic producer session five times. For SessionEnd’s capture contract, use a separate two-session lifecycle suite that creates one deterministic fact, closes the producer, waits for the capture barrier, and asks a fresh consumer.

### Run sizes

| Stage | Calculation | Native consumer sessions | Claim status |
|---|---:|---:|---|
| Schema smoke | 2 tasks × 1 repeat × 2 harnesses × 5 conditions | 20 | Instrument validation only |
| Component pilot | 12 tasks × 2 repeats × 2 harnesses × 5 conditions | 240 | Directional percentages; explicitly non-claimable |
| Claimable matrix | 32 tasks × 5 repeats × 2 harnesses × 5 conditions | 1,600 | Eligible for claims if all gates pass |

The lifecycle and retrieval suites are local deterministic runs and do not multiply native-agent sessions. Before the 1,600-session matrix, run a power check from pilot variance. If 32 tasks × 5 repeats is underpowered for a 10% effect, increase repeats; never shrink the effect threshold after seeing results.

### Randomization and blocking

- Treat all five samples for `(task_id, harness, repeat)` as one block.
- Randomize the five-condition order inside each block with a recorded deterministic seed.
- Counterbalance harness order and distribute conditions across time to avoid quota, cache, and machine-temperature drift.
- Bootstrap whole task clusters, keeping all harnesses, repeats, and conditions for a task together.
- Use 10,000 bootstrap resamples and adjust the preregistered primary contrasts with Holm’s method.

---

## Metrics and formulas

### Goal 1: net token usage

Primary metric:

`net_token_savings_% = 100 × (baseline_native_tokens − condition_native_tokens) / baseline_native_tokens`

Also report:

- absolute tokens saved and burned;
- input, cache-creation, cache-read/cached-input, output, and reasoning breakdowns without double counting;
- Brain context overhead as `100 × injected_brain_tokens / native_total_tokens`;
- tokens per successful task and its reduction percentage;
- onboarding tokens before the first correct action and their reduction percentage;
- median and task-clustered ratio-of-sums with 95% confidence intervals.

Claude total remains `input + cache_creation_input + cache_read_input + output`. Codex total remains the last monotonic native `total_token_usage.total_tokens`. A malformed or missing native counter invalidates that sample; it is never estimated from text length.

### Goal 2: query, retrieval, and coding execution time

Primary metric:

`end_to_end_speedup_% = 100 × (baseline_wall_ms − condition_wall_ms) / baseline_wall_ms`

Also report:

- median, p50, p95, and p99 wall time in milliseconds plus relative percentage change;
- SessionStart service compile and hook round-trip latency;
- UserPromptSubmit decision and reply latency;
- Brain MCP search/timeline/evidence latency, warm and cold;
- CodeGraph call latency;
- time to first correct file identification, first valid edit, and first passing test;
- tool-call reduction percentage;
- turn reduction percentage;
- file-read call reduction percentage and unique-file-read reduction percentage;
- test-cycle reduction percentage;
- timeout/failure rate as a percentage.

The end-to-end clock is authoritative for “faster coding.” A 99% faster database query does not support that claim if model/tool time makes the complete task slower.

### Goal 3: retrieval and final-answer accuracy, precision, and quality

Create a 240-case project-specific gold set in addition to rerunning all 500 LongMemEval-S questions:

- 80 SessionStart cases with expected continuation facts and prohibited stale facts.
- 80 UserPromptSubmit cases: 40 relevant-memory positives and 40 healthy-silence negatives.
- 80 on-demand historical pulls across day/week/month/custom cutoffs and both harness sources.
- Split each group 50/50 into a visible calibration set and a locked test set. Tune only on calibration cases.

Every gold record contains a project, as-of cutoff, query or lifecycle trigger, expected fact IDs, acceptable memory/event IDs, exact transcript source offsets, prohibited superseded facts, and an abstain expectation when no answer exists.

Retrieval metrics:

- `precision@k_% = 100 × relevant_returned@k / returned@k`
- `recall@k_% = 100 × relevant_returned@k / total_relevant`
- `F1_% = 200 × precision × recall / (precision + recall)`
- MRR and nDCG@5 as percentages.
- stale-contradiction rate, cross-project leakage rate, and duplicate-result rate.

Answer metrics:

- required-fact accuracy percentage;
- exact-answer success percentage where an exact fact set exists;
- automated test pass percentage for coding tasks;
- blind rubric pass percentage;
- citation precision: cited claims whose cited source really supports them;
- citation coverage: verifiable claims carrying a source;
- source faithfulness: verifiable claims supported by the cited source;
- freshness accuracy: claims consistent with the as-of cutoff and supersession state;
- abstention accuracy when the evidence does not contain an answer.

Prompt-push-specific metrics:

- useful-push precision and eligible-memory recall;
- healthy-silence specificity;
- harmful-injection rate;
- repeat-memory rate;
- budget truncation rate;
- net downstream token/time change, because a relevant push that adds more than it saves can still fail Goals 1 and 2.

Final answers are graded against pinned code/tests and evidence offsets, not against another model’s unsupported opinion. Automated checks decide deterministic coding facts. Two blinded reviewers adjudicate semantic rubric items; Cohen’s kappa must be at least 80% before subjective quality is claimable.

---

## Concrete release gates

Each goal receives one of `proven`, `inconclusive`, `regressed`, or `invalid`.

### Goal 1 gate: lower net tokens

For C4 versus C0:

- point estimate is at least **10.0%** net savings;
- Holm-adjusted 95% confidence lower bound is above **0.0%** overall and for each harness;
- overall quality is non-inferior by the Goal 3 safety gate;
- zero treatment-only critical regressions;
- every native usage record and isolation check is valid.

Otherwise no token-saving claim is made. A positive point estimate whose interval touches zero is `inconclusive`, not “saved approximately X%.”

### Goal 2 gate: faster execution

For C4 versus C0:

- point estimate is at least **10.0%** end-to-end wall-time reduction;
- Holm-adjusted 95% confidence lower bound is above **0.0%** overall and for each harness;
- SessionStart warm p95 is at most **500 ms** and remains below each harness timeout in **99.9%** of invocations;
- warm historical query p95 is at most **25 ms**, cold p95 at most **1,000 ms**;
- timeout and harness-failure rates do not increase by more than **1.0 percentage point**.

Tool-call and file-read reductions are explanatory diagnostics. They cannot replace the wall-clock gate.

### Goal 3 gate: better answers and retrieval

Safety gate for every performance claim:

- overall and per-harness blind pass-rate one-sided 95% lower bound is at least **−2.0 percentage points**;
- automated checks have zero treatment-only critical regressions;
- cross-project leakage is exactly **0.0%**;
- citation precision is **100.0%** and source faithfulness is at least **98.0%**;
- stale-contradiction rate is **0.0%**.

Superiority gate for a “better final answers” claim:

- historical/continuation pass rate improves by at least **+5.0 percentage points**;
- adjusted 95% confidence lower bound is above **0.0 percentage points** overall and for each harness;
- locked project gold-set precision@5 and recall@5 are each at least **90.0%**;
- LongMemEval-S R@5 remains at least **95.0%** and does not regress by more than **1.0 percentage point** from the frozen current baseline.

### Lifecycle and observability gates

- hook-receipt telemetry records at least **99.9%** of injected test invocations;
- replies selected for delivery are confirmed flushed at least **99.9%** of the time;
- SessionEnd boundaries persist at least **99.9%** of the time;
- capture-caught-up confirmation reaches at least **99.9%** within its declared window;
- injected service/hook/capture faults produce the correct alert with **100.0% recall**;
- healthy scenarios have at most **1.0% false alerts**;
- `healthy silence` is never counted as a missing UserPromptSubmit delivery;
- `not requested` is never counted as a failed Brain MCP pull.

### Incremental keep/tune/remove rule

For C1, C2, C3, and C4, keep a component enabled by default only if its incremental contrast meets at least one goal’s superiority gate and passes all safety gates. If it adds tokens or time with no quality gain, disable it by default. If it helps only one task stratum, make it conditional for that stratum rather than charging every prompt.

---

## Priority 2: conditional SessionStart orientation enhancement

Priority 2 is included, but it is **not** implemented before measuring the current C2 behavior.

Trigger it when the C2 − C1 pilot shows any of the following on the locked set:

- startup fact recall below **90.0%**;
- irrelevant orientation tokens above **20.0%**;
- C2 adds net tokens or time while historical/continuation pass rate improves by less than **+5.0 percentage points**;
- stale-contradiction rate above **0.0%**;
- SessionStart p95 above **500 ms**.

Candidate enhancement: an evidence-cited `ContinuationCapsule` built deterministically at SessionEnd and selected at SessionStart.

The capsule contains only:

- last explicit user goal;
- completed changes and touched paths;
- latest test command/result;
- unresolved failure or blocker;
- next stated action;
- branch, HEAD, dirty/conflict state;
- producer harness/session, ended-at time, and evidence UUIDs/source offsets.

Rules:

- no provider/LLM call and no new vector search on the hook critical path;
- append-only record derived from captured evidence;
- mark revision-incompatible facts historical rather than current;
- reserve 300–600 tokens inside the existing 1,500-token normal budget, never in addition to it;
- emit at most one newest compatible capsule plus one concurrent-session warning;
- preserve citations and indicate truncation;
- compare current C2 with candidate C2e on the same pilot blocks before promotion.

Promote C2e only if startup fact recall improves by at least **+5.0 percentage points** or onboarding tokens/time improve by at least **10.0%**, with no safety-gate failure. If it does not, retain current C2 or disable startup orientation according to the C2 − C1 result.

---

## Priority 3: held

Hold stronger coordination enforcement. Do not add automatic file locks, edit denial, lease acquisition, or worktree policy to this benchmark.

Reason: enforcement changes agent behavior and can independently reduce conflicts, tokens, and time, which would confound the retrieval ablation. Current advisory lease/path warnings remain observable, and conflict-warning recall is measured. Revisit enforcement in a separate experiment after the retrieval stack’s value is known.

---

## Priority 4: prompt-push calibration and retrieval observability

Priority 4 is included in two parts.

1. **Mandatory, behavior-neutral telemetry before the current-state pilot.** Persist candidate count, selected IDs, dropped IDs/reasons, shared-term score, channel ranks, token count, latency, reply flush, and healthy-silence reason. Hash the prompt; do not persist raw prompt text in the lifecycle table.
2. **Conditional behavior tuning after the current-state pilot.** Tune minimum prompt length, relevance floor, maximum memories, token budget, and per-session cap using only the calibration half. Do not re-enable the cross-encoder unless a separate locked ablation reverses its known quality/cost regression.

The current implementation’s `MAX_SESSION_PUSHES = 20` is actually a cap on delivered memories, not hook pushes. Rename the concept in schema and UI to `max_session_pushed_memories` so the benchmark and operator are not misled.

Promote tuned prompt pushes only when locked useful-push precision is at least **90.0%**, healthy-silence specificity at least **95.0%**, harmful-injection rate at most **1.0%**, repeat-memory rate **0.0%**, and C3 − C2 passes at least one goal superiority gate.

---

## Priority 5: per-session status, indicators, and alerts

Implement near-real-time status, not an invented continuous heartbeat. Between hook/MCP interactions the strongest truthful statement is “last observed working at time T.”

### Append-only lifecycle facts

Persist these stages with project, harness, native session ID, event/request ID, timestamp, duration, outcome, and error reason:

- `hook_received` for SessionStart, UserPromptSubmit, and SessionEnd;
- `retrieval_decided`: delivered, healthy silence, truncated, or failed;
- `reply_flushed` after the client has the bytes;
- `session_end_boundary_persisted`;
- `capture_caught_up` after the source cursor covers the observed source length;
- `mcp_requested`, `mcp_succeeded`, and `mcp_failed`, separately for search/timeline/evidence/claims/leases;
- service start, stop, and degraded transitions.

Never infer invocation from `context_deliveries`. Preserve the existing delivery table as the receipt for actual pushed text.

MCP attribution must use an exact native session ID or a validated correlation token established at SessionStart. If the harness transport cannot supply one, store the request as `unattributed` and show that limitation. Never assign an MCP call to a session merely because its timestamp is nearby; concurrent mega-sessions make that guess unsafe.

### Session fold

Fold append-only facts into these states:

- `active`: no SessionEnd and activity within 30 minutes;
- `stale_open`: no SessionEnd and last activity older than 30 minutes;
- `closed`: SessionEnd boundary persisted;
- `historical_uninstrumented`: captured before lifecycle telemetry existed.

For each active or closed session, show four independent channels:

1. SessionStart orientation: not observed, compiling, fully delivered, truncated, healthy empty, or failed.
2. UserPromptSubmit current-session push: prompt count; delivered count; healthy-silence count; truncated/error count; latest decision.
3. On-demand historical pull: not requested, succeeded with tool/result count, or failed. Never label `not requested` as missing.
4. SessionEnd capture/store: not ended, hook received, boundary stored, capture caught up, or stale-open/missing boundary.

### Exact user-facing indicator examples

- `Brain service: last observed working 3s ago`
- `SessionStart: fully delivered — 1,045 tokens, 8 citations`
- `Prompt push: healthy silence — no new relevant memory`
- `Historical pull: not requested`
- `SessionEnd: boundary stored; transcript capture caught up`
- `Warning: SessionStart hook not observed within 10s; check Codex hook trust`
- `Error: hook received but reply was not flushed before timeout`
- `Warning: session inactive for 30m with no SessionEnd boundary`

### Alert rules

- Red: service task stopped, binary missing/drifted, pipe unreachable, reply-flush failure, cross-project leakage, or boundary persistence failure.
- Amber: SessionStart not observed within 10 seconds of a captured native session start; capture backlog past its window; stale-open session; orientation truncated; prompt-push error.
- Green: explicit receipt/delivery success.
- Neutral: healthy silence, not requested, or historical uninstrumented state.

The first Codex diagnostic remains `[hooks.state]` trust. The dashboard should link that repair instruction when Codex session activity exists but no hook receipt does.

---

## Implementation tasks

### Task 1: Preregister the v2 suite and result contract

**Files:**

- Create: `benchmarks/second-brain/v2/suite.json`
- Create: `benchmarks/second-brain/v2/README.md`
- Create: `benchmarks/second-brain/v2/result-schema.json`
- Create: `benchmarks/second-brain/v2/rubrics/*.md`
- Create: `benchmarks/second-brain/v2/retrieval-gold/calibration.jsonl`
- Create: `benchmarks/second-brain/v2/retrieval-gold/locked-test.jsonl`
- Create: `crates/brain-cli/tests/second_brain_benchmark_suite.rs`

- [x] Write failing tests for 32 balanced tasks, episode/cross-harness coverage, unique IDs, pinned commits, evidence offsets, time/tool limits, and 240 non-overlapping gold cases.
- [x] Encode the six preregistered contrasts, formulas, gates, 10,000 bootstrap resamples, seed, and Holm adjustment in the manifest.
- [x] Add `current_state` and `candidate` benchmark labels so an enhanced run cannot overwrite or masquerade as the original architecture’s result.
- [x] Add a machine-readable result state for each goal: `proven`, `inconclusive`, `regressed`, `invalid`, or `not_run`.
- [x] Run `cargo test -p brain-cli --test second_brain_benchmark_suite`.

### Task 2: Extend the benchmark domain from two to five conditions

**Files:**

- Modify: `crates/brain-cli/src/token_benchmark/model.rs`
- Modify: `crates/brain-cli/src/token_benchmark/matrix.rs`
- Modify: `crates/brain-cli/src/token_benchmark/mod.rs`
- Modify: `crates/brain-cli/tests/token_benchmark_model.rs`
- Modify: `crates/brain-cli/tests/token_benchmark_matrix.rs`

- [x] Replace `BrainOff/BrainOn` and `opposite()` with the ordered C0–C4 enum and explicit contrast definitions.
- [x] Replace pair-only records with a repeated-measures block containing all five conditions while retaining backward deserialization for schema v1 artifacts.
- [x] Generalize execution templates from `brain_off/brain_on` to a condition map validated to contain exactly C0–C4 for both harnesses.
- [x] Test deterministic five-condition randomization, balance of every order position, unique sample IDs, stable seed behavior, and complete blocks.
- [x] Bump benchmark artifact schemas to v2 without changing the dashboard meaning of old v1 reports.
- [x] Run the model and matrix focused tests.

### Task 3: Add end-to-end time and native trace metrics

**Files:**

- Modify: `crates/brain-cli/src/token_benchmark/runner.rs`
- Modify: `crates/brain-cli/src/token_benchmark/orchestrator.rs`
- Modify: `crates/brain-cli/src/token_benchmark/usage.rs`
- Modify: `crates/brain-cli/src/token_benchmark/model.rs`
- Create: `crates/brain-cli/src/token_benchmark/trace.rs`
- Create: `crates/brain-cli/tests/token_benchmark_trace.rs`
- Modify: `crates/brain-cli/tests/token_benchmark_e2e.rs`

- [x] Add `elapsed_ms` to `ProcessOutput`, measured with `Instant` around spawn through final output collection, including timeout cleanup.
- [x] Parse turns, total tool calls, tool calls by name, Brain MCP calls, CodeGraph calls, file-read calls, unique files, edit calls, and test commands from preserved native output.
- [x] Record timestamps for first correct file, first edit, and first passing test when the task’s automated trace markers exist; otherwise store `not_observable`, never zero.
- [x] Test that Claude/Codex cumulative usage is not double counted and timing is not reconstructed from timestamps with different clocks.
- [x] Preserve raw stdout/stderr content-addressed before normalization.
- [x] Run `cargo test -p brain-cli --test token_benchmark_trace --test token_benchmark_e2e`.

### Task 4: Persist lifecycle and retrieval-decision telemetry append-only

**Files:**

- Create: `crates/brain-store/src/lifecycle.rs`
- Modify: `crates/brain-store/src/migrations.rs`
- Modify: `crates/brain-store/src/ledger.rs`
- Modify: `crates/brain-store/src/lib.rs`
- Create: `crates/brain-store/tests/lifecycle.rs`

- [x] Add an append-only `lifecycle_events` table keyed by project plus immutable event UUID; never update a prior stage.
- [x] Add an append-only `retrieval_decisions` table carrying channel, outcome, reason code, candidate/selected/dropped counts, token count, latency, query hash, and selected evidence IDs.
- [x] Add project/session/time-bounded reads and fold functions; reject another project’s writes and reads.
- [x] Define stable reason codes for short prompt, missing session ID, session memory cap, no relevant candidate, budget drop, retrieval error, timeout, and not requested.
- [x] Prove duplicate event idempotency, conflicting duplicate refusal, source isolation, and old-ledger migration.
- [x] Run `cargo test -p brain-store --test lifecycle`.

### Task 5: Instrument hooks, delivery flush, MCP, and capture completion

**Files:**

- Modify: `crates/brain-service/src/pipe.rs`
- Modify: `crates/brain-service/src/hook_handler.rs`
- Modify: `crates/brain-service/src/query_api.rs`
- Modify: `crates/brain-service/src/capture.rs`
- Modify: `crates/brain-mcp/src/tools.rs`
- Modify: `crates/brain-service/tests/hook_handler.rs`
- Modify: `crates/brain-service/tests/mid_session_push.rs`
- Modify: `crates/brain-service/tests/mcp_delivery.rs`
- Modify: `crates/brain-service/tests/session_end.rs`
- Create: `crates/brain-service/tests/lifecycle_telemetry.rs`

- [x] Record `hook_received` before routing, as the current log does, and prove telemetry failure cannot block the hook.
- [x] Record retrieval decisions for every SessionStart/UserPromptSubmit, including healthy silence.
- [x] Record `reply_flushed` only after the pipe write and flush succeed, alongside the existing context-delivery receipt when text exists.
- [x] Instrument every Brain MCP tool request/success/failure independently from prompt hooks.
- [x] Require an exact native-session/correlation identifier where the harness can supply one; retain an explicit `unattributed` state where it cannot.
- [x] Record SessionEnd boundary persistence and capture-caught-up state without claiming EOF when the source is still growing.
- [x] Rename the 20-memory session cap in constants, telemetry, and tests; do not change its value in the current-state build.
- [x] Remove stale source comments/tests that still claim Codex Desktop does not fire SessionStart; keep the evidence-backed hook contract in `AGENTS.md` authoritative.
- [x] Inject write, pipe, retrieval, and capture failures and assert exact lifecycle stages and fail-open behavior.
- [x] Run the focused service/MCP tests.

### Task 6: Build the gold-set retrieval evaluator

**Files:**

- Create: `crates/brain-cli/src/retrieval_benchmark.rs`
- Modify: `crates/brain-cli/src/lib.rs`
- Modify: `crates/brain-cli/src/main.rs`
- Create: `crates/brain-cli/tests/retrieval_benchmark.rs`

- [x] Implement deterministic SessionStart compilation, prompt-push decision, and on-demand search evaluation at an `as_of` cutoff.
- [x] Score precision/recall/F1, MRR, nDCG@5, fact accuracy, citation precision/coverage, faithfulness, freshness, abstention, harmful push, and healthy silence.
- [x] Resolve every accepted citation to the actual event/memory and transcript source offset before crediting it.
- [x] Refuse gold records whose expected source is unreadable, superseded incorrectly, outside the project, or newer than the cutoff.
- [x] Emit per-case JSONL and aggregate percentages; keep calibration and locked-test commands separate.
- [x] Add a `brain benchmark retrieval` CLI path that is dry/local and records the executable/config hashes.
- [x] Run `cargo test -p brain-cli --test retrieval_benchmark`.

### Task 7: Generalize statistics and reports to three goals

**Files:**

- Modify: `crates/brain-cli/src/token_benchmark/statistics.rs`
- Modify: `crates/brain-cli/src/token_benchmark/report.rs`
- Modify: `crates/brain-cli/src/token_benchmark/model.rs`
- Create: `crates/brain-cli/tests/second_brain_benchmark_statistics.rs`
- Modify: `crates/brain-cli/tests/token_benchmark_report.rs`

- [x] Implement task-clustered estimates for token ratio-of-sums, wall-time ratio-of-sums, pass-rate differences, and retrieval percentages for each contrast.
- [x] Add Holm-adjusted intervals and deterministic boundary tests at 0%, 10%, −2 percentage points, and +5 percentage points.
- [x] Emit separate token, speed, and quality statuses plus an overall statement that never hides a failed goal.
- [x] Keep failed-quality samples in token/time denominators; keep timed-out samples in failure-rate and capped-time sensitivity analyses.
- [x] Report intention-to-treat and MCP adoption diagnostics separately.
- [x] Test conflicting per-harness directions, missing condition blocks, invalid native usage, critical regressions, and `not_run` rendering.
- [x] Run the focused statistics/report tests.

### Task 8: Materialize and verify ten isolated condition profiles

**Files:**

- Create: `benchmarks/second-brain/v2/configs/claude-c0-native-default.json` through `claude-c4-full-brain.json`
- Create: `benchmarks/second-brain/v2/configs/codex-c0-native-default.json` through `codex-c4-full-brain.json`
- Create: `benchmarks/second-brain/v2/instructions/AGENTS.md`
- Create: `benchmarks/second-brain/v2/instructions/CLAUDE.md`
- Modify: `crates/brain-cli/src/token_benchmark/preflight.rs`
- Modify: `crates/brain-cli/src/token_benchmark/orchestrator.rs`
- Modify: `crates/brain-cli/tests/token_benchmark_preflight.rs`

- [x] Validate the exact cumulative difference table C0–C4; reject any extra model, effort, permission, native-memory, skill, provider, or non-Brain hook difference.
- [x] Strip project Brain/CodeGraph wiring identically, then add only the assigned benchmark capability outside live settings.
- [x] Clone a fresh native harness home, checkout, Brain home, pipe, and writable CodeGraph state for each sample.
- [x] Hash production settings before and after every sample and abort on any mutation.
- [x] Verify C0 has no reachable Brain binary, MCP, hook, environment endpoint, CodeGraph tool, or project instruction asking for them.
- [x] Verify C1 has CodeGraph and no Brain path; C2 adds only SessionStart/SessionEnd; C3 adds only UserPromptSubmit; C4 adds only Brain MCP.
- [x] Replace the old `treatment_delivery_observed` check with condition-specific lifecycle assertions, including healthy silence.
- [x] Run preflight tests and `brain benchmark preflight` without `--execute`.

### Task 9: Add per-session status CLI and dashboard snapshot schema v3

**Files:**

- Create: `crates/brain-cli/src/session_status.rs`
- Modify: `crates/brain-cli/src/dashboard.rs`
- Modify: `crates/brain-cli/src/status.rs`
- Modify: `crates/brain-cli/src/main.rs`
- Modify: `crates/brain-cli/src/lib.rs`
- Create: `crates/brain-cli/tests/session_status.rs`
- Modify: `crates/brain-cli/tests/dashboard_token_contract.rs`

- [x] Add `brain sessions status --active --closed --json` with project scoping, bounded pagination, and RFC 3339 timestamps.
- [x] Fold lifecycle events into active, stale-open, closed, and historical-uninstrumented sessions.
- [x] Add independent startup, prompt-push, MCP-pull, and SessionEnd/capture channel states.
- [x] Bump dashboard schema to v3 and include bounded active sessions, recent closed sessions, active alerts, and three-goal benchmark summaries.
- [x] Preserve old delivery aggregates; label them delivery counts, not hook invocation counts.
- [x] Test service-down, untrusted/missing hook, healthy silence, full delivery, truncation, MCP not requested, MCP failure, boundary stored, capture caught up, stale-open, and cross-project refusal.
- [x] Run the focused CLI/dashboard tests.

### Task 10: Wire and verify Brain MCP depth in Claude Code

The installed state at plan creation is asymmetric: Codex has `[mcp_servers.brain]` pointing to `brain-mcp.exe`; Claude Code has the three Brain hooks but no configured Brain MCP server. This task makes C4 an actual deployable architecture rather than only a normalized benchmark condition.

**Files:**

- Create: `crates/brain-cli/src/install_mcp.rs`
- Modify: `crates/brain-cli/src/main.rs`
- Modify: `crates/brain-cli/src/lib.rs`
- Modify: `crates/brain-cli/src/config_panel.rs`
- Create: `crates/brain-cli/tests/claude_mcp_install.rs`
- Modify: `crates/brain-cli/tests/dashboard.rs`
- Modify: `docs/registering-a-project.md`
- Modify: `docs/operations/mcp-and-query.md`

- [x] Add an idempotent `brain install-mcp --harness claude` command that invokes Claude Code’s supported MCP configuration interface at user scope; never hand-edit or replace `~/.claude.json`.
- [x] Resolve and pin the installed `BRAIN_HOME/bin/brain-mcp.exe`; reject `target/release` and a missing/stale binary.
- [x] Add a symmetric uninstall path that removes only the server named `brain` after confirming its command matches Agent Brain.
- [x] Extend the config panel to inspect Claude’s effective MCP wiring as well as Codex’s and report configured, approved/connected, command path, and binary-present states without reading secrets.
- [x] Verify all 16 tools are discoverable in a fresh Claude Code session and run read-only smoke calls for `brain_status`, `brain_search`, `brain_timeline`, and `brain_evidence` against a fixture project.
- [x] Prove Claude search results remain project-scoped and that an unknown/foreign project or evidence UUID is refused.
- [x] Record MCP request/success/failure telemetry with Claude harness and exact session correlation when available; use the explicit `unattributed` state otherwise.
- [x] Keep installation an explicit operator action. Project registration remains one local-path command because the user-scoped MCP server is global and project selection happens inside every tool request.
- [x] Run `cargo test -p brain-cli --test claude_mcp_install` and the dashboard/config-panel tests.

### Task 11: Build the dashboard session and benchmark views

**Repository:** `C:/Users/quekm/Desktop/projects/agent-brain-dashboard`

**Files:**

- Modify: `lib/snapshot-types.ts`
- Create: `features/session-lifecycle/session-lifecycle-panel.tsx`
- Create: `features/session-lifecycle/session-lifecycle-panel.test.tsx`
- Create: `features/session-lifecycle/session-row.tsx`
- Create: `features/alerts/brain-alerts.tsx`
- Create: `features/alerts/brain-alerts.test.tsx`
- Modify: `features/service-health/service-health-panel.tsx`
- Modify: `features/token-savings/token-savings-panel.tsx`
- Modify: `app/page.tsx`

- [x] Mirror dashboard schema v3 and make every new field optional for cached v2 snapshots.
- [x] Add active, stale-open, and closed tabs with per-channel status and last-observed timestamps.
- [x] Render healthy silence and not requested as neutral/healthy states, never zero/failure.
- [x] Show exact repair guidance for service stopped, binary drift, Codex trust missing, reply-flush failure, and capture lag.
- [x] Render three independent benchmark cards with C4−C0 headline and incremental contrast drill-downs; `not_run` must say `Not measured`.
- [x] Poll/push at no more than a 10-second freshness target and label it near-real-time rather than live streaming.
- [x] Run `pnpm test`, `pnpm lint`, and `pnpm build` in the dashboard repository.

### Task 12: Freeze current behavior and run the non-claimable component pilot

- [x] Run all workspace unit/integration tests and the 500-case LongMemEval-S evaluation at the pinned current-behavior commit.
- [x] Verify the six supplied AgentMemory scorecards against the public repository and pin their revision, downloaded-file hashes, claims, evidence classes, and comparability limits in the suite.
- [x] Generate `FINAL-OUTCOMES.md` with the current three-goal `Not measured` verdict, Token Savings reference table, LongMemEval comparison, coding-agent-life-v1 table, and honest competitor analysis.
- [x] Make generated `BENCHMARK.md` reports render external rows separately and refuse to let them populate any C0-C4 goal.
- [ ] Run the 20-session schema smoke with explicit `--execute`; inspect all raw native counters, timings, lifecycle receipts, and isolation hashes.
- [ ] Blind-export and grade the smoke; repair instrumentation only, never retrieval behavior.
- [ ] Run the 240-session component pilot with explicit operator approval.
- [ ] Publish current-state C0–C4 percentages, confidence intervals, raw counts, invalid samples, per-harness results, per-stratum results, and all three goal verdicts as non-claimable.
- [ ] Use the preregistered trigger rules to decide whether Priority 2 and Priority 4 behavior changes are justified.

### Task 13: Calibrate prompt pushes only if triggered

**Files:**

- Modify: `crates/brain-service/src/hook_handler.rs`
- Modify: `crates/brain-service/tests/mid_session_push.rs`
- Create: `benchmarks/second-brain/v2/calibration/prompt-push.json`

- [ ] Sweep candidate thresholds on the visible calibration set and record precision/recall, healthy silence, token budget, and latency percentages.
- [ ] Select the smallest/most conservative configuration that meets the Priority 4 gates; do not inspect locked-test labels during selection.
- [ ] Run the locked evaluator once, record it immutably, and compare current C3 with tuned C3e.
- [ ] Reject tuning if it fails an accuracy, leakage, token, time, or reliability gate.

### Task 14: Implement the continuation capsule only if triggered

**Files:**

- Create: `crates/brain-domain/src/continuation.rs`
- Modify: `crates/brain-domain/src/lib.rs`
- Modify: `crates/brain-service/src/hook_handler.rs`
- Modify: `crates/brain-context/src/compiler.rs`
- Create: `crates/brain-context/tests/continuation_capsule.rs`
- Modify: `crates/brain-service/tests/session_end.rs`
- Create: `benchmarks/second-brain/v2/calibration/orientation.json`

- [ ] Write failing tests for exact field derivation, evidence citations, incompatible revision labeling, project isolation, 600-token cap, and no provider/vector call.
- [ ] Build the capsule deterministically from existing session events and live Git state at SessionEnd.
- [ ] Append it as evidence-backed state and select the newest compatible capsule at SessionStart.
- [ ] Run current C2 versus C2e on the pilot blocks and locked startup gold set.
- [ ] Promote only under the Priority 2 gate; otherwise retain or disable current orientation according to measured C2 − C1 value.

### Task 15: Run the candidate pilot and choose the shipping architecture

- [ ] Freeze a new candidate commit/config hash after any accepted Priority 2/4 changes.
- [ ] Re-run the same 240-session pilot, not a handpicked subset.
- [ ] Compare candidate C4 with current C4 and native C0 using the same three independent gates.
- [ ] Select the smallest architecture whose component increments earn their cost.
- [ ] Record rejected components and their measured regression percentages so they are not reintroduced by intuition later.

### Task 16: Run and publish the claimable matrix

- [ ] Recompute power from pilot variance and set repeats before seeing full-run outcomes.
- [ ] Run preflight and archive its hashes.
- [ ] Execute the 1,600-session matrix only with explicit operator approval.
- [ ] Export blind grades, run automated checks, adjudicate disagreements, and verify kappa at least 80%.
- [ ] Generate all contrasts, adjusted intervals, per-harness/stratum tables, lifecycle reliability, and retrieval locked-test results.
- [x] Run `cargo fmt --all -- --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`.
- [x] If deployment is authorized, deploy through `scripts/deploy.ps1`, then verify the installed commit and binaries in `brain dashboard` rather than assuming the build shipped.
- [ ] Publish only the statements generated by the report evaluator.

### Task 17: Reconcile architecture and operating documentation with measured truth

**Files:**

- Modify: `docs/Secondary Brain — Architecture v3.2.html`
- Modify: `docs/status.md`
- Modify: `docs/roadmap.md`
- Modify: `docs/operations/health-and-alerts.md`
- Modify: `AGENTS.md`
- Modify: `CLAUDE.md`

- [x] Show SessionStart recency/BM25 orientation, UserPromptSubmit BM25+vector push, and explicit four-channel Brain pull as three distinct paths.
- [x] State actual Claude/Codex hook and MCP parity from preflight rather than from historical notes.
- [x] Add measured C0–C4 percentages only when the run is claimable; otherwise show `Not measured` or `Pilot — non-claimable`.
- [x] Document the lifecycle truth table, near-real-time limitation, alert meanings, and Codex trust-first diagnosis.
- [x] Mirror project-registration instructions in `AGENTS.md` and `CLAUDE.md`, with `docs/registering-a-project.md` remaining authoritative.

---

## Evidence and extended benchmark record

Keep full local artifacts under:

`BRAIN_HOME/runtime/token-benchmarks/<project_id>/<run_id>/`

Required contents:

- `manifest.json`: commits, hashes, versions, seed, task blocks, conditions, repeats, and claimability;
- `environment.json`: effective native defaults, machine/runtime information, CodeGraph/vector state, and production-setting hashes;
- `samples.jsonl`: append-only native tokens, elapsed time, traces, status, and artifact hashes;
- `lifecycle.jsonl`: hook/MCP/capture stages used to prove component exposure;
- `retrieval-cases.jsonl`: per-case gold expectations, returned IDs, scores, and verdicts;
- `grades.jsonl`: append-only opaque blind grades;
- `commands.jsonl`: append-only exact CLI argument vectors for preflight, execution, retrieval attachment, grading, and report generation;
- `contrasts.json`: all absolute and percentage comparisons with intervals;
- `report.json` and `summary.json`: generated three-goal verdicts;
- `raw/`: content-addressed native stdout/stderr and trace payloads;
- `checksums.sha256`: recursive artifact hashes.

Publish a compact, non-sensitive record at:

`benchmarks/second-brain/v2/results/<run_id>/BENCHMARK.md`

It must include:

- whether the run is smoke, pilot, or claimable;
- current-state or candidate architecture label;
- C0–C4 configuration hashes;
- task/session counts and invalid percentage;
- all six contrasts with absolute counts, percentages, and confidence intervals;
- Goal 1, Goal 2, and Goal 3 verdicts separately;
- Claude and Codex results separately;
- retrieval path results for SessionStart, prompt push, and historical pull;
- lifecycle reliability and alert validation percentages;
- failures, regressions, caveats, and the exact commands used;
- pinned external competitor rows with `vendor-run`, `vendor-modeled`, or `implementation heuristic` evidence labels and an explicit statement that they do not populate the three causal goals;
- SHA-256 link to the full local record without committing raw private transcripts.

## Required final result table

The generated report must contain this shape; values remain `Not measured` until the corresponding run completes:

| Goal | C0 native | C4 full Brain | Change | 95% CI | Claude | Codex | Gate | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|---|
| Net native tokens | Not measured | Not measured | Not measured | Not measured | Not measured | Not measured | ≥10%, lower bound >0 | Not run |
| End-to-end time | Not measured | Not measured | Not measured | Not measured | Not measured | Not measured | ≥10%, lower bound >0 | Not run |
| Final-answer pass rate | Not measured | Not measured | Not measured | Not measured | Not measured | Not measured | +5 pp superiority on historical tasks; −2 pp overall safety | Not run |

It must also include the four incremental component rows C1−C0, C2−C1, C3−C2, and C4−C3. That table is what decides which layers remain enabled.

The companion `benchmarks/second-brain/v2/FINAL-OUTCOMES.md` contains the required Token Savings and
competitor-analysis presentation. It currently reports the native goals as `Not measured`, sets the
locally reproduced 500-question LongMemEval result beside AgentMemory's published result, and shows
why AgentMemory's annual `~170K` token estimate and dashboard calculators are context models rather
than net native-token measurements. A generated run report repeats the pinned external rows under
`External published context (not a C0-C4 result)`.

## Final decision policy

- If C4 proves all three goals, keep the full architecture and quote only the measured percentages and intervals.
- If C4 saves tokens/time but final quality is merely non-inferior, claim efficiency—not superior answers.
- If C4 improves historical quality but burns more tokens or time, describe the cost explicitly and make the feature opt-in or task-conditional.
- If C2 fails, do not assume “more orientation” is the cure. Test C2e under the stated gate; disable startup injection if it still fails.
- If C3 fails, keep UserPromptSubmit telemetry but default to healthy silence/disable unsolicited memory delivery.
- If C4−C3 is flat because agents never call Brain MCP, the availability alone has not earned its place. Improve discoverability in a separate preregistered experiment or remove it from the default tool set.
- If C1 accounts for most gains and C2–C4 add no safe incremental value, the honest optimal architecture is native harness plus CodeGraph, with the Brain retained as an auditable historical store rather than injected on every session.
- Regardless of performance outcome, keep Priority 5 lifecycle observability if it meets its reliability/false-alert gates; it diagnoses whether the experiment and production integrations are actually operating.

This policy answers the original question without protecting the architecture from an unfavorable result: the Secondary Brain is useful only to the extent that its measured benefits exceed its context, latency, complexity, and error costs.
