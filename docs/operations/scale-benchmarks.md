# Scale benchmarks

The generator is deterministic by seed and emits Claude Code, Codex, and
Hermes sessions containing compactions, decisions/reversals, tool and test
outcomes, file changes, deployments, handoffs, schema-unknown evidence,
worktrees, known-answer markers, and a same-named isolated project.

Profiles:

- `smoke`: 100 sessions and 10,000 events; runs in ordinary tests.
- `primary`: 12,000 sessions and 6,000,000 events; required release gate.
- `stress`: 120,000 sessions and 60,000,000 events; overnight release-candidate
  gate and intentionally ignored by ordinary tests.

The JSON report records reproducibility hashes, ingest throughput, completeness,
gap coverage, replay duplicates, storage, peak bounded batch size, startup and
query p50/p95/p99 both warm and cold, 1,000-session baseline degradation,
evidence precision and recall, project leakage, token reduction, online
backup/isolated restore time, and RPO/RTO results.

Warm figures measure repeated retrieval through the bounded scoped-query cache,
which is what an agent experiences within a session. Cold figures open a fresh
ledger for each query so the cache is empty, which is what actually shows
whether retrieval still scales as history grows.

Primary acceptance is at least 99.9% capture completeness with gaps visible,
zero stored replay duplicates and project leakage, at least 95% precision and
recall, 100% supersession fixtures, at least 80% session-start token reduction,
startup p95 at most 500 ms, warm query p95 at most 25 ms, cold query p95 at most
one second, and complete backup recovery within two hours. The 1,000-session
degradation percentage is reported for trend analysis but is not a gate; see
§15.2 of the design specification for why it was superseded. Preserve the
emitted JSON alongside the release binary and hardware description.

Every gate copies its report to `target/<profile>-benchmark-report.json` before
asserting, so a run that fails a threshold still leaves its measurements behind.
Set `BRAIN_BENCHMARK_REPORT_DIR` to redirect that copy when the corpus is built
on a scratch volume.
