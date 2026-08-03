# Production verification

Verified on 2026-08-03 from commit `e10cd59` on Windows 11
10.0.26200 (64-bit), AMD Ryzen 7 5800H (8 cores/16 logical processors),
27.9 GiB visible memory, Rust/Cargo 1.88.0.

## Primary production gate

Command:

```powershell
cargo test --release -p brain-cli --test primary `
  primary_production_corpus_passes_all_gates -- --ignored --nocapture
```

The deterministic seed-42 corpus passed every gate with no failures:

| Measurement | Result |
| --- | ---: |
| Sessions | 12,000 |
| Generated/captured events | 6,000,000 / 6,000,000 |
| Capture completeness | 100% |
| Explicit gap coverage | yes |
| Replay duplicate rows | 0 |
| Ingest throughput | 2,030.68 events/s |
| Canonical storage | 8,082,862,080 bytes |
| Peak bounded batch | 1,300,920 bytes |
| Startup p50 / p95 / p99 | 2.2786 / 3.2650 / 3.2650 ms |
| Warm scoped query p50 / p95 / p99 | 0.0070 / 0.0078 / 0.0081 ms |
| 1,000-session query p95 baseline | 0.0075 ms |
| Query degradation | 4.0% (then gated at 20%; now diagnostic only) |
| Historical precision / recall | 100% / 100% |
| Supersession fixtures | 100% correct |
| Cross-project leakage | 0 hits |
| New-session token reduction | 99.99875% |
| Backup / isolated restore | 286.55 / 272.42 seconds |
| RPO / RTO | pass / pass |
| Hook contract | unchanged |

The query measurement is the warm service-side path used by the long-running
brain service. The canonical query cache is capped at 128 entries and 16 MiB,
cleared after local writes, and invalidated after commits from other
connections through SQLite `data_version`. The preceding uncached six-million
event diagnostic measured raw FTS p95 at 4.92 ms, also well under the one-second
absolute gate.

The final release hook latency gate also passed with warm p95 9.46 ms and p99
9.79 ms against a 50 ms p95 limit.

## Ten-times stress gate

**Status: attempted, terminated before reporting. Not yet qualified.**

The deterministic stress profile (120,000 sessions and 60,000,000 events) is
implemented as an ignored/manual release-candidate test. The primary run
temporarily required about 22.58 GB for the source, verified backup, and
isolated restore, so the 10-times profile needs roughly 226 GB of scratch space
before safety margin. The system volume had only 88.1 GB free, but drive D: had
about 347 GB, which is sufficient.

An attempt ran on 2026-08-03 with temporary storage redirected to D::

```powershell
$env:TEMP = 'D:\AgentBrainStress'; $env:TMP = $env:TEMP
cargo test --release -p brain-cli --test stress `
  stress_corpus_keeps_startup_query_and_memory_bounds -- --ignored --nocapture
```

It ingested for 8.9 hours, reaching about 74.95 GiB — roughly 92% of the
expected corpus — with process memory flat near 24 MiB throughout, which is the
bounded-memory behaviour the gate exists to demonstrate. It then stopped at
13:32:57 without reaching the backup phase and without emitting a report. No
cause appears in the Windows Application or System event logs. The temporary
directory survived, which indicates abrupt termination rather than a normal
return or a failed assertion, since either of those would have run the
`TempDir` destructor.

Two defects that attempt exposed are now fixed in `3998d9c`:

- the report was written inside the temporary corpus directory, so any normal
  completion would have deleted it along with the corpus;
- the run was launched without redirecting stdout to a file, so its console
  output had no durable destination once the launching session ended.

Re-run with output redirected to a log so an interrupted attempt remains
diagnosable, and check progress sparsely rather than in a polling loop:

```powershell
$env:TEMP = 'D:\AgentBrainStress'; $env:TMP = $env:TEMP
cargo test --release -p brain-cli --test stress `
  stress_corpus_keeps_startup_query_and_memory_bounds -- --ignored --nocapture `
  *> D:\AgentBrainStress\stress-run.log
```

Not running a disk-destructive workload on an undersized system volume remains
an explicit safety constraint, not a waived correctness gate.
