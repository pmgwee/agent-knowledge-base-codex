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

Stress acceptance is narrower and deliberately so: complete without unbounded
memory growth, integer or cursor overflow, or linear startup scans, with scoped
query latency and context size within twice the primary tier. Recovery *time* is
recorded at that tier rather than gated, because a corpus ten times larger is
expected to take proportionally longer to restore — measured at 8,493 s for
sixty million events against 272 s for six million. RPO remains gated everywhere,
since whether the restored ledger holds every captured event is correctness
rather than duration.

Every gate copies its report to `target/<profile>-benchmark-report.json` before
asserting, so a run that fails a threshold still leaves its measurements behind.
Set `BRAIN_BENCHMARK_REPORT_DIR` to redirect that copy when the corpus is built
on a scratch volume.

## Running a profile outside the test harness

The same generator and gates are available from the CLI, which writes its report
to a directory you choose rather than a temporary one:

```powershell
brain benchmark --profile primary --output D:\AgentBrainBench\primary --seed 42
```

`--output` must not already exist. `--profile` accepts `smoke`, `primary`, or
`stress`, and `--seed` defaults to 42; an identical seed reproduces an identical
corpus and identical known-answer markers. The command prints the report and
exits non-zero if any gate fails.

## Measuring a corpus that already exists

To qualify a large ledger without regenerating it — an interrupted run, or a
real brain that has grown over time — use the read-only diagnostic instead:

```powershell
$env:BRAIN_DIAGNOSTIC_DB = 'D:\path\to\ledger.sqlite'
$env:BRAIN_DIAGNOSTIC_PROJECT = '<project-uuid>'
cargo test --release -p brain-cli --lib existing_corpus -- --ignored --nocapture
```

It reports event count, open cost, startup percentiles, and cold retrieval
percentiles, and writes nothing. Run it twice: the first execution pays
write-ahead-log recovery and a cold operating system page cache, the second
reports steady state. It covers the performance gates only, since correctness
gates need ground truth that only a full generated run produces.
