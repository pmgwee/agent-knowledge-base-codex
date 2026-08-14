# Second Brain benchmark v2

This directory preregisters the five-condition component-ablation benchmark. It defines what will be measured; it does not contain or imply an empirical performance claim.

## Conditions and comparisons

Every task is run as one randomized five-condition block:

- `c0`: native harness defaults only.
- `c1`: native harness plus CodeGraph.
- `c2`: `c1` plus `SessionStart` orientation and `SessionEnd` capture.
- `c3`: `c2` plus `UserPromptSubmit` current-session pushes.
- `c4`: `c3` plus on-demand Brain MCP historical retrieval.

The six contrasts in `suite.json` isolate each component, the full architecture against native defaults, and Second Brain after CodeGraph is held constant. Results must be labelled `current_state` or `candidate`; the labels are not interchangeable.

## Three independent verdicts

`result-schema.json` requires separate states for net native token use, end-to-end speed, and quality. A result can be `proven`, `inconclusive`, `regressed`, `invalid`, or `not_run`. Passing one goal never substitutes for another.

All percentages use the formulas and fixed gates in `suite.json`. The analysis uses 10,000 task-clustered bootstrap resamples with the preregistered seed and Holm adjustment. Raw counters and artifact hashes are retained so every percentage can be reconstructed.

`FINAL-OUTCOMES.md` is the current compact outcome and competitor-analysis surface. It keeps the
three native goals as `Not measured` until paid C0-C4 sessions pass their gates, while presenting
the already-reproduced LongMemEval retrieval result beside pinned AgentMemory references.

## External reference contract

The `external_references` entries in `suite.json` pin the source revision, downloaded-file SHA-256,
claim, evidence class, and comparability rule for every AgentMemory row. Generated `BENCHMARK.md`
reports include those rows under **External published context (not a C0-C4 result)**. They never
populate Goal 1, Goal 2, or Goal 3.

In particular, AgentMemory's published annual `~170K` tokens/year and its 82%-100% scale savings are
modeled context-payload comparisons, not observed native Claude/Codex usage. Its CLI and viewer use
explicit estimation formulas, which are pinned as implementation-heuristic references in the suite.
They are valid competitor context and invalid substitutes for the matched native-token experiment.

## Retrieval gold set

`retrieval-gold/calibration.jsonl` and `retrieval-gold/locked-test.jsonl` each contain 120 deterministic cases: 40 `session_start`, 40 `prompt_push`, and 40 `historical_pull`. Half of the prompt-push cases are healthy-silence negatives. Positive cases include exact acceptable event IDs and byte offsets; negatives require abstention and intentionally contain no acceptable evidence.

The calibration split may be used to choose thresholds. The locked split must remain unseen by retrieval tuning and is opened only for the final evaluation. `generate-gold.ps1` reproducibly materializes both manifests; it creates synthetic evaluation fixtures, not benchmark outcomes.

The evaluator's `--fixture` mode materializes one isolated ledger per case from those declared UUIDs
and offsets. This avoids making 40 mutually exclusive SessionStart questions share one orientation.
It is a component correctness test, not a live-corpus or coding-agent comparison. On 13 August 2026
the visible 120-case calibration fixture resolved 120/120 cases and produced 100.0% precision,
recall, F1, MRR, nDCG@5, fact accuracy, citation precision/coverage, faithfulness, freshness and
abstention, with 0.0% harmful pushes and 100.0% healthy silence. Those figures prove the fixture and
evaluator agree; they do **not** prove C4 beats native retrieval. The locked split remains unopened.

## Execution safety

Preflight and manifest generation are local and non-billable. Native Claude Code or Codex benchmark execution must refuse to run unless the operator supplies the explicit execution gate documented by the benchmark CLI. Pilot and claimable outputs belong under an immutable run directory with their configuration hash, raw traces, grades, and validity report.

Build the release binaries, then generate machine-local execution templates without committing
credential paths:

```powershell
cargo build --workspace --release
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/New-BenchmarkExecutionTemplates.ps1 `
  -OutputPath "$env:TEMP\second-brain-execution-templates.json"
```

`brain benchmark preflight --smoke --repeats 1` creates exactly 20 planned sessions (two tasks ×
two harnesses × five conditions) and launches zero. `--pilot --repeats 1` uses the preregistered
pilot tasks. `brain benchmark run` is a preview; only the separate `--execute` flag crosses the paid
boundary. The launcher uses fresh user/harness homes, removes inherited project integration files,
copies credentials only for the child lifetime, and scrubs them again after success, failure, or
timeout.

The full evidence directory contains `manifest.json`, `environment.json`, append-only
`samples.jsonl`, `lifecycle.jsonl`, `retrieval-cases.jsonl`, `grades.jsonl`, `contrasts.json`, the
generated reports, raw content-addressed native output, and `checksums.sha256`. Attach calibration or
locked retrieval results to a run with `brain benchmark retrieval ... --run <uuid>`.
